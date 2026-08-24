// SPDX-License-Identifier: MIT
// Copyright (C) 2026 Myelin developers

//! Continuous finite-Cell session chains.
//!
//! This crate deliberately separates deterministic execution, finality,
//! persistence, networking, and settlement. A [`SessionChain`] only advances
//! after an engine-specific proof has been verified and the new head, latest
//! state checkpoint, block record, and outbox have been atomically committed
//! by a [`SessionStore`]. It is a closed-session runtime, not an independent
//! L1.

use myelin_consensus::{ConsensusKind, ConsensusModuleDescriptor, FinalityProof, MyelinBlock};
use myelin_exec::{
    deserialize_outpoint_molecule, deserialize_script_molecule, deserialize_transaction_molecule, serialize_outpoint_molecule,
    serialize_script_molecule, CellOutput, CellTx,
};
use myelin_state::{CellEntry, CellStateTree, ResolvedStateInput, StateTransitionContext, StateTransitionEngine};
use serde::{de::DeserializeOwned, Deserialize, Serialize};
use std::{collections::HashSet, fmt, sync::Arc};

/// Fixed-width identifier used for sessions, blocks, roots, and commitments.
pub type Hash32 = [u8; 32];

const RECORD_MAGIC: &[u8; 4] = b"MREC";
const RECORD_FORMAT_VERSION: u16 = 5;
const LEGACY_JSON_RECORD_FORMAT_VERSION: u16 = 4;
const RECORD_HEADER_BYTES: usize = 4 + 2 + 8 + 32;
const SNAPSHOT_MAGIC: &[u8; 4] = b"MSNP";
const SNAPSHOT_VERSION: u16 = 1;
const MAX_SNAPSHOT_ITEMS: usize = 16_777_216;
const MAX_SNAPSHOT_FIELD_BYTES: usize = 64 * 1024 * 1024;
const OUTBOX_DOMAIN: &[u8] = b"myelin:session-outbox";
const MAX_CONSENSUS_WAL_PAYLOAD_BYTES: usize = 16 * 1024 * 1024;
const MAX_OUTBOX_TOPIC_BYTES: usize = 256;
const MAX_OUTBOX_PAYLOAD_BYTES: usize = 64 * 1024 * 1024;
const RECOVERY_PAGE_SIZE: usize = 1_024;

/// Immutable limits and genesis commitment for one continuous session chain.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct SessionConfig {
    /// Globally unique session identifier.
    pub session_id: Hash32,
    /// Consensus engine selected for the full session.
    pub consensus_kind: ConsensusKind,
    /// Exact immutable validator/quorum configuration commitment.
    pub consensus_config_commitment: Hash32,
    /// Exact compiled-in module/proof/message/config descriptor commitment.
    pub consensus_module_commitment: Hash32,
    /// Owner-defined durable consensus round-state schema commitment.
    pub consensus_wal_schema_hash: Hash32,
    /// Exact state root from which height zero executes.
    pub initial_state_root: Hash32,
    /// Maximum number of transactions admitted to one block.
    pub max_block_transactions: u32,
    /// Maximum aggregate encoded transaction bytes in one block.
    pub max_block_bytes: u64,
    /// Maximum permitted proposed timestamp drift beyond the local clock.
    pub max_future_drift_ms: u64,
}

impl SessionConfig {
    /// Validate fail-closed limits and identifiers.
    pub fn validate(&self) -> Result<()> {
        if self.session_id == [0; 32] {
            return Err(SessionError::InvalidConfig("session_id must not be zero".to_owned()));
        }
        if self.consensus_config_commitment == [0; 32] {
            return Err(SessionError::InvalidConfig("consensus_config_commitment must not be zero".to_owned()));
        }
        if self.consensus_module_commitment == [0; 32] {
            return Err(SessionError::InvalidConfig("consensus_module_commitment must not be zero".to_owned()));
        }
        if self.consensus_wal_schema_hash == [0; 32] {
            return Err(SessionError::InvalidConfig("consensus_wal_schema_hash must not be zero".to_owned()));
        }
        if self.max_block_transactions == 0 {
            return Err(SessionError::InvalidConfig("max_block_transactions must be non-zero".to_owned()));
        }
        if self.max_block_bytes == 0 {
            return Err(SessionError::InvalidConfig("max_block_bytes must be non-zero".to_owned()));
        }
        Ok(())
    }

    /// Encode a checksummed, versioned persistence record.
    pub fn encode(&self) -> Result<Vec<u8>> {
        encode_record(&SessionConfigWire::from(self))
    }

    /// Decode a checksummed config, rejecting unknown fields and versions.
    pub fn decode(bytes: &[u8]) -> Result<Self> {
        let wire: SessionConfigWire = decode_record(bytes)?;
        let config = Self::try_from(wire)?;
        config.validate()?;
        Ok(config)
    }
}

/// Last atomically committed point of a session chain.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct SessionHead {
    /// Session identifier.
    pub session_id: Hash32,
    /// Last finalised height, or `None` before height zero.
    pub finalised_height: Option<u64>,
    /// Last finalised block hash, or zero before height zero.
    pub block_hash: Hash32,
    /// State root committed by the head.
    pub state_root: Hash32,
    /// Last block timestamp, or zero before height zero.
    pub timestamp_ms: u64,
}

impl SessionHead {
    /// Height that must be used by the next block.
    pub fn next_height(&self) -> Result<u64> {
        match self.finalised_height {
            Some(height) => height.checked_add(1).ok_or(SessionError::HeightOverflow),
            None => Ok(0),
        }
    }

    /// Encode a checksummed, versioned persistence record.
    pub fn encode(&self) -> Result<Vec<u8>> {
        encode_record(&SessionHeadWire::from(self))
    }

    /// Decode a checksummed head.
    pub fn decode(bytes: &[u8]) -> Result<Self> {
        Ok(Self::from(decode_record::<SessionHeadWire>(bytes)?))
    }
}

/// Durable external side effect emitted by deterministic execution.
///
/// Delivery state is maintained by the store. Handlers must be idempotent on
/// `id`, because a crash may happen after delivery and before acknowledgement.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct OutboxMessage {
    /// Deterministic event identifier assigned during block preparation.
    pub id: Hash32,
    /// Consensus module descriptor bound by the session that emitted it.
    pub consensus_module_commitment: Hash32,
    /// Namespaced plugin topic, such as `asset-exit/ckb-v1`.
    pub topic: String,
    /// Opaque, versioned plugin payload.
    pub payload: Vec<u8>,
}

impl OutboxMessage {
    /// Validate module namespace and finite resource bounds.
    pub fn validate(&self) -> Result<()> {
        if self.id == [0; 32] || self.consensus_module_commitment == [0; 32] {
            return Err(SessionError::Codec("outbox id and module commitment must not be zero".to_owned()));
        }
        if self.topic.is_empty() || self.topic.len() > MAX_OUTBOX_TOPIC_BYTES {
            return Err(SessionError::Codec(format!("outbox topic length must be 1..={MAX_OUTBOX_TOPIC_BYTES} bytes")));
        }
        if self.payload.len() > MAX_OUTBOX_PAYLOAD_BYTES {
            return Err(SessionError::Codec(format!("outbox payload exceeds {MAX_OUTBOX_PAYLOAD_BYTES} bytes")));
        }
        Ok(())
    }

    /// Commitment included in the block's data commitments.
    pub fn commitment(&self) -> Hash32 {
        let mut hasher = blake3::Hasher::new();
        hasher.update(OUTBOX_DOMAIN);
        hasher.update(&self.id);
        hasher.update(&self.consensus_module_commitment);
        hasher.update(&(self.topic.len() as u32).to_le_bytes());
        hasher.update(self.topic.as_bytes());
        hasher.update(&(self.payload.len() as u64).to_le_bytes());
        hasher.update(&self.payload);
        *hasher.finalize().as_bytes()
    }

    /// Encode a checksummed, versioned persistence record.
    pub fn encode(&self) -> Result<Vec<u8>> {
        self.validate()?;
        encode_record(self)
    }

    /// Decode a checksummed outbox record.
    pub fn decode(bytes: &[u8]) -> Result<Self> {
        let message: Self = decode_record(bytes)?;
        message.validate()?;
        Ok(message)
    }
}

/// Outbox item before its deterministic session/height/index id is assigned.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct PendingOutboxMessage {
    /// Namespaced plugin topic.
    pub topic: String,
    /// Opaque plugin payload.
    pub payload: Vec<u8>,
}

/// Result of executing an ordered transaction batch on a cloned state.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ExecutionOutcome {
    /// Raw transaction identifiers, in execution order.
    pub ordered_cell_tx_commitments: Vec<Hash32>,
    /// Application/DA commitments produced while executing the batch.
    pub data_commitments: Vec<Hash32>,
    /// External effects to atomically enqueue with the block.
    pub outbox: Vec<PendingOutboxMessage>,
}

