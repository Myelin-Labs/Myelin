//! Bounded mTLS authority RPC. TLS authenticates transport peers; administrator
//! and worker signatures authorize mutations, and the pinned certificate key
//! authenticates every successful RPC response, including remote rejections.
//!
//! A transport error or rejected retry is never proof that an earlier submission
//! did not commit. Persist the exact submission first and reconcile its commands.
//! Cancelling a client future does not cancel an admitted blocking operation.
mod wire;
pub use wire::MAX_PACKET_BYTES;

use crate::{protocol, *};
use myelin_session::{bounded_codec, CommandIdentity};
pub use myelin_session_network::{Clock, MutualTlsClientConfig, MutualTlsServerConfig, SystemClock};
use serde::{Deserialize, Serialize};
use std::{future::Future, sync::Arc, time::Duration};
use tokio::{net::TcpListener, sync::Semaphore};
use tokio_stream::wrappers::TcpListenerStream;
use tonic::{transport, Request, Response, Status};
use wire::{Call, Operation, Outcome, Reply};

pub mod proto {
    tonic::include_proto!("myelin.session.authority.v1");
}

#[derive(Clone, Copy, Debug)]
pub struct NetworkLimits {
    pub request_bytes: usize,
    pub response_bytes: usize,
    /// Global admitted blocking operations, shared across connections.
    pub in_flight: usize,
}
impl Default for NetworkLimits {
    fn default() -> Self {
        Self { request_bytes: 8 * 1024 * 1024, response_bytes: 40 * 1024 * 1024, in_flight: 16 }
    }
}
impl NetworkLimits {
    fn validate(self) -> NetworkResult<()> {
        if !(1024..=MAX_PACKET_BYTES).contains(&self.request_bytes)
            || !(1024..=MAX_PACKET_BYTES).contains(&self.response_bytes)
            || !(1..=64).contains(&self.in_flight)
        {
            return Err(ClientError::Local("invalid authority network resource limits".into()));
        }
        Ok(())
    }
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum RemoteCode {
    Invalid,
    Authorization,
    Fenced,
    Session,
    Storage,
    Uncertain,
}
/// Applies to this RPC attempt only, never to every prior attempt of a command.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, thiserror::Error)]
#[serde(deny_unknown_fields)]
#[error("{code:?}: {message}")]
pub struct RemoteFailure {
    pub code: RemoteCode,
    #[serde(deserialize_with = "bounded_codec::bounded_string")]
    pub message: String,
}
impl From<AuthorityError> for RemoteFailure {
    fn from(error: AuthorityError) -> Self {
        let code = match &error {
            AuthorityError::Invalid(_) => RemoteCode::Invalid,
            AuthorityError::Authorization(_) => RemoteCode::Authorization,
            AuthorityError::Fenced(_) => RemoteCode::Fenced,
            AuthorityError::Session(_) | AuthorityError::ReplayPaused(_) => RemoteCode::Session,
            AuthorityError::Storage(_) => RemoteCode::Storage,
            AuthorityError::Uncertain(_) => RemoteCode::Uncertain,
        };
        Self { code, message: error.to_string().chars().take(512).collect() }
    }
}
#[derive(Debug, thiserror::Error)]
pub enum ClientError {
    #[error("local authority RPC validation failed before sending: {0}")]
    Local(String),
    #[error("authority transport failed; submitted outcome is unknown: {0}")]
    Transport(String),
    #[error("untrusted authority response; submitted outcome is unknown: {0}")]
    Untrusted(String),
    #[error("authority rejected this attempt (reconcile prior attempts): {0}")]
    Remote(RemoteFailure),
}
pub type NetworkResult<T> = std::result::Result<T, ClientError>;
fn transport_error(error: impl std::fmt::Display) -> ClientError {
    ClientError::Transport(error.to_string())
}
fn untrusted(error: impl std::fmt::Display) -> ClientError {
    ClientError::Untrusted(error.to_string())
}

/// Head and owner are observed under one authority lock. This is a fresh RPC
/// observation, not an archival absence proof or permission for a worker to sign.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Observation {
    #[serde(deserialize_with = "bounded_codec::bounded_bytes")]
    pub encoded_head: Vec<u8>,
    pub grant: Option<WriterGrant>,
}
impl Observation {
    pub fn head(&self) -> crate::Result<SessionHead> {
        SessionHead::decode(&self.encoded_head).map_err(protocol::session)
    }
    fn verify(&self, policy: &FencingPolicy) -> NetworkResult<()> {
        let head = self.head().map_err(untrusted)?;
        if head.session_id != policy.session_id {
            return Err(untrusted("observed foreign session"));
        }
        if let Some(grant) = &self.grant {
            grant.verify(policy).map_err(untrusted)?;
            if grant.first_height().map_err(untrusted)? > head.next_height().map_err(untrusted)? {
                return Err(untrusted("grant anchor is ahead of observed head"));
            }
        } else if head.finalised_height.is_some() {
            return Err(untrusted("canonical progress without a writer grant"));
        }
        Ok(())
    }
}

