#![cfg(all(target_os = "linux", any(target_arch = "x86_64", target_arch = "aarch64"), feature = "test-fixture"))]
#[path = "fixtures/application.rs"]
mod application;
use application::{profile, Application};
use myelin_consensus::{
    Authority, CommitteeSigner, ConsensusConfig, ConsensusKind, FinalityProof, ProofOfAuthorityConfig, SelectedConsensus,
};
use myelin_session::{
    BlockRequest, CommandIdentity, CommandStatus, CommitResourceLimits, FinalityVerifier, FrameInput, HandoffIntent, HandoffTarget,
    PreparedBlock, SessionChain, SessionConfig, SessionStore, TransitionExecutor,
};
use myelin_session_runtime::RegisteredFinalityVerifier;
use myelin_session_store_rocksdb::RocksSessionStore;
use myelin_session_worker::{FailureKind, OperationalLimits, ProcessExecutor, WorkerProgram, WorkerSettings};
use std::{
    sync::{Arc, OnceLock},
    time::{Duration, Instant},
};

fn program() -> WorkerProgram {
    static PROGRAM: OnceLock<WorkerProgram> = OnceLock::new();
    PROGRAM
        .get_or_init(|| {
            let path = env!("CARGO_BIN_EXE_myelin-worker-test-fixture");
            WorkerProgram::load(path, *blake3::hash(&std::fs::read(path).unwrap()).as_bytes(), vec![]).unwrap()
        })
        .clone()
}
fn settings() -> WorkerSettings {
    WorkerSettings {
        profile: profile(),
        commit_limits: CommitResourceLimits::default(),
        operational: OperationalLimits {
            wall_time: Duration::from_millis(300),
            cpu_seconds: 2,
            address_space_bytes: 64 * 1024 * 1024,
            output_bytes: 1024 * 1024,
            ..Default::default()
        },
    }
}
fn executor(settings: WorkerSettings) -> ProcessExecutor {
    ProcessExecutor::restore(program(), settings, vec![1; 32], [1; 32]).unwrap()
}
fn input(command: &[u8], position: u64) -> FrameInput {
    FrameInput {
        commands: vec![CommandIdentity { id: [40 + position as u8; 32], payload_hash: *blake3::hash(command).as_bytes() }],
        start_position: position,
        end_position: position + 1,
        logical_time_start: position,
        logical_time_end: position + 1,
        payload: command.to_vec(),
    }
}
fn request(command: &[u8], position: u64) -> BlockRequest {
    BlockRequest {
        transactions: vec![],
        frame_input: input(command, position),
        scheduler_commitment: [5; 32],
        data_commitments: vec![],
        timestamp_ms: 10 + position,
        local_now_ms: 10 + position,
        successor: None,
        handoffs: vec![],
        consume_handoffs: vec![],
        handoff_actions: vec![],
    }
}
fn consensus() -> (Arc<RegisteredFinalityVerifier>, CommitteeSigner) {
    let signer = CommitteeSigner::new("worker-authority", [7; 32]).unwrap();
    let verifier = RegisteredFinalityVerifier::new(ConsensusConfig::proof_of_authority(ProofOfAuthorityConfig {
        authorities: vec![Authority { id: signer.validator_id().into(), public_key: signer.ckb_public_key() }],
    }))
    .unwrap();
    (Arc::new(verifier), signer)
}
fn config(verifier: &RegisteredFinalityVerifier, id: u8) -> SessionConfig {
    let descriptor = verifier.descriptor();
    SessionConfig {
        session_id: [id; 32],
        consensus_kind: ConsensusKind::ProofOfAuthority,
        consensus_config_commitment: descriptor.config_commitment,
        consensus_module_commitment: descriptor.commitment(),
        consensus_wal_schema_hash: descriptor.wal_schema_hash,
        application_profile: profile(),
        initial_state_root: [1; 32],
        initial_input_position: 0,
        initial_logical_time: 0,
        initial_timestamp_ms: 0,
        predecessor: None,
        max_block_transactions: 100,
        max_block_bytes: 1_000_000,
        max_future_drift_ms: 1000,
        commit_limits: CommitResourceLimits::default(),
        protocol_effects: myelin_session::ProtocolEffectsPolicy::AuthorityMetadata { policy_hash: [99; 32] },
    }
}
fn proof<E: TransitionExecutor>(
    verifier: &RegisteredFinalityVerifier,
    signer: &CommitteeSigner,
    prepared: &PreparedBlock<E>,
) -> FinalityProof {
    let SelectedConsensus::ProofOfAuthority(engine) = verifier.selected_consensus() else { unreachable!() };
    FinalityProof::ProofOfAuthority(engine.seal_from_signer(prepared.block().hash(), prepared.block().number, signer).unwrap())
}
fn assert_reaped(pid: u32) {
    assert!(!std::path::Path::new(&format!("/proc/{pid}")).exists(), "worker {pid} remains alive or unreaped");
}