/// Pluggable deterministic state transition implementation.
///
/// The runtime clones an executor, mutates only the clone, commits its snapshot,
/// and swaps it into memory only after the durable CAS succeeds.
pub trait TransitionExecutor: Clone + Send + Sync + 'static {
    /// Current canonical state root.
    fn state_root(&self) -> Hash32;

    /// Execute one ordered block against the current root.
    fn execute_block(&mut self, height: u64, transactions: &[Vec<u8>]) -> std::result::Result<ExecutionOutcome, String>;

    /// Produce a complete restart snapshot for the post-state.
    fn snapshot(&self) -> std::result::Result<Vec<u8>, String>;
}

/// Finality result produced only after a local deterministic verifier accepts
/// the exact block and typed proof.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct VerifiedFinality {
    /// Canonical hash of the verified block.
    pub block_hash: Hash32,
    /// Exact module descriptor used for verification.
    pub consensus_module_commitment: Hash32,
}

/// Session-owned port for deterministic finality verification.
///
/// Concrete consensus engines are adapted in the runtime composition root.
/// The session domain never drives rounds, signs votes, or trusts a remote
/// coordinator's success flag.
pub trait FinalityVerifier: Send + Sync + 'static {
    /// Exact compiled-in module descriptor and validator/quorum commitment.
    fn descriptor(&self) -> ConsensusModuleDescriptor;

    /// Verify the exact block and typed proof without mutating session state.
    fn verify(
        &self,
        block: &MyelinBlock,
        proof: &FinalityProof,
    ) -> std::result::Result<VerifiedFinality, myelin_consensus::ConsensusError>;
}

/// Request used to build a candidate block before finality is collected.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct BlockRequest {
    /// Canonical encoded transactions in execution order.
    pub transactions: Vec<Vec<u8>>,
    /// Commitment to scheduler sidecar evidence; never placed in witnesses.
    pub scheduler_commitment: Hash32,
    /// Additional DA commitments known before execution.
    pub data_commitments: Vec<Hash32>,
    /// Proposed block time in milliseconds.
    pub timestamp_ms: u64,
    /// Local wall-clock time used only for bounded future-drift validation.
    pub local_now_ms: u64,
}

/// Candidate block and isolated post-state awaiting an external finality proof.
pub struct PreparedBlock<E: TransitionExecutor> {
    block: MyelinBlock,
    base_head: SessionHead,
    executor: E,
    transactions: Vec<Vec<u8>>,
    snapshot: Vec<u8>,
    outbox: Vec<OutboxMessage>,
}

impl<E: TransitionExecutor> PreparedBlock<E> {
    /// Candidate block to propose and sign through the selected consensus plugin.
    pub fn block(&self) -> &MyelinBlock {
        &self.block
    }

    /// Deterministic outbox messages committed by the candidate.
    pub fn outbox(&self) -> &[OutboxMessage] {
        &self.outbox
    }
}

/// One finalised, replayable block record.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct FinalisedBlockRecord {
    /// Finalised block header and commitments.
    pub block: MyelinBlock,
    /// Exact registered consensus module used to verify this record.
    pub consensus_module_commitment: Hash32,
    /// Engine-specific proof verified before persistence.
    pub proof: FinalityProof,
    /// Canonical transaction payloads used to reproduce execution.
    pub transactions: Vec<Vec<u8>>,
    /// Complete post-state snapshot supplied at commit time.
    ///
    /// Stores may elide this payload from historical non-checkpoint records
    /// when they atomically retain the latest full checkpoint.
    pub state_snapshot: Vec<u8>,
    /// External effects committed in the same storage transaction.
    pub outbox: Vec<OutboxMessage>,
}

impl FinalisedBlockRecord {
    /// Encode a checksummed, versioned persistence record.
    pub fn encode(&self) -> Result<Vec<u8>> {
        encode_record(&FinalisedBlockWire::try_from(self)?)
    }

    /// Decode a stored record, checking its checksum and strict schema.
    pub fn decode(bytes: &[u8]) -> Result<Self> {
        Self::try_from(decode_record::<FinalisedBlockWire>(bytes)?)
    }
}

/// Genesis record atomically created with the initial chain head.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct SessionGenesis {
    /// Immutable session configuration.
    pub config: SessionConfig,
    /// Initial full execution snapshot.
    pub state_snapshot: Vec<u8>,
}

/// Atomically replaceable full state used to restart a session without
/// retaining a duplicate snapshot in every historical block.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct StateCheckpoint {
    /// Session owning the snapshot.
    pub session_id: Hash32,
    /// Height represented by the snapshot, or `None` for genesis.
    pub finalised_height: Option<u64>,
    /// State root reconstructed from the snapshot.
    pub state_root: Hash32,
    /// Complete executor-specific restart snapshot.
    pub state_snapshot: Vec<u8>,
}

impl StateCheckpoint {
    /// Encode a checksummed checkpoint record.
    pub fn encode(&self) -> Result<Vec<u8>> {
        if self.session_id == [0; 32] || self.state_snapshot.is_empty() {
            return Err(SessionError::Codec("checkpoint session id and snapshot must be non-empty".to_owned()));
        }
        encode_record(&StateCheckpointWire::from(self))
    }

    /// Decode a checksummed checkpoint record.
    pub fn decode(bytes: &[u8]) -> Result<Self> {
        let checkpoint = Self::from(decode_record::<StateCheckpointWire>(bytes)?);
        if checkpoint.session_id == [0; 32] || checkpoint.state_snapshot.is_empty() {
            return Err(SessionError::Codec("checkpoint session id and snapshot must be non-empty".to_owned()));
        }
        Ok(checkpoint)
    }
}

impl SessionGenesis {
    /// Encode immutable genesis material for a store backend.
    pub fn encode(&self) -> Result<Vec<u8>> {
        encode_record(&SessionGenesisWire {
            config: SessionConfigWire::from(&self.config),
            state_snapshot: self.state_snapshot.clone(),
        })
    }

    /// Decode checksummed immutable genesis material.
    pub fn decode(bytes: &[u8]) -> Result<Self> {
        let wire: SessionGenesisWire = decode_record(bytes)?;
        let config = SessionConfig::try_from(wire.config)?;
        config.validate()?;
        Ok(Self { config, state_snapshot: wire.state_snapshot })
    }
}

/// Persisted Tendermint or other consensus round WAL.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ConsensusWal {
    /// Session identifier.
    pub session_id: Hash32,
    /// Height whose messages are represented.
    pub height: u64,
    /// Exact registered consensus module that owns the WAL payload.
    pub consensus_module_commitment: Hash32,
    /// Exact validator/authority/quorum configuration for the module.
    pub consensus_config_commitment: Hash32,
    /// Owner-defined WAL schema commitment.
    pub wal_schema_hash: Hash32,
    /// Monotonic CAS revision.
    pub revision: u64,
    /// Consensus-plugin-owned, versioned state.
    pub payload: Vec<u8>,
    /// Integrity commitment to the opaque payload.
    pub payload_hash: Hash32,
}

impl ConsensusWal {
    /// Construct a bounded, module-bound WAL record.
    pub fn new(
        session_id: Hash32,
        height: u64,
        consensus_module_commitment: Hash32,
        consensus_config_commitment: Hash32,
        wal_schema_hash: Hash32,
        revision: u64,
        payload: Vec<u8>,
    ) -> Result<Self> {
        let payload_hash = *blake3::hash(&payload).as_bytes();
        let wal = Self {
            session_id,
            height,
            consensus_module_commitment,
            consensus_config_commitment,
            wal_schema_hash,
            revision,
            payload,
            payload_hash,
        };
        wal.validate()?;
        Ok(wal)
    }

    /// Validate identity, schema, size, and payload integrity.
    pub fn validate(&self) -> Result<()> {
        if self.session_id == [0; 32]
            || self.consensus_module_commitment == [0; 32]
            || self.consensus_config_commitment == [0; 32]
            || self.wal_schema_hash == [0; 32]
        {
            return Err(SessionError::Codec("consensus WAL identity/schema commitments must not be zero".to_owned()));
        }
        if self.payload.len() > MAX_CONSENSUS_WAL_PAYLOAD_BYTES {
            return Err(SessionError::Codec(format!("consensus WAL payload exceeds {MAX_CONSENSUS_WAL_PAYLOAD_BYTES} bytes")));
        }
        if *blake3::hash(&self.payload).as_bytes() != self.payload_hash {
            return Err(SessionError::Codec("consensus WAL payload hash mismatch".to_owned()));
        }
        Ok(())
    }

    /// Encode a checksummed, versioned WAL record.
    pub fn encode(&self) -> Result<Vec<u8>> {
        self.validate()?;
        encode_record(self)
    }

    /// Decode a checksummed WAL record.
    pub fn decode(bytes: &[u8]) -> Result<Self> {
        let wal: Self = decode_record(bytes)?;
        wal.validate()?;
        Ok(wal)
    }
}

/// Undelivered outbox entry returned by a store implementation.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct PendingDelivery {
    /// Height that committed the event.
    pub height: u64,
    /// Event to deliver idempotently.
    pub message: OutboxMessage,
}

