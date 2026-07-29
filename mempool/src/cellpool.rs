// SPDX-License-Identifier: MIT
// Copyright (C) 2026 Myelin developers

//! Atomic Cell transaction memory pool.

use crate::{MempoolError, Result, TransactionScore, TransactionScorer};
use indexmap::IndexMap;
use myelin_exec::{celltx::sighash::compute_wtxid, CellTx, OutPoint};
use myelin_state::VerifiedStateTransaction;
use parking_lot::RwLock;
use std::{
    cmp::Ordering,
    collections::{BTreeMap, BTreeSet, VecDeque},
    sync::Arc,
};

const DETERMINISTIC_POOL_TIMESTAMP: u64 = 0;
const CYCLES_PER_BYTE: u64 = 100;

/// Pool entry derived from a contextually verified transaction.
#[derive(Clone, Debug)]
pub struct PoolEntry {
    /// Transaction.
    pub tx: CellTx,
    /// Canonical raw transaction id used for identity and dependency edges.
    pub txid: [u8; 32],
    /// Witness-inclusive hash retained as evidence, never as Cell identity.
    pub wtxid: [u8; 32],
    /// Transaction score.
    pub score: TransactionScore,
    /// Deterministic placeholder timestamp.
    pub timestamp: u64,
    /// Fee derived from resolved live inputs minus outputs.
    pub fee: u64,
    /// VM-verified cycles.
    pub cycles: u64,
    /// State root used for contextual admission.
    pub admitted_state_root: [u8; 32],
    /// State root after applying this transaction to its admitted pre-state.
    pub resulting_state_root: [u8; 32],
    /// Parent raw transaction ids.
    pub dependencies: Vec<[u8; 32]>,
    /// Child raw transaction ids.
    pub dependents: Vec<[u8; 32]>,
}

/// Pool statistics.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct PoolStats {
    /// Total transactions.
    pub total_txs: usize,
    /// Total serialized size in bytes.
    pub total_size: usize,
    /// Total verified fees.
    pub total_fee: u64,
    /// Transactions added, including replacements.
    pub txs_added: u64,
    /// Transactions removed, including RBF descendants.
    pub txs_removed: u64,
    /// Successful atomic replacement operations.
    pub rbf_count: u64,
}

#[derive(Clone, Default)]
struct PoolInner {
    txs: IndexMap<[u8; 32], PoolEntry>,
    spent_outputs: BTreeMap<OutPoint, [u8; 32]>,
    stats: PoolStats,
}

/// Deterministic, package-aware Cell transaction memory pool.
pub struct CellPool {
    inner: Arc<RwLock<PoolInner>>,
    scorer: Arc<TransactionScorer>,
    max_size: usize,
    base_state_root: [u8; 32],
}

impl CellPool {
    /// Create an empty pool with a transaction-count limit.
    pub fn new(max_size: usize, base_state_root: [u8; 32]) -> Self {
        Self {
            inner: Arc::new(RwLock::new(PoolInner::default())),
            scorer: Arc::new(TransactionScorer::default()),
            max_size,
            base_state_root,
        }
    }

