// SPDX-License-Identifier: MIT
// Copyright (C) 2026 Myelin developers

//! Authenticated, durable validator messaging for Myelin sessions.
//!
//! The wire transport is mutually authenticated TLS. Every envelope is also
//! session-bound and Schnorr-signed, so authorization does not depend on TLS
//! termination topology. A receiver acknowledges only after [`NetworkStore`]
//! durably enqueues a fresh sequence number.

use prost::Message;
use secp256k1::{schnorr::Signature, Keypair, Message as SecpMessage, Secp256k1, SecretKey, XOnlyPublicKey};
use std::{collections::HashMap, net::SocketAddr, sync::Arc, time::SystemTime};
use tonic::{transport::Server, Request, Response, Status};

/// Generated gRPC protocol.
pub mod proto {
    tonic::include_proto!("myelin.session.network.v1");
}

pub use proto::{Envelope, PushResponse};

const PROTOCOL_VERSION: u32 = 1;
const SIGNATURE_DOMAIN: &[u8] = b"myelin:session-network-envelope";
const MESSAGE_ID_DOMAIN: &[u8] = b"myelin:session-network-message-id";
const MAX_PEERS: usize = 4_096;
const MAX_PEER_ID_BYTES: usize = 128;
const MAX_CONFIGURED_PAYLOAD_BYTES: usize = 64 * 1024 * 1024;
const MAX_ENVELOPE_OVERHEAD_BYTES: usize = 64 * 1024;
const MAX_OUTBOUND_SEQUENCE_RETRIES: usize = 16;

/// Transport-level message class. Consensus-specific phases remain inside
/// the registered module payload and type tag.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum MessageClass {
    /// Module-owned proposal, vote, or other consensus driver message.
    Consensus,
    /// Candidate block header and commitments.
    BlockCandidate,
    /// PoA/committee/Tendermint finality proof.
    FinalityProof,
    /// Canonical transaction gossip.
    Transaction,
    /// Bounded chain synchronization request.
    SyncRequest,
    /// Bounded chain synchronization response.
    SyncResponse,
}

impl MessageClass {
    /// Stable signed wire name.
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Consensus => "consensus",
            Self::BlockCandidate => "block-candidate",
            Self::FinalityProof => "finality-proof",
            Self::Transaction => "transaction",
            Self::SyncRequest => "sync-request",
            Self::SyncResponse => "sync-response",
        }
    }
}

impl TryFrom<&str> for MessageClass {
    type Error = NetworkError;

    fn try_from(value: &str) -> Result<Self, Self::Error> {
        match value {
            "consensus" => Ok(Self::Consensus),
            "block-candidate" => Ok(Self::BlockCandidate),
            "finality-proof" => Ok(Self::FinalityProof),
            "transaction" => Ok(Self::Transaction),
            "sync-request" => Ok(Self::SyncRequest),
            "sync-response" => Ok(Self::SyncResponse),
            other => Err(NetworkError::InvalidEnvelope(format!("unknown message kind {other}"))),
        }
    }
}

/// Module-neutral message type bound by the signed envelope.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct MessageType {
    /// Transport-level class.
    pub class: MessageClass,
    /// Owner-defined message codec version.
    pub module_message_version: u32,
    /// Owner-defined type tag interpreted only by the registered module.
    pub type_tag: u32,
}

impl MessageType {
    /// Construct a versioned module-neutral type.
    pub const fn new(class: MessageClass, module_message_version: u32, type_tag: u32) -> Self {
        Self { class, module_message_version, type_tag }
    }
}

/// Immutable session/module binding used by outbound signing.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct NetworkBinding {
    /// Exact session identifier.
    pub session_id: [u8; 32],
    /// Exact module descriptor/config commitment from session genesis.
    pub consensus_module_commitment: [u8; 32],
}

impl NetworkBinding {
    fn validate(&self) -> Result<(), NetworkError> {
        if self.session_id == [0; 32] || self.consensus_module_commitment == [0; 32] {
            return Err(NetworkError::InvalidConfig("network session and consensus module commitments must not be zero".to_owned()));
        }
        Ok(())
    }
}

/// One authorized validator peer.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct NetworkPeer {
    /// Stable signer identifier.
    pub id: String,
    /// X-only secp256k1 Schnorr public key.
    pub public_key: [u8; 32],
}

