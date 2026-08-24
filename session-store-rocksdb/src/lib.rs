// SPDX-License-Identifier: MIT
// Copyright (C) 2026 Myelin developers

//! Transactional RocksDB implementation of the Myelin session store.

use myelin_session::{
    ConsensusWal, FinalisedBlockRecord, Hash32, OutboxMessage, PendingDelivery, SessionGenesis, SessionHead, SessionStore, StoreError,
};
use myelin_session_network::{DurableEnvelope, EnqueueStatus, NetworkStore, NetworkStoreError};
use rocksdb::{
    Direction, Error as RocksError, ErrorKind, IteratorMode, OptimisticTransactionDB, OptimisticTransactionOptions, Options,
    WriteOptions,
};
use std::{collections::HashSet, path::Path};

const SCHEMA_KEY: &[u8] = b"\0myelin-session-store-schema";
const SCHEMA_VERSION: &[u8] = b"4";
const KEY_GENESIS: u8 = b'g';
const KEY_HEAD: u8 = b'h';
const KEY_BLOCK: u8 = b'b';
const KEY_WAL: u8 = b'w';
const KEY_OUTBOX: u8 = b'o';
const KEY_OUTBOX_INDEX: u8 = b'i';
const KEY_OUTBOUND_SEQUENCE: u8 = b'q';
const KEY_OUTBOUND: u8 = b'x';
const KEY_OUTBOUND_INDEX: u8 = b'y';
const KEY_INBOUND_SEQUENCE: u8 = b'r';
const KEY_INBOUND: u8 = b'n';
const KEY_INBOUND_INDEX: u8 = b'm';
const KEY_INBOUND_RECEIPT: u8 = b'R';

/// Durable, optimistic-transaction-backed session store.
///
/// Every mutating transaction uses RocksDB WAL with synchronous writes. A
/// compare-and-swap on the encoded head prevents two local writers from
/// finalising different blocks at the same height.
pub struct RocksSessionStore {
    db: OptimisticTransactionDB,
}

impl RocksSessionStore {
    /// Open or create a store at an explicit, narrow path.
    pub fn open(path: impl AsRef<Path>) -> Result<Self, StoreError> {
        let mut options = Options::default();
        options.create_if_missing(true);
        options.set_paranoid_checks(true);
        options.set_compression_type(rocksdb::DBCompressionType::Snappy);
        let db = OptimisticTransactionDB::open(&options, path).map_err(backend)?;
        match db.get(SCHEMA_KEY).map_err(backend)? {
            Some(version) if version == SCHEMA_VERSION => {}
            Some(version) => {
                return Err(StoreError::Corrupt(format!(
                    "unsupported RocksDB session-store schema {:?}",
                    String::from_utf8_lossy(&version)
                )));
            }
            None => {
                if let Some(item) = db.iterator(IteratorMode::Start).next() {
                    item.map_err(backend)?;
                    return Err(StoreError::Corrupt(
                        "unversioned non-empty RocksDB session store requires an explicit migration".to_owned(),
                    ));
                }
                let mut writes = WriteOptions::default();
                writes.set_sync(true);
                db.put_opt(SCHEMA_KEY, SCHEMA_VERSION, &writes).map_err(backend)?;
            }
        }
        Ok(Self { db })
    }

    fn sync_transaction(&self) -> rocksdb::Transaction<'_, OptimisticTransactionDB> {
        let mut writes = WriteOptions::default();
        writes.set_sync(true);
        let mut transaction_options = OptimisticTransactionOptions::default();
        transaction_options.set_snapshot(true);
        self.db.transaction_opt(&writes, &transaction_options)
    }
}

impl SessionStore for RocksSessionStore {
    fn create_session(&self, genesis: &SessionGenesis, head: &SessionHead) -> Result<(), StoreError> {
        validate_genesis(genesis, head)?;
        let genesis_key = session_key(KEY_GENESIS, genesis.config.session_id);
        let head_key = session_key(KEY_HEAD, genesis.config.session_id);
        let genesis_bytes = genesis.encode().map_err(corrupt)?;
        let head_bytes = head.encode().map_err(corrupt)?;
        let transaction = self.sync_transaction();
        if transaction.get_for_update(&genesis_key, true).map_err(backend)?.is_some()
            || transaction.get_for_update(&head_key, true).map_err(backend)?.is_some()
        {
            return Err(StoreError::Conflict("session id already exists".to_owned()));
        }
        transaction.put(&genesis_key, genesis_bytes).map_err(backend)?;
        transaction.put(&head_key, head_bytes).map_err(backend)?;
        transaction.commit().map_err(commit_error)
    }

