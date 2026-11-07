//! Durable external-proof inbox. Import does not create source session records
//! and never marks a transfer consumed. Target commit owns that atomic change.
use super::*;
use myelin_session::{
    FinalisedBlockRecord, HandoffBlockReference, HandoffImportKind, HandoffImportRegistry, HandoffInboxRecord, HandoffLifecycle,
    HandoffTarget, HandoffTerminal, VerifiedHandoffImport,
};

const KEY_REMOTE_IMPORT: u8 = b'z';
const KEY_REMOTE_IMPORT_INDEX: u8 = b'p';
const KEY_REMOTE_SOURCE_BLOCK: u8 = b'v';
const KEY_REMOTE_QUARANTINE: u8 = b'f';
const REMOTE_USAGE_KEY: &[u8] = b"\0myelin-remote-inbox-usage";

#[derive(Clone, Copy, Debug)]
pub struct RemoteInboxLimits {
    pub max_entries: u64,
    pub max_bytes: u64,
}
impl RemoteInboxLimits {
    fn validate(self) -> Result<(), StoreError> {
        if self.max_entries == 0 || self.max_entries > 65_536 || self.max_bytes == 0 || self.max_bytes > 8 * 1024 * 1024 * 1024 {
            return Err(StoreError::Corrupt("remote inbox limits outside supported envelope".into()));
        }
        Ok(())
    }
}
pub(super) struct RemoteInbox {
    pub(super) registry: Arc<HandoffImportRegistry>,
    limits: RemoteInboxLimits,
}
pub struct RemoteImportOutcome {
    pub inserted: bool,
    pub handoff: StoredHandoff,
}
fn inbox_key(namespace: Hash32) -> Vec<u8> {
    session_key(KEY_REMOTE_IMPORT, namespace)
}
fn import_index_key(id: Hash32) -> Vec<u8> {
    session_key(KEY_REMOTE_IMPORT_INDEX, id)
}
fn source_block_key(genesis: Hash32, height: u64) -> Vec<u8> {
    let mut key = session_key(KEY_REMOTE_SOURCE_BLOCK, genesis);
    key.extend(height.to_be_bytes());
    key
}
fn anchor(bytes: &[u8]) -> Result<(Hash32, Hash32), StoreError> {
    if bytes.len() != 64 {
        return Err(StoreError::Corrupt("invalid remote source block anchor".into()));
    }
    Ok((bytes[..32].try_into().unwrap(), bytes[32..].try_into().unwrap()))
}
fn source_history(
    transaction: &rocksdb::Transaction<'_, OptimisticTransactionDB>,
    genesis: Hash32,
    proof: &myelin_session::HandoffProofBundle,
) -> Result<Option<Vec<u8>>, StoreError> {
    let mut blocks = std::collections::BTreeMap::new();
    let issuing = FinalisedBlockRecord::decode(&proof.issuing_block).map_err(corrupt)?;
    blocks.insert(issuing.block.number, issuing.block);
    if let Some(evidence) = &proof.evidence {
        let message = FinalisedBlockRecord::decode(&evidence.message_block).map_err(corrupt)?;
        blocks.insert(message.block.number, message.block);
    }
    for (height, block) in &blocks {
        let hash = block.hash();
        let existing = transaction.get_for_update(source_block_key(genesis, *height), true).map_err(backend)?;
        let conflicting = if let Some(bytes) = existing {
            let (old, _) = anchor(&bytes)?;
            if old != hash {
                Some(old)
            } else {
                None
            }
        } else {
            None
        };
        let before = height
            .checked_sub(1)
            .map(|height| transaction.get_for_update(source_block_key(genesis, height), true))
            .transpose()
            .map_err(backend)?
            .flatten();
        let after = height
            .checked_add(1)
            .map(|height| transaction.get_for_update(source_block_key(genesis, height), true))
            .transpose()
            .map_err(backend)?
            .flatten();
        let conflicting = conflicting.or(if let Some(bytes) = before {
            let (previous, _) = anchor(&bytes)?;
            if previous != block.parent_hash {
                Some(previous)
            } else {
                None
            }
        } else {
            None
        });
        let conflicting = conflicting.or(if let Some(bytes) = after {
            let (_, parent) = anchor(&bytes)?;
            if parent != hash {
                Some(parent)
            } else {
                None
            }
        } else {
            None
        });
        if let Some(previous) = conflicting {
            let mut marker = height.to_le_bytes().to_vec();
            marker.extend(previous);
            marker.extend(hash);
            marker.extend(blake3::hash(&proof.encode(myelin_session::MAX_REMOTE_PROOF_BYTES).map_err(corrupt)?).as_bytes());
            return Ok(Some(marker));
        }
    }
    for (height, block) in blocks {
        let mut value = block.hash().to_vec();
        value.extend(block.parent_hash);
        transaction.put(source_block_key(genesis, height), value).map_err(backend)?;
    }
    Ok(None)
}
impl RocksSessionStore {
    /// Configure independently obtained source/application verifiers before
    /// constructing or recovering writable chains. Persisted keys cannot add trust.
    pub fn with_handoff_imports(
        mut self,
        registry: Arc<HandoffImportRegistry>,
        limits: RemoteInboxLimits,
    ) -> Result<Self, StoreError> {
        limits.validate()?;
        self.remote = Some(RemoteInbox { registry, limits });
        Ok(self)
    }

