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

mod application;
#[path = "codec.rs"]
pub mod bounded_codec;
use bounded_codec as codec;
mod commands;
mod continuity;
mod effects;
mod evidence;
mod lifecycle;
mod record_size;
mod recovery;
mod remote;
mod remote_terminal;
mod replay;
mod resources;
mod succession;

pub use application::{
    ApplicationProfile, ApplicationResourceEnvelope, ApplicationVmProfile, ExecutionFrame, ExecutionResources, FrameInput,
    InspectContext, InspectPort, InspectionReceipt,
};
pub use commands::{CommandIdentity, CommandReceipt, CommandStatus};
pub use continuity::{
    EvidenceRequirement, HandoffIntent, HandoffTarget, PredecessorReference, SessionHandoff, StoredHandoff, SuccessorDeclaration,
};
pub use effects::{
    validate_handoff_source, validate_handoff_trust, EffectAuthorizationScope, ProtocolEffectsContext, ProtocolEffectsPolicy,
};
pub use evidence::{EvidencePipelineDescriptor, EvidenceReceipt, EvidenceRecord, EvidenceStage};
pub use lifecycle::{
    audit_handoff_lifecycle, continuation_lineage, handoff_owner_lineage, validate_consumption_record, validate_handoff_action,
    HandoffAction, HandoffBlockReference, HandoffLifecycle, HandoffRetentionPolicy, HandoffTerminal, ResolvedHandoffAction,
    RetainUntilTerminal, MAX_HANDOFF_OWNER_DEPTH,
};
pub use recovery::{
    PausedRecovery, RecoveryAudit, RecoveryBudget, RecoveryCancellation, RecoveryOutcome, RecoveryPauseReason, RecoveryProgress,
    RecoveryUsage,
};
pub use remote::{
    resolve_handoff_source, validate_handoff_evidence, verify_remote_block, HandoffEvidenceProof, HandoffImportKind,
    HandoffImportPolicy, HandoffImportRegistry, HandoffImportVerifier, HandoffInboxRecord, HandoffProofBundle,
    RemoteFinalityVerifiers, ResolvedHandoffSource, VerifiedHandoffImport, MAX_REMOTE_PROOF_BYTES, REMOTE_HANDOFF_FORMAT_VERSION,
};
pub use remote_terminal::{
    remote_cancellation_digest, validate_remote_consumption, RemoteConsumptionProof, RemoteTerminalCertificate,
    RemoteTerminalDecision, RemoteTerminalPolicy, VerifiedRemoteTerminal, MAX_REMOTE_TERMINAL_BYTES, REMOTE_TERMINAL_VERSION,
};
pub use replay::{PausedReplay, RangeReplay, RangeReplayReceipt, ReplayAnchor, ReplayOutcome};
pub use resources::{CommitResourceLimits, HandoffUsage};
pub use succession::{MigrationArtifact, SuccessorState, SuccessorTransition};

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
pub const RECORD_FORMAT_VERSION: u16 = 14;
pub const SESSION_BLOCK_VERSION: u32 = 6;
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
    /// Complete session-wide application semantics and resource identity.
    pub application_profile: ApplicationProfile,
    /// Exact state root from which height zero executes.
    pub initial_state_root: Hash32,
    /// First input position accepted by height zero.
    pub initial_input_position: u64,
    /// Logical time visible at height zero.
    pub initial_logical_time: u64,
    /// Timestamp floor inherited by height zero (zero for an independent genesis).
    pub initial_timestamp_ms: u64,
    /// Reverse lineage for an atomically created successor.
    pub predecessor: Option<PredecessorReference>,
    /// Maximum number of transactions admitted to one block.
    pub max_block_transactions: u32,
    /// Maximum aggregate encoded transaction bytes in one block.
    pub max_block_bytes: u64,
    /// Maximum permitted proposed timestamp drift beyond the local clock.
    pub max_future_drift_ms: u64,
    /// Mandatory aggregate transition bounds.
    pub commit_limits: CommitResourceLimits,
    /// Source-effect trust model fixed at genesis.
    pub protocol_effects: ProtocolEffectsPolicy,
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
        self.application_profile.validate()?;
        self.protocol_effects.validate()?;
        if let Some(terminal) = self.protocol_effects.remote_terminal() {
            if self.predecessor.is_some() || (self.session_id != terminal.source_session && self.session_id != terminal.target_session)
            {
                return Err(SessionError::InvalidConfig("remote terminal policy requires its fixed original source/target".into()));
            }
        }
        self.commit_limits.validate()?;
        resources::check("transactions", self.max_block_transactions as u64, codec::MAX_SEQUENCE_ITEMS as u64)?;
        resources::check("transaction_bytes", self.max_block_bytes, codec::MAX_RECORD_BYTES as u64)?;
        if let Some(predecessor) = &self.predecessor {
            if predecessor.session_id == [0; 32]
                || predecessor.session_id == self.session_id
                || predecessor.final_state_root == [0; 32]
                || predecessor.application_profile_commitment == [0; 32]
            {
                return Err(SessionError::InvalidConfig("invalid predecessor lineage reference".to_owned()));
            }
        }
        if self.max_block_transactions == 0 {
            return Err(SessionError::InvalidConfig("max_block_transactions must be non-zero".to_owned()));
        }
        if self.max_block_bytes == 0 {
            return Err(SessionError::InvalidConfig("max_block_bytes must be non-zero".to_owned()));
        }
        Ok(())
    }

    /// Encode the current persistence format.
    pub fn encode(&self) -> Result<Vec<u8>> {
        self.validate()?;
        encode_record(&SessionConfigWire::from(self))
    }
    /// Decode only the current config format.
    pub fn decode(bytes: &[u8]) -> Result<Self> {
        let config = Self::try_from(decode_record::<SessionConfigWire>(bytes)?)?;
        config.validate()?;
        Ok(config)
    }

    /// Canonical immutable session configuration commitment used by successor
    /// declarations and cross-session audit material.
    pub fn commitment(&self) -> Hash32 {
        let mut hasher = blake3::Hasher::new();
        hasher.update(b"myelin:session-config:v3");
        hasher.update(&self.session_id);
        hasher.update(self.consensus_kind.as_str().as_bytes());
        hasher.update(&self.consensus_config_commitment);
        hasher.update(&self.consensus_module_commitment);
        hasher.update(&self.consensus_wal_schema_hash);
        hasher.update(&self.application_profile.commitment());
        hasher.update(&self.initial_state_root);
        hasher.update(&self.initial_input_position.to_le_bytes());
        hasher.update(&self.initial_logical_time.to_le_bytes());
        hasher.update(&self.initial_timestamp_ms.to_le_bytes());
        match &self.predecessor {
            Some(predecessor) => {
                hasher.update(&[1]);
                hasher.update(&predecessor.session_id);
                hasher.update(&predecessor.final_height.to_le_bytes());
                hasher.update(&predecessor.final_state_root);
                hasher.update(&predecessor.application_profile_commitment);
            }
            None => {
                hasher.update(&[0]);
            }
        }
        hasher.update(&self.max_block_transactions.to_le_bytes());
        hasher.update(&self.max_block_bytes.to_le_bytes());
        hasher.update(&self.max_future_drift_ms.to_le_bytes());
        hasher.update(&self.commit_limits.commitment());
        hasher.update(&self.protocol_effects.commitment());
        *hasher.finalize().as_bytes()
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
    /// Exclusive end of the last finalised application input range.
    pub input_position: u64,
    /// Exclusive logical-time end of the last finalised execution frame.
    pub logical_time: u64,
    /// Target session id once a final successor declaration seals this chain.
    pub sealed_by_successor: Option<Hash32>,
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
    #[serde(deserialize_with = "crate::codec::bounded_string")]
    pub topic: String,
    /// Opaque, versioned plugin payload.
    #[serde(deserialize_with = "crate::codec::bounded_bytes")]
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
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct PendingOutboxMessage {
    /// Namespaced plugin topic.
    #[serde(deserialize_with = "crate::codec::bounded_string")]
    pub topic: String,
    /// Opaque plugin payload.
    #[serde(deserialize_with = "crate::codec::bounded_bytes")]
    pub payload: Vec<u8>,
}

/// Result of executing an ordered transaction batch on a cloned state.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ExecutionOutcome {
    /// Raw transaction identifiers, in execution order.
    #[serde(deserialize_with = "crate::codec::bounded_vec")]
    pub ordered_cell_tx_commitments: Vec<Hash32>,
    /// Application/DA commitments produced while executing the batch.
    #[serde(deserialize_with = "crate::codec::bounded_vec")]
    pub data_commitments: Vec<Hash32>,
    /// External effects to atomically enqueue with the block.
    #[serde(deserialize_with = "crate::codec::bounded_vec")]
    pub outbox: Vec<PendingOutboxMessage>,
    /// Aggregate CKB-VM cycles consumed by the ordered transaction batch.
    pub cycles: u64,
}