/// Store-level failure categories used for safe retry decisions.
#[derive(Clone, Debug, PartialEq, Eq, thiserror::Error)]
pub enum StoreError {
    /// Another writer changed the head or WAL revision.
    #[error("compare-and-swap conflict: {0}")]
    Conflict(String),
    /// Requested session or record is absent.
    #[error("record not found: {0}")]
    NotFound(String),
    /// Durable bytes failed checksum/schema/invariant validation.
    #[error("corrupt durable record: {0}")]
    Corrupt(String),
    /// Backend I/O or database failure.
    #[error("store backend failure: {0}")]
    Backend(String),
}

/// Atomic persistence boundary for a session chain.
pub trait SessionStore: Send + Sync + 'static {
    /// Create genesis config, initial snapshot, and head iff the id is unused.
    fn create_session(&self, genesis: &SessionGenesis, head: &SessionHead) -> std::result::Result<(), StoreError>;
    /// Load immutable genesis material.
    fn load_genesis(&self, session_id: Hash32) -> std::result::Result<SessionGenesis, StoreError>;
    /// Load the current committed head.
    fn load_head(&self, session_id: Hash32) -> std::result::Result<SessionHead, StoreError>;
    /// Load all finalised blocks in ascending height order.
    fn load_chain(&self, session_id: Hash32) -> std::result::Result<Vec<FinalisedBlockRecord>, StoreError>;
    /// Load a bounded page beginning at `start_height`.
    ///
    /// Backends should override this to stream directly from durable keys.
    /// The default preserves compatibility for small/bounded in-memory stores.
    fn load_chain_page(
        &self,
        session_id: Hash32,
        start_height: u64,
        limit: usize,
    ) -> std::result::Result<Vec<FinalisedBlockRecord>, StoreError> {
        if limit == 0 {
            return Ok(Vec::new());
        }
        Ok(self.load_chain(session_id)?.into_iter().filter(|record| record.block.number >= start_height).take(limit).collect())
    }
    /// Load the latest full restart checkpoint.
    ///
    /// The default supports stores that retain a full snapshot in each block.
    fn load_checkpoint(&self, session_id: Hash32) -> std::result::Result<StateCheckpoint, StoreError> {
        let genesis = self.load_genesis(session_id)?;
        if let Some(record) = self.load_chain(session_id)?.into_iter().rev().find(|record| !record.state_snapshot.is_empty()) {
            return Ok(StateCheckpoint {
                session_id,
                finalised_height: Some(record.block.number),
                state_root: record.block.state_root_after,
                state_snapshot: record.state_snapshot,
            });
        }
        Ok(StateCheckpoint {
            session_id,
            finalised_height: None,
            state_root: genesis.config.initial_state_root,
            state_snapshot: genesis.state_snapshot,
        })
    }
    /// Atomically CAS the head and write block, snapshot, proof, and outbox.
    fn commit_block(
        &self,
        expected_head: &SessionHead,
        new_head: &SessionHead,
        record: &FinalisedBlockRecord,
    ) -> std::result::Result<(), StoreError>;
    /// Load a consensus WAL, if one exists.
    fn load_consensus_wal(&self, session_id: Hash32) -> std::result::Result<Option<ConsensusWal>, StoreError>;
    /// Atomically replace the expected WAL revision.
    fn compare_and_set_consensus_wal(&self, expected_revision: Option<u64>, wal: &ConsensusWal)
        -> std::result::Result<(), StoreError>;
    /// Return undelivered effects in deterministic height/id order.
    fn pending_outbox(&self, session_id: Hash32, limit: usize) -> std::result::Result<Vec<PendingDelivery>, StoreError>;
    /// Acknowledge one idempotently delivered effect.
    fn acknowledge_outbox(&self, session_id: Hash32, message_id: Hash32) -> std::result::Result<(), StoreError>;
}

/// Fully verified recovery material. Fields are private so callers cannot
/// bypass chain/finality audit before constructing a writable session.
pub struct AuditedRecovery {
    config: SessionConfig,
    head: SessionHead,
    checkpoint: StateCheckpoint,
}

impl AuditedRecovery {
    /// Snapshot bytes an executor plugin must restore.
    pub fn state_snapshot(&self) -> &[u8] {
        &self.checkpoint.state_snapshot
    }

    /// Root the restored executor must produce.
    pub fn state_root(&self) -> Hash32 {
        self.checkpoint.state_root
    }

    /// Audited durable head.
    pub fn head(&self) -> &SessionHead {
        &self.head
    }
}

/// A recovered, continuously advancing session chain.
pub struct SessionChain<E: TransitionExecutor, S: SessionStore> {
    config: SessionConfig,
    head: SessionHead,
    finality_verifier: Arc<dyn FinalityVerifier>,
    executor: E,
    store: Arc<S>,
}

impl<E: TransitionExecutor, S: SessionStore> SessionChain<E, S> {
    /// Create a new session and durably bind its initial snapshot.
    pub fn create(config: SessionConfig, finality_verifier: Arc<dyn FinalityVerifier>, executor: E, store: Arc<S>) -> Result<Self> {
        config.validate()?;
        ensure_consensus_config(finality_verifier.as_ref(), &config)?;
        if executor.state_root() != config.initial_state_root {
            return Err(SessionError::StateRootMismatch { expected: config.initial_state_root, actual: executor.state_root() });
        }
        let state_snapshot = executor.snapshot().map_err(SessionError::Execution)?;
        let head = SessionHead {
            session_id: config.session_id,
            finalised_height: None,
            block_hash: [0; 32],
            state_root: config.initial_state_root,
            timestamp_ms: 0,
        };
        store.create_session(&SessionGenesis { config: config.clone(), state_snapshot }, &head)?;
        Ok(Self { config, head, finality_verifier, executor, store })
    }

    /// Recover a chain after the caller reconstructs an executor from the most
    /// recent snapshot returned by [`Self::recovery_snapshot`]. Every block and
    /// finality proof is streamed and revalidated before the service becomes
    /// writable.
    pub fn recover(session_id: Hash32, finality_verifier: Arc<dyn FinalityVerifier>, executor: E, store: Arc<S>) -> Result<Self> {
        let audited = Self::audit_recovery(session_id, finality_verifier.as_ref(), store.as_ref())?;
        Self::recover_audited(audited, finality_verifier, executor, store)
    }

    /// Stream and verify a complete durable history exactly once, then return
    /// its atomically stored restart checkpoint.
    pub fn audit_recovery(session_id: Hash32, finality_verifier: &dyn FinalityVerifier, store: &S) -> Result<AuditedRecovery> {
        let genesis = store.load_genesis(session_id)?;
        genesis.config.validate()?;
        ensure_consensus_config(finality_verifier, &genesis.config)?;
        let persisted_head = store.load_head(session_id)?;
        let mut audited_head = genesis_head(&genesis);
        let mut next_height = 0u64;
        loop {
            let page = store.load_chain_page(session_id, next_height, RECOVERY_PAGE_SIZE)?;
            if page.is_empty() {
                break;
            }
            for record in &page {
                audit_record(&genesis, finality_verifier, &mut audited_head, record)?;
                next_height = record.block.number.checked_add(1).ok_or(SessionError::HeightOverflow)?;
            }
            if page.len() < RECOVERY_PAGE_SIZE {
                break;
            }
        }
        if audited_head != persisted_head {
            return Err(SessionError::Recovery("persisted head does not match audited chain".to_owned()));
        }
        let checkpoint = store.load_checkpoint(session_id)?;
        if checkpoint.session_id != session_id
            || checkpoint.finalised_height != persisted_head.finalised_height
            || checkpoint.state_root != persisted_head.state_root
            || checkpoint.state_snapshot.is_empty()
        {
            return Err(SessionError::Recovery("restart checkpoint does not match the audited head".to_owned()));
        }
        Ok(AuditedRecovery { config: genesis.config, head: persisted_head, checkpoint })
    }

    /// Attach an executor restored from [`AuditedRecovery::state_snapshot`]
    /// without reading or auditing the chain a second time.
    pub fn recover_audited(
        audited: AuditedRecovery,
        finality_verifier: Arc<dyn FinalityVerifier>,
        executor: E,
        store: Arc<S>,
    ) -> Result<Self> {
        ensure_consensus_config(finality_verifier.as_ref(), &audited.config)?;
        if executor.state_root() != audited.checkpoint.state_root {
            return Err(SessionError::StateRootMismatch { expected: audited.checkpoint.state_root, actual: executor.state_root() });
        }
        Ok(Self { config: audited.config, head: audited.head, finality_verifier, executor, store })
    }

    /// Obtain the snapshot that an executor plugin must restore before calling
    /// [`Self::recover`]. The returned head is informational and is audited by recovery.
    pub fn recovery_snapshot(store: &S, session_id: Hash32) -> Result<(SessionHead, Vec<u8>)> {
        let head = store.load_head(session_id)?;
        let checkpoint = store.load_checkpoint(session_id)?;
        if checkpoint.session_id != session_id
            || checkpoint.finalised_height != head.finalised_height
            || checkpoint.state_root != head.state_root
        {
            return Err(SessionError::Recovery("restart checkpoint does not match the durable head".to_owned()));
        }
        Ok((head, checkpoint.state_snapshot))
    }

