#![cfg(all(target_os = "linux", any(target_arch = "x86_64", target_arch = "aarch64"), feature = "test-fixture"))]
// Shared setup helpers are intentionally broader than this process-only lane.
#[allow(dead_code)]
mod support;
use myelin_session::{BlockRequest, SessionChain, SessionError, SessionStore};
use myelin_session_store_rocksdb::RocksSessionStore;
use myelin_session_transfer::*;
use myelin_session_worker::{OperationalLimits, ProcessExecutor, WorkerProgram, WorkerSettings};
use std::sync::Arc;
use support::*;

#[test]
fn actual_reference_policy_runs_and_replays_inside_the_bounded_worker() {
    let directory = tempfile::tempdir().unwrap();
    let store = Arc::new(RocksSessionStore::open(directory.path()).unwrap());
    let (verifier, validator) = consensus();
    let initial = state(policy(TransferKind::Scarce));
    let mut cfg = config(&verifier, 3, &initial);
    let path = env!("CARGO_BIN_EXE_myelin-transfer-test-worker");
    let digest = *blake3::hash(&std::fs::read(path).unwrap()).as_bytes();
    cfg.application_profile.program_digest = digest;
    let args = vec![cfg.encode().unwrap().iter().map(|b| format!("{b:02x}")).collect::<String>()];
    let program = WorkerProgram::load(path, digest, args).unwrap();
    let settings = WorkerSettings {
        profile: cfg.application_profile.clone(),
        commit_limits: cfg.commit_limits.clone(),
        operational: OperationalLimits::default(),
    };
    let worker =
        ProcessExecutor::restore(program.clone(), settings.clone(), initial.encode().unwrap(), initial.root().unwrap()).unwrap();
    let monitor = worker.monitor();
    let mut chain = SessionChain::create(cfg.clone(), verifier.clone(), worker, store.clone()).unwrap();
    let intent = signer()
        .sign(
            payload(&cfg, initial.policy(), TransferTarget::Session { session_id: [4; 32], genesis_commitment: [5; 32] }, 1, 0, 10),
            100,
            None,
        )
        .unwrap();
    let command = TransferCommand::Issue { transfer_id: verify_intent(&intent).unwrap().0 };
    let req = BlockRequest {
        transactions: vec![],
        frame_input: command_frame(&cfg, 0, 0, vec![command]).unwrap(),
        scheduler_commitment: [5; 32],
        data_commitments: vec![],
        timestamp_ms: 10,
        local_now_ms: 10,
        successor: None,
        handoffs: vec![intent],
        consume_handoffs: vec![],
        handoff_actions: vec![],
    };
    let native_store_dir = tempfile::tempdir().unwrap();
    let native_store = Arc::new(RocksSessionStore::open(native_store_dir.path()).unwrap());
    let native = create_chain(cfg.clone(), initial, verifier.clone(), native_store).unwrap();
    let expected = native.prepare_block(req.clone()).unwrap();
    let prepared = chain.prepare_block(req).unwrap();
    assert_eq!(prepared.block(), expected.block());
    assert_eq!(prepared.frame().resources.cycles, 0);
    let myelin_consensus::SelectedConsensus::ProofOfAuthority(engine) = verifier.selected_consensus() else { unreachable!() };
    let proof =
        myelin_consensus::FinalityProof::ProofOfAuthority(engine.seal_from_signer(prepared.block().hash(), 0, &validator).unwrap());
    chain.commit_finalised(prepared, proof).unwrap();
    let snapshot = store.load_checkpoint([3; 32]).unwrap().state_snapshot;
    let state = TransferState::decode(&snapshot).unwrap();
    assert_eq!(state.account(ASSET, signer().actor()).unwrap().available, 90);
    let replay_program = program.clone();
    let replay_settings = settings.clone();
    let replay_root = cfg.initial_state_root;
    chain
        .replay_range(0, 0, move |bytes| {
            ProcessExecutor::restore(replay_program.clone(), replay_settings.clone(), bytes.to_vec(), replay_root)
                .map_err(|e| SessionError::Replay(e.to_string()))
        })
        .unwrap();
    let handoff = store.load_block([3; 32], 0).unwrap().issued_handoffs[0].clone();
    for (command, action) in [
        (signer().cancel(&cfg, &handoff).unwrap(), myelin_session::HandoffAction::Cancel { handoff_id: handoff.id() }),
        (TransferCommand::Reclaim { handoff_id: handoff.id() }, myelin_session::HandoffAction::Reclaim { handoff_id: handoff.id() }),
    ] {
        let req = BlockRequest {
            transactions: vec![],
            frame_input: command_frame(&cfg, chain.head().input_position, chain.head().logical_time, vec![command]).unwrap(),
            scheduler_commitment: [5; 32],
            data_commitments: vec![],
            timestamp_ms: 11,
            local_now_ms: 11,
            successor: None,
            handoffs: vec![],
            consume_handoffs: vec![],
            handoff_actions: vec![action],
        };
        let prepared = chain.prepare_block(req).unwrap();
        let proof = myelin_consensus::FinalityProof::ProofOfAuthority(
            engine.seal_from_signer(prepared.block().hash(), prepared.block().number, &validator).unwrap(),
        );
        chain.commit_finalised(prepared, proof).unwrap();
    }
    let restored = stored_state(&store, [3; 32]);
    let account = restored.account(ASSET, signer().actor()).unwrap();
    assert_eq!((account.available, account.locked), (100, 0));
    chain
        .replay_range(0, 2, move |bytes| {
            ProcessExecutor::restore(program.clone(), settings.clone(), bytes.to_vec(), cfg.initial_state_root)
                .map_err(|e| SessionError::Replay(e.to_string()))
        })
        .unwrap();
    let report = monitor.last_report().unwrap();
    assert!(report.measurements.user_cpu_us + report.measurements.system_cpu_us > 0);
    assert!(report.failure.is_none());
    assert!(!std::path::Path::new(&format!("/proc/{}", report.measurements.pid)).exists());
}

