//! Controlled loopback provider: durable blobs, signed receipts and retrieval.
//! Development keys and injected faults; no public DA network/quorum claim.
use super::*;
use async_trait::async_trait;
use myelin_session_bundle::BlobId;
use myelin_session_runtime::{EvidenceAdapter, EvidenceAdapterError, EvidenceRuntime, EvidenceRuntimeError};
use secp256k1::{schnorr::Signature, Keypair, Message, Secp256k1, SecretKey};
use std::{
    collections::BTreeMap,
    fs::OpenOptions,
    net::{SocketAddr, TcpListener, TcpStream},
    process::{Child, Command, Stdio},
    result::Result,
    sync::{
        atomic::{AtomicBool, Ordering},
        LazyLock,
    },
    thread::JoinHandle,
};

const TOPIC: &str = "myelin/reference-transfer/v2";
const MAX_PACKET: usize = 16 * 1024;
const NETWORK_TIMEOUT: Duration = Duration::from_millis(250);

#[derive(Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub(super) enum Mode {
    Disabled,
    Slow,
    Offline,
}
pub(super) type Commits = Arc<Mutex<BTreeMap<Hash32, u64>>>;

fn key() -> Keypair {
    Keypair::from_secret_key(&Secp256k1::new(), &SecretKey::from_slice(&[5; 32]).unwrap())
}
fn provider_key() -> secp256k1::XOnlyPublicKey {
    static PUBLIC: LazyLock<secp256k1::XOnlyPublicKey> = LazyLock::new(|| key().x_only_public_key().0);
    *PUBLIC
}
fn descriptor() -> EvidencePipelineDescriptor {
    static DESCRIPTOR: LazyLock<EvidencePipelineDescriptor> = LazyLock::new(|| {
        EvidencePipelineDescriptor::new(
            "measurement/signed-blob-delivery",
            TOPIC,
            ["provider-stored", "retrieval-verified"]
                .into_iter()
                .map(|name| EvidenceStage {
                    name: name.into(),
                    verifier_commitment: hash(
                        &[b"measurement:provider-verifier:v1:".as_slice(), name.as_bytes(), &provider_key().serialize()].concat(),
                    ),
                })
                .collect(),
        )
        .unwrap()
    });
    DESCRIPTOR.clone()
}
#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct Request {
    message: OutboxMessage,
    stage: String,
    previous: Hash32,
}
#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct Receipt {
    message: Hash32,
    stage: String,
    previous: Hash32,
    payload_hash: Hash32,
    payload_bytes: u64,
    signature: Vec<u8>,
}
impl Receipt {
    fn digest(&self) -> Hash32 {
        hash(
            &[
                b"measurement:provider-receipt:v1:".as_slice(),
                &self.message,
                self.stage.as_bytes(),
                &self.previous,
                &self.payload_hash,
                &self.payload_bytes.to_le_bytes(),
            ]
            .concat(),
        )
    }
}
fn read_packet(stream: &mut TcpStream) -> AnyResult<Vec<u8>> {
    stream.set_read_timeout(Some(NETWORK_TIMEOUT))?;
    let mut header = [0; 4];
    stream.read_exact(&mut header)?;
    let size = u32::from_le_bytes(header) as usize;
    if size == 0 || size > MAX_PACKET {
        return Err("provider packet exceeds bound".into());
    }
    let mut bytes = vec![0; size];
    stream.read_exact(&mut bytes)?;
    Ok(bytes)
}
fn write_packet(stream: &mut TcpStream, bytes: &[u8]) -> AnyResult<()> {
    if bytes.is_empty() || bytes.len() > MAX_PACKET {
        return Err("provider packet exceeds bound".into());
    }
    stream.set_write_timeout(Some(NETWORK_TIMEOUT))?;
    stream.write_all(&(bytes.len() as u32).to_le_bytes())?;
    stream.write_all(bytes)?;
    Ok(())
}