    fn load_genesis(&self, session_id: Hash32) -> Result<SessionGenesis, StoreError> {
        let bytes = self
            .db
            .get(session_key(KEY_GENESIS, session_id))
            .map_err(backend)?
            .ok_or_else(|| StoreError::NotFound("session genesis".to_owned()))?;
        let genesis = SessionGenesis::decode(&bytes).map_err(corrupt)?;
        if genesis.config.session_id != session_id {
            return Err(StoreError::Corrupt("genesis key/session id mismatch".to_owned()));
        }
        Ok(genesis)
    }

    fn load_head(&self, session_id: Hash32) -> Result<SessionHead, StoreError> {
        let bytes = self
            .db
            .get(session_key(KEY_HEAD, session_id))
            .map_err(backend)?
            .ok_or_else(|| StoreError::NotFound("session head".to_owned()))?;
        decode_head_for_session(&bytes, session_id)
    }

    fn load_chain(&self, session_id: Hash32) -> Result<Vec<FinalisedBlockRecord>, StoreError> {
        let genesis = self.load_genesis(session_id)?;
        let prefix = session_key(KEY_BLOCK, session_id);
        let mut records = Vec::new();
        for item in self.db.iterator(IteratorMode::From(&prefix, Direction::Forward)) {
            let (key, value) = item.map_err(backend)?;
            if !key.starts_with(&prefix) {
                break;
            }
            if key.len() != prefix.len() + 8 {
                return Err(StoreError::Corrupt("malformed block key".to_owned()));
            }
            let height = u64::from_be_bytes(key[prefix.len()..].try_into().expect("checked length"));
            let record = FinalisedBlockRecord::decode(&value).map_err(corrupt)?;
            if record.block.number != height {
                return Err(StoreError::Corrupt(format!("block key/record height mismatch at {height}")));
            }
            if record.block.consensus_kind != genesis.config.consensus_kind
                || record.consensus_module_commitment != genesis.config.consensus_module_commitment
            {
                return Err(StoreError::Corrupt(format!("block consensus module mismatch at height {height}")));
            }
            records.push(record);
        }
        Ok(records)
    }

    fn commit_block(
        &self,
        expected_head: &SessionHead,
        new_head: &SessionHead,
        record: &FinalisedBlockRecord,
    ) -> Result<(), StoreError> {
        validate_commit(expected_head, new_head, record)?;
        let session_id = expected_head.session_id;
        let head_key = session_key(KEY_HEAD, session_id);
        let block_key = block_key(session_id, record.block.number);
        let expected_bytes = expected_head.encode().map_err(corrupt)?;
        let new_head_bytes = new_head.encode().map_err(corrupt)?;
        let record_bytes = record.encode().map_err(corrupt)?;

        let transaction = self.sync_transaction();
        let genesis = load_genesis_transaction(&transaction, session_id)?;
        if genesis.config.consensus_kind != record.block.consensus_kind
            || genesis.config.consensus_module_commitment != record.consensus_module_commitment
        {
            return Err(StoreError::Corrupt("block record consensus module does not match session genesis".to_owned()));
        }
        let actual = transaction
            .get_for_update(&head_key, true)
            .map_err(backend)?
            .ok_or_else(|| StoreError::NotFound("session head".to_owned()))?;
        if actual != expected_bytes {
            return Err(StoreError::Conflict("session head changed".to_owned()));
        }
        if transaction.get_for_update(&block_key, true).map_err(backend)?.is_some() {
            return Err(StoreError::Conflict(format!("height {} already exists", record.block.number)));
        }

        transaction.put(&block_key, record_bytes).map_err(backend)?;
        transaction.put(&head_key, new_head_bytes).map_err(backend)?;
        for message in &record.outbox {
            let message_key = outbox_key(session_id, record.block.number, message.id);
            let index_key = outbox_index_key(session_id, message.id);
            if transaction.get_for_update(&index_key, true).map_err(backend)?.is_some() {
                return Err(StoreError::Conflict("duplicate outbox message id".to_owned()));
            }
            transaction.put(&message_key, message.encode().map_err(corrupt)?).map_err(backend)?;
            transaction.put(&index_key, &message_key).map_err(backend)?;
        }

        let wal_key = session_key(KEY_WAL, session_id);
        if let Some(wal_bytes) = transaction.get_for_update(&wal_key, true).map_err(backend)? {
            let wal = ConsensusWal::decode(&wal_bytes).map_err(corrupt)?;
            if wal.consensus_module_commitment != genesis.config.consensus_module_commitment
                || wal.consensus_config_commitment != genesis.config.consensus_config_commitment
                || wal.wal_schema_hash != genesis.config.consensus_wal_schema_hash
            {
                return Err(StoreError::Corrupt("consensus WAL module/config/schema does not match session genesis".to_owned()));
            }
            if wal.height > record.block.number {
                return Err(StoreError::Conflict("consensus WAL is ahead of the block being committed".to_owned()));
            }
            transaction.delete(&wal_key).map_err(backend)?;
        }
        transaction.commit().map_err(commit_error)
    }