    /// Current durable head.
    pub fn head(&self) -> &SessionHead {
        &self.head
    }

    /// Immutable session config.
    pub fn config(&self) -> &SessionConfig {
        &self.config
    }

    /// Execute a request on an isolated clone and build the exact block that
    /// validators must sign. This function performs no durable mutation.
    pub fn prepare_block(&self, request: BlockRequest) -> Result<PreparedBlock<E>> {
        let tx_count = request.transactions.len();
        if tx_count > self.config.max_block_transactions as usize {
            return Err(SessionError::BlockLimit(format!(
                "transaction count {tx_count} exceeds {}",
                self.config.max_block_transactions
            )));
        }
        let total_bytes = request.transactions.iter().try_fold(0u64, |total, tx| {
            total.checked_add(tx.len() as u64).ok_or_else(|| SessionError::BlockLimit("transaction bytes overflow".to_owned()))
        })?;
        if total_bytes > self.config.max_block_bytes {
            return Err(SessionError::BlockLimit(format!("block bytes {total_bytes} exceed {}", self.config.max_block_bytes)));
        }
        if request.timestamp_ms < self.head.timestamp_ms {
            return Err(SessionError::InvalidTimestamp("timestamp regressed below the finalised head".to_owned()));
        }
        let latest_allowed = request.local_now_ms.saturating_add(self.config.max_future_drift_ms);
        if request.timestamp_ms > latest_allowed {
            return Err(SessionError::InvalidTimestamp("timestamp exceeds configured future drift".to_owned()));
        }

        let height = self.head.next_height()?;
        let mut candidate = self.executor.clone();
        if candidate.state_root() != self.head.state_root {
            return Err(SessionError::StateRootMismatch { expected: self.head.state_root, actual: candidate.state_root() });
        }
        let outcome = candidate.execute_block(height, &request.transactions).map_err(SessionError::Execution)?;
        if outcome.ordered_cell_tx_commitments.len() != request.transactions.len() {
            return Err(SessionError::Execution("executor returned a different transaction count".to_owned()));
        }
        let mut seen = HashSet::with_capacity(outcome.ordered_cell_tx_commitments.len());
        if outcome.ordered_cell_tx_commitments.iter().any(|txid| !seen.insert(*txid)) {
            return Err(SessionError::Execution("executor returned duplicate raw txids".to_owned()));
        }

        let outbox = outcome
            .outbox
            .into_iter()
            .enumerate()
            .map(|(index, message)| {
                make_outbox_message(self.config.session_id, self.config.consensus_module_commitment, height, index, message)
            })
            .collect::<Result<Vec<_>>>()?;
        let mut data_commitments = request.data_commitments;
        data_commitments.extend(outcome.data_commitments);
        data_commitments.extend(outbox.iter().map(OutboxMessage::commitment));

        let block = MyelinBlock {
            version: 1,
            parent_hash: self.head.block_hash,
            number: height,
            timestamp_ms: request.timestamp_ms,
            consensus_kind: self.config.consensus_kind,
            state_root_before: self.head.state_root,
            state_root_after: candidate.state_root(),
            ordered_cell_tx_commitments: outcome.ordered_cell_tx_commitments,
            data_commitments,
            scheduler_commitment: request.scheduler_commitment,
        };
        let snapshot = candidate.snapshot().map_err(SessionError::Execution)?;
        Ok(PreparedBlock {
            block,
            base_head: self.head.clone(),
            executor: candidate,
            transactions: request.transactions,
            snapshot,
            outbox,
        })
    }

    /// Verify finality and atomically advance the chain. A stale candidate is
    /// rejected both in memory and by the store's durable CAS.
    pub fn commit_finalised(&mut self, prepared: PreparedBlock<E>, proof: FinalityProof) -> Result<SessionHead> {
        if prepared.base_head != self.head {
            return Err(SessionError::StalePreparedBlock);
        }
        let finalised = self.finality_verifier.verify(&prepared.block, &proof)?;
        let canonical_block_hash = prepared.block.hash();
        if finalised.block_hash != canonical_block_hash {
            return Err(SessionError::FinalityMismatch { expected: canonical_block_hash, actual: finalised.block_hash });
        }
        if finalised.consensus_module_commitment != self.config.consensus_module_commitment {
            return Err(SessionError::InvalidConfig("finality verifier returned a different module commitment".to_owned()));
        }
        let new_head = SessionHead {
            session_id: self.config.session_id,
            finalised_height: Some(prepared.block.number),
            block_hash: finalised.block_hash,
            state_root: prepared.block.state_root_after,
            timestamp_ms: prepared.block.timestamp_ms,
        };
        let record = FinalisedBlockRecord {
            block: prepared.block,
            consensus_module_commitment: finalised.consensus_module_commitment,
            proof,
            transactions: prepared.transactions,
            state_snapshot: prepared.snapshot,
            outbox: prepared.outbox,
        };
        self.store.commit_block(&self.head, &new_head, &record)?;
        self.executor = prepared.executor;
        self.head = new_head.clone();
        Ok(new_head)
    }
}

fn ensure_consensus_config(verifier: &dyn FinalityVerifier, config: &SessionConfig) -> Result<()> {
    let descriptor = verifier.descriptor();
    if descriptor.consensus_kind != config.consensus_kind {
        return Err(SessionError::ConsensusKind {
            expected: config.consensus_kind.as_str(),
            actual: descriptor.consensus_kind.as_str(),
        });
    }
    if descriptor.config_commitment != config.consensus_config_commitment {
        return Err(SessionError::InvalidConfig(
            "selected consensus validator/quorum config does not match its immutable commitment".to_owned(),
        ));
    }
    if descriptor.commitment() != config.consensus_module_commitment {
        return Err(SessionError::InvalidConfig(
            "selected consensus module descriptor does not match its immutable commitment".to_owned(),
        ));
    }
    if descriptor.wal_schema_hash != config.consensus_wal_schema_hash {
        return Err(SessionError::InvalidConfig("selected consensus WAL schema does not match session genesis".to_owned()));
    }
    Ok(())
}

fn genesis_head(genesis: &SessionGenesis) -> SessionHead {
    SessionHead {
        session_id: genesis.config.session_id,
        finalised_height: None,
        block_hash: [0; 32],
        state_root: genesis.config.initial_state_root,
        timestamp_ms: 0,
    }
}

fn audit_record(
    genesis: &SessionGenesis,
    verifier: &dyn FinalityVerifier,
    head: &mut SessionHead,
    record: &FinalisedBlockRecord,
) -> Result<()> {
    if record.block.parent_hash != head.block_hash
        || record.block.number != head.next_height()?
        || record.block.state_root_before != head.state_root
        || record.block.timestamp_ms < head.timestamp_ms
        || record.block.consensus_kind != genesis.config.consensus_kind
        || record.consensus_module_commitment != genesis.config.consensus_module_commitment
    {
        return Err(SessionError::Recovery(format!("invalid chain linkage at height {}", record.block.number)));
    }
    if record.transactions.len() != record.block.ordered_cell_tx_commitments.len() {
        return Err(SessionError::Recovery(format!("payload count mismatch at height {}", record.block.number)));
    }
    let verified = verifier.verify(&record.block, &record.proof)?;
    if verified.block_hash != record.block.hash() || verified.consensus_module_commitment != record.consensus_module_commitment {
        return Err(SessionError::Recovery(format!("finality verifier result mismatch at height {}", record.block.number)));
    }
    *head = SessionHead {
        session_id: genesis.config.session_id,
        finalised_height: Some(record.block.number),
        block_hash: record.block.hash(),
        state_root: record.block.state_root_after,
        timestamp_ms: record.block.timestamp_ms,
    };
    Ok(())
}

fn make_outbox_message(
    session_id: Hash32,
    consensus_module_commitment: Hash32,
    height: u64,
    index: usize,
    pending: PendingOutboxMessage,
) -> Result<OutboxMessage> {
    if pending.topic.is_empty() || pending.topic.len() > MAX_OUTBOX_TOPIC_BYTES {
        return Err(SessionError::Execution(format!("outbox topic length must be 1..={MAX_OUTBOX_TOPIC_BYTES} bytes")));
    }
    if pending.payload.len() > MAX_OUTBOX_PAYLOAD_BYTES {
        return Err(SessionError::Execution(format!("outbox payload exceeds {MAX_OUTBOX_PAYLOAD_BYTES} bytes")));
    }
    let index = u32::try_from(index).map_err(|_| SessionError::Execution("too many outbox messages".to_owned()))?;
    let mut hasher = blake3::Hasher::new();
    hasher.update(OUTBOX_DOMAIN);
    hasher.update(&session_id);
    hasher.update(&consensus_module_commitment);
    hasher.update(&height.to_le_bytes());
    hasher.update(&index.to_le_bytes());
    hasher.update(&(pending.topic.len() as u32).to_le_bytes());
    hasher.update(pending.topic.as_bytes());
    hasher.update(&(pending.payload.len() as u64).to_le_bytes());
    hasher.update(&pending.payload);
    Ok(OutboxMessage {
        id: *hasher.finalize().as_bytes(),
        consensus_module_commitment,
        topic: pending.topic,
        payload: pending.payload,
    })
}

