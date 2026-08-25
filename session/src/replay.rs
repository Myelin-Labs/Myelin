// SPDX-License-Identifier: MIT
// Copyright (C) 2026 Myelin developers

//! Bounded deterministic reexecution from retained state checkpoints.

use crate::{
    audit_record, genesis_head, make_outbox_message, ApplicationProfile, FinalityVerifier, Hash32, OutboxMessage, Result,
    SessionError, SessionHead, SessionStore, TransitionExecutor,
};
use serde::{Deserialize, Serialize};

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

pub(crate) fn replay_range<S, E, F>(
    store: &S,
    verifier: &dyn FinalityVerifier,
    session_id: Hash32,
    expected_profile: &ApplicationProfile,
    start_height: u64,
    end_height: u64,
    restore: F,
) -> Result<RangeReplayReceipt>
where
    S: SessionStore,
    E: TransitionExecutor,
    F: FnOnce(&[u8]) -> Result<E>,
{
    if start_height > end_height {
        return Err(SessionError::Replay("range start exceeds range end".to_owned()));
    }
    expected_profile.validate()?;
    let genesis = store.load_genesis(session_id)?;
    if &genesis.config.application_profile != expected_profile {
        return Err(SessionError::Replay("local application profile does not match session genesis".to_owned()));
    }
    let durable_head = store.load_head(session_id)?;
    if durable_head.finalised_height.is_none_or(|height| end_height > height) {
        return Err(SessionError::Replay("requested range extends beyond the finalised head".to_owned()));
    }
    let checkpoint = store.load_checkpoint_before(session_id, start_height)?;
    if checkpoint.session_id != session_id || checkpoint.state_snapshot.is_empty() {
        return Err(SessionError::Replay("retained checkpoint identity or snapshot is invalid".to_owned()));
    }
    let checkpoint_snapshot_hash = *blake3::hash(&checkpoint.state_snapshot).as_bytes();
    let (mut replay_head, checkpoint_block_hash) = match checkpoint.finalised_height {
        None => {
            if checkpoint.state_root != genesis.config.initial_state_root {
                return Err(SessionError::Replay("genesis checkpoint root mismatch".to_owned()));
            }
            (genesis_head(&genesis), [0; 32])
        }
        Some(height) => {
            if height >= start_height {
                return Err(SessionError::Replay("checkpoint is not before the requested range".to_owned()));
            }
            let record = store.load_block(session_id, height)?;
            if record.state_snapshot.is_empty()
                || record.state_snapshot != checkpoint.state_snapshot
                || record.block.state_root_after != checkpoint.state_root
            {
                return Err(SessionError::Replay("archival checkpoint does not match its finalised block".to_owned()));
            }
            record.frame.validate(expected_profile, &record.frame_input)?;
            let verified = verifier.verify(&record.block, &record.proof)?;
            if verified.block_hash != record.block.hash() || verified.consensus_module_commitment != record.consensus_module_commitment
            {
                return Err(SessionError::Replay("checkpoint finality proof does not match its block".to_owned()));
            }
            let block_hash = record.block.hash();
            (
                SessionHead {
                    session_id,
                    finalised_height: Some(height),
                    block_hash,
                    state_root: record.block.state_root_after,
                    timestamp_ms: record.block.timestamp_ms,
                    input_position: record.frame.input_end,
                    logical_time: record.frame.logical_time_end,
                    sealed_by_successor: record.successor.as_ref().map(|value| value.target_config.session_id),
                },
                block_hash,
            )
        }
    };
    let mut executor = restore(&checkpoint.state_snapshot)?;
    if executor.state_root() != checkpoint.state_root {
        return Err(SessionError::Replay("restored checkpoint state root mismatch".to_owned()));
    }

    let first_height = replay_head.next_height()?;
    let mut state_root_before = [0; 32];
    let mut first_frame_commitment = [0; 32];
    let mut last_frame_commitment = [0; 32];
    let mut total_cycles = 0u64;
    let mut total_transaction_bytes = 0u64;
    let mut total_input_bytes = 0u64;
    for height in first_height..=end_height {
        let record = store.load_block(session_id, height)?;
        let pre_root = replay_head.state_root;
        audit_record(&genesis, verifier, &mut replay_head, &record)?;
        if executor.state_root() != pre_root {
            return Err(SessionError::Replay(format!("executor pre-root mismatch at height {height}")));
        }
        let replay_handoffs = record
            .consumed_handoff_ids
            .iter()
            .map(|handoff_id| {
                let stored = store
                    .load_handoff(*handoff_id)?
                    .ok_or_else(|| SessionError::Replay(format!("consumed handoff is absent at height {height}")))?;
                if stored.consumed_by_session != Some(session_id)
                    || stored.consumed_at_height != Some(height)
                    || stored.consumed_by_block_hash != Some(record.block.hash())
                {
                    return Err(SessionError::Replay(format!("handoff consumption marker mismatch at height {height}")));
                }
                Ok(stored.handoff)
            })
            .collect::<Result<Vec<_>>>()?;
        let outcome = executor
            .execute_block(height, &record.frame_input, &replay_handoffs, &record.transactions)
            .map_err(|error| SessionError::Replay(format!("execution failed at height {height}: {error}")))?;
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
            replay_data_commitments.push(successor.commitment());
        }
        if replay_outbox != record.outbox || replay_data_commitments != record.block.data_commitments {
            return Err(SessionError::Replay(format!("data or outbox result mismatch at height {height}")));
        }
        if !record.state_snapshot.is_empty() {
            let reproduced = executor.snapshot().map_err(SessionError::Replay)?;
            if reproduced != record.state_snapshot {
                return Err(SessionError::Replay(format!("archival snapshot bytes mismatch at height {height}")));
            }
        }
        if height >= start_height {
            if height == start_height {
                state_root_before = pre_root;
                first_frame_commitment = record.frame.commitment();
            }
            last_frame_commitment = record.frame.commitment();
            total_cycles = total_cycles
                .checked_add(record.frame.resources.cycles)
                .ok_or_else(|| SessionError::Replay("cycle total overflow".to_owned()))?;
            total_transaction_bytes = total_transaction_bytes
                .checked_add(record.frame.resources.transaction_bytes)
                .ok_or_else(|| SessionError::Replay("transaction byte total overflow".to_owned()))?;
            total_input_bytes = total_input_bytes
                .checked_add(record.frame.resources.input_bytes)
                .ok_or_else(|| SessionError::Replay("input byte total overflow".to_owned()))?;
        }
    }
    Ok(RangeReplayReceipt {
        session_id,
        application_profile_commitment: expected_profile.commitment(),
        checkpoint_height: checkpoint.finalised_height,
        checkpoint_block_hash,
        checkpoint_state_root: checkpoint.state_root,
        checkpoint_snapshot_hash,
        start_height,
        end_height,
        state_root_before,
        state_root_after: executor.state_root(),
        first_frame_commitment,
        last_frame_commitment,
        frame_count: end_height - start_height + 1,
        total_cycles,
        total_transaction_bytes,
        total_input_bytes,
    })
}
