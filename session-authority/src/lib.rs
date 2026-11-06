// SPDX-License-Identifier: MIT
//! Explicit single-authority fencing, separate from routing/discovery registries.
//!
//! Workers possess only request-signing keys and non-authoritative replicas. The
//! authority owns canonical storage, PoA signing and external-effect certification.
//! This reference is a trusted single authority, not Byzantine/quorum HA. Restoring
//! a second authority from backup with the same secrets is outside its safety model.

mod protocol;
pub use protocol::*;
mod reconciliation;
pub use reconciliation::{CommandObservation, CommandReconciliation, ObservedStatus, VerifiedObservation};
mod claims;
pub use claims::{ClaimObservation, ClaimReconciliation, ClaimSession, ObservedClaim, MAX_RECONCILIATION_CLAIMS};
mod ledger;
pub use ledger::CanonicalLedger;
mod sink;
pub use sink::{CanonicalEffectStore, EffectDisposition};
pub mod network;

use myelin_consensus::{CommitteeSigner, ConsensusConfig, FinalityProof, SelectedConsensus};
use myelin_session::{
    BlockRequest, CommandIdentity, CommandStatus, FinalisedBlockRecord, Hash32, OutboxMessage, SessionChain, SessionConfig,
    SessionHead, SessionStore, StoreError, TransitionExecutor,
};
use myelin_session_runtime::RegisteredFinalityVerifier;
use myelin_session_store_rocksdb::RocksSessionStore;
use rocksdb::{Options, WriteOptions, DB};
use serde::{Deserialize, Serialize};
use std::{
    path::Path,
    sync::{Arc, Mutex},
};

const EPOCH_KEY: &[u8] = b"myelin-authoritative-writer-epochs";
const MAGIC: &[u8] = b"MFEN\x01";
const MAX_EPOCHS: usize = 1024;
const MAX_EPOCH_BYTES: usize = 4 * 1024 * 1024;

#[derive(Debug, thiserror::Error)]
pub enum AuthorityError {
    #[error("invalid fencing data: {0}")]
    Invalid(String),
    #[error("writer authorization rejected: {0}")]
    Authorization(String),
    #[error("writer fenced or recovery anchor stale: {0}")]
    Fenced(String),
    #[error("authority storage: {0}")]
    Storage(String),
    #[error("authority session: {0}")]
    Session(String),
    #[error("authority replay paused: {0}")]
    ReplayPaused(Box<myelin_session::PausedReplay>),
    #[error("authority outcome uncertain; reopen and audit before further operations: {0}")]
    Uncertain(String),
}
pub type Result<T> = std::result::Result<T, AuthorityError>;

pub struct AuthorityConfig {
    pub session: SessionConfig,
    pub consensus: ConsensusConfig,
    pub administrator: Hash32,
    pub effect_domain: Hash32,
}
pub struct AuthorityKeys {
    pub finality: CommitteeSigner,
    pub certificates: AuthoritySigner,
}

#[derive(Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct Ownership {
    policy: FencingPolicy,
    grants: Vec<WriterGrant>,
    canonical_head: Vec<u8>,
    /// Intent is durable before signing; recovery permits exactly this next
    /// head or the retained head, never an arbitrary stale canonical backup.
    pending_head: Option<Vec<u8>>,
}
struct Inner<E: TransitionExecutor> {
    ledger_session: ledger::LedgerSession,
    chain: SessionChain<E, RocksSessionStore>,
    store: Arc<RocksSessionStore>,
    verifier: Arc<RegisteredFinalityVerifier>,
    db: DB,
    ownership: Ownership,
    keys: AuthorityKeys,
    uncertain: bool,
}

