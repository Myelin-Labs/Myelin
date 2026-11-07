//! Explicit external source proofs. Verification does not consume a handoff or
//! insert remote history into a local session chain.
use crate::{
    bounded_codec, decode_record, encode_record, ensure_consensus_config, validate_handoff_source, validate_handoff_trust,
    EvidenceReceipt, EvidenceRecord, EvidenceStage, FinalisedBlockRecord, FinalityVerifier, HandoffTarget, Hash32, OutboxMessage,
    Result, SessionConfig, SessionError, SessionHandoff, SessionStore,
};
use serde::{Deserialize, Serialize};

pub const MAX_REMOTE_PROOF_BYTES: usize = 64 * 1024 * 1024;
pub const REMOTE_HANDOFF_FORMAT_VERSION: u16 = 1;

/// Operator-selected trust inputs, supplied independently of the proof. Exact
/// genesis commitments bind the source rules/authority and target intake profile.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct HandoffImportPolicy {
    pub source_genesis: Hash32,
    pub target_genesis: Hash32,
    pub max_proof_bytes: u64,
}
impl HandoffImportPolicy {
    pub fn new(source: &SessionConfig, target: &SessionConfig, max_proof_bytes: usize) -> Result<Self> {
        source.validate()?;
        target.validate()?;
        validate_handoff_trust(target, source)?;
        let policy =
            Self { source_genesis: source.commitment(), target_genesis: target.commitment(), max_proof_bytes: max_proof_bytes as u64 };
        policy.validate()?;
        Ok(policy)
    }
    pub fn validate(&self) -> Result<()> {
        if self.source_genesis == [0; 32]
            || self.target_genesis == [0; 32]
            || self.source_genesis == self.target_genesis
            || self.max_proof_bytes == 0
            || self.max_proof_bytes > MAX_REMOTE_PROOF_BYTES as u64
        {
            return Err(invalid("invalid remote import trust or byte limit"));
        }
        Ok(())
    }
    pub fn commitment(&self) -> Result<Hash32> {
        self.validate()?;
        let mut h = blake3::Hasher::new();
        h.update(b"myelin:remote-handoff-import-policy:v1");
        h.update(&encode_record(self)?);
        Ok(*h.finalize().as_bytes())
    }
}

/// A required pipeline can reference an earlier outbox block. That block has
/// its own source-finality proof; it is not trusted because the relay supplied it.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct HandoffEvidenceProof {
    #[serde(deserialize_with = "bounded_codec::bounded_bytes")]
    pub message_block: Vec<u8>,
    pub record: EvidenceRecord,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct HandoffProofBundle {
    pub format_version: u16,
    pub handoff_id: Hash32,
    pub target_genesis: Hash32,
    #[serde(deserialize_with = "bounded_codec::bounded_bytes")]
    pub source_config: Vec<u8>,
    #[serde(deserialize_with = "bounded_codec::bounded_bytes")]
    pub issuing_block: Vec<u8>,
    pub evidence: Option<HandoffEvidenceProof>,
}

/// The application supplies exact schema, owner authorization and source-policy
/// checks. Pipeline adapters must verify actual evidence bytes at every stage.
/// Implementations are local trusted code, never selected by an incoming bundle.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum HandoffImportKind {
    CopyableReceipt,
    CoordinatedScarce { terminal_policy: Hash32 },
}
pub trait HandoffImportVerifier: Send + Sync + 'static {
    fn verify_application(
        &self,
        source: &SessionConfig,
        target: &SessionConfig,
        handoff: &SessionHandoff,
    ) -> Result<HandoffImportKind>;
    fn terminal_policy(&self) -> Option<crate::RemoteTerminalPolicy> {
        None
    }
    fn verify_cancellation(&self, _source: &SessionConfig, _handoff: &SessionHandoff, _authorization: &[u8]) -> Result<()> {
        Err(invalid("application does not support remote cancellation"))
    }
    fn verify_evidence(
        &self,
        message: &OutboxMessage,
        stage: &EvidenceStage,
        previous: Option<&EvidenceReceipt>,
        evidence: &[u8],
    ) -> Result<()>;
}

