// SPDX-License-Identifier: MIT
// Copyright (C) 2026 Myelin developers

//! Explicit session succession and source-committed, single-use handoffs.

use crate::{decode_record, encode_record, Hash32, Result, SessionConfig, SessionError};
use serde::{Deserialize, Serialize};

const HANDOFF_DOMAIN: &[u8] = b"myelin:session-handoff";
const HANDOFF_CONSUMPTION_DOMAIN: &[u8] = b"myelin:session-handoff-consumption";
const MAX_HANDOFF_PAYLOAD_BYTES: usize = 16 * 1024 * 1024;
const MAX_HANDOFF_AUTHORIZATION_BYTES: usize = 64 * 1024;

/// Reverse lineage embedded in a successor's immutable genesis config.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct PredecessorReference {
    pub session_id: Hash32,
    pub final_height: u64,
    pub final_state_root: Hash32,
    pub application_profile_commitment: Hash32,
}

/// Final declaration that seals one session and atomically creates its sole
/// successor from the exact post-state snapshot.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct SuccessorDeclaration {
    pub source_session_id: Hash32,
    pub source_height: u64,
    pub source_state_root: Hash32,
    pub source_application_profile_commitment: Hash32,
    pub target_config: SessionConfig,
    /// Source-authorized continuation or state conversion receipt.
    pub transition: crate::SuccessorTransition,
}

impl SuccessorDeclaration {
    pub fn validate(&self) -> Result<()> {
        self.target_config.validate()?;
        if self.source_session_id == [0; 32]
            || self.source_session_id == self.target_config.session_id
            || self.source_state_root == [0; 32]
            || self.source_application_profile_commitment == [0; 32]
            || self.target_config.predecessor
                != Some(PredecessorReference {
                    session_id: self.source_session_id,
                    final_height: self.source_height,
                    final_state_root: self.source_state_root,
                    application_profile_commitment: self.source_application_profile_commitment,
                })
        {
            return Err(SessionError::Continuity("invalid successor source/target identity or state root".to_owned()));
        }
        self.transition.validate_shape(self.source_state_root, self.target_config.initial_state_root)?;
        Ok(())
    }

    pub fn commitment(&self) -> Hash32 {
        successor_commitment(
            self.source_session_id,
            self.source_height,
            self.source_state_root,
            self.source_application_profile_commitment,
            self.target_config.commitment(),
            &self.transition,
        )
    }
}

pub(crate) fn successor_commitment(
    source_session_id: Hash32,
    source_height: u64,
    source_state_root: Hash32,
    source_profile: Hash32,
    target_config: Hash32,
    transition: &crate::SuccessorTransition,
) -> Hash32 {
    let mut hasher = blake3::Hasher::new();
    hasher.update(b"myelin:successor-declaration-v2");
    hasher.update(&source_session_id);
    hasher.update(&source_height.to_le_bytes());
    hasher.update(&source_state_root);
    hasher.update(&source_profile);
    hasher.update(&target_config);
    transition.update_commitment(&mut hasher);
    *hasher.finalize().as_bytes()
}

/// Exact target session or any session whose genesis-bound intake policy
/// matches the supplied commitment.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields, rename_all = "kebab-case")]
pub enum HandoffTarget {
    Session(Hash32),
    Policy(Hash32),
}

/// Minimum locally verified evidence stage required before target inclusion.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct EvidenceRequirement {
    pub message_id: Hash32,
    pub pipeline_commitment: Hash32,
    /// Zero-based required stage index.
    pub minimum_stage_index: u16,
}

/// Application request to commit a transferable payload in the source block.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct HandoffIntent {
    pub target: HandoffTarget,
    pub expires_at_ms: u64,
    #[serde(deserialize_with = "crate::codec::bounded_bytes")]
    pub payload: Vec<u8>,
    /// Application-interpreted authorization. New source execution must validate
    /// it under its genesis-bound protocol-effects policy; target intake applies
    /// its own allowlist and semantics. Nonempty bytes alone confer no rights.
    #[serde(deserialize_with = "crate::codec::bounded_bytes")]
    pub authorization: Vec<u8>,
    pub evidence_requirement: Option<EvidenceRequirement>,
}

impl HandoffIntent {
    pub fn validate(&self) -> Result<()> {
        validate_handoff_fields(
            &self.target,
            self.expires_at_ms,
            self.payload.len(),
            self.authorization.len(),
            self.evidence_requirement.as_ref(),
        )?;
        Ok(())
    }
}

/// Source-derived handoff committed by one finalised block.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct SessionHandoff {
    pub source_session_id: Hash32,
    pub source_height: u64,
    pub source_state_root: Hash32,
    pub source_application_profile_commitment: Hash32,
    pub source_consensus_module_commitment: Hash32,
    pub target: HandoffTarget,
    pub expires_at_ms: u64,
    #[serde(deserialize_with = "crate::codec::bounded_bytes")]
    pub payload: Vec<u8>,
    #[serde(deserialize_with = "crate::codec::bounded_bytes")]
    pub authorization: Vec<u8>,
    pub evidence_requirement: Option<EvidenceRequirement>,
}