#[test]
fn native_and_process_execution_agree_and_host_metrics_are_separate_from_cycles() {
    let mut limits = settings();
    limits.operational.wall_time = Duration::from_secs(5);
    let mut worker = executor(limits);
    let mut native = Application::restore(&[1; 32]).unwrap();
    let input = input(b"work", 0);
    let expected = native.execute_block(0, &input, &[], &[]).unwrap();
    let outcome = worker.execute_block(0, &input, &[], &[]).unwrap();
    assert_eq!(outcome, expected);
    assert_eq!(worker.snapshot().unwrap(), native.snapshot().unwrap());
    assert_eq!(outcome.cycles, 0);
    let report = worker.monitor().last_report().unwrap();
    assert_eq!(report.failure, None);
    assert!(report.measurements.wall_time_us > 0);
    assert!(report.measurements.user_cpu_us > 0);
    assert!(report.measurements.observed_post_exec_peak_rss_bytes.unwrap() >= 4 * 1024 * 1024);
    assert!(report.measurements.observed_post_exec_peak_rss_bytes.unwrap() <= 64 * 1024 * 1024);
    assert_reaped(report.measurements.pid);
    // This optional file is acceptance evidence from a real invocation, never
    // a fixed benchmark value encoded into protocol tests.
    if let Some(path) = std::env::var_os("MYELIN_WORKER_METRICS_OUT") {
        let m = report.measurements;
        std::fs::write(
            path,
            serde_json::to_vec_pretty(&serde_json::json!({"vm_cycles":outcome.cycles,
            "wall_time_us":m.wall_time_us,"user_cpu_us":m.user_cpu_us,"system_cpu_us":m.system_cpu_us,
            "lifetime_peak_rss_bytes":m.lifetime_peak_rss_bytes,"observed_post_exec_peak_rss_bytes":m.observed_post_exec_peak_rss_bytes,"rss_samples":m.rss_samples,"address_space_limit_bytes":64*1024*1024,
            "method":"Linux wait4 lifetime CPU/RSS plus parent samples of post-exec /proc/PID/status VmHWM; single process, no child threads or processes"}))
            .unwrap(),
        )
        .unwrap();
    }
}

#[test]
fn failed_workers_preserve_head_receipts_handoff_marker_and_outbox_then_retry_once() {
    for (command, kind) in [
        (b"spin".as_slice(), FailureKind::Deadline),
        (b"sleep", FailureKind::Deadline),
        (b"allocate", FailureKind::ProcessExit),
        (b"exit", FailureKind::ProcessExit),
        (b"flood", FailureKind::OutputLimit),
        (b"stderr", FailureKind::OutputLimit),
        (b"bad-packet", FailureKind::Protocol),
    ] {
        let dir = tempfile::tempdir().unwrap();
        let store = Arc::new(RocksSessionStore::open(dir.path()).unwrap());
        let (verifier, signer) = consensus();
        let mut source =
            SessionChain::create(config(&verifier, 5), verifier.clone(), Application::restore(&[1; 32]).unwrap(), store.clone())
                .unwrap();
        let mut issue = request(b"seed", 0);
        issue.handoffs.push(HandoffIntent {
            target: HandoffTarget::Session([3; 32]),
            expires_at_ms: 1000,
            payload: b"item".to_vec(),
            authorization: b"test policy".to_vec(),
            evidence_requirement: None,
        });
        let prepared = source.prepare_block(issue).unwrap();
        let id = prepared.issued_handoffs()[0].id();
        let seal = proof(&verifier, &signer, &prepared);
        source.commit_finalised(prepared, seal).unwrap();
        let worker = executor(settings());
        let monitor = worker.monitor();
        let mut chain = SessionChain::create(config(&verifier, 3), verifier.clone(), worker, store.clone()).unwrap();
        let mut req = request(command, 0);
        req.consume_handoffs = vec![id];
        let identity = req.frame_input.commands[0].clone();
        let start = Instant::now();
        assert!(chain.prepare_block(req).is_err());
        assert!(start.elapsed() < Duration::from_secs(3));
        let report = monitor.last_report().unwrap();
        assert_eq!(report.failure, Some(kind), "{command:?}: {report:?}");
        if command == b"allocate" {
            assert_eq!(report.exit_code, Some(72));
        }
        assert_reaped(report.measurements.pid);
        assert_eq!(chain.head().finalised_height, None);
        assert_eq!(store.load_head([3; 32]).unwrap(), *chain.head());
        assert_eq!(store.load_checkpoint([3; 32]).unwrap().state_snapshot, vec![1; 32]);
        assert_eq!(chain.command_status(&identity).unwrap(), CommandStatus::Missing);
        assert!(!store.load_handoff(id).unwrap().unwrap().is_consumed());
        assert!(store.pending_outbox([3; 32], 100).unwrap().is_empty());
        let mut retry = request(b"ok", 0);
        retry.consume_handoffs = vec![id];
        let prepared = chain.prepare_block(retry).unwrap();
        let seal = proof(&verifier, &signer, &prepared);
        chain.commit_finalised(prepared, seal).unwrap();
        assert_eq!(chain.head().finalised_height, Some(0));
        assert!(store.load_handoff(id).unwrap().unwrap().is_consumed());
        assert_eq!(store.pending_outbox([3; 32], 100).unwrap().len(), 1);
    }
}

