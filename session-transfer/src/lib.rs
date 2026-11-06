//! A deterministic reference application with explicit scarce-transfer and
//! copyable-receipt policies. It requires owner signatures and source disposition;
//! it does not implement a bridge, remote uniqueness, custody or L1 adjudication.
mod codec;
mod state;
pub use state::{AssetAccount, PendingLock, Receipt, TransferExecutor, TransferState};

use myelin_session::{
    bounded_codec, CommandIdentity, EffectAuthorizationScope, EvidenceRequirement, FrameInput, HandoffIntent, HandoffTarget, Hash32,
    ProtocolEffectsPolicy, SessionConfig, SessionStore,
};
use secp256k1::{schnorr::Signature, Keypair, Message, Secp256k1, SecretKey, XOnlyPublicKey};
use serde::{Deserialize, Serialize};

pub const MAX_COMPONENT_BYTES: usize = 65_536;
pub const MAX_TRANSFER_BYTES: usize = 128 * 1024;
const MAX_COMMAND_BYTES: usize = 16 * 1024 * 1024;

pub type Result<T> = std::result::Result<T, Error>;
#[derive(Debug, thiserror::Error)]
pub enum Error {
    #[error("transfer codec: {0}")]
    Codec(String),
    #[error("transfer policy: {0}")]
    Policy(String),
    #[error("transfer authorization: {0}")]
    Authorization(String),
    #[error("transfer state: {0}")]
    State(String),
    #[error("transfer bound exceeded: {0}")]
    Bounds(&'static str),
    #[error("unsupported transfer operation: {0}")]
    Unsupported(&'static str),
    #[error("source lookup: {0}")]
    Source(String),
}

/// Distinct policies and intake schemas: copying never creates a scarce balance.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum TransferKind {
    Scarce,
    CopyableReceipt,
}
impl TransferKind {
    pub fn intake_schema(self) -> Hash32 {
        *blake3::hash(match self {
            Self::Scarce => b"myelin:reference-scarce-payload:v1",
            Self::CopyableReceipt => b"myelin:reference-copyable-payload:v1",
        })
        .as_bytes()
    }
}

/// Namespace is deployment/application-owned, not a ticker or display name.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct AssetId {
    pub namespace: Hash32,
    pub object: Hash32,
}
impl AssetId {
    pub fn commitment(&self) -> Hash32 {
        let mut h = blake3::Hasher::new();
        h.update(b"myelin:reference-transfer-object:v1");
        h.update(&self.namespace);
        h.update(&self.object);
        *h.finalize().as_bytes()
    }
    pub fn validate(&self) -> Result<()> {
        if self.namespace == [0; 32] || self.object == [0; 32] {
            Err(Error::Policy("zero asset namespace/object".into()))
        } else {
            Ok(())
        }
    }
}

