// SPDX-License-Identifier: MIT
// Copyright (C) 2026 Myelin developers

//! Bounded deterministic reexecution from retained state checkpoints.

use crate::recovery::MeteredStore;
use crate::{
    audit_record, genesis_head, make_outbox_message, ApplicationProfile, FinalityVerifier, Hash32, OutboxMessage, Result,
    SessionError, SessionHead, SessionStore, TransitionExecutor,
};
use crate::{
    FinalisedBlockRecord, RecoveryBudget, RecoveryCancellation, RecoveryPauseReason, RecoveryProgress, RecoveryUsage, SessionGenesis,
    StoreError,
};
use serde::{Deserialize, Serialize};
use std::{fmt, ops::RangeInclusive, sync::Arc};

const RANGE_REPLAY_DOMAIN: &[u8] = b"myelin:range-replay-receipt";

/// Verifiable summary of exact deterministic reexecution over an inclusive
/// block range. The checkpoint may precede the requested range; warm-up blocks
/// are verified but excluded from the reported resource totals.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct RangeReplayReceipt {
    pub session_id: Hash32,
    pub application_profile_commitment: Hash32,
    pub checkpoint_height: Option<u64>,
    pub checkpoint_block_hash: Hash32,
    pub checkpoint_state_root: Hash32,
    pub checkpoint_snapshot_hash: Hash32,
    pub start_height: u64,
    pub end_height: u64,
    pub state_root_before: Hash32,
    pub state_root_after: Hash32,
    pub first_frame_commitment: Hash32,
    pub last_frame_commitment: Hash32,
    pub frame_count: u64,
    pub total_cycles: u64,
    pub total_transaction_bytes: u64,
    pub total_input_bytes: u64,
}

impl RangeReplayReceipt {
    pub fn commitment(&self) -> Hash32 {
        let mut hasher = blake3::Hasher::new();
        hasher.update(RANGE_REPLAY_DOMAIN);
        hasher.update(&self.session_id);
        hasher.update(&self.application_profile_commitment);
        hasher.update(&self.checkpoint_height.unwrap_or(u64::MAX).to_le_bytes());
        hasher.update(&self.checkpoint_block_hash);
        hasher.update(&self.checkpoint_state_root);
        hasher.update(&self.checkpoint_snapshot_hash);
        hasher.update(&self.start_height.to_le_bytes());
        hasher.update(&self.end_height.to_le_bytes());
        hasher.update(&self.state_root_before);
        hasher.update(&self.state_root_after);
        hasher.update(&self.first_frame_commitment);
        hasher.update(&self.last_frame_commitment);
        hasher.update(&self.frame_count.to_le_bytes());
        hasher.update(&self.total_cycles.to_le_bytes());
        hasher.update(&self.total_transaction_bytes.to_le_bytes());
        hasher.update(&self.total_input_bytes.to_le_bytes());
        *hasher.finalize().as_bytes()
    }
}

/// Independently trusted checkpoint identity. Supplying one explicitly skips
/// application execution before it; merely reading these fields from storage
/// does not establish that trust. None means execute from genesis.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ReplayAnchor {
    pub genesis_commitment: Hash32,
    pub height: u64,
    pub block_hash: Hash32,
    pub state_root: Hash32,
    pub snapshot_hash: Hash32,
}

pub enum ReplayOutcome {
    Complete { receipt: Box<RangeReplayReceipt>, usage: RecoveryUsage },
    Paused(Box<PausedReplay>),
}

/// Process-local execution state. No serialized progress grants replay trust.
pub struct PausedReplay {
    reason: RecoveryPauseReason,
    progress: RecoveryProgress,
    replay: RangeReplay,
}
impl PausedReplay {
    pub fn reason(&self) -> RecoveryPauseReason {
        self.reason
    }
    pub fn progress(&self) -> &RecoveryProgress {
        &self.progress
    }
    pub fn resume(self, budget: RecoveryBudget, cancellation: &RecoveryCancellation) -> Result<ReplayOutcome> {
        self.replay.advance(budget, cancellation)
    }
}
impl fmt::Debug for PausedReplay {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("PausedReplay").field("reason", &self.reason).field("progress", &self.progress).finish_non_exhaustive()
    }
}
impl fmt::Display for PausedReplay {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{:?} before height {}; resume this cursor to continue execution", self.reason, self.progress.next_height)
    }
}