/// All grants, preparation, signing and canonical commits serialize under one
/// authority lock. Store handles and signer secrets never escape to workers.
pub struct SessionAuthority<E: TransitionExecutor> {
    inner: Mutex<Inner<E>>,
    policy: FencingPolicy,
}
impl<E: TransitionExecutor> SessionAuthority<E> {
    /// Create only an empty canonical deployment, or recover the same existing
    /// deployment. Never adopt a nonempty unfenced store or silently reset epochs.
    pub fn open(
        ledger: Arc<CanonicalLedger>,
        epoch_path: impl AsRef<Path>,
        config: AuthorityConfig,
        keys: AuthorityKeys,
        initial: E,
        restore: impl FnOnce(&[u8]) -> std::result::Result<E, String>,
    ) -> Result<Self> {
        let poa = config
            .consensus
            .proof_of_authority
            .as_ref()
            .ok_or_else(|| AuthorityError::Invalid("fenced authority supports explicit single-key PoA only".into()))?;
        if poa.authorities.len() != 1
            || poa.authorities[0].id != keys.finality.validator_id()
            || poa.authorities[0].public_key != keys.finality.ckb_public_key()
            || keys.finality.public_key() == config.administrator
            || keys.finality.public_key() == keys.certificates.public_key()
        {
            return Err(AuthorityError::Invalid("finality key/config mismatch or overlapping credential roles".into()));
        }
        let policy = FencingPolicy::new(&config.session, config.administrator, keys.certificates.public_key(), config.effect_domain)?;
        let verifier = Arc::new(RegisteredFinalityVerifier::new(config.consensus).map_err(protocol::session)?);
        let mut options = Options::default();
        options.create_if_missing(true);
        // Both ownership and canonical storage are exclusively owned before
        // mutating either. A shared ledger admits one live authority per session.
        let db = DB::open(&options, epoch_path).map_err(storage)?;
        let ledger_session = ledger.register(config.session.session_id)?;
        let store = ledger.store.clone();
        let existing = db.get(EPOCH_KEY).map_err(storage)?;
        let ownership = match existing {
            Some(bytes) => decode(&bytes)?,
            None => {
                if db.iterator(rocksdb::IteratorMode::Start).next().transpose().map_err(storage)?.is_some() {
                    return Err(AuthorityError::Invalid("nonempty epoch store has no ownership record".into()));
                }
                match store.load_head(config.session.session_id) {
                    Ok(head) if head.finalised_height.is_some() => {
                        return Err(AuthorityError::Invalid("nonempty canonical history has no fencing record".into()))
                    }
                    Err(StoreError::NotFound(_)) | Ok(_) => {}
                    Err(error) => return Err(protocol::session(error)),
                }
                let value = Ownership {
                    policy: policy.clone(),
                    grants: vec![],
                    canonical_head: initial_head(&config.session).encode().map_err(protocol::session)?,
                    pending_head: None,
                };
                persist(&db, &value)?;
                value
            }
        };
        if ownership.policy != policy {
            return Err(AuthorityError::Invalid("epoch policy/genesis/key mismatch".into()));
        }
        let chain = match store.load_genesis(config.session.session_id) {
            Ok(genesis) => {
                if genesis.config != config.session {
                    return Err(AuthorityError::Invalid("canonical genesis mismatch".into()));
                }
                let (_, snapshot) = SessionChain::<E, RocksSessionStore>::recovery_snapshot(&*store, config.session.session_id)
                    .map_err(protocol::session)?;
                let executor = restore(&snapshot).map_err(AuthorityError::Session)?;
                SessionChain::recover(
                    config.session.session_id,
                    &config.session.application_profile,
                    verifier.clone(),
                    executor,
                    store.clone(),
                )
                .map_err(protocol::session)?
            }
            Err(StoreError::NotFound(_)) => {
                SessionChain::create(config.session, verifier.clone(), initial, store.clone()).map_err(protocol::session)?
            }
            Err(error) => return Err(protocol::session(error)),
        };
        let mut inner = Inner { ledger_session, chain, store, verifier, db, ownership, keys, uncertain: false };
        audit(&inner)?;
        reconcile_pending(&mut inner)?;
        Ok(Self { inner: Mutex::new(inner), policy })
    }
    pub fn policy(&self) -> &FencingPolicy {
        &self.policy
    }
    fn locked(&self) -> Result<std::sync::MutexGuard<'_, Inner<E>>> {
        let inner = self.inner.lock().map_err(|_| AuthorityError::Uncertain("authority mutex poisoned".into()))?;
        if inner.uncertain {
            return Err(AuthorityError::Uncertain("previous write failed".into()));
        }
        Ok(inner)
    }
    pub fn head(&self) -> Result<SessionHead> {
        Ok(self.locked()?.chain.head().clone())
    }
    pub fn current_grant(&self) -> Result<Option<WriterGrant>> {
        Ok(self.locked()?.ownership.grants.last().cloned())
    }
    pub fn command_status(&self, command: &CommandIdentity) -> Result<CommandStatus> {
        self.locked()?.chain.command_status(command).map_err(protocol::session)
    }
    pub fn block(&self, height: u64) -> Result<FinalisedBlockRecord> {
        let inner = self.locked()?;
        inner.store.load_block(self.policy.session_id, height).map_err(protocol::session)
    }
    /// Inspect the audited live checkpoint, including when the corresponding
    /// historical block record deliberately omits its snapshot.
    pub fn inspect_head(&self, port: &dyn myelin_session::InspectPort, query: &[u8]) -> Result<myelin_session::InspectionReceipt> {
        self.locked()?.chain.inspect_head(port, query).map_err(protocol::session)
    }
    pub fn replay_range<R: TransitionExecutor>(
        &self,
        start: u64,
        end: u64,
        restore: impl FnOnce(&[u8]) -> myelin_session::Result<R> + Send + Sync + 'static,
    ) -> Result<myelin_session::RangeReplayReceipt> {
        match self
            .start_replay_range(start, end, None, restore)?
            .advance(myelin_session::RecoveryBudget::default(), &myelin_session::RecoveryCancellation::default())
            .map_err(protocol::session)?
        {
            myelin_session::ReplayOutcome::Complete { receipt, .. } => Ok(*receipt),
            myelin_session::ReplayOutcome::Paused(cursor) => Err(AuthorityError::ReplayPaused(cursor)),
        }
    }
    /// Capture replay configuration under the writer lock, then perform the
    /// read-only audit outside it. A changed canonical head rejects the cursor.
    pub fn start_replay_range<R: TransitionExecutor>(
        &self,
        start: u64,
        end: u64,
        anchor: Option<myelin_session::ReplayAnchor>,
        restore: impl FnOnce(&[u8]) -> myelin_session::Result<R> + Send + Sync + 'static,
    ) -> Result<myelin_session::RangeReplay> {
        Ok(self.locked()?.chain.start_replay_range(start, end, anchor, restore))
    }

    /// An administrator signs the exact current canonical drain/recovery head.
    /// Timeouts, wall clock, discovery leases and routing changes never grant it.
    pub fn grant(&self, request: GrantRequest) -> Result<WriterGrant> {
        let mut inner = self.locked()?;
        if request.policy != self.policy.commitment() {
            return Err(AuthorityError::Authorization("wrong fencing policy".into()));
        }
        protocol::verify(self.policy.administrator, request.digest()?, &request.signature)?;
        if let Some(previous) = inner.ownership.grants.last() {
            if previous.request == request {
                return Ok(previous.clone());
            }
        }
        let epoch = inner.ownership.grants.last().map_or(0, |g| g.epoch);
        if request.expected_epoch != epoch || request.anchor()? != *inner.chain.head() {
            return Err(AuthorityError::Fenced("grant epoch or exact recovery head changed".into()));
        }
        if request.owner == self.policy.administrator
            || request.owner == self.policy.certificate_key
            || request.owner == inner.keys.finality.public_key()
        {
            return Err(AuthorityError::Invalid("worker credentials cannot equal a privileged authority role".into()));
        }
        secp256k1::XOnlyPublicKey::from_slice(&request.owner).map_err(|e| AuthorityError::Invalid(e.to_string()))?;
        if inner.ownership.grants.len() >= MAX_EPOCHS {
            return Err(AuthorityError::Invalid("epoch retention bound reached; explicit archival protocol required".into()));
        }
        let grant =
            WriterGrant { epoch: epoch.checked_add(1).ok_or_else(|| AuthorityError::Invalid("epoch overflow".into()))?, request };
        let mut ownership = inner.ownership.clone();
        ownership.grants.push(grant.clone());
        failpoint("before-grant");
        if let Err(error) = persist(&inner.db, &ownership) {
            inner.uncertain = true;
            return Err(AuthorityError::Uncertain(error.to_string()));
        }
        failpoint("after-grant");
        inner.ownership = ownership;
        Ok(grant)
    }

    /// Execute independently at the canonical head. The caller supplies the
    /// authority clock; request.local_now_ms from the worker is overwritten.
    /// No proof/certificate escapes before synchronous canonical commit.
    pub fn commit(
        &self,
        authorization: CommitAuthorization,
        mut request: BlockRequest,
        authority_now_ms: u64,
    ) -> Result<CanonicalReceipt> {
        let mut inner = self.locked()?;
        let grant =
            inner.ownership.grants.last().cloned().ok_or_else(|| AuthorityError::Fenced("no writer has been granted".into()))?;
        if authorization.policy != self.policy.commitment()
            || authorization.epoch != grant.epoch
            || authorization.owner != grant.owner()
        {
            return Err(AuthorityError::Fenced("writer epoch/owner has been revoked".into()));
        }
        protocol::verify(grant.owner(), authorization.digest()?, &authorization.signature)?;
        if SessionHead::decode(&authorization.base_head).map_err(protocol::session)? != *inner.chain.head() {
            return Err(AuthorityError::Fenced("worker backup/construction head is stale".into()));
        }
        if request.successor.is_some() {
            return Err(AuthorityError::Invalid("successor changes require a separately authorized compatibility protocol".into()));
        }
        if authorization.request_digest != protocol::request_digest(&request)? {
            return Err(AuthorityError::Authorization("candidate bytes changed".into()));
        }
        let stamp = grant.stamp(inner.chain.head().next_height().map_err(protocol::session)?, inner.chain.head().block_hash)?;
        if request.data_commitments.contains(&stamp) {
            return Err(AuthorityError::Invalid("worker supplied a reserved fencing stamp".into()));
        }
        request.data_commitments.push(stamp);
        request.local_now_ms = authority_now_ms;
        let prepared = inner.chain.prepare_block(request).map_err(protocol::session)?;
        let next_head = SessionHead {
            session_id: self.policy.session_id,
            finalised_height: Some(prepared.block().number),
            block_hash: prepared.block().hash(),
            state_root: prepared.block().state_root_after,
            timestamp_ms: prepared.block().timestamp_ms,
            input_position: prepared.frame().input_end,
            logical_time: prepared.frame().logical_time_end,
            sealed_by_successor: None,
        };
        let mut pending = inner.ownership.clone();
        pending.pending_head = Some(next_head.encode().map_err(protocol::session)?);
        failpoint("before-intent");
        if let Err(error) = persist(&inner.db, &pending) {
            inner.uncertain = true;
            return Err(AuthorityError::Uncertain(error.to_string()));
        }
        inner.ownership = pending;
        failpoint("after-intent");
        let SelectedConsensus::ProofOfAuthority(engine) = inner.verifier.selected_consensus() else {
            return Err(AuthorityError::Invalid("not a PoA deployment".into()));
        };
        let seal = match engine.seal_from_signer(prepared.block().hash(), prepared.block().number, &inner.keys.finality) {
            Ok(seal) => seal,
            Err(error) => {
                inner.uncertain = true;
                return Err(AuthorityError::Uncertain(error.to_string()));
            }
        };
        failpoint("after-sign");
        // Preparation stays session-local. Only the atomic store commit and its
        // ownership watermark serialize with shared-claim observations.
        let ledger = inner.ledger_session.ledger.clone();
        let _commit = match ledger.commit_lock() {
            Ok(guard) => guard,
            Err(error) => {
                inner.uncertain = true;
                return Err(error);
            }
        };
        let head = match inner.chain.commit_finalised(prepared, FinalityProof::ProofOfAuthority(seal)) {
            Ok(head) => head,
            Err(error) => {
                inner.uncertain = true;
                return Err(AuthorityError::Uncertain(error.to_string()));
            }
        };
        failpoint("after-canonical-commit");
        reconcile_pending(&mut inner)?;
        failpoint("after-watermark");
        let result = receipt(&inner, head.finalised_height.ok_or_else(|| AuthorityError::Invalid("commit has no height".into()))?)?;
        failpoint("after-receipt");
        Ok(result)
    }

    /// Read-only retries can recover historical certificates, even after owner
    /// rotation. They cannot authenticate an uncommitted worker-local block.
    pub fn canonical_receipt(&self, height: u64) -> Result<CanonicalReceipt> {
        let inner = self.locked()?;
        receipt(&inner, height)
    }
    pub fn effect_certificate(&self, height: u64, id: Hash32) -> Result<(CanonicalEffect, OutboxMessage)> {
        let inner = self.locked()?;
        let record = inner.store.load_block(self.policy.session_id, height).map_err(protocol::session)?;
        let message = record
            .outbox
            .iter()
            .find(|m| m.id == id)
            .cloned()
            .ok_or_else(|| AuthorityError::Invalid("message is not in the canonical block".into()))?;
        let mut certificate = CanonicalEffect {
            receipt: receipt(&inner, height)?,
            message_id: id,
            message_commitment: protocol::message_commitment(&message)?,
            signature: vec![],
        };
        certificate.signature = inner.keys.certificates.sign(certificate.digest()?);
        Ok((certificate, message))
    }
}