    fn load_consensus_wal(&self, session_id: Hash32) -> Result<Option<ConsensusWal>, StoreError> {
        let genesis = self.load_genesis(session_id)?;
        self.db
            .get(session_key(KEY_WAL, session_id))
            .map_err(backend)?
            .map(|bytes| {
                let wal = ConsensusWal::decode(&bytes).map_err(corrupt)?;
                if wal.session_id != session_id {
                    return Err(StoreError::Corrupt("WAL key/session id mismatch".to_owned()));
                }
                if wal.consensus_module_commitment != genesis.config.consensus_module_commitment
                    || wal.consensus_config_commitment != genesis.config.consensus_config_commitment
                    || wal.wal_schema_hash != genesis.config.consensus_wal_schema_hash
                {
                    return Err(StoreError::Corrupt("consensus WAL module/config/schema does not match session genesis".to_owned()));
                }
                Ok(wal)
            })
            .transpose()
    }

    fn compare_and_set_consensus_wal(&self, expected_revision: Option<u64>, wal: &ConsensusWal) -> Result<(), StoreError> {
        wal.validate().map_err(corrupt)?;
        let wal_key = session_key(KEY_WAL, wal.session_id);
        let head_key = session_key(KEY_HEAD, wal.session_id);
        let transaction = self.sync_transaction();
        let head_bytes = transaction
            .get_for_update(&head_key, false)
            .map_err(backend)?
            .ok_or_else(|| StoreError::NotFound("session head".to_owned()))?;
        let head = decode_head_for_session(&head_bytes, wal.session_id)?;
        let genesis = load_genesis_transaction(&transaction, wal.session_id)?;
        if wal.consensus_module_commitment != genesis.config.consensus_module_commitment
            || wal.consensus_config_commitment != genesis.config.consensus_config_commitment
            || wal.wal_schema_hash != genesis.config.consensus_wal_schema_hash
        {
            return Err(StoreError::Conflict("consensus WAL module/config/schema does not match session genesis".to_owned()));
        }
        let expected_height = head.next_height().map_err(corrupt)?;
        if wal.height != expected_height {
            return Err(StoreError::Conflict(format!("WAL height {} does not match next height {expected_height}", wal.height)));
        }
        let current = transaction.get_for_update(&wal_key, true).map_err(backend)?;
        let current_revision = current.as_deref().map(ConsensusWal::decode).transpose().map_err(corrupt)?.map(|value| value.revision);
        if current_revision != expected_revision {
            return Err(StoreError::Conflict("consensus WAL revision changed".to_owned()));
        }
        let required_revision = expected_revision
            .map_or(Some(0), |revision| revision.checked_add(1))
            .ok_or_else(|| StoreError::Conflict("WAL revision overflow".to_owned()))?;
        if wal.revision != required_revision {
            return Err(StoreError::Conflict(format!("next WAL revision must be {required_revision}")));
        }
        transaction.put(&wal_key, wal.encode().map_err(corrupt)?).map_err(backend)?;
        transaction.commit().map_err(commit_error)
    }

