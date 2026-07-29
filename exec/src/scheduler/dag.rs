// SPDX-License-Identifier: MIT
// Copyright (C) 2026 Myelin developers
//
// CellDAG: RW-Set dependency graph construction

use crate::celltx::types::{CellTx, OutPoint};
use std::collections::{BTreeMap, BTreeSet};

/// Node ID in the transaction DAG
pub type NodeId = usize;

/// DAG edge type
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DagEdge {
    /// Dependency edge: A produces Cell that B consumes
    Dependency,
    /// Read dependency: A produces Cell that B reads (deps)
    ReadDep,
}

/// Access mode for conflict detection
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum AccessMode {
    /// Read-only access (READ_REF)
    Read,
    /// Write access (CONSUME, CREATE, DESTROY, TRANSFER)
    Write,
}

/// Conflict entry with access mode
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ConflictEntry {
    /// Node ID of the transaction
    pub node_id: NodeId,
    /// Access mode (read or write)
    pub mode: AccessMode,
}

/// Scheduler accesses authenticated and bound to one raw transaction.
///
/// This is a sidecar admission object. It is deliberately not decoded from
/// transaction witnesses by CellDAG: callers must obtain it from the
/// CellScript artifact adapter or another trusted metadata source.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct SchedulerPlan {
    txid: [u8; 32],
    accesses: Vec<([u8; 32], AccessMode)>,
}

impl SchedulerPlan {
    /// Create a plan from already resolved conflict domains.
    pub fn new(tx: &CellTx, accesses: impl IntoIterator<Item = ([u8; 32], AccessMode)>) -> Result<Self, DagError> {
        let mut aggregated = BTreeMap::<[u8; 32], AccessMode>::new();
        for (conflict_hash, mode) in accesses {
            if conflict_hash == [0; 32] {
                return Err(DagError::InvalidSchedulerPlan("zero conflict hash".to_owned()));
            }
            aggregated
                .entry(conflict_hash)
                .and_modify(|current| {
                    if mode == AccessMode::Write {
                        *current = AccessMode::Write;
                    }
                })
                .or_insert(mode);
        }
        Ok(Self { txid: tx.id(), accesses: aggregated.into_iter().collect() })
    }

    /// Create an empty plan for a transaction without typed scheduling metadata.
    pub fn empty(tx: &CellTx) -> Self {
        Self { txid: tx.id(), accesses: Vec::new() }
    }

    /// Raw transaction hash this plan is bound to.
    pub fn txid(&self) -> [u8; 32] {
        self.txid
    }

    /// Aggregated logical accesses, sorted by conflict hash.
    pub fn accesses(&self) -> &[([u8; 32], AccessMode)] {
        &self.accesses
    }
}

/// Cell transaction DAG
///
/// Builds a dependency graph from RW-Sets:
/// - Nodes: transactions
/// - Edges: data dependencies (outputs -> inputs)
/// - Conflicts: typed transactions touching the same logical conflict domain
#[derive(Debug, Clone)]
pub struct CellDAG {
    /// Number of nodes (transactions)
    pub node_count: usize,

    /// Adjacency list: node → [(successor, edge_type)]
    pub edges: BTreeMap<NodeId, Vec<(NodeId, DagEdge)>>,

    /// Reverse adjacency: node → [predecessors]
    pub reverse_edges: BTreeMap<NodeId, Vec<NodeId>>,

    /// Conflict-hash-level entries: conflict_hash -> [ConflictEntry]
    ///
    /// Used for typed-cell conflict detection where multiple transactions
    /// touch the same stable conflict domain.
    pub conflict_hash_conflicts: BTreeMap<[u8; 32], Vec<ConflictEntry>>,

    /// Topological layers (for parallel execution)
    pub layers: Vec<Vec<NodeId>>,
}