/// Budgeted application reexecution. The restore callback must remain available
/// if initialization pauses; the exact executor/store/verifier survive resumes.
pub struct RangeReplay(Box<dyn ReplayTask>);
trait ReplayTask: Send + Sync {
    fn advance(self: Box<Self>, budget: RecoveryBudget, cancellation: &RecoveryCancellation) -> Result<ReplayOutcome>;
}
impl RangeReplay {
    pub fn new<E, F>(
        store: Arc<dyn SessionStore>,
        verifier: Arc<dyn FinalityVerifier>,
        session_id: Hash32,
        expected_profile: ApplicationProfile,
        range: RangeInclusive<u64>,
        anchor: Option<ReplayAnchor>,
        restore: F,
    ) -> Self
    where
        E: TransitionExecutor,
        F: FnOnce(&[u8]) -> Result<E> + Send + Sync + 'static,
    {
        Self(Box::new(ReplayWork {
            store,
            verifier,
            session_id,
            expected_profile,
            range,
            anchor,
            restore: Some(restore),
            state: None,
            total: RecoveryUsage::default(),
        }))
    }
    pub fn advance(self, budget: RecoveryBudget, cancellation: &RecoveryCancellation) -> Result<ReplayOutcome> {
        self.0.advance(budget, cancellation)
    }
}
struct ReplayState<E> {
    genesis: SessionGenesis,
    captured: SessionHead,
    head: SessionHead,
    executor: E,
    receipt: RangeReplayReceipt,
}
struct ReplayWork<E, F> {
    store: Arc<dyn SessionStore>,
    verifier: Arc<dyn FinalityVerifier>,
    session_id: Hash32,
    expected_profile: ApplicationProfile,
    range: RangeInclusive<u64>,
    anchor: Option<ReplayAnchor>,
    restore: Option<F>,
    state: Option<ReplayState<E>>,
    total: RecoveryUsage,
}
impl<E, F> ReplayTask for ReplayWork<E, F>
where
    E: TransitionExecutor,
    F: FnOnce(&[u8]) -> Result<E> + Send + Sync + 'static,
{
    fn advance(mut self: Box<Self>, budget: RecoveryBudget, cancellation: &RecoveryCancellation) -> Result<ReplayOutcome> {
        let store = MeteredStore::new(self.store.clone(), budget, cancellation);
        let result = self.run(&store);
        let step = store.usage();
        self.total.add(step)?;
        match result {
            Ok(receipt) => Ok(ReplayOutcome::Complete { receipt: Box::new(receipt), usage: self.total }),
            Err(SessionError::Store(StoreError::ReadBudget(reason))) => {
                let progress = RecoveryProgress {
                    session_id: self.session_id,
                    captured_head: self.state.as_ref().map(|s| s.captured.block_hash),
                    target_height: Some(*self.range.end()),
                    next_height: self
                        .state
                        .as_ref()
                        .map(|s| s.head.next_height())
                        .transpose()?
                        .unwrap_or(self.anchor.as_ref().map(|a| a.height.saturating_add(1)).unwrap_or(0)),
                    step,
                    total: self.total,
                };
                Ok(ReplayOutcome::Paused(Box::new(PausedReplay { reason, progress, replay: RangeReplay(self) })))
            }
            Err(error) => Err(error),
        }
    }
}
impl<E, F> ReplayWork<E, F>
where
    E: TransitionExecutor,
    F: FnOnce(&[u8]) -> Result<E> + Send + Sync + 'static,
{
    fn run(&mut self, store: &MeteredStore) -> Result<RangeReplayReceipt> {
        store.check()?;
        if self.state.is_none() {
            let (genesis, captured, head, snapshot, receipt) =
                initialise(store, self.verifier.as_ref(), self.session_id, &self.expected_profile, &self.range, self.anchor.as_ref())?;
            let executor = self.restore.take().expect("restore is consumed only on successful initialization")(&snapshot)?;
            if executor.state_root() != head.state_root {
                return Err(SessionError::Replay("restored checkpoint state root mismatch".into()));
            }
            self.state = Some(ReplayState { genesis, captured, head, executor, receipt });
        }
        let state = self.state.as_mut().expect("initialized above");
        if store.load_head(self.session_id)? != state.captured {
            return Err(SessionError::Replay("captured head changed during replay; start a new replay".into()));
        }
        while state.head.finalised_height.is_none_or(|h| h < *self.range.end()) {
            store.check_block()?;
            let height = state.head.next_height()?;
            let record = store.load_block(self.session_id, height)?;
            let mut executor = state.executor.clone();
            let mut head = state.head.clone();
            execute_record(store, self.verifier.as_ref(), &state.genesis, &mut head, &mut executor, &record)?;
            // A dependency read can exhaust its budget after execution. Keep the
            // previous executor until every check on this block has completed.
            store.check()?;
            let mut receipt = state.receipt.clone();
            if height >= *self.range.start() {
                if height == *self.range.start() {
                    receipt.state_root_before = state.head.state_root;
                    receipt.first_frame_commitment = record.frame.commitment();
                }
                receipt.last_frame_commitment = record.frame.commitment();
                receipt.frame_count =
                    receipt.frame_count.checked_add(1).ok_or_else(|| SessionError::Replay("frame total overflow".into()))?;
                receipt.total_cycles = receipt
                    .total_cycles
                    .checked_add(record.frame.resources.cycles)
                    .ok_or_else(|| SessionError::Replay("cycle total overflow".into()))?;
                receipt.total_transaction_bytes = receipt
                    .total_transaction_bytes
                    .checked_add(record.frame.resources.transaction_bytes)
                    .ok_or_else(|| SessionError::Replay("transaction byte total overflow".into()))?;
                receipt.total_input_bytes = receipt
                    .total_input_bytes
                    .checked_add(record.frame.resources.input_bytes)
                    .ok_or_else(|| SessionError::Replay("input byte total overflow".into()))?;
            }
            receipt.state_root_after = head.state_root;
            state.receipt = receipt;
            state.head = head;
            state.executor = executor;
            store.completed_block();
        }
        if store.load_head(self.session_id)? != state.captured {
            return Err(SessionError::Replay("captured head changed during replay; start a new replay".into()));
        }
        if state.head.finalised_height == state.captured.finalised_height && state.head != state.captured {
            return Err(SessionError::Replay("reexecuted head does not match the captured durable head".into()));
        }
        store.check()?;
        Ok(state.receipt.clone())
    }
}