#[test]
fn cancellation_and_cpu_limit_reap_a_synchronous_infinite_loop() {
    let mut configuration = settings();
    configuration.operational.wall_time = Duration::from_secs(5);
    configuration.operational.cpu_seconds = 1;
    let mut cpu_worker = executor(configuration.clone());
    assert!(cpu_worker.execute_block(0, &input(b"spin", 0), &[], &[]).is_err());
    let report = cpu_worker.monitor().last_report().unwrap();
    assert_eq!(report.failure, Some(FailureKind::CpuLimit));
    assert!(report.measurements.user_cpu_us + report.measurements.system_cpu_us >= 900_000);
    assert_reaped(report.measurements.pid);
    let mut worker = executor(configuration);
    let monitor = worker.monitor();
    let cancellation = worker.cancellation();
    let task = std::thread::spawn(move || worker.execute_block(0, &input(b"spin", 0), &[], &[]));
    std::thread::sleep(Duration::from_millis(60));
    cancellation.cancel();
    assert!(task.join().unwrap().is_err());
    let report = monitor.last_report().unwrap();
    assert_eq!(report.failure, Some(FailureKind::Cancelled));
    assert_reaped(report.measurements.pid);
}

#[test]
fn descendant_processes_threads_and_mutable_artifact_paths_cannot_escape_the_boundary() {
    let mut worker = executor(settings());
    worker.execute_block(0, &input(b"spawn", 0), &[], &[]).unwrap();
    assert_eq!(worker.monitor().last_report().unwrap().failure, None);
    let directory = tempfile::tempdir().unwrap();
    let fifo_path = directory.path().join("not-an-executable");
    let fifo = std::ffi::CString::new(fifo_path.as_os_str().as_encoded_bytes()).unwrap();
    // A non-regular artifact must fail without waiting for a FIFO writer.
    assert_eq!(unsafe { libc::mkfifo(fifo.as_ptr(), 0o600) }, 0);
    assert!(matches!(WorkerProgram::load(&fifo_path, [9;32], vec![]), Err(error) if error.kind == FailureKind::Artifact));
    let copied = directory.path().join("worker");
    std::fs::copy(env!("CARGO_BIN_EXE_myelin-worker-test-fixture"), &copied).unwrap();
    let digest = *blake3::hash(&std::fs::read(&copied).unwrap()).as_bytes();
    assert!(matches!(WorkerProgram::load(&copied, [9;32], vec![]), Err(error) if error.kind == FailureKind::Artifact));
    let pinned = WorkerProgram::load(&copied, digest, vec![]).unwrap();
    std::fs::write(&copied, b"replacement is not executable").unwrap();
    ProcessExecutor::restore(pinned, settings(), vec![1; 32], [1; 32]).unwrap();
}

#[test]
fn independent_workers_and_replay_keep_deterministic_results_after_restart() {
    let dir = tempfile::tempdir().unwrap();
    let store = Arc::new(RocksSessionStore::open(dir.path()).unwrap());
    let (verifier, signer) = consensus();
    let mut slow = executor(settings());
    let slow_task = std::thread::spawn(move || slow.execute_block(0, &input(b"spin", 0), &[], &[]));
    let mut chain = SessionChain::create(config(&verifier, 3), verifier.clone(), executor(settings()), store.clone()).unwrap();
    for position in 0..3 {
        let prepared = chain.prepare_block(request(b"ok", position)).unwrap();
        let seal = proof(&verifier, &signer, &prepared);
        chain.commit_finalised(prepared, seal).unwrap();
    }
    assert!(slow_task.join().unwrap().is_err());
    let expected = chain.head().clone();
    drop(chain);
    let checkpoint = store.load_checkpoint([3; 32]).unwrap();
    let restored = ProcessExecutor::restore(program(), settings(), checkpoint.state_snapshot, checkpoint.state_root).unwrap();
    let chain = SessionChain::recover([3; 32], &profile(), verifier, restored, store).unwrap();
    assert_eq!(chain.head(), &expected);
    chain
        .replay_range(0, 2, |bytes| {
            ProcessExecutor::restore(program(), settings(), bytes.to_vec(), bytes.try_into().unwrap())
                .map_err(|e| myelin_session::SessionError::Replay(e.to_string()))
        })
        .unwrap();
}