/// Exact source anchor, retained in the target's genesis-bound policy.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct SourceIdentity {
    pub session_id: Hash32,
    pub genesis_commitment: Hash32,
    pub application_profile_commitment: Hash32,
    pub consensus_module_commitment: Hash32,
    pub effects_policy_hash: Hash32,
}
impl SourceIdentity {
    pub fn from_config(config: &SessionConfig) -> Result<Self> {
        config.validate().map_err(|e| Error::Policy(e.to_string()))?;
        let ProtocolEffectsPolicy::Application { policy_hash } = config.protocol_effects else {
            return Err(Error::Policy("reference transfer requires an application-verified policy".into()));
        };
        Ok(Self {
            session_id: config.session_id,
            genesis_commitment: config.commitment(),
            application_profile_commitment: config.application_profile.commitment(),
            consensus_module_commitment: config.consensus_module_commitment,
            effects_policy_hash: policy_hash,
        })
    }
    fn validate(&self) -> Result<()> {
        if [
            self.session_id,
            self.genesis_commitment,
            self.application_profile_commitment,
            self.consensus_module_commitment,
            self.effects_policy_hash,
        ]
        .contains(&[0; 32])
        {
            Err(Error::Policy("zero source trust anchor".into()))
        } else {
            Ok(())
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub enum TransferTarget {
    Session { session_id: Hash32, genesis_commitment: Hash32 },
    SharedStorePolicy { policy_commitment: Hash32 },
}
impl TransferTarget {
    pub fn outer(&self) -> HandoffTarget {
        match self {
            Self::Session { session_id, .. } => HandoffTarget::Session(*session_id),
            Self::SharedStorePolicy { policy_commitment } => HandoffTarget::Policy(*policy_commitment),
        }
    }
    fn validate(&self) -> Result<()> {
        match self {
            Self::Session { session_id, genesis_commitment } if *session_id != [0; 32] && *genesis_commitment != [0; 32] => Ok(()),
            Self::SharedStorePolicy { policy_commitment } if *policy_commitment != [0; 32] => Ok(()),
            _ => Err(Error::Policy("zero target binding".into())),
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct TransferPayload {
    pub deployment_id: Hash32,
    /// Operator-selected identity of one shared uniqueness store. Merely using
    /// the same value in independent stores does not make their commits atomic.
    pub uniqueness_domain: Hash32,
    pub kind: TransferKind,
    pub intake_schema: Hash32,
    pub source: SourceIdentity,
    pub asset: AssetId,
    pub owner: Hash32,
    pub recipient: Hash32,
    pub source_version: u64,
    pub amount: u64,
    #[serde(deserialize_with = "bounded_codec::bounded_bytes")]
    pub components: Vec<u8>,
    pub nonce: Hash32,
    pub target: TransferTarget,
}
impl TransferPayload {
    pub fn validate(&self) -> Result<()> {
        self.source.validate()?;
        self.asset.validate()?;
        self.target.validate()?;
        if [self.deployment_id, self.uniqueness_domain, self.owner, self.recipient, self.nonce].contains(&[0; 32])
            || self.amount == 0
            || self.intake_schema != self.kind.intake_schema()
        {
            return Err(Error::Policy("invalid transfer identity, amount or intake schema".into()));
        }
        if self.kind == TransferKind::CopyableReceipt && self.amount != 1 {
            return Err(Error::Policy("copyable receipts require amount one".into()));
        }
        XOnlyPublicKey::from_slice(&self.owner).map_err(|_| Error::Authorization("owner is not a Schnorr public key".into()))?;
        XOnlyPublicKey::from_slice(&self.recipient)
            .map_err(|_| Error::Authorization("recipient is not a Schnorr public key".into()))?;
        if self.components.len() > MAX_COMPONENT_BYTES {
            return Err(Error::Bounds("components"));
        }
        Ok(())
    }
    pub fn encode(&self) -> Result<Vec<u8>> {
        self.validate()?;
        codec::encode(b"MTRP", self, MAX_TRANSFER_BYTES)
    }
    pub fn decode(bytes: &[u8]) -> Result<Self> {
        let p: Self = codec::decode(b"MTRP", bytes, MAX_TRANSFER_BYTES)?;
        p.validate()?;
        Ok(p)
    }
    pub fn scope(&self) -> EffectAuthorizationScope {
        EffectAuthorizationScope {
            actor: self.owner,
            object: self.asset.commitment(),
            object_version: self.source_version,
            nonce: self.nonce,
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct Authorization {
    scope: EffectAuthorizationScope,
    #[serde(deserialize_with = "bounded_codec::bounded_bytes")]
    signature: Vec<u8>,
}

/// Signing is distinct from wallet login; this key authorizes the complete
/// versioned transfer intent. Callers own key storage and signing UX.
pub struct TransferSigner {
    key: SecretKey,
}
impl TransferSigner {
    pub fn new(secret: [u8; 32]) -> Result<Self> {
        Ok(Self { key: SecretKey::from_slice(&secret).map_err(|e| Error::Authorization(e.to_string()))? })
    }
    pub fn actor(&self) -> Hash32 {
        Keypair::from_secret_key(&Secp256k1::new(), &self.key).x_only_public_key().0.serialize()
    }
    /// Cancel one exact earlier transfer. No platform login or validator key
    /// substitutes for this owner authorization.
    pub fn cancel(&self, config: &SessionConfig, handoff: &myelin_session::SessionHandoff) -> Result<TransferCommand> {
        let (id, payload) = verify_handoff(handoff)?;
        let current = SourceIdentity::from_config(config)?;
        if payload.owner != self.actor()
            || current.application_profile_commitment != payload.source.application_profile_commitment
            || current.effects_policy_hash != payload.source.effects_policy_hash
        {
            return Err(Error::Authorization("cancellation signer or application policy mismatch".into()));
        }
        let digest = cancellation_digest(config.commitment(), id, handoff.id(), payload.owner)?;
        let key = Keypair::from_secret_key(&Secp256k1::new(), &self.key);
        let authorization = Secp256k1::new().sign_schnorr_no_aux_rand(&Message::from_digest(digest), &key).serialize().to_vec();
        Ok(TransferCommand::Cancel { handoff_id: handoff.id(), authorization })
    }
    /// Authorize the complete target genesis at one exact source state. Every
    /// current account owner and receipt recipient must approve continuation.
    pub fn continue_session(
        &self,
        source: &SessionConfig,
        state: &TransferState,
        target: &SessionConfig,
    ) -> Result<ContinuationApproval> {
        source.validate().map_err(|e| Error::Policy(e.to_string()))?;
        target.validate().map_err(|e| Error::Policy(e.to_string()))?;
        if state.owners().binary_search(&self.actor()).is_err() {
            return Err(Error::Authorization("continuation signer has no rights in this state".into()));
        }
        let digest = continuation_digest(source.commitment(), state.root()?, target.commitment())?;
        let key = Keypair::from_secret_key(&Secp256k1::new(), &self.key);
        Ok(ContinuationApproval {
            owner: self.actor(),
            signature: Secp256k1::new().sign_schnorr_no_aux_rand(&Message::from_digest(digest), &key).serialize().to_vec(),
        })
    }
    pub fn sign(
        &self,
        payload: TransferPayload,
        expires_at_ms: u64,
        evidence_requirement: Option<EvidenceRequirement>,
    ) -> Result<HandoffIntent> {
        payload.validate()?;
        if self.actor() != payload.owner {
            return Err(Error::Authorization("signer is not the stated owner".into()));
        }
        let mut intent = HandoffIntent {
            target: payload.target.outer(),
            expires_at_ms,
            payload: payload.encode()?,
            authorization: vec![],
            evidence_requirement,
        };
        let scope = payload.scope();
        let digest =
            scope.handoff_digest(payload.source.genesis_commitment, &intent).map_err(|e| Error::Authorization(e.to_string()))?;
        let key = Keypair::from_secret_key(&Secp256k1::new(), &self.key);
        let signature = Secp256k1::new().sign_schnorr_no_aux_rand(&Message::from_digest(digest), &key).serialize().to_vec();
        intent.authorization = codec::encode(b"MTRA", &Authorization { scope, signature }, 512)?;
        intent.validate().map_err(|e| Error::Policy(e.to_string()))?;
        Ok(intent)
    }
}

/// Signature-independent identity: an alternative signature cannot produce a
/// new transferable right or defeat the source nonce / target nullifier checks.
pub fn verify_intent(intent: &HandoffIntent) -> Result<(Hash32, TransferPayload)> {
    intent.validate().map_err(|e| Error::Policy(e.to_string()))?;
    let payload = TransferPayload::decode(&intent.payload)?;
    if payload.target.outer() != intent.target {
        return Err(Error::Authorization("inner/outer target mismatch".into()));
    }
    let auth: Authorization = codec::decode(b"MTRA", &intent.authorization, 512)?;
    if auth.scope != payload.scope() {
        return Err(Error::Authorization("authorization scope does not match payload".into()));
    }
    let digest =
        auth.scope.handoff_digest(payload.source.genesis_commitment, intent).map_err(|e| Error::Authorization(e.to_string()))?;
    let key = XOnlyPublicKey::from_slice(&payload.owner).map_err(|e| Error::Authorization(e.to_string()))?;
    let signature = Signature::from_slice(&auth.signature).map_err(|e| Error::Authorization(e.to_string()))?;
    Secp256k1::verification_only()
        .verify_schnorr(&signature, &Message::from_digest(digest), &key)
        .map_err(|_| Error::Authorization("owner signature is invalid".into()))?;
    Ok((digest, payload))
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct TransferLimits {
    pub max_accounts: u32,
    pub max_pending: u32,
    pub max_receipts: u32,
    pub max_nonces: u32,
    pub max_components_bytes: u32,
}
impl Default for TransferLimits {
    fn default() -> Self {
        Self {
            max_accounts: 4096,
            max_pending: 4096,
            max_receipts: 16384,
            max_nonces: 32768,
            max_components_bytes: MAX_COMPONENT_BYTES as u32,
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct TransferPolicy {
    pub deployment_id: Hash32,
    pub uniqueness_domain: Hash32,
    pub kind: TransferKind,
    pub limits: TransferLimits,
    #[serde(deserialize_with = "bounded_codec::bounded_vec")]
    pub accepted_sources: Vec<SourceIdentity>,
}
impl TransferPolicy {
    pub fn validate(&self) -> Result<()> {
        if self.deployment_id == [0; 32] || self.uniqueness_domain == [0; 32] {
            return Err(Error::Policy("zero deployment/uniqueness domain".into()));
        }
        for value in [self.limits.max_accounts, self.limits.max_pending, self.limits.max_receipts, self.limits.max_nonces] {
            if value == 0 || value > bounded_codec::MAX_SEQUENCE_ITEMS as u32 {
                return Err(Error::Bounds("policy count ceiling"));
            }
        }
        if self.limits.max_components_bytes as usize > MAX_COMPONENT_BYTES || self.accepted_sources.len() > 1024 {
            return Err(Error::Bounds("policy bytes/sources"));
        }
        for source in &self.accepted_sources {
            source.validate()?;
        }
        if self.accepted_sources.windows(2).any(|w| w[0].session_id >= w[1].session_id) {
            return Err(Error::Policy("source allowlist must be sorted and unique by session".into()));
        }
        Ok(())
    }
    pub fn commitment(&self) -> Result<Hash32> {
        self.validate()?;
        codec::hash(b"myelin:reference-transfer-policy:v1", self)
    }
    pub fn effect_policy(&self) -> Result<ProtocolEffectsPolicy> {
        Ok(ProtocolEffectsPolicy::Application { policy_hash: self.commitment()? })
    }
    pub fn intake_policy(&self) -> Result<Hash32> {
        codec::hash(b"myelin:reference-transfer-intake:v1", &self.commitment()?)
    }
    pub fn bind_config(&self, config: &mut SessionConfig) -> Result<()> {
        config.protocol_effects = self.effect_policy()?;
        config.application_profile.command_identity_required = true;
        config.application_profile.input_schema_hash = input_schema();
        config.application_profile.state_codec_hash = state_codec();
        config.application_profile.handoff_policy_hash = self.intake_policy()?;
        Ok(())
    }
    pub(crate) fn matches_payload(&self, p: &TransferPayload) -> Result<()> {
        if self.kind != p.kind
            || self.deployment_id != p.deployment_id
            || self.uniqueness_domain != p.uniqueness_domain
            || p.components.len() > self.limits.max_components_bytes as usize
        {
            return Err(Error::Policy("transfer kind, deployment, store domain or components do not match policy".into()));
        }
        Ok(())
    }
}

pub fn input_schema() -> Hash32 {
    *blake3::hash(b"myelin:reference-transfer-commands:v3").as_bytes()
}
pub fn state_codec() -> Hash32 {
    *blake3::hash(b"myelin:reference-transfer-state:v1").as_bytes()
}

impl myelin_session::HandoffRetentionPolicy for TransferPolicy {
    fn is_copyable(&self, config: &SessionConfig, handoff: &myelin_session::SessionHandoff) -> myelin_session::Result<bool> {
        let classify = || -> Result<bool> {
            if config.protocol_effects != self.effect_policy()?
                || config.application_profile.handoff_policy_hash != self.intake_policy()?
            {
                return Err(Error::Policy("retention policy differs from source genesis".into()));
            }
            let (_, payload) = verify_handoff(handoff)?;
            self.matches_payload(&payload)?;
            if payload.source != SourceIdentity::from_config(config)? || handoff.source_session_id != config.session_id {
                return Err(Error::Policy("retention handoff differs from source genesis".into()));
            }
            Ok(self.kind == TransferKind::CopyableReceipt)
        };
        classify().map_err(|e| myelin_session::SessionError::ProtocolEffects(e.to_string()))
    }
}

/// A local capability proving the allowlisted genesis records match this
/// store. It does not prove source application replay or remote finality.
#[derive(Clone)]
pub struct SourceBindings {
    policy_commitment: Hash32,
    inheritance: Option<(Hash32, std::collections::BTreeMap<Hash32, usize>)>,
}
impl SourceBindings {
    /// Leaf source applications have no intake dependencies to resolve.
    pub fn no_sources(policy: &TransferPolicy) -> Result<Self> {
        policy.validate()?;
        if !policy.accepted_sources.is_empty() {
            return Err(Error::Source("source dependencies require store/archived-genesis validation".into()));
        }
        Ok(Self { policy_commitment: policy.commitment()?, inheritance: None })
    }
    pub fn from_store<S: SessionStore>(policy: &TransferPolicy, store: &S) -> Result<Self> {
        Self::from_genesis(policy, |id| store.load_genesis(id).map_err(|e| Error::Source(e.to_string())))
    }
    /// Resolve immutable source records through a read-only authority/catalog
    /// without exposing the canonical store's mutation capability.
    pub fn from_genesis(
        policy: &TransferPolicy,
        mut genesis: impl FnMut(Hash32) -> Result<myelin_session::SessionGenesis>,
    ) -> Result<Self> {
        policy.validate()?;
        for anchor in &policy.accepted_sources {
            let actual = genesis(anchor.session_id)?;
            if SourceIdentity::from_config(&actual.config)? != *anchor {
                return Err(Error::Source("source genesis differs from pinned allowlist".into()));
            }
        }
        Ok(Self { policy_commitment: policy.commitment()?, inheritance: None })
    }
    fn for_state<S: SessionStore>(config: &SessionConfig, state: &TransferState, store: &S) -> Result<Self> {
        let mut bindings = Self::from_store(state.policy(), store)?;
        let mut depths = std::collections::BTreeMap::new();
        for lock in state.pending() {
            if lock.source.session_id == config.session_id {
                if lock.source != SourceIdentity::from_config(config)? {
                    return Err(Error::Source("lock differs from its current genesis".into()));
                }
                continue;
            }
            let origin = store.load_genesis(lock.source.session_id).map_err(|e| Error::Source(e.to_string()))?;
            if SourceIdentity::from_config(&origin.config)? != lock.source {
                return Err(Error::Source("inherited lock differs from its original genesis".into()));
            }
            if depths.contains_key(&lock.source.session_id) {
                continue;
            }
            let lineage = myelin_session::continuation_lineage(store, config, lock.source.session_id)
                .map_err(|e| Error::Source(e.to_string()))?;
            depths.insert(lock.source.session_id, lineage.len());
        }
        bindings.inheritance = Some((config.commitment(), depths));
        Ok(bindings)
    }
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ContinuationApproval {
    pub owner: Hash32,
    #[serde(deserialize_with = "bounded_codec::bounded_bytes")]
    pub signature: Vec<u8>,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub enum TransferCommand {
    Issue {
        transfer_id: Hash32,
    },
    Intake {
        handoff_id: Hash32,
    },
    Cancel {
        handoff_id: Hash32,
        #[serde(deserialize_with = "bounded_codec::bounded_bytes")]
        authorization: Vec<u8>,
    },
    /// Once owner-authorized cancellation is durable, anyone may trigger the
    /// exact return to the original owner's account. No beneficiary is supplied.
    Reclaim {
        handoff_id: Hash32,
    },
    /// Remove an already consumed source lock without crediting any account.
    /// The protocol action separately binds the exact target consumption block.
    Retire {
        handoff_id: Hash32,
    },
    Continue {
        target_genesis: Hash32,
        #[serde(deserialize_with = "bounded_codec::bounded_vec")]
        approvals: Vec<ContinuationApproval>,
    },
}
impl TransferCommand {
    pub fn identity(&self, session_genesis: Hash32) -> Result<CommandIdentity> {
        Ok(CommandIdentity {
            id: match self {
                Self::Cancel { handoff_id, .. } => {
                    codec::hash(b"myelin:reference-transfer-command-id:v2", &(session_genesis, 2u8, handoff_id))?
                }
                _ => codec::hash(b"myelin:reference-transfer-command-id:v2", &(session_genesis, self))?,
            },
            payload_hash: codec::hash(b"myelin:reference-transfer-command-payload:v2", self)?,
        })
    }
}
#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct Commands {
    #[serde(deserialize_with = "bounded_codec::bounded_vec")]
    commands: Vec<TransferCommand>,
}

pub fn command_frame(config: &SessionConfig, position: u64, logical_time: u64, commands: Vec<TransferCommand>) -> Result<FrameInput> {
    if config.application_profile.input_schema_hash != input_schema() {
        return Err(Error::Policy("unsupported transfer command schema".into()));
    }
    let count = commands.len() as u64;
    let frame = FrameInput {
        commands: commands.iter().map(|c| c.identity(config.commitment())).collect::<Result<_>>()?,
        start_position: position,
        end_position: position.checked_add(count).ok_or(Error::Bounds("input position"))?,
        logical_time_start: logical_time,
        logical_time_end: logical_time.checked_add(count).ok_or(Error::Bounds("logical time"))?,
        payload: codec::encode(b"MTRD", &Commands { commands }, MAX_COMMAND_BYTES)?,
    };
    frame.validate(&config.application_profile).map_err(|e| Error::Policy(e.to_string()))?;
    Ok(frame)
}

/// Construct the executor and session against the same selected store so the
/// genesis allowlist cannot accidentally be checked against another backend.
pub fn create_chain<S: SessionStore>(
    config: SessionConfig,
    state: TransferState,
    verifier: std::sync::Arc<dyn myelin_session::FinalityVerifier>,
    store: std::sync::Arc<S>,
) -> Result<myelin_session::SessionChain<TransferExecutor, S>> {
    let sources = SourceBindings::for_state(&config, &state, store.as_ref())?;
    let executor = TransferExecutor::new(&config, state, sources)?;
    myelin_session::SessionChain::create(config, verifier, executor, store).map_err(|e| Error::State(e.to_string()))
}

/// Bind the original policy/allowlist again before accepting a restart snapshot.
pub fn restore_executor<S: SessionStore>(config: &SessionConfig, snapshot: &[u8], store: &S) -> Result<TransferExecutor> {
    let state = TransferState::decode(snapshot)?;
    let sources = SourceBindings::for_state(config, &state, store)?;
    TransferExecutor::restore(config, snapshot, sources)
}

fn verify_handoff(handoff: &myelin_session::SessionHandoff) -> Result<(Hash32, TransferPayload)> {
    verify_intent(&HandoffIntent {
        target: handoff.target.clone(),
        expires_at_ms: handoff.expires_at_ms,
        payload: handoff.payload.clone(),
        authorization: handoff.authorization.clone(),
        evidence_requirement: handoff.evidence_requirement.clone(),
    })
}
fn cancellation_digest(genesis: Hash32, transfer_id: Hash32, handoff_id: Hash32, owner: Hash32) -> Result<Hash32> {
    codec::hash(b"myelin:reference-transfer-cancel:v1", &(genesis, transfer_id, handoff_id, owner))
}

fn continuation_digest(source: Hash32, state: Hash32, target: Hash32) -> Result<Hash32> {
    codec::hash(b"myelin:reference-transfer-continuation:v1", &(source, state, target))
}