pub struct AuthorityService<E: TransitionExecutor, C: Clock> {
    authority: Arc<SessionAuthority<E>>,
    clock: Arc<C>,
    limits: NetworkLimits,
    admission: Arc<Semaphore>,
}
impl<E: TransitionExecutor, C: Clock> AuthorityService<E, C> {
    pub fn new(authority: Arc<SessionAuthority<E>>, clock: C, limits: NetworkLimits) -> NetworkResult<Self> {
        limits.validate()?;
        Ok(Self { authority, clock: Arc::new(clock), limits, admission: Arc::new(Semaphore::new(limits.in_flight)) })
    }
}

#[tonic::async_trait]
impl<E: TransitionExecutor, C: Clock> proto::authority_gateway_server::AuthorityGateway for AuthorityService<E, C> {
    async fn call(&self, request: Request<proto::Packet>) -> std::result::Result<Response<proto::Packet>, Status> {
        // Enforce TLS even if a host mounts the generated service incorrectly.
        if request.peer_certs().is_none_or(|certs| certs.is_empty()) {
            return Err(Status::unauthenticated("authority requires mutually authenticated TLS"));
        }
        let permit = self.admission.clone().try_acquire_owned().map_err(|_| Status::resource_exhausted("authority busy"))?;
        let authority = self.authority.clone();
        let clock = self.clock.clone();
        let limits = self.limits;
        let packet = request.into_inner();
        // Blocking execution/WAL work must not run on the transport reactor.
        // The permit lives with the job even when its RPC receiver disconnects.
        let packet = tokio::task::spawn_blocking(move || {
            let _permit = permit;
            handle(&authority, &*clock, packet, limits)
        })
        .await
        .map_err(|_| Status::unavailable("authority job interrupted; reconcile submission"))?
        .map_err(|status| *status)?;
        Ok(Response::new(packet))
    }
}

fn handle<E: TransitionExecutor, C: Clock>(
    authority: &SessionAuthority<E>,
    clock: &C,
    packet: proto::Packet,
    limits: NetworkLimits,
) -> std::result::Result<proto::Packet, Box<Status>> {
    let call: Call = wire::decode(&packet, limits.request_bytes).map_err(Status::invalid_argument)?;
    let digest = wire::request_digest(&packet.body);
    let outcome = if call.policy != authority.policy.commitment() || call.challenge == [0; 32] {
        Err(AuthorityError::Authorization("RPC policy or challenge mismatch".into()))
    } else {
        dispatch(authority, clock, call.operation)
    };
    let mut reply = Reply { request_digest: digest, outcome: outcome.map_err(RemoteFailure::from), signature: vec![] };
    // Reserve the maximum fixed signature overhead before hashing/encoding.
    if bounded_codec::size_with_limit(&reply, "authority_response_bytes", (limits.response_bytes - 128) as u64).is_err() {
        reply.outcome = Err(RemoteFailure {
            code: RemoteCode::Invalid,
            message: "response exceeds configured transport limit; query fewer commands or use an explicit larger limit".into(),
        });
    }
    let signing_digest = reply.digest().map_err(|e| Status::internal(e.to_string()))?;
    // Even an uncertain authority can describe the failed attempt, but it may
    // not perform further writes. This signature never asserts command absence.
    let inner = authority.inner.lock().map_err(|_| Status::unavailable("authority mutex poisoned"))?;
    reply.signature = inner.keys.certificates.sign(signing_digest);
    drop(inner);
    wire::encode(&reply, limits.response_bytes).map_err(|error| Box::new(Status::resource_exhausted(error)))
}