    fn pending_outbox(&self, session_id: Hash32, limit: usize) -> Result<Vec<PendingDelivery>, StoreError> {
        if limit == 0 {
            return Ok(Vec::new());
        }
        let genesis = self.load_genesis(session_id)?;
        let prefix = session_key(KEY_OUTBOX, session_id);
        let expected_key_len = prefix.len() + 8 + 32;
        let mut pending = Vec::with_capacity(limit.min(1024));
        for item in self.db.iterator(IteratorMode::From(&prefix, Direction::Forward)) {
            let (key, value) = item.map_err(backend)?;
            if !key.starts_with(&prefix) || pending.len() == limit {
                break;
            }
            if key.len() != expected_key_len {
                return Err(StoreError::Corrupt("malformed outbox key".to_owned()));
            }
            let height = u64::from_be_bytes(key[prefix.len()..prefix.len() + 8].try_into().expect("checked length"));
            let message = OutboxMessage::decode(&value).map_err(corrupt)?;
            if key[prefix.len() + 8..] != message.id {
                return Err(StoreError::Corrupt("outbox key/message id mismatch".to_owned()));
            }
            if message.consensus_module_commitment != genesis.config.consensus_module_commitment {
                return Err(StoreError::Corrupt("outbox consensus module does not match session genesis".to_owned()));
            }
            pending.push(PendingDelivery { height, message });
        }
        Ok(pending)
    }

    fn acknowledge_outbox(&self, session_id: Hash32, message_id: Hash32) -> Result<(), StoreError> {
        let index_key = outbox_index_key(session_id, message_id);
        let transaction = self.sync_transaction();
        let message_key = transaction
            .get_for_update(&index_key, true)
            .map_err(backend)?
            .ok_or_else(|| StoreError::NotFound("outbox message".to_owned()))?;
        if !message_key.starts_with(&session_key(KEY_OUTBOX, session_id)) {
            return Err(StoreError::Corrupt("outbox index points outside its session".to_owned()));
        }
        transaction.delete(&message_key).map_err(backend)?;
        transaction.delete(&index_key).map_err(backend)?;
        transaction.commit().map_err(commit_error)
    }
}

impl NetworkStore for RocksSessionStore {
    fn reserve_outbound_sequence(&self, session_id: Hash32, sender_id: &str, recipient_id: &str) -> Result<u64, NetworkStoreError> {
        ensure_network_session(&self.db, session_id)?;
        let key = peer_pair_key(KEY_OUTBOUND_SEQUENCE, session_id, sender_id, recipient_id);
        match self.db.get(key).map_err(network_backend)? {
            Some(bytes) => decode_sequence(&bytes)?
                .checked_add(1)
                .ok_or_else(|| NetworkStoreError::Conflict("outbound sequence overflow".to_owned())),
            None => Ok(0),
        }
    }

    fn enqueue_outbound(&self, message: &DurableEnvelope) -> Result<(), NetworkStoreError> {
        let session_id = envelope_session_id(message)?;
        validate_envelope_routing(message)?;
        let sequence_key =
            peer_pair_key(KEY_OUTBOUND_SEQUENCE, session_id, &message.envelope.sender_id, &message.envelope.recipient_id);
        let queue_key = outbound_queue_key(session_id, &message.envelope.recipient_id, message.envelope.sequence, message.message_id);
        let index_key = network_index_key(KEY_OUTBOUND_INDEX, session_id, message.message_id);
        let transaction = self.sync_transaction();
        let genesis = ensure_network_session_transaction(&transaction, session_id)?;
        ensure_network_module_binding(message, &genesis)?;
        let current = transaction.get_for_update(&sequence_key, true).map_err(network_backend)?;
        let expected = match current.as_deref() {
            Some(bytes) => decode_sequence(bytes)?
                .checked_add(1)
                .ok_or_else(|| NetworkStoreError::Conflict("outbound sequence overflow".to_owned()))?,
            None => 0,
        };
        if message.envelope.sequence != expected {
            return Err(NetworkStoreError::Conflict(format!(
                "outbound sequence {} does not match next sequence {expected}",
                message.envelope.sequence
            )));
        }
        if transaction.get_for_update(&index_key, true).map_err(network_backend)?.is_some() {
            return Err(NetworkStoreError::Conflict("outbound message id already exists".to_owned()));
        }
        transaction.put(&queue_key, message.encode()).map_err(network_backend)?;
        transaction.put(&index_key, &queue_key).map_err(network_backend)?;
        transaction.put(&sequence_key, message.envelope.sequence.to_be_bytes()).map_err(network_backend)?;
        transaction.commit().map_err(network_commit_error)
    }