pub(super) fn serve(root: &Path, mode: &str) -> AnyResult<serde_json::Value> {
    if !matches!(mode, "normal" | "slow" | "exit-after-store") {
        return Err("unknown provider mode".into());
    }
    let listener = TcpListener::bind("127.0.0.1:0")?;
    save(&root.join("endpoint.json"), &listener.local_addr()?)?;
    let mut log = OpenOptions::new().create(true).append(true).open(root.join("requests.jsonl"))?;
    for stream in listener.incoming() {
        let mut stream = stream?;
        let request: Request = serde_json::from_slice(&read_packet(&mut stream)?)?;
        request.message.validate()?;
        if request.message.topic != TOPIC || !descriptor().stages.iter().any(|s| s.name == request.stage) {
            return Err("provider request outside declared pipeline".into());
        }
        let started = Instant::now();
        if mode == "slow" {
            std::thread::sleep(Duration::from_millis(10));
        }
        let path = root.join(BlobId(request.message.commitment()).hex());
        let existed = path.exists();
        if request.stage == "provider-stored" && !existed {
            let mut file = OpenOptions::new().create_new(true).write(true).open(&path)?;
            file.write_all(&request.message.payload)?;
            file.sync_all()?;
            File::open(root)?.sync_all()?;
        }
        let payload = fs::read(path)?;
        if payload != request.message.payload {
            return Err("provider retrieval differs from committed payload".into());
        }
        let mut receipt = Receipt {
            message: request.message.commitment(),
            stage: request.stage,
            previous: request.previous,
            payload_hash: hash(&payload),
            payload_bytes: payload.len() as u64,
            signature: vec![],
        };
        receipt.signature =
            Secp256k1::new().sign_schnorr_no_aux_rand(&Message::from_digest(receipt.digest()), &key()).as_ref().to_vec();
        json_line(
            &mut log,
            &serde_json::json!({"message":BlobId(request.message.id),"stage":receipt.stage,
            "reused_durable_blob":existed,"provider_ns":ns(started.elapsed()),"mode":mode}),
        )?;
        log.sync_all()?;
        if mode == "exit-after-store" {
            std::process::exit(77);
        }
        write_packet(&mut stream, &serde_json::to_vec(&receipt)?)?;
    }
    Err("provider listener stopped".into())
}

struct Provider {
    child: Child,
    address: SocketAddr,
}
impl Provider {
    fn start(root: &Path, mode: &str) -> AnyResult<Self> {
        fs::create_dir_all(root)?;
        let ready = root.join("endpoint.json");
        if ready.exists() {
            fs::remove_file(&ready)?;
        }
        let log = OpenOptions::new().create(true).append(true).open(root.join("process.log"))?;
        let mut child = Command::new(std::env::current_exe()?)
            .arg("evidence-provider")
            .arg(root)
            .arg(mode)
            .stdin(Stdio::null())
            .stdout(log.try_clone()?)
            .stderr(log)
            .spawn()?;
        let start = Instant::now();
        loop {
            if ready.exists() {
                // Readiness may be observed while save() is still writing.
                if let Ok(address) = load(&ready) {
                    return Ok(Self { child, address });
                }
            }
            if child.try_wait()?.is_some() {
                return Err("provider failed before readiness".into());
            }
            if start.elapsed() > Duration::from_secs(5) {
                let _ = child.kill();
                let _ = child.wait();
                return Err("provider readiness timeout".into());
            }
            std::thread::sleep(Duration::from_millis(2));
        }
    }
}
impl Drop for Provider {
    fn drop(&mut self) {
        let _ = self.child.kill();
        let _ = self.child.wait();
    }
}

#[derive(Clone, Copy, Default, Serialize)]
struct Costs {
    collect_ns: u64,
    verify_ns: u64,
}
struct Adapter {
    address: SocketAddr,
    costs: Arc<Mutex<Costs>>,
}
fn verify(message: &OutboxMessage, stage: &EvidenceStage, previous: Option<&EvidenceReceipt>, bytes: &[u8]) -> AnyResult<()> {
    let receipt: Receipt = serde_json::from_slice(bytes)?;
    let binding = previous.map_or([0; 32], |p| p.commitment(descriptor().commitment(), message.commitment()));
    if receipt.message != message.commitment()
        || receipt.stage != stage.name
        || receipt.previous != binding
        || receipt.payload_hash != hash(&message.payload)
        || receipt.payload_bytes != message.payload.len() as u64
    {
        return Err("provider receipt is not bound to exact message/stage/payload".into());
    }
    Secp256k1::verification_only().verify_schnorr(
        &Signature::from_slice(&receipt.signature)?,
        &Message::from_digest(receipt.digest()),
        &provider_key(),
    )?;
    Ok(())
}
#[async_trait]
impl EvidenceAdapter for Adapter {
    fn descriptor(&self) -> EvidencePipelineDescriptor {
        descriptor()
    }
    async fn collect_evidence(
        &self,
        message: &OutboxMessage,
        stage: &EvidenceStage,
        previous: Option<&EvidenceReceipt>,
    ) -> Result<Vec<u8>, EvidenceAdapterError> {
        let start = Instant::now();
        let result = (|| -> AnyResult<Vec<u8>> {
            let mut stream = TcpStream::connect_timeout(&self.address, NETWORK_TIMEOUT)?;
            let request = Request {
                message: message.clone(),
                stage: stage.name.clone(),
                previous: previous.map_or([0; 32], |p| p.commitment(descriptor().commitment(), message.commitment())),
            };
            write_packet(&mut stream, &serde_json::to_vec(&request)?)?;
            read_packet(&mut stream)
        })();
        self.costs.lock().unwrap().collect_ns = ns(start.elapsed());
        result.map_err(|e| EvidenceAdapterError::new(e.to_string()))
    }
    fn verify_evidence(
        &self,
        message: &OutboxMessage,
        stage: &EvidenceStage,
        previous: Option<&EvidenceReceipt>,
        bytes: &[u8],
    ) -> Result<(), EvidenceAdapterError> {
        let start = Instant::now();
        let result = verify(message, stage, previous, bytes);
        self.costs.lock().unwrap().verify_ns = ns(start.elapsed());
        result.map_err(|e| EvidenceAdapterError::new(e.to_string()))
    }
}

