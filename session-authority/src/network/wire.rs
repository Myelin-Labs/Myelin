use super::*;
use myelin_session::{bounded_codec, FrameInput, HandoffAction, HandoffIntent};
use serde::{de::DeserializeOwned, Deserialize, Serialize};

pub(super) const VERSION: u32 = 2;
pub const MAX_PACKET_BYTES: usize = 64 * 1024 * 1024;

#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub(super) struct Call {
    pub policy: Hash32,
    pub challenge: Hash32,
    pub operation: Operation,
}

#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub(super) enum Operation {
    Observe,
    Grant(GrantRequest),
    Commit {
        authorization: CommitAuthorization,
        candidate: Box<Candidate>,
    },
    Reconcile {
        authorization: CommitAuthorization,
        #[serde(deserialize_with = "bounded_codec::bounded_vec")]
        commands: Vec<CommandIdentity>,
    },
    Block {
        height: u64,
    },
    Effect {
        height: u64,
        message_id: Hash32,
    },
    Claims {
        #[serde(deserialize_with = "bounded_codec::bounded_vec")]
        ids: Vec<Hash32>,
        minimum_epoch: u64,
    },
}

// This gateway rejects successor changes. Worker local_now_ms is deliberately
// absent from both the signed candidate identity and this transport.
#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub(super) struct Candidate {
    #[serde(deserialize_with = "bounded_codec::transactions")]
    transactions: Vec<Vec<u8>>,
    frame_input: FrameInput,
    scheduler_commitment: Hash32,
    #[serde(deserialize_with = "bounded_codec::bounded_vec")]
    data_commitments: Vec<Hash32>,
    timestamp_ms: u64,
    #[serde(deserialize_with = "bounded_codec::bounded_vec")]
    handoffs: Vec<HandoffIntent>,
    #[serde(deserialize_with = "bounded_codec::bounded_vec")]
    consume_handoffs: Vec<Hash32>,
    #[serde(deserialize_with = "bounded_codec::bounded_vec")]
    handoff_actions: Vec<HandoffAction>,
}
impl Candidate {
    pub fn from_request(request: BlockRequest) -> NetworkResult<Self> {
        if request.successor.is_some() {
            return Err(ClientError::Local("successor requires a separately authorized protocol".into()));
        }
        Ok(Self {
            transactions: request.transactions,
            frame_input: request.frame_input,
            scheduler_commitment: request.scheduler_commitment,
            data_commitments: request.data_commitments,
            timestamp_ms: request.timestamp_ms,
            handoffs: request.handoffs,
            consume_handoffs: request.consume_handoffs,
            handoff_actions: request.handoff_actions,
        })
    }
    pub fn into_request(self) -> BlockRequest {
        BlockRequest {
            transactions: self.transactions,
            frame_input: self.frame_input,
            scheduler_commitment: self.scheduler_commitment,
            data_commitments: self.data_commitments,
            timestamp_ms: self.timestamp_ms,
            // The server supplies the only operational clock, in commit().
            local_now_ms: 0,
            successor: None,
            handoffs: self.handoffs,
            consume_handoffs: self.consume_handoffs,
            handoff_actions: self.handoff_actions,
        }
    }
}

#[derive(Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub(super) enum Outcome {
    Observation(Observation),
    Grant(WriterGrant),
    Commit(CanonicalReceipt),
    Reconciliation(CommandReconciliation),
    Block {
        #[serde(deserialize_with = "bounded_codec::bounded_bytes")]
        record: Vec<u8>,
        receipt: CanonicalReceipt,
    },
    Effect {
        certificate: CanonicalEffect,
        message: OutboxMessage,
    },
    Claims(ClaimReconciliation),
}