impl CellDAG {
    /// Build DAG from a set of Cell transactions
    ///
    /// # Algorithm
    /// 1. Build RW-Sets for each transaction
    /// 2. Detect dependencies: A.outputs ∩ B.inputs → A → B
    /// 3. Detect read cell_deps: A.outputs ∩ B.cell_deps → A → B
    /// 4. Detect conflicts: A.inputs ∩ B.inputs ≠ ∅
    /// 5. Compute topological layers
    pub fn build(txs: &[CellTx]) -> Result<Self, DagError> {
        let node_count = txs.len();
        let mut edges: BTreeMap<NodeId, Vec<(NodeId, DagEdge)>> = BTreeMap::new();
        let mut reverse_edges: BTreeMap<NodeId, Vec<NodeId>> = BTreeMap::new();
        let mut txids: BTreeMap<[u8; 32], NodeId> = BTreeMap::new();
        let mut consumers: BTreeMap<OutPoint, NodeId> = BTreeMap::new();

        // Step 1: Build producers map (OutPoint → NodeId)
        let mut producers: BTreeMap<OutPoint, NodeId> = BTreeMap::new();
        for (node_id, tx) in txs.iter().enumerate() {
            // CKB OutPoints are identified by the raw transaction hash. Witness
            // changes must not change the identity of a produced Cell.
            let tx_hash = tx.id();
            if let Some(first_node) = txids.insert(tx_hash, node_id) {
                return Err(DagError::DuplicateTransaction { txid: tx_hash, first_node, duplicate_node: node_id });
            }
            for (idx, _) in tx.outputs.iter().enumerate() {
                let out_point = OutPoint::new(tx_hash, idx as u32);
                producers.insert(out_point, node_id);
            }
        }

        // Step 2: Detect dependencies and conflicts
        for (consumer_id, tx) in txs.iter().enumerate() {
            // Check inputs (consume edges)
            for input in &tx.inputs {
                if let Some(&producer_id) = producers.get(&input.previous_output) {
                    // Dependency: producer → consumer
                    Self::add_edge(&mut edges, &mut reverse_edges, producer_id, consumer_id, DagEdge::Dependency)?;
                } else {
                    // External Cell (not in this DAG)
                    // Will be resolved from state layer
                }

                // A physical Cell can be consumed exactly once in a batch. This
                // is a validity rule, not a scheduler ordering opportunity.
                if let Some(first_consumer) = consumers.insert(input.previous_output, consumer_id) {
                    return Err(DagError::DoubleSpend {
                        out_point: input.previous_output,
                        first_consumer,
                        second_consumer: consumer_id,
                    });
                }
            }

            // Check deps (read-only edges)
            for dep in &tx.cell_deps {
                if let Some(&producer_id) = producers.get(&dep.out_point) {
                    Self::add_edge(&mut edges, &mut reverse_edges, producer_id, consumer_id, DagEdge::ReadDep)?;
                }
            }
        }

        // Step 3: Compute topological layers
        let layers = Self::compute_layers(node_count, &edges, &reverse_edges)?;

        Ok(CellDAG { node_count, edges, reverse_edges, conflict_hash_conflicts: BTreeMap::new(), layers })
    }

