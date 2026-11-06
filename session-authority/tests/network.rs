#[path = "../../session-journal/tests/support/mod.rs"]
mod inventory;
use inventory::*;
use myelin_consensus::{Authority, CommitteeSigner, ConsensusConfig, ProofOfAuthorityConfig};
use myelin_session::{BlockRequest, SessionConfig, SessionHead, SessionStore, TransitionExecutor};
use myelin_session_authority::{network::*, *};
use myelin_session_journal::*;
#[cfg(feature = "test-fixture")]
use std::process::{Child, Stdio};
use std::{
    path::{Path, PathBuf},
    process::Command,
    sync::{
        atomic::{AtomicBool, Ordering},
        Arc,
    },
    time::Duration,
};
use tokio::{
    io::{AsyncReadExt, AsyncWriteExt},
    net::{TcpListener, TcpStream},
    sync::oneshot,
    task::JoinHandle,
};
#[path = "support/claims.rs"]
mod claims;

fn signer(n: u8) -> AuthoritySigner {
    AuthoritySigner::new([n; 32]).unwrap()
}
fn cfg() -> SessionConfig {
    config(&consensus().0, 1, 100)
}
fn open(root: &Path) -> SessionAuthority<Inventory> {
    let cfg = cfg();
    let finality = CommitteeSigner::new("journal-validator", [7; 32]).unwrap();
    let consensus = ConsensusConfig::proof_of_authority(ProofOfAuthorityConfig {
        authorities: vec![Authority { id: finality.validator_id().into(), public_key: finality.ckb_public_key() }],
    });
    let scope = CommandScope::from_config(&cfg);
    SessionAuthority::open(
        myelin_session_authority::CanonicalLedger::open(root.join("canonical")).unwrap(),
        root.join("epochs"),
        AuthorityConfig { session: cfg, consensus, administrator: signer(8).public_key(), effect_domain: [99; 32] },
        AuthorityKeys { finality, certificates: signer(9) },
        Inventory { scope: scope.clone(), balance: 100 },
        |bytes| Ok(Inventory { scope, balance: u64::from_le_bytes(bytes.try_into().unwrap()) }),
    )
    .unwrap()
}
#[derive(Clone)]
struct FixedClock;
impl Clock for FixedClock {
    fn now_ms(&self) -> std::result::Result<u64, myelin_session_network::NetworkError> {
        Ok(1000)
    }
}
fn openssl(root: &Path, args: &[&str]) {
    let output = Command::new("openssl")
        .args(args)
        .env("OPENSSL_CONF", root.join("openssl.cnf"))
        .current_dir(root)
        .output()
        .expect("network tests require openssl");
    assert!(output.status.success(), "openssl: {}", String::from_utf8_lossy(&output.stderr));
}
fn make_tls(root: &Path) {
    std::fs::create_dir_all(root).unwrap();
    // All fixture extensions are explicit below. A host config can otherwise
    // add a second basicConstraints extension and produce an invalid test CA.
    std::fs::write(root.join("openssl.cnf"), "[req]\ndistinguished_name=dn\n[dn]\n").unwrap();
    openssl(
        root,
        &[
            "req",
            "-x509",
            "-newkey",
            "ec",
            "-pkeyopt",
            "ec_paramgen_curve:P-256",
            "-nodes",
            "-keyout",
            "ca.key",
            "-out",
            "ca.pem",
            "-days",
            "2",
            "-subj",
            "/CN=Myelin ephemeral test CA",
            "-addext",
            "basicConstraints=critical,CA:TRUE",
        ],
    );
    for (name, usage) in [("server", "serverAuth"), ("client", "clientAuth")] {
        openssl(
            root,
            &[
                "req",
                "-newkey",
                "ec",
                "-pkeyopt",
                "ec_paramgen_curve:P-256",
                "-nodes",
                "-keyout",
                &format!("{name}.key"),
                "-out",
                &format!("{name}.csr"),
                "-subj",
                &format!("/CN={name}"),
            ],
        );
        std::fs::write(root.join(format!("{name}.ext")), format!("basicConstraints=critical,CA:FALSE\nkeyUsage=critical,digitalSignature\nextendedKeyUsage={usage}\nsubjectAltName=DNS:localhost\n")).unwrap();
        openssl(
            root,
            &[
                "x509",
                "-req",
                "-in",
                &format!("{name}.csr"),
                "-CA",
                "ca.pem",
                "-CAkey",
                "ca.key",
                "-CAcreateserial",
                "-out",
                &format!("{name}.pem"),
                "-days",
                "2",
                "-extfile",
                &format!("{name}.ext"),
            ],
        );
    }
}
fn server_tls(root: &Path) -> MutualTlsServerConfig {
    MutualTlsServerConfig {
        certificate_pem: std::fs::read(root.join("server.pem")).unwrap(),
        private_key_pem: std::fs::read(root.join("server.key")).unwrap(),
        client_ca_pem: std::fs::read(root.join("ca.pem")).unwrap(),
    }
}
fn client_tls(root: &Path) -> MutualTlsClientConfig {
    MutualTlsClientConfig {
        certificate_pem: std::fs::read(root.join("client.pem")).unwrap(),
        private_key_pem: std::fs::read(root.join("client.key")).unwrap(),
        server_ca_pem: std::fs::read(root.join("ca.pem")).unwrap(),
        domain_name: "localhost".into(),
    }
}
struct Server<E: TransitionExecutor = Inventory> {
    authority: Arc<SessionAuthority<E>>,
    endpoint: String,
    stop: Option<oneshot::Sender<()>>,
    job: JoinHandle<NetworkResult<()>>,
}
impl<E: TransitionExecutor> Drop for Server<E> {
    fn drop(&mut self) {
        if let Some(stop) = self.stop.take() {
            let _ = stop.send(());
        }
        self.job.abort();
    }
}
async fn start(root: &Path) -> Server {
    start_with_limits(root, NetworkLimits::default()).await
}
async fn start_with_limits(root: &Path, limits: NetworkLimits) -> Server {
    let authority = Arc::new(open(root));
    start_authority(root, authority, limits).await
}
async fn start_authority<E: TransitionExecutor>(root: &Path, authority: Arc<SessionAuthority<E>>, limits: NetworkLimits) -> Server<E> {
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let endpoint = format!("https://{}", listener.local_addr().unwrap());
    let service = AuthorityService::new(authority.clone(), FixedClock, limits).unwrap();
    let tls = server_tls(root);
    let (stop, shutdown) = oneshot::channel();
    let job = tokio::spawn(serve_mtls(listener, service, tls, async {
        let _ = shutdown.await;
    }));
    Server { authority, endpoint, stop: Some(stop), job }
}
async fn connect(endpoint: &str, root: &Path, policy: &FencingPolicy) -> AuthorityClient {
    AuthorityClient::connect(endpoint.into(), client_tls(root), policy.clone(), NetworkLimits::default(), Duration::from_secs(2))
        .await
        .unwrap()
}
async fn rotate(client: &mut AuthorityClient, owner: u8) -> WriterGrant {
    let observation = client.observe().await.unwrap();
    let epoch = observation.grant.as_ref().map_or(0, |g| g.epoch);
    let request = signer(8).authorize_grant(client.policy(), epoch, signer(owner).public_key(), &observation.head().unwrap()).unwrap();
    client.grant(request).await.unwrap()
}
fn request_at(head: &SessionHead, commands: Vec<OriginalCommand>) -> BlockRequest {
    BlockRequest {
        transactions: vec![],
        frame_input: myelin_session::FrameInput {
            commands: commands.iter().map(|c| c.identity.clone()).collect(),
            start_position: head.input_position,
            end_position: head.input_position + commands.len() as u64,
            logical_time_start: head.logical_time,
            logical_time_end: head.logical_time + 1,
            payload: serde_json::to_vec(&commands).unwrap(),
        },
        scheduler_commitment: [20; 32],
        data_commitments: vec![],
        timestamp_ms: head.timestamp_ms + 1,
        local_now_ms: u64::MAX,
        successor: None,
        handoffs: vec![],
        consume_handoffs: vec![],
        handoff_actions: vec![],
    }
}
fn begin(
    root: &Path,
    policy: &FencingPolicy,
    head: &SessionHead,
    grant: &WriterGrant,
    owner: u8,
) -> (CommandJournal, Reservation, FencedSubmission, BlockRequest, OriginalCommand) {
    let journal = CommandJournal::open_fenced(root.join("journal"), &cfg(), JournalLimits::default(), policy.clone()).unwrap();
    let original = command(&cfg(), 30, 25);
    journal.enqueue(original.clone(), &Auth).unwrap();
    let reserved =
        journal.reserve(head, 32, 100_000, &Auth, |c, _| BuildDecision::Ready(serde_json::to_vec(c).unwrap())).unwrap().unwrap();
    let request = request_at(head, vec![original.clone()]);
    let auth = signer(owner).authorize_commit(policy, grant, head, &request).unwrap();
    let binding = journal.begin_fenced_commit(reserved.token, policy.clone(), grant.clone(), auth, &request).unwrap();
    (journal, reserved, binding, request, original)
}
fn assert_unknown(journal: &CommandJournal, original: &OriginalCommand) {
    assert!(matches!(
        journal.entry(original.identity.id).unwrap().unwrap().status,
        EntryStatus::Reserved { phase: ReservationPhase::Unknown, .. }
    ));
}
fn copy_tree(source: &Path, target: &Path) {
    std::fs::create_dir_all(target).unwrap();
    for e in std::fs::read_dir(source).unwrap() {
        let e = e.unwrap();
        if e.file_type().unwrap().is_dir() {
            copy_tree(&e.path(), &target.join(e.file_name()));
        } else {
            std::fs::copy(e.path(), target.join(e.file_name())).unwrap();
        }
    }
}