/// Pluggable deterministic state transition implementation.
///
/// The runtime clones an executor, mutates only the clone, commits its snapshot,
/// and swaps it into memory only after the durable CAS succeeds.
pub trait TransitionExecutor: Clone + Send + Sync + 'static {
    /// Current canonical state root.
    fn state_root(&self) -> Hash32;

    /// Execute one ordered block against the current root.
    fn execute_block(
        &mut self,
        height: u64,
        frame_input: &FrameInput,
        consumed_handoffs: &[SessionHandoff],
        transactions: &[Vec<u8>],
    ) -> std::result::Result<ExecutionOutcome, String>;

    /// Execute with replayable source intents. The default rejects issuance or
    /// migration; overriding this method is an explicit application policy.
    /// The override must authenticate the complete intent and enforce its state
    /// disposition. Both live execution and v2-block replay call this method.
    fn execute_block_with_context(
        &mut self,
        context: &ProtocolEffectsContext,
        frame_input: &FrameInput,
        consumed_handoffs: &[SessionHandoff],
        transactions: &[Vec<u8>],
    ) -> std::result::Result<ExecutionOutcome, String> {
        context.validate().map_err(|e| e.to_string())?;
        if context.has_source_effects() {
            return Err("executor has no source issuance or migration policy".into());
        }
        self.execute_block(context.height, frame_input, consumed_handoffs, transactions)
    }

    /// Executors must explicitly implement lifecycle authorization before
    /// accepting cancellation, refunds or retirement.
    fn execute_block_with_lifecycle(
        &mut self,
        context: &ProtocolEffectsContext,
        actions: &[ResolvedHandoffAction],
        frame: &FrameInput,
        consumed: &[SessionHandoff],
        transactions: &[Vec<u8>],
    ) -> std::result::Result<ExecutionOutcome, String> {
        if !actions.is_empty() {
            return Err("executor has no lifecycle policy".into());
        }
        self.execute_block_with_context(context, frame, consumed, transactions)
    }

    /// Authorize and deterministically derive a successor after source execution.
    /// The default only supports identical application/effect semantics. Overrides
    /// must check compatibility vectors or execute the pinned migration artifact,
    /// authenticate all migration inputs, and preserve outstanding obligations.
    /// Offline replay calls this same hook and compares the entire receipt.
    fn prepare_successor_state(
        &self,
        _source: &SessionConfig,
        _target: &SessionConfig,
        _input: &FrameInput,
    ) -> std::result::Result<SuccessorState, String> {
        Ok(SuccessorState::Continue)
    }

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
    /// Canonical application input range and payload represented by this block.
    pub frame_input: FrameInput,
    /// Commitment to scheduler sidecar evidence; never placed in witnesses.
    pub scheduler_commitment: Hash32,
    /// Additional DA commitments known before execution.
    pub data_commitments: Vec<Hash32>,
    /// Proposed block time in milliseconds.
    pub timestamp_ms: u64,
    /// Local wall-clock time used only for bounded future-drift validation.
    pub local_now_ms: u64,
    /// If present, this is the final block and atomically creates the target
    /// session from its post-state snapshot.
    pub successor: Option<SessionConfig>,
    /// New transferable payloads committed by this source block.
    pub handoffs: Vec<HandoffIntent>,
    /// Previously finalised handoffs consumed exactly once by this block.
    pub consume_handoffs: Vec<Hash32>,
    /// Source cancellation/refund operations, committed with this block.
    pub handoff_actions: Vec<HandoffAction>,
}

/// Candidate block and isolated post-state awaiting an external finality proof.
pub struct PreparedBlock<E: TransitionExecutor> {
    block: MyelinBlock,
    base_head: SessionHead,
    executor: E,
    transactions: Vec<Vec<u8>>,
    snapshot: Vec<u8>,
    outbox: Vec<OutboxMessage>,
    frame: ExecutionFrame,
    frame_input: FrameInput,
    successor: Option<SuccessorDeclaration>,
    issued_handoffs: Vec<SessionHandoff>,
    consumed_handoffs: Vec<SessionHandoff>,
    handoff_actions: Vec<ResolvedHandoffAction>,
    request_data_commitments: Vec<Hash32>,
    remote_terminals: Vec<RemoteTerminalCertificate>,
}

impl<E: TransitionExecutor> PreparedBlock<E> {
    /// Candidate block to propose and sign through the selected consensus plugin.
    /// Attach separately signed terminal evidence without changing the candidate hash.
    /// Commit rechecks every certificate against local import trust and this exact block.
    pub fn with_remote_terminals(mut self, certificates: Vec<RemoteTerminalCertificate>) -> Self {
        self.remote_terminals = certificates;
        self
    }

    pub fn block(&self) -> &MyelinBlock {
        &self.block
    }

    /// Build the exact retained candidate for terminal-authority admission. This
    /// does not commit it. The target finality proof is still independently checked.
    pub fn candidate_record(&self, proof: FinalityProof, module: Hash32) -> FinalisedBlockRecord {
        FinalisedBlockRecord {
            block: self.block.clone(),
            consensus_module_commitment: module,
            proof,
            transactions: self.transactions.clone(),
            state_snapshot: self.snapshot.clone(),
            outbox: self.outbox.clone(),
            frame: self.frame.clone(),
            frame_input: self.frame_input.clone(),
            successor: self.successor.clone(),
            issued_handoffs: self.issued_handoffs.clone(),
            consumed_handoff_ids: self.consumed_handoffs.iter().map(SessionHandoff::id).collect(),
            request_data_commitments: self.request_data_commitments.clone(),
            resource_snapshot_bytes: self.snapshot.len() as u64,
            handoff_actions: self.handoff_actions.clone(),
            remote_terminals: self.remote_terminals.clone(),
        }
    }

    /// Exact head used for construction and execution.
    pub fn base_head(&self) -> &SessionHead {
        &self.base_head
    }

    /// Deterministic outbox messages committed by the candidate.
    pub fn outbox(&self) -> &[OutboxMessage] {
        &self.outbox
    }

    /// Exact application execution frame committed by the candidate block.
    pub fn frame(&self) -> &ExecutionFrame {
        &self.frame
    }

    pub fn successor(&self) -> Option<&SuccessorDeclaration> {
        self.successor.as_ref()
    }

    pub fn issued_handoffs(&self) -> &[SessionHandoff] {
        &self.issued_handoffs
    }

    pub fn consumed_handoffs(&self) -> &[SessionHandoff] {
        &self.consumed_handoffs
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
    /// Application execution frame committed by the block.
    pub frame: ExecutionFrame,
    /// Canonical application input retained for bounded replay.
    pub frame_input: FrameInput,
    /// Optional declaration that seals this session and creates its successor.
    pub successor: Option<SuccessorDeclaration>,
    /// Source-committed handoffs made available by this block.
    pub issued_handoffs: Vec<SessionHandoff>,
    /// Handoffs atomically consumed by this block.
    pub consumed_handoff_ids: Vec<Hash32>,
    /// Commitments supplied with the request before deterministic execution
    /// adds application/outbox commitments.
    pub request_data_commitments: Vec<Hash32>,
    /// Original snapshot length, committed in block data even when storage
    /// elides a non-checkpoint snapshot.
    pub resource_snapshot_bytes: u64,
    /// Replayable lifecycle witnesses in the current record format.
    pub handoff_actions: Vec<ResolvedHandoffAction>,
    /// External consume authorizations bind the exact signed candidate. They
    /// are retained for commit, recovery and offline application replay.
    pub remote_terminals: Vec<RemoteTerminalCertificate>,
}

impl FinalisedBlockRecord {
    /// Encode a checksummed, versioned persistence record.
    pub fn encode(&self) -> Result<Vec<u8>> {
        self.encode_borrowed()
    }

    /// Decode a stored record, checking its checksum and strict schema.
    pub fn decode(bytes: &[u8]) -> Result<Self> {
        let record = Self::try_from(decode_record::<FinalisedBlockWire>(bytes)?)?;
        record.check_successor_format()?;
        Ok(record)
    }

    /// Reject oversized imports before deserializing their nested containers.
    pub fn decode_for_config(bytes: &[u8], config: &SessionConfig) -> Result<Self> {
        config.validate()?;
        {
            let limits = &config.commit_limits;
            resources::check("record_bytes", bytes.len() as u64, limits.max_record_bytes)?;
        }
        let record = Self::decode(bytes)?;
        record.validate_resources(config)?;
        Ok(record)
    }

    /// Reconstruct the original source request from committed record fields.
    pub fn effects_context(&self, config: &SessionConfig) -> Result<ProtocolEffectsContext> {
        ProtocolEffectsContext::new(
            config,
            self.block.number,
            self.block.timestamp_ms,
            self.block.state_root_before,
            &self.frame_input,
            self.issued_handoffs.iter().map(effects::intent_from_handoff).collect(),
            self.successor.as_ref().map(|s| &s.target_config),
        )
    }