/// Immutable authorization and resource policy for a network endpoint.
#[derive(Clone, Debug)]
pub struct NetworkConfig {
    /// Exact session accepted by this endpoint.
    pub session_id: [u8; 32],
    /// Exact consensus module descriptor/config commitment from genesis.
    pub consensus_module_commitment: [u8; 32],
    /// Exact owner-defined consensus message format accepted by this endpoint.
    pub module_message_version: u32,
    /// Exact recipient id accepted by this endpoint.
    pub local_peer_id: String,
    /// Closed set of authorized message signers.
    pub peers: Vec<NetworkPeer>,
    /// Maximum application payload size.
    pub max_payload_bytes: usize,
    /// Maximum accepted age for a newly observed message.
    pub max_message_age_ms: u64,
    /// Maximum future clock drift accepted from a peer.
    pub max_future_drift_ms: u64,
}

impl NetworkConfig {
    /// Validate peer ids, keys, and finite resource bounds.
    pub fn validate(&self) -> Result<(), NetworkError> {
        NetworkBinding { session_id: self.session_id, consensus_module_commitment: self.consensus_module_commitment }.validate()?;
        if self.module_message_version == 0 {
            return Err(NetworkError::InvalidConfig("module_message_version must not be zero".to_owned()));
        }
        if self.local_peer_id.is_empty() || self.local_peer_id.len() > MAX_PEER_ID_BYTES {
            return Err(NetworkError::InvalidConfig("local peer id length is invalid".to_owned()));
        }
        if self.peers.is_empty() || self.peers.len() > MAX_PEERS {
            return Err(NetworkError::InvalidConfig(format!("peer count must be 1..={MAX_PEERS}")));
        }
        if self.max_payload_bytes == 0 || self.max_payload_bytes > MAX_CONFIGURED_PAYLOAD_BYTES {
            return Err(NetworkError::InvalidConfig(format!("max_payload_bytes must be 1..={MAX_CONFIGURED_PAYLOAD_BYTES}")));
        }
        let mut ids = std::collections::HashSet::new();
        let mut keys = std::collections::HashSet::new();
        for peer in &self.peers {
            if peer.id.is_empty() || peer.id.len() > MAX_PEER_ID_BYTES || !ids.insert(peer.id.as_str()) {
                return Err(NetworkError::InvalidConfig("peer ids must be non-empty and unique".to_owned()));
            }
            XOnlyPublicKey::from_slice(&peer.public_key)
                .map_err(|error| NetworkError::InvalidConfig(format!("peer {} has invalid public key: {error}", peer.id)))?;
            if !keys.insert(peer.public_key) {
                return Err(NetworkError::InvalidConfig("peer public keys must be unique".to_owned()));
            }
        }
        Ok(())
    }

    /// Signed session/module binding for outbound adapters.
    pub const fn binding(&self) -> NetworkBinding {
        NetworkBinding { session_id: self.session_id, consensus_module_commitment: self.consensus_module_commitment }
    }
}

/// Local envelope signing capability. Secret bytes never enter protocol config.
#[derive(Clone)]
pub struct NetworkSigner {
    signer_id: String,
    secret_key: SecretKey,
}

impl NetworkSigner {
    /// Construct a signer from an explicit validator id and secret key.
    pub fn new(signer_id: impl Into<String>, secret_key: [u8; 32]) -> Result<Self, NetworkError> {
        let signer_id = signer_id.into();
        if signer_id.is_empty() {
            return Err(NetworkError::InvalidConfig("signer id must not be empty".to_owned()));
        }
        let secret_key = SecretKey::from_slice(&secret_key)
            .map_err(|error| NetworkError::InvalidConfig(format!("invalid network secret key: {error}")))?;
        Ok(Self { signer_id, secret_key })
    }

    /// Stable signer id carried by envelopes.
    pub fn signer_id(&self) -> &str {
        &self.signer_id
    }

    /// X-only public key for peer configuration.
    pub fn public_key(&self) -> [u8; 32] {
        let secp = Secp256k1::new();
        let keypair = Keypair::from_secret_key(&secp, &self.secret_key);
        XOnlyPublicKey::from_keypair(&keypair).0.serialize()
    }