fn receipt<E: TransitionExecutor>(inner: &Inner<E>, height: u64) -> Result<CanonicalReceipt> {
    let record = inner.store.load_block(inner.ownership.policy.session_id, height).map_err(protocol::session)?;
    let grant = grant_for(&inner.ownership, height)?;
    check_stamp(&record, grant)?;
    let mut receipt = CanonicalReceipt {
        policy: inner.ownership.policy.commitment(),
        epoch: grant.epoch,
        owner: grant.owner(),
        height,
        block_hash: record.block.hash(),
        state_root: record.block.state_root_after,
        signature: vec![],
    };
    receipt.signature = inner.keys.certificates.sign(receipt.digest()?);
    Ok(receipt)
}
fn grant_for(ownership: &Ownership, height: u64) -> Result<&WriterGrant> {
    for grant in ownership.grants.iter().rev() {
        if grant.first_height()? <= height {
            return Ok(grant);
        }
    }
    Err(AuthorityError::Invalid("canonical block has no authoritative writer epoch".into()))
}
fn check_stamp(record: &FinalisedBlockRecord, grant: &WriterGrant) -> Result<()> {
    let stamp = grant.stamp(record.block.number, record.block.parent_hash)?;
    if record.request_data_commitments.iter().filter(|c| **c == stamp).count() != 1 || !record.block.data_commitments.contains(&stamp)
    {
        return Err(AuthorityError::Invalid("block does not bind its authoritative fencing epoch".into()));
    }
    Ok(())
}
fn audit<E: TransitionExecutor>(inner: &Inner<E>) -> Result<()> {
    let config = inner.chain.config();
    let mut previous_height = 0;
    for (index, grant) in inner.ownership.grants.iter().enumerate() {
        if grant.epoch != index as u64 + 1
            || grant.request.expected_epoch != index as u64
            || grant.request.policy != inner.ownership.policy.commitment()
        {
            return Err(AuthorityError::Invalid("non-monotonic or foreign ownership history".into()));
        }
        protocol::verify(inner.ownership.policy.administrator, grant.request.digest()?, &grant.request.signature)?;
        let anchor = grant.request.anchor()?;
        let expected = match anchor.finalised_height {
            None => initial_head(config),
            Some(height) => {
                let record = inner.store.load_block(config.session_id, height).map_err(protocol::session)?;
                SessionHead {
                    session_id: config.session_id,
                    finalised_height: Some(height),
                    block_hash: record.block.hash(),
                    state_root: record.block.state_root_after,
                    timestamp_ms: record.block.timestamp_ms,
                    input_position: record.frame.input_end,
                    logical_time: record.frame.logical_time_end,
                    sealed_by_successor: record.successor.map(|s| s.target_config.session_id),
                }
            }
        };
        if anchor != expected || anchor.sealed_by_successor.is_some() || grant.first_height()? < previous_height {
            return Err(AuthorityError::Invalid("grant recovery anchor is not canonical".into()));
        }
        previous_height = grant.first_height()?;
    }
    let mut start = 0;
    loop {
        let page = inner.store.load_chain_page(config.session_id, start, 128).map_err(protocol::session)?;
        if page.is_empty() {
            break;
        }
        for record in &page {
            check_stamp(record, grant_for(&inner.ownership, record.block.number)?)?;
        }
        start = page.last().unwrap().block.number.checked_add(1).ok_or_else(|| AuthorityError::Invalid("height overflow".into()))?;
    }
    Ok(())
}
fn persist(db: &DB, ownership: &Ownership) -> Result<()> {
    let body = serde_json::to_vec(ownership).map_err(|e| AuthorityError::Invalid(e.to_string()))?;
    if body.len() + MAGIC.len() + 32 > MAX_EPOCH_BYTES {
        return Err(AuthorityError::Invalid("ownership record too large".into()));
    }
    let mut bytes = MAGIC.to_vec();
    bytes.extend_from_slice(blake3::hash(&body).as_bytes());
    bytes.extend_from_slice(&body);
    let mut options = WriteOptions::default();
    options.set_sync(true);
    db.put_opt(EPOCH_KEY, bytes, &options).map_err(storage)
}
fn decode(bytes: &[u8]) -> Result<Ownership> {
    if bytes.len() < MAGIC.len() + 32 || bytes.len() > MAX_EPOCH_BYTES || &bytes[..MAGIC.len()] != MAGIC {
        return Err(AuthorityError::Invalid("unsupported ownership format/size".into()));
    }
    let body = &bytes[MAGIC.len() + 32..];
    if &bytes[MAGIC.len()..MAGIC.len() + 32] != blake3::hash(body).as_bytes() {
        return Err(AuthorityError::Invalid("ownership checksum mismatch".into()));
    }
    let value: Ownership = serde_json::from_slice(body).map_err(|e| AuthorityError::Invalid(e.to_string()))?;
    if value.grants.len() > MAX_EPOCHS {
        return Err(AuthorityError::Invalid("ownership epoch limit exceeded".into()));
    }
    Ok(value)
}
fn storage(error: rocksdb::Error) -> AuthorityError {
    AuthorityError::Storage(error.to_string())
}