// Forward client bytes while optionally blackholing server bytes. TLS remains
// end to end; no mocked success/error response or application-layer interception.
struct Proxy {
    endpoint: String,
    drop_responses: Arc<AtomicBool>,
    job: JoinHandle<()>,
    stop: Option<oneshot::Sender<()>>,
}
impl Drop for Proxy {
    fn drop(&mut self) {
        self.job.abort();
    }
}
impl Proxy {
    async fn disconnect(mut self) {
        self.stop.take().unwrap().send(()).unwrap();
        (&mut self.job).await.unwrap();
    }
}
async fn proxy(server: &str) -> Proxy {
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let endpoint = format!("https://{}", listener.local_addr().unwrap());
    let address = server.trim_start_matches("https://").to_owned();
    let drop_responses = Arc::new(AtomicBool::new(false));
    let drop_flag = drop_responses.clone();
    let (stop, mut shutdown) = oneshot::channel();
    let job = tokio::spawn(async move {
        let mut connections = tokio::task::JoinSet::new();
        loop {
            let (incoming, _) = tokio::select! {
                value = listener.accept() => value.unwrap(),
                _ = &mut shutdown => { connections.shutdown().await; return; }
            };
            let upstream = TcpStream::connect(&address).await.unwrap();
            let drop_flag = drop_flag.clone();
            connections.spawn(async move {
                let (mut client_read, mut client_write) = incoming.into_split();
                let (mut server_read, mut server_write) = upstream.into_split();
                let upload = tokio::io::copy(&mut client_read, &mut server_write);
                let download = async {
                    let mut bytes = [0u8; 8192];
                    loop {
                        let n = server_read.read(&mut bytes).await?;
                        if n == 0 {
                            break;
                        }
                        if !drop_flag.load(Ordering::SeqCst) {
                            client_write.write_all(&bytes[..n]).await?;
                        }
                    }
                    std::io::Result::Ok(())
                };
                tokio::select! { _ = upload => (), _ = download => () }
            });
        }
    });
    Proxy { endpoint, drop_responses, job, stop: Some(stop) }
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn lost_tls_response_recovers_original_result_and_rejected_retry_is_not_absence() {
    let dir = tempfile::tempdir().unwrap();
    let root = dir.path();
    make_tls(root);
    let initial = open(root);
    drop(initial);
    copy_tree(&root.join("canonical"), &root.join("old-worker"));
    copy_tree(&root.join("canonical"), &root.join("new-worker"));
    let (mut old_replica, old_store) = open_chain(&root.join("old-worker"));
    let (_new_replica, _new_store) = open_chain(&root.join("new-worker"));
    let server = start(root).await;
    let policy = server.authority.policy().clone();
    let mut admin = connect(&server.endpoint, root, &policy).await;
    let grant = rotate(&mut admin, 10).await;
    let proxy = proxy(&server.endpoint).await;
    let mut worker = connect(&proxy.endpoint, root, &policy).await;
    let head = worker.observe().await.unwrap().head().unwrap();
    let (journal, reservation, binding, request, original) = begin(root, &policy, &head, &grant, 10);
    journal.mark_unknown(reservation.token).unwrap();
    proxy.drop_responses.store(true, Ordering::SeqCst);
    assert!(matches!(worker.commit(binding.authorization.clone(), request.clone()).await, Err(ClientError::Transport(_))));
    assert_eq!(admin.observe().await.unwrap().head().unwrap().finalised_height, Some(0));
    assert_unknown(&journal, &original);
    drop(worker);
    drop(proxy);
    drop(journal);
    // Independent disk can make local progress with a deliberately provided TEST
    // finality key, but its conflicting block is not remotely canonical.
    commit(&mut old_replica, vec![command(&cfg(), 31, 50)]);
    let local = old_store.load_block(cfg().session_id, 0).unwrap();
    let mut reconnected = connect(&server.endpoint, root, &policy).await;
    let journal = CommandJournal::open_fenced(root.join("journal"), &cfg(), JournalLimits::default(), policy.clone()).unwrap();
    // Repeat the submitted bytes. Its old head is now rejected, yet reconciliation
    // recovers the *original* committed purchase, not a releasable reservation.
    let report = journal.submit_fenced_remote(reservation.token, &mut reconnected, request, result).await.unwrap();
    assert!(matches!(report.attempt, Err(ClientError::Remote(RemoteFailure { code: RemoteCode::Fenced, .. }))));
    assert!(matches!(report.entries[0].status, EntryStatus::Committed { .. }));
    assert_eq!(journal.acknowledge(original.identity.id).unwrap(), BusinessOutcome::Succeeded(75u64.to_le_bytes().to_vec()));
    let (record, receipt) = reconnected.block(0).await.unwrap();
    assert_ne!(local.block.hash(), receipt.block_hash);
    assert_eq!(record.state_snapshot, 75u64.to_le_bytes());
    assert!(reconnected.effect(0, local.outbox[0].id).await.is_err());
    let (certificate, message) = reconnected.effect(0, record.outbox[0].id).await.unwrap();
    let sink = CanonicalEffectStore::open(root.join("sink"), policy, 0u64.to_le_bytes().to_vec()).unwrap();
    assert_eq!(
        sink.apply(&certificate, &message, |state, _| Ok((u64::from_le_bytes(state.try_into().unwrap()) + 1).to_le_bytes().to_vec()))
            .unwrap(),
        EffectDisposition::Applied
    );
    assert_eq!(sink.apply(&certificate, &message, |_, _| panic!("duplicate payout")).unwrap(), EffectDisposition::Duplicate);
    assert_eq!(admin.observe().await.unwrap().head().unwrap().finalised_height, Some(0));
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn partition_before_submission_requires_rotation_then_same_id_and_fences_reconnecting_owner() {
    let dir = tempfile::tempdir().unwrap();
    let root = dir.path();
    make_tls(root);
    let server = start(root).await;
    let policy = server.authority.policy().clone();
    let mut admin = connect(&server.endpoint, root, &policy).await;
    let grant = rotate(&mut admin, 10).await;
    let proxy = proxy(&server.endpoint).await;
    let mut worker = connect(&proxy.endpoint, root, &policy).await;
    let head = worker.observe().await.unwrap().head().unwrap();
    let (journal, reserved, binding, request, original) = begin(root, &policy, &head, &grant, 10);
    proxy.disconnect().await; // Close listener and every established connection before sending.
    let result_error = journal.submit_fenced_remote(reserved.token, &mut worker, request.clone(), result).await;
    assert!(matches!(result_error, Err(RemoteJournalError::Rpc(ClientError::Transport(_)))));
    assert_unknown(&journal, &original);
    journal.reconcile_remote(reserved.token, &mut admin, result).await.unwrap();
    assert_unknown(&journal, &original); // Same-epoch Missing is not absence proof.
    let newer = rotate(&mut admin, 11).await;
    let recovered = journal.reconcile_remote(reserved.token, &mut admin, result).await.unwrap();
    assert_eq!(recovered[0].status, EntryStatus::Queued);
    assert_eq!(recovered[0].original, original);
    let mut old_reconnected = connect(&server.endpoint, root, &policy).await;
    assert!(matches!(
        old_reconnected.commit(binding.authorization, request).await,
        Err(ClientError::Remote(RemoteFailure { code: RemoteCode::Fenced, .. }))
    ));
    let reserved =
        journal.reserve(&head, 32, 100_000, &Auth, |c, _| BuildDecision::Ready(serde_json::to_vec(c).unwrap())).unwrap().unwrap();
    let request = request_at(&head, vec![original.clone()]);
    let authorization = signer(11).authorize_commit(&policy, &newer, &head, &request).unwrap();
    journal.begin_fenced_commit(reserved.token, policy, newer, authorization, &request).unwrap();
    let report = journal.submit_fenced_remote(reserved.token, &mut admin, request, result).await.unwrap();
    assert!(report.attempt.is_ok());
    assert_eq!(journal.acknowledge(original.identity.id).unwrap(), BusinessOutcome::Succeeded(75u64.to_le_bytes().to_vec()));
    assert_eq!(admin.observe().await.unwrap().head().unwrap().finalised_height, Some(0));
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn mutual_tls_pinned_policy_mutation_and_resource_limits_fail_without_progress() {
    let dir = tempfile::tempdir().unwrap();
    let root = dir.path();
    make_tls(root);
    let server = start(root).await;
    let policy = server.authority.policy().clone();
    let limits = NetworkLimits::default();
    let timeout = Duration::from_secs(2);
    let mut wrong_dns = client_tls(root);
    wrong_dns.domain_name = "wrong.invalid".into();
    assert!(AuthorityClient::connect(server.endpoint.clone(), wrong_dns, policy.clone(), limits, timeout).await.is_err());
    assert!(AuthorityClient::connect(server.endpoint.replace("https:", "http:"), client_tls(root), policy.clone(), limits, timeout)
        .await
        .is_err());
    let no_client = tonic::transport::Endpoint::from_shared(server.endpoint.clone())
        .unwrap()
        .tls_config(
            tonic::transport::ClientTlsConfig::new()
                .ca_certificate(tonic::transport::Certificate::from_pem(std::fs::read(root.join("ca.pem")).unwrap()))
                .domain_name("localhost"),
        )
        .unwrap()
        .connect()
        .await;
    if let Ok(channel) = no_client {
        let mut raw = proto::authority_gateway_client::AuthorityGatewayClient::new(channel);
        assert!(raw.call(proto::Packet { version: 1, body: vec![] }).await.is_err());
    }
    let mut wrong_policy = policy.clone();
    wrong_policy.certificate_key = signer(22).public_key();
    let mut wrong = connect(&server.endpoint, root, &wrong_policy).await;
    assert!(matches!(wrong.observe().await, Err(ClientError::Untrusted(_))));
    let mut client = connect(&server.endpoint, root, &policy).await;
    let head = client.observe().await.unwrap().head().unwrap();
    let invalid = signer(10).authorize_grant(&policy, 0, signer(10).public_key(), &head).unwrap();
    assert!(matches!(client.grant(invalid).await, Err(ClientError::Remote(RemoteFailure { code: RemoteCode::Authorization, .. }))));
    let grant = rotate(&mut client, 10).await;
    let mut request = request_at(&head, vec![command(&cfg(), 30, 25)]);
    let authorization = signer(10).authorize_commit(&policy, &grant, &head, &request).unwrap();
    request.timestamp_ms += 1;
    assert!(matches!(
        client.commit(authorization.clone(), request.clone()).await,
        Err(ClientError::Remote(RemoteFailure { code: RemoteCode::Authorization, .. }))
    ));
    let mut small = AuthorityClient::connect(
        server.endpoint.clone(),
        client_tls(root),
        policy.clone(),
        NetworkLimits { request_bytes: 1024, ..limits },
        timeout,
    )
    .await
    .unwrap();
    request.frame_input.payload = vec![0; 2048];
    assert!(matches!(small.commit(authorization, request).await, Err(ClientError::Local(_))));
    assert_eq!(client.observe().await.unwrap().head().unwrap(), head);
}

async fn raw_channel(
    endpoint: &str,
    tls: MutualTlsClientConfig,
) -> std::result::Result<tonic::transport::Channel, tonic::transport::Error> {
    tonic::transport::Endpoint::from_shared(endpoint.to_owned())
        .unwrap()
        .connect_timeout(Duration::from_secs(2))
        .timeout(Duration::from_secs(2))
        .tls_config(
            tonic::transport::ClientTlsConfig::new()
                .ca_certificate(tonic::transport::Certificate::from_pem(tls.server_ca_pem))
                .identity(tonic::transport::Identity::from_pem(tls.certificate_pem, tls.private_key_pem))
                .domain_name(tls.domain_name),
        )
        .unwrap()
        .connect()
        .await
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn server_requires_client_ca_and_enforces_wire_version_request_and_response_limits() {
    let dir = tempfile::tempdir().unwrap();
    let root = dir.path();
    make_tls(root);
    let server = start_with_limits(root, NetworkLimits { request_bytes: 4096, response_bytes: 1024, in_flight: 2 }).await;
    let policy = server.authority.policy().clone();
    let foreign = root.join("foreign");
    make_tls(&foreign);
    let mut tls = client_tls(&foreign);
    tls.server_ca_pem = std::fs::read(root.join("ca.pem")).unwrap();
    if let Ok(channel) = raw_channel(&server.endpoint, tls).await {
        let mut raw = proto::authority_gateway_client::AuthorityGatewayClient::new(channel);
        assert!(raw.call(proto::Packet { version: 99, body: vec![] }).await.is_err());
    }
    let channel = raw_channel(&server.endpoint, client_tls(root)).await.unwrap();
    let mut raw = proto::authority_gateway_client::AuthorityGatewayClient::new(channel);
    assert_eq!(raw.call(proto::Packet { version: 99, body: vec![] }).await.unwrap_err().code(), tonic::Code::InvalidArgument);
    assert!(raw.call(proto::Packet { version: 1, body: vec![0; 5000] }).await.is_err());
    let mut client = connect(&server.endpoint, root, &policy).await;
    let grant = rotate(&mut client, 10).await;
    let head = client.observe().await.unwrap().head().unwrap();
    let request = request_at(&head, vec![command(&cfg(), 30, 25)]);
    let auth = signer(10).authorize_commit(&policy, &grant, &head, &request).unwrap();
    client.commit(auth.clone(), request).await.unwrap();
    // The block and reconciliation contain more than this server's reply cap.
    assert!(matches!(client.block(0).await, Err(ClientError::Remote(RemoteFailure { code: RemoteCode::Invalid, .. }))));
    assert!(matches!(
        client.reconcile(auth, vec![command(&cfg(), 30, 25).identity]).await,
        Err(ClientError::Remote(RemoteFailure { code: RemoteCode::Invalid, .. }))
    ));
    assert_eq!(client.observe().await.unwrap().head().unwrap().finalised_height, Some(0));
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn incorrectly_mounted_plaintext_service_refuses_all_rpc_calls() {
    let dir = tempfile::tempdir().unwrap();
    let authority = Arc::new(open(dir.path()));
    let service = AuthorityService::new(authority.clone(), FixedClock, NetworkLimits::default()).unwrap();
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let endpoint = format!("http://{}", listener.local_addr().unwrap());
    let (stop, shutdown) = oneshot::channel();
    let server = tokio::spawn(
        tonic::transport::Server::builder()
            .add_service(proto::authority_gateway_server::AuthorityGatewayServer::new(service))
            .serve_with_incoming_shutdown(tokio_stream::wrappers::TcpListenerStream::new(listener), async {
                let _ = shutdown.await;
            }),
    );
    let mut raw = proto::authority_gateway_client::AuthorityGatewayClient::connect(endpoint).await.unwrap();
    let error = raw.call(proto::Packet { version: 99, body: vec![] }).await.unwrap_err();
    assert_eq!(error.code(), tonic::Code::Unauthenticated);
    assert!(authority.current_grant().unwrap().is_none());
    drop(raw);
    stop.send(()).unwrap();
    tokio::time::timeout(Duration::from_secs(3), server).await.unwrap().unwrap().unwrap();
}

#[derive(Clone)]
struct GatedInventory {
    inner: Inventory,
    gate: Arc<(std::sync::Mutex<bool>, std::sync::Condvar)>,
    entered: Arc<AtomicBool>,
}
impl TransitionExecutor for GatedInventory {
    fn state_root(&self) -> myelin_session::Hash32 {
        self.inner.state_root()
    }
    fn snapshot(&self) -> std::result::Result<Vec<u8>, String> {
        self.inner.snapshot()
    }
    fn execute_block(
        &mut self,
        height: u64,
        input: &myelin_session::FrameInput,
        handoffs: &[myelin_session::SessionHandoff],
        txs: &[Vec<u8>],
    ) -> std::result::Result<myelin_session::ExecutionOutcome, String> {
        self.entered.store(true, Ordering::SeqCst);
        let (lock, condition) = &*self.gate;
        let (held, _) = condition.wait_timeout_while(lock.lock().unwrap(), Duration::from_secs(10), |held| *held).unwrap();
        if *held {
            return Err("test did not release execution gate".into());
        }
        self.inner.execute_block(height, input, handoffs, txs)
    }
}
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn cancelled_rpc_keeps_durable_unknown_and_global_permit_until_execution_finishes() {
    let dir = tempfile::tempdir().unwrap();
    let root = dir.path();
    make_tls(root);
    let config = cfg();
    let (_, finality) = consensus();
    let consensus = ConsensusConfig::proof_of_authority(ProofOfAuthorityConfig {
        authorities: vec![Authority { id: finality.validator_id().into(), public_key: finality.ckb_public_key() }],
    });
    let gate = Arc::new((std::sync::Mutex::new(true), std::sync::Condvar::new()));
    let entered = Arc::new(AtomicBool::new(false));
    let executor = GatedInventory {
        inner: Inventory { scope: CommandScope::from_config(&config), balance: 100 },
        gate: gate.clone(),
        entered: entered.clone(),
    };
    let restore = executor.clone();
    let authority = Arc::new(
        SessionAuthority::open(
            myelin_session_authority::CanonicalLedger::open(root.join("canonical")).unwrap(),
            root.join("epochs"),
            AuthorityConfig { session: config, consensus, administrator: signer(8).public_key(), effect_domain: [99; 32] },
            AuthorityKeys { finality, certificates: signer(9) },
            executor,
            move |_| Ok(restore),
        )
        .unwrap(),
    );
    let policy = authority.policy().clone();
    let head = authority.head().unwrap();
    let grant = authority.grant(signer(8).authorize_grant(&policy, 0, signer(10).public_key(), &head).unwrap()).unwrap();
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let endpoint = format!("https://{}", listener.local_addr().unwrap());
    let service = AuthorityService::new(authority, FixedClock, NetworkLimits { in_flight: 1, ..NetworkLimits::default() }).unwrap();
    let (stop, shutdown) = oneshot::channel();
    let server = tokio::spawn(serve_mtls(listener, service, server_tls(root), async {
        let _ = shutdown.await;
    }));
    let mut worker = connect(&endpoint, root, &policy).await;
    let mut observer = connect(&endpoint, root, &policy).await;
    let (journal, reserved, _, request, original) = begin(root, &policy, &head, &grant, 10);
    let journal = Arc::new(journal);
    let submitted_journal = journal.clone();
    let submission =
        tokio::spawn(async move { submitted_journal.submit_fenced_remote(reserved.token, &mut worker, request, result).await });
    tokio::time::timeout(Duration::from_secs(3), async {
        while !entered.load(Ordering::SeqCst) {
            tokio::time::sleep(Duration::from_millis(10)).await;
        }
    })
    .await
    .unwrap();
    submission.abort();
    assert!(matches!(submission.await, Err(error) if error.is_cancelled()));
    assert_unknown(&journal, &original);
    assert!(matches!(observer.observe().await, Err(ClientError::Transport(_))));
    // Client cancellation cannot free the global admission slot while blocking
    // execution is still alive, nor authorize a second copy of the command.
    *gate.0.lock().unwrap() = false;
    gate.1.notify_all();
    tokio::time::timeout(Duration::from_secs(3), async {
        loop {
            if observer.observe().await.is_ok() {
                break;
            }
            tokio::time::sleep(Duration::from_millis(10)).await;
        }
    })
    .await
    .unwrap();
    journal.reconcile_remote(reserved.token, &mut observer, result).await.unwrap();
    assert_eq!(journal.acknowledge(original.identity.id).unwrap(), BusinessOutcome::Succeeded(75u64.to_le_bytes().to_vec()));
    drop(observer);
    stop.send(()).unwrap();
    tokio::time::timeout(Duration::from_secs(3), server).await.unwrap().unwrap().unwrap();
}

#[cfg(feature = "test-fixture")]
struct Process(Child);
#[cfg(feature = "test-fixture")]
impl Drop for Process {
    fn drop(&mut self) {
        let _ = self.0.kill();
        let _ = self.0.wait();
    }
}
#[cfg(feature = "test-fixture")]
async fn child(root: &Path, crash: &str) -> (Process, String) {
    let endpoint_file = root.join("endpoint");
    let _ = std::fs::remove_file(&endpoint_file);
    let process = Command::new(std::env::current_exe().unwrap())
        .args(["--exact", "authority_network_child", "--nocapture"])
        .env("MYELIN_NETWORK_CHILD_ROOT", root)
        .env("MYELIN_AUTHORITY_CRASH_POINT", crash)
        .stdout(Stdio::null())
        .stderr(Stdio::inherit())
        .spawn()
        .unwrap();
    let mut process = Process(process);
    for _ in 0..200 {
        assert!(process.0.try_wait().unwrap().is_none(), "authority child stopped before listen");
        if let Ok(endpoint) = std::fs::read_to_string(&endpoint_file) {
            return (process, endpoint);
        }
        tokio::time::sleep(Duration::from_millis(25)).await;
    }
    panic!("authority did not bind within five seconds");
}
#[test]
fn authority_network_child() {
    let Ok(root) = std::env::var("MYELIN_NETWORK_CHILD_ROOT") else { return };
    let root = PathBuf::from(root);
    tokio::runtime::Runtime::new().unwrap().block_on(async {
        let authority = Arc::new(open(&root));
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let endpoint = format!("https://{}", listener.local_addr().unwrap());
        let service = AuthorityService::new(authority, FixedClock, NetworkLimits::default()).unwrap();
        std::fs::write(root.join("endpoint"), endpoint).unwrap();
        serve_mtls(listener, service, server_tls(&root), std::future::pending()).await.unwrap();
    });
}
#[cfg(feature = "test-fixture")]
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn real_authority_process_exit_before_commit_and_after_receipt_recovers_over_tls_twice() {
    for point in ["before-intent", "after-receipt"] {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path();
        make_tls(root);
        let authority = open(root);
        let policy = authority.policy().clone();
        drop(authority);
        let (mut process, endpoint) = child(root, point).await;
        let mut client = connect(&endpoint, root, &policy).await;
        let grant = rotate(&mut client, 10).await;
        let head = client.observe().await.unwrap().head().unwrap();
        let (journal, reserved, binding, request, original) = begin(root, &policy, &head, &grant, 10);
        assert!(journal.submit_fenced_remote(reserved.token, &mut client, request.clone(), result).await.is_err());
        assert_unknown(&journal, &original);
        drop(client);
        let status = tokio::time::timeout(Duration::from_secs(5), async {
            loop {
                if let Some(status) = process.0.try_wait().unwrap() {
                    break status;
                }
                tokio::time::sleep(Duration::from_millis(10)).await;
            }
        })
        .await
        .expect("crash child did not terminate");
        assert_eq!(status.code(), Some(74));
        drop(process);
        for pass in 0..2 {
            let (process, endpoint) = child(root, "none").await;
            let mut client = connect(&endpoint, root, &policy).await;
            if pass == 0 {
                let entries = journal.reconcile_remote(reserved.token, &mut client, result).await.unwrap();
                if point == "before-intent" {
                    assert_unknown(&journal, &original);
                    let newer = rotate(&mut client, 11).await;
                    journal.reconcile_remote(reserved.token, &mut client, result).await.unwrap();
                    let next = journal
                        .reserve(&head, 32, 100_000, &Auth, |c, _| BuildDecision::Ready(serde_json::to_vec(c).unwrap()))
                        .unwrap()
                        .unwrap();
                    let auth = signer(11).authorize_commit(&policy, &newer, &head, &request).unwrap();
                    journal.begin_fenced_commit(next.token, policy.clone(), newer, auth, &request).unwrap();
                    journal.submit_fenced_remote(next.token, &mut client, request.clone(), result).await.unwrap();
                } else {
                    assert!(matches!(entries[0].status, EntryStatus::Committed { .. }));
                }
                journal.acknowledge(original.identity.id).unwrap();
            }
            let (record, _) = client.block(0).await.unwrap();
            assert_eq!(record.state_snapshot, 75u64.to_le_bytes());
            assert_eq!(client.observe().await.unwrap().head().unwrap().finalised_height, Some(0));
            assert!(client.commit(binding.authorization.clone(), request.clone()).await.is_err());
            drop(client);
            drop(process);
        }
    }
}