    /// Sign an envelope with an already durably reserved sequence number.
    pub fn sign(
        &self,
        binding: NetworkBinding,
        recipient_id: impl Into<String>,
        sequence: u64,
        timestamp_ms: u64,
        message_type: MessageType,
        payload: Vec<u8>,
    ) -> Result<Envelope, NetworkError> {
        binding.validate()?;
        if message_type.module_message_version == 0 {
            return Err(NetworkError::InvalidConfig("module message version must not be zero".to_owned()));
        }
        let payload_hash = blake3::hash(&payload).as_bytes().to_vec();
        let mut envelope = Envelope {
            protocol_version: PROTOCOL_VERSION,
            session_id: binding.session_id.to_vec(),
            sender_id: self.signer_id.clone(),
            recipient_id: recipient_id.into(),
            sequence,
            timestamp_ms,
            message_class: message_type.class.as_str().to_owned(),
            payload,
            signature: Vec::new(),
            consensus_module_commitment: binding.consensus_module_commitment.to_vec(),
            module_message_version: message_type.module_message_version,
            message_type_tag: message_type.type_tag,
            payload_hash,
        };
        let secp = Secp256k1::new();
        let keypair = Keypair::from_secret_key(&secp, &self.secret_key);
        envelope.signature =
            secp.sign_schnorr_no_aux_rand(&SecpMessage::from_digest(signature_digest(&envelope)), &keypair).serialize().to_vec();
        Ok(envelope)
    }
}

/// Durable inbound or outbound network item.
#[derive(Clone, Debug, PartialEq)]
pub struct DurableEnvelope {
    /// Hash of the complete signed envelope.
    pub message_id: [u8; 32],
    /// Signed protocol envelope.
    pub envelope: Envelope,
}

impl DurableEnvelope {
    /// Strictly decode the known protobuf envelope and rederive its message id.
    pub fn decode(bytes: &[u8]) -> Result<Self, NetworkError> {
        let envelope = Envelope::decode(bytes).map_err(|error| NetworkError::Codec(error.to_string()))?;
        if envelope.encode_to_vec() != bytes {
            return Err(NetworkError::Codec("network envelope encoding is not canonical or contains unknown fields".to_owned()));
        }
        validate_envelope_integrity(&envelope)?;
        Ok(Self::new(envelope))
    }

    /// Encode the complete signed envelope.
    pub fn encode(&self) -> Vec<u8> {
        self.envelope.encode_to_vec()
    }

    /// Build a durable wrapper and derive its id.
    pub fn new(envelope: Envelope) -> Self {
        let message_id = message_id(&envelope);
        Self { message_id, envelope }
    }

    /// Revalidate canonical envelope integrity before durable storage or
    /// delivery. Authentication remains the receiver's responsibility.
    pub fn validate(&self) -> Result<(), NetworkError> {
        validate_envelope_integrity(&self.envelope)?;
        if self.message_id != message_id(&self.envelope) {
            return Err(NetworkError::Codec("durable envelope message id mismatch".to_owned()));
        }
        Ok(())
    }
}

/// Result of atomically applying the inbound replay check and enqueue.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum EnqueueStatus {
    /// Sequence was new and the message is durably queued.
    Enqueued,
    /// The exact already-observed message was retried.
    Duplicate,
}

/// Durable network queue and replay boundary.
pub trait NetworkStore: Send + Sync + 'static {
    /// Read the next candidate local sequence. `enqueue_outbound` atomically
    /// claims it, so callers must retry a concurrent claim conflict.
    fn reserve_outbound_sequence(&self, session_id: [u8; 32], sender_id: &str, recipient_id: &str) -> Result<u64, NetworkStoreError>;
    /// Persist a signed outbound envelope before attempting transmission.
    fn enqueue_outbound(&self, message: &DurableEnvelope) -> Result<(), NetworkStoreError>;
    /// Load a bounded retry batch.
    fn pending_outbound(
        &self,
        session_id: [u8; 32],
        recipient_id: &str,
        limit: usize,
    ) -> Result<Vec<DurableEnvelope>, NetworkStoreError>;
    /// Remove an outbound item after an authenticated receiver acknowledges it.
    fn acknowledge_outbound(&self, session_id: [u8; 32], message_id: [u8; 32]) -> Result<(), NetworkStoreError>;
    /// Atomically reject replays and enqueue a fresh authenticated message.
    fn enqueue_inbound(&self, message: &DurableEnvelope) -> Result<EnqueueStatus, NetworkStoreError>;
    /// Load a bounded deterministic processing batch.
    fn pending_inbound(&self, session_id: [u8; 32], limit: usize) -> Result<Vec<DurableEnvelope>, NetworkStoreError>;
    /// Remove an inbound message after idempotent application handling.
    fn acknowledge_inbound(&self, session_id: [u8; 32], message_id: [u8; 32]) -> Result<(), NetworkStoreError>;
}