    pub fn validate_resources(&self, config: &SessionConfig) -> Result<()> {
        if self.block.version != SESSION_BLOCK_VERSION {
            return Err(SessionError::Codec("unsupported session block version".into()));
        }
        self.check_successor_format()?;
        remote_terminal::validate_terminal_envelope(self)?;
        if let Some(successor) = &self.successor {
            successor.validate_source(config, self.frame_input.root())?;
            if !self.state_snapshot.is_empty() {
                successor.target_snapshot(&self.state_snapshot)?;
            }
        }
        {
            let limits = &config.commit_limits;
            let count = self
                .consumed_handoff_ids
                .len()
                .checked_add(self.handoff_actions.len())
                .ok_or_else(|| SessionError::BlockLimit("handoff/action count overflow".into()))?;
            resources::check("consumed_handoffs", count as u64, limits.max_consumed_handoffs)?;
        }
        if !self.handoff_actions.is_empty() {
            if config.protocol_effects.application_policy_hash().is_none() {
                return Err(SessionError::Continuity("lifecycle actions require an application policy".into()));
            }
            let mut seen = self.consumed_handoff_ids.iter().copied().collect::<HashSet<_>>();
            for action in &self.handoff_actions {
                action.validate_for(self.block.session_id, self.block.number)?;
                if !seen.insert(action.action.handoff_id()) {
                    return Err(SessionError::Continuity("duplicate or consumed lifecycle action".into()));
                }
            }
            if self.block.data_commitments.len().checked_sub(3).and_then(|i| self.block.data_commitments.get(i))
                != Some(&lifecycle::actions_commitment(self.block.session_id, self.block.number, &self.handoff_actions)?)
            {
                return Err(SessionError::Continuity("lifecycle commitment missing or changed".into()));
            }
        }
        {
            let limits = &config.commit_limits;
            resources::check("transactions", self.transactions.len() as u64, config.max_block_transactions as u64)?;
            let bytes = self.transactions.iter().try_fold(0u64, |sum, tx| {
                sum.checked_add(tx.len() as u64).ok_or_else(|| SessionError::BlockLimit("transaction bytes overflow".into()))
            })?;
            resources::check("transaction_bytes", bytes, config.max_block_bytes)?;
            limits.record(self)?;
            {
                let context = self.effects_context(config)?;
                let index = self.block.data_commitments.len().checked_sub(2);
                if index.and_then(|i| self.block.data_commitments.get(i)) != Some(&context.commitment()?) {
                    return Err(SessionError::ProtocolEffects("source effect context commitment changed".into()));
                }
            }
            Ok(())
        }
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
        encode_record(&(self.session_id, self.finalised_height, self.state_root, &self.state_snapshot))
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
        self.config.validate()?;
        self.config.commit_limits.snapshot(&self.state_snapshot)?;
        encode_record(&(SessionConfigWire::from(&self.config), &self.state_snapshot))
    }
    /// Decode immutable genesis material in the current format.
    pub fn decode(bytes: &[u8]) -> Result<Self> {
        let wire: SessionGenesisWire = decode_record(bytes)?;
        let genesis = Self { config: SessionConfig::try_from(wire.config)?, state_snapshot: wire.state_snapshot };
        genesis.config.validate()?;
        genesis.config.commit_limits.snapshot(&genesis.state_snapshot)?;
        Ok(genesis)
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
    #[serde(deserialize_with = "crate::codec::bounded_bytes")]
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
    /// A bounded recovery read reached a local work limit.
    #[error("recovery read budget: {0:?}")]
    ReadBudget(RecoveryPauseReason),
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
    /// Load a bounded page beginning at `start_height`. Backends must implement
    /// this without materializing the complete history. Only an empty page ends
    /// iteration; a byte budget may return fewer records than `limit`. Returned
    /// records together must fit the 128 MiB codec ceiling, before decoding.
    fn load_chain_page(
        &self,
        session_id: Hash32,
        start_height: u64,
        limit: usize,
    ) -> std::result::Result<Vec<FinalisedBlockRecord>, StoreError>;
    /// Load one exact finalised block.
    fn load_block(&self, session_id: Hash32, height: u64) -> std::result::Result<FinalisedBlockRecord, StoreError> {
        self.load_chain_page(session_id, height, 1)?
            .into_iter()
            .find(|record| record.block.number == height)
            .ok_or_else(|| StoreError::NotFound(format!("session block {height}")))
    }
    /// Load a receipt from the backend's canonical command index.
    fn load_command_receipt(&self, session_id: Hash32, command_id: Hash32) -> std::result::Result<Option<CommandReceipt>, StoreError>;
    /// Load the latest full restart checkpoint.
    fn load_checkpoint(&self, session_id: Hash32) -> std::result::Result<StateCheckpoint, StoreError>;
    /// Load the newest retained checkpoint strictly before `height`, or genesis.
    fn load_checkpoint_before(&self, session_id: Hash32, height: u64) -> std::result::Result<StateCheckpoint, StoreError>;
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
    /// Load the durable, locally verified receipt chain for one outbox item.
    fn load_evidence(&self, session_id: Hash32, message_id: Hash32) -> std::result::Result<Option<EvidenceRecord>, StoreError> {
        let _ = (session_id, message_id);
        Ok(None)
    }
    /// Atomically append one verified receipt using the record revision as a CAS.
    fn compare_and_set_evidence(
        &self,
        expected_revision: Option<u64>,
        record: &EvidenceRecord,
    ) -> std::result::Result<(), StoreError> {
        let _ = (expected_revision, record);
        Err(StoreError::Backend("evidence persistence is not implemented by this store".to_owned()))
    }
    /// Load one source-committed handoff and its one-time consumption state.
    fn load_handoff(&self, handoff_id: Hash32) -> std::result::Result<Option<StoredHandoff>, StoreError> {
        let _ = handoff_id;
        Ok(None)
    }
    /// An explicit independently reverified external source. Local-only stores
    /// have no imports; remote-aware stores must not fake local `load_block`.
    fn load_handoff_import(&self, handoff_id: Hash32) -> std::result::Result<Option<VerifiedHandoffImport>, StoreError> {
        let _ = handoff_id;
        Ok(None)
    }
    /// Local composition configuration, never proof-supplied authority keys.
    fn handoff_import_registry(&self) -> Option<Arc<HandoffImportRegistry>> {
        None
    }
}

/// Complete chain/finality audit material; application execution is verified separately. Fields are private so callers cannot
/// bypass chain/finality audit before constructing a writable session.
pub struct AuditedRecovery {
    config: SessionConfig,
    head: SessionHead,
    checkpoint: StateCheckpoint,
    store: Arc<dyn SessionStore>,
    usage: RecoveryUsage,
}

impl AuditedRecovery {
    /// Total work across every resumed step, including repeated dependency reads.
    pub fn usage(&self) -> RecoveryUsage {
        self.usage
    }
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
        if config.predecessor.is_some() {
            return Err(SessionError::Continuity(
                "a successor genesis can only be created atomically by its predecessor's final block".to_owned(),
            ));
        }
        ensure_consensus_config(finality_verifier.as_ref(), &config)?;
        if executor.state_root() != config.initial_state_root {
            return Err(SessionError::StateRootMismatch { expected: config.initial_state_root, actual: executor.state_root() });
        }
        let state_snapshot = executor.snapshot().map_err(SessionError::Execution)?;
        config.commit_limits.snapshot(&state_snapshot)?;
        let head = SessionHead {
            session_id: config.session_id,
            finalised_height: None,
            block_hash: [0; 32],
            state_root: config.initial_state_root,
            timestamp_ms: config.initial_timestamp_ms,
            input_position: config.initial_input_position,
            logical_time: config.initial_logical_time,
            sealed_by_successor: None,
        };
        store.create_session(&SessionGenesis { config: config.clone(), state_snapshot }, &head)?;
        Ok(Self { config, head, finality_verifier, executor, store })
    }

    /// Recover a chain after the caller reconstructs an executor from the most
    /// recent snapshot returned by [`Self::recovery_snapshot`]. Every block and
    /// finality proof is streamed and revalidated before the service becomes
    /// writable.
    pub fn recover(
        session_id: Hash32,
        expected_application_profile: &ApplicationProfile,
        finality_verifier: Arc<dyn FinalityVerifier>,
        executor: E,
        store: Arc<S>,
    ) -> Result<Self> {
        let audited = Self::audit_recovery(session_id, expected_application_profile, finality_verifier.clone(), store.clone())?;
        Self::recover_audited(audited, expected_application_profile, finality_verifier, executor, store)
    }

    /// Audit history and indexes under the default finite recovery budget.
    /// `SessionError::RecoveryPaused` owns the cursor when more work is needed;
    /// use [`RecoveryAudit`] directly to choose budgets or cancellation.
    pub fn audit_recovery(
        session_id: Hash32,
        expected_application_profile: &ApplicationProfile,
        finality_verifier: Arc<dyn FinalityVerifier>,
        store: Arc<S>,
    ) -> Result<AuditedRecovery> {
        match RecoveryAudit::new(session_id, expected_application_profile.clone(), finality_verifier, store)
            .advance(RecoveryBudget::default(), &RecoveryCancellation::default())?
        {
            RecoveryOutcome::Complete(audited) => Ok(*audited),
            RecoveryOutcome::Paused(paused) => Err(SessionError::RecoveryPaused(paused)),
        }
    }