/// Script verifier boundary for the built-in finite-Cell executor.
pub trait CellScriptVerifier: Send + Sync + 'static {
    /// Verify the exact transaction against inputs resolved from its pre-state.
    fn verify(&self, tx: &CellTx, resolved_inputs: &[ResolvedStateInput]) -> std::result::Result<u64, String>;
}

impl<F> CellScriptVerifier for F
where
    F: Fn(&CellTx, &[ResolvedStateInput]) -> std::result::Result<u64, String> + Send + Sync + 'static,
{
    fn verify(&self, tx: &CellTx, resolved_inputs: &[ResolvedStateInput]) -> std::result::Result<u64, String> {
        self(tx, resolved_inputs)
    }
}

/// Built-in sequential CellTx executor with complete restart snapshots.
///
/// Production session/court callers should install a verifier backed by
/// `TransactionScriptVerifier` using `VmSemantics::CkbStrict` and an immutable
/// dependency/header context. This type never supplies an always-success path.
#[derive(Clone)]
pub struct CellTransitionExecutor {
    engine: StateTransitionEngine,
    state_root: Hash32,
    verifier: Arc<dyn CellScriptVerifier>,
}

impl fmt::Debug for CellTransitionExecutor {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.debug_struct("CellTransitionExecutor").field("state_root", &self.state_root).finish_non_exhaustive()
    }
}

impl CellTransitionExecutor {
    /// Bind a complete live-cell state to an external strict verifier.
    pub fn new(mut state: CellStateTree, verifier: Arc<dyn CellScriptVerifier>) -> Self {
        let state_root = state.root().as_bytes();
        Self { engine: StateTransitionEngine::new(state), state_root, verifier }
    }

    /// Restore a complete snapshot. All scripts/data are rehashed while decoding.
    pub fn from_snapshot(snapshot: &[u8], verifier: Arc<dyn CellScriptVerifier>) -> Result<Self> {
        Ok(Self::new(decode_cell_snapshot(snapshot)?, verifier))
    }

    /// Borrow the live state for read-only RPC/query plugins.
    pub fn state(&self) -> &CellStateTree {
        self.engine.state()
    }
}

impl TransitionExecutor for CellTransitionExecutor {
    fn state_root(&self) -> Hash32 {
        self.state_root
    }

    fn execute_block(&mut self, height: u64, transactions: &[Vec<u8>]) -> std::result::Result<ExecutionOutcome, String> {
        let mut txids = Vec::with_capacity(transactions.len());
        for bytes in transactions {
            let tx = deserialize_transaction_molecule(bytes).map_err(|error| format!("invalid canonical CellTx: {error}"))?;
            if tx.version() != 0 {
                return Err(format!("CellTx version {} is not supported; new transactions must use version 0", tx.version()));
            }
            let verifier = Arc::clone(&self.verifier);
            let receipt = self
                .engine
                .apply_transaction(&tx, StateTransitionContext::ordinary(height), move |tx, inputs| verifier.verify(tx, inputs))
                .map_err(|error| error.to_string())?;
            txids.push(receipt.txid);
            self.state_root = receipt.state_root_after.as_bytes();
        }
        Ok(ExecutionOutcome { ordered_cell_tx_commitments: txids, data_commitments: Vec::new(), outbox: Vec::new() })
    }

    fn snapshot(&self) -> std::result::Result<Vec<u8>, String> {
        encode_cell_snapshot(self.engine.state()).map_err(|error| error.to_string())
    }
}

/// Encode every live cell, including full scripts and data required after restart.
pub fn encode_cell_snapshot(state: &CellStateTree) -> Result<Vec<u8>> {
    let count = u32::try_from(state.len()).map_err(|_| SessionError::Snapshot("too many live cells".to_owned()))?;
    let mut out = Vec::new();
    out.extend_from_slice(SNAPSHOT_MAGIC);
    out.extend_from_slice(&SNAPSHOT_VERSION.to_le_bytes());
    out.extend_from_slice(&count.to_le_bytes());
    for (outpoint, entry) in state.iter_by_outpoint() {
        let lock = entry
            .lock_script
            .as_ref()
            .ok_or_else(|| SessionError::Snapshot(format!("cell {outpoint} is missing its full lock script")))?;
        let data = entry.data.as_ref().ok_or_else(|| SessionError::Snapshot(format!("cell {outpoint} is missing full data")))?;
        let rebuilt = CellEntry::from_output(
            &CellOutput { capacity: entry.capacity, lock: lock.clone(), type_: entry.type_script.clone() },
            data,
            entry.created_block_number,
            entry.is_cellbase,
        )
        .map_err(|error| SessionError::Snapshot(error.to_string()))?;
        if rebuilt != *entry {
            return Err(SessionError::Snapshot(format!("cell {outpoint} full metadata does not match its committed hashes")));
        }
        put_bytes(&mut out, &serialize_outpoint_molecule(outpoint).map_err(|error| SessionError::Snapshot(error.to_string()))?)?;
        put_bytes(&mut out, &serialize_script_molecule(lock).map_err(|error| SessionError::Snapshot(error.to_string()))?)?;
        match &entry.type_script {
            Some(script) => {
                out.push(1);
                put_bytes(&mut out, &serialize_script_molecule(script).map_err(|error| SessionError::Snapshot(error.to_string()))?)?;
            }
            None => out.push(0),
        }
        put_bytes(&mut out, data)?;
        out.extend_from_slice(&entry.capacity.to_le_bytes());
        out.extend_from_slice(&entry.created_block_number.to_le_bytes());
        out.push(u8::from(entry.is_cellbase));
    }
    Ok(out)
}

/// Decode a bounded, canonical live-cell snapshot and rederive every commitment.
pub fn decode_cell_snapshot(bytes: &[u8]) -> Result<CellStateTree> {
    let mut decoder = SnapshotDecoder::new(bytes);
    if decoder.take(4)? != SNAPSHOT_MAGIC {
        return Err(SessionError::Snapshot("wrong snapshot magic".to_owned()));
    }
    let version = decoder.u16()?;
    if version != SNAPSHOT_VERSION {
        return Err(SessionError::Snapshot(format!("unsupported snapshot version {version}")));
    }
    let count = decoder.u32()? as usize;
    if count > MAX_SNAPSHOT_ITEMS {
        return Err(SessionError::Snapshot("snapshot item limit exceeded".to_owned()));
    }
    let mut state = CellStateTree::new();
    let mut previous_key: Option<Vec<u8>> = None;
    for _ in 0..count {
        let outpoint_bytes = decoder.bytes()?;
        let outpoint = deserialize_outpoint_molecule(outpoint_bytes).map_err(|error| SessionError::Snapshot(error.to_string()))?;
        let key = outpoint.to_key().to_vec();
        if previous_key.as_ref().is_some_and(|previous| previous >= &key) {
            return Err(SessionError::Snapshot("outpoints are duplicated or not canonically ordered".to_owned()));
        }
        previous_key = Some(key);
        let lock = deserialize_script_molecule(decoder.bytes()?).map_err(|error| SessionError::Snapshot(error.to_string()))?;
        let type_script = match decoder.byte()? {
            0 => None,
            1 => Some(deserialize_script_molecule(decoder.bytes()?).map_err(|error| SessionError::Snapshot(error.to_string()))?),
            other => return Err(SessionError::Snapshot(format!("invalid type-script flag {other}"))),
        };
        let data = decoder.bytes()?.to_vec();
        let capacity = decoder.u64()?;
        let created_block_number = decoder.u64()?;
        let is_cellbase = match decoder.byte()? {
            0 => false,
            1 => true,
            other => return Err(SessionError::Snapshot(format!("invalid cellbase flag {other}"))),
        };
        let output = CellOutput { capacity, lock, type_: type_script };
        let entry = CellEntry::from_output(&output, &data, created_block_number, is_cellbase)
            .map_err(|error| SessionError::Snapshot(error.to_string()))?;
        state.insert(outpoint, entry);
    }
    if !decoder.remaining().is_empty() {
        return Err(SessionError::Snapshot("trailing snapshot bytes".to_owned()));
    }
    Ok(state)
}

fn put_bytes(output: &mut Vec<u8>, value: &[u8]) -> Result<()> {
    let len = u32::try_from(value.len()).map_err(|_| SessionError::Snapshot("snapshot field exceeds u32".to_owned()))?;
    output.extend_from_slice(&len.to_le_bytes());
    output.extend_from_slice(value);
    Ok(())
}

struct SnapshotDecoder<'a> {
    remaining: &'a [u8],
}

impl<'a> SnapshotDecoder<'a> {
    fn new(bytes: &'a [u8]) -> Self {
        Self { remaining: bytes }
    }