/// Storage failures surfaced by network queues.
#[derive(Clone, Debug, PartialEq, Eq, thiserror::Error)]
pub enum NetworkStoreError {
    /// Sequence or queue CAS conflict.
    #[error("network store conflict: {0}")]
    Conflict(String),
    /// Requested queued message does not exist.
    #[error("network record not found: {0}")]
    NotFound(String),
    /// Durable network bytes are corrupt.
    #[error("corrupt network record: {0}")]
    Corrupt(String),
    /// Backend I/O/database failure.
    #[error("network store backend failure: {0}")]
    Backend(String),
}

/// Time source used to make authentication tests deterministic.
pub trait Clock: Send + Sync + 'static {
    /// Current Unix epoch time in milliseconds.
    fn now_ms(&self) -> Result<u64, NetworkError>;
}

/// System wall clock.
#[derive(Clone, Copy, Debug, Default)]
pub struct SystemClock;

impl Clock for SystemClock {
    fn now_ms(&self) -> Result<u64, NetworkError> {
        let duration =
            SystemTime::now().duration_since(SystemTime::UNIX_EPOCH).map_err(|error| NetworkError::Clock(error.to_string()))?;
        u64::try_from(duration.as_millis()).map_err(|_| NetworkError::Clock("Unix timestamp exceeds u64".to_owned()))
    }
}

/// Signature, authorization, time, and resource-bound verifier.
pub struct NetworkAuthenticator<C: Clock> {
    config: NetworkConfig,
    peers: HashMap<String, [u8; 32]>,
    clock: C,
}

impl<C: Clock> NetworkAuthenticator<C> {
    /// Construct from a validated closed-peer config.
    pub fn new(config: NetworkConfig, clock: C) -> Result<Self, NetworkError> {
        config.validate()?;
        let peers = config.peers.iter().map(|peer| (peer.id.clone(), peer.public_key)).collect();
        Ok(Self { config, peers, clock })
    }

    /// Verify an envelope without changing replay state.
    pub fn verify(&self, envelope: Envelope) -> Result<DurableEnvelope, NetworkError> {
        validate_envelope_integrity(&envelope)?;
        if envelope.protocol_version != PROTOCOL_VERSION {
            return Err(NetworkError::InvalidEnvelope(format!("unsupported protocol version {}", envelope.protocol_version)));
        }
        if envelope.session_id.as_slice() != self.config.session_id {
            return Err(NetworkError::Unauthorized("wrong session id".to_owned()));
        }
        if envelope.consensus_module_commitment.as_slice() != self.config.consensus_module_commitment {
            return Err(NetworkError::Unauthorized("wrong consensus module commitment".to_owned()));
        }
        if envelope.module_message_version != self.config.module_message_version {
            return Err(NetworkError::InvalidEnvelope(format!(
                "unsupported module message version {}",
                envelope.module_message_version
            )));
        }
        if envelope.recipient_id != self.config.local_peer_id {
            return Err(NetworkError::Unauthorized("wrong recipient id".to_owned()));
        }
        if envelope.payload.len() > self.config.max_payload_bytes {
            return Err(NetworkError::ResourceLimit(format!(
                "payload {} exceeds {} bytes",
                envelope.payload.len(),
                self.config.max_payload_bytes
            )));
        }
        MessageClass::try_from(envelope.message_class.as_str())?;
        let public_key = self
            .peers
            .get(&envelope.sender_id)
            .ok_or_else(|| NetworkError::Unauthorized(format!("unknown sender {}", envelope.sender_id)))?;
        let now = self.clock.now_ms()?;
        if envelope.timestamp_ms > now.saturating_add(self.config.max_future_drift_ms) {
            return Err(NetworkError::InvalidEnvelope("message timestamp is too far in the future".to_owned()));
        }
        if now.saturating_sub(envelope.timestamp_ms) > self.config.max_message_age_ms {
            return Err(NetworkError::InvalidEnvelope("message timestamp is too old".to_owned()));
        }
        let signature = Signature::from_slice(&envelope.signature)
            .map_err(|_| NetworkError::InvalidEnvelope("signature must be exactly 64 bytes".to_owned()))?;
        let public_key = XOnlyPublicKey::from_slice(public_key)
            .map_err(|error| NetworkError::InvalidConfig(format!("invalid configured peer key: {error}")))?;
        Secp256k1::verification_only()
            .verify_schnorr(&signature, &SecpMessage::from_digest(signature_digest(&envelope)), &public_key)
            .map_err(|_| NetworkError::Unauthorized("invalid envelope signature".to_owned()))?;
        Ok(DurableEnvelope::new(envelope))
    }
}

