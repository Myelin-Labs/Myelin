//! Genesis-bound source-effect policies and the replayable execution boundary.
//!
//! A policy is part of the trusted application, not a proof that the application
//! is correct. Application policies must verify owner authorization and enforce
//! their source disposition (lock/debit or explicitly permitted copying).

use crate::{FrameInput, HandoffIntent, Hash32, Result, SessionConfig, SessionError, SessionHandoff};
use serde::{Deserialize, Serialize};

/// Exported with genesis so authority statements cannot masquerade as an
/// application-verified economic transition. Historical absence means only an
/// authority statement, never a retroactively verified application effect.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields, rename_all = "kebab-case")]
pub enum ProtocolEffectsPolicy {
    Disabled,
    AuthorityMetadata { policy_hash: Hash32 },
    Application { policy_hash: Hash32 },
}

impl ProtocolEffectsPolicy {
    pub fn validate(&self) -> Result<()> {
        match self {
            Self::Disabled => Ok(()),
            Self::AuthorityMetadata { policy_hash } | Self::Application { policy_hash } if *policy_hash != [0; 32] => Ok(()),
            _ => Err(SessionError::ProtocolEffects("effect policy hash must not be zero".into())),
        }
    }

    pub fn commitment(&self) -> Hash32 {
        let mut h = blake3::Hasher::new();
        h.update(b"myelin:protocol-effects-policy:v1");
        match self {
            Self::Disabled => {
                h.update(&[0]);
            }
            Self::AuthorityMetadata { policy_hash } => {
                h.update(&[1]);
                h.update(policy_hash);
            }
            Self::Application { policy_hash } => {
                h.update(&[2]);
                h.update(policy_hash);
            }
        }
        *h.finalize().as_bytes()
    }
}

/// Exact source requests supplied to both live execution and deterministic
/// replay. The complete successor config is carried in its versioned canonical
/// encoding; its authority, program, time and resource rules are all included.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ProtocolEffectsContext {
    pub source_genesis_commitment: Hash32,
    pub source_session_id: Hash32,
    pub policy: ProtocolEffectsPolicy,
    pub height: u64,
    pub timestamp_ms: u64,
    pub pre_state_root: Hash32,
    pub input_root: Hash32,
    #[serde(deserialize_with = "crate::codec::bounded_vec")]
    pub issued_handoffs: Vec<HandoffIntent>,
    /// Empty means no successor. Otherwise this is `SessionConfig::encode()`.
    #[serde(deserialize_with = "crate::codec::bounded_bytes")]
    pub successor_config: Vec<u8>,
}

impl ProtocolEffectsContext {
    pub(crate) fn new(
        config: &SessionConfig,
        height: u64,
        timestamp_ms: u64,
        pre_state_root: Hash32,
        input: &FrameInput,
        issued: Vec<HandoffIntent>,
        successor: Option<&SessionConfig>,
    ) -> Result<Self> {
        let context = Self {
            source_genesis_commitment: config.commitment(),
            source_session_id: config.session_id,
            policy: config.protocol_effects.clone(),
            height,
            timestamp_ms,
            pre_state_root,
            input_root: input.root(),
            issued_handoffs: issued,
            successor_config: successor.map(SessionConfig::encode).transpose()?.unwrap_or_default(),
        };
        context.validate()?;
        Ok(context)
    }

    pub fn has_source_effects(&self) -> bool {
        !self.issued_handoffs.is_empty() || !self.successor_config.is_empty()
    }

    pub fn successor(&self) -> Result<Option<SessionConfig>> {
        if self.successor_config.is_empty() {
            Ok(None)
        } else {
            SessionConfig::decode(&self.successor_config).map(Some)
        }
    }

    pub fn validate(&self) -> Result<()> {
        self.policy.validate()?;
        if self.source_genesis_commitment == [0; 32] || self.source_session_id == [0; 32] {
            return Err(SessionError::ProtocolEffects("missing source genesis identity".into()));
        }
        if self.has_source_effects() && self.policy == ProtocolEffectsPolicy::Disabled {
            return Err(SessionError::ProtocolEffects("source effects are disabled by genesis".into()));
        }
        let mut seen = std::collections::HashSet::new();
        for intent in &self.issued_handoffs {
            intent.validate()?;
            if intent.expires_at_ms <= self.timestamp_ms || !seen.insert(intent_commitment(intent)?) {
                return Err(SessionError::ProtocolEffects("expired or duplicate source intent".into()));
            }
        }
        self.successor()?;
        Ok(())
    }

    /// Explicit opt-in for fixtures or applications that issue only operator
    /// statements. Economic intake rejects their source policy independently.
    pub fn require_authority_metadata(&self, expected_policy_hash: Hash32) -> std::result::Result<(), String> {
        self.validate().map_err(|e| e.to_string())?;
        if self.policy != (ProtocolEffectsPolicy::AuthorityMetadata { policy_hash: expected_policy_hash }) {
            return Err("executor does not implement the genesis-bound authority metadata policy".into());
        }
        Ok(())
    }