    /// Build DAG from typed cell transactions with conflict_hash awareness.
    ///
    /// Extends the base `build` with typed-cell conflict rules:
    ///
    /// ```text
    /// READ  + READ  same conflict_hash → same layer (no edge)
    /// READ  + WRITE same conflict_hash → dependency edge
    /// WRITE + WRITE same conflict_hash → dependency edge (different layers)
    /// ```
    pub fn build_with_scheduler_plans(txs: &[CellTx], plans: &[SchedulerPlan]) -> Result<Self, DagError> {
        if txs.len() != plans.len() {
            return Err(DagError::SchedulerPlanCountMismatch { transactions: txs.len(), plans: plans.len() });
        }
        let mut dag = Self::build(txs)?;

        // Extract and validate conflict_hash accesses. Each transaction is
        // aggregated to at most one access per conflict domain, with Write
        // dominating Read. This prevents legitimate consume+create updates
        // from generating a self-edge.
        let mut conflict_hash_conflicts: BTreeMap<[u8; 32], Vec<ConflictEntry>> = BTreeMap::new();

        for (node_id, (tx, plan)) in txs.iter().zip(plans).enumerate() {
            let actual_txid = tx.id();
            if plan.txid != actual_txid {
                return Err(DagError::SchedulerPlanTxidMismatch { node_id, expected: actual_txid, actual: plan.txid });
            }
            for &(conflict_hash, mode) in &plan.accesses {
                conflict_hash_conflicts.entry(conflict_hash).or_default().push(ConflictEntry { node_id, mode });
            }
        }

        // Apply conflict rules: add dependency edges where needed
        for entries in conflict_hash_conflicts.values() {
            // For each pair of entries sharing the same conflict_hash,
            // add a dependency edge if at least one is a Write.
            for i in 0..entries.len() {
                for j in (i + 1)..entries.len() {
                    let a = &entries[i];
                    let b = &entries[j];

                    // READ + READ → no edge (same layer)
                    if a.mode == AccessMode::Read && b.mode == AccessMode::Read {
                        continue;
                    }

                    // READ + WRITE or WRITE + WRITE → dependency edge
                    // Earlier transaction must come first
                    let (from, to) = if a.node_id < b.node_id { (a.node_id, b.node_id) } else { (b.node_id, a.node_id) };

                    Self::add_edge(&mut dag.edges, &mut dag.reverse_edges, from, to, DagEdge::Dependency)?;
                }
            }
        }

        dag.conflict_hash_conflicts = conflict_hash_conflicts;

        // Recompute layers with the new edges
        dag.layers = Self::compute_layers(dag.node_count, &dag.edges, &dag.reverse_edges)?;

        Ok(dag)
    }

    fn add_edge(
        edges: &mut BTreeMap<NodeId, Vec<(NodeId, DagEdge)>>,
        reverse_edges: &mut BTreeMap<NodeId, Vec<NodeId>>,
        from: NodeId,
        to: NodeId,
        edge: DagEdge,
    ) -> Result<(), DagError> {
        if from == to {
            return Err(DagError::InvalidRWSet(format!("self-dependency for transaction node {from}")));
        }
        if edges.get(&from).is_some_and(|successors| successors.iter().any(|(successor, _)| *successor == to)) {
            return Ok(());
        }
        edges.entry(from).or_default().push((to, edge));
        reverse_edges.entry(to).or_default().push(from);
        Ok(())
    }

    /// Check if two transactions can be placed in the same layer
    /// based on their conflict_hash access patterns.
    ///
    /// Returns `true` if there is no Write-based conflict between them.
    pub fn can_parallel(accesses_a: &[([u8; 32], AccessMode)], accesses_b: &[([u8; 32], AccessMode)]) -> bool {
        // Build a map of conflict_hash → AccessMode for A
        let mut a_map: BTreeMap<[u8; 32], AccessMode> = BTreeMap::new();
        for &(hash, mode) in accesses_a {
            // If we've seen this hash before, upgrade to Write if either access is Write
            let entry = a_map.entry(hash).or_insert(AccessMode::Read);
            if mode == AccessMode::Write {
                *entry = AccessMode::Write;
            }
        }

        // Check for conflicts with B
        for &(hash, mode_b) in accesses_b {
            if let Some(mode_a) = a_map.get(&hash) {
                // READ + READ is fine; anything involving a Write creates a conflict
                if *mode_a == AccessMode::Write || mode_b == AccessMode::Write {
                    return false;
                }
            }
        }

        true
    }