    fn pending_outbound(
        &self,
        session_id: Hash32,
        recipient_id: &str,
        limit: usize,
    ) -> Result<Vec<DurableEnvelope>, NetworkStoreError> {
        load_network_queue(&self.db, outbound_route_key(session_id, recipient_id), session_id, limit)
    }

    fn acknowledge_outbound(&self, session_id: Hash32, message_id: Hash32) -> Result<(), NetworkStoreError> {
        acknowledge_network_message(&self.db, self.sync_transaction(), KEY_OUTBOUND, KEY_OUTBOUND_INDEX, session_id, message_id)
    }

    fn enqueue_inbound(&self, message: &DurableEnvelope) -> Result<EnqueueStatus, NetworkStoreError> {
        let session_id = envelope_session_id(message)?;
        validate_envelope_routing(message)?;
        let sequence_key =
            peer_pair_key(KEY_INBOUND_SEQUENCE, session_id, &message.envelope.sender_id, &message.envelope.recipient_id);
        let receipt_key =
            inbound_receipt_key(session_id, &message.envelope.sender_id, &message.envelope.recipient_id, message.envelope.sequence);
        let queue_key = network_queue_key(KEY_INBOUND, session_id, message.envelope.sequence, message.message_id);
        let index_key = network_index_key(KEY_INBOUND_INDEX, session_id, message.message_id);
        let transaction = self.sync_transaction();
        let genesis = ensure_network_session_transaction(&transaction, session_id)?;
        ensure_network_module_binding(message, &genesis)?;
        if let Some(receipt) = transaction.get_for_update(&receipt_key, true).map_err(network_backend)? {
            let received_message_id: Hash32 = receipt
                .as_slice()
                .try_into()
                .map_err(|_| NetworkStoreError::Corrupt("inbound receipt must contain one message id".to_owned()))?;
            if received_message_id == message.message_id {
                return Ok(EnqueueStatus::Duplicate);
            }
            return Err(NetworkStoreError::Conflict(format!("inbound sender equivocated at sequence {}", message.envelope.sequence)));
        }
        let current = transaction.get_for_update(&sequence_key, true).map_err(network_backend)?;
        if let Some(bytes) = current.as_deref() {
            let (current, _) = decode_inbound_cursor(bytes)?;
            if message.envelope.sequence <= current {
                return Err(NetworkStoreError::Corrupt(format!(
                    "inbound receipt for accepted sequence {} is missing",
                    message.envelope.sequence
                )));
            }
            let expected =
                current.checked_add(1).ok_or_else(|| NetworkStoreError::Conflict("inbound sequence overflow".to_owned()))?;
            if message.envelope.sequence != expected {
                return Err(NetworkStoreError::Conflict(format!(
                    "inbound sequence {} has a gap; expected {expected}",
                    message.envelope.sequence
                )));
            }
        } else if message.envelope.sequence != 0 {
            return Err(NetworkStoreError::Conflict(format!("first inbound sequence must be 0, got {}", message.envelope.sequence)));
        }
        if transaction.get_for_update(&index_key, true).map_err(network_backend)?.is_some() {
            return Err(NetworkStoreError::Conflict("inbound message id already exists outside the replay cursor".to_owned()));
        }
        transaction.put(&queue_key, message.encode()).map_err(network_backend)?;
        transaction.put(&index_key, &queue_key).map_err(network_backend)?;
        transaction.put(&receipt_key, message.message_id).map_err(network_backend)?;
        transaction
            .put(&sequence_key, encode_inbound_cursor(message.envelope.sequence, message.message_id))
            .map_err(network_backend)?;
        transaction.commit().map_err(network_commit_error)?;
        Ok(EnqueueStatus::Enqueued)
    }

    fn pending_inbound(&self, session_id: Hash32, limit: usize) -> Result<Vec<DurableEnvelope>, NetworkStoreError> {
        load_network_queue(&self.db, session_key(KEY_INBOUND, session_id), session_id, limit)
    }

    fn acknowledge_inbound(&self, session_id: Hash32, message_id: Hash32) -> Result<(), NetworkStoreError> {
        acknowledge_network_message(&self.db, self.sync_transaction(), KEY_INBOUND, KEY_INBOUND_INDEX, session_id, message_id)
    }
}