pub struct RemoteFinalityVerifiers {
    pub source: std::sync::Arc<dyn FinalityVerifier>,
    pub target: std::sync::Arc<dyn FinalityVerifier>,
}

struct ImportRegistration {
    policy: HandoffImportPolicy,
    source: SessionConfig,
    target: SessionConfig,
    finality: RemoteFinalityVerifiers,
    application: std::sync::Arc<dyn HandoffImportVerifier>,
}

/// Locally configured verifiers. No registration is taken from an incoming
/// proof or restored inbox; changing these trust inputs is an operator action.
#[derive(Default)]
pub struct HandoffImportRegistry {
    policies: std::collections::BTreeMap<Hash32, ImportRegistration>,
}
impl HandoffImportRegistry {
    /// Resolve an immutable source config from independent local registration,
    /// without creating or claiming a local source SessionGenesis/history.
    pub fn source_config(&self, session_id: Hash32, genesis: Hash32) -> Option<SessionConfig> {
        self.policies
            .values()
            .find(|entry| entry.source.session_id == session_id && entry.policy.source_genesis == genesis)
            .map(|entry| entry.source.clone())
    }
    pub fn register(
        &mut self,
        policy: HandoffImportPolicy,
        source: SessionConfig,
        target: SessionConfig,
        finality: RemoteFinalityVerifiers,
        application: std::sync::Arc<dyn HandoffImportVerifier>,
    ) -> Result<Hash32> {
        policy.validate()?;
        source.validate()?;
        target.validate()?;
        ensure_consensus_config(finality.source.as_ref(), &source)?;
        ensure_consensus_config(finality.target.as_ref(), &target)?;
        validate_handoff_trust(&target, &source)?;
        match (target.protocol_effects.remote_terminal(), application.terminal_policy()) {
            (Some(expected), Some(actual)) if *expected == actual => {}
            (None, None) => {}
            _ => return Err(invalid("terminal verifier differs from genesis coordination policy")),
        }
        if source.commitment() != policy.source_genesis || target.commitment() != policy.target_genesis {
            return Err(invalid("registered genesis differs from local policy"));
        }
        let id = policy.commitment()?;
        if self.policies.len() >= 1024 || self.policies.contains_key(&id) {
            return Err(invalid("duplicate import policy or registry bound exceeded"));
        }
        self.policies.insert(id, ImportRegistration { policy, source, target, finality, application });
        Ok(id)
    }
    pub fn terminal_registration(&self, config: &SessionConfig, terminal: Hash32) -> Result<Hash32> {
        self.policies
            .iter()
            .find_map(|(id, entry)| {
                let policy = entry.application.terminal_policy()?;
                (policy.commitment().ok()? == terminal && (entry.source == *config || entry.target == *config)).then_some(*id)
            })
            .ok_or_else(|| invalid("current remote application has no independently registered terminal scope"))
    }
    pub fn classify_registered(
        &self,
        source: &SessionConfig,
        target: &SessionConfig,
        handoff: &SessionHandoff,
    ) -> Result<Option<(Hash32, HandoffImportKind)>> {
        let Some((id, entry)) = self.policies.iter().find(|(_, entry)| entry.source == *source && entry.target == *target) else {
            return Ok(None);
        };
        Ok(Some((*id, entry.application.verify_application(source, target, handoff)?)))
    }
    pub fn terminal_policy(&self, id: Hash32) -> Result<crate::RemoteTerminalPolicy> {
        self.policies
            .get(&id)
            .and_then(|entry| entry.application.terminal_policy())
            .ok_or_else(|| invalid("no independently configured terminal policy"))
    }
    pub fn verify_terminal(
        &self,
        record: &HandoffInboxRecord,
        certificate: &crate::RemoteTerminalCertificate,
    ) -> Result<crate::VerifiedRemoteTerminal> {
        let verified = self.verify(record)?;
        let policy = self.terminal_policy(record.policy)?;
        if verified.kind() != (HandoffImportKind::CoordinatedScarce { terminal_policy: policy.commitment()? }) {
            return Err(invalid("terminal policy differs from scarce application admission"));
        }
        certificate.verify(&policy, verified.policy().source_genesis, verified.policy().target_genesis, verified.handoff().id())
    }
    pub fn verify_cancellation(&self, record: &HandoffInboxRecord, authorization: &[u8]) -> Result<()> {
        let verified = self.verify(record)?;
        let entry = self.policies.get(&record.policy).ok_or_else(|| invalid("unrecognized import policy"))?;
        entry.application.verify_cancellation(&entry.source, verified.handoff(), authorization)
    }
    pub fn verify_target_block(&self, policy: Hash32, bytes: &[u8]) -> Result<FinalisedBlockRecord> {
        if bytes.len() > MAX_REMOTE_PROOF_BYTES {
            return Err(invalid("remote target record exceeds proof ceiling"));
        }
        let entry = self.policies.get(&policy).ok_or_else(|| invalid("unrecognized import policy"))?;
        verify_remote_block(bytes, &entry.target, entry.finality.target.as_ref())
    }
    pub fn configs(&self, policy: Hash32) -> Result<(&SessionConfig, &SessionConfig)> {
        let entry = self.policies.get(&policy).ok_or_else(|| invalid("unrecognized import policy"))?;
        Ok((&entry.source, &entry.target))
    }
    pub fn verify(&self, record: &HandoffInboxRecord) -> Result<VerifiedHandoffImport> {
        let registration = self.policies.get(&record.policy).ok_or_else(|| invalid("unrecognized remote import policy"))?;
        record.proof.verify(
            &registration.policy,
            &registration.source,
            &registration.target,
            registration.finality.source.as_ref(),
            registration.application.as_ref(),
        )
    }
}