#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub(super) struct Reply {
    pub request_digest: Hash32,
    pub outcome: std::result::Result<Outcome, RemoteFailure>,
    #[serde(deserialize_with = "bounded_codec::bounded_bytes")]
    pub signature: Vec<u8>,
}
impl Reply {
    pub fn digest(&self) -> crate::Result<Hash32> {
        protocol::hash(b"myelin:writer-fence-rpc-reply:v1", &(self.request_digest, &self.outcome))
    }
    pub fn verify(&self, policy: &FencingPolicy, request: Hash32) -> NetworkResult<()> {
        if self.request_digest != request {
            return Err(ClientError::Untrusted("response belongs to another request".into()));
        }
        protocol::verify(policy.certificate_key, self.digest().map_err(untrusted)?, &self.signature).map_err(untrusted)
    }
}
pub(super) fn request_digest(body: &[u8]) -> Hash32 {
    let mut h = blake3::Hasher::new();
    h.update(b"myelin:writer-fence-rpc-request:v1");
    h.update(body);
    *h.finalize().as_bytes()
}
pub(super) fn encode(value: &impl Serialize, limit: usize) -> std::result::Result<proto::Packet, String> {
    bounded_codec::size_with_limit(value, "authority_packet_bytes", limit.min(MAX_PACKET_BYTES) as u64).map_err(|e| e.to_string())?;
    Ok(proto::Packet { version: VERSION, body: postcard::to_allocvec(value).map_err(|e| e.to_string())? })
}
pub(super) fn decode<T: DeserializeOwned>(packet: &proto::Packet, limit: usize) -> std::result::Result<T, String> {
    if packet.version != VERSION || packet.body.len() > limit.min(MAX_PACKET_BYTES) {
        return Err("unsupported authority protocol version or packet limit".into());
    }
    let (value, rest) = postcard::take_from_bytes(&packet.body).map_err(|e| e.to_string())?;
    if !rest.is_empty() {
        return Err("trailing authority packet bytes".into());
    }
    Ok(value)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn signed_replies_bind_exact_request_result_and_pinned_certificate_key() {
        let key = AuthoritySigner::new([9; 32]).unwrap();
        let policy = FencingPolicy {
            session_id: [1; 32],
            genesis_commitment: [2; 32],
            administrator: AuthoritySigner::new([8; 32]).unwrap().public_key(),
            certificate_key: key.public_key(),
            effect_domain: [3; 32],
        };
        let request = Call { policy: policy.commitment(), challenge: [1; 32], operation: Operation::Observe };
        let packet = encode(&request, 1024).unwrap();
        let digest = request_digest(&packet.body);
        let mut reply = Reply {
            request_digest: digest,
            outcome: Err(RemoteFailure { code: RemoteCode::Fenced, message: "old epoch".into() }),
            signature: vec![],
        };
        reply.signature = key.sign(reply.digest().unwrap());
        reply.verify(&policy, digest).unwrap();
        let encoded = encode(&reply, 1024).unwrap();
        for mutation in 0..5 {
            let mut copy: Reply = decode(&encoded, 1024).unwrap();
            let mut pinned = policy.clone();
            let mut expected = digest;
            match mutation {
                0 => copy.outcome.as_mut().unwrap_err().code = RemoteCode::Uncertain,
                1 => copy.outcome.as_mut().unwrap_err().message.push('!'),
                2 => copy.signature[0] ^= 1,
                3 => pinned.certificate_key = AuthoritySigner::new([10; 32]).unwrap().public_key(),
                _ => expected[0] ^= 1,
            }
            assert!(copy.verify(&pinned, expected).is_err());
        }
        let changed = Call { challenge: [2; 32], ..request };
        assert_ne!(request_digest(&encode(&changed, 1024).unwrap().body), digest);
    }

    #[test]
    fn packet_versions_trailing_bytes_and_limits_reject_before_dispatch() {
        let call = Call { policy: [1; 32], challenge: [2; 32], operation: Operation::Observe };
        let mut packet = encode(&call, 1024).unwrap();
        let exact_size = packet.body.len();
        assert!(encode(&call, exact_size).is_ok());
        assert!(encode(&call, exact_size - 1).is_err());
        assert!(decode::<Call>(&packet, exact_size - 1).is_err());
        for version in [0, 1, VERSION + 1] {
            packet.version = version;
            assert!(decode::<Call>(&packet, 1024).is_err());
        }
        packet.version = VERSION;
        packet.body.push(0);
        assert!(decode::<Call>(&packet, 1024).is_err());
        // Valid scope/challenge, unknown operation discriminant.
        packet.body[64] = 127;
        assert!(decode::<Call>(&packet, 1024).is_err());
    }
}