fn validate_genesis(genesis: &SessionGenesis, head: &SessionHead) -> Result<(), StoreError> {
    genesis.config.validate().map_err(corrupt)?;
    if genesis.config.session_id != head.session_id
        || head.finalised_height.is_some()
        || head.block_hash != [0; 32]
        || head.state_root != genesis.config.initial_state_root
        || head.timestamp_ms != 0
    {
        return Err(StoreError::Corrupt("invalid genesis head".to_owned()));
    }
    if genesis.state_snapshot.is_empty() {
        return Err(StoreError::Corrupt("initial state snapshot must not be empty".to_owned()));
    }
    Ok(())
}

fn validate_commit(expected: &SessionHead, new: &SessionHead, record: &FinalisedBlockRecord) -> Result<(), StoreError> {
    if expected.session_id != new.session_id
        || record.block.parent_hash != expected.block_hash
        || record.block.state_root_before != expected.state_root
        || record.block.number != expected.next_height().map_err(corrupt)?
        || new.finalised_height != Some(record.block.number)
        || new.block_hash != record.block.hash()
        || new.state_root != record.block.state_root_after
        || new.timestamp_ms != record.block.timestamp_ms
        || record.proof.kind() != record.block.consensus_kind
        || record.consensus_module_commitment == [0; 32]
    {
        return Err(StoreError::Corrupt("block/head linkage is inconsistent".to_owned()));
    }
    if record.transactions.len() != record.block.ordered_cell_tx_commitments.len() {
        return Err(StoreError::Corrupt("transaction payload/commitment count mismatch".to_owned()));
    }
    if record.state_snapshot.is_empty() {
        return Err(StoreError::Corrupt("post-state snapshot must not be empty".to_owned()));
    }
    let commitments = record.block.data_commitments.iter().copied().collect::<HashSet<_>>();
    if record.outbox.iter().any(|message| {
        message.consensus_module_commitment != record.consensus_module_commitment || !commitments.contains(&message.commitment())
    }) {
        return Err(StoreError::Corrupt("outbox message is not committed by the block".to_owned()));
    }
    Ok(())
}

fn session_key(prefix: u8, session_id: Hash32) -> Vec<u8> {
    let mut key = Vec::with_capacity(33);
    key.push(prefix);
    key.extend_from_slice(&session_id);
    key
}

fn block_key(session_id: Hash32, height: u64) -> Vec<u8> {
    let mut key = session_key(KEY_BLOCK, session_id);
    key.extend_from_slice(&height.to_be_bytes());
    key
}

fn outbox_key(session_id: Hash32, height: u64, message_id: Hash32) -> Vec<u8> {
    let mut key = session_key(KEY_OUTBOX, session_id);
    key.extend_from_slice(&height.to_be_bytes());
    key.extend_from_slice(&message_id);
    key
}

fn outbox_index_key(session_id: Hash32, message_id: Hash32) -> Vec<u8> {
    let mut key = session_key(KEY_OUTBOX_INDEX, session_id);
    key.extend_from_slice(&message_id);
    key
}

fn peer_pair_key(prefix: u8, session_id: Hash32, sender_id: &str, recipient_id: &str) -> Vec<u8> {
    let mut key = session_key(prefix, session_id);
    let mut hasher = blake3::Hasher::new();
    hasher.update(b"myelin:session-network:peer-pair");
    hasher.update(&(sender_id.len() as u64).to_be_bytes());
    hasher.update(sender_id.as_bytes());
    hasher.update(&(recipient_id.len() as u64).to_be_bytes());
    hasher.update(recipient_id.as_bytes());
    key.extend_from_slice(hasher.finalize().as_bytes());
    key
}

fn outbound_route_key(session_id: Hash32, recipient_id: &str) -> Vec<u8> {
    let mut key = session_key(KEY_OUTBOUND, session_id);
    key.extend_from_slice(blake3::hash(recipient_id.as_bytes()).as_bytes());
    key
}

fn inbound_receipt_key(session_id: Hash32, sender_id: &str, recipient_id: &str, sequence: u64) -> Vec<u8> {
    let mut key = peer_pair_key(KEY_INBOUND_RECEIPT, session_id, sender_id, recipient_id);
    key.extend_from_slice(&sequence.to_be_bytes());
    key
}

fn outbound_queue_key(session_id: Hash32, recipient_id: &str, sequence: u64, message_id: Hash32) -> Vec<u8> {
    let mut key = outbound_route_key(session_id, recipient_id);
    key.extend_from_slice(&sequence.to_be_bytes());
    key.extend_from_slice(&message_id);
    key
}