fn initial_head(config: &SessionConfig) -> SessionHead {
    SessionHead {
        session_id: config.session_id,
        finalised_height: None,
        block_hash: [0; 32],
        state_root: config.initial_state_root,
        timestamp_ms: config.initial_timestamp_ms,
        input_position: config.initial_input_position,
        logical_time: config.initial_logical_time,
        sealed_by_successor: None,
    }
}

fn reconcile_pending<E: TransitionExecutor>(inner: &mut Inner<E>) -> Result<()> {
    let before = SessionHead::decode(&inner.ownership.canonical_head).map_err(protocol::session)?;
    let pending = inner.ownership.pending_head.as_ref().map(|b| SessionHead::decode(b)).transpose().map_err(protocol::session)?;
    let head = inner.chain.head();
    if let Some(after) = &pending {
        if after.session_id != before.session_id
            || after.finalised_height != Some(before.next_height().map_err(protocol::session)?)
            || after.sealed_by_successor.is_some()
            || after.block_hash == [0; 32]
        {
            inner.uncertain = true;
            return Err(AuthorityError::Invalid("pending canonical head is not the exact next step".into()));
        }
    }
    if *head != before && pending.as_ref() != Some(head) {
        inner.uncertain = true;
        return Err(AuthorityError::Invalid("canonical store is behind or diverges from the durable ownership watermark".into()));
    }
    if pending.is_some() {
        // On reopen, both database locks exclude the former authority process.
        // If the block is absent, no certificate escaped; discard only that
        // unsent signing intent. Worker unknown commands still need reconciliation.
        let mut ownership = inner.ownership.clone();
        ownership.canonical_head = head.encode().map_err(protocol::session)?;
        ownership.pending_head = None;
        if let Err(error) = persist(&inner.db, &ownership) {
            inner.uncertain = true;
            return Err(AuthorityError::Uncertain(error.to_string()));
        }
        inner.ownership = ownership;
    }
    Ok(())
}

fn failpoint(name: &str) {
    #[cfg(feature = "test-fixture")]
    if std::env::var("MYELIN_AUTHORITY_CRASH_POINT").as_deref() == Ok(name) {
        std::process::exit(74);
    }
    #[cfg(not(feature = "test-fixture"))]
    let _ = name;
}
