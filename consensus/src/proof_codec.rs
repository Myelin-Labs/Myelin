use super::{
    catalog::proof_schema_hash, CommitteeCertificate, CommitteeSignature, ConsensusError, ConsensusKind, FinalityProof, Hash32,
    ProofOfAuthoritySeal, Result, TendermintDecision, TendermintStep, TendermintVote,
};
use serde::{Deserialize, Serialize};

const PROOF_FORMAT_VERSION: u16 = 1;
const MAX_PROOF_BYTES: usize = 8 * 1024 * 1024;
const MAX_SIGNATURES: usize = 4_096;
const MAX_VALIDATOR_ID_BYTES: usize = 128;

pub(crate) fn schema_hash(kind: ConsensusKind) -> Hash32 {
    proof_schema_hash(kind)
}

pub(crate) fn encode(proof: &FinalityProof) -> Result<Vec<u8>> {
    let payload = ProofWire::from(proof);
    validate_payload(&payload)?;
    let payload_bytes = serde_json::to_vec(&payload).map_err(codec)?;
    let envelope = ProofEnvelopeWire {
        format_version: PROOF_FORMAT_VERSION,
        consensus_kind: proof.kind().as_str().to_owned(),
        proof_schema_hash: schema_hash(proof.kind()),
        payload_hash: *blake3::hash(&payload_bytes).as_bytes(),
        payload,
    };
    let encoded = serde_json::to_vec(&envelope).map_err(codec)?;
    if encoded.len() > MAX_PROOF_BYTES {
        return Err(ConsensusError::ProofCodec(format!("proof exceeds {MAX_PROOF_BYTES} bytes")));
    }
    Ok(encoded)
}

pub(crate) fn decode(bytes: &[u8]) -> Result<FinalityProof> {
    if bytes.is_empty() || bytes.len() > MAX_PROOF_BYTES {
        return Err(ConsensusError::ProofCodec(format!("proof size must be 1..={MAX_PROOF_BYTES} bytes")));
    }
    let envelope: ProofEnvelopeWire = serde_json::from_slice(bytes).map_err(codec)?;
    if envelope.format_version != PROOF_FORMAT_VERSION {
        return Err(ConsensusError::ProofCodec(format!("unsupported proof format version {}", envelope.format_version)));
    }
    let kind = ConsensusKind::from_canonical_str(&envelope.consensus_kind)?;
    if envelope.proof_schema_hash != schema_hash(kind) {
        return Err(ConsensusError::ProofCodec("proof schema commitment mismatch".to_owned()));
    }
    if envelope.payload.kind() != kind {
        return Err(ConsensusError::ProofCodec("proof kind does not match its envelope".to_owned()));
    }
    validate_payload(&envelope.payload)?;
    let payload_bytes = serde_json::to_vec(&envelope.payload).map_err(codec)?;
    if *blake3::hash(&payload_bytes).as_bytes() != envelope.payload_hash {
        return Err(ConsensusError::ProofCodec("proof payload hash mismatch".to_owned()));
    }
    let proof = FinalityProof::try_from(envelope.payload)?;
    let canonical = encode(&proof)?;
    if canonical != bytes {
        return Err(ConsensusError::ProofCodec("proof encoding is not canonical".to_owned()));
    }
    Ok(proof)
}

#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct ProofEnvelopeWire {
    format_version: u16,
    consensus_kind: String,
    proof_schema_hash: Hash32,
    payload_hash: Hash32,
    payload: ProofWire,
}

#[derive(Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "kebab-case", deny_unknown_fields)]
enum ProofWire {
    StaticClosedCommittee { block_hash: Hash32, signatures: Vec<SignatureWire> },
    ProofOfAuthority { block_hash: Hash32, height: u64, authority_id: String, signature: Vec<u8> },
    Tendermint { decision: TendermintDecision },
}

impl ProofWire {
    fn kind(&self) -> ConsensusKind {
        match self {
            Self::StaticClosedCommittee { .. } => ConsensusKind::StaticClosedCommittee,
            Self::ProofOfAuthority { .. } => ConsensusKind::ProofOfAuthority,
            Self::Tendermint { .. } => ConsensusKind::Tendermint,
        }
    }
}

#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct SignatureWire {
    validator_id: String,
    signature: Vec<u8>,
}

impl From<&FinalityProof> for ProofWire {
    fn from(value: &FinalityProof) -> Self {
        match value {
            FinalityProof::StaticClosedCommittee(certificate) => Self::StaticClosedCommittee {
                block_hash: certificate.block_hash,
                signatures: certificate.signatures.iter().map(SignatureWire::from).collect(),
            },
            FinalityProof::ProofOfAuthority(seal) => Self::ProofOfAuthority {
                block_hash: seal.block_hash,
                height: seal.height,
                authority_id: seal.authority_id.clone(),
                signature: seal.signature.to_vec(),
            },
            FinalityProof::Tendermint(decision) => Self::Tendermint { decision: decision.clone() },
        }
    }
}