fn network_queue_key(prefix: u8, session_id: Hash32, sequence: u64, message_id: Hash32) -> Vec<u8> {
    let mut key = session_key(prefix, session_id);
    key.extend_from_slice(&sequence.to_be_bytes());
    key.extend_from_slice(&message_id);
    key
}

fn network_index_key(prefix: u8, session_id: Hash32, message_id: Hash32) -> Vec<u8> {
    let mut key = session_key(prefix, session_id);
    key.extend_from_slice(&message_id);
    key
}

fn envelope_session_id(message: &DurableEnvelope) -> Result<Hash32, NetworkStoreError> {
    message
        .envelope
        .session_id
        .as_slice()
        .try_into()
        .map_err(|_| NetworkStoreError::Corrupt("network envelope session id must be exactly 32 bytes".to_owned()))
}

fn validate_envelope_routing(message: &DurableEnvelope) -> Result<(), NetworkStoreError> {
    message.validate().map_err(|error| NetworkStoreError::Corrupt(error.to_string()))?;
    if message.envelope.sender_id.is_empty() || message.envelope.recipient_id.is_empty() {
        return Err(NetworkStoreError::Corrupt("network envelope sender and recipient must be non-empty".to_owned()));
    }
    Ok(())
}

fn decode_sequence(bytes: &[u8]) -> Result<u64, NetworkStoreError> {
    let bytes: [u8; 8] =
        bytes.try_into().map_err(|_| NetworkStoreError::Corrupt("network sequence value must be exactly 8 bytes".to_owned()))?;
    Ok(u64::from_be_bytes(bytes))
}

fn encode_inbound_cursor(sequence: u64, message_id: Hash32) -> [u8; 40] {
    let mut bytes = [0; 40];
    bytes[..8].copy_from_slice(&sequence.to_be_bytes());
    bytes[8..].copy_from_slice(&message_id);
    bytes
}

fn decode_inbound_cursor(bytes: &[u8]) -> Result<(u64, Hash32), NetworkStoreError> {
    if bytes.len() != 40 {
        return Err(NetworkStoreError::Corrupt("inbound replay cursor must be exactly 40 bytes".to_owned()));
    }
    let sequence = u64::from_be_bytes(bytes[..8].try_into().expect("checked length"));
    let message_id = bytes[8..].try_into().expect("checked length");
    Ok((sequence, message_id))
}

fn ensure_network_session(db: &OptimisticTransactionDB, session_id: Hash32) -> Result<(), NetworkStoreError> {
    if db.get(session_key(KEY_HEAD, session_id)).map_err(network_backend)?.is_none() {
        return Err(NetworkStoreError::NotFound("session head".to_owned()));
    }
    Ok(())
}

fn ensure_network_session_transaction(
    transaction: &rocksdb::Transaction<'_, OptimisticTransactionDB>,
    session_id: Hash32,
) -> Result<SessionGenesis, NetworkStoreError> {
    if transaction.get_for_update(session_key(KEY_HEAD, session_id), false).map_err(network_backend)?.is_none() {
        return Err(NetworkStoreError::NotFound("session head".to_owned()));
    }
    let bytes = transaction
        .get_for_update(session_key(KEY_GENESIS, session_id), false)
        .map_err(network_backend)?
        .ok_or_else(|| NetworkStoreError::NotFound("session genesis".to_owned()))?;
    let genesis = SessionGenesis::decode(&bytes).map_err(|error| NetworkStoreError::Corrupt(error.to_string()))?;
    if genesis.config.session_id != session_id {
        return Err(NetworkStoreError::Corrupt("genesis key/session id mismatch".to_owned()));
    }
    Ok(genesis)
}