    pub fn import_handoff(&self, record: &HandoffInboxRecord) -> Result<RemoteImportOutcome, StoreError> {
        let remote = self.remote.as_ref().ok_or_else(|| StoreError::Backend("remote handoff intake is not configured".into()))?;
        let verified = remote.registry.verify(record).map_err(corrupt)?;
        if let HandoffImportKind::CoordinatedScarce { terminal_policy } = verified.kind() {
            if remote.registry.terminal_policy(record.policy).map_err(corrupt)?.commitment().map_err(corrupt)? != terminal_policy {
                return Err(StoreError::Conflict("remote scarce intake has no matching terminal policy".into()));
            }
        }
        let HandoffTarget::Session(target) = verified.handoff().target else {
            return Err(StoreError::Corrupt("remote policy targets unsupported".into()));
        };
        let source = verified.handoff().source_session_id;
        let id = verified.handoff().id();
        let namespace = verified.inbox_key();
        let encoded = record.encode().map_err(corrupt)?;
        let issuing = FinalisedBlockRecord::decode(&record.proof.issuing_block).map_err(corrupt)?;
        let handoff = StoredHandoff {
            handoff: verified.handoff().clone(),
            lifecycle: HandoffLifecycle {
                issued_at: HandoffBlockReference::from_block(&issuing.block),
                terminal: HandoffTerminal::Issued,
            },
        };
        let handoff_bytes = handoff.encode().map_err(corrupt)?;
        let transaction = self.sync_transaction();
        let target_bytes = transaction
            .get_for_update(session_key(KEY_GENESIS, target), true)
            .map_err(backend)?
            .ok_or_else(|| StoreError::NotFound("remote target genesis".into()))?;
        let target_genesis = SessionGenesis::decode(&target_bytes).map_err(corrupt)?;
        if target_genesis.config.commitment() != verified.policy().target_genesis {
            return Err(StoreError::Corrupt("configured remote target differs from local genesis".into()));
        }
        if transaction.get_for_update(session_key(KEY_GENESIS, source), true).map_err(backend)?.is_some() {
            return Err(StoreError::Conflict("local source history cannot also be imported as remote".into()));
        }
        let quarantine = session_key(KEY_REMOTE_QUARANTINE, verified.policy().source_genesis);
        if transaction.get_for_update(&quarantine, true).map_err(backend)?.is_some() {
            return Err(StoreError::Conflict("remote source is quarantined after signed-history equivocation".into()));
        }
        if let Some(marker) = source_history(&transaction, verified.policy().source_genesis, &record.proof)? {
            transaction.put(quarantine, marker).map_err(backend)?;
            transaction.commit().map_err(commit_error)?;
            return Err(StoreError::Conflict("signed remote source equivocation recorded; source quarantined".into()));
        }
        let indexed = transaction.get_for_update(import_index_key(id), true).map_err(backend)?;
        let existing = transaction.get_for_update(inbox_key(namespace), true).map_err(backend)?;
        if let Some(indexed) = indexed {
            if indexed.as_slice() != namespace {
                return Err(StoreError::Conflict("ambiguous handoff identity across source genesis namespaces".into()));
            }
            let existing = existing.ok_or_else(|| StoreError::Corrupt("namespaced inbox proof missing from identity index".into()))?;
            let existing = HandoffInboxRecord::decode(&existing).map_err(corrupt)?;
            let original = remote.registry.verify(&existing).map_err(corrupt)?;
            let original_block = FinalisedBlockRecord::decode(&existing.proof.issuing_block).map_err(corrupt)?;
            if existing.policy != record.policy
                || original.handoff() != verified.handoff()
                || original_block.block.hash() != issuing.block.hash()
            {
                return Err(StoreError::Conflict("conflicting source proof/policy for existing inbox identity".into()));
            }
            let stored = self
                .transaction_handoff(&transaction, id, &|_| Ok(()))?
                .ok_or_else(|| StoreError::Corrupt("remote inbox lacks its durable handoff marker".into()))?;
            if stored.handoff != handoff.handoff {
                return Err(StoreError::Corrupt("remote proof and marker differ".into()));
            }
            return Ok(RemoteImportOutcome { inserted: false, handoff: stored });
        }
        if existing.is_some() {
            return Err(StoreError::Corrupt("inbox proof lacks its identity index".into()));
        }
        if self.transaction_handoff(&transaction, id, &|_| Ok(()))?.is_some() {
            return Err(StoreError::Conflict("handoff identity already belongs to another source record".into()));
        }
        let (count, bytes) = match transaction.get_for_update(REMOTE_USAGE_KEY, true).map_err(backend)? {
            Some(bytes) if bytes.len() == 16 => {
                (u64::from_le_bytes(bytes[..8].try_into().unwrap()), u64::from_le_bytes(bytes[8..].try_into().unwrap()))
            }
            None => (0, 0),
            Some(_) => return Err(StoreError::Corrupt("invalid remote inbox usage record".into())),
        };
        let count = count
            .checked_add(1)
            .filter(|n| *n <= remote.limits.max_entries)
            .ok_or_else(|| StoreError::Conflict("remote inbox entry quota reached".into()))?;
        let bytes = bytes
            .checked_add(encoded.len() as u64)
            .and_then(|n| n.checked_add(handoff_bytes.len() as u64))
            // Conservative fixed metadata reservation: two source anchors,
            // namespaced proof/identity/handoff keys, quarantine and usage metadata.
            .and_then(|n|n.checked_add(768))
            .filter(|n| *n <= remote.limits.max_bytes)
            .ok_or_else(|| StoreError::Conflict("remote inbox byte quota reached".into()))?;
        let mut usage = count.to_le_bytes().to_vec();
        usage.extend(bytes.to_le_bytes());
        transaction.put(REMOTE_USAGE_KEY, usage).map_err(backend)?;
        transaction.put(inbox_key(namespace), encoded).map_err(backend)?;
        transaction.put(import_index_key(id), namespace).map_err(backend)?;
        transaction.put(handoff_key(id), handoff_bytes).map_err(backend)?;
        transaction.commit().map_err(commit_error)?;
        Ok(RemoteImportOutcome { inserted: true, handoff })
    }
    pub(super) fn verified_handoff_import(&self, id: Hash32) -> Result<Option<VerifiedHandoffImport>, StoreError> {
        let Some(namespace) = self.db.get(import_index_key(id)).map_err(backend)? else { return Ok(None) };
        let namespace: Hash32 =
            namespace.as_slice().try_into().map_err(|_| StoreError::Corrupt("invalid inbox namespace index".into()))?;
        let bytes = self
            .db
            .get(inbox_key(namespace))
            .map_err(backend)?
            .ok_or_else(|| StoreError::Corrupt("namespaced inbox proof missing".into()))?;
        let remote =
            self.remote.as_ref().ok_or_else(|| StoreError::Corrupt("remote inbox requires independently configured trust".into()))?;
        let record = HandoffInboxRecord::decode(&bytes).map_err(corrupt)?;
        let verified = remote.registry.verify(&record).map_err(corrupt)?;
        if self.db.get(session_key(KEY_REMOTE_QUARANTINE, verified.policy().source_genesis)).map_err(backend)?.is_some() {
            return Err(StoreError::Corrupt("remote source is quarantined after signed-history equivocation".into()));
        }
        let issuing = FinalisedBlockRecord::decode(&record.proof.issuing_block).map_err(corrupt)?;
        let bytes = self
            .db
            .get(source_block_key(verified.policy().source_genesis, issuing.block.number))
            .map_err(backend)?
            .ok_or_else(|| StoreError::Corrupt("remote source anchor missing".into()))?;
        if anchor(&bytes)?.0 != issuing.block.hash() {
            return Err(StoreError::Corrupt("remote source anchor differs from inbox proof".into()));
        }
        if verified.handoff().id() != id || verified.inbox_key() != namespace {
            return Err(StoreError::Corrupt("remote inbox key/proof mismatch".into()));
        }
        Ok(Some(verified))
    }
}
