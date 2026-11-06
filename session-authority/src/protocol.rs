use crate::{AuthorityError, Result};
use myelin_session::{bounded_codec, BlockRequest, Hash32, OutboxMessage, SessionConfig, SessionHead};
use secp256k1::{Keypair, Message, Secp256k1, SecretKey, XOnlyPublicKey};
use serde::{Deserialize, Serialize};

/// Consumers pin this complete policy, not just an ordinary PoA public key.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct FencingPolicy {
    pub session_id: Hash32,
    pub genesis_commitment: Hash32,
    pub administrator: Hash32,
    pub certificate_key: Hash32,
    /// One authoritative external economic-effect uniqueness domain.
    pub effect_domain: Hash32,
}
impl FencingPolicy {
    pub fn new(config: &SessionConfig, administrator: Hash32, certificate_key: Hash32, effect_domain: Hash32) -> Result<Self> {
        config.validate().map_err(session)?;
        key(administrator)?;
        key(certificate_key)?;
        if administrator == certificate_key || effect_domain == [0; 32] {
            return Err(AuthorityError::Invalid("administrator and certificate roles must use different keys".into()));
        }
        Ok(Self {
            session_id: config.session_id,
            genesis_commitment: config.commitment(),
            administrator,
            certificate_key,
            effect_domain,
        })
    }
    pub fn commitment(&self) -> Hash32 {
        let mut h = blake3::Hasher::new();
        h.update(b"myelin:writer-fencing-policy:v1");
        for value in [&self.session_id, &self.genesis_commitment, &self.administrator, &self.certificate_key, &self.effect_domain] {
            h.update(value);
        }
        *h.finalize().as_bytes()
    }
}