    /// Compute topological layers for parallel execution
    ///
    /// Uses Kahn's algorithm with layer tracking:
    /// - Layer 0: nodes with no predecessors
    /// - Layer N: nodes whose all predecessors are in layers < N
    fn compute_layers(
        node_count: usize,
        edges: &BTreeMap<NodeId, Vec<(NodeId, DagEdge)>>,
        reverse_edges: &BTreeMap<NodeId, Vec<NodeId>>,
    ) -> Result<Vec<Vec<NodeId>>, DagError> {
        if node_count == 0 {
            return Ok(Vec::new());
        }

        let mut in_degree = vec![0usize; node_count];
        let mut layers = Vec::new();
        let mut current_layer = Vec::new();
        let mut processed = 0usize;

        // Compute in-degrees
        for (node, degree) in in_degree.iter_mut().enumerate().take(node_count) {
            *degree = reverse_edges.get(&node).map_or(0, |preds| preds.len());
            if *degree == 0 {
                current_layer.push(node);
            }
        }

        while !current_layer.is_empty() {
            current_layer.sort_unstable();
            layers.push(current_layer.clone());
            processed += current_layer.len();

            let mut next_layer = Vec::new();
            for node in current_layer {
                if let Some(successors) = edges.get(&node) {
                    for &(successor, _) in successors {
                        let degree = in_degree
                            .get_mut(successor)
                            .ok_or_else(|| DagError::InvalidRWSet(format!("successor node {successor} is out of bounds")))?;
                        if *degree == 0 {
                            return Err(DagError::InvalidRWSet(format!(
                                "successor node {successor} reached zero in-degree too early"
                            )));
                        }
                        *degree -= 1;
                        if *degree == 0 {
                            next_layer.push(successor);
                        }
                    }
                }
            }

            current_layer = next_layer;
        }

        if processed != node_count {
            return Err(DagError::CycleDetected);
        }

        Ok(layers)
    }

    /// Get successors of a node
    pub fn successors(&self, node: NodeId) -> Option<&[(NodeId, DagEdge)]> {
        self.edges.get(&node).map(|v| v.as_slice())
    }

    /// Get predecessors of a node
    pub fn predecessors(&self, node: NodeId) -> Option<&[NodeId]> {
        self.reverse_edges.get(&node).map(|v| v.as_slice())
    }

    /// Check if there's a dependency path from A to B
    pub fn has_path(&self, from: NodeId, to: NodeId) -> bool {
        if from == to {
            return true;
        }

        let mut visited = BTreeSet::new();
        let mut stack = vec![from];

        while let Some(node) = stack.pop() {
            if node == to {
                return true;
            }
            if visited.insert(node) {
                if let Some(succs) = self.edges.get(&node) {
                    for &(succ, _) in succs {
                        stack.push(succ);
                    }
                }
            }
        }

        false
    }
}

/// DAG node metadata
#[derive(Debug, Clone)]
pub struct DagNode {
    /// Node ID
    pub id: NodeId,
    /// Transaction
    pub tx: CellTx,
    /// Layer in topological sort
    pub layer: usize,
}

/// DAG construction errors
#[derive(Debug, Clone, thiserror::Error)]
pub enum DagError {
    /// Cycle detected in dependency graph
    #[error("Cycle detected in transaction DAG")]
    CycleDetected,

    /// Invalid RW-Set (missing declarations)
    #[error("Invalid RW-Set: {0}")]
    InvalidRWSet(String),

    /// Two transactions, or two inputs in one transaction, consume the same Cell.
    #[error("Cell {out_point:?} is consumed more than once by nodes {first_consumer} and {second_consumer}")]
    DoubleSpend {
        /// Conflicting physical Cell.
        out_point: OutPoint,
        /// First consumer node.
        first_consumer: NodeId,
        /// Second consumer node.
        second_consumer: NodeId,
    },

    /// The same raw transaction appears more than once in the batch.
    #[error("Raw transaction {txid:02x?} appears more than once at nodes {first_node} and {duplicate_node}")]
    DuplicateTransaction {
        /// Raw transaction hash.
        txid: [u8; 32],
        /// First node.
        first_node: NodeId,
        /// Duplicate node.
        duplicate_node: NodeId,
    },

    /// Number of scheduler plans does not match the transaction batch.
    #[error("Scheduler plan count mismatch: {transactions} transactions, {plans} plans")]
    SchedulerPlanCountMismatch {
        /// Transaction count.
        transactions: usize,
        /// Plan count.
        plans: usize,
    },

    /// A scheduler plan is bound to a different transaction.
    #[error("Scheduler plan on node {node_id} is bound to {actual:02x?}, expected {expected:02x?}")]
    SchedulerPlanTxidMismatch {
        /// Transaction node.
        node_id: NodeId,
        /// Actual raw transaction hash.
        expected: [u8; 32],
        /// Hash carried by the plan.
        actual: [u8; 32],
    },

    /// Scheduler metadata is malformed or not admitted.
    #[error("Invalid scheduler plan: {0}")]
    InvalidSchedulerPlan(String),
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::celltx::types::{CellInput, CellOutput, Script};
    use std::sync::atomic::{AtomicU64, Ordering};