/// Durable untrusted bytes plus a reference to independently configured trust.
/// Decoding this record never constructs a verified import capability.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct HandoffInboxRecord {
    pub policy: Hash32,
    pub proof: HandoffProofBundle,
}
impl HandoffInboxRecord {
    pub fn encode(&self) -> Result<Vec<u8>> {
        self.proof.encode(MAX_REMOTE_PROOF_BYTES)?;
        encode_record(self)
    }
    pub fn decode(bytes: &[u8]) -> Result<Self> {
        if bytes.len() > MAX_REMOTE_PROOF_BYTES + 1024 {
            return Err(invalid("remote inbox record byte limit"));
        }
        let record: Self = decode_record(bytes)?;
        record.proof.encode(MAX_REMOTE_PROOF_BYTES)?;
        Ok(record)
    }
}

/// Only proof verification constructs this capability. Durable import will
/// retain the original proof; possessing it does not authorize target consumption.
#[derive(Clone, Debug)]
pub struct VerifiedHandoffImport {
    policy: HandoffImportPolicy,
    proof: HandoffProofBundle,
    handoff: SessionHandoff,
    kind: HandoffImportKind,
}
impl VerifiedHandoffImport {
    pub fn retained_size(&self) -> Result<u64> {
        #[derive(Serialize)]
        struct InboxRef<'a> {
            policy: Hash32,
            proof: &'a HandoffProofBundle,
        }
        let body = InboxRef { policy: self.policy.commitment()?, proof: &self.proof };
        Ok(bounded_codec::size_with_limit(&body, "remote_inbox_bytes", bounded_codec::MAX_RECORD_BYTES as u64)?
            + crate::RECORD_HEADER_BYTES as u64)
    }
    pub fn kind(&self) -> HandoffImportKind {
        self.kind
    }
    pub fn inbox_record(&self) -> Result<HandoffInboxRecord> {
        Ok(HandoffInboxRecord { policy: self.policy.commitment()?, proof: self.proof.clone() })
    }
    pub fn policy(&self) -> &HandoffImportPolicy {
        &self.policy
    }
    pub fn proof(&self) -> &HandoffProofBundle {
        &self.proof
    }
    pub fn handoff(&self) -> &SessionHandoff {
        &self.handoff
    }
    /// A source namespace includes its immutable genesis, rather than a relay's
    /// sender string. The handoff ID already commits exact source block contents.
    pub fn inbox_key(&self) -> Hash32 {
        let mut h = blake3::Hasher::new();
        h.update(b"myelin:remote-handoff-inbox:v1");
        h.update(&self.policy.source_genesis);
        h.update(&self.handoff.id());
        *h.finalize().as_bytes()
    }
}