/// Secret credentials live with their own role. Worker backups must contain
/// neither the administrator, PoA signer nor canonical-certificate secret.
pub struct AuthoritySigner(Keypair);
impl AuthoritySigner {
    pub fn new(secret: [u8; 32]) -> Result<Self> {
        let secret = SecretKey::from_slice(&secret).map_err(|e| AuthorityError::Invalid(e.to_string()))?;
        Ok(Self(Keypair::from_secret_key(&Secp256k1::new(), &secret)))
    }
    pub fn public_key(&self) -> Hash32 {
        self.0.x_only_public_key().0.serialize()
    }
    pub(crate) fn sign(&self, digest: Hash32) -> Vec<u8> {
        Secp256k1::new().sign_schnorr_no_aux_rand(&Message::from_digest(digest), &self.0).as_ref().to_vec()
    }
    pub fn authorize_grant(
        &self,
        policy: &FencingPolicy,
        expected_epoch: u64,
        owner: Hash32,
        anchor: &SessionHead,
    ) -> Result<GrantRequest> {
        let mut request = GrantRequest {
            policy: policy.commitment(),
            expected_epoch,
            owner,
            anchor: anchor.encode().map_err(session)?,
            signature: vec![],
        };
        request.signature = self.sign(request.digest()?);
        Ok(request)
    }
    pub fn authorize_commit(
        &self,
        policy: &FencingPolicy,
        grant: &WriterGrant,
        head: &SessionHead,
        request: &BlockRequest,
    ) -> Result<CommitAuthorization> {
        let mut authorization = CommitAuthorization {
            policy: policy.commitment(),
            epoch: grant.epoch,
            owner: self.public_key(),
            base_head: head.encode().map_err(session)?,
            request_digest: request_digest(request)?,
            signature: vec![],
        };
        authorization.signature = self.sign(authorization.digest()?);
        Ok(authorization)
    }
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct GrantRequest {
    pub policy: Hash32,
    pub expected_epoch: u64,
    pub owner: Hash32,
    #[serde(deserialize_with = "bounded_codec::bounded_bytes")]
    pub anchor: Vec<u8>,
    #[serde(deserialize_with = "bounded_codec::bounded_bytes")]
    pub signature: Vec<u8>,
}
impl GrantRequest {
    pub fn digest(&self) -> Result<Hash32> {
        hash(b"myelin:writer-fence-grant:v1", &(self.policy, self.expected_epoch, self.owner, &self.anchor))
    }
    pub fn anchor(&self) -> Result<SessionHead> {
        SessionHead::decode(&self.anchor).map_err(session)
    }
}
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct WriterGrant {
    pub epoch: u64,
    pub request: GrantRequest,
}
impl WriterGrant {
    pub fn verify(&self, policy: &FencingPolicy) -> Result<()> {
        if self.request.policy != policy.commitment()
            || self.epoch == 0
            || self.request.expected_epoch.checked_add(1) != Some(self.epoch)
            || self.request.anchor()?.session_id != policy.session_id
        {
            return Err(AuthorityError::Invalid("writer grant identity/epoch mismatch".into()));
        }
        verify(policy.administrator, self.request.digest()?, &self.request.signature)
    }
    pub fn owner(&self) -> Hash32 {
        self.request.owner
    }
    pub fn first_height(&self) -> Result<u64> {
        self.request.anchor()?.next_height().map_err(session)
    }
    pub fn stamp(&self, height: u64, parent: Hash32) -> Result<Hash32> {
        hash(b"myelin:writer-fence-block:v1", &(self.request.policy, self.epoch, self.owner(), height, parent))
    }
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct CommitAuthorization {
    pub policy: Hash32,
    pub epoch: u64,
    pub owner: Hash32,
    #[serde(deserialize_with = "bounded_codec::bounded_bytes")]
    pub base_head: Vec<u8>,
    pub request_digest: Hash32,
    #[serde(deserialize_with = "bounded_codec::bounded_bytes")]
    pub signature: Vec<u8>,
}
impl CommitAuthorization {
    pub fn verify_candidate(&self, policy: &FencingPolicy, grant: &WriterGrant, request: &BlockRequest) -> Result<()> {
        self.verify_binding(policy, grant)?;
        if self.request_digest != request_digest(request)? {
            return Err(AuthorityError::Authorization("signed candidate bytes changed".into()));
        }
        Ok(())
    }
    pub fn verify_binding(&self, policy: &FencingPolicy, grant: &WriterGrant) -> Result<()> {
        grant.verify(policy)?;
        if self.policy != policy.commitment()
            || self.epoch != grant.epoch
            || self.owner != grant.owner()
            || SessionHead::decode(&self.base_head).map_err(session)?.session_id != policy.session_id
            || self.request_digest == [0; 32]
        {
            return Err(AuthorityError::Authorization("signed candidate differs from grant, scope or request".into()));
        }
        verify(self.owner, self.digest()?, &self.signature)
    }
    pub fn digest(&self) -> Result<Hash32> {
        hash(b"myelin:writer-fence-request:v1", &(self.policy, self.epoch, self.owner, &self.base_head, self.request_digest))
    }
}

/// This certificate is emitted only after the canonical store commits. A local
/// PoA block from a worker copy is insufficient for a fenced deployment.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct CanonicalReceipt {
    pub policy: Hash32,
    pub epoch: u64,
    pub owner: Hash32,
    pub height: u64,
    pub block_hash: Hash32,
    pub state_root: Hash32,
    #[serde(deserialize_with = "bounded_codec::bounded_bytes")]
    pub signature: Vec<u8>,
}
impl CanonicalReceipt {
    pub fn digest(&self) -> Result<Hash32> {
        hash(
            b"myelin:writer-fence-canonical:v1",
            &(self.policy, self.epoch, self.owner, self.height, self.block_hash, self.state_root),
        )
    }
    pub fn verify(&self, policy: &FencingPolicy) -> Result<()> {
        if self.policy != policy.commitment() || self.epoch == 0 {
            return Err(AuthorityError::Invalid("canonical receipt policy/epoch mismatch".into()));
        }
        verify(policy.certificate_key, self.digest()?, &self.signature)
    }
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct CanonicalEffect {
    pub receipt: CanonicalReceipt,
    pub message_id: Hash32,
    pub message_commitment: Hash32,
    #[serde(deserialize_with = "bounded_codec::bounded_bytes")]
    pub signature: Vec<u8>,
}
impl CanonicalEffect {
    pub fn digest(&self) -> Result<Hash32> {
        hash(b"myelin:writer-fence-effect:v1", &(self.receipt.digest()?, self.message_id, self.message_commitment))
    }
    pub fn verify(&self, policy: &FencingPolicy, message: &OutboxMessage) -> Result<()> {
        self.receipt.verify(policy)?;
        if self.message_id != message.id || self.message_commitment != message_commitment(message)? {
            return Err(AuthorityError::Invalid("effect certificate does not bind this message".into()));
        }
        verify(policy.certificate_key, self.digest()?, &self.signature)
    }
}
pub(crate) fn message_commitment(message: &OutboxMessage) -> Result<Hash32> {
    hash(b"myelin:writer-fence-message:v1", &(message.id, message.consensus_module_commitment, &message.topic, &message.payload))
}
pub(crate) fn request_digest(request: &BlockRequest) -> Result<Hash32> {
    // local_now_ms is supplied by the authority's clock, never by the worker.
    let successor = request.successor.as_ref().map(SessionConfig::encode).transpose().map_err(session)?;
    hash(
        b"myelin:writer-fence-candidate:v1",
        &(
            &request.transactions,
            &request.frame_input,
            request.scheduler_commitment,
            &request.data_commitments,
            request.timestamp_ms,
            successor,
            &request.handoffs,
            &request.consume_handoffs,
            &request.handoff_actions,
        ),
    )
}
pub(crate) fn hash(domain: &[u8], value: &impl Serialize) -> Result<Hash32> {
    let mut h = blake3::Hasher::new();
    h.update(domain);
    postcard::serialize_with_flavor(value, HashFlavor { hasher: h, bytes: 0 }).map_err(|e| AuthorityError::Invalid(e.to_string()))
}
struct HashFlavor {
    hasher: blake3::Hasher,
    bytes: usize,
}
impl postcard::ser_flavors::Flavor for HashFlavor {
    type Output = Hash32;
    fn try_push(&mut self, byte: u8) -> postcard::Result<()> {
        self.try_extend(&[byte])
    }
    fn try_extend(&mut self, bytes: &[u8]) -> postcard::Result<()> {
        self.bytes =
            self.bytes.checked_add(bytes.len()).filter(|n| *n <= 64 * 1024 * 1024).ok_or(postcard::Error::SerializeBufferFull)?;
        self.hasher.update(bytes);
        Ok(())
    }
    fn finalize(self) -> postcard::Result<Hash32> {
        Ok(*self.hasher.finalize().as_bytes())
    }
}
pub(crate) fn verify(public: Hash32, digest: Hash32, signature: &[u8]) -> Result<()> {
    let signature = secp256k1::schnorr::Signature::from_slice(signature).map_err(|e| AuthorityError::Authorization(e.to_string()))?;
    Secp256k1::verification_only()
        .verify_schnorr(&signature, &Message::from_digest(digest), &key(public)?)
        .map_err(|e| AuthorityError::Authorization(e.to_string()))
}
fn key(public: Hash32) -> Result<XOnlyPublicKey> {
    XOnlyPublicKey::from_slice(&public).map_err(|e| AuthorityError::Invalid(e.to_string()))
}
pub(crate) fn session(error: impl std::fmt::Display) -> AuthorityError {
    AuthorityError::Session(error.to_string())
}