    static NEXT_TEST_TX_TAG: AtomicU64 = AtomicU64::new(1);

    fn unique_test_lock() -> Script {
        Script::new([0x00; 32], 0, NEXT_TEST_TX_TAG.fetch_add(1, Ordering::Relaxed).to_le_bytes().to_vec())
    }

    fn create_test_tx(inputs: Vec<OutPoint>, outputs_count: usize) -> CellTx {
        let lock = unique_test_lock();
        let inputs = inputs.into_iter().map(|op| CellInput::new(op, 0)).collect();
        let outputs = vec![CellOutput { lock: lock.clone(), type_: None, capacity: 1000 }; outputs_count];
        let outputs_data = vec![vec![]; outputs_count];
        CellTx::new(inputs, vec![], outputs, outputs_data, vec![]).unwrap()
    }

    #[test]
    fn test_dag_simple_chain() {
        // tx0 → tx1 → tx2 (simple chain)
        let tx0 = create_test_tx(vec![], 1);
        let tx0_hash = tx0.id();

        let tx1 = create_test_tx(vec![OutPoint::new(tx0_hash, 0)], 1);
        let tx1_hash = tx1.id();

        let tx2 = create_test_tx(vec![OutPoint::new(tx1_hash, 0)], 1);

        let dag = CellDAG::build(&[tx0, tx1, tx2]).unwrap();

        assert_eq!(dag.node_count, 3);
        assert_eq!(dag.layers, vec![vec![0], vec![1], vec![2]]);
        assert!(dag.has_path(0, 2));
        assert!(!dag.has_path(2, 0));
    }

    #[test]
    fn test_dag_conflict_detection() {
        // tx0 produces Cell
        // tx1 and tx2 both try to consume it (conflict)
        let tx0 = create_test_tx(vec![], 1);
        let tx0_hash = tx0.id();
        let out = OutPoint::new(tx0_hash, 0);

        let tx1 = create_test_tx(vec![out], 1);
        let tx2 = create_test_tx(vec![out], 1);

        let error = CellDAG::build(&[tx0, tx1, tx2]).unwrap_err();
        assert!(matches!(error, DagError::DoubleSpend { first_consumer: 1, second_consumer: 2, .. }));
    }

    #[test]
    fn test_dag_parallel_branches() {
        // tx0 produces 2 outputs
        // tx1 consumes output 0
        // tx2 consumes output 1
        // (parallel, no conflict)
        let tx0 = create_test_tx(vec![], 2);
        let tx0_hash = tx0.id();

        let tx1 = create_test_tx(vec![OutPoint::new(tx0_hash, 0)], 1);
        let tx2 = create_test_tx(vec![OutPoint::new(tx0_hash, 1)], 1);

        let dag = CellDAG::build(&[tx0, tx1, tx2]).unwrap();

        assert_eq!(dag.layers, vec![vec![0], vec![1, 2]]);

        // Both tx1 and tx2 depend on tx0
        assert!(dag.has_path(0, 1));
        assert!(dag.has_path(0, 2));
        assert!(!dag.has_path(1, 2)); // tx1 and tx2 are independent
    }

    #[test]
    fn test_compute_layers_detects_cycle() {
        let mut edges = BTreeMap::new();
        let mut reverse_edges = BTreeMap::new();

        edges.insert(0, vec![(1, DagEdge::Dependency)]);
        edges.insert(1, vec![(0, DagEdge::Dependency)]);
        reverse_edges.insert(0, vec![1]);
        reverse_edges.insert(1, vec![0]);

        let result = CellDAG::compute_layers(2, &edges, &reverse_edges);
        assert!(matches!(result, Err(DagError::CycleDetected)));
    }

    #[test]
    fn test_compute_layers_empty_dag() {
        let edges = BTreeMap::new();
        let reverse_edges = BTreeMap::new();

        let layers = CellDAG::compute_layers(0, &edges, &reverse_edges).unwrap();
        assert!(layers.is_empty());
    }