#[test]
fn consumed_lock_retirement_runs_and_replays_inside_the_bounded_worker() {
    let directory = tempfile::tempdir().unwrap();
    let store = Arc::new(RocksSessionStore::open(directory.path()).unwrap());
    let (verifier, validator) = consensus();
    let initial = state(policy(TransferKind::Scarce));
    let mut cfg = config(&verifier, 3, &initial);
    let path = env!("CARGO_BIN_EXE_myelin-transfer-test-worker");
    let digest = *blake3::hash(&std::fs::read(path).unwrap()).as_bytes();
    cfg.application_profile.program_digest = digest;
    let args = vec![cfg.encode().unwrap().iter().map(|b| format!("{b:02x}")).collect::<String>()];
    let program = WorkerProgram::load(path, digest, args).unwrap();
    let settings = WorkerSettings {
        profile: cfg.application_profile.clone(),
        commit_limits: cfg.commit_limits.clone(),
        operational: OperationalLimits::default(),
    };
    // Native source setup keeps request construction identical to other tests;
    // the retirement is committed through the actual process executor below.
    let mut source = create_chain(cfg.clone(), initial, verifier.clone(), store.clone()).unwrap();
    let mut target_policy = policy(TransferKind::Scarce);
    target_policy.accepted_sources.push(SourceIdentity::from_config(&cfg).unwrap());
    let initial = TransferState::new(target_policy, vec![]).unwrap();
    let target_cfg = config(&verifier, 4, &initial);
    let mut target = create_chain(target_cfg.clone(), initial, verifier.clone(), store.clone()).unwrap();
    let intent = signer()
        .sign(
            payload(
                &cfg,
                stored_state(&store, [3; 32]).policy(),
                TransferTarget::Session { session_id: [4; 32], genesis_commitment: target_cfg.commitment() },
                1,
                0,
                25,
            ),
            100,
            None,
        )
        .unwrap();
    let req = issue_request(&source, vec![intent]);
    let id = commit(&mut source, &verifier, &validator, req)[0];
    let req = intake_request(&target, vec![id]);
    commit(&mut target, &verifier, &validator, req);
    let at = myelin_session::HandoffBlockReference::from_block(&store.load_block([4; 32], 0).unwrap().block);
    let mut req = request(&source, vec![TransferCommand::Retire { handoff_id: id }], vec![], vec![]);
    req.handoff_actions.push(myelin_session::HandoffAction::Retire { handoff_id: id, consumed_at: at });
    let expected = source.prepare_block(req.clone()).unwrap();
    let checkpoint = store.load_checkpoint([3; 32]).unwrap();
    let worker =
        ProcessExecutor::restore(program.clone(), settings.clone(), checkpoint.state_snapshot, source.head().state_root).unwrap();
    let mut worker_chain = SessionChain::recover([3; 32], &cfg.application_profile, verifier.clone(), worker, store.clone()).unwrap();
    let candidate = worker_chain.prepare_block(req).unwrap();
    assert_eq!(candidate.block(), expected.block());
    let myelin_consensus::SelectedConsensus::ProofOfAuthority(engine) = verifier.selected_consensus() else { unreachable!() };
    let proof = myelin_consensus::FinalityProof::ProofOfAuthority(
        engine.seal_from_signer(candidate.block().hash(), candidate.block().number, &validator).unwrap(),
    );
    worker_chain.commit_finalised(candidate, proof).unwrap();
    let state = stored_state(&store, [3; 32]);
    let account = state.account(ASSET, signer().actor()).unwrap();
    assert_eq!((account.available, account.locked), (75, 0));
    worker_chain
        .replay_range(0, 1, move |bytes| {
            ProcessExecutor::restore(program.clone(), settings.clone(), bytes.to_vec(), cfg.initial_state_root)
                .map_err(|e| SessionError::Replay(e.to_string()))
        })
        .unwrap();
}