impl From<&CommitteeSignature> for SignatureWire {
    fn from(value: &CommitteeSignature) -> Self {
        Self { validator_id: value.validator_id.clone(), signature: value.signature.to_vec() }
    }
}

impl TryFrom<ProofWire> for FinalityProof {
    type Error = ConsensusError;

    fn try_from(value: ProofWire) -> Result<Self> {
        match value {
            ProofWire::StaticClosedCommittee { block_hash, signatures } => Ok(Self::StaticClosedCommittee(CommitteeCertificate {
                block_hash,
                signatures: signatures.into_iter().map(CommitteeSignature::try_from).collect::<Result<Vec<_>>>()?,
            })),
            ProofWire::ProofOfAuthority { block_hash, height, authority_id, signature } => {
                Ok(Self::ProofOfAuthority(ProofOfAuthoritySeal {
                    block_hash,
                    height,
                    authority_id,
                    signature: fixed_signature::<65>(signature, "CKB PoA signature")?,
                }))
            }
            ProofWire::Tendermint { decision } => Ok(Self::Tendermint(decision)),
        }
    }
}

impl TryFrom<SignatureWire> for CommitteeSignature {
    type Error = ConsensusError;

    fn try_from(value: SignatureWire) -> Result<Self> {
        Ok(Self { validator_id: value.validator_id, signature: fixed_signature::<64>(value.signature, "signature")? })
    }
}

fn validate_payload(payload: &ProofWire) -> Result<()> {
    match payload {
        ProofWire::StaticClosedCommittee { signatures, .. } => validate_signatures(signatures),
        ProofWire::ProofOfAuthority { authority_id, signature, .. } => {
            validate_id(authority_id)?;
            if signature.len() != 65 {
                return Err(ConsensusError::ProofCodec("CKB PoA signature must be exactly 65 bytes".to_owned()));
            }
            Ok(())
        }
        ProofWire::Tendermint { decision } => {
            if decision.precommits.len() > MAX_SIGNATURES {
                return Err(ConsensusError::ProofCodec("too many Tendermint precommits".to_owned()));
            }
            for TendermintVote { step, validator_id, signature, .. } in &decision.precommits {
                if *step != TendermintStep::Precommit {
                    return Err(ConsensusError::ProofCodec("Tendermint decision contains a non-precommit vote".to_owned()));
                }
                validate_id(validator_id)?;
                if signature.len() != 64 {
                    return Err(ConsensusError::ProofCodec("Tendermint signature must be exactly 64 bytes".to_owned()));
                }
            }
            Ok(())
        }
    }
}

fn validate_signatures(signatures: &[SignatureWire]) -> Result<()> {
    if signatures.len() > MAX_SIGNATURES {
        return Err(ConsensusError::ProofCodec("too many committee signatures".to_owned()));
    }
    for signature in signatures {
        validate_id(&signature.validator_id)?;
        if signature.signature.len() != 64 {
            return Err(ConsensusError::ProofCodec("signature must be exactly 64 bytes".to_owned()));
        }
    }
    Ok(())
}

fn validate_id(id: &str) -> Result<()> {
    if id.is_empty() || id.len() > MAX_VALIDATOR_ID_BYTES {
        return Err(ConsensusError::ProofCodec(format!("validator/authority id length must be 1..={MAX_VALIDATOR_ID_BYTES} bytes")));
    }
    Ok(())
}

fn fixed_signature<const N: usize>(value: Vec<u8>, label: &str) -> Result<[u8; N]> {
    value.try_into().map_err(|_| ConsensusError::ProofCodec(format!("{label} must be exactly {N} bytes")))
}

fn codec(error: impl std::fmt::Display) -> ConsensusError {
    ConsensusError::ProofCodec(error.to_string())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn proof_codec_roundtrips_and_rejects_noncanonical_or_mismatched_envelopes() {
        let proof = FinalityProof::StaticClosedCommittee(CommitteeCertificate {
            block_hash: [3; 32],
            signatures: vec![CommitteeSignature { validator_id: "alice".to_owned(), signature: [4; 64] }],
        });
        let encoded = proof.encode().unwrap();
        assert_eq!(FinalityProof::decode(&encoded).unwrap(), proof);

        let mut with_whitespace = encoded.clone();
        with_whitespace.push(b'\n');
        assert!(matches!(FinalityProof::decode(&with_whitespace), Err(ConsensusError::ProofCodec(_))));

        let mut value: serde_json::Value = serde_json::from_slice(&encoded).unwrap();
        value["proof_schema_hash"] = serde_json::json!(vec![0_u8; 32]);
        let wrong_schema = serde_json::to_vec(&value).unwrap();
        assert!(matches!(FinalityProof::decode(&wrong_schema), Err(ConsensusError::ProofCodec(_))));
    }
}
