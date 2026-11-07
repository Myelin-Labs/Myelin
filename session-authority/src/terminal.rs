//! One trusted durable consume/cancel authority. Its database and signing key
//! must never be cloned or rolled back. An unavailable decision remains unknown.
use crate::{protocol, AuthorityError, AuthoritySigner, Result};
use myelin_session::{
    remote_cancellation_digest, FinalisedBlockRecord, HandoffBlockReference, HandoffImportKind, HandoffImportRegistry,
    HandoffInboxRecord, Hash32, RemoteTerminalCertificate, RemoteTerminalDecision, RemoteTerminalPolicy,
};
use rocksdb::{Options, WriteBatch, WriteOptions, DB};
use serde::{Deserialize, Serialize};
use std::{
    path::Path,
    sync::{Arc, Mutex},
};

const CONFIG: &[u8] = b"terminal-config-v1";
const USAGE: &[u8] = b"terminal-usage-v1";
const QUARANTINE: &[u8] = b"terminal-source-quarantine-v1";
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct TerminalAuthorityConfig {
    pub import_policy: Hash32,
    pub terminal: RemoteTerminalPolicy,
    pub max_claims: u64,
    pub max_bytes: u64,
}
/// Retain the original decision and its dependencies before a response can
/// escape. Repeated requests return the identical certificate, never resign it.
#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct Claim {
    #[serde(deserialize_with = "myelin_session::bounded_codec::bounded_bytes")]
    source: Vec<u8>,
    #[serde(deserialize_with = "myelin_session::bounded_codec::bounded_bytes")]
    candidate: Vec<u8>,
    #[serde(deserialize_with = "myelin_session::bounded_codec::bounded_bytes")]
    authorization: Vec<u8>,
    #[serde(deserialize_with = "myelin_session::bounded_codec::bounded_bytes")]
    certificate: Vec<u8>,
}
pub struct TerminalAuthority {
    db: DB,
    lock: Mutex<()>,
    config: TerminalAuthorityConfig,
    registry: Arc<HandoffImportRegistry>,
    signer: AuthoritySigner,
}
impl TerminalAuthority {
    pub fn create(
        path: impl AsRef<Path>,
        config: TerminalAuthorityConfig,
        registry: Arc<HandoffImportRegistry>,
        signer: AuthoritySigner,
    ) -> Result<Self> {
        Self::validate(&config, registry.as_ref(), &signer)?;
        // No create-if-missing recovery: an existing or partially initialized
        // identity must be diagnosed, never silently reset to no claims.
        std::fs::create_dir(path.as_ref()).map_err(|e| AuthorityError::Storage(e.to_string()))?;
        let mut options = Options::default();
        options.create_if_missing(true);
        let db = DB::open(&options, path.as_ref().join("claims")).map_err(storage)?;
        let mut writes = WriteOptions::default();
        writes.set_sync(true);
        db.put_opt(CONFIG, serde_json::to_vec(&config).map_err(invalid)?, &writes).map_err(storage)?;
        Ok(Self { db, lock: Mutex::new(()), config, registry, signer })
    }
    pub fn open(
        path: impl AsRef<Path>,
        config: TerminalAuthorityConfig,
        registry: Arc<HandoffImportRegistry>,
        signer: AuthoritySigner,
    ) -> Result<Self> {
        Self::validate(&config, registry.as_ref(), &signer)?;
        if !path.as_ref().join("claims/CURRENT").is_file() {
            return Err(invalid("terminal database is unavailable; restore original state, never reinitialize"));
        }
        let db = DB::open(&Options::default(), path.as_ref().join("claims")).map_err(storage)?;
        let saved =
            db.get(CONFIG).map_err(storage)?.ok_or_else(|| invalid("terminal authority has no durable identity; do not reset"))?;
        let actual: TerminalAuthorityConfig = serde_json::from_slice(&saved).map_err(invalid)?;
        if actual != config {
            return Err(invalid("terminal authority config differs from durable identity"));
        }
        let authority = Self { db, lock: Mutex::new(()), config, registry, signer };
        authority.audit()?;
        Ok(authority)
    }
    fn audit(&self) -> Result<()> {
        let mut count = 0u64;
        let mut used = 0u64;
        for item in self.db.prefix_iterator(b"claim") {
            let (key, bytes) = item.map_err(storage)?;
            if !key.starts_with(b"claim") {
                break;
            }
            if key.len() != 37 {
                return Err(invalid("invalid retained terminal claim key"));
            }
            count = count
                .checked_add(1)
                .filter(|n| *n <= self.config.max_claims)
                .ok_or_else(|| invalid("retained terminal claim quota exceeded"))?;
            used = used
                .checked_add(bytes.len() as u64)
                .and_then(|n| n.checked_add(256))
                .filter(|n| *n <= self.config.max_bytes)
                .ok_or_else(|| invalid("retained terminal byte quota exceeded"))?;
            self.load(key[5..].try_into().unwrap())?.ok_or_else(|| invalid("retained terminal decision disappeared"))?;
        }
        let mut expected = count.to_le_bytes().to_vec();
        expected.extend(used.to_le_bytes());
        match self.db.get(USAGE).map_err(storage)? {
            None if count == 0 => Ok(()),
            Some(actual) if actual == expected => Ok(()),
            _ => Err(invalid("terminal retention counters differ; do not reset or reconstruct missing claims")),
        }
    }
    fn validate(config: &TerminalAuthorityConfig, registry: &HandoffImportRegistry, signer: &AuthoritySigner) -> Result<()> {
        let policy = registry.terminal_policy(config.import_policy).map_err(protocol::session)?;
        let (source, target) = registry.configs(config.import_policy).map_err(protocol::session)?;
        if policy != config.terminal
            || policy.coordinator_key != signer.public_key()
            || source.session_id != policy.source_session
            || target.session_id != policy.target_session
            || config.max_claims == 0
            || config.max_claims > 65_536
            || config.max_bytes == 0
            || config.max_bytes > 8 * 1024 * 1024 * 1024
        {
            return Err(invalid("terminal authority trust/limits mismatch"));
        }
        Ok(())
    }
    /// Authenticated target finality binds an exact candidate, but does not assert
    /// that the target has committed. The source remains locked until completion.
    pub fn authorize_consume(&self, source: &HandoffInboxRecord, target_record: &[u8]) -> Result<RemoteTerminalCertificate> {
        let verified = self.verify_source(source)?;
        let target = self.registry.verify_target_block(source.policy, target_record).map_err(protocol::session)?;
        if target.consumed_handoff_ids != [verified.handoff().id()]
            || !target.issued_handoffs.is_empty()
            || target.successor.is_some()
            || !target.handoff_actions.is_empty()
            || target.block.timestamp_ms > verified.handoff().expires_at_ms
        {
            return Err(invalid("terminal authority requires one unexpired exact-target consumption"));
        }
        self.decide(
            source,
            target_record,
            &[],
            RemoteTerminalDecision::ConsumeAuthorized { candidate: HandoffBlockReference::from_block(&target.block) },
        )
    }
    pub fn authorize_cancel(&self, source: &HandoffInboxRecord, authorization: &[u8]) -> Result<RemoteTerminalCertificate> {
        self.verify_source(source)?;
        self.registry.verify_cancellation(source, authorization).map_err(protocol::session)?;
        let digest = remote_cancellation_digest(
            self.registry.configs(source.policy).map_err(protocol::session)?.0.commitment(),
            source.proof.handoff_id,
            authorization,
        )
        .map_err(protocol::session)?;
        self.decide(source, &[], authorization, RemoteTerminalDecision::Cancelled { authorization_digest: digest })
    }
    pub fn lookup(&self, handoff_id: Hash32) -> Result<Option<RemoteTerminalCertificate>> {
        let _lock = self.lock.lock().map_err(|_| AuthorityError::Uncertain("terminal lock poisoned".into()))?;
        if self.db.get(QUARANTINE).map_err(storage)?.is_some() {
            return Err(invalid("source is quarantined"));
        }
        self.load(handoff_id)
    }
    fn load(&self, id: Hash32) -> Result<Option<RemoteTerminalCertificate>> {
        let Some(bytes) = self.db.get(claim_key(id)).map_err(storage)? else { return Ok(None) };
        let claim: Claim = postcard::from_bytes(&bytes).map_err(invalid)?;
        let source = HandoffInboxRecord::decode(&claim.source).map_err(protocol::session)?;
        self.verify_source(&source)?;
        let certificate = RemoteTerminalCertificate::decode(&claim.certificate).map_err(protocol::session)?;
        self.registry.verify_terminal(&source, &certificate).map_err(protocol::session)?;
        if certificate.handoff_id != id {
            return Err(invalid("terminal claim key mismatch"));
        }
        match &certificate.decision {
            RemoteTerminalDecision::ConsumeAuthorized { candidate } => {
                let target = self.registry.verify_target_block(source.policy, &claim.candidate).map_err(protocol::session)?;
                if HandoffBlockReference::from_block(&target.block) != *candidate
                    || target.consumed_handoff_ids != [id]
                    || !claim.authorization.is_empty()
                {
                    return Err(invalid("retained consumption decision differs from original evidence"));
                }
            }
            RemoteTerminalDecision::Cancelled { authorization_digest } => {
                self.registry.verify_cancellation(&source, &claim.authorization).map_err(protocol::session)?;
                if *authorization_digest
                    != remote_cancellation_digest(certificate.source_genesis, id, &claim.authorization).map_err(protocol::session)?
                    || !claim.candidate.is_empty()
                {
                    return Err(invalid("retained cancellation decision differs from owner evidence"));
                }
            }
        }
        Ok(Some(certificate))
    }
    fn verify_source(&self, source: &HandoffInboxRecord) -> Result<myelin_session::VerifiedHandoffImport> {
        if source.policy != self.config.import_policy {
            return Err(invalid("unrecognized terminal import domain"));
        }
        let verified = self.registry.verify(source).map_err(protocol::session)?;
        if verified.kind()
            != (HandoffImportKind::CoordinatedScarce {
                terminal_policy: self.config.terminal.commitment().map_err(protocol::session)?,
            })
        {
            return Err(invalid("terminal authority requires its typed scarce application"));
        }
        Ok(verified)
    }
    fn decide(
        &self,
        source: &HandoffInboxRecord,
        candidate: &[u8],
        authorization: &[u8],
        decision: RemoteTerminalDecision,
    ) -> Result<RemoteTerminalCertificate> {
        let _lock = self.lock.lock().map_err(|_| AuthorityError::Uncertain("terminal lock poisoned".into()))?;
        if self.db.get(QUARANTINE).map_err(storage)?.is_some() {
            return Err(invalid("source is quarantined"));
        }
        let verified = self.verify_source(source)?;
        let issuing = FinalisedBlockRecord::decode(&source.proof.issuing_block).map_err(protocol::session)?;
        let mut history_key = b"source-height".to_vec();
        history_key.extend(issuing.block.number.to_be_bytes());
        let hash = issuing.block.hash();
        if let Some(old) = self.db.get(&history_key).map_err(storage)? {
            if old.as_slice() != hash {
                let mut writes = WriteOptions::default();
                writes.set_sync(true);
                self.db.put_opt(QUARANTINE, source.encode().map_err(protocol::session)?, &writes).map_err(storage)?;
                return Err(invalid("conflicting signed source history quarantined durably"));
            }
        }
        if let Some(original) = self.load(verified.handoff().id())? {
            if original.decision != decision {
                return Err(AuthorityError::Fenced("terminal decision is irreversible; reconcile original outcome".into()));
            }
            return Ok(original);
        }
        let mut certificate = RemoteTerminalCertificate::unsigned(
            &self.config.terminal,
            verified.policy().source_genesis,
            verified.policy().target_genesis,
            verified.handoff().id(),
            decision,
        )
        .map_err(protocol::session)?;
        certificate.signature = self.signer.sign(certificate.digest().map_err(protocol::session)?);
        let claim = Claim {
            source: source.encode().map_err(protocol::session)?,
            candidate: candidate.to_vec(),
            authorization: authorization.to_vec(),
            certificate: certificate.encode().map_err(protocol::session)?,
        };
        let bytes = postcard::to_allocvec(&claim).map_err(invalid)?;
        let (count, used) = match self.db.get(USAGE).map_err(storage)? {
            None => (0u64, 0u64),
            Some(bytes) if bytes.len() == 16 => {
                (u64::from_le_bytes(bytes[..8].try_into().unwrap()), u64::from_le_bytes(bytes[8..].try_into().unwrap()))
            }
            Some(_) => return Err(invalid("invalid terminal usage record")),
        };
        let count =
            count.checked_add(1).filter(|n| *n <= self.config.max_claims).ok_or_else(|| invalid("terminal claim quota reached"))?;
        let used = used
            .checked_add(bytes.len() as u64)
            .and_then(|n| n.checked_add(256))
            .filter(|n| *n <= self.config.max_bytes)
            .ok_or_else(|| invalid("terminal byte quota reached"))?;
        let mut usage = count.to_le_bytes().to_vec();
        usage.extend(used.to_le_bytes());
        let mut batch = WriteBatch::default();
        batch.put(claim_key(certificate.handoff_id), bytes);
        batch.put(USAGE, usage);
        batch.put(history_key, hash);
        let mut writes = WriteOptions::default();
        writes.set_sync(true);
        crate::failpoint("before-terminal-decision");
        self.db.write_opt(batch, &writes).map_err(|e| AuthorityError::Uncertain(e.to_string()))?;
        crate::failpoint("after-terminal-decision");
        Ok(certificate)
    }
}
fn claim_key(id: Hash32) -> Vec<u8> {
    let mut key = b"claim".to_vec();
    key.extend(id);
    key
}
fn invalid(error: impl ToString) -> AuthorityError {
    AuthorityError::Invalid(error.to_string())
}
fn storage(error: rocksdb::Error) -> AuthorityError {
    AuthorityError::Storage(error.to_string())
}

