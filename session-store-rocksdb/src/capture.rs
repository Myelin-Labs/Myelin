//! Consistent physical checkpoints and complete keyspace fingerprints. These
//! include operational queues, WAL and permanent identities, not signing keys
//! from a separate authority store and not external cold archive files.
use super::*;
use serde::{Deserialize, Serialize};
use std::collections::BTreeSet;

#[derive(Clone, Copy, Debug)]
pub struct SnapshotLimits {
    pub max_entries: u64,
    pub max_bytes: u64,
    pub max_sessions: usize,
    pub max_archives: usize,
}
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct SnapshotHead {
    pub session_id: Hash32,
    pub finalised_height: Option<u64>,
    pub block_hash: Hash32,
    pub state_root: Hash32,
    pub head_record_digest: Hash32,
}
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct SnapshotInventory {
    pub fingerprint: Hash32,
    pub entries: u64,
    pub encoded_bytes: u64,
    pub heads: Vec<SnapshotHead>,
    pub cold_blocks: u64,
    pub archives: Vec<Hash32>,
}
impl RocksSessionStore {
    pub fn storage_options(&self) -> RocksSessionStoreOptions {
        RocksSessionStoreOptions {
            archival_checkpoint_interval: self.archival_checkpoint_interval,
            max_network_queue_messages: self.max_network_queue_messages,
            max_network_queue_bytes: self.max_network_queue_bytes,
        }
    }
    /// RocksDB captures one consistent checkpoint while writers continue. The
    /// destination must not exist. This synchronous native operation cannot be
    /// preempted; callers must meter/check its resulting files before publishing
    /// a backup, and retain every cold manifest referenced by this capture.
    pub fn create_checkpoint(&self, destination: impl AsRef<Path>) -> Result<(), StoreError> {
        rocksdb::checkpoint::Checkpoint::new(&self.db).map_err(backend)?.create_checkpoint(destination).map_err(backend)
    }
    /// Fingerprint every key/value pair under one read snapshot. The callback
    /// reserves the record ceiling plus a 64 KiB key before each iterator read.
    /// No index is rebuilt and no operational state is omitted. This establishes
    /// exact storage identity, not application replay or renewed writer rights.
    pub fn snapshot_inventory(
        &self,
        limits: SnapshotLimits,
        check: &dyn Fn(u64) -> Result<(), StoreError>,
    ) -> Result<SnapshotInventory, StoreError> {
        let snapshot = self.db.snapshot();
        let mut iterator = snapshot.iterator(IteratorMode::Start);
        let mut hash = blake3::Hasher::new();
        hash.update(b"myelin:session-store-snapshot:v1");
        let mut entries = 0u64;
        let mut encoded_bytes = 0u64;
        let mut heads = vec![];
        let mut archives = BTreeSet::new();
        let mut cold_blocks = 0;
        let mut schema = false;
        loop {
            check(myelin_session::bounded_codec::MAX_RECORD_BYTES as u64 + 65536)?;
            let Some(entry) = iterator.next() else { break };
            let (key, value) = entry.map_err(backend)?;
            if key.is_empty() || key.len() > 65536 || value.len() > myelin_session::bounded_codec::MAX_RECORD_BYTES {
                return Err(StoreError::Corrupt("snapshot key/value exceeds current record bounds".into()));
            }
            entries = entries.checked_add(1).ok_or_else(|| StoreError::Corrupt("snapshot entry count overflow".into()))?;
            encoded_bytes = encoded_bytes
                .checked_add(key.len() as u64 + value.len() as u64 + 16)
                .ok_or_else(|| StoreError::Corrupt("snapshot byte count overflow".into()))?;
            if entries > limits.max_entries || encoded_bytes > limits.max_bytes {
                return Err(StoreError::Backend("snapshot exceeds entry/byte limits".into()));
            }
            hash.update(&(key.len() as u64).to_le_bytes());
            hash.update(&key);
            hash.update(&(value.len() as u64).to_le_bytes());
            hash.update(&value);
            if key.as_ref() == SCHEMA_KEY {
                if value.as_ref() != SCHEMA_VERSION {
                    return Err(StoreError::Corrupt("snapshot is not current storage schema".into()));
                }
                schema = true;
            } else if key[0] == KEY_HEAD {
                if key.len() != 33 || heads.len() >= limits.max_sessions {
                    return Err(StoreError::Corrupt("snapshot head key or session limit is invalid".into()));
                }
                let head = decode_head_for_session(&value, key[1..].try_into().unwrap())?;
                heads.push(SnapshotHead {
                    session_id: head.session_id,
                    finalised_height: head.finalised_height,
                    block_hash: head.block_hash,
                    state_root: head.state_root,
                    head_record_digest: *blake3::hash(&value).as_bytes(),
                });
            } else if key[0] == KEY_COLD_BLOCK {
                if key.len() != 41 {
                    return Err(StoreError::Corrupt("snapshot cold block key is malformed".into()));
                }
                let reference = ArchiveReference::decode(&value)?;
                archives.insert(reference.archive);
                if archives.len() > limits.max_archives {
                    return Err(StoreError::Backend("snapshot archive limit exceeded".into()));
                }
                cold_blocks += 1;
            }
        }
        if !schema {
            return Err(StoreError::Corrupt("snapshot has no current storage schema marker".into()));
        }
        Ok(SnapshotInventory {
            fingerprint: *hash.finalize().as_bytes(),
            entries,
            encoded_bytes,
            heads,
            cold_blocks,
            archives: archives.into_iter().collect(),
        })
    }
}