    fn take(&mut self, len: usize) -> Result<&'a [u8]> {
        if self.remaining.len() < len {
            return Err(SessionError::Snapshot("truncated snapshot".to_owned()));
        }
        let (value, remaining) = self.remaining.split_at(len);
        self.remaining = remaining;
        Ok(value)
    }

    fn byte(&mut self) -> Result<u8> {
        Ok(self.take(1)?[0])
    }

    fn u16(&mut self) -> Result<u16> {
        Ok(u16::from_le_bytes(self.take(2)?.try_into().expect("fixed length")))
    }

    fn u32(&mut self) -> Result<u32> {
        Ok(u32::from_le_bytes(self.take(4)?.try_into().expect("fixed length")))
    }

    fn u64(&mut self) -> Result<u64> {
        Ok(u64::from_le_bytes(self.take(8)?.try_into().expect("fixed length")))
    }

    fn bytes(&mut self) -> Result<&'a [u8]> {
        let len = self.u32()? as usize;
        if len > MAX_SNAPSHOT_FIELD_BYTES {
            return Err(SessionError::Snapshot("snapshot field limit exceeded".to_owned()));
        }
        self.take(len)
    }

    fn remaining(&self) -> &'a [u8] {
        self.remaining
    }
}

/// Session runtime failures.
#[derive(Debug, thiserror::Error)]
pub enum SessionError {
    /// Invalid immutable config.
    #[error("invalid session config: {0}")]
    InvalidConfig(String),
    /// Selected finality engine is inconsistent with the session.
    #[error("consensus kind mismatch: expected {expected}, got {actual}")]
    ConsensusKind {
        /// Required engine.
        expected: &'static str,
        /// Supplied engine.
        actual: &'static str,
    },
    /// Execution plugin rejected a request.
    #[error("transition execution failed: {0}")]
    Execution(String),
    /// State root does not match the durable head.
    #[error("state root mismatch: expected {expected:?}, got {actual:?}")]
    StateRootMismatch {
        /// Durable expected root.
        expected: Hash32,
        /// In-memory executor root.
        actual: Hash32,
    },
    /// Candidate was prepared on an older head.
    #[error("prepared block is stale")]
    StalePreparedBlock,
    /// A verifier adapter reported finality for different canonical bytes.
    #[error("finality verifier block hash mismatch: expected {expected:?}, got {actual:?}")]
    FinalityMismatch {
        /// Canonical candidate hash.
        expected: Hash32,
        /// Hash returned by the verifier adapter.
        actual: Hash32,
    },
    /// Height exhausted the session-local integer range.
    #[error("session height overflow")]
    HeightOverflow,
    /// Request exceeded configured resource bounds.
    #[error("block limit exceeded: {0}")]
    BlockLimit(String),
    /// Proposed timestamp violated monotonic/drift rules.
    #[error("invalid block timestamp: {0}")]
    InvalidTimestamp(String),
    /// Durable chain did not pass recovery audit.
    #[error("recovery audit failed: {0}")]
    Recovery(String),
    /// Snapshot is missing data or is malformed.
    #[error("invalid state snapshot: {0}")]
    Snapshot(String),
    /// Strict persistent record codec failure.
    #[error("record codec failed: {0}")]
    Codec(String),
    /// Consensus proof verification failed.
    #[error(transparent)]
    Consensus(#[from] myelin_consensus::ConsensusError),
    /// Atomic store operation failed.
    #[error(transparent)]
    Store(#[from] StoreError),
}

/// Session result alias.
pub type Result<T> = std::result::Result<T, SessionError>;

#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct RecordEnvelope<T> {
    format_version: u16,
    body: T,
    checksum: Hash32,
}

fn encode_record<T: Serialize>(body: &T) -> Result<Vec<u8>> {
    let body_bytes = postcard::to_allocvec(body).map_err(|error| SessionError::Codec(error.to_string()))?;
    let checksum = *blake3::hash(&body_bytes).as_bytes();
    let body_len = u64::try_from(body_bytes.len()).map_err(|_| SessionError::Codec("record body length overflow".to_owned()))?;
    let mut encoded = Vec::with_capacity(RECORD_HEADER_BYTES + body_bytes.len());
    encoded.extend_from_slice(RECORD_MAGIC);
    encoded.extend_from_slice(&RECORD_FORMAT_VERSION.to_le_bytes());
    encoded.extend_from_slice(&body_len.to_le_bytes());
    encoded.extend_from_slice(&checksum);
    encoded.extend_from_slice(&body_bytes);
    Ok(encoded)
}

fn decode_record<T: DeserializeOwned + Serialize>(bytes: &[u8]) -> Result<T> {
    if !bytes.starts_with(RECORD_MAGIC) {
        let envelope: RecordEnvelope<T> = serde_json::from_slice(bytes).map_err(|error| SessionError::Codec(error.to_string()))?;
        if envelope.format_version != LEGACY_JSON_RECORD_FORMAT_VERSION {
            return Err(SessionError::Codec(format!("unsupported legacy record version {}", envelope.format_version)));
        }
        let body_bytes = serde_json::to_vec(&envelope.body).map_err(|error| SessionError::Codec(error.to_string()))?;
        if *blake3::hash(&body_bytes).as_bytes() != envelope.checksum {
            return Err(SessionError::Codec("record checksum mismatch".to_owned()));
        }
        return Ok(envelope.body);
    }
    if bytes.len() < RECORD_HEADER_BYTES {
        return Err(SessionError::Codec("truncated binary record header".to_owned()));
    }
    let version = u16::from_le_bytes(bytes[4..6].try_into().expect("checked header"));
    if version != RECORD_FORMAT_VERSION {
        return Err(SessionError::Codec(format!("unsupported record version {version}")));
    }
    let body_len = u64::from_le_bytes(bytes[6..14].try_into().expect("checked header"));
    let body_len = usize::try_from(body_len).map_err(|_| SessionError::Codec("record body length overflow".to_owned()))?;
    if bytes.len() != RECORD_HEADER_BYTES.saturating_add(body_len) {
        return Err(SessionError::Codec("binary record length mismatch".to_owned()));
    }
    let expected_checksum: Hash32 = bytes[14..46].try_into().expect("checked header");
    let body_bytes = &bytes[RECORD_HEADER_BYTES..];
    if *blake3::hash(body_bytes).as_bytes() != expected_checksum {
        return Err(SessionError::Codec("record checksum mismatch".to_owned()));
    }
    postcard::from_bytes(body_bytes).map_err(|error| SessionError::Codec(error.to_string()))
}

#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct SessionConfigWire {
    session_id: Hash32,
    consensus_kind: String,
    consensus_config_commitment: Hash32,
    consensus_module_commitment: Hash32,
    consensus_wal_schema_hash: Hash32,
    initial_state_root: Hash32,
    max_block_transactions: u32,
    max_block_bytes: u64,
    max_future_drift_ms: u64,
}

#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct SessionGenesisWire {
    config: SessionConfigWire,
    state_snapshot: Vec<u8>,
}

#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct StateCheckpointWire {
    session_id: Hash32,
    finalised_height: Option<u64>,
    state_root: Hash32,
    state_snapshot: Vec<u8>,
}

impl From<&StateCheckpoint> for StateCheckpointWire {
    fn from(value: &StateCheckpoint) -> Self {
        Self {
            session_id: value.session_id,
            finalised_height: value.finalised_height,
            state_root: value.state_root,
            state_snapshot: value.state_snapshot.clone(),
        }
    }
}

impl From<StateCheckpointWire> for StateCheckpoint {
    fn from(value: StateCheckpointWire) -> Self {
        Self {
            session_id: value.session_id,
            finalised_height: value.finalised_height,
            state_root: value.state_root,
            state_snapshot: value.state_snapshot,
        }
    }
}

impl From<&SessionConfig> for SessionConfigWire {
    fn from(value: &SessionConfig) -> Self {
        Self {
            session_id: value.session_id,
            consensus_kind: value.consensus_kind.as_str().to_owned(),
            consensus_config_commitment: value.consensus_config_commitment,
            consensus_module_commitment: value.consensus_module_commitment,
            consensus_wal_schema_hash: value.consensus_wal_schema_hash,
            initial_state_root: value.initial_state_root,
            max_block_transactions: value.max_block_transactions,
            max_block_bytes: value.max_block_bytes,
            max_future_drift_ms: value.max_future_drift_ms,
        }
    }
}

impl TryFrom<SessionConfigWire> for SessionConfig {
    type Error = SessionError;

    fn try_from(value: SessionConfigWire) -> Result<Self> {
        Ok(Self {
            session_id: value.session_id,
            consensus_kind: parse_kind(&value.consensus_kind)?,
            consensus_config_commitment: value.consensus_config_commitment,
            consensus_module_commitment: value.consensus_module_commitment,
            consensus_wal_schema_hash: value.consensus_wal_schema_hash,
            initial_state_root: value.initial_state_root,
            max_block_transactions: value.max_block_transactions,
            max_block_bytes: value.max_block_bytes,
            max_future_drift_ms: value.max_future_drift_ms,
        })
    }
}

#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct SessionHeadWire {
    session_id: Hash32,
    finalised_height: Option<u64>,
    block_hash: Hash32,
    state_root: Hash32,
    timestamp_ms: u64,
}