/// Build, sign, and durably queue one outbound message.
pub fn queue_outbound<S: NetworkStore, C: Clock>(
    store: &S,
    clock: &C,
    signer: &NetworkSigner,
    binding: NetworkBinding,
    recipient_id: &str,
    message_type: MessageType,
    payload: Vec<u8>,
) -> Result<DurableEnvelope, NetworkError> {
    binding.validate()?;
    if recipient_id.is_empty() || recipient_id.len() > MAX_PEER_ID_BYTES {
        return Err(NetworkError::InvalidConfig("recipient id length is invalid".to_owned()));
    }
    if payload.len() > MAX_CONFIGURED_PAYLOAD_BYTES {
        return Err(NetworkError::ResourceLimit(format!(
            "outbound payload {} exceeds hard limit {} bytes",
            payload.len(),
            MAX_CONFIGURED_PAYLOAD_BYTES
        )));
    }
    for attempt in 0..MAX_OUTBOUND_SEQUENCE_RETRIES {
        let sequence = store.reserve_outbound_sequence(binding.session_id, signer.signer_id(), recipient_id)?;
        let message =
            DurableEnvelope::new(signer.sign(binding, recipient_id, sequence, clock.now_ms()?, message_type, payload.clone())?);
        match store.enqueue_outbound(&message) {
            Ok(()) => return Ok(message),
            Err(NetworkStoreError::Conflict(reason))
                if reason.starts_with("outbound sequence ") && attempt + 1 < MAX_OUTBOUND_SEQUENCE_RETRIES => {}
            Err(error) => return Err(NetworkError::Store(error)),
        }
    }
    Err(NetworkError::Store(NetworkStoreError::Conflict(format!(
        "outbound sequence remained contended after {MAX_OUTBOUND_SEQUENCE_RETRIES} attempts"
    ))))
}

/// gRPC receiver that acknowledges only after durable enqueue.
pub struct SessionNetworkService<S: NetworkStore, C: Clock> {
    authenticator: Arc<NetworkAuthenticator<C>>,
    store: Arc<S>,
}

impl<S: NetworkStore, C: Clock> SessionNetworkService<S, C> {
    /// Bind authentication policy and durable queue.
    pub fn new(authenticator: NetworkAuthenticator<C>, store: Arc<S>) -> Self {
        Self { authenticator: Arc::new(authenticator), store }
    }
}

#[tonic::async_trait]
impl<S: NetworkStore, C: Clock> proto::session_network_server::SessionNetwork for SessionNetworkService<S, C> {
    async fn push(&self, request: Request<Envelope>) -> Result<Response<PushResponse>, Status> {
        let message = self.authenticator.verify(request.into_inner()).map_err(network_status)?;
        let status = self.store.enqueue_inbound(&message).map_err(store_status)?;
        Ok(Response::new(PushResponse { message_id: message.message_id.to_vec(), duplicate: status == EnqueueStatus::Duplicate }))
    }
}

/// PEM material for a server that requires trusted client certificates.
#[derive(Clone, Debug)]
pub struct MutualTlsServerConfig {
    /// Server certificate chain in PEM.
    pub certificate_pem: Vec<u8>,
    /// Server private key in PEM.
    pub private_key_pem: Vec<u8>,
    /// CA roots allowed to authenticate clients.
    pub client_ca_pem: Vec<u8>,
}