fn dispatch<E: TransitionExecutor, C: Clock>(authority: &SessionAuthority<E>, clock: &C, call: Operation) -> crate::Result<Outcome> {
    match call {
        Operation::Observe => {
            let inner = authority.locked()?;
            Ok(Outcome::Observation(Observation {
                encoded_head: inner.chain.head().encode().map_err(protocol::session)?,
                grant: inner.ownership.grants.last().cloned(),
            }))
        }
        Operation::Grant(request) => authority.grant(request).map(Outcome::Grant),
        Operation::Commit { authorization, candidate } => {
            let now = clock.now_ms().map_err(|e| AuthorityError::Invalid(e.to_string()))?;
            authority.commit(authorization, candidate.into_request(), now).map(Outcome::Commit)
        }
        Operation::Reconcile { authorization, commands } => {
            authority.reconcile_submission(&authorization, &commands).map(Outcome::Reconciliation)
        }
        Operation::Claims { ids, minimum_epoch } => authority.reconcile_claims(&ids, minimum_epoch).map(Outcome::Claims),
        Operation::Block { height } => {
            let record = authority.block(height)?.encode().map_err(protocol::session)?;
            Ok(Outcome::Block { record, receipt: authority.canonical_receipt(height)? })
        }
        Operation::Effect { height, message_id } => {
            let (certificate, message) = authority.effect_certificate(height, message_id)?;
            Ok(Outcome::Effect { certificate, message })
        }
    }
}

/// The caller binds the listener before starting the service. Shutdown stops
/// admission and drains existing RPCs; process recovery audits any interrupted
/// durable intent. Do not open another authority with copied keys/databases.
pub async fn serve_mtls<E: TransitionExecutor, C: Clock>(
    listener: TcpListener,
    service: AuthorityService<E, C>,
    tls: MutualTlsServerConfig,
    shutdown: impl Future<Output = ()> + Send + 'static,
) -> NetworkResult<()> {
    let limits = service.limits;
    transport::Server::builder()
        .tls_config(
            transport::ServerTlsConfig::new()
                .identity(transport::Identity::from_pem(tls.certificate_pem, tls.private_key_pem))
                .client_ca_root(transport::Certificate::from_pem(tls.client_ca_pem)),
        )
        .map_err(transport_error)?
        .concurrency_limit_per_connection(limits.in_flight)
        .add_service(
            proto::authority_gateway_server::AuthorityGatewayServer::new(service)
                .max_decoding_message_size(limits.request_bytes + 128)
                .max_encoding_message_size(limits.response_bytes + 128),
        )
        .serve_with_incoming_shutdown(TcpListenerStream::new(listener), shutdown)
        .await
        .map_err(transport_error)
}