impl From<&SessionHead> for SessionHeadWire {
    fn from(value: &SessionHead) -> Self {
        Self {
            session_id: value.session_id,
            finalised_height: value.finalised_height,
            block_hash: value.block_hash,
            state_root: value.state_root,
            timestamp_ms: value.timestamp_ms,
        }
    }
}

impl From<SessionHeadWire> for SessionHead {
    fn from(value: SessionHeadWire) -> Self {
        Self {
            session_id: value.session_id,
            finalised_height: value.finalised_height,
            block_hash: value.block_hash,
            state_root: value.state_root,
            timestamp_ms: value.timestamp_ms,
        }
    }
}

#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct BlockWire {
    version: u32,
    parent_hash: Hash32,
    number: u64,
    timestamp_ms: u64,
    consensus_kind: String,
    state_root_before: Hash32,
    state_root_after: Hash32,
    ordered_cell_tx_commitments: Vec<Hash32>,
    data_commitments: Vec<Hash32>,
    scheduler_commitment: Hash32,
}

impl From<&MyelinBlock> for BlockWire {
    fn from(value: &MyelinBlock) -> Self {
        Self {
            version: value.version,
            parent_hash: value.parent_hash,
            number: value.number,
            timestamp_ms: value.timestamp_ms,
            consensus_kind: value.consensus_kind.as_str().to_owned(),
            state_root_before: value.state_root_before,
            state_root_after: value.state_root_after,
            ordered_cell_tx_commitments: value.ordered_cell_tx_commitments.clone(),
            data_commitments: value.data_commitments.clone(),
            scheduler_commitment: value.scheduler_commitment,
        }
    }
}

impl TryFrom<BlockWire> for MyelinBlock {
    type Error = SessionError;

    fn try_from(value: BlockWire) -> Result<Self> {
        Ok(Self {
            version: value.version,
            parent_hash: value.parent_hash,
            number: value.number,
            timestamp_ms: value.timestamp_ms,
            consensus_kind: parse_kind(&value.consensus_kind)?,
            state_root_before: value.state_root_before,
            state_root_after: value.state_root_after,
            ordered_cell_tx_commitments: value.ordered_cell_tx_commitments,
            data_commitments: value.data_commitments,
            scheduler_commitment: value.scheduler_commitment,
        })
    }
}

#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct FinalisedBlockWire {
    block: BlockWire,
    consensus_module_commitment: Hash32,
    proof: Vec<u8>,
    transactions: Vec<Vec<u8>>,
    state_snapshot: Vec<u8>,
    outbox: Vec<OutboxMessage>,
}

impl TryFrom<&FinalisedBlockRecord> for FinalisedBlockWire {
    type Error = SessionError;

    fn try_from(value: &FinalisedBlockRecord) -> Result<Self> {
        Ok(Self {
            block: BlockWire::from(&value.block),
            consensus_module_commitment: value.consensus_module_commitment,
            proof: value.proof.encode()?,
            transactions: value.transactions.clone(),
            state_snapshot: value.state_snapshot.clone(),
            outbox: value.outbox.clone(),
        })
    }
}

impl TryFrom<FinalisedBlockWire> for FinalisedBlockRecord {
    type Error = SessionError;

    fn try_from(value: FinalisedBlockWire) -> Result<Self> {
        Ok(Self {
            block: MyelinBlock::try_from(value.block)?,
            consensus_module_commitment: value.consensus_module_commitment,
            proof: FinalityProof::decode(&value.proof)?,
            transactions: value.transactions,
            state_snapshot: value.state_snapshot,
            outbox: value.outbox,
        })
    }
}

fn parse_kind(value: &str) -> Result<ConsensusKind> {
    ConsensusKind::from_canonical_str(value).map_err(|error| SessionError::Codec(error.to_string()))
}

#[cfg(test)]
mod tests {
    use super::*;
    use myelin_consensus::{
        Authority, CommitteeSigner, ConsensusCatalog, ProofOfAuthority, ProofOfAuthorityConfig, ProofOfAuthoritySeal,
        SelectedConsensus,
    };
    use myelin_exec::{serialize_transaction_molecule, CellInput, Script};
    use std::sync::Mutex;

    #[derive(Clone)]
    struct MemoryStore {
        inner: Arc<Mutex<MemoryState>>,
    }

    #[derive(Default)]
    struct MemoryState {
        genesis: Option<SessionGenesis>,
        head: Option<SessionHead>,
        chain: Vec<FinalisedBlockRecord>,
    }

    impl MemoryStore {
        fn new() -> Self {
            Self { inner: Arc::new(Mutex::new(MemoryState::default())) }
        }
    }

    impl SessionStore for MemoryStore {
        fn create_session(&self, genesis: &SessionGenesis, head: &SessionHead) -> std::result::Result<(), StoreError> {
            let mut state = self.inner.lock().unwrap();
            if state.genesis.is_some() {
                return Err(StoreError::Conflict("already exists".to_owned()));
            }
            state.genesis = Some(genesis.clone());
            state.head = Some(head.clone());
            Ok(())
        }

        fn load_genesis(&self, _session_id: Hash32) -> std::result::Result<SessionGenesis, StoreError> {
            self.inner.lock().unwrap().genesis.clone().ok_or_else(|| StoreError::NotFound("genesis".to_owned()))
        }

        fn load_head(&self, _session_id: Hash32) -> std::result::Result<SessionHead, StoreError> {
            self.inner.lock().unwrap().head.clone().ok_or_else(|| StoreError::NotFound("head".to_owned()))
        }

        fn load_chain(&self, _session_id: Hash32) -> std::result::Result<Vec<FinalisedBlockRecord>, StoreError> {
            Ok(self.inner.lock().unwrap().chain.clone())
        }

        fn commit_block(
            &self,
            expected_head: &SessionHead,
            new_head: &SessionHead,
            record: &FinalisedBlockRecord,
        ) -> std::result::Result<(), StoreError> {
            let mut state = self.inner.lock().unwrap();
            if state.head.as_ref() != Some(expected_head) {
                return Err(StoreError::Conflict("stale head".to_owned()));
            }
            state.chain.push(record.clone());
            state.head = Some(new_head.clone());
            Ok(())
        }

        fn load_consensus_wal(&self, _session_id: Hash32) -> std::result::Result<Option<ConsensusWal>, StoreError> {
            Ok(None)
        }

        fn compare_and_set_consensus_wal(
            &self,
            _expected_revision: Option<u64>,
            _wal: &ConsensusWal,
        ) -> std::result::Result<(), StoreError> {
            Ok(())
        }

        fn pending_outbox(&self, _session_id: Hash32, _limit: usize) -> std::result::Result<Vec<PendingDelivery>, StoreError> {
            Ok(Vec::new())
        }

        fn acknowledge_outbox(&self, _session_id: Hash32, _message_id: Hash32) -> std::result::Result<(), StoreError> {
            Ok(())
        }
    }

    fn cell_output(capacity: u64) -> CellOutput {
        CellOutput { capacity, lock: Script::new([7; 32], 0, vec![]), type_: None }
    }

    fn seeded_executor() -> (CellTransitionExecutor, myelin_exec::OutPoint) {
        let outpoint = myelin_exec::OutPoint::new([8; 32], 0);
        let mut state = CellStateTree::new();
        state.insert(outpoint, CellEntry::from_output(&cell_output(1_000), &[], 0, false).unwrap());
        (CellTransitionExecutor::new(state, Arc::new(|_: &CellTx, _: &[ResolvedStateInput]| Ok(10))), outpoint)
    }

    #[derive(Clone)]
    struct TestFinalityVerifier(SelectedConsensus);

    impl FinalityVerifier for TestFinalityVerifier {
        fn descriptor(&self) -> ConsensusModuleDescriptor {
            self.0.module_descriptor()
        }

        fn verify(
            &self,
            block: &MyelinBlock,
            proof: &FinalityProof,
        ) -> std::result::Result<VerifiedFinality, myelin_consensus::ConsensusError> {
            let finalised = self.0.finalise_with_proof(block.clone(), proof.clone())?;
            Ok(VerifiedFinality { block_hash: finalised.block_hash, consensus_module_commitment: self.0.module_commitment() })
        }
    }

    fn poa() -> (Arc<TestFinalityVerifier>, CommitteeSigner) {
        let signer = CommitteeSigner::new("alice", [3; 32]).unwrap();
        let config =
            ProofOfAuthorityConfig { authorities: vec![Authority { id: "alice".to_owned(), public_key: signer.ckb_public_key() }] };
        (Arc::new(TestFinalityVerifier(SelectedConsensus::ProofOfAuthority(ProofOfAuthority::new(config).unwrap()))), signer)
    }