/// Run the concrete gRPC receiver with mandatory mutual TLS.
pub async fn serve_mtls<S: NetworkStore, C: Clock>(
    address: SocketAddr,
    service: SessionNetworkService<S, C>,
    tls: MutualTlsServerConfig,
) -> Result<(), NetworkError> {
    use tonic::transport::{Certificate, Identity, ServerTlsConfig};
    let identity = Identity::from_pem(tls.certificate_pem, tls.private_key_pem);
    let client_ca = Certificate::from_pem(tls.client_ca_pem);
    let max_decoding_message_size = service.authenticator.config.max_payload_bytes.saturating_add(MAX_ENVELOPE_OVERHEAD_BYTES);
    Server::builder()
        .tls_config(ServerTlsConfig::new().identity(identity).client_ca_root(client_ca))
        .map_err(|error| NetworkError::Transport(error.to_string()))?
        .add_service(
            proto::session_network_server::SessionNetworkServer::new(service).max_decoding_message_size(max_decoding_message_size),
        )
        .serve(address)
        .await
        .map_err(|error| NetworkError::Transport(error.to_string()))
}

/// PEM material for an mTLS validator client.
#[derive(Clone, Debug)]
pub struct MutualTlsClientConfig {
    /// CA roots used to authenticate the server.
    pub server_ca_pem: Vec<u8>,
    /// Client certificate chain in PEM.
    pub certificate_pem: Vec<u8>,
    /// Client private key in PEM.
    pub private_key_pem: Vec<u8>,
    /// DNS name expected in the server certificate.
    pub domain_name: String,
}

/// Deliver one previously persisted envelope through mutual TLS.
pub async fn push_mtls(endpoint: String, tls: MutualTlsClientConfig, message: &DurableEnvelope) -> Result<PushResponse, NetworkError> {
    use tonic::transport::{Certificate, ClientTlsConfig, Endpoint, Identity};
    let tls = ClientTlsConfig::new()
        .ca_certificate(Certificate::from_pem(tls.server_ca_pem))
        .identity(Identity::from_pem(tls.certificate_pem, tls.private_key_pem))
        .domain_name(tls.domain_name);
    let channel = Endpoint::from_shared(endpoint)
        .map_err(|error| NetworkError::Transport(error.to_string()))?
        .tls_config(tls)
        .map_err(|error| NetworkError::Transport(error.to_string()))?
        .connect()
        .await
        .map_err(|error| NetworkError::Transport(error.to_string()))?;
    let mut client = proto::session_network_client::SessionNetworkClient::new(channel);
    let response =
        client.push(message.envelope.clone()).await.map_err(|error| NetworkError::Transport(error.to_string()))?.into_inner();
    if response.message_id.as_slice() != message.message_id {
        return Err(NetworkError::Transport("receiver acknowledged a different message id".to_owned()));
    }
    Ok(response)
}

/// Deliver a bounded recipient-specific retry batch over one mTLS connection.
///
/// Each item remains durable until the remote endpoint acknowledges the exact
/// signed message id. A transport failure stops the batch and leaves that item
/// and all later items available for retry.
pub async fn deliver_pending_mtls<S: NetworkStore>(
    store: &S,
    session_id: [u8; 32],
    recipient_id: &str,
    endpoint: String,
    tls: MutualTlsClientConfig,
    limit: usize,
) -> Result<usize, NetworkError> {
    use tonic::transport::{Certificate, ClientTlsConfig, Endpoint, Identity};

    let pending = store.pending_outbound(session_id, recipient_id, limit)?;
    if pending.is_empty() {
        return Ok(0);
    }
    let tls = ClientTlsConfig::new()
        .ca_certificate(Certificate::from_pem(tls.server_ca_pem))
        .identity(Identity::from_pem(tls.certificate_pem, tls.private_key_pem))
        .domain_name(tls.domain_name);
    let channel = Endpoint::from_shared(endpoint)
        .map_err(|error| NetworkError::Transport(error.to_string()))?
        .tls_config(tls)
        .map_err(|error| NetworkError::Transport(error.to_string()))?
        .connect()
        .await
        .map_err(|error| NetworkError::Transport(error.to_string()))?;
    let mut client = proto::session_network_client::SessionNetworkClient::new(channel);
    let mut delivered = 0;
    for message in pending {
        if message.envelope.recipient_id != recipient_id {
            return Err(NetworkError::Store(NetworkStoreError::Corrupt(
                "recipient-specific outbound queue returned a different recipient".to_owned(),
            )));
        }
        let response =
            client.push(message.envelope.clone()).await.map_err(|error| NetworkError::Transport(error.to_string()))?.into_inner();
        if response.message_id.as_slice() != message.message_id {
            return Err(NetworkError::Transport("receiver acknowledged a different message id".to_owned()));
        }
        store.acknowledge_outbound(session_id, message.message_id)?;
        delivered += 1;
    }
    Ok(delivered)
}