type InitialReplay = (SessionGenesis, SessionHead, SessionHead, Vec<u8>, RangeReplayReceipt);
fn initialise(
    store: &MeteredStore,
    verifier: &dyn FinalityVerifier,
    session_id: Hash32,
    profile: &ApplicationProfile,
    range: &RangeInclusive<u64>,
    anchor: Option<&ReplayAnchor>,
) -> Result<InitialReplay> {
    if range.is_empty() {
        return Err(SessionError::Replay("range start exceeds range end".into()));
    }
    profile.validate()?;
    let genesis = store.load_genesis(session_id)?;
    genesis.config.validate()?;
    if genesis.config.session_id != session_id || &genesis.config.application_profile != profile {
        return Err(SessionError::Replay("local application profile/session does not match session genesis".into()));
    }
    crate::ensure_consensus_config(verifier, &genesis.config)?;
    let captured = store.load_head(session_id)?;
    if captured.finalised_height.is_none_or(|height| *range.end() > height) {
        return Err(SessionError::Replay("requested range extends beyond the finalised head".into()));
    }
    let (head, snapshot) = if let Some(anchor) = anchor {
        if anchor.genesis_commitment != genesis.config.commitment() || anchor.height >= *range.start() {
            return Err(SessionError::Replay("trusted anchor must bind this genesis and precede the requested range".into()));
        }
        let record = store.load_block(session_id, anchor.height)?;
        record.validate_resources(&genesis.config)?;
        record.frame.validate(profile, &record.frame_input)?;
        let verified = verifier.verify(&record.block, &record.proof)?;
        if record.block.hash() != anchor.block_hash
            || record.block.state_root_after != anchor.state_root
            || record.block.session_id != session_id
            || record.block.number != anchor.height
            || verified.block_hash != anchor.block_hash
            || verified.consensus_module_commitment != genesis.config.consensus_module_commitment
            || record.consensus_module_commitment != genesis.config.consensus_module_commitment
        {
            return Err(SessionError::Replay("trusted checkpoint identity/finality mismatch".into()));
        }
        let snapshot = record.state_snapshot;
        if snapshot.is_empty() || *blake3::hash(&snapshot).as_bytes() != anchor.snapshot_hash {
            return Err(SessionError::Replay("trusted checkpoint snapshot is missing or changed".into()));
        }
        (
            SessionHead {
                session_id,
                finalised_height: Some(anchor.height),
                block_hash: anchor.block_hash,
                state_root: anchor.state_root,
                timestamp_ms: record.block.timestamp_ms,
                input_position: record.frame.input_end,
                logical_time: record.frame.logical_time_end,
                sealed_by_successor: record.successor.as_ref().map(|s| s.target_config.session_id),
            },
            snapshot,
        )
    } else {
        (genesis_head(&genesis), genesis.state_snapshot.clone())
    };
    genesis.config.commit_limits.snapshot(&snapshot)?;
    let receipt = RangeReplayReceipt {
        session_id,
        application_profile_commitment: profile.commitment(),
        checkpoint_height: head.finalised_height,
        checkpoint_block_hash: head.block_hash,
        checkpoint_state_root: head.state_root,
        checkpoint_snapshot_hash: *blake3::hash(&snapshot).as_bytes(),
        start_height: *range.start(),
        end_height: *range.end(),
        state_root_before: [0; 32],
        state_root_after: head.state_root,
        first_frame_commitment: [0; 32],
        last_frame_commitment: [0; 32],
        frame_count: 0,
        total_cycles: 0,
        total_transaction_bytes: 0,
        total_input_bytes: 0,
    };
    Ok((genesis, captured, head, snapshot, receipt))
}
fn execute_record<E: TransitionExecutor>(
    store: &MeteredStore,
    verifier: &dyn FinalityVerifier,
    genesis: &SessionGenesis,
    replay_head: &mut SessionHead,
    executor: &mut E,
    record: &FinalisedBlockRecord,
) -> Result<()> {
    let session_id = genesis.config.session_id;
    let height = record.block.number;
    let pre_root = replay_head.state_root;
    audit_record(genesis, verifier, replay_head, record)?;
    if executor.state_root() != pre_root {
        return Err(SessionError::Replay(format!("executor pre-root mismatch at height {height}")));
    }
    let mut handoff_usage = genesis
        .config
        .commit_limits
        .handoffs(record.issued_handoffs.iter().chain(record.handoff_actions.iter().map(|a| &a.handoff)))?;
    let replay_handoffs = record
        .consumed_handoff_ids
        .iter()
        .map(|handoff_id| {
            let stored = store
                .load_handoff(*handoff_id)?
                .ok_or_else(|| SessionError::Replay(format!("consumed handoff is absent at height {height}")))?;
            {
                let limits = &genesis.config.commit_limits;
                handoff_usage.add(limits, stored.handoff.payload.len(), stored.handoff.authorization.len())?;
            }
            if stored.consumption().map(|at| at.session_id) != Some(session_id)
                || stored.consumption().map(|at| at.height) != Some(height)
                || stored.consumption().map(|at| at.block_hash) != Some(record.block.hash())
            {
                return Err(SessionError::Replay(format!("handoff consumption marker mismatch at height {height}")));
            }
            let source = store.load_genesis(stored.handoff.source_session_id)?;
            crate::validate_handoff_trust(&genesis.config, &source.config)?;
            let source_record = store.load_block(stored.handoff.source_session_id, stored.handoff.source_height)?;
            crate::validate_handoff_source(&source.config, &source_record, &stored.handoff)?;
            Ok(stored.handoff)
        })
        .collect::<Result<Vec<_>>>()?;
    crate::lifecycle::audit_actions(store, record)?;
    let effects_context = record.effects_context(&genesis.config)?;
    let outcome = executor
        .execute_block_with_lifecycle(
            &effects_context,
            &record.handoff_actions,
            &record.frame_input,
            &replay_handoffs,
            &record.transactions,
        )
        .map_err(|error| SessionError::Replay(format!("execution failed at height {height}: {error}")))?;
    {
        let limits = &genesis.config.commit_limits;
        limits.outcome_parts(
            record.request_data_commitments.len() + usize::from(!record.handoff_actions.is_empty()),
            record.issued_handoffs.len(),
            record.consumed_handoff_ids.len(),
            record.successor.is_some(),
            true,
            &outcome,
        )?;
    }
    if outcome.ordered_cell_tx_commitments != record.block.ordered_cell_tx_commitments
        || outcome.cycles != record.frame.resources.cycles
        || executor.state_root() != record.block.state_root_after
    {
        return Err(SessionError::Replay(format!("execution result mismatch at height {height}")));
    }
    let replay_outbox = outcome
        .outbox
        .into_iter()
        .enumerate()
        .map(|(index, message)| make_outbox_message(session_id, record.consensus_module_commitment, height, index, message))
        .collect::<Result<Vec<_>>>()?;
    let mut replay_data_commitments = record.request_data_commitments.clone();
    replay_data_commitments.extend(outcome.data_commitments);
    replay_data_commitments.extend(replay_outbox.iter().map(OutboxMessage::commitment));
    replay_data_commitments.extend(record.issued_handoffs.iter().map(|handoff| handoff.id()));
    replay_data_commitments.extend(
        record
            .consumed_handoff_ids
            .iter()
            .map(|handoff_id| crate::continuity::handoff_consumption_commitment(session_id, height, *handoff_id)),
    );
    if let Some(successor) = &record.successor {
        {
            let expected = &successor.transition;
            let snapshot = executor.snapshot().map_err(SessionError::Replay)?;
            let reproduced = crate::SuccessorTransition::execute(
                executor,
                &genesis.config,
                &successor.target_config,
                &record.frame_input,
                &snapshot,
            )?;
            if &reproduced != expected {
                return Err(SessionError::Replay(format!("successor transition mismatch at height {height}")));
            }
            let target = store.load_genesis(successor.target_config.session_id)?;
            if target.config != successor.target_config || target.state_snapshot != reproduced.target_snapshot(&snapshot) {
                return Err(SessionError::Replay(format!("successor genesis mismatch at height {height}")));
            }
        }
        replay_data_commitments.push(successor.commitment());
    }
    if !record.handoff_actions.is_empty() {
        replay_data_commitments.push(crate::lifecycle::actions_commitment(session_id, height, &record.handoff_actions)?);
    }
    replay_data_commitments.push(effects_context.commitment()?);
    {
        let bytes = record.resource_snapshot_bytes;
        replay_data_commitments.push(crate::resources::snapshot_commitment(&genesis.config.commit_limits, bytes));
    }
    if replay_outbox != record.outbox || replay_data_commitments != record.block.data_commitments {
        return Err(SessionError::Replay(format!("data or outbox result mismatch at height {height}")));
    }
    {
        let reproduced = executor.snapshot().map_err(SessionError::Replay)?;
        {
            let limits = &genesis.config.commit_limits;
            limits.snapshot(&reproduced)?;
            if record.resource_snapshot_bytes != reproduced.len() as u64 {
                return Err(SessionError::Replay(format!("snapshot resource usage mismatch at height {height}")));
            }
        }
        if !record.state_snapshot.is_empty() && reproduced != record.state_snapshot {
            return Err(SessionError::Replay(format!("archival snapshot bytes mismatch at height {height}")));
        }
    }
    Ok(())
}