/// Explicit source resolution shared by live execution, commit and replay.
/// Remote proof reads never manufacture a local source genesis/block index.
pub struct ResolvedHandoffSource {
    pub config: SessionConfig,
    pub record: FinalisedBlockRecord,
    remote_target: Option<Hash32>,
    proof_bytes: u64,
}
impl ResolvedHandoffSource {
    pub fn proof_bytes(&self) -> u64 {
        self.proof_bytes
    }
    pub fn validate_target(&self, target: &SessionConfig) -> Result<()> {
        validate_handoff_trust(target, &self.config)?;
        if self.remote_target.is_some_and(|commitment| commitment != target.commitment()) {
            return Err(invalid("remote import was verified for another target genesis"));
        }
        Ok(())
    }
    pub fn evidence_was_independently_verified(&self) -> bool {
        self.remote_target.is_some()
    }
}
pub fn resolve_handoff_source<S: SessionStore>(store: &S, handoff: &SessionHandoff) -> Result<ResolvedHandoffSource> {
    let source = if let Some(import) = store.load_handoff_import(handoff.id())? {
        if import.handoff() != handoff {
            return Err(invalid("remote inbox differs from consumed handoff"));
        }
        ResolvedHandoffSource {
            config: SessionConfig::decode(&import.proof().source_config)?,
            record: FinalisedBlockRecord::decode(&import.proof().issuing_block)?,
            remote_target: Some(import.policy().target_genesis),
            proof_bytes: import.retained_size()?,
        }
    } else {
        ResolvedHandoffSource {
            config: store.load_genesis(handoff.source_session_id)?.config,
            record: store.load_block(handoff.source_session_id, handoff.source_height)?,
            remote_target: None,
            proof_bytes: 0,
        }
    };
    validate_handoff_source(&source.config, &source.record, handoff)?;
    Ok(source)
}
pub fn validate_handoff_evidence<S: SessionStore>(store: &S, handoff: &SessionHandoff, source: &ResolvedHandoffSource) -> Result<()> {
    if !source.evidence_was_independently_verified() {
        if let Some(required) = &handoff.evidence_requirement {
            let evidence = store
                .load_evidence(handoff.source_session_id, required.message_id)?
                .ok_or_else(|| invalid("required handoff evidence is absent"))?;
            if evidence.descriptor.commitment() != required.pipeline_commitment
                || evidence.receipts.len() <= required.minimum_stage_index as usize
            {
                return Err(invalid("required handoff evidence stage has not been reached"));
            }
        }
    }
    Ok(())
}

