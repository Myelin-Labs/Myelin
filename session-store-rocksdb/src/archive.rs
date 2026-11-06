//! Exact record relocation. Cold references are durable retention pins, never
//! permission to forget a block, command identity or scarce-claim terminal state.
use super::*;
use myelin_session::{HandoffRetentionPolicy, HandoffTerminal};
use std::ops::RangeInclusive;

const MAGIC: &[u8; 5] = b"MARC\x01";
const REFERENCE_BYTES: usize = 77;
const RECORD_RESERVATION: u64 = myelin_session::bounded_codec::MAX_RECORD_BYTES as u64;

/// A provider must bound reads to `expected_bytes` before allocation. The store
/// independently checks the returned length and digest on every use. Implementors
/// retain the referenced manifest and its complete dependency/artifact closure.
pub trait ArchiveRecordSource: Send + Sync {
    fn read_record(&self, archive: Hash32, blob: Hash32, expected_bytes: u64) -> Result<Vec<u8>, StoreError>;
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct ArchiveReference {
    pub archive: Hash32,
    pub blob: Hash32,
    pub bytes: u64,
}
impl ArchiveReference {
    fn validate(&self) -> Result<(), StoreError> {
        if self.archive == [0; 32]
            || self.blob == [0; 32]
            || self.bytes == 0
            || self.bytes > myelin_session::bounded_codec::MAX_RECORD_BYTES as u64
        {
            return Err(StoreError::Corrupt("invalid cold block reference".into()));
        }
        Ok(())
    }
    fn encode(&self) -> Result<Vec<u8>, StoreError> {
        self.validate()?;
        let mut bytes = MAGIC.to_vec();
        bytes.extend(self.archive);
        bytes.extend(self.blob);
        bytes.extend(self.bytes.to_le_bytes());
        Ok(bytes)
    }
    pub(super) fn decode(bytes: &[u8]) -> Result<Self, StoreError> {
        if bytes.len() != REFERENCE_BYTES || !bytes.starts_with(MAGIC) {
            return Err(StoreError::Corrupt("malformed cold block reference".into()));
        }
        let value = Self {
            archive: bytes[5..37].try_into().unwrap(),
            blob: bytes[37..69].try_into().unwrap(),
            bytes: u64::from_le_bytes(bytes[69..77].try_into().unwrap()),
        };
        value.validate()?;
        Ok(value)
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ArchivedBlock {
    pub session_id: Hash32,
    pub height: u64,
    pub reference: ArchiveReference,
}

/// Explicit local hot-storage requirements. Dispute/recovery ranges are supplied
/// by the operator/application; wall time alone cannot discharge such a hold.
/// The current tip, pending deliveries and outstanding claims are also pinned.
#[derive(Clone, Debug)]
pub struct RetentionWindow {
    pub hot_from_height: u64,
    pub protected: Vec<RangeInclusive<u64>>,
}
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum RetentionHold {
    CurrentTip,
    RecoveryWindow,
    ProtectedRange,
    PendingDelivery,
    OutstandingHandoff,
}
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum OffloadOutcome {
    /// Logical bytes removed from the hot record key, before RocksDB compaction.
    Archived {
        hot_record_bytes_removed: u64,
        handoffs: HandoffCompaction,
    },
    AlreadyArchived {
        handoffs: HandoffCompaction,
    },
    /// The exact same block now pins a different verified archive manifest.
    Rebound {
        previous_archive: Hash32,
        handoffs: HandoffCompaction,
    },
    Held(RetentionHold),
}

fn cold_key(session: Hash32, height: u64) -> Vec<u8> {
    let mut key = session_key(KEY_COLD_BLOCK, session);
    key.extend(height.to_be_bytes());
    key
}
impl RocksSessionStore {
    pub(super) fn resolve_block_bytes(&self, hot: Option<Vec<u8>>, cold: Option<Vec<u8>>, height: u64) -> Result<Vec<u8>, StoreError> {
        match (hot, cold) {
            (Some(bytes), None) => Ok(bytes),
            (None, Some(reference)) => self.read_archived(&ArchiveReference::decode(&reference)?),
            (None, None) => Err(StoreError::NotFound(format!("session block {height}"))),
            (Some(_), Some(_)) => Err(StoreError::Corrupt("block has both hot and cold authoritative entries".into())),
        }
    }
    fn read_archived(&self, reference: &ArchiveReference) -> Result<Vec<u8>, StoreError> {
        reference.validate()?;
        let source =
            self.archive.as_ref().ok_or_else(|| StoreError::NotFound("cold block requires its configured archive source".into()))?;
        let bytes = source.read_record(reference.archive, reference.blob, reference.bytes)?;
        if bytes.len() as u64 != reference.bytes || blake3::hash(&bytes).as_bytes() != &reference.blob {
            return Err(StoreError::Corrupt("archived block length/digest mismatch".into()));
        }
        Ok(bytes)
    }
    pub(super) fn block_bytes(&self, session: Hash32, height: u64) -> Result<Vec<u8>, StoreError> {
        // One RocksDB snapshot prevents relocation between the two key reads
        // from looking like a gap or overlapping authoritative records.
        let snapshot = self.db.snapshot();
        self.resolve_block_bytes(
            snapshot.get(block_key(session, height)).map_err(backend)?,
            snapshot.get(cold_key(session, height)).map_err(backend)?,
            height,
        )
    }
    pub(super) fn transaction_block_bytes(
        &self,
        transaction: &rocksdb::Transaction<'_, OptimisticTransactionDB>,
        session: Hash32,
        height: u64,
    ) -> Result<Vec<u8>, StoreError> {
        let snapshot = transaction.snapshot();
        self.resolve_block_bytes(
            snapshot.get(block_key(session, height)).map_err(backend)?,
            snapshot.get(cold_key(session, height)).map_err(backend)?,
            height,
        )
    }
    pub(super) fn cold_block_exists(
        &self,
        transaction: &rocksdb::Transaction<'_, OptimisticTransactionDB>,
        session: Hash32,
        height: u64,
    ) -> Result<bool, StoreError> {
        Ok(transaction.get_for_update(cold_key(session, height), true).map_err(backend)?.is_some())
    }

    /// Relocate one block only after reading the exact archived bytes. The block
    /// key and its durable archive pin change atomically. All identities, heads,
    /// checkpoints, handoff states, command receipts and evidence remain intact.
    /// A stale head or failed read/policy check leaves every key unchanged.
    /// `check` runs before each opaque record read (reserving 128 MiB), each
    /// archive read (its exact length), and commit (zero bytes). It may park or
    /// abort. Budgets are logical reservations, not physical I/O measurements.
    /// Existing cold blocks may rebind only their manifest, preserving digest
    /// and length. This also repairs a missing old archive using identical data.
    pub fn offload_block(
        &self,
        expected_head: &SessionHead,
        height: u64,
        reference: ArchiveReference,
        window: &RetentionWindow,
        policy: &dyn HandoffRetentionPolicy,
        check: &dyn Fn(u64) -> Result<(), StoreError>,
    ) -> Result<OffloadOutcome, StoreError> {
        reference.validate()?;
        if window.protected.len() > 256 || window.protected.iter().any(|r| r.is_empty()) {
            return Err(StoreError::Corrupt("invalid or excessive retention hold ranges".into()));
        }
        let session = expected_head.session_id;
        let transaction = self.sync_transaction();
        check(RECORD_RESERVATION)?;
        let current = transaction
            .get_for_update(session_key(KEY_HEAD, session), true)
            .map_err(backend)?
            .ok_or_else(|| StoreError::NotFound("session head".into()))?;
        if current != expected_head.encode().map_err(corrupt)? {
            return Err(StoreError::Conflict("head changed before archival relocation".into()));
        }
        let tip = expected_head.finalised_height.ok_or_else(|| StoreError::NotFound("session has no blocks to archive".into()))?;
        if height > tip {
            return Err(StoreError::NotFound("requested archive height is beyond the head".into()));
        }
        let key = block_key(session, height);
        let cold = cold_key(session, height);
        check(RECORD_RESERVATION * 2)?;
        let (hot, existing) = {
            let snapshot = transaction.snapshot();
            let mut reads = rocksdb::ReadOptions::default();
            reads.set_snapshot(&snapshot);
            (
                transaction.get_for_update_opt(&key, true, &reads).map_err(backend)?,
                transaction.get_for_update_opt(&cold, true, &reads).map_err(backend)?,
            )
        };
        let Some(bytes) = hot else {
            let old = ArchiveReference::decode(&existing.ok_or_else(|| StoreError::NotFound("block to archive".into()))?)?;
            if old.blob != reference.blob || old.bytes != reference.bytes {
                return Err(StoreError::Conflict("cold rebinding must preserve the exact block digest and length".into()));
            }
            check(reference.bytes)?;
            let archived = self.read_archived(&reference)?;
            check(RECORD_RESERVATION)?;
            let genesis = load_genesis_transaction(&transaction, session)?;
            let record = FinalisedBlockRecord::decode_for_config(&archived, &genesis.config).map_err(corrupt)?;
            if record.block.number != height || record.block.session_id != session {
                return Err(StoreError::Corrupt("cold rebind block key mismatch".into()));
            }
            let handoffs = self.compact_issued_handoffs(&transaction, &record, expected_head, window, check)?;
            if old == reference && handoffs.records == 0 {
                return Ok(OffloadOutcome::AlreadyArchived { handoffs });
            }
            check(0)?;
            transaction.put(&cold, reference.encode()?).map_err(backend)?;
            transaction.commit().map_err(commit_error)?;
            return Ok(if old == reference {
                OffloadOutcome::AlreadyArchived { handoffs }
            } else {
                OffloadOutcome::Rebound { previous_archive: old.archive, handoffs }
            });
        };
        if existing.is_some() {
            return Err(StoreError::Corrupt("block has overlapping hot/cold entries".into()));
        }
        if height == tip {
            return Ok(OffloadOutcome::Held(RetentionHold::CurrentTip));
        }
        if height >= window.hot_from_height {
            return Ok(OffloadOutcome::Held(RetentionHold::RecoveryWindow));
        }
        if window.protected.iter().any(|r| r.contains(&height)) {
            return Ok(OffloadOutcome::Held(RetentionHold::ProtectedRange));
        }
        check(RECORD_RESERVATION)?;
        let genesis = load_genesis_transaction(&transaction, session)?;
        let record = FinalisedBlockRecord::decode_for_config(&bytes, &genesis.config).map_err(corrupt)?;
        if record.block.number != height || record.block.session_id != session {
            return Err(StoreError::Corrupt("archival block key mismatch".into()));
        }
        for message in &record.outbox {
            check(RECORD_RESERVATION)?;
            if transaction.get_for_update(outbox_index_key(session, message.id), false).map_err(backend)?.is_some() {
                return Ok(OffloadOutcome::Held(RetentionHold::PendingDelivery));
            }
        }
        let handoffs: std::collections::BTreeSet<_> = record
            .issued_handoffs
            .iter()
            .map(|h| h.id())
            .chain(record.consumed_handoff_ids.iter().copied())
            .chain(record.handoff_actions.iter().map(|a| a.action.handoff_id()))
            .collect();
        for id in handoffs {
            let stored = self
                .transaction_handoff(&transaction, id, check)?
                .ok_or_else(|| StoreError::Corrupt("archive dependency handoff is absent".into()))?;
            let closed = match &stored.lifecycle.terminal {
                HandoffTerminal::Retired { .. } | HandoffTerminal::Reclaimed { .. } => true,
                HandoffTerminal::Consumed { .. } => {
                    check(RECORD_RESERVATION)?;
                    let source = load_genesis_transaction(&transaction, stored.handoff.source_session_id)?;
                    policy.is_copyable(&source.config, &stored.handoff).map_err(corrupt)?
                }
                HandoffTerminal::Issued | HandoffTerminal::Cancelled { .. } => false,
            };
            if !closed {
                return Ok(OffloadOutcome::Held(RetentionHold::OutstandingHandoff));
            }
        }
        check(reference.bytes)?;
        let archived = self.read_archived(&reference)?;
        if archived != bytes {
            return Err(StoreError::Corrupt("archive does not contain the exact retained block encoding".into()));
        }
        let handoffs = self.compact_issued_handoffs(&transaction, &record, expected_head, window, check)?;
        check(0)?;
        transaction.put(&cold, reference.encode()?).map_err(backend)?;
        transaction.delete(&key).map_err(backend)?;
        transaction.commit().map_err(commit_error)?;
        Ok(OffloadOutcome::Archived { hot_record_bytes_removed: bytes.len() as u64, handoffs })
    }

    /// Enumerate durable archive pins in bounded key order. Archive GC must use
    /// a consistent store capture (or a quiescent store), not treat a moving
    /// multi-page scan as a complete liveness set. Pins have no expiry/ACK API.
    pub fn archived_blocks(&self, after: Option<(Hash32, u64)>, limit: usize) -> Result<Vec<ArchivedBlock>, StoreError> {
        let limit = limit.min(1024);
        if limit == 0 {
            return Ok(vec![]);
        }
        let start = after.map(|(s, h)| cold_key(s, h)).unwrap_or_else(|| vec![KEY_COLD_BLOCK]);
        let mut found = Vec::with_capacity(limit);
        for item in self.db.iterator(IteratorMode::From(&start, Direction::Forward)) {
            let (key, bytes) = item.map_err(backend)?;
            if key.first() != Some(&KEY_COLD_BLOCK) || found.len() == limit {
                break;
            }
            if key.len() != 41 {
                return Err(StoreError::Corrupt("malformed archived block key".into()));
            }
            if after.is_some() && key.as_ref() == start.as_slice() {
                continue;
            }
            found.push(ArchivedBlock {
                session_id: key[1..33].try_into().unwrap(),
                height: u64::from_be_bytes(key[33..41].try_into().unwrap()),
                reference: ArchiveReference::decode(&bytes)?,
            });
        }
        Ok(found)
    }
}
