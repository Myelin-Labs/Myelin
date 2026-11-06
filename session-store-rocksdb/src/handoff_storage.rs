//! Terminal payload compaction retains a permanent lifecycle/nullifier key.
//! Its original payload is resolved only through the pinned issuing block.
use super::*;
use myelin_session::{HandoffBlockReference, HandoffLifecycle, HandoffTerminal};

const MAGIC: &[u8; 5] = b"MHCP\x01";
const MAX_COMPACT_BYTES: usize = 256;
const RECORD_RESERVATION: u64 = myelin_session::bounded_codec::MAX_RECORD_BYTES as u64;

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct HandoffCompaction {
    pub records: u64,
    /// Net logical bytes removed from the handoff index, after compact markers.
    /// Physical SST reclamation is separate.
    pub bytes_removed: u64,
}

pub(super) fn compact_key(id: Hash32) -> Vec<u8> {
    session_key(KEY_COMPACT_HANDOFF, id)
}

fn terminal(lifecycle: &HandoffLifecycle) -> bool {
    matches!(lifecycle.terminal, HandoffTerminal::Retired { .. } | HandoffTerminal::Reclaimed { .. })
}
fn encode(lifecycle: &HandoffLifecycle) -> Result<Vec<u8>, StoreError> {
    if !terminal(lifecycle) {
        return Err(StoreError::Corrupt("only retired/reclaimed handoffs may be compacted".into()));
    }
    let mut bytes = MAGIC.to_vec();
    bytes.extend(postcard::to_allocvec(lifecycle).map_err(corrupt)?);
    if bytes.len() > MAX_COMPACT_BYTES {
        return Err(StoreError::Corrupt("compact handoff exceeds fixed metadata bound".into()));
    }
    Ok(bytes)
}
fn decode(bytes: &[u8]) -> Result<HandoffLifecycle, StoreError> {
    if bytes.len() > MAX_COMPACT_BYTES || !bytes.starts_with(MAGIC) {
        return Err(StoreError::Corrupt("invalid compact handoff format".into()));
    }
    let value: HandoffLifecycle = postcard::from_bytes(&bytes[MAGIC.len()..]).map_err(corrupt)?;
    if encode(&value)? != bytes {
        return Err(StoreError::Corrupt("noncanonical compact handoff".into()));
    }
    Ok(value)
}
fn expand(id: Hash32, lifecycle: HandoffLifecycle, source: &FinalisedBlockRecord) -> Result<StoredHandoff, StoreError> {
    if lifecycle.issued_at != HandoffBlockReference::from_block(&source.block) {
        return Err(StoreError::Corrupt("compact handoff issuer block mismatch".into()));
    }
    let mut matching = source.issued_handoffs.iter().filter(|h| h.id() == id);
    let handoff = matching.next().ok_or_else(|| StoreError::Corrupt("compact handoff is absent from issuing block".into()))?;
    if matching.next().is_some() {
        return Err(StoreError::Corrupt("duplicate compact handoff issuance".into()));
    }
    let stored = StoredHandoff { handoff: handoff.clone(), lifecycle };
    stored.validate_lifecycle().map_err(corrupt)?;
    stored.handoff.validate(source.block.timestamp_ms).map_err(corrupt)?;
    Ok(stored)
}
pub(super) fn resolve(
    id: Hash32,
    full: Option<Vec<u8>>,
    compact: Option<Vec<u8>>,
    source: impl FnOnce(&HandoffBlockReference) -> Result<FinalisedBlockRecord, StoreError>,
) -> Result<Option<StoredHandoff>, StoreError> {
    let value = match (full, compact) {
        (Some(bytes), None) => StoredHandoff::decode(&bytes).map_err(corrupt)?,
        (None, Some(bytes)) => {
            let lifecycle = decode(&bytes)?;
            let record = source(&lifecycle.issued_at)?;
            expand(id, lifecycle, &record)?
        }
        (None, None) => return Ok(None),
        (Some(_), Some(_)) => return Err(StoreError::Corrupt("handoff has overlapping full/compact records".into())),
    };
    if value.handoff.id() != id {
        return Err(StoreError::Corrupt("handoff key/record identity mismatch".into()));
    }
    Ok(Some(value))
}