impl HandoffProofBundle {
    pub fn export<S: SessionStore>(store: &S, handoff_id: Hash32, target: &SessionConfig) -> Result<Self> {
        target.validate()?;
        let stored = store.load_handoff(handoff_id)?.ok_or_else(|| invalid("export handoff absent"))?;
        if stored.handoff.id() != handoff_id || !stored.handoff.accepts_target(target) {
            return Err(invalid("export handoff identity or exact target mismatch"));
        }
        if !matches!(stored.handoff.target, HandoffTarget::Session(_)) {
            return Err(invalid("remote policy targets are unsupported"));
        }
        let source = store.load_genesis(stored.handoff.source_session_id)?.config;
        let mut issuing = store.load_block(source.session_id, stored.handoff.source_height)?;
        validate_handoff_source(&source, &issuing, &stored.handoff)?;
        issuing.state_snapshot.clear();
        let evidence = if let Some(required) = &stored.handoff.evidence_requirement {
            let record = store
                .load_evidence(source.session_id, required.message_id)?
                .ok_or_else(|| invalid("required export evidence absent"))?;
            // The source may have emitted the message before the issuing block.
            // Export uses bounded pages and a bounded search budget.
            let mut found = None;
            let mut reads = 0u64;
            let mut height = 0;
            while height <= stored.handoff.source_height {
                let page = store.load_chain_page(source.session_id, height, 16)?;
                if page.is_empty() {
                    break;
                }
                for mut block in page {
                    reads = reads.checked_add(block.encode()?.len() as u64).ok_or_else(|| invalid("export byte overflow"))?;
                    if reads > MAX_REMOTE_PROOF_BYTES as u64 {
                        return Err(invalid("evidence message lookup exceeds bounded export work"));
                    }
                    if block.block.number > stored.handoff.source_height {
                        break;
                    }
                    height = block.block.number.checked_add(1).ok_or_else(|| invalid("export height overflow"))?;
                    if block.outbox.iter().any(|m| m.id == required.message_id) {
                        block.state_snapshot.clear();
                        found = Some(block.encode()?);
                        break;
                    }
                }
                if found.is_some() {
                    break;
                }
            }
            Some(HandoffEvidenceProof { message_block: found.ok_or_else(|| invalid("evidence outbox block absent"))?, record })
        } else {
            None
        };
        let proof = Self {
            format_version: REMOTE_HANDOFF_FORMAT_VERSION,
            handoff_id,
            target_genesis: target.commitment(),
            source_config: source.encode()?,
            issuing_block: issuing.encode()?,
            evidence,
        };
        proof.encode(MAX_REMOTE_PROOF_BYTES)?;
        Ok(proof)
    }

    pub fn encode(&self, max_bytes: usize) -> Result<Vec<u8>> {
        self.validate_format()?;
        check_size(self, max_bytes)?;
        encode_record(self)
    }
    pub fn decode(bytes: &[u8], max_bytes: usize) -> Result<Self> {
        if max_bytes == 0 || max_bytes > MAX_REMOTE_PROOF_BYTES || bytes.len() > max_bytes {
            return Err(invalid("remote proof byte limit"));
        }
        let proof: Self = decode_record(bytes)?;
        proof.validate_format()?;
        check_size(&proof, max_bytes)?;
        Ok(proof)
    }

    pub fn verify(
        &self,
        policy: &HandoffImportPolicy,
        source: &SessionConfig,
        target: &SessionConfig,
        finality: &dyn FinalityVerifier,
        application: &dyn HandoffImportVerifier,
    ) -> Result<VerifiedHandoffImport> {
        self.validate_format()?;
        policy.validate()?;
        check_size(self, policy.max_proof_bytes as usize)?;
        source.validate()?;
        target.validate()?;
        if source.commitment() != policy.source_genesis
            || target.commitment() != policy.target_genesis
            || self.target_genesis != policy.target_genesis
            || SessionConfig::decode(&self.source_config)? != *source
        {
            return Err(invalid("proof differs from independently pinned source/target genesis"));
        }
        validate_handoff_trust(target, source)?;
        ensure_consensus_config(finality, source)?;
        let block = verify_remote_block(&self.issuing_block, source, finality)?;
        let mut matches = block.issued_handoffs.iter().filter(|h| h.id() == self.handoff_id);
        let handoff = matches.next().ok_or_else(|| invalid("handoff absent from signed source block"))?;
        if matches.next().is_some() || !matches!(handoff.target, HandoffTarget::Session(_)) || !handoff.accepts_target(target) {
            return Err(invalid("remote handoff is duplicate or has an unsupported/wrong target"));
        }
        validate_handoff_source(source, &block, handoff)?;
        let kind = application.verify_application(source, target, handoff)?;
        match (kind, target.protocol_effects.remote_terminal()) {
            (HandoffImportKind::CoordinatedScarce { terminal_policy }, Some(policy)) if policy.commitment()? == terminal_policy => {}
            (HandoffImportKind::CopyableReceipt, None) => {}
            _ => return Err(invalid("remote application classification differs from kernel genesis policy")),
        }
        match (&handoff.evidence_requirement, &self.evidence) {
            (None, None) => {}
            (Some(required), Some(evidence)) => {
                let message_block = verify_remote_block(&evidence.message_block, source, finality)?;
                if message_block.block.number > block.block.number {
                    return Err(invalid("evidence outbox message follows handoff issuance"));
                }
                if message_block.block.number == block.block.number && message_block.block.hash() != block.block.hash()
                    || message_block.block.number.checked_add(1) == Some(block.block.number)
                        && block.block.parent_hash != message_block.block.hash()
                {
                    return Err(invalid("required evidence belongs to a conflicting source history"));
                }
                let message = message_block
                    .outbox
                    .iter()
                    .find(|message| message.id == required.message_id)
                    .ok_or_else(|| invalid("required message absent from its signed block"))?;
                let record = &evidence.record;
                record.validate_for(source.session_id, message, &record.descriptor)?;
                if record.descriptor.commitment() != required.pipeline_commitment
                    || record.receipts.len() <= required.minimum_stage_index as usize
                {
                    return Err(invalid("required evidence pipeline/stage is missing"));
                }
                for (i, receipt) in record.receipts.iter().enumerate() {
                    application.verify_evidence(
                        message,
                        &record.descriptor.stages[i],
                        i.checked_sub(1).map(|j| &record.receipts[j]),
                        &receipt.evidence,
                    )?;
                }
            }
            _ => return Err(invalid("required evidence absent or unrequested evidence supplied")),
        }
        Ok(VerifiedHandoffImport { policy: policy.clone(), proof: self.clone(), handoff: handoff.clone(), kind })
    }
    fn validate_format(&self) -> Result<()> {
        if self.format_version != REMOTE_HANDOFF_FORMAT_VERSION {
            return Err(invalid("unsupported remote handoff proof format"));
        }
        Ok(())
    }
}