    pub fn commitment(&self) -> Result<Hash32> {
        hash_serialized(b"myelin:protocol-effects-context:v1", self)
    }
}

/// This checks only the declared source trust class. Application intake still
/// verifies the source policy allowlist, payload semantics and disposition.
pub fn validate_handoff_trust(target: &SessionConfig, source: &SessionConfig) -> Result<()> {
    match &target.protocol_effects {
        ProtocolEffectsPolicy::Disabled => Err(SessionError::ProtocolEffects("handoff intake is disabled by genesis".into())),
        ProtocolEffectsPolicy::Application { .. } if !matches!(source.protocol_effects, ProtocolEffectsPolicy::Application { .. }) => {
            Err(SessionError::ProtocolEffects("authority metadata cannot become an application-verified handoff".into()))
        }
        _ => Ok(()),
    }
}

/// Check a handoff's exact source record and genesis linkage, including the
/// source's committed effect context. This is not source finality verification
/// or application re-execution; offline import must supply those separately.
pub fn validate_handoff_source(config: &SessionConfig, record: &crate::FinalisedBlockRecord, handoff: &SessionHandoff) -> Result<()> {
    config.validate()?;
    record.validate_resources(config)?;
    handoff.validate(record.block.timestamp_ms)?;
    if record.block.session_id != config.session_id
        || handoff.source_session_id != config.session_id
        || record.block.number != handoff.source_height
        || record.block.state_root_after != handoff.source_state_root
        || record.block.application_profile_commitment != config.application_profile.commitment()
        || record.block.application_profile_commitment != handoff.source_application_profile_commitment
        || record.consensus_module_commitment != config.consensus_module_commitment
        || record.consensus_module_commitment != handoff.source_consensus_module_commitment
        || !record.issued_handoffs.contains(handoff)
        || !record.block.data_commitments.contains(&handoff.id())
    {
        return Err(SessionError::ProtocolEffects("handoff is not committed by its claimed source genesis/block".into()));
    }
    Ok(())
}

pub(crate) fn intent_from_handoff(h: &SessionHandoff) -> HandoffIntent {
    HandoffIntent {
        target: h.target.clone(),
        expires_at_ms: h.expires_at_ms,
        payload: h.payload.clone(),
        authorization: h.authorization.clone(),
        evidence_requirement: h.evidence_requirement.clone(),
    }
}

fn intent_commitment(intent: &HandoffIntent) -> Result<Hash32> {
    hash_serialized(b"myelin:source-intent:v1", intent)
}

/// Application-defined actor and object identities. The application verifies
/// the signature scheme and ownership, checks the current object version and
/// durably consumes the nonce as part of its state transition.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct EffectAuthorizationScope {
    pub actor: Hash32,
    pub object: Hash32,
    pub object_version: u64,
    pub nonce: Hash32,
}

impl EffectAuthorizationScope {
    fn validate(&self) -> Result<()> {
        if self.actor == [0; 32] || self.object == [0; 32] || self.nonce == [0; 32] {
            return Err(SessionError::ProtocolEffects("authorization actor, object and nonce must be nonzero".into()));
        }
        Ok(())
    }

    /// Complete handoff authorization digest, excluding the signature bytes
    /// themselves. Recipient/amount/components must be canonical payload fields.
    pub fn handoff_digest(&self, source_genesis: Hash32, intent: &HandoffIntent) -> Result<Hash32> {
        self.validate()?;
        if source_genesis == [0; 32] {
            return Err(SessionError::ProtocolEffects("missing authorization source genesis".into()));
        }
        hash_serialized(
            b"myelin:session-effect-handoff-authorization:v1",
            &(source_genesis, self, &intent.target, intent.expires_at_ms, &intent.payload, &intent.evidence_requirement),
        )
    }

    /// Authorization of the entire successor configuration, including its
    /// program, finality module and keys, resource envelope and time policies.
    pub fn successor_digest(&self, source_genesis: Hash32, target: &SessionConfig) -> Result<Hash32> {
        self.validate()?;
        target.validate()?;
        if source_genesis == [0; 32] {
            return Err(SessionError::ProtocolEffects("missing authorization source genesis".into()));
        }
        hash_serialized(b"myelin:session-effect-successor-authorization:v1", &(source_genesis, self, target.commitment()))
    }
}

fn hash_serialized<T: Serialize>(domain: &[u8], value: &T) -> Result<Hash32> {
    crate::codec::size_with_limit(value, "effect_context_bytes", crate::codec::MAX_RECORD_BYTES as u64)?;
    let bytes = postcard::to_allocvec(value).map_err(|e| SessionError::ProtocolEffects(e.to_string()))?;
    let mut h = blake3::Hasher::new();
    h.update(domain);
    h.update(&bytes);
    Ok(*h.finalize().as_bytes())
}