    /// Atomically admit a contextually verified transaction.
    ///
    /// Fee, cycles and pre-state root come only from
    /// [`VerifiedStateTransaction`]; callers cannot pass independent scoring
    /// values. Conflicts and every descendant are replaced as one package or
    /// the original pool remains byte-for-byte unchanged.
    pub fn add(&self, admitted: VerifiedStateTransaction) -> Result<[u8; 32]> {
        let tx = admitted.transaction().clone();
        let txid = admitted.txid();
        if tx.id() != txid {
            return Err(MempoolError::InvalidTx("admission proof transaction id mismatch".to_owned()));
        }
        if tx.inputs.iter().map(|input| input.previous_output).collect::<BTreeSet<_>>().len() != tx.inputs.len() {
            return Err(MempoolError::InvalidTx("duplicate transaction input".to_owned()));
        }

        let mut guard = self.inner.write();
        if guard.txs.contains_key(&txid) {
            return Err(MempoolError::TxExists(txid));
        }

        let direct_conflicts =
            tx.inputs.iter().filter_map(|input| guard.spent_outputs.get(&input.previous_output).copied()).collect::<BTreeSet<_>>();
        let evicted = descendants_including(&guard.txs, &direct_conflicts);

        if guard.txs.len().saturating_sub(evicted.len()) >= self.max_size {
            return Err(MempoolError::MempoolFull(self.max_size));
        }
        if !evicted.is_empty() {
            self.validate_replacement(&guard, &admitted, &evicted)?;
        }

        let dependencies = tx
            .inputs
            .iter()
            .filter_map(|input| {
                guard
                    .txs
                    .iter()
                    .find(|(parent_txid, parent)| !evicted.contains(*parent_txid) && input.previous_output.tx_hash == parent.txid)
                    .map(|(parent_txid, _)| *parent_txid)
            })
            .collect::<BTreeSet<_>>()
            .into_iter()
            .collect::<Vec<_>>();
        let admitted_root = admitted.state_root().as_bytes();
        match dependencies.as_slice() {
            [] if admitted_root != self.base_state_root => {
                return Err(MempoolError::InvalidTx("transaction was not admitted against the pool base state root".to_owned()));
            }
            [] => {}
            [parent_txid] => {
                let expected = guard.txs.get(parent_txid).ok_or(MempoolError::TxNotFound(*parent_txid))?.resulting_state_root;
                if admitted_root != expected {
                    return Err(MempoolError::InvalidTx("child admission root does not match its parent result root".to_owned()));
                }
            }
            _ => {
                return Err(MempoolError::InvalidTx(
                    "multi-parent package admission requires a combined overlay proof and is not supported".to_owned(),
                ));
            }
        }

        let mut next = guard.clone();
        for evicted_txid in &evicted {
            remove_one(&mut next, evicted_txid);
        }

        let fee = admitted.fee();
        let cycles = admitted.cycles();
        let entry = PoolEntry {
            score: self.scorer.compute_score(&tx, fee, cycles),
            wtxid: compute_wtxid(&tx),
            tx: tx.clone(),
            txid,
            timestamp: DETERMINISTIC_POOL_TIMESTAMP,
            fee,
            cycles,
            admitted_state_root: *admitted.state_root().as_ref(),
            resulting_state_root: admitted.state_root_after().as_bytes(),
            dependencies: dependencies.clone(),
            dependents: Vec::new(),
        };
        for input in &tx.inputs {
            next.spent_outputs.insert(input.previous_output, txid);
        }
        for parent_txid in &dependencies {
            if let Some(parent) = next.txs.get_mut(parent_txid) {
                parent.dependents.push(txid);
                parent.dependents.sort_unstable();
                parent.dependents.dedup();
            }
        }
        next.txs.insert(txid, entry);
        next.stats.total_txs += 1;
        next.stats.total_size += tx.serialized_size();
        next.stats.total_fee = next.stats.total_fee.saturating_add(fee);
        next.stats.txs_added += 1;
        if !evicted.is_empty() {
            next.stats.rbf_count += 1;
        }
        *guard = next;
        Ok(txid)
    }

    /// Remove a transaction and all of its descendants atomically.
    pub fn remove_package(&self, txid: &[u8; 32]) -> Result<Vec<CellTx>> {
        let mut guard = self.inner.write();
        if !guard.txs.contains_key(txid) {
            return Err(MempoolError::TxNotFound(*txid));
        }
        let roots = BTreeSet::from([*txid]);
        let package = descendants_including(&guard.txs, &roots);
        let mut next = guard.clone();
        let mut removed = Vec::with_capacity(package.len());
        for member in &package {
            if let Some(tx) = remove_one(&mut next, member) {
                removed.push(tx);
            }
        }
        *guard = next;
        Ok(removed)
    }

    /// Get a transaction by raw transaction id.
    pub fn get(&self, txid: &[u8; 32]) -> Option<PoolEntry> {
        self.inner.read().txs.get(txid).cloned()
    }

    /// Get transactions sorted by score, then raw txid.
    pub fn get_sorted(&self, limit: usize) -> Vec<PoolEntry> {
        let mut entries = self.inner.read().txs.values().cloned().collect::<Vec<_>>();
        entries.sort_by(|a, b| b.score.total.total_cmp(&a.score.total).then_with(|| a.txid.cmp(&b.txid)));
        entries.into_iter().take(limit).collect()
    }