    // ─── Typed Cell Conflict Hash Tests ─────────────────────────────────────────

    fn create_typed_test_tx(accesses: Vec<([u8; 32], AccessMode)>) -> (CellTx, SchedulerPlan) {
        let lock = unique_test_lock();
        let tx = CellTx::new(vec![], vec![], vec![CellOutput { lock, type_: None, capacity: 1000 }], vec![vec![]], vec![]).unwrap();
        let plan = SchedulerPlan::new(&tx, accesses).unwrap();
        (tx, plan)
    }

    fn build_typed(items: Vec<(CellTx, SchedulerPlan)>) -> CellDAG {
        let (txs, plans): (Vec<_>, Vec<_>) = items.into_iter().unzip();
        CellDAG::build_with_scheduler_plans(&txs, &plans).unwrap()
    }

    #[test]
    fn typed_conflict_rules_are_applied_to_authenticated_plans() {
        let shared = [0xAA; 32];
        let other = [0xBB; 32];

        let dag = build_typed(vec![
            create_typed_test_tx(vec![(shared, AccessMode::Write)]),
            create_typed_test_tx(vec![(other, AccessMode::Write)]),
            create_typed_test_tx(vec![(shared, AccessMode::Read)]),
            create_typed_test_tx(vec![(shared, AccessMode::Write)]),
        ]);

        assert!(!dag.has_path(0, 1), "different conflict domains remain parallel");
        assert!(dag.has_path(0, 2), "write precedes read on the same domain");
        assert!(dag.has_path(2, 3), "read precedes a later write on the same domain");
        assert!(dag.has_path(0, 3), "write ordering is transitive");
    }

    #[test]
    fn read_read_is_parallel_and_write_conflicts_are_ordered() {
        let key = [0xCC; 32];
        let read_dag = build_typed(vec![
            create_typed_test_tx(vec![(key, AccessMode::Read)]),
            create_typed_test_tx(vec![(key, AccessMode::Read)]),
        ]);
        assert_eq!(read_dag.layers.len(), 1);

        let write_dag = build_typed(vec![
            create_typed_test_tx(vec![(key, AccessMode::Read)]),
            create_typed_test_tx(vec![(key, AccessMode::Write)]),
        ]);
        assert!(write_dag.has_path(0, 1));
        assert_eq!(write_dag.layers.len(), 2);
    }

    #[test]
    fn same_transaction_accesses_are_aggregated_without_self_edge() {
        let key = [0xDD; 32];
        let (tx, plan) = create_typed_test_tx(vec![(key, AccessMode::Read), (key, AccessMode::Write)]);
        assert_eq!(plan.accesses(), &[(key, AccessMode::Write)]);
        let dag = CellDAG::build_with_scheduler_plans(&[tx], &[plan]).unwrap();
        assert_eq!(dag.layers, vec![vec![0]]);
        assert!(dag.edges.is_empty());
    }

    #[test]
    fn scheduler_plan_is_bound_to_raw_transaction_hash() {
        let key = [0xEE; 32];
        let (tx_a, plan_a) = create_typed_test_tx(vec![(key, AccessMode::Write)]);
        let tx_b = create_test_tx(vec![OutPoint::new([9; 32], 0)], 1);
        let error = CellDAG::build_with_scheduler_plans(&[tx_b], &[plan_a]).unwrap_err();
        assert!(matches!(error, DagError::SchedulerPlanTxidMismatch { node_id: 0, .. }));
        assert_ne!(tx_a.id(), [0; 32]);
    }

    #[test]
    fn can_parallel_uses_write_dominance() {
        let shared = [0x11; 32];
        let other = [0x22; 32];
        assert!(CellDAG::can_parallel(&[(shared, AccessMode::Read)], &[(shared, AccessMode::Read)]));
        assert!(!CellDAG::can_parallel(&[(shared, AccessMode::Read)], &[(shared, AccessMode::Write)]));
        assert!(!CellDAG::can_parallel(&[(shared, AccessMode::Write)], &[(shared, AccessMode::Write)]));
        assert!(CellDAG::can_parallel(&[(shared, AccessMode::Write)], &[(other, AccessMode::Write)]));
    }
}