pub fn verify_remote_block(bytes: &[u8], config: &SessionConfig, verifier: &dyn FinalityVerifier) -> Result<FinalisedBlockRecord> {
    let record = FinalisedBlockRecord::decode_for_config(bytes, config)?;
    if record.block.session_id != config.session_id
        || record.block.consensus_kind != config.consensus_kind
        || record.block.application_profile_commitment != config.application_profile.commitment()
        || record.consensus_module_commitment != config.consensus_module_commitment
    {
        return Err(invalid("remote block belongs to another session/profile/consensus"));
    }
    let mut messages = std::collections::HashSet::new();
    for message in &record.outbox {
        message.validate()?;
        if !messages.insert(message.id) || !record.block.data_commitments.contains(&message.commitment()) {
            return Err(invalid("outbox message is duplicate or not committed by source finality"));
        }
    }
    // Reuse the complete signed-record checks. At height zero the predecessor
    // is pinned by genesis. Later records prove a signed source statement, not
    // an audit of an unavailable genesis prefix.
    let first = record.block.number == 0;
    let mut previous = crate::SessionHead {
        session_id: config.session_id,
        finalised_height: record.block.number.checked_sub(1),
        block_hash: if first { [0; 32] } else { record.block.parent_hash },
        state_root: if first { config.initial_state_root } else { record.block.state_root_before },
        timestamp_ms: if first { config.initial_timestamp_ms } else { record.block.timestamp_ms },
        input_position: if first { config.initial_input_position } else { record.frame.input_start },
        logical_time: if first { config.initial_logical_time } else { record.frame.logical_time_start },
        sealed_by_successor: None,
    };
    crate::audit_record(config, verifier, &mut previous, &record)?;
    Ok(record)
}
fn check_size<T: Serialize>(value: &T, max_bytes: usize) -> Result<()> {
    if max_bytes == 0 || max_bytes > MAX_REMOTE_PROOF_BYTES {
        return Err(invalid("invalid remote proof byte bound"));
    }
    // Include the current MREC envelope/checksum, not just postcard payload.
    bounded_codec::size_with_limit(value, "remote_proof_bytes", max_bytes.saturating_sub(crate::RECORD_HEADER_BYTES) as u64)?;
    Ok(())
}
fn invalid(message: &str) -> SessionError {
    SessionError::Continuity(message.to_owned())
}