    /// Snapshot pool statistics.
    pub fn stats(&self) -> PoolStats {
        self.inner.read().stats.clone()
    }

    fn validate_replacement(
        &self,
        inner: &PoolInner,
        admitted: &VerifiedStateTransaction,
        evicted: &BTreeSet<[u8; 32]>,
    ) -> Result<()> {
        let mut old_fee = 0u64;
        let mut old_effective_size = 0u64;
        for txid in evicted {
            let entry = inner.txs.get(txid).ok_or(MempoolError::TxNotFound(*txid))?;
            old_fee = old_fee.checked_add(entry.fee).ok_or_else(|| MempoolError::RBFFailed("evicted fee overflow".to_owned()))?;
            old_effective_size = old_effective_size
                .checked_add(effective_size(&entry.tx, entry.cycles))
                .ok_or_else(|| MempoolError::RBFFailed("evicted effective size overflow".to_owned()))?;
        }
        let new_fee = admitted.fee();
        let new_effective_size = effective_size(admitted.transaction(), admitted.cycles());
        if new_fee <= old_fee {
            return Err(MempoolError::RBFFailed(format!("replacement fee {new_fee} must exceed evicted package fee {old_fee}")));
        }
        let density_order = (new_fee as u128 * old_effective_size as u128).cmp(&(old_fee as u128 * new_effective_size as u128));
        if density_order != Ordering::Greater {
            return Err(MempoolError::RBFFailed("replacement fee density must exceed the evicted package".to_owned()));
        }
        Ok(())
    }
}

fn effective_size(tx: &CellTx, cycles: u64) -> u64 {
    (tx.serialized_size() as u64).saturating_mul(CYCLES_PER_BYTE).max(cycles)
}

fn descendants_including(txs: &IndexMap<[u8; 32], PoolEntry>, roots: &BTreeSet<[u8; 32]>) -> BTreeSet<[u8; 32]> {
    let mut result = roots.clone();
    let mut queue = roots.iter().copied().collect::<VecDeque<_>>();
    while let Some(txid) = queue.pop_front() {
        if let Some(entry) = txs.get(&txid) {
            for child in &entry.dependents {
                if result.insert(*child) {
                    queue.push_back(*child);
                }
            }
        }
    }
    result
}

fn remove_one(inner: &mut PoolInner, txid: &[u8; 32]) -> Option<CellTx> {
    let entry = inner.txs.shift_remove(txid)?;
    for input in &entry.tx.inputs {
        if inner.spent_outputs.get(&input.previous_output) == Some(txid) {
            inner.spent_outputs.remove(&input.previous_output);
        }
    }
    for parent_txid in &entry.dependencies {
        if let Some(parent) = inner.txs.get_mut(parent_txid) {
            parent.dependents.retain(|child| child != txid);
        }
    }
    inner.stats.total_txs -= 1;
    inner.stats.total_size -= entry.tx.serialized_size();
    inner.stats.total_fee -= entry.fee;
    inner.stats.txs_removed += 1;
    Some(entry.tx)
}

#[cfg(test)]
mod tests {
    use super::*;
    use myelin_exec::{CellInput, CellOutput, Script};
    use myelin_state::{CellEntry, CellStateTree, StateTransitionContext, StateTransitionEngine};

    fn output(capacity: u64) -> CellOutput {
        CellOutput { lock: Script::new([0x10; 32], 0, vec![]), type_: None, capacity }
    }

    fn tx(input: OutPoint, output_capacity: u64) -> CellTx {
        CellTx::new(vec![CellInput::new(input, 0)], vec![], vec![output(output_capacity)], vec![vec![]], vec![vec![0; 65]]).unwrap()
    }

    fn engine_with_inputs(inputs: &[(OutPoint, u64)]) -> StateTransitionEngine {
        let mut state = CellStateTree::new();
        for (outpoint, capacity) in inputs {
            state.insert(*outpoint, CellEntry::from_output(&output(*capacity), &[], 0, false).unwrap());
        }
        StateTransitionEngine::new(state)
    }

