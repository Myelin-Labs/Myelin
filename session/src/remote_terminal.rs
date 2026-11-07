//! Shared wire contract for the trusted remote terminal authority. A signed
//! consume authorization is an irreversible reservation, not a target commit.
use crate::{bounded_codec, decode_record, encode_record, HandoffBlockReference, Hash32, Result, SessionError};
use secp256k1::{schnorr::Signature, Message, Secp256k1, XOnlyPublicKey};
use serde::{Deserialize, Serialize};

pub const REMOTE_TERMINAL_VERSION: u16 = 1;
pub const MAX_REMOTE_TERMINAL_BYTES: usize = 4096;

/// Genesis-bound coordination identity. Session IDs, rather than full genesis
/// hashes, avoid a circular commitment through the application's own policy.
/// Every individual decision additionally binds both complete genesis hashes.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct RemoteTerminalPolicy {
    pub version: u16,
    pub domain: Hash32,
    pub coordinator_key: Hash32,
    pub source_session: Hash32,
    pub target_session: Hash32,
}
impl RemoteTerminalPolicy {
    pub fn validate(&self) -> Result<()> {
        if self.version != REMOTE_TERMINAL_VERSION
            || [self.domain, self.coordinator_key, self.source_session, self.target_session].contains(&[0; 32])
            || self.source_session == self.target_session
        {
            return Err(invalid("invalid remote terminal policy"));
        }
        XOnlyPublicKey::from_slice(&self.coordinator_key).map_err(invalid)?;
        Ok(())
    }
    pub fn commitment(&self) -> Result<Hash32> {
        self.validate()?;
        digest(b"myelin:remote-terminal-policy:v1", self)
    }
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub enum RemoteTerminalDecision {
    ConsumeAuthorized { candidate: HandoffBlockReference },
    Cancelled { authorization_digest: Hash32 },
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct RemoteTerminalCertificate {
    pub version: u16,
    pub policy: Hash32,
    pub source_genesis: Hash32,
    pub target_genesis: Hash32,
    pub handoff_id: Hash32,
    pub decision: RemoteTerminalDecision,
    #[serde(deserialize_with = "bounded_codec::bounded_bytes")]
    pub signature: Vec<u8>,
}
/// Only coordinator-signature verification constructs this value. It supplies
/// no assertion that a candidate has actually committed to the target store.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct VerifiedRemoteTerminal {
    certificate: RemoteTerminalCertificate,
}
impl VerifiedRemoteTerminal {
    pub fn certificate(&self) -> &RemoteTerminalCertificate {
        &self.certificate
    }
    pub fn decision(&self) -> &RemoteTerminalDecision {
        &self.certificate.decision
    }
}
impl RemoteTerminalCertificate {
    pub fn unsigned(
        policy: &RemoteTerminalPolicy,
        source_genesis: Hash32,
        target_genesis: Hash32,
        handoff_id: Hash32,
        decision: RemoteTerminalDecision,
    ) -> Result<Self> {
        let certificate = Self {
            version: REMOTE_TERMINAL_VERSION,
            policy: policy.commitment()?,
            source_genesis,
            target_genesis,
            handoff_id,
            decision,
            signature: vec![],
        };
        certificate.digest()?;
        Ok(certificate)
    }
    pub fn digest(&self) -> Result<Hash32> {
        if self.version != REMOTE_TERMINAL_VERSION
            || [self.policy, self.source_genesis, self.target_genesis, self.handoff_id].contains(&[0; 32])
            || self.source_genesis == self.target_genesis
        {
            return Err(invalid("invalid remote terminal statement"));
        }
        match &self.decision {
            RemoteTerminalDecision::ConsumeAuthorized { candidate } => {
                if [candidate.session_id, candidate.block_hash].contains(&[0; 32]) {
                    return Err(invalid("invalid consumption candidate"));
                }
            }
            RemoteTerminalDecision::Cancelled { authorization_digest } if *authorization_digest == [0; 32] => {
                return Err(invalid("cancellation has no authorization binding"))
            }
            _ => {}
        }
        digest(
            b"myelin:remote-terminal-certificate:v1",
            &(self.version, self.policy, self.source_genesis, self.target_genesis, self.handoff_id, &self.decision),
        )
    }
    pub fn encode(&self) -> Result<Vec<u8>> {
        self.digest()?;
        if self.signature.len() != 64 {
            return Err(invalid("remote terminal signature must contain 64 bytes"));
        }
        bounded_codec::size_with_limit(
            self,
            "remote_terminal_bytes",
            (MAX_REMOTE_TERMINAL_BYTES - crate::RECORD_HEADER_BYTES) as u64,
        )?;
        encode_record(self)
    }
    pub fn decode(bytes: &[u8]) -> Result<Self> {
        if bytes.len() > MAX_REMOTE_TERMINAL_BYTES {
            return Err(invalid("remote terminal certificate exceeds byte limit"));
        }
        let certificate: Self = decode_record(bytes)?;
        certificate.encode()?;
        Ok(certificate)
    }
    pub fn verify(
        &self,
        policy: &RemoteTerminalPolicy,
        source_genesis: Hash32,
        target_genesis: Hash32,
        handoff_id: Hash32,
    ) -> Result<VerifiedRemoteTerminal> {
        if self.policy != policy.commitment()?
            || self.source_genesis != source_genesis
            || self.target_genesis != target_genesis
            || self.handoff_id != handoff_id
        {
            return Err(invalid("remote terminal policy/genesis/handoff scope mismatch"));
        }
        if let RemoteTerminalDecision::ConsumeAuthorized { candidate } = &self.decision {
            if candidate.session_id != policy.target_session {
                return Err(invalid("remote consumption targets another session"));
            }
        }
        let signature = Signature::from_slice(&self.signature).map_err(invalid)?;
        let key = XOnlyPublicKey::from_slice(&policy.coordinator_key).map_err(invalid)?;
        Secp256k1::verification_only().verify_schnorr(&signature, &Message::from_digest(self.digest()?), &key).map_err(invalid)?;
        Ok(VerifiedRemoteTerminal { certificate: self.clone() })
    }
}

/// Binds the original application's cancellation authorization bytes. The
/// application must also verify its owner's signature and exact lock semantics.
pub fn remote_cancellation_digest(source_genesis: Hash32, handoff_id: Hash32, authorization: &[u8]) -> Result<Hash32> {
    if authorization.is_empty() || authorization.len() > MAX_REMOTE_TERMINAL_BYTES {
        return Err(invalid("invalid remote cancellation authorization length"));
    }
    digest(b"myelin:remote-terminal-cancellation:v1", &(source_genesis, handoff_id, authorization))
}
fn digest<T: Serialize>(domain: &[u8], value: &T) -> Result<Hash32> {
    bounded_codec::size_with_limit(value, "remote_terminal_digest_bytes", MAX_REMOTE_TERMINAL_BYTES as u64)?;
    let mut hash = blake3::Hasher::new();
    hash.update(domain);
    hash.update(&postcard::to_allocvec(value).map_err(invalid)?);
    Ok(*hash.finalize().as_bytes())
}
fn invalid(error: impl ToString) -> SessionError {
    SessionError::Continuity(error.to_string())
}

#[cfg(test)]
mod tests {
    use super::*;
    use secp256k1::{Keypair, SecretKey};
    fn sign(certificate: &mut RemoteTerminalCertificate, key: &Keypair) {
        certificate.signature =
            Secp256k1::new().sign_schnorr_no_aux_rand(&Message::from_digest(certificate.digest().unwrap()), key).serialize().to_vec();
    }
    #[test]
    fn terminal_signatures_bind_policy_geneses_identity_exact_candidate_and_decision() {
        let key = Keypair::from_secret_key(&Secp256k1::new(), &SecretKey::from_slice(&[17; 32]).unwrap());
        let policy = RemoteTerminalPolicy {
            version: REMOTE_TERMINAL_VERSION,
            domain: [3; 32],
            coordinator_key: key.x_only_public_key().0.serialize(),
            source_session: [1; 32],
            target_session: [2; 32],
        };
        let decision = RemoteTerminalDecision::ConsumeAuthorized {
            candidate: HandoffBlockReference { session_id: [2; 32], height: 0, block_hash: [8; 32] },
        };
        let mut certificate = RemoteTerminalCertificate::unsigned(&policy, [4; 32], [5; 32], [6; 32], decision).unwrap();
        sign(&mut certificate, &key);
        let bytes = certificate.encode().unwrap();
        let decoded = RemoteTerminalCertificate::decode(&bytes).unwrap();
        assert_eq!(decoded.verify(&policy, [4; 32], [5; 32], [6; 32]).unwrap().certificate(), &certificate);
        for case in 0..8 {
            let mut forged = certificate.clone();
            match case {
                0 => forged.policy[0] ^= 1,
                1 => forged.source_genesis[0] ^= 1,
                2 => forged.target_genesis[0] ^= 1,
                3 => forged.handoff_id[0] ^= 1,
                4 => forged.version += 1,
                5 => {
                    if let RemoteTerminalDecision::ConsumeAuthorized { candidate } = &mut forged.decision {
                        candidate.height += 1
                    }
                }
                6 => {
                    if let RemoteTerminalDecision::ConsumeAuthorized { candidate } = &mut forged.decision {
                        candidate.block_hash[0] ^= 1
                    }
                }
                7 => forged.decision = RemoteTerminalDecision::Cancelled { authorization_digest: [9; 32] },
                _ => unreachable!(),
            }
            assert!(forged.verify(&policy, [4; 32], [5; 32], [6; 32]).is_err(), "mutation {case}");
        }
        let other = Keypair::from_secret_key(&Secp256k1::new(), &SecretKey::from_slice(&[18; 32]).unwrap());
        let mut wrong_key = policy.clone();
        wrong_key.coordinator_key = other.x_only_public_key().0.serialize();
        let mut forged = certificate.clone();
        forged.policy = wrong_key.commitment().unwrap();
        sign(&mut forged, &key);
        assert!(forged.verify(&wrong_key, [4; 32], [5; 32], [6; 32]).is_err());
        let auth = remote_cancellation_digest([4; 32], [6; 32], &[9; 64]).unwrap();
        let mut cancel = RemoteTerminalCertificate::unsigned(
            &policy,
            [4; 32],
            [5; 32],
            [6; 32],
            RemoteTerminalDecision::Cancelled { authorization_digest: auth },
        )
        .unwrap();
        sign(&mut cancel, &key);
        assert!(matches!(
            cancel.verify(&policy, [4; 32], [5; 32], [6; 32]).unwrap().decision(),
            RemoteTerminalDecision::Cancelled { .. }
        ));
        assert_ne!(remote_cancellation_digest([4; 32], [6; 32], &[10; 64]).unwrap(), auth);
        let mut trailing = bytes;
        trailing.push(0);
        assert!(RemoteTerminalCertificate::decode(&trailing).is_err());
        assert!(remote_cancellation_digest([4; 32], [6; 32], &[]).is_err());
    }
}

/// Structural checks independent of local trust configuration. Extra or repeated
/// evidence is rejected rather than ignored. A candidate signature alone is not
/// an authorization to consume remote scarce rights.
pub(crate) fn validate_terminal_envelope(record: &crate::FinalisedBlockRecord) -> Result<()> {
    let mut seen = std::collections::HashSet::new();
    let at = HandoffBlockReference::from_block(&record.block);
    for certificate in &record.remote_terminals {
        certificate.encode()?;
        if !seen.insert(certificate.handoff_id)
            || !record.consumed_handoff_ids.contains(&certificate.handoff_id)
            || certificate.decision != (RemoteTerminalDecision::ConsumeAuthorized { candidate: at.clone() })
        {
            return Err(invalid("terminal evidence does not match this unique consumption candidate"));
        }
    }
    Ok(())
}

/// Recheck scarce consumption at the live, durable commit and replay boundaries.
/// No certificate is required for a genuine shared-store handoff or copyable
/// receipt; supplying an unrelated certificate is rejected by the envelope.
pub fn validate_remote_consumption(
    target: &crate::SessionConfig,
    source: &crate::SessionConfig,
    record: &crate::FinalisedBlockRecord,
    handoff: &crate::SessionHandoff,
) -> Result<()> {
    crate::validate_handoff_trust(target, source)?;
    let certificates = record.remote_terminals.iter().filter(|certificate| certificate.handoff_id == handoff.id()).collect::<Vec<_>>();
    let Some(policy) = target.protocol_effects.remote_terminal() else {
        if !certificates.is_empty() {
            return Err(invalid("terminal evidence supplied for uncoordinated consumption"));
        }
        return Ok(());
    };
    if certificates.len() != 1 {
        return Err(invalid("genesis requires exactly one exclusive terminal certificate"));
    }
    let verified = certificates[0].verify(policy, source.commitment(), target.commitment(), handoff.id())?;
    if verified.decision()
        != &(RemoteTerminalDecision::ConsumeAuthorized { candidate: HandoffBlockReference::from_block(&record.block) })
    {
        return Err(invalid("terminal authorization names another target candidate"));
    }
    Ok(())
}

/// Flat foreign consumption evidence, retained inside the source retirement
/// action. It never creates local target history. Its target record must itself
/// be an isolated terminal-authorized consume, so proof depth cannot recurse.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct RemoteConsumptionProof {
    pub import_policy: Hash32,
    #[serde(deserialize_with = "bounded_codec::bounded_bytes")]
    pub target_record: Vec<u8>,
}
impl RemoteConsumptionProof {
    pub fn verify(
        &self,
        registry: &crate::HandoffImportRegistry,
        source: &crate::SessionConfig,
        handoff: &crate::SessionHandoff,
    ) -> Result<crate::FinalisedBlockRecord> {
        if self.target_record.len() > crate::MAX_REMOTE_PROOF_BYTES {
            return Err(invalid("remote consumption proof exceeds ceiling"));
        }
        let (registered_source, target) = registry.configs(self.import_policy)?;
        if registered_source != source || source.session_id != handoff.source_session_id {
            return Err(invalid("remote completion belongs to another source"));
        }
        let Some((id, crate::HandoffImportKind::CoordinatedScarce { terminal_policy })) =
            registry.classify_registered(source, target, handoff)?
        else {
            return Err(invalid("remote completion requires recognized scarce semantics"));
        };
        let policy = registry.terminal_policy(id)?;
        if policy.commitment()? != terminal_policy {
            return Err(invalid("remote completion terminal policy differs"));
        }
        let record = registry.verify_target_block(self.import_policy, &self.target_record)?;
        if record.consumed_handoff_ids != [handoff.id()]
            || !record.issued_handoffs.is_empty()
            || !record.handoff_actions.is_empty()
            || record.successor.is_some()
            || record.remote_terminals.len() != 1
            || record.block.timestamp_ms > handoff.expires_at_ms
        {
            return Err(invalid("remote completion requires the exact isolated consumption"));
        }
        let certificate = &record.remote_terminals[0];
        let verified = certificate.verify(&policy, source.commitment(), target.commitment(), handoff.id())?;
        if verified.decision()
            != &(RemoteTerminalDecision::ConsumeAuthorized { candidate: HandoffBlockReference::from_block(&record.block) })
        {
            return Err(invalid("remote completion certificate names another candidate"));
        }
        Ok(record)
    }
}