/// Apply a bounded durable inbound batch and acknowledge only after the
/// caller's idempotent handler succeeds.
pub fn apply_pending_inbound<S, F>(store: &S, session_id: [u8; 32], limit: usize, mut apply: F) -> Result<usize, NetworkError>
where
    S: NetworkStore,
    F: FnMut(&DurableEnvelope) -> Result<(), NetworkError>,
{
    let pending = store.pending_inbound(session_id, limit)?;
    let mut applied = 0;
    for message in pending {
        apply(&message)?;
        store.acknowledge_inbound(session_id, message.message_id)?;
        applied += 1;
    }
    Ok(applied)
}

fn signature_digest(envelope: &Envelope) -> [u8; 32] {
    let mut hasher = blake3::Hasher::new();
    hasher.update(SIGNATURE_DOMAIN);
    hasher.update(&envelope.protocol_version.to_le_bytes());
    put_hash_field(&mut hasher, &envelope.session_id);
    put_field(&mut hasher, envelope.sender_id.as_bytes());
    put_field(&mut hasher, envelope.recipient_id.as_bytes());
    hasher.update(&envelope.sequence.to_le_bytes());
    hasher.update(&envelope.timestamp_ms.to_le_bytes());
    put_field(&mut hasher, envelope.message_class.as_bytes());
    put_hash_field(&mut hasher, &envelope.consensus_module_commitment);
    hasher.update(&envelope.module_message_version.to_le_bytes());
    hasher.update(&envelope.message_type_tag.to_le_bytes());
    put_hash_field(&mut hasher, &envelope.payload_hash);
    put_field(&mut hasher, &envelope.payload);
    *hasher.finalize().as_bytes()
}

fn validate_envelope_integrity(envelope: &Envelope) -> Result<(), NetworkError> {
    if envelope.session_id.len() != 32 {
        return Err(NetworkError::InvalidEnvelope("session id must be exactly 32 bytes".to_owned()));
    }
    if envelope.consensus_module_commitment.len() != 32 || envelope.consensus_module_commitment.iter().all(|byte| *byte == 0) {
        return Err(NetworkError::InvalidEnvelope("consensus module commitment must be one non-zero 32-byte hash".to_owned()));
    }
    if envelope.module_message_version == 0 {
        return Err(NetworkError::InvalidEnvelope("module message version must not be zero".to_owned()));
    }
    if envelope.payload_hash.len() != 32 || envelope.payload_hash.as_slice() != blake3::hash(&envelope.payload).as_bytes() {
        return Err(NetworkError::InvalidEnvelope("payload hash mismatch".to_owned()));
    }
    MessageClass::try_from(envelope.message_class.as_str())?;
    Ok(())
}

fn message_id(envelope: &Envelope) -> [u8; 32] {
    let mut hasher = blake3::Hasher::new();
    hasher.update(MESSAGE_ID_DOMAIN);
    hasher.update(&signature_digest(envelope));
    put_field(&mut hasher, &envelope.signature);
    *hasher.finalize().as_bytes()
}

fn put_hash_field(hasher: &mut blake3::Hasher, value: &[u8]) {
    hasher.update(&(value.len() as u32).to_le_bytes());
    hasher.update(value);
}

fn put_field(hasher: &mut blake3::Hasher, value: &[u8]) {
    hasher.update(&(value.len() as u64).to_le_bytes());
    hasher.update(value);
}

fn network_status(error: NetworkError) -> Status {
    match error {
        NetworkError::Unauthorized(message) => Status::unauthenticated(message),
        NetworkError::ResourceLimit(message) => Status::resource_exhausted(message),
        other => Status::invalid_argument(other.to_string()),
    }
}

fn store_status(error: NetworkStoreError) -> Status {
    match error {
        NetworkStoreError::Conflict(message) => Status::already_exists(message),
        NetworkStoreError::NotFound(message) => Status::not_found(message),
        NetworkStoreError::Corrupt(message) => Status::data_loss(message),
        NetworkStoreError::Backend(message) => Status::unavailable(message),
    }
}