#[derive(Clone)]
pub struct AuthorityClient {
    client: proto::authority_gateway_client::AuthorityGatewayClient<transport::Channel>,
    policy: FencingPolicy,
    limits: NetworkLimits,
    timeout: Duration,
}
impl AuthorityClient {
    pub async fn connect(
        endpoint: String,
        tls: MutualTlsClientConfig,
        policy: FencingPolicy,
        limits: NetworkLimits,
        timeout: Duration,
    ) -> NetworkResult<Self> {
        limits.validate()?;
        if !endpoint.starts_with("https://") || timeout.is_zero() || timeout > Duration::from_secs(300) {
            return Err(ClientError::Local("HTTPS and a finite nonzero deadline (<=300s) are required".into()));
        }
        let channel = transport::Endpoint::from_shared(endpoint)
            .map_err(|e| ClientError::Local(e.to_string()))?
            .connect_timeout(timeout)
            .timeout(timeout)
            .tls_config(
                transport::ClientTlsConfig::new()
                    .ca_certificate(transport::Certificate::from_pem(tls.server_ca_pem))
                    .identity(transport::Identity::from_pem(tls.certificate_pem, tls.private_key_pem))
                    .domain_name(tls.domain_name),
            )
            .map_err(|e| ClientError::Local(e.to_string()))?
            .connect()
            .await
            .map_err(transport_error)?;
        Ok(Self {
            client: proto::authority_gateway_client::AuthorityGatewayClient::new(channel)
                .max_decoding_message_size(limits.response_bytes + 128)
                .max_encoding_message_size(limits.request_bytes + 128),
            policy,
            limits,
            timeout,
        })
    }
    pub fn policy(&self) -> &FencingPolicy {
        &self.policy
    }
    async fn call(&mut self, operation: Operation) -> NetworkResult<Outcome> {
        let call = Call { policy: self.policy.commitment(), challenge: rand::random(), operation };
        let packet = wire::encode(&call, self.limits.request_bytes).map_err(ClientError::Local)?;
        let request_digest = wire::request_digest(&packet.body);
        let mut request = Request::new(packet);
        request.set_timeout(self.timeout);
        let packet = self.client.call(request).await.map_err(transport_error)?.into_inner();
        let reply: Reply = wire::decode(&packet, self.limits.response_bytes).map_err(ClientError::Untrusted)?;
        reply.verify(&self.policy, request_digest)?;
        reply.outcome.map_err(ClientError::Remote)
    }
    pub async fn observe(&mut self) -> NetworkResult<Observation> {
        let Outcome::Observation(value) = self.call(Operation::Observe).await? else {
            return Err(untrusted("expected an authority observation"));
        };
        value.verify(&self.policy)?;
        Ok(value)
    }
    pub async fn grant(&mut self, request: GrantRequest) -> NetworkResult<WriterGrant> {
        let expected = request.clone();
        let Outcome::Grant(grant) = self.call(Operation::Grant(request)).await? else {
            return Err(untrusted("expected a writer grant"));
        };
        grant.verify(&self.policy).map_err(untrusted)?;
        if grant.request != expected {
            return Err(untrusted("grant belongs to a different request"));
        }
        Ok(grant)
    }
    /// Persist the signed submission in a fenced journal BEFORE this call.
    /// Neither a transport error nor a rejected retry permits releasing it.
    pub async fn commit(&mut self, authorization: CommitAuthorization, request: BlockRequest) -> NetworkResult<CanonicalReceipt> {
        let expected = authorization.clone();
        let candidate = Box::new(wire::Candidate::from_request(request)?);
        let Outcome::Commit(receipt) = self.call(Operation::Commit { authorization, candidate }).await? else {
            return Err(untrusted("expected a canonical commit receipt"));
        };
        receipt.verify(&self.policy).map_err(untrusted)?;
        let base = SessionHead::decode(&expected.base_head).map_err(untrusted)?;
        if receipt.epoch != expected.epoch
            || receipt.owner != expected.owner
            || receipt.height != base.next_height().map_err(untrusted)?
        {
            return Err(untrusted("commit receipt belongs to another submission"));
        }
        Ok(receipt)
    }
    pub async fn reconcile(
        &mut self,
        authorization: CommitAuthorization,
        commands: Vec<CommandIdentity>,
    ) -> NetworkResult<CommandReconciliation> {
        let Outcome::Reconciliation(certificate) =
            self.call(Operation::Reconcile { authorization: authorization.clone(), commands: commands.clone() }).await?
        else {
            return Err(untrusted("expected command reconciliation"));
        };
        certificate.verify(&self.policy, &authorization, &commands).map_err(untrusted)?;
        Ok(certificate)
    }
    /// Membership is authenticated; a restoring application must still audit
    /// the complete history/profile and replay or verify its chosen checkpoint.
    pub async fn block(&mut self, height: u64) -> NetworkResult<(FinalisedBlockRecord, CanonicalReceipt)> {
        let Outcome::Block { record, receipt } = self.call(Operation::Block { height }).await? else {
            return Err(untrusted("expected a canonical block"));
        };
        receipt.verify(&self.policy).map_err(untrusted)?;
        let record = FinalisedBlockRecord::decode(&record).map_err(untrusted)?;
        if receipt.height != height
            || record.block.number != height
            || record.block.session_id != self.policy.session_id
            || record.block.hash() != receipt.block_hash
            || record.block.state_root_after != receipt.state_root
        {
            return Err(untrusted("record differs from its canonical certificate"));
        }
        Ok((record, receipt))
    }
    pub async fn effect(&mut self, height: u64, message_id: Hash32) -> NetworkResult<(CanonicalEffect, OutboxMessage)> {
        let Outcome::Effect { certificate, message } = self.call(Operation::Effect { height, message_id }).await? else {
            return Err(untrusted("expected a canonical effect"));
        };
        certificate.verify(&self.policy, &message).map_err(untrusted)?;
        if certificate.receipt.height != height || message.id != message_id {
            return Err(untrusted("effect belongs to another request"));
        }
        Ok((certificate, message))
    }
    /// Query the issuer's canonical ledger after reconnect/worker migration.
    /// Missing/Issued retain uncertainty; neither grants cancellation or refund.
    pub async fn reconcile_claims(&mut self, ids: Vec<Hash32>, minimum_epoch: u64) -> NetworkResult<ClaimReconciliation> {
        let Outcome::Claims(certificate) = self.call(Operation::Claims { ids: ids.clone(), minimum_epoch }).await? else {
            return Err(untrusted("expected claim reconciliation"));
        };
        certificate.verify(&self.policy, &ids, minimum_epoch).map_err(untrusted)?;
        Ok(certificate)
    }
}