    #[test]
    fn continuous_chain_advances_and_recovers() {
        let (executor, input) = seeded_executor();
        let root = executor.state_root();
        let (verifier, signer) = poa();
        let store = Arc::new(MemoryStore::new());
        let config = SessionConfig {
            session_id: [4; 32],
            consensus_kind: ConsensusKind::ProofOfAuthority,
            consensus_config_commitment: verifier.descriptor().config_commitment,
            consensus_module_commitment: verifier.descriptor().commitment(),
            consensus_wal_schema_hash: verifier.descriptor().wal_schema_hash,
            initial_state_root: root,
            max_block_transactions: 100,
            max_block_bytes: 1_000_000,
            max_future_drift_ms: 1_000,
        };
        let mut chain = SessionChain::create(config, verifier.clone(), executor, Arc::clone(&store)).unwrap();
        let tx = CellTx::new(vec![CellInput::new(input, 0)], vec![], vec![cell_output(900)], vec![vec![]], vec![]).unwrap();
        let prepared = chain
            .prepare_block(BlockRequest {
                transactions: vec![serialize_transaction_molecule(&tx).unwrap()],
                scheduler_commitment: [5; 32],
                data_commitments: vec![],
                timestamp_ms: 10,
                local_now_ms: 10,
            })
            .unwrap();
        let engine = match &verifier.0 {
            SelectedConsensus::ProofOfAuthority(engine) => engine,
            _ => unreachable!(),
        };
        let seal = engine.seal_from_signer(prepared.block().hash(), 0, &signer).unwrap();
        let head = chain.commit_finalised(prepared, FinalityProof::ProofOfAuthority(seal)).unwrap();
        assert_eq!(head.finalised_height, Some(0));
        assert_ne!(head.state_root, root);

        let (_, snapshot) = SessionChain::<CellTransitionExecutor, MemoryStore>::recovery_snapshot(&store, [4; 32]).unwrap();
        let restored =
            CellTransitionExecutor::from_snapshot(&snapshot, Arc::new(|_: &CellTx, _: &[ResolvedStateInput]| Ok(10))).unwrap();
        let wrong_signer = CommitteeSigner::new("alice", [9; 32]).unwrap();
        let wrong_verifier = Arc::new(TestFinalityVerifier(SelectedConsensus::ProofOfAuthority(
            ProofOfAuthority::new(ProofOfAuthorityConfig {
                authorities: vec![Authority { id: "alice".to_owned(), public_key: wrong_signer.ckb_public_key() }],
            })
            .unwrap(),
        )));
        assert!(matches!(
            SessionChain::recover([4; 32], wrong_verifier, restored.clone(), Arc::clone(&store)),
            Err(SessionError::InvalidConfig(_))
        ));
        let recovered = SessionChain::recover([4; 32], verifier, restored, store).unwrap();
        assert_eq!(recovered.head(), &head);
    }

    #[test]
    fn snapshot_roundtrip_rehashes_full_cell_metadata() {
        let (executor, _) = seeded_executor();
        let snapshot = executor.snapshot().unwrap();
        let restored = decode_cell_snapshot(&snapshot).unwrap();
        assert_eq!(restored.len(), 1);
        let mut restored_for_root = restored;
        assert_eq!(restored_for_root.root().as_bytes(), executor.state_root());
    }

    #[test]
    fn record_codec_rejects_tampering() {
        let config = SessionConfig {
            session_id: [1; 32],
            consensus_kind: ConsensusKind::ProofOfAuthority,
            consensus_config_commitment: [3; 32],
            consensus_module_commitment: [4; 32],
            consensus_wal_schema_hash: [5; 32],
            initial_state_root: [2; 32],
            max_block_transactions: 10,
            max_block_bytes: 100,
            max_future_drift_ms: 1,
        };
        let mut encoded = config.encode().unwrap();
        let last = encoded.len() - 2;
        encoded[last] ^= 1;
        assert!(SessionConfig::decode(&encoded).is_err());
    }

    #[test]
    fn binary_record_codec_reads_legacy_json_v4() {
        let config = SessionConfig {
            session_id: [1; 32],
            consensus_kind: ConsensusKind::ProofOfAuthority,
            consensus_config_commitment: [3; 32],
            consensus_module_commitment: [4; 32],
            consensus_wal_schema_hash: [5; 32],
            initial_state_root: [2; 32],
            max_block_transactions: 10,
            max_block_bytes: 100,
            max_future_drift_ms: 1,
        };
        let body = SessionConfigWire::from(&config);
        let checksum = *blake3::hash(&serde_json::to_vec(&body).unwrap()).as_bytes();
        let legacy =
            serde_json::to_vec(&RecordEnvelope { format_version: LEGACY_JSON_RECORD_FORMAT_VERSION, body, checksum }).unwrap();
        assert_eq!(SessionConfig::decode(&legacy).unwrap(), config);
        assert!(config.encode().unwrap().starts_with(RECORD_MAGIC));
    }

    #[derive(Clone)]
    struct DescriptorOnlyVerifier {
        descriptor: ConsensusModuleDescriptor,
        returned_block_hash: Hash32,
    }

    impl FinalityVerifier for DescriptorOnlyVerifier {
        fn descriptor(&self) -> ConsensusModuleDescriptor {
            self.descriptor.clone()
        }

        fn verify(
            &self,
            _block: &MyelinBlock,
            _proof: &FinalityProof,
        ) -> std::result::Result<VerifiedFinality, myelin_consensus::ConsensusError> {
            Ok(VerifiedFinality { block_hash: self.returned_block_hash, consensus_module_commitment: self.descriptor.commitment() })
        }
    }

    #[test]
    fn finality_port_rejects_an_adapter_result_for_different_block_bytes() {
        let (executor, _) = seeded_executor();
        let descriptor = ConsensusCatalog::descriptor(ConsensusKind::ProofOfAuthority, [7; 32]);
        let verifier = Arc::new(DescriptorOnlyVerifier { descriptor: descriptor.clone(), returned_block_hash: [9; 32] });
        let config = SessionConfig {
            session_id: [6; 32],
            consensus_kind: descriptor.consensus_kind,
            consensus_config_commitment: descriptor.config_commitment,
            consensus_module_commitment: descriptor.commitment(),
            consensus_wal_schema_hash: descriptor.wal_schema_hash,
            initial_state_root: executor.state_root(),
            max_block_transactions: 1,
            max_block_bytes: 1,
            max_future_drift_ms: 1,
        };
        let mut chain = SessionChain::create(config, verifier, executor, Arc::new(MemoryStore::new())).unwrap();
        let prepared = chain
            .prepare_block(BlockRequest {
                transactions: vec![],
                scheduler_commitment: [5; 32],
                data_commitments: vec![],
                timestamp_ms: 1,
                local_now_ms: 1,
            })
            .unwrap();
        let proof = FinalityProof::ProofOfAuthority(ProofOfAuthoritySeal {
            block_hash: prepared.block().hash(),
            height: 0,
            authority_id: "test".to_owned(),
            signature: [1; 65],
        });
        assert!(matches!(chain.commit_finalised(prepared, proof), Err(SessionError::FinalityMismatch { .. })));
    }

    #[test]
    fn prepared_workload_is_invariant_across_registered_finality_kinds() {
        let (executor, input) = seeded_executor();
        let tx = CellTx::new(vec![CellInput::new(input, 0)], vec![], vec![cell_output(900)], vec![vec![]], vec![]).unwrap();
        let transaction = serialize_transaction_molecule(&tx).unwrap();
        let kinds = [ConsensusKind::StaticClosedCommittee, ConsensusKind::ProofOfAuthority, ConsensusKind::Tendermint];
        let mut blocks = Vec::new();
        for (index, kind) in kinds.into_iter().enumerate() {
            let descriptor = ConsensusCatalog::descriptor(kind, [7; 32]);
            let verifier = Arc::new(DescriptorOnlyVerifier { descriptor: descriptor.clone(), returned_block_hash: [0; 32] });
            let config = SessionConfig {
                session_id: [10 + index as u8; 32],
                consensus_kind: kind,
                consensus_config_commitment: descriptor.config_commitment,
                consensus_module_commitment: descriptor.commitment(),
                consensus_wal_schema_hash: descriptor.wal_schema_hash,
                initial_state_root: executor.state_root(),
                max_block_transactions: 10,
                max_block_bytes: 1_000_000,
                max_future_drift_ms: 1,
            };
            let chain = SessionChain::create(config, verifier, executor.clone(), Arc::new(MemoryStore::new())).unwrap();
            blocks.push(
                chain
                    .prepare_block(BlockRequest {
                        transactions: vec![transaction.clone()],
                        scheduler_commitment: [5; 32],
                        data_commitments: vec![[6; 32]],
                        timestamp_ms: 1,
                        local_now_ms: 1,
                    })
                    .unwrap()
                    .block()
                    .clone(),
            );
        }

        for block in &blocks[1..] {
            assert_eq!(block.state_root_before, blocks[0].state_root_before);
            assert_eq!(block.state_root_after, blocks[0].state_root_after);
            assert_eq!(block.ordered_cell_tx_commitments, blocks[0].ordered_cell_tx_commitments);
            assert_eq!(block.data_commitments, blocks[0].data_commitments);
            assert_eq!(block.scheduler_commitment, blocks[0].scheduler_commitment);
        }
        assert_ne!(blocks[0].hash(), blocks[1].hash());
        assert_ne!(blocks[0].hash(), blocks[2].hash());
        assert_ne!(blocks[1].hash(), blocks[2].hash());
    }
}