impl SessionHandoff {
    pub(crate) fn from_intent(
        source_session_id: Hash32,
        source_height: u64,
        source_state_root: Hash32,
        source_application_profile_commitment: Hash32,
        source_consensus_module_commitment: Hash32,
        source_timestamp_ms: u64,
        intent: HandoffIntent,
    ) -> Result<Self> {
        intent.validate()?;
        if intent.expires_at_ms <= source_timestamp_ms {
            return Err(SessionError::Continuity("handoff must expire after its source block timestamp".to_owned()));
        }
        Ok(Self {
            source_session_id,
            source_height,
            source_state_root,
            source_application_profile_commitment,
            source_consensus_module_commitment,
            target: intent.target,
            expires_at_ms: intent.expires_at_ms,
            payload: intent.payload,
            authorization: intent.authorization,
            evidence_requirement: intent.evidence_requirement,
        })
    }

    pub fn id(&self) -> Hash32 {
        let mut hasher = blake3::Hasher::new();
        hasher.update(HANDOFF_DOMAIN);
        hasher.update(&self.source_session_id);
        hasher.update(&self.source_height.to_le_bytes());
        hasher.update(&self.source_state_root);
        hasher.update(&self.source_application_profile_commitment);
        hasher.update(&self.source_consensus_module_commitment);
        match self.target {
            HandoffTarget::Session(id) => {
                hasher.update(&[0]);
                hasher.update(&id);
            }
            HandoffTarget::Policy(id) => {
                hasher.update(&[1]);
                hasher.update(&id);
            }
        }
        hasher.update(&self.expires_at_ms.to_le_bytes());
        hasher.update(&(self.payload.len() as u64).to_le_bytes());
        hasher.update(&self.payload);
        hasher.update(&(self.authorization.len() as u64).to_le_bytes());
        hasher.update(&self.authorization);
        match &self.evidence_requirement {
            Some(requirement) => {
                hasher.update(&[1]);
                hasher.update(&requirement.message_id);
                hasher.update(&requirement.pipeline_commitment);
                hasher.update(&requirement.minimum_stage_index.to_le_bytes());
            }
            None => {
                hasher.update(&[0]);
            }
        }
        *hasher.finalize().as_bytes()
    }

    pub fn validate(&self, source_timestamp_ms: u64) -> Result<()> {
        validate_handoff_fields(
            &self.target,
            self.expires_at_ms,
            self.payload.len(),
            self.authorization.len(),
            self.evidence_requirement.as_ref(),
        )?;
        if self.source_session_id == [0; 32]
            || self.source_state_root == [0; 32]
            || self.source_application_profile_commitment == [0; 32]
            || self.source_consensus_module_commitment == [0; 32]
            || self.expires_at_ms <= source_timestamp_ms
        {
            return Err(SessionError::Continuity("handoff source identity or expiry is invalid".to_owned()));
        }
        Ok(())
    }

    pub fn accepts_target(&self, target_config: &SessionConfig) -> bool {
        match self.target {
            HandoffTarget::Session(session_id) => session_id == target_config.session_id,
            HandoffTarget::Policy(policy) => policy == target_config.application_profile.handoff_policy_hash,
        }
    }

    pub fn consumption_commitment(session_id: Hash32, height: u64, handoff_id: Hash32) -> Hash32 {
        handoff_consumption_commitment(session_id, height, handoff_id)
    }
}

/// Durable one-time-consumption state for a handoff.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct StoredHandoff {
    pub handoff: SessionHandoff,
    pub lifecycle: crate::HandoffLifecycle,
}

impl StoredHandoff {
    pub fn is_consumed(&self) -> bool {
        self.consumption().is_some()
    }

    pub fn consumption(&self) -> Option<&crate::HandoffBlockReference> {
        match &self.lifecycle.terminal {
            crate::HandoffTerminal::Consumed { at } | crate::HandoffTerminal::Retired { consumed_at: at, .. } => Some(at),
            _ => None,
        }
    }

    pub fn encode(&self) -> Result<Vec<u8>> {
        self.validate_lifecycle()?;
        encode_record(self)
    }
    pub fn decode(bytes: &[u8]) -> Result<Self> {
        let value: Self = decode_record(bytes)?;
        value.validate_lifecycle()?;
        Ok(value)
    }
}

pub(crate) fn handoff_consumption_commitment(session_id: Hash32, height: u64, handoff_id: Hash32) -> Hash32 {
    let mut hasher = blake3::Hasher::new();
    hasher.update(HANDOFF_CONSUMPTION_DOMAIN);
    hasher.update(&session_id);
    hasher.update(&height.to_le_bytes());
    hasher.update(&handoff_id);
    *hasher.finalize().as_bytes()
}

fn validate_handoff_fields(
    target: &HandoffTarget,
    expires_at_ms: u64,
    payload_len: usize,
    authorization_len: usize,
    evidence_requirement: Option<&EvidenceRequirement>,
) -> Result<()> {
    match target {
        HandoffTarget::Session(id) | HandoffTarget::Policy(id) if *id == [0; 32] => {
            return Err(SessionError::Continuity("handoff target must not be zero".to_owned()));
        }
        _ => {}
    }
    if expires_at_ms == 0 || payload_len == 0 || payload_len > MAX_HANDOFF_PAYLOAD_BYTES {
        return Err(SessionError::Continuity("handoff expiry or payload bounds are invalid".to_owned()));
    }
    if authorization_len == 0 || authorization_len > MAX_HANDOFF_AUTHORIZATION_BYTES {
        return Err(SessionError::Continuity("handoff authorization bounds are invalid".to_owned()));
    }
    if let Some(requirement) = evidence_requirement {
        if requirement.message_id == [0; 32] || requirement.pipeline_commitment == [0; 32] {
            return Err(SessionError::Continuity("handoff evidence requirement has a zero commitment".to_owned()));
        }
    }
    Ok(())
}