/// Network protocol failures.
#[derive(Debug, thiserror::Error)]
pub enum NetworkError {
    /// Invalid immutable networking policy.
    #[error("invalid network config: {0}")]
    InvalidConfig(String),
    /// Malformed, expired, or unsupported envelope.
    #[error("invalid network envelope: {0}")]
    InvalidEnvelope(String),
    /// Sender is not authorized for this session.
    #[error("unauthorized network envelope: {0}")]
    Unauthorized(String),
    /// Bounded input limit was exceeded.
    #[error("network resource limit: {0}")]
    ResourceLimit(String),
    /// Wall-clock source failed.
    #[error("network clock failed: {0}")]
    Clock(String),
    /// Durable envelope codec failed.
    #[error("network codec failed: {0}")]
    Codec(String),
    /// TLS/gRPC transport failed.
    #[error("network transport failed: {0}")]
    Transport(String),
    /// Durable queue failed.
    #[error(transparent)]
    Store(#[from] NetworkStoreError),
}

#[cfg(test)]
mod tests {
    use super::*;

    #[derive(Clone, Copy)]
    struct FixedClock(u64);

    impl Clock for FixedClock {
        fn now_ms(&self) -> Result<u64, NetworkError> {
            Ok(self.0)
        }
    }

    #[test]
    fn signature_binds_session_sequence_kind_and_payload() {
        let signer = NetworkSigner::new("alice", [1; 32]).unwrap();
        let binding = NetworkBinding { session_id: [2; 32], consensus_module_commitment: [9; 32] };
        let config = NetworkConfig {
            session_id: [2; 32],
            consensus_module_commitment: [9; 32],
            module_message_version: 1,
            local_peer_id: "bob".to_owned(),
            peers: vec![NetworkPeer { id: "alice".to_owned(), public_key: signer.public_key() }],
            max_payload_bytes: 100,
            max_message_age_ms: 1_000,
            max_future_drift_ms: 100,
        };
        let auth = NetworkAuthenticator::new(config, FixedClock(10_000)).unwrap();
        let envelope =
            signer.sign(binding, "bob", 7, 10_000, MessageType::new(MessageClass::FinalityProof, 1, 0), vec![3, 4]).unwrap();
        auth.verify(envelope.clone()).unwrap();
        let mut changed = envelope;
        changed.sequence = 8;
        assert!(matches!(auth.verify(changed), Err(NetworkError::Unauthorized(_))));
    }

    #[test]
    fn rejects_unknown_sender_old_message_and_oversized_payload() {
        let signer = NetworkSigner::new("alice", [1; 32]).unwrap();
        let outsider = NetworkSigner::new("mallory", [3; 32]).unwrap();
        let config = NetworkConfig {
            session_id: [2; 32],
            consensus_module_commitment: [9; 32],
            module_message_version: 1,
            local_peer_id: "bob".to_owned(),
            peers: vec![NetworkPeer { id: "alice".to_owned(), public_key: signer.public_key() }],
            max_payload_bytes: 2,
            max_message_age_ms: 10,
            max_future_drift_ms: 1,
        };
        let auth = NetworkAuthenticator::new(config, FixedClock(100)).unwrap();
        let binding = NetworkBinding { session_id: [2; 32], consensus_module_commitment: [9; 32] };
        assert!(matches!(
            auth.verify(outsider.sign(binding, "bob", 0, 100, MessageType::new(MessageClass::Transaction, 1, 0), vec![]).unwrap()),
            Err(NetworkError::Unauthorized(_))
        ));
        assert!(matches!(
            auth.verify(signer.sign(binding, "bob", 0, 80, MessageType::new(MessageClass::Transaction, 1, 0), vec![]).unwrap()),
            Err(NetworkError::InvalidEnvelope(_))
        ));
        assert!(matches!(
            auth.verify(signer.sign(binding, "bob", 0, 100, MessageType::new(MessageClass::Transaction, 1, 0), vec![0; 3]).unwrap()),
            Err(NetworkError::ResourceLimit(_))
        ));
        assert!(matches!(
            auth.verify(signer.sign(binding, "carol", 0, 100, MessageType::new(MessageClass::Transaction, 1, 0), vec![]).unwrap()),
            Err(NetworkError::Unauthorized(_))
        ));

        let wrong_binding = NetworkBinding { session_id: [2; 32], consensus_module_commitment: [8; 32] };
        assert!(matches!(
            auth.verify(signer.sign(wrong_binding, "bob", 0, 100, MessageType::new(MessageClass::Consensus, 1, 7), vec![]).unwrap()),
            Err(NetworkError::Unauthorized(_))
        ));
    }
}