    fn admitted(engine: &mut StateTransitionEngine, tx: &CellTx, cycles: u64) -> VerifiedStateTransaction {
        engine.verify_transaction(tx, StateTransitionContext::ordinary(1), |_, _| Ok::<_, String>(cycles)).unwrap()
    }

    fn apply(engine: &mut StateTransitionEngine, tx: &CellTx, cycles: u64) {
        engine.apply_transaction(tx, StateTransitionContext::ordinary(1), |_, _| Ok::<_, String>(cycles)).unwrap();
    }

    #[test]
    fn add_uses_verified_fee_and_raw_txid() {
        let input = OutPoint::new([1; 32], 0);
        let transaction = tx(input, 1_000);
        let mut engine = engine_with_inputs(&[(input, 1_100)]);
        let pool = CellPool::new(10, engine.state_root().as_bytes());
        let txid = pool.add(admitted(&mut engine, &transaction, 1_000)).unwrap();
        let entry = pool.get(&txid).unwrap();
        assert_eq!(txid, transaction.id());
        assert_eq!(entry.fee, 100);
        assert_eq!(entry.cycles, 1_000);
    }

    #[test]
    fn rbf_replaces_conflict_and_descendants_atomically() {
        let external = OutPoint::new([2; 32], 0);
        let mut base_engine = engine_with_inputs(&[(external, 1_100)]);
        let pool = CellPool::new(10, base_engine.state_root().as_bytes());
        let parent = tx(external, 1_000);
        let parent_admitted = admitted(&mut base_engine, &parent, 1_000);
        let parent_id = pool.add(parent_admitted).unwrap();
        let mut parent_overlay = base_engine.clone();
        apply(&mut parent_overlay, &parent, 1_000);
        let child = tx(OutPoint::new(parent.id(), 0), 800);
        let child_id = pool.add(admitted(&mut parent_overlay, &child, 1_000)).unwrap();

        let replacement = tx(external, 600);
        let replacement_id = pool.add(admitted(&mut base_engine, &replacement, 1_000)).unwrap();
        assert!(pool.get(&parent_id).is_none());
        assert!(pool.get(&child_id).is_none());
        assert!(pool.get(&replacement_id).is_some());
        assert_eq!(pool.stats().rbf_count, 1);
        assert_eq!(pool.stats().total_txs, 1);
    }

    #[test]
    fn rejected_rbf_leaves_entire_pool_unchanged() {
        let external = OutPoint::new([3; 32], 0);
        let mut engine = engine_with_inputs(&[(external, 1_100)]);
        let pool = CellPool::new(10, engine.state_root().as_bytes());
        let original = tx(external, 1_000);
        let original_id = pool.add(admitted(&mut engine, &original, 1_000)).unwrap();
        let stats_before = pool.stats();
        let replacement = tx(external, 1_050);

        assert!(matches!(pool.add(admitted(&mut engine, &replacement, 1_000)), Err(MempoolError::RBFFailed(_))));
        assert!(pool.get(&original_id).is_some());
        assert_eq!(pool.stats(), stats_before);
    }

    #[test]
    fn max_size_is_checked_after_atomic_replacement() {
        let external = OutPoint::new([4; 32], 0);
        let mut engine = engine_with_inputs(&[(external, 1_100)]);
        let pool = CellPool::new(1, engine.state_root().as_bytes());
        let original = tx(external, 1_000);
        pool.add(admitted(&mut engine, &original, 1_000)).unwrap();
        let replacement = tx(external, 800);
        assert!(pool.add(admitted(&mut engine, &replacement, 1_000)).is_ok());
        assert_eq!(pool.stats().total_txs, 1);
    }

    #[test]
    fn remove_package_removes_descendants() {
        let external = OutPoint::new([5; 32], 0);
        let mut base_engine = engine_with_inputs(&[(external, 1_100)]);
        let pool = CellPool::new(10, base_engine.state_root().as_bytes());
        let parent = tx(external, 1_000);
        let parent_id = pool.add(admitted(&mut base_engine, &parent, 1_000)).unwrap();
        apply(&mut base_engine, &parent, 1_000);
        let child = tx(OutPoint::new(parent.id(), 0), 900);
        let child_id = pool.add(admitted(&mut base_engine, &child, 1_000)).unwrap();

        let removed = pool.remove_package(&parent_id).unwrap();
        assert_eq!(removed.len(), 2);
        assert!(pool.get(&parent_id).is_none());
        assert!(pool.get(&child_id).is_none());
    }