struct Entries {
    full: Option<Vec<u8>>,
    compact: Option<Vec<u8>>,
}
fn entries(
    transaction: &rocksdb::Transaction<'_, OptimisticTransactionDB>,
    id: Hash32,
    check: &dyn Fn(u64) -> Result<(), StoreError>,
) -> Result<Entries, StoreError> {
    check(RECORD_RESERVATION + MAX_COMPACT_BYTES as u64)?;
    let snapshot = transaction.snapshot();
    let mut reads = rocksdb::ReadOptions::default();
    reads.set_snapshot(&snapshot);
    Ok(Entries {
        full: transaction.get_for_update_opt(handoff_key(id), true, &reads).map_err(backend)?,
        compact: transaction.get_for_update_opt(compact_key(id), true, &reads).map_err(backend)?,
    })
}

impl RocksSessionStore {
    pub(super) fn transaction_handoff(
        &self,
        transaction: &rocksdb::Transaction<'_, OptimisticTransactionDB>,
        id: Hash32,
        check: &dyn Fn(u64) -> Result<(), StoreError>,
    ) -> Result<Option<StoredHandoff>, StoreError> {
        let entries = entries(transaction, id, check)?;
        resolve(id, entries.full, entries.compact, |at| {
            check(RECORD_RESERVATION)?;
            let genesis = load_genesis_transaction(transaction, at.session_id)?;
            check(RECORD_RESERVATION)?;
            let bytes = self.transaction_block_bytes(transaction, at.session_id, at.height)?;
            FinalisedBlockRecord::decode_for_config(&bytes, &genesis.config).map_err(corrupt)
        })
    }

    /// Called only after the exact cold issuing block has been validated. Uses
    /// the same transaction as relocation/rebinding, so no marker can become
    /// authoritative before its archive pin. Dispute/hot/outbox holds also apply
    /// when the block was already cold. Nonterminal and consumed claims keep
    /// their full entries, even when a copyable claim's source block is cold.
    pub(super) fn compact_issued_handoffs(
        &self,
        transaction: &rocksdb::Transaction<'_, OptimisticTransactionDB>,
        record: &FinalisedBlockRecord,
        expected_head: &SessionHead,
        window: &RetentionWindow,
        check: &dyn Fn(u64) -> Result<(), StoreError>,
    ) -> Result<HandoffCompaction, StoreError> {
        let mut outcome = HandoffCompaction::default();
        let height = record.block.number;
        if Some(height) == expected_head.finalised_height
            || height >= window.hot_from_height
            || window.protected.iter().any(|r| r.contains(&height))
        {
            return Ok(outcome);
        }
        for message in &record.outbox {
            check(RECORD_RESERVATION)?;
            if transaction.get_for_update(outbox_index_key(record.block.session_id, message.id), false).map_err(backend)?.is_some() {
                return Ok(outcome);
            }
        }
        let issuer = HandoffBlockReference::from_block(&record.block);
        for handoff in &record.issued_handoffs {
            let id = handoff.id();
            let entries = entries(transaction, id, check)?;
            match (entries.full, entries.compact) {
                (Some(bytes), None) => {
                    let stored = StoredHandoff::decode(&bytes).map_err(corrupt)?;
                    if stored.handoff != *handoff || stored.lifecycle.issued_at != issuer {
                        return Err(StoreError::Corrupt("handoff compaction differs from exact issuing block".into()));
                    }
                    if !terminal(&stored.lifecycle) {
                        continue;
                    }
                    let compact = encode(&stored.lifecycle)?;
                    let removed =
                        bytes.len().checked_sub(compact.len()).ok_or_else(|| StoreError::Corrupt("compact handoff grew".into()))?;
                    transaction.put(compact_key(id), compact).map_err(backend)?;
                    transaction.delete(handoff_key(id)).map_err(backend)?;
                    outcome.records += 1;
                    outcome.bytes_removed += removed as u64;
                }
                (None, Some(bytes)) => {
                    let lifecycle = decode(&bytes)?;
                    if lifecycle.issued_at != issuer {
                        return Err(StoreError::Corrupt("compact handoff issuer block mismatch".into()));
                    }
                    // The enclosing loop already has the exact issuing handoff;
                    // searching/hashing the entire list again would be quadratic.
                    let stored = StoredHandoff { handoff: handoff.clone(), lifecycle };
                    stored.validate_lifecycle().map_err(corrupt)?;
                    stored.handoff.validate(record.block.timestamp_ms).map_err(corrupt)?;
                }
                (None, None) => return Err(StoreError::Corrupt("compaction handoff is absent".into())),
                (Some(_), Some(_)) => return Err(StoreError::Corrupt("handoff has overlapping full/compact records".into())),
            }
        }
        Ok(outcome)
    }
}