    /// Attach an executor restored from [`AuditedRecovery::state_snapshot`]
    /// without auditing the chain a second time. The store identity and current
    /// head must still match the completed audit.
    pub fn recover_audited(
        audited: AuditedRecovery,
        expected_application_profile: &ApplicationProfile,
        finality_verifier: Arc<dyn FinalityVerifier>,
        executor: E,
        store: Arc<S>,
    ) -> Result<Self> {
        expected_application_profile.validate()?;
        let selected: Arc<dyn SessionStore> = store.clone();
        if !Arc::ptr_eq(&selected, &audited.store) || store.load_head(audited.config.session_id)? != audited.head {
            return Err(SessionError::Recovery("audited recovery belongs to another store or a changed head".into()));
        }
        if &audited.config.application_profile != expected_application_profile {
            return Err(SessionError::Recovery("local application profile does not match the audited genesis profile".to_owned()));
        }
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

    /// Inspect the current head snapshot without entering the write, producer,
    /// handoff, or outbox paths.
    pub fn inspect_head(&self, port: &dyn InspectPort, query: &[u8]) -> Result<InspectionReceipt> {
        let snapshot = self.executor.snapshot().map_err(SessionError::Inspection)?;
        self.inspect_snapshot(self.head.finalised_height, self.head.state_root, &snapshot, port, query)
    }

    /// Inspect genesis or a retained archival checkpoint. Historical blocks
    /// whose snapshot was pruned fail closed instead of silently inspecting a
    /// different state.
    pub fn inspect_at(&self, finalised_height: Option<u64>, port: &dyn InspectPort, query: &[u8]) -> Result<InspectionReceipt> {
        if finalised_height == self.head.finalised_height {
            return self.inspect_head(port, query);
        }
        let (state_root, snapshot) = match finalised_height {
            None => {
                let genesis = self.store.load_genesis(self.config.session_id)?;
                (genesis.config.initial_state_root, genesis.state_snapshot)
            }
            Some(height) => {
                let record = self.store.load_block(self.config.session_id, height)?;
                if record.state_snapshot.is_empty() {
                    return Err(SessionError::Inspection(format!("no archival snapshot is retained at height {height}")));
                }
                (record.block.state_root_after, record.state_snapshot)
            }
        };
        self.inspect_snapshot(finalised_height, state_root, &snapshot, port, query)
    }

    /// Execute from genesis under the default finite recovery budget. Warm-up
    /// blocks before `start_height` are verified but excluded from receipt totals.
    /// Budget exhaustion preserves the executor in `SessionError::ReplayPaused`.
    pub fn replay_range<R, F>(&self, start_height: u64, end_height: u64, restore: F) -> Result<RangeReplayReceipt>
    where
        R: TransitionExecutor,
        F: FnOnce(&[u8]) -> Result<R> + Send + Sync + 'static,
    {
        match self
            .start_replay_range(start_height, end_height, None, restore)
            .advance(RecoveryBudget::default(), &RecoveryCancellation::default())?
        {
            ReplayOutcome::Complete { receipt, .. } => Ok(*receipt),
            ReplayOutcome::Paused(cursor) => Err(SessionError::ReplayPaused(cursor)),
        }
    }

    /// Construct a resumable replay without I/O. None starts at genesis; an
    /// explicit independent anchor permits trusted-checkpoint suffix execution.
    pub fn start_replay_range<R, F>(&self, start_height: u64, end_height: u64, anchor: Option<ReplayAnchor>, restore: F) -> RangeReplay
    where
        R: TransitionExecutor,
        F: FnOnce(&[u8]) -> Result<R> + Send + Sync + 'static,
    {
        RangeReplay::new(
            self.store.clone(),
            self.finality_verifier.clone(),
            self.config.session_id,
            self.config.application_profile.clone(),
            start_height..=end_height,
            anchor,
            restore,
        )
    }

    fn inspect_snapshot(
        &self,
        finalised_height: Option<u64>,
        state_root: Hash32,
        snapshot: &[u8],
        port: &dyn InspectPort,
        query: &[u8],
    ) -> Result<InspectionReceipt> {
        if query.len() as u64 > self.config.application_profile.resources.max_inspect_query_bytes {
            return Err(SessionError::Inspection(format!(
                "inspection query bytes {} exceed {}",
                query.len(),
                self.config.application_profile.resources.max_inspect_query_bytes
            )));
        }
        let context = InspectContext {
            session_id: self.config.session_id,
            finalised_height,
            state_root,
            application_profile: &self.config.application_profile,
        };
        let result = port.inspect(&context, snapshot, query).map_err(SessionError::Inspection)?;
        if result.len() as u64 > self.config.application_profile.resources.max_inspect_result_bytes {
            return Err(SessionError::Inspection(format!(
                "inspection result bytes {} exceed {}",
                result.len(),
                self.config.application_profile.resources.max_inspect_result_bytes
            )));
        }
        let profile = self.config.application_profile.commitment();
        let query_hash = application::inspect_query_hash(query);
        let result_hash =
            application::inspect_result_hash(self.config.session_id, finalised_height, state_root, profile, query_hash, &result);
        Ok(InspectionReceipt {
            session_id: self.config.session_id,
            finalised_height,
            state_root,
            application_profile_commitment: profile,
            query_hash,
            result_hash,
            result,
        })
    }

    /// Execute a request on an isolated clone and build the exact block that
    /// validators must sign. This function performs no durable mutation.
    /// Prepare only against the exact snapshot used by a transaction builder.
    /// A stale request must be rebuilt; it is never silently rebased.
    pub fn prepare_block_at(&self, base_head: &SessionHead, request: BlockRequest) -> Result<PreparedBlock<E>> {
        if base_head != &self.head {
            return Err(SessionError::StalePreparedBlock);
        }
        self.prepare_block(request)
    }

    pub fn prepare_block(&self, request: BlockRequest) -> Result<PreparedBlock<E>> {
        if self.head.sealed_by_successor.is_some() {
            return Err(SessionError::Continuity("session is sealed by its declared successor".to_owned()));
        }
        let limits = &self.config.commit_limits;
        let mut handoff_usage = limits.request(&request)?;
        if let Some(target) = &request.successor {
            target.validate()?;
        }
        request.frame_input.validate(&self.config.application_profile)?;
        for command in &request.frame_input.commands {
            if !matches!(self.command_status(command)?, CommandStatus::Missing) {
                return Err(SessionError::InvalidFrame("command identity already committed; reconcile before retrying".to_owned()));
            }
        }
        if request.frame_input.start_position != self.head.input_position {
            return Err(SessionError::InvalidFrame(format!(
                "frame input starts at {}, expected {}",
                request.frame_input.start_position, self.head.input_position
            )));
        }
        if request.frame_input.logical_time_start != self.head.logical_time {
            return Err(SessionError::InvalidFrame(format!(
                "frame logical time starts at {}, expected {}",
                request.frame_input.logical_time_start, self.head.logical_time
            )));
        }
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
        let mut consumed_handoffs = Vec::with_capacity(request.consume_handoffs.len());
        let mut consumed_ids = HashSet::with_capacity(request.consume_handoffs.len());
        for handoff_id in &request.consume_handoffs {
            if !consumed_ids.insert(*handoff_id) {
                return Err(SessionError::Continuity("duplicate handoff consumption id".to_owned()));
            }
            let stored = self
                .store
                .load_handoff(*handoff_id)?
                .ok_or_else(|| SessionError::Continuity("requested handoff does not exist".to_owned()))?;
            handoff_usage.add(limits, stored.handoff.payload.len(), stored.handoff.authorization.len())?;
            if stored.handoff.id() != *handoff_id || !stored.is_available() {
                return Err(SessionError::Continuity("handoff identity mismatch or already consumed".to_owned()));
            }
            if !stored.handoff.accepts_target(&self.config) || request.timestamp_ms > stored.handoff.expires_at_ms {
                return Err(SessionError::Continuity("handoff target policy mismatch or handoff expired".to_owned()));
            }
            audit_handoff_lifecycle(self.store.as_ref(), &stored)?;
            let source = resolve_handoff_source(self.store.as_ref(), &stored.handoff)?;
            handoff_usage.add_remote_proof(limits, source.proof_bytes())?;
            source.validate_target(&self.config)?;
            validate_handoff_evidence(self.store.as_ref(), &stored.handoff, &source)?;
            consumed_handoffs.push(stored.handoff);
        }
        let mut handoff_actions = Vec::with_capacity(request.handoff_actions.len());
        for action in &request.handoff_actions {
            if !consumed_ids.insert(action.handoff_id()) {
                return Err(SessionError::Continuity("duplicate lifecycle or consumption action".into()));
            }
            let stored = self
                .store
                .load_handoff(action.handoff_id())?
                .ok_or_else(|| SessionError::Continuity("action handoff absent; reconcile".into()))?;
            handoff_usage.add(limits, stored.handoff.payload.len(), stored.handoff.authorization.len())?;
            audit_handoff_lifecycle(self.store.as_ref(), &stored)?;
            let cancelled_at = match (&action, &stored.lifecycle.terminal) {
                (HandoffAction::Cancel { .. }, HandoffTerminal::Issued) => None,
                (HandoffAction::RemoteRetire { .. }, HandoffTerminal::Issued) => None,
                (HandoffAction::Reclaim { .. }, HandoffTerminal::Cancelled { at }) => Some(at.clone()),
                (HandoffAction::Retire { consumed_at, .. }, HandoffTerminal::Consumed { at }) if consumed_at == at => {
                    let target = self.store.load_block(at.session_id, at.height)?;
                    validate_consumption_record(at, action.handoff_id(), &target)?;
                    None
                }
                _ => return Err(SessionError::Continuity("handoff lifecycle conflict; reconcile".into())),
            };
            let resolved = ResolvedHandoffAction { action: action.clone(), handoff: stored.handoff, cancelled_at };
            validate_handoff_action(self.store.as_ref(), &self.config, height, &resolved)?;
            handoff_actions.push(resolved);
        }
        if !handoff_actions.is_empty() && self.config.protocol_effects.application_policy_hash().is_none() {
            return Err(SessionError::Continuity("lifecycle actions require an application policy".into()));
        }
        let mut candidate = self.executor.clone();
        if candidate.state_root() != self.head.state_root {
            return Err(SessionError::StateRootMismatch { expected: self.head.state_root, actual: candidate.state_root() });
        }
        let canonical_txids = request
            .transactions
            .iter()
            .map(|bytes| {
                let tx = deserialize_transaction_molecule(bytes)
                    .map_err(|error| SessionError::Execution(format!("invalid canonical CellTx: {error}")))?;
                if tx.version() != 0 {
                    return Err(SessionError::Execution(format!(
                        "CellTx version {} is not supported; new transactions must use version 0",
                        tx.version()
                    )));
                }
                Ok(tx.id())
            })
            .collect::<Result<Vec<_>>>()?;
        let effects_context = ProtocolEffectsContext::new(
            &self.config,
            height,
            request.timestamp_ms,
            self.head.state_root,
            &request.frame_input,
            request.handoffs.clone(),
            request.successor.as_ref(),
        )?;
        let outcome = candidate
            .execute_block_with_lifecycle(
                &effects_context,
                &handoff_actions,
                &request.frame_input,
                &consumed_handoffs,
                &request.transactions,
            )
            .map_err(SessionError::Execution)?;
        limits.outcome(&request, &outcome)?;
        if outcome.ordered_cell_tx_commitments.len() != request.transactions.len() {
            return Err(SessionError::Execution("executor returned a different transaction count".to_owned()));
        }
        if outcome.ordered_cell_tx_commitments != canonical_txids {
            return Err(SessionError::Execution("executor raw txids do not match the canonical transaction bytes".to_owned()));
        }
        let mut seen = HashSet::with_capacity(outcome.ordered_cell_tx_commitments.len());
        if outcome.ordered_cell_tx_commitments.iter().any(|txid| !seen.insert(*txid)) {
            return Err(SessionError::Execution("executor returned duplicate raw txids".to_owned()));
        }
        if outcome.cycles > self.config.application_profile.resources.max_frame_cycles {
            return Err(SessionError::InvalidFrame(format!(
                "execution cycles {} exceed {}",
                outcome.cycles, self.config.application_profile.resources.max_frame_cycles
            )));
        }

        let outbox = outcome
            .outbox
            .into_iter()
            .enumerate()
            .map(|(index, message)| {
                make_outbox_message(self.config.session_id, self.config.consensus_module_commitment, height, index, message)
            })
            .collect::<Result<Vec<_>>>()?;
        let request_data_commitments = request.data_commitments;
        let mut data_commitments = request_data_commitments.clone();
        data_commitments.extend(outcome.data_commitments);
        data_commitments.extend(outbox.iter().map(OutboxMessage::commitment));
        let mut issued_handoffs = Vec::with_capacity(request.handoffs.len());
        let mut issued_ids = HashSet::with_capacity(request.handoffs.len());
        for intent in request.handoffs {
            let handoff = SessionHandoff::from_intent(
                self.config.session_id,
                height,
                candidate.state_root(),
                self.config.application_profile.commitment(),
                self.config.consensus_module_commitment,
                request.timestamp_ms,
                intent,
            )?;
            if !issued_ids.insert(handoff.id()) {
                return Err(SessionError::Continuity("duplicate issued handoff id".to_owned()));
            }
            data_commitments.push(handoff.id());
            issued_handoffs.push(handoff);
        }
        for handoff_id in &request.consume_handoffs {
            data_commitments.push(continuity::handoff_consumption_commitment(self.config.session_id, height, *handoff_id));
        }

        let frame = ExecutionFrame {
            session_id: self.config.session_id,
            height,
            application_profile_commitment: self.config.application_profile.commitment(),
            input_start: request.frame_input.start_position,
            input_end: request.frame_input.end_position,
            input_root: request.frame_input.root(),
            logical_time_start: request.frame_input.logical_time_start,
            logical_time_end: request.frame_input.logical_time_end,
            state_root_before: self.head.state_root,
            state_root_after: candidate.state_root(),
            ordered_cell_tx_commitments: outcome.ordered_cell_tx_commitments.clone(),
            resources: ExecutionResources {
                cycles: outcome.cycles,
                transaction_bytes: total_bytes,
                input_bytes: request.frame_input.payload.len() as u64,
            },
        };
        frame.validate(&self.config.application_profile, &request.frame_input)?;
        let snapshot = candidate.snapshot().map_err(SessionError::Execution)?;
        limits.snapshot(&snapshot)?;
        let successor = request
            .successor
            .map(|target_config| {
                if target_config.initial_input_position != request.frame_input.end_position
                    || target_config.initial_logical_time != request.frame_input.logical_time_end
                    || target_config.initial_timestamp_ms != request.timestamp_ms
                {
                    return Err(SessionError::Continuity(
                        "successor genesis must preserve the predecessor frame cursor, logical time and timestamp".to_owned(),
                    ));
                }
                let transition =
                    SuccessorTransition::execute(&candidate, &self.config, &target_config, &request.frame_input, &snapshot)?;
                let declaration = SuccessorDeclaration {
                    source_session_id: self.config.session_id,
                    source_height: height,
                    source_state_root: candidate.state_root(),
                    source_application_profile_commitment: self.config.application_profile.commitment(),
                    target_config,
                    transition,
                };
                declaration.validate()?;
                Ok(declaration)
            })
            .transpose()?;
        if let Some(declaration) = &successor {
            data_commitments.push(declaration.commitment());
        }
        if !handoff_actions.is_empty() {
            data_commitments.push(lifecycle::actions_commitment(self.config.session_id, height, &handoff_actions)?);
        }
        data_commitments.push(effects_context.commitment()?);
        data_commitments.push(resources::snapshot_commitment(limits, snapshot.len() as u64));
        let block = MyelinBlock {
            version: SESSION_BLOCK_VERSION,
            session_id: self.config.session_id,
            parent_hash: self.head.block_hash,
            number: height,
            timestamp_ms: request.timestamp_ms,
            consensus_kind: self.config.consensus_kind,
            application_profile_commitment: self.config.application_profile.commitment(),
            execution_frame_commitment: frame.commitment(),
            state_root_before: self.head.state_root,
            state_root_after: candidate.state_root(),
            ordered_cell_tx_commitments: outcome.ordered_cell_tx_commitments,
            data_commitments,
            scheduler_commitment: request.scheduler_commitment,
        };
        let prepared = PreparedBlock {
            block,
            base_head: self.head.clone(),
            executor: candidate,
            transactions: request.transactions,
            snapshot,
            outbox,
            frame,
            frame_input: request.frame_input,
            successor,
            issued_handoffs,
            consumed_handoffs,
            handoff_actions,
            request_data_commitments,
            remote_terminals: vec![],
        };
        // An empty proof gives a strict lower bound before consensus work.
        prepared.check_record_size(&self.config.consensus_module_commitment, &[], limits.max_record_bytes)?;
        Ok(prepared)
    }

    /// Verify finality and atomically advance the chain. A stale candidate is
    /// rejected both in memory and by the store's durable CAS.
    pub fn commit_finalised(&mut self, prepared: PreparedBlock<E>, proof: FinalityProof) -> Result<SessionHead> {
        if prepared.base_head != self.head {
            return Err(SessionError::StalePreparedBlock);
        }
        let proof_bytes = proof.encode()?;
        prepared.check_record_size(
            &self.config.consensus_module_commitment,
            &proof_bytes,
            self.config.commit_limits.max_record_bytes,
        )?;
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
            input_position: prepared.frame.input_end,
            logical_time: prepared.frame.logical_time_end,
            sealed_by_successor: prepared.successor.as_ref().map(|declaration| declaration.target_config.session_id),
        };
        let record = FinalisedBlockRecord {
            block: prepared.block,
            consensus_module_commitment: finalised.consensus_module_commitment,
            proof,
            transactions: prepared.transactions,
            resource_snapshot_bytes: prepared.snapshot.len() as u64,
            state_snapshot: prepared.snapshot,
            outbox: prepared.outbox,
            frame: prepared.frame,
            frame_input: prepared.frame_input,
            successor: prepared.successor,
            issued_handoffs: prepared.issued_handoffs,
            consumed_handoff_ids: prepared.consumed_handoffs.iter().map(SessionHandoff::id).collect(),
            request_data_commitments: prepared.request_data_commitments,
            handoff_actions: prepared.handoff_actions,
            remote_terminals: prepared.remote_terminals,
        };
        record.validate_resources(&self.config)?;
        for handoff in &prepared.consumed_handoffs {
            let source = resolve_handoff_source(self.store.as_ref(), handoff)?;
            validate_remote_consumption(&self.config, &source.config, &record, handoff)?;
        }
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
        timestamp_ms: genesis.config.initial_timestamp_ms,
        input_position: genesis.config.initial_input_position,
        logical_time: genesis.config.initial_logical_time,
        sealed_by_successor: None,
    }
}

fn audit_record(
    config: &SessionConfig,
    verifier: &dyn FinalityVerifier,
    head: &mut SessionHead,
    record: &FinalisedBlockRecord,
) -> Result<()> {
    record.validate_resources(config)?;
    if head.sealed_by_successor.is_some() {
        return Err(SessionError::Recovery("a block appears after the session was sealed".to_owned()));
    }
    if record.block.parent_hash != head.block_hash
        || record.block.number != head.next_height()?
        || record.block.session_id != config.session_id
        || record.block.state_root_before != head.state_root
        || record.block.timestamp_ms < head.timestamp_ms
        || record.frame.input_start != head.input_position
        || record.frame.logical_time_start != head.logical_time
        || record.block.consensus_kind != config.consensus_kind
        || record.block.application_profile_commitment != config.application_profile.commitment()
        || record.block.execution_frame_commitment != record.frame.commitment()
        || record.consensus_module_commitment != config.consensus_module_commitment
    {
        return Err(SessionError::Recovery(format!("invalid chain linkage at height {}", record.block.number)));
    }
    if record.transactions.len() != record.block.ordered_cell_tx_commitments.len() {
        return Err(SessionError::Recovery(format!("payload count mismatch at height {}", record.block.number)));
    }
    if !record.block.data_commitments.starts_with(&record.request_data_commitments) {
        return Err(SessionError::Recovery(format!(
            "request data commitments are not the canonical prefix at height {}",
            record.block.number
        )));
    }
    record.frame.validate(&config.application_profile, &record.frame_input)?;
    if record.frame.session_id != config.session_id
        || record.frame.height != record.block.number
        || record.frame.state_root_before != record.block.state_root_before
        || record.frame.state_root_after != record.block.state_root_after
        || record.frame.ordered_cell_tx_commitments != record.block.ordered_cell_tx_commitments
    {
        return Err(SessionError::Recovery(format!("execution frame mismatch at height {}", record.block.number)));
    }
    if let Some(successor) = &record.successor {
        successor.validate_source(config, record.frame_input.root())?;
        if successor.source_session_id != config.session_id
            || successor.source_height != record.block.number
            || successor.source_state_root != record.block.state_root_after
            || successor.source_application_profile_commitment != config.application_profile.commitment()
            || successor.target_config.initial_input_position != record.frame.input_end
            || successor.target_config.initial_logical_time != record.frame.logical_time_end
            || successor.target_config.initial_timestamp_ms != record.block.timestamp_ms
            || !record.block.data_commitments.contains(&successor.commitment())
        {
            return Err(SessionError::Recovery(format!("successor declaration mismatch at height {}", record.block.number)));
        }
    }
    let mut handoff_ids = HashSet::with_capacity(record.issued_handoffs.len());
    for handoff in &record.issued_handoffs {
        handoff.validate(record.block.timestamp_ms)?;
        let handoff_id = handoff.id();
        if handoff.source_session_id != config.session_id
            || handoff.source_height != record.block.number
            || handoff.source_state_root != record.block.state_root_after
            || handoff.source_application_profile_commitment != config.application_profile.commitment()
            || handoff.source_consensus_module_commitment != config.consensus_module_commitment
            || !handoff_ids.insert(handoff_id)
            || !record.block.data_commitments.contains(&handoff_id)
        {
            return Err(SessionError::Recovery(format!("issued handoff mismatch at height {}", record.block.number)));
        }
    }
    let mut consumed_ids = HashSet::with_capacity(record.consumed_handoff_ids.len());
    if record.consumed_handoff_ids.iter().any(|handoff_id| {
        !consumed_ids.insert(*handoff_id)
            || !record.block.data_commitments.contains(&continuity::handoff_consumption_commitment(
                config.session_id,
                record.block.number,
                *handoff_id,
            ))
    }) {
        return Err(SessionError::Recovery(format!("handoff consumption mismatch at height {}", record.block.number)));
    }
    let mut transaction_bytes = 0u64;
    for (bytes, expected_txid) in record.transactions.iter().zip(&record.block.ordered_cell_tx_commitments) {
        transaction_bytes = transaction_bytes
            .checked_add(bytes.len() as u64)
            .ok_or_else(|| SessionError::Recovery("transaction byte total overflow".to_owned()))?;
        let transaction = deserialize_transaction_molecule(bytes)
            .map_err(|error| SessionError::Recovery(format!("invalid transaction at height {}: {error}", record.block.number)))?;
        if transaction.id() != *expected_txid {
            return Err(SessionError::Recovery(format!("raw txid mismatch at height {}", record.block.number)));
        }
    }
    if record.frame.resources.transaction_bytes != transaction_bytes {
        return Err(SessionError::Recovery(format!("execution frame transaction byte mismatch at height {}", record.block.number)));
    }
    let verified = verifier.verify(&record.block, &record.proof)?;
    if verified.block_hash != record.block.hash() || verified.consensus_module_commitment != record.consensus_module_commitment {
        return Err(SessionError::Recovery(format!("finality verifier result mismatch at height {}", record.block.number)));
    }
    *head = SessionHead {
        session_id: config.session_id,
        finalised_height: Some(record.block.number),
        block_hash: record.block.hash(),
        state_root: record.block.state_root_after,
        timestamp_ms: record.block.timestamp_ms,
        input_position: record.frame.input_end,
        logical_time: record.frame.logical_time_end,
        sealed_by_successor: record.successor.as_ref().map(|declaration| declaration.target_config.session_id),
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

    fn execute_block(
        &mut self,
        height: u64,
        _frame_input: &FrameInput,
        consumed_handoffs: &[SessionHandoff],
        transactions: &[Vec<u8>],
    ) -> std::result::Result<ExecutionOutcome, String> {
        if !consumed_handoffs.is_empty() {
            return Err("the built-in Cell transition executor has no application handoff intake policy".to_owned());
        }
        let mut txids = Vec::with_capacity(transactions.len());
        let mut cycles = 0u64;
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
            cycles = cycles.checked_add(receipt.cycles).ok_or_else(|| "execution cycle total overflow".to_owned())?;
            txids.push(receipt.txid);
            self.state_root = receipt.state_root_after.as_bytes();
        }
        Ok(ExecutionOutcome { ordered_cell_tx_commitments: txids, data_commitments: Vec::new(), outbox: Vec::new(), cycles })
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
    /// Application execution frame violated its genesis-bound contract.
    #[error("invalid execution frame: {0}")]
    InvalidFrame(String),
    /// Read-only inspection failed or exceeded its committed bounds.
    #[error("inspection failed: {0}")]
    Inspection(String),
    /// Evidence receipt or pipeline linkage failed validation.
    #[error("evidence verification failed: {0}")]
    Evidence(String),
    /// Deterministic range replay did not reproduce committed history.
    #[error("range replay failed: {0}")]
    Replay(String),
    /// Session succession or cross-session handoff violated its commitment.
    #[error("session continuity failed: {0}")]
    Continuity(String),
    /// Genesis-bound effect policy or application authorization failed.
    #[error("protocol effects failed: {0}")]
    ProtocolEffects(String),
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
    /// Genesis-bound aggregate budget exceeded. `used` may be the first known
    /// lower bound above the cap when serialization stops early.
    #[error("{resource} limit exceeded: {used} > {limit}")]
    ResourceLimit { resource: &'static str, used: u64, limit: u64 },
    /// Proposed timestamp violated monotonic/drift rules.
    #[error("invalid block timestamp: {0}")]
    InvalidTimestamp(String),
    /// A finite audit budget ended before completion; the cursor can be resumed.
    #[error("recovery paused: {0}")]
    RecoveryPaused(Box<PausedRecovery>),
    #[error("application replay paused: {0}")]
    ReplayPaused(Box<PausedReplay>),
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

fn encode_record<T: Serialize>(body: &T) -> Result<Vec<u8>> {
    codec::size_with_limit(body, "encoded_record_bytes", (codec::MAX_RECORD_BYTES - RECORD_HEADER_BYTES) as u64)?;
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
    record_version(bytes)?;
    let body_bytes = &bytes[RECORD_HEADER_BYTES..];
    let (value, rest) = postcard::take_from_bytes(body_bytes).map_err(|error| SessionError::Codec(error.to_string()))?;
    if !rest.is_empty() {
        return Err(SessionError::Codec("trailing record bytes".into()));
    }
    Ok(value)
}

fn record_version(bytes: &[u8]) -> Result<u16> {
    resources::check("encoded_record_bytes", bytes.len() as u64, codec::MAX_RECORD_BYTES as u64)?;
    if !bytes.starts_with(RECORD_MAGIC) {
        return Err(SessionError::Codec("record does not use the canonical binary envelope".to_owned()));
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
    Ok(version)
}

#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct SessionConfigWire {
    session_id: Hash32,
    #[serde(deserialize_with = "crate::codec::bounded_string")]
    consensus_kind: String,
    consensus_config_commitment: Hash32,
    consensus_module_commitment: Hash32,
    consensus_wal_schema_hash: Hash32,
    application_profile: ApplicationProfile,
    initial_state_root: Hash32,
    initial_input_position: u64,
    initial_logical_time: u64,
    initial_timestamp_ms: u64,
    predecessor: Option<PredecessorReference>,
    max_block_transactions: u32,
    max_block_bytes: u64,
    max_future_drift_ms: u64,
    commit_limits: CommitResourceLimits,
    protocol_effects: ProtocolEffectsPolicy,
}

#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct SessionGenesisWire {
    config: SessionConfigWire,
    #[serde(deserialize_with = "crate::codec::bounded_bytes")]
    state_snapshot: Vec<u8>,
}

#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct StateCheckpointWire {
    session_id: Hash32,
    finalised_height: Option<u64>,
    state_root: Hash32,
    #[serde(deserialize_with = "crate::codec::bounded_bytes")]
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
            application_profile: value.application_profile.clone(),
            initial_state_root: value.initial_state_root,
            initial_input_position: value.initial_input_position,
            initial_logical_time: value.initial_logical_time,
            initial_timestamp_ms: value.initial_timestamp_ms,
            predecessor: value.predecessor.clone(),
            max_block_transactions: value.max_block_transactions,
            max_block_bytes: value.max_block_bytes,
            max_future_drift_ms: value.max_future_drift_ms,
            commit_limits: value.commit_limits.clone(),
            protocol_effects: value.protocol_effects.clone(),
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
            application_profile: value.application_profile,
            initial_state_root: value.initial_state_root,
            initial_input_position: value.initial_input_position,
            initial_logical_time: value.initial_logical_time,
            initial_timestamp_ms: value.initial_timestamp_ms,
            predecessor: value.predecessor,
            max_block_transactions: value.max_block_transactions,
            max_block_bytes: value.max_block_bytes,
            max_future_drift_ms: value.max_future_drift_ms,
            commit_limits: value.commit_limits.clone(),
            protocol_effects: value.protocol_effects.clone(),
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
    input_position: u64,
    logical_time: u64,
    sealed_by_successor: Option<Hash32>,
}

impl From<&SessionHead> for SessionHeadWire {
    fn from(value: &SessionHead) -> Self {
        Self {
            session_id: value.session_id,
            finalised_height: value.finalised_height,
            block_hash: value.block_hash,
            state_root: value.state_root,
            timestamp_ms: value.timestamp_ms,
            input_position: value.input_position,
            logical_time: value.logical_time,
            sealed_by_successor: value.sealed_by_successor,
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
            input_position: value.input_position,
            logical_time: value.logical_time,
            sealed_by_successor: value.sealed_by_successor,
        }
    }
}

#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct BlockWire {
    version: u32,
    session_id: Hash32,
    parent_hash: Hash32,
    number: u64,
    timestamp_ms: u64,
    #[serde(deserialize_with = "crate::codec::bounded_string")]
    consensus_kind: String,
    application_profile_commitment: Hash32,
    execution_frame_commitment: Hash32,
    state_root_before: Hash32,
    state_root_after: Hash32,
    #[serde(deserialize_with = "crate::codec::bounded_vec")]
    ordered_cell_tx_commitments: Vec<Hash32>,
    #[serde(deserialize_with = "crate::codec::bounded_vec")]
    data_commitments: Vec<Hash32>,
    scheduler_commitment: Hash32,
}

impl From<&MyelinBlock> for BlockWire {
    fn from(value: &MyelinBlock) -> Self {
        Self {
            version: value.version,
            session_id: value.session_id,
            parent_hash: value.parent_hash,
            number: value.number,
            timestamp_ms: value.timestamp_ms,
            consensus_kind: value.consensus_kind.as_str().to_owned(),
            application_profile_commitment: value.application_profile_commitment,
            execution_frame_commitment: value.execution_frame_commitment,
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
            session_id: value.session_id,
            parent_hash: value.parent_hash,
            number: value.number,
            timestamp_ms: value.timestamp_ms,
            consensus_kind: parse_kind(&value.consensus_kind)?,
            application_profile_commitment: value.application_profile_commitment,
            execution_frame_commitment: value.execution_frame_commitment,
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
    #[serde(deserialize_with = "crate::codec::bounded_bytes")]
    proof: Vec<u8>,
    #[serde(deserialize_with = "crate::codec::transactions")]
    transactions: Vec<Vec<u8>>,
    #[serde(deserialize_with = "crate::codec::bounded_bytes")]
    state_snapshot: Vec<u8>,
    #[serde(deserialize_with = "crate::codec::bounded_vec")]
    outbox: Vec<OutboxMessage>,
    frame: ExecutionFrame,
    frame_input: FrameInput,
    successor: Option<SuccessorDeclarationWire>,
    #[serde(deserialize_with = "crate::codec::bounded_vec")]
    issued_handoffs: Vec<SessionHandoff>,
    #[serde(deserialize_with = "crate::codec::bounded_vec")]
    consumed_handoff_ids: Vec<Hash32>,
    #[serde(deserialize_with = "crate::codec::bounded_vec")]
    request_data_commitments: Vec<Hash32>,
    snapshot_bytes: u64,
    #[serde(deserialize_with = "crate::codec::bounded_vec")]
    handoff_actions: Vec<ResolvedHandoffAction>,
    #[serde(deserialize_with = "crate::codec::bounded_vec")]
    remote_terminals: Vec<RemoteTerminalCertificate>,
}

#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct SuccessorDeclarationWire {
    source_session_id: Hash32,
    source_height: u64,
    source_state_root: Hash32,
    source_application_profile_commitment: Hash32,
    target_config: SessionConfigWire,
    transition: SuccessorTransition,
}

impl From<&SuccessorDeclaration> for SuccessorDeclarationWire {
    fn from(value: &SuccessorDeclaration) -> Self {
        Self {
            source_session_id: value.source_session_id,
            source_height: value.source_height,
            source_state_root: value.source_state_root,
            source_application_profile_commitment: value.source_application_profile_commitment,
            target_config: SessionConfigWire::from(&value.target_config),
            transition: value.transition.clone(),
        }
    }
}

impl TryFrom<SuccessorDeclarationWire> for SuccessorDeclaration {
    type Error = SessionError;