    #[test]
    fn sorted_order_uses_verified_fee_density() {
        let fixtures = [(6, 50), (7, 200), (8, 100)];
        let inputs = fixtures.map(|(byte, fee)| (OutPoint::new([byte; 32], 0), 1_000 + fee));
        let mut engine = engine_with_inputs(&inputs);
        let pool = CellPool::new(10, engine.state_root().as_bytes());
        for (byte, _fee) in fixtures {
            let transaction = tx(OutPoint::new([byte; 32], 0), 1_000);
            pool.add(admitted(&mut engine, &transaction, 1_000)).unwrap();
        }
        let sorted = pool.get_sorted(10);
        assert!(sorted[0].fee > sorted[1].fee && sorted[1].fee > sorted[2].fee);
    }

    #[test]
    fn root_transaction_admitted_against_another_snapshot_is_rejected() {
        let input = OutPoint::new([9; 32], 0);
        let extra = OutPoint::new([10; 32], 0);
        let transaction = tx(input, 1_000);
        let mut pool_engine = engine_with_inputs(&[(input, 1_100)]);
        let pool = CellPool::new(10, pool_engine.state_root().as_bytes());
        let mut foreign_engine = engine_with_inputs(&[(input, 1_100), (extra, 500)]);

        assert!(matches!(
            pool.add(admitted(&mut foreign_engine, &transaction, 1_000)),
            Err(MempoolError::InvalidTx(message)) if message.contains("pool base state root")
        ));
        assert_eq!(pool.stats().total_txs, 0);
    }

    #[test]
    fn child_admission_must_match_parent_result_root() {
        let external = OutPoint::new([11; 32], 0);
        let extra = OutPoint::new([12; 32], 0);
        let mut base = engine_with_inputs(&[(external, 1_100)]);
        let pool = CellPool::new(10, base.state_root().as_bytes());
        let parent = tx(external, 1_000);
        let parent_id = pool.add(admitted(&mut base, &parent, 1_000)).unwrap();

        let mut foreign_overlay = engine_with_inputs(&[(external, 1_100), (extra, 500)]);
        apply(&mut foreign_overlay, &parent, 1_000);
        let child = tx(OutPoint::new(parent.id(), 0), 900);
        assert!(matches!(
            pool.add(admitted(&mut foreign_overlay, &child, 1_000)),
            Err(MempoolError::InvalidTx(message)) if message.contains("parent result root")
        ));
        assert!(pool.get(&parent_id).is_some());
        assert_eq!(pool.stats().total_txs, 1);
    }

    #[test]
    fn multi_parent_child_requires_combined_package_proof() {
        let input_a = OutPoint::new([13; 32], 0);
        let input_b = OutPoint::new([14; 32], 0);
        let mut base = engine_with_inputs(&[(input_a, 1_100), (input_b, 1_100)]);
        let pool = CellPool::new(10, base.state_root().as_bytes());
        let parent_a = tx(input_a, 1_000);
        let parent_b = tx(input_b, 1_000);
        pool.add(admitted(&mut base, &parent_a, 1_000)).unwrap();
        pool.add(admitted(&mut base, &parent_b, 1_000)).unwrap();

        let mut combined_overlay = base.clone();
        apply(&mut combined_overlay, &parent_a, 1_000);
        apply(&mut combined_overlay, &parent_b, 1_000);
        let child = CellTx::new(
            vec![CellInput::new(OutPoint::new(parent_a.id(), 0), 0), CellInput::new(OutPoint::new(parent_b.id(), 0), 0)],
            vec![],
            vec![output(1_700)],
            vec![vec![]],
            vec![],
        )
        .unwrap();

        assert!(matches!(
            pool.add(admitted(&mut combined_overlay, &child, 1_000)),
            Err(MempoolError::InvalidTx(message)) if message.contains("multi-parent package admission")
        ));
        assert_eq!(pool.stats().total_txs, 2);
    }
}