pub(super) struct Worker {
    done: Arc<AtomicBool>,
    abort: Arc<AtomicBool>,
    handle: Option<JoinHandle<Result<(), String>>>,
}
fn runtime(
    store: Arc<RocksSessionStore>,
    address: SocketAddr,
    costs: Arc<Mutex<Costs>>,
) -> AnyResult<EvidenceRuntime<RocksSessionStore>> {
    let mut runtime = EvidenceRuntime::new(store);
    runtime.register(Arc::new(Adapter { address, costs }))?;
    Ok(runtime)
}
impl Worker {
    pub(super) fn start(
        root: &Path,
        pair: u32,
        sessions: [Hash32; 2],
        mode: Mode,
        store: Arc<RocksSessionStore>,
        commits: Commits,
        origin: Instant,
    ) -> AnyResult<Self> {
        let root = root.to_owned();
        let provider_root = root.join(format!("provider-{pair}"));
        let provider_mode = match (pair, mode) {
            (0, Mode::Slow) => "slow",
            (0, Mode::Offline) => "exit-after-store",
            _ => "normal",
        };
        let provider = Provider::start(&provider_root, provider_mode)?;
        let done = Arc::new(AtomicBool::new(false));
        let abort = Arc::new(AtomicBool::new(false));
        let (finished, cancelled) = (done.clone(), abort.clone());
        let handle = std::thread::spawn(move || {
            let execute = || -> AnyResult<()> {
                let mut provider = provider;
                let costs = Arc::new(Mutex::new(Costs::default()));
                let mut evidence = runtime(store.clone(), provider.address, costs.clone())?;
                let mut log = BufWriter::new(File::create(root.join(format!("evidence-{pair}.jsonl")))?);
                let mut restarted = false;
                let mut attempts = 0u64;
                let mut failures = 0u64;
                let mut complete = 0u64;
                loop {
                    if cancelled.load(Ordering::Acquire) || origin.elapsed() > Duration::from_secs(60) {
                        return Err("evidence worker cancelled or timed out".into());
                    }
                    let mut idle = true;
                    for session in sessions {
                        for delivery in store.pending_outbox(session, 1)? {
                            let Some(committed_ns) = commits.lock().unwrap().get(&delivery.message.id).copied() else {
                                continue;
                            };
                            idle = false;
                            attempts += 1;
                            let before = store.load_evidence(session, delivery.message.id)?;
                            *costs.lock().unwrap() = Costs::default();
                            let start_ns = ns(origin.elapsed());
                            let result = futures::executor::block_on(evidence.advance_delivery(session, delivery.clone()));
                            let finished_ns = ns(origin.elapsed());
                            let cost = *costs.lock().unwrap();
                            let (status, stage, error) = match result {
                                Ok(advance) => {
                                    if advance.complete {
                                        complete += 1;
                                    }
                                    (if advance.complete { "complete" } else { "advanced" }, Some(advance.accepted_stage), None)
                                }
                                Err(EvidenceRuntimeError::Adapter(error)) => {
                                    failures += 1;
                                    if store.load_evidence(session, delivery.message.id)? != before
                                        || !store.pending_outbox(session, 1)?.iter().any(|d| d.message.id == delivery.message.id)
                                    {
                                        return Err("failed provider request advanced evidence or acknowledged message".into());
                                    }
                                    ("failed", None, Some(error.to_string()))
                                }
                                Err(error) => return Err(error.into()),
                            };
                            json_line(
                                &mut log,
                                &serde_json::json!({"pair":pair,"session":BlobId(session),"message":BlobId(delivery.message.id),
                                "status":status,"stage":stage,"error":error,"committed_ns":committed_ns,"started_ns":start_ns,
                                "finished_ns":finished_ns,"costs":cost,"evidence_store_and_ack_ns":finished_ns-start_ns-cost.collect_ns-cost.verify_ns}),
                            )?;
                            log.flush()?;
                            if status == "failed" {
                                if pair != 0 || mode != Mode::Offline || restarted {
                                    return Err("unexpected provider failure".into());
                                }
                                if provider.child.wait()?.code() != Some(77) {
                                    return Err("provider did not exit at durable-store fault".into());
                                }
                                let outage = Instant::now();
                                let before = sessions.iter().map(|s| store.load_head(*s)).collect::<Result<Vec<_>, _>>()?;
                                match TcpStream::connect_timeout(&provider.address, NETWORK_TIMEOUT) {
                                    Err(error) if error.kind() == std::io::ErrorKind::ConnectionRefused => {}
                                    _ => return Err("offline provider did not refuse connections".into()),
                                }
                                std::thread::sleep(Duration::from_millis(500));
                                provider = Provider::start(&provider_root, "normal")?;
                                evidence = runtime(store.clone(), provider.address, costs.clone())?;
                                let heads =
                                    sessions.iter().map(|s| head_view(&store.load_head(*s)?)).collect::<AnyResult<Vec<_>>>()?;
                                if heads.iter().map(|h| h.input_position).sum::<u64>()
                                    <= before.iter().map(|h| h.input_position).sum::<u64>()
                                {
                                    return Err("canonical rewards did not advance while provider was offline".into());
                                }
                                save(
                                    &root.join(format!("provider-restart-{pair}.json")),
                                    &serde_json::json!({"exit_code":77,"connection_refused":true,
                                    "outage_and_restart_ns":ns(outage.elapsed()),"heads_before":before.iter().map(head_view).collect::<AnyResult<Vec<_>>>()?,
                                    "canonical_heads":heads,"canonical_progress_while_offline":true}),
                                )?;
                                restarted = true;
                            }
                        }
                    }
                    if idle && finished.load(Ordering::Acquire) {
                        // A commit may precede its timing entry, then the done
                        // flag. Recheck durable work after observing the flag.
                        if store.pending_outbox(sessions[0], 1)?.is_empty() && store.pending_outbox(sessions[1], 1)?.is_empty() {
                            break;
                        }
                    }
                    if idle {
                        std::thread::sleep(Duration::from_millis(1));
                    }
                }
                if pair == 0 && mode == Mode::Offline && (!restarted || failures != 1) {
                    return Err("offline drill never observed the intended failure".into());
                }
                save(
                    &root.join(format!("evidence-summary-{pair}.json")),
                    &serde_json::json!({"pair":pair,"attempts":attempts,"failures":failures,
                    "complete":complete,"finished_ns":ns(origin.elapsed()),"provider_restarted":restarted}),
                )?;
                Ok(())
            };
            execute().map_err(|e| e.to_string())
        });
        Ok(Self { done, abort, handle: Some(handle) })
    }
    pub(super) fn finish(mut self) -> AnyResult<()> {
        self.done.store(true, Ordering::Release);
        self.handle.take().unwrap().join().map_err(|_| "evidence worker panicked")??;
        Ok(())
    }
}
impl Drop for Worker {
    fn drop(&mut self) {
        self.abort.store(true, Ordering::Release);
        if let Some(handle) = self.handle.take() {
            let _ = handle.join();
        }
    }
}

pub(super) fn check<S: SessionStore>(store: &S, session: Hash32, messages: &[OutboxMessage]) -> AnyResult<()> {
    let descriptor = descriptor();
    for message in messages {
        let record = store.load_evidence(session, message.id)?.ok_or("reward evidence absent after publication")?;
        record.validate_for(session, message, &descriptor)?;
        if !record.is_complete() {
            return Err("reward evidence is not complete".into());
        }
        for (index, receipt) in record.receipts.iter().enumerate() {
            verify(message, &descriptor.stages[index], index.checked_sub(1).map(|i| &record.receipts[i]), &receipt.evidence)?;
        }
    }
    Ok(())
}