    fn try_from(value: SuccessorDeclarationWire) -> Result<Self> {
        let declaration = Self {
            source_session_id: value.source_session_id,
            source_height: value.source_height,
            source_state_root: value.source_state_root,
            source_application_profile_commitment: value.source_application_profile_commitment,
            target_config: SessionConfig::try_from(value.target_config)?,
            transition: value.transition,
        };
        Ok(declaration)
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
            frame: value.frame,
            frame_input: value.frame_input,
            successor: value.successor.map(SuccessorDeclaration::try_from).transpose()?,
            issued_handoffs: value.issued_handoffs,
            consumed_handoff_ids: value.consumed_handoff_ids,
            request_data_commitments: value.request_data_commitments,
            resource_snapshot_bytes: value.snapshot_bytes,
            handoff_actions: value.handoff_actions,
            remote_terminals: value.remote_terminals,
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

        fn load_chain_page(
            &self,
            _session_id: Hash32,
            start_height: u64,
            limit: usize,
        ) -> std::result::Result<Vec<FinalisedBlockRecord>, StoreError> {
            if limit == 0 {
                return Ok(Vec::new());
            }
            Ok(self
                .inner
                .lock()
                .unwrap()
                .chain
                .iter()
                .filter(|record| record.block.number >= start_height)
                .take(limit)
                .cloned()
                .collect())
        }

        fn load_command_receipt(
            &self,
            _session_id: Hash32,
            command_id: Hash32,
        ) -> std::result::Result<Option<CommandReceipt>, StoreError> {
            for record in &self.inner.lock().unwrap().chain {
                if let Some(index) = record.frame_input.commands.iter().position(|command| command.id == command_id) {
                    return CommandReceipt::for_record(record, index)
                        .map(Some)
                        .map_err(|error| StoreError::Corrupt(error.to_string()));
                }
            }
            Ok(None)
        }

        fn load_checkpoint(&self, session_id: Hash32) -> std::result::Result<StateCheckpoint, StoreError> {
            let genesis = self.load_genesis(session_id)?;
            if let Some(record) = self.inner.lock().unwrap().chain.iter().rev().find(|record| !record.state_snapshot.is_empty()) {
                return Ok(StateCheckpoint {
                    session_id,
                    finalised_height: Some(record.block.number),
                    state_root: record.block.state_root_after,
                    state_snapshot: record.state_snapshot.clone(),
                });
            }
            Ok(StateCheckpoint {
                session_id,
                finalised_height: None,
                state_root: genesis.config.initial_state_root,
                state_snapshot: genesis.state_snapshot,
            })
        }

        fn load_checkpoint_before(&self, session_id: Hash32, height: u64) -> std::result::Result<StateCheckpoint, StoreError> {
            let genesis = self.load_genesis(session_id)?;
            if height != 0 {
                if let Some(record) = self
                    .inner
                    .lock()
                    .unwrap()
                    .chain
                    .iter()
                    .rev()
                    .find(|record| record.block.number < height && !record.state_snapshot.is_empty())
                {
                    return Ok(StateCheckpoint {
                        session_id,
                        finalised_height: Some(record.block.number),
                        state_root: record.block.state_root_after,
                        state_snapshot: record.state_snapshot.clone(),
                    });
                }
            }
            Ok(StateCheckpoint {
                session_id,
                finalised_height: None,
                state_root: genesis.config.initial_state_root,
                state_snapshot: genesis.state_snapshot,
            })
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

    fn application_profile() -> ApplicationProfile {
        ApplicationProfile {
            command_identity_required: false,
            application_id: "myelin-test-application".to_owned(),
            program_digest: [11; 32],
            input_schema_hash: [12; 32],
            state_codec_hash: [13; 32],
            logical_time_policy_hash: [14; 32],
            entropy_policy_hash: [15; 32],
            vm: ApplicationVmProfile { spawn_ipc_required: false },
            resources: ApplicationResourceEnvelope {
                max_frame_inputs: 1_000,
                max_frame_input_bytes: 1_000_000,
                max_frame_cycles: 10_000_000,
                max_logical_time_span: 1_000_000,
                max_inspect_query_bytes: 1_024,
                max_inspect_result_bytes: 4_096,
            },
            court_profile_hash: [16; 32],
            handoff_policy_hash: [17; 32],
        }
    }

    fn frame_input(input_count: u64) -> FrameInput {
        FrameInput {
            commands: vec![],
            start_position: 0,
            end_position: input_count,
            logical_time_start: 0,
            logical_time_end: input_count,
            payload: if input_count == 0 { Vec::new() } else { b"canonical-test-input".to_vec() },
        }
    }

    struct TestInspector {
        expected_root: Hash32,
        result_bytes: Option<usize>,
    }

    impl InspectPort for TestInspector {
        fn inspect(&self, context: &InspectContext<'_>, snapshot: &[u8], query: &[u8]) -> std::result::Result<Vec<u8>, String> {
            assert_eq!(context.state_root, self.expected_root);
            if let Some(result_bytes) = self.result_bytes {
                return Ok(vec![0; result_bytes]);
            }
            let mut result = snapshot.len().to_le_bytes().to_vec();
            result.extend_from_slice(query);
            Ok(result)
        }
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
            application_profile: application_profile(),
            initial_state_root: root,
            initial_input_position: 0,
            initial_logical_time: 0,
            initial_timestamp_ms: 0,
            predecessor: None,
            max_block_transactions: 100,
            max_block_bytes: 1_000_000,
            max_future_drift_ms: 1_000,
            commit_limits: CommitResourceLimits::default(),
            protocol_effects: ProtocolEffectsPolicy::Disabled,
        };
        let mut chain = SessionChain::create(config, verifier.clone(), executor, Arc::clone(&store)).unwrap();
        let tx = CellTx::new(vec![CellInput::new(input, 0)], vec![], vec![cell_output(900)], vec![vec![]], vec![]).unwrap();
        let prepared = chain
            .prepare_block(BlockRequest {
                transactions: vec![serialize_transaction_molecule(&tx).unwrap()],
                frame_input: frame_input(1),
                scheduler_commitment: [5; 32],
                data_commitments: vec![],
                timestamp_ms: 10,
                local_now_ms: 10,
                successor: None,
                handoffs: vec![],
                consume_handoffs: vec![],
                handoff_actions: vec![],
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
        let inspection_head = chain.head().clone();
        let stored_blocks = store.inner.lock().unwrap().chain.len();
        let inspection = chain
            .inspect_head(&TestInspector { expected_root: inspection_head.state_root, result_bytes: None }, b"live-cells")
            .unwrap();
        assert_eq!(inspection.state_root, head.state_root);
        assert_eq!(chain.head(), &inspection_head);
        assert_eq!(store.inner.lock().unwrap().chain.len(), stored_blocks);
        assert!(chain
            .inspect_head(&TestInspector { expected_root: inspection_head.state_root, result_bytes: Some(5_000) }, b"too-large-result")
            .is_err());
        assert!(chain
            .inspect_head(&TestInspector { expected_root: inspection_head.state_root, result_bytes: Some(0) }, &[0; 2_000])
            .is_err());
        let replay = chain
            .replay_range(0, 0, |snapshot| {
                CellTransitionExecutor::from_snapshot(snapshot, Arc::new(|_: &CellTx, _: &[ResolvedStateInput]| Ok(10)))
            })
            .unwrap();
        assert_eq!(replay.state_root_before, root);
        assert_eq!(replay.state_root_after, head.state_root);
        assert_eq!(replay.total_cycles, 10);

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
        let mut wrong_profile = application_profile();
        wrong_profile.program_digest = [99; 32];
        assert!(matches!(
            SessionChain::recover([4; 32], &wrong_profile, verifier.clone(), restored.clone(), Arc::clone(&store)),
            Err(SessionError::Recovery(_))
        ));
        assert!(matches!(
            SessionChain::recover([4; 32], &application_profile(), wrong_verifier, restored.clone(), Arc::clone(&store)),
            Err(SessionError::InvalidConfig(_))
        ));
        let recovered =
            SessionChain::recover([4; 32], &application_profile(), verifier.clone(), restored, Arc::clone(&store)).unwrap();
        assert_eq!(recovered.head(), &head);
        // A paused cursor must still reject corruption/missing data in its
        // unverified suffix. Its captured head alone grants no trust.
        for remove in [false, true] {
            let cancellation = RecoveryCancellation::default();
            let RecoveryOutcome::Paused(cursor) = RecoveryAudit::new([4; 32], application_profile(), verifier.clone(), store.clone())
                .advance(RecoveryBudget { max_blocks: 0, ..Default::default() }, &cancellation)
                .unwrap()
            else {
                panic!("block audit unexpectedly completed")
            };
            let ReplayOutcome::Paused(replay_cursor) = recovered
                .start_replay_range(0, 0, None, |snapshot| {
                    CellTransitionExecutor::from_snapshot(snapshot, Arc::new(|_: &CellTx, _: &[ResolvedStateInput]| Ok(10)))
                })
                .advance(RecoveryBudget { max_blocks: 0, ..Default::default() }, &cancellation)
                .unwrap()
            else {
                panic!("block replay unexpectedly completed")
            };
            let original = store.inner.lock().unwrap().chain[0].clone();
            if remove {
                store.inner.lock().unwrap().chain.clear();
            } else {
                store.inner.lock().unwrap().chain[0].frame_input.payload[0] ^= 1;
            }
            assert!(cursor.resume(RecoveryBudget::default(), &cancellation).is_err());
            assert!(replay_cursor.resume(RecoveryBudget::default(), &cancellation).is_err());
            store.inner.lock().unwrap().chain = vec![original];
        }
        store.inner.lock().unwrap().head.as_mut().unwrap().block_hash[0] ^= 1;
        let error = recovered
            .replay_range(0, 0, |snapshot| {
                CellTransitionExecutor::from_snapshot(snapshot, Arc::new(|_: &CellTx, _: &[ResolvedStateInput]| Ok(10)))
            })
            .unwrap_err();
        assert!(error.to_string().contains("reexecuted head does not match"), "{error}");
        store.inner.lock().unwrap().head = Some(head.clone());
        store.inner.lock().unwrap().chain[0].frame_input.payload[0] ^= 1;
        assert!(recovered
            .replay_range(0, 0, |snapshot| {
                CellTransitionExecutor::from_snapshot(snapshot, Arc::new(|_: &CellTx, _: &[ResolvedStateInput]| Ok(10)))
            })
            .is_err());
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
            application_profile: application_profile(),
            initial_state_root: [2; 32],
            initial_input_position: 0,
            initial_logical_time: 0,
            initial_timestamp_ms: 0,
            predecessor: None,
            max_block_transactions: 10,
            max_block_bytes: 100,
            max_future_drift_ms: 1,
            commit_limits: CommitResourceLimits::default(),
            protocol_effects: ProtocolEffectsPolicy::Disabled,
        };
        let mut encoded = config.encode().unwrap();
        let last = encoded.len() - 2;
        encoded[last] ^= 1;
        assert!(SessionConfig::decode(&encoded).is_err());
    }

    #[test]
    fn binary_record_codec_rejects_noncanonical_internal_formats() {
        let config = SessionConfig {
            session_id: [1; 32],
            consensus_kind: ConsensusKind::ProofOfAuthority,
            consensus_config_commitment: [3; 32],
            consensus_module_commitment: [4; 32],
            consensus_wal_schema_hash: [5; 32],
            application_profile: application_profile(),
            initial_state_root: [2; 32],
            initial_input_position: 0,
            initial_logical_time: 0,
            initial_timestamp_ms: 0,
            predecessor: None,
            max_block_transactions: 10,
            max_block_bytes: 100,
            max_future_drift_ms: 1,
            commit_limits: CommitResourceLimits::default(),
            protocol_effects: ProtocolEffectsPolicy::Disabled,
        };
        assert!(SessionConfig::decode(br#"{"format_version":4}"#).is_err());
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
            application_profile: application_profile(),
            initial_state_root: executor.state_root(),
            initial_input_position: 0,
            initial_logical_time: 0,
            initial_timestamp_ms: 0,
            predecessor: None,
            max_block_transactions: 1,
            max_block_bytes: 1,
            max_future_drift_ms: 1,
            commit_limits: CommitResourceLimits::default(),
            protocol_effects: ProtocolEffectsPolicy::Disabled,
        };
        let mut chain = SessionChain::create(config, verifier, executor, Arc::new(MemoryStore::new())).unwrap();
        let prepared = chain
            .prepare_block(BlockRequest {
                transactions: vec![],
                frame_input: frame_input(0),
                scheduler_commitment: [5; 32],
                data_commitments: vec![],
                timestamp_ms: 1,
                local_now_ms: 1,
                successor: None,
                handoffs: vec![],
                consume_handoffs: vec![],
                handoff_actions: vec![],
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
                application_profile: application_profile(),
                initial_state_root: executor.state_root(),
                initial_input_position: 0,
                initial_logical_time: 0,
                initial_timestamp_ms: 0,
                predecessor: None,
                max_block_transactions: 10,
                max_block_bytes: 1_000_000,
                max_future_drift_ms: 1,
                commit_limits: CommitResourceLimits::default(),
                protocol_effects: ProtocolEffectsPolicy::Disabled,
            };
            let chain = SessionChain::create(config, verifier, executor.clone(), Arc::new(MemoryStore::new())).unwrap();
            blocks.push(
                chain
                    .prepare_block(BlockRequest {
                        transactions: vec![transaction.clone()],
                        frame_input: frame_input(1),
                        scheduler_commitment: [5; 32],
                        data_commitments: vec![[6; 32]],
                        timestamp_ms: 1,
                        local_now_ms: 1,
                        successor: None,
                        handoffs: vec![],
                        consume_handoffs: vec![],
                        handoff_actions: vec![],
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
            // Application and resource commitments are workload-invariant.
            // The new effect context binds the exact genesis (including finality).
            assert_eq!(block.data_commitments.len(), 3);
            assert_eq!(block.data_commitments[0], blocks[0].data_commitments[0]);
            assert_ne!(block.data_commitments[1], blocks[0].data_commitments[1]);
            assert_eq!(block.data_commitments[2], blocks[0].data_commitments[2]);
            assert_eq!(block.scheduler_commitment, blocks[0].scheduler_commitment);
        }
        assert_ne!(blocks[0].hash(), blocks[1].hash());
        assert_ne!(blocks[0].hash(), blocks[2].hash());
        assert_ne!(blocks[1].hash(), blocks[2].hash());
    }
}
