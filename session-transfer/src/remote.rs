//! Application admission for independently verified remote source proofs.
//! This performs owner/schema/policy checks; the session verifies source finality
//! and the store supplies atomic consumption and any required terminal decision.
use crate::*;
use myelin_session::{
    EvidenceReceipt, EvidenceStage, HandoffImportKind, HandoffImportVerifier, OutboxMessage, SessionError, SessionHandoff,
};
use std::sync::Arc;

pub type RemoteEvidenceCheck =
    dyn Fn(&OutboxMessage, &EvidenceStage, Option<&EvidenceReceipt>, &[u8]) -> myelin_session::Result<()> + Send + Sync;

pub struct TransferImportVerifier {
    policy: TransferPolicy,
    source_policies: std::collections::BTreeMap<Hash32, TransferPolicy>,
    evidence: Arc<RemoteEvidenceCheck>,
}
impl TransferImportVerifier {
    pub fn new(policy: TransferPolicy, source_policies: Vec<TransferPolicy>, evidence: Arc<RemoteEvidenceCheck>) -> Result<Self> {
        policy.validate()?;
        if source_policies.is_empty() || source_policies.len() > 1024 {
            return Err(Error::Bounds("recognized source policies"));
        }
        let mut recognized = std::collections::BTreeMap::new();
        for source in source_policies {
            if recognized.insert(source.commitment()?, source).is_some() {
                return Err(Error::Policy("duplicate source policy".into()));
            }
        }
        Ok(Self { policy, source_policies: recognized, evidence })
    }
}
impl HandoffImportVerifier for TransferImportVerifier {
    fn verify_application(
        &self,
        source: &SessionConfig,
        target: &SessionConfig,
        handoff: &SessionHandoff,
    ) -> myelin_session::Result<HandoffImportKind> {
        let verify = || -> Result<HandoffImportKind> {
            let intent = HandoffIntent {
                target: handoff.target.clone(),
                expires_at_ms: handoff.expires_at_ms,
                payload: handoff.payload.clone(),
                authorization: handoff.authorization.clone(),
                evidence_requirement: handoff.evidence_requirement.clone(),
            };
            let (_, payload) = verify_intent(&intent)?;
            self.policy.matches_payload(&payload)?;
            let Some(policy_hash) = source.protocol_effects.application_policy_hash() else {
                return Err(Error::Policy("remote intake requires application-verified source semantics".into()));
            };
            let source_policy = self
                .source_policies
                .get(&policy_hash)
                .ok_or_else(|| Error::Policy("source semantics have no independently recognized policy".into()))?;
            source_policy.matches_payload(&payload)?;
            if source.application_profile.input_schema_hash != input_schema()
                || source.application_profile.state_codec_hash != state_codec()
                || source.application_profile.handoff_policy_hash != source_policy.intake_policy()?
            {
                return Err(Error::Policy("source application schema/policy mismatch".into()));
            }
            if payload.kind == TransferKind::Scarce {
                let TransferCoordination::Remote { terminal } = &self.policy.coordination else {
                    return Err(Error::Unsupported("shared-store scarce policies cannot be imported remotely"));
                };
                if source_policy.coordination != self.policy.coordination
                    || !source_policy.accepted_sources.is_empty()
                    || terminal.source_session != source.session_id
                    || terminal.target_session != target.session_id
                {
                    return Err(Error::Policy("remote source/target terminal semantics mismatch".into()));
                }
            }
            if payload.source != SourceIdentity::from_config(source)?
                || !self.policy.accepted_sources.contains(&payload.source)
                || payload.source.session_id != handoff.source_session_id
                || payload.source.application_profile_commitment != handoff.source_application_profile_commitment
                || payload.source.consensus_module_commitment != handoff.source_consensus_module_commitment
                || target.protocol_effects != self.policy.effect_policy()?
                || target.application_profile.handoff_policy_hash != self.policy.intake_policy()?
                || target.application_profile.input_schema_hash != input_schema()
                || target.application_profile.state_codec_hash != state_codec()
                || payload.target
                    != (TransferTarget::Session { session_id: target.session_id, genesis_commitment: target.commitment() })
            {
                return Err(Error::Policy("remote source/schema/authorization/target policy mismatch".into()));
            }
            Ok(match payload.kind {
                TransferKind::CopyableReceipt => HandoffImportKind::CopyableReceipt,
                TransferKind::Scarce => HandoffImportKind::CoordinatedScarce { terminal_policy: payload.uniqueness_domain },
            })
        };
        verify().map_err(|error| SessionError::ProtocolEffects(error.to_string()))
    }
    fn terminal_policy(&self) -> Option<myelin_session::RemoteTerminalPolicy> {
        match &self.policy.coordination {
            TransferCoordination::Remote { terminal } => Some(terminal.clone()),
            TransferCoordination::SharedStore => None,
        }
    }
    fn verify_cancellation(
        &self,
        source: &SessionConfig,
        handoff: &SessionHandoff,
        authorization: &[u8],
    ) -> myelin_session::Result<()> {
        verify_cancellation(source.commitment(), handoff, authorization).map_err(|e| SessionError::ProtocolEffects(e.to_string()))
    }
    fn verify_evidence(
        &self,
        message: &OutboxMessage,
        stage: &EvidenceStage,
        previous: Option<&EvidenceReceipt>,
        evidence: &[u8],
    ) -> myelin_session::Result<()> {
        (self.evidence)(message, stage, previous, evidence)
    }
}