#[test]
fn request_caps_and_restore_identity_fail_before_state_changes() {
    let mut configuration = settings();
    configuration.operational.input_bytes = 256;
    let mut worker = executor(configuration);
    let before = worker.snapshot().unwrap();
    let report = worker.monitor().last_report().unwrap();
    assert!(worker.execute_block(0, &input(&vec![1; 512], 0), &[], &[]).is_err());
    assert_eq!(worker.snapshot().unwrap(), before);
    assert_eq!(worker.monitor().last_report().unwrap().measurements.pid, report.measurements.pid);
    assert!(
        matches!(ProcessExecutor::restore(program(), settings(), vec![1;32], [2;32]), Err(error) if error.kind == FailureKind::Application)
    );
    worker.cancellation().cancel();
    assert!(worker.execute_block(0, &input(b"ok", 0), &[], &[]).unwrap_err().contains("Cancelled"));
}

#[test]
fn a_successful_process_exit_cannot_substitute_a_reply_from_another_request_or_state() {
    let path = env!("CARGO_BIN_EXE_myelin-worker-test-fixture");
    for mode in ["stale", "wrong-state"] {
        let program = WorkerProgram::load(path, program().digest(), vec![mode.into()]).unwrap();
        let error = ProcessExecutor::restore(program, settings(), vec![1; 32], [1; 32]).err().unwrap();
        assert_eq!(error.kind, FailureKind::Protocol);
        let report = error.report.unwrap();
        assert_eq!(report.exit_code, Some(0));
        assert_eq!(report.failure, Some(FailureKind::Protocol));
        assert_reaped(report.measurements.pid);
    }
}

#[test]
fn concurrent_candidate_failures_retain_their_own_request_and_process_report() {
    let worker = executor(settings());
    let tasks = [b"spin".as_slice(), b"bad-packet"]
        .into_iter()
        .map(|command| {
            let mut candidate = worker.clone();
            std::thread::spawn(move || {
                let frame = input(command, 0);
                let request = myelin_session_worker::wire::Request {
                    profile: profile().commitment(),
                    pre_root: [1; 32],
                    snapshot: vec![1; 32],
                    operation: myelin_session_worker::wire::Operation::Frame {
                        height: 0,
                        input: frame.clone(),
                        handoffs: vec![],
                        transactions: vec![],
                    },
                };
                let expected_hash = *blake3::hash(&myelin_session_worker::wire::encode(&request, 1024 * 1024).unwrap()).as_bytes();
                let error = candidate.execute_frame(0, &frame, &[], &[]).unwrap_err();
                let report = error.report.unwrap();
                assert_eq!(report.request_hash, expected_hash);
                assert_eq!(report.failure, Some(error.kind));
                assert_eq!(error.kind, if command == b"spin" { FailureKind::Deadline } else { FailureKind::Protocol });
                assert_reaped(report.measurements.pid);
            })
        })
        .collect::<Vec<_>>();
    for task in tasks {
        task.join().unwrap();
    }
    assert_eq!(worker.snapshot().unwrap(), vec![1; 32]);
}

#[test]
fn process_execution_transmits_and_enforces_the_genesis_effect_policy() {
    let dir = tempfile::tempdir().unwrap();
    let store = Arc::new(RocksSessionStore::open(dir.path()).unwrap());
    let (verifier, _) = consensus();
    let mut cfg = config(&verifier, 43);
    cfg.protocol_effects = myelin_session::ProtocolEffectsPolicy::Application { policy_hash: [71; 32] };
    let worker = executor(settings());
    let monitor = worker.monitor();
    let chain = SessionChain::create(cfg.clone(), verifier, worker, store.clone()).unwrap();
    let mut req = request(b"work", 0);
    req.handoffs.push(HandoffIntent {
        target: HandoffTarget::Session([44; 32]),
        expires_at_ms: 100,
        payload: vec![1],
        authorization: vec![1],
        evidence_requirement: None,
    });
    let err = chain.prepare_block(req).err().unwrap().to_string();
    assert!(err.contains("does not implement the genesis-bound authority metadata policy"), "{err}");
    assert_eq!(store.load_head(cfg.session_id).unwrap(), *chain.head());
    let report = monitor.last_report().unwrap();
    assert_eq!(report.failure, Some(FailureKind::Application));
    assert_reaped(report.measurements.pid);
}