/// Private durable target reservation. It owns the original stamped request
/// and target finality proof before the terminal-authority request may escape.
#[derive(Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct RemotePending {
    pub(crate) request: Vec<u8>,
    pub(crate) record: Vec<u8>,
}
#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct TargetRequest {
    transactions: Vec<Vec<u8>>,
    frame_input: myelin_session::FrameInput,
    scheduler_commitment: Hash32,
    data_commitments: Vec<Hash32>,
    timestamp_ms: u64,
    local_now_ms: u64,
    consume_handoffs: Vec<Hash32>,
}
impl TargetRequest {
    fn from_request(request: myelin_session::BlockRequest) -> Self {
        Self {
            transactions: request.transactions,
            frame_input: request.frame_input,
            scheduler_commitment: request.scheduler_commitment,
            data_commitments: request.data_commitments,
            timestamp_ms: request.timestamp_ms,
            local_now_ms: request.local_now_ms,
            consume_handoffs: request.consume_handoffs,
        }
    }
    fn into_request(self) -> myelin_session::BlockRequest {
        myelin_session::BlockRequest {
            transactions: self.transactions,
            frame_input: self.frame_input,
            scheduler_commitment: self.scheduler_commitment,
            data_commitments: self.data_commitments,
            timestamp_ms: self.timestamp_ms,
            local_now_ms: self.local_now_ms,
            successor: None,
            handoffs: vec![],
            handoff_actions: vec![],
            consume_handoffs: self.consume_handoffs,
        }
    }
}
impl<E: myelin_session::TransitionExecutor> crate::SessionAuthority<E> {
    /// Reserve one candidate durably; return bytes for the external terminal
    /// request. Other writes and ownership rotation remain fenced until resolved.
    pub fn prepare_remote(
        &self,
        authorization: crate::CommitAuthorization,
        request: myelin_session::BlockRequest,
        authority_now_ms: u64,
    ) -> Result<Vec<u8>> {
        use myelin_consensus::{FinalityProof, SelectedConsensus};
        if request.consume_handoffs.len() != 1
            || !request.handoffs.is_empty()
            || request.successor.is_some()
            || !request.handoff_actions.is_empty()
        {
            return Err(invalid("remote preparation requires one isolated consumption"));
        }
        let mut inner = self.locked()?;
        let (request, prepared) = crate::prepare_authorized(&mut inner, &self.policy, authorization, request, authority_now_ms)?;
        let id = request.consume_handoffs[0];
        use myelin_session::SessionStore;
        let import = inner
            .store
            .load_handoff_import(id)
            .map_err(protocol::session)?
            .ok_or_else(|| invalid("remote scarce import is absent"))?;
        if !matches!(import.kind(), HandoffImportKind::CoordinatedScarce { .. }) {
            return Err(invalid("remote reservation requires typed scarce intake"));
        }
        let SelectedConsensus::ProofOfAuthority(engine) = inner.verifier.selected_consensus() else {
            return Err(invalid("target is not PoA"));
        };
        let proof = FinalityProof::ProofOfAuthority(
            engine
                .seal_from_signer(prepared.block().hash(), prepared.block().number, &inner.keys.finality)
                .map_err(protocol::session)?,
        );
        let record = prepared.candidate_record(proof, inner.chain.config().consensus_module_commitment);
        record.validate_resources(inner.chain.config()).map_err(protocol::session)?;
        let bytes = record.encode().map_err(protocol::session)?;
        let head = myelin_session::SessionHead {
            session_id: record.block.session_id,
            finalised_height: Some(record.block.number),
            block_hash: record.block.hash(),
            state_root: record.block.state_root_after,
            timestamp_ms: record.block.timestamp_ms,
            input_position: record.frame.input_end,
            logical_time: record.frame.logical_time_end,
            sealed_by_successor: None,
        };
        let mut ownership = inner.ownership.clone();
        ownership.pending_head = Some(head.encode().map_err(protocol::session)?);
        ownership.remote_pending = Some(RemotePending {
            request: serde_json::to_vec(&TargetRequest::from_request(request)).map_err(invalid)?,
            record: bytes.clone(),
        });
        crate::failpoint("before-remote-reservation");
        if let Err(error) = crate::persist(&inner.db, &ownership) {
            inner.uncertain = true;
            return Err(AuthorityError::Uncertain(error.to_string()));
        }
        inner.ownership = ownership;
        crate::failpoint("after-remote-reservation");
        Ok(bytes)
    }
    pub fn pending_remote_candidate(&self) -> Result<Option<Vec<u8>>> {
        Ok(self.locked()?.ownership.remote_pending.as_ref().map(|pending| pending.record.clone()))
    }
    /// No new request, timestamp or signature is chosen during recovery. Replay
    /// the original reservation and finish the exact authorized candidate.
    pub fn commit_remote(&self, certificate: &RemoteTerminalCertificate) -> Result<crate::CanonicalReceipt> {
        use myelin_session::SessionStore;
        let mut inner = self.locked()?;
        let RemoteTerminalDecision::ConsumeAuthorized { candidate } = &certificate.decision else {
            return Err(invalid("terminal decision is cancellation"));
        };
        if candidate.session_id != self.policy.session_id {
            return Err(invalid("terminal candidate belongs to another target authority"));
        }
        let Some(pending) = inner.ownership.remote_pending.clone() else {
            let record = inner.store.load_block(candidate.session_id, candidate.height).map_err(protocol::session)?;
            if HandoffBlockReference::from_block(&record.block) != *candidate || record.remote_terminals != [certificate.clone()] {
                return Err(invalid("no original canonical result for terminal certificate"));
            }
            return crate::receipt(&inner, candidate.height);
        };
        let original = FinalisedBlockRecord::decode(&pending.record).map_err(protocol::session)?;
        if HandoffBlockReference::from_block(&original.block) != *candidate {
            return Err(invalid("terminal authorization names another reserved candidate"));
        }
        let request: TargetRequest = serde_json::from_slice(&pending.request).map_err(invalid)?;
        let prepared = inner.chain.prepare_block(request.into_request()).map_err(protocol::session)?;
        if prepared.candidate_record(original.proof.clone(), original.consensus_module_commitment) != original {
            return Err(invalid("original pending transition no longer reproduces"));
        }
        let ledger = inner.ledger_session.ledger.clone();
        let _commit = ledger.commit_lock()?;
        crate::failpoint("before-remote-commit");
        if let Err(error) = inner.chain.commit_finalised(prepared.with_remote_terminals(vec![certificate.clone()]), original.proof) {
            inner.uncertain = true;
            return Err(AuthorityError::Uncertain(error.to_string()));
        }
        crate::failpoint("after-remote-commit");
        crate::reconcile_pending(&mut inner)?;
        crate::receipt(&inner, candidate.height)
    }
    pub fn release_cancelled_remote(&self, certificate: &RemoteTerminalCertificate) -> Result<()> {
        use myelin_session::SessionStore;
        let mut inner = self.locked()?;
        let pending = inner.ownership.remote_pending.as_ref().ok_or_else(|| invalid("no pending remote reservation"))?;
        let original = FinalisedBlockRecord::decode(&pending.record).map_err(protocol::session)?;
        if original.consumed_handoff_ids != [certificate.handoff_id]
            || !matches!(certificate.decision, RemoteTerminalDecision::Cancelled { .. })
        {
            return Err(invalid("cancellation differs from reserved handoff"));
        }
        let import = inner
            .store
            .load_handoff_import(certificate.handoff_id)
            .map_err(protocol::session)?
            .ok_or_else(|| invalid("pending import absent"))?;
        inner
            .store
            .handoff_import_registry()
            .ok_or_else(|| invalid("terminal trust unavailable"))?
            .verify_terminal(&import.inbox_record().map_err(protocol::session)?, certificate)
            .map_err(protocol::session)?;
        let mut ownership = inner.ownership.clone();
        ownership.pending_head = None;
        ownership.remote_pending = None;
        if let Err(error) = crate::persist(&inner.db, &ownership) {
            inner.uncertain = true;
            return Err(AuthorityError::Uncertain(error.to_string()));
        }
        inner.ownership = ownership;
        Ok(())
    }
}