fn load_network_queue(
    db: &OptimisticTransactionDB,
    prefix: Vec<u8>,
    session_id: Hash32,
    limit: usize,
) -> Result<Vec<DurableEnvelope>, NetworkStoreError> {
    if limit == 0 {
        return Ok(Vec::new());
    }
    ensure_network_session(db, session_id)?;
    let genesis_bytes = db
        .get(session_key(KEY_GENESIS, session_id))
        .map_err(network_backend)?
        .ok_or_else(|| NetworkStoreError::NotFound("session genesis".to_owned()))?;
    let genesis = SessionGenesis::decode(&genesis_bytes).map_err(|error| NetworkStoreError::Corrupt(error.to_string()))?;
    let expected_len = prefix.len() + 8 + 32;
    let mut messages = Vec::with_capacity(limit.min(1024));
    for item in db.iterator(IteratorMode::From(&prefix, Direction::Forward)) {
        let (key, value) = item.map_err(network_backend)?;
        if !key.starts_with(&prefix) || messages.len() == limit {
            break;
        }
        if key.len() != expected_len {
            return Err(NetworkStoreError::Corrupt("malformed durable network queue key".to_owned()));
        }
        let sequence = u64::from_be_bytes(key[prefix.len()..prefix.len() + 8].try_into().expect("checked length"));
        let message = DurableEnvelope::decode(&value).map_err(|error| NetworkStoreError::Corrupt(error.to_string()))?;
        if message.envelope.sequence != sequence || key[prefix.len() + 8..] != message.message_id {
            return Err(NetworkStoreError::Corrupt("network queue key/envelope mismatch".to_owned()));
        }
        if envelope_session_id(&message)? != session_id {
            return Err(NetworkStoreError::Corrupt("network queue session mismatch".to_owned()));
        }
        ensure_network_module_binding(&message, &genesis)?;
        messages.push(message);
    }
    Ok(messages)
}

fn acknowledge_network_message(
    _db: &OptimisticTransactionDB,
    transaction: rocksdb::Transaction<'_, OptimisticTransactionDB>,
    queue_prefix: u8,
    index_prefix: u8,
    session_id: Hash32,
    message_id: Hash32,
) -> Result<(), NetworkStoreError> {
    let index_key = network_index_key(index_prefix, session_id, message_id);
    let queue_key = transaction
        .get_for_update(&index_key, true)
        .map_err(network_backend)?
        .ok_or_else(|| NetworkStoreError::NotFound("durable network message".to_owned()))?;
    if !queue_key.starts_with(&session_key(queue_prefix, session_id)) {
        return Err(NetworkStoreError::Corrupt("network index points outside its queue".to_owned()));
    }
    transaction.delete(&queue_key).map_err(network_backend)?;
    transaction.delete(&index_key).map_err(network_backend)?;
    transaction.commit().map_err(network_commit_error)
}

fn decode_head_for_session(bytes: &[u8], session_id: Hash32) -> Result<SessionHead, StoreError> {
    let head = SessionHead::decode(bytes).map_err(corrupt)?;
    if head.session_id != session_id {
        return Err(StoreError::Corrupt("head key/session id mismatch".to_owned()));
    }
    Ok(head)
}

fn load_genesis_transaction(
    transaction: &rocksdb::Transaction<'_, OptimisticTransactionDB>,
    session_id: Hash32,
) -> Result<SessionGenesis, StoreError> {
    let bytes = transaction
        .get_for_update(session_key(KEY_GENESIS, session_id), false)
        .map_err(backend)?
        .ok_or_else(|| StoreError::NotFound("session genesis".to_owned()))?;
    let genesis = SessionGenesis::decode(&bytes).map_err(corrupt)?;
    if genesis.config.session_id != session_id {
        return Err(StoreError::Corrupt("genesis key/session id mismatch".to_owned()));
    }
    Ok(genesis)
}

fn ensure_network_module_binding(message: &DurableEnvelope, genesis: &SessionGenesis) -> Result<(), NetworkStoreError> {
    if message.envelope.consensus_module_commitment.as_slice() != genesis.config.consensus_module_commitment {
        return Err(NetworkStoreError::Conflict("network envelope consensus module does not match session genesis".to_owned()));
    }
    Ok(())
}

fn corrupt(error: impl std::fmt::Display) -> StoreError {
    StoreError::Corrupt(error.to_string())
}

fn backend(error: RocksError) -> StoreError {
    StoreError::Backend(error.to_string())
}

fn commit_error(error: RocksError) -> StoreError {
    match error.kind() {
        ErrorKind::Busy | ErrorKind::TryAgain => StoreError::Conflict(error.to_string()),
        _ => backend(error),
    }
}

fn network_backend(error: RocksError) -> NetworkStoreError {
    NetworkStoreError::Backend(error.to_string())
}

fn network_commit_error(error: RocksError) -> NetworkStoreError {
    match error.kind() {
        ErrorKind::Busy | ErrorKind::TryAgain => NetworkStoreError::Conflict(error.to_string()),
        _ => network_backend(error),
    }
}
