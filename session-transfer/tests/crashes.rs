//! Real process termination around the reference policy, not a fake executor.
//! Cancellation/refund crash points will join this matrix when those operations
//! are implemented. These tests make no cross-database atomicity claim.
#[allow(dead_code)]
mod support;
use myelin_session::{CommandStatus, Hash32, SessionChain, SessionConfig, SessionStore};
use myelin_session_store_rocksdb::RocksSessionStore;
use myelin_session_transfer::*;
use std::{fs, io::Write, path::Path, process::Command, sync::Arc, time::Duration};
use support::*;

const EXIT_AT_BOUNDARY: i32 = 71;
const STAGES: &[&str] = &[
    "before-source-commit",
    "after-source-commit",
    "before-target-commit",
    "after-target-commit",
    "before-delivery",
    "after-delivery-before-ack",
    "after-ack",
];

fn recover(store: Arc<RocksSessionStore>, cfg: &SessionConfig) -> Chain {
    let snapshot = store.load_checkpoint(cfg.session_id).unwrap().state_snapshot;
    let executor = restore_executor(cfg, &snapshot, store.as_ref()).unwrap();
    SessionChain::recover(cfg.session_id, &cfg.application_profile, consensus().0, executor, store).unwrap()
}

/// An independent idempotent delivery sink. A durable file is the external
/// effect; the outbox acknowledgement is a separate RocksDB transaction.
fn deliver(path: &Path, id: Hash32, payload: &[u8]) {
    let name = id.iter().map(|b| format!("{b:02x}")).collect::<String>();
    let receipt = path.join(name);
    match fs::OpenOptions::new().write(true).create_new(true).open(&receipt) {
        Ok(mut f) => {
            f.write_all(payload).unwrap();
            f.sync_all().unwrap();
            #[cfg(unix)]
            fs::File::open(path).unwrap().sync_all().unwrap();
        }
        Err(e) if e.kind() == std::io::ErrorKind::AlreadyExists => assert_eq!(fs::read(receipt).unwrap(), payload),
        Err(e) => panic!("delivery failed: {e}"),
    }
}

fn stop_at(selected: &str, boundary: &str) {
    if selected == boundary {
        // In particular, do not drop RocksDB, chains, or pending candidates.
        std::process::exit(EXIT_AT_BOUNDARY);
    }
}

fn run_source_and_target(path: &Path, policy_target: bool, stage: &str) {
    let store = Arc::new(RocksSessionStore::open(path.join("store")).unwrap());
    let (v, validator) = consensus();
    let source_cfg = store.load_genesis([3; 32]).unwrap().config;
    let target_cfg = store.load_genesis([4; 32]).unwrap().config;
    let mut source = recover(store.clone(), &source_cfg);
    let mut target = recover(store.clone(), &target_cfg);
    let source_policy = stored_state(&store, [3; 32]).policy().clone();
    let destination = if policy_target {
        TransferTarget::SharedStorePolicy { policy_commitment: target_cfg.application_profile.handoff_policy_hash }
    } else {
        TransferTarget::Session { session_id: target_cfg.session_id, genesis_commitment: target_cfg.commitment() }
    };
    // Reconstruct the original authorized intent, never a new nonce/version.
    let intent = signer().sign(payload(&source_cfg, &source_policy, destination, 1, 0, 25), 100, None).unwrap();
    let issue = TransferCommand::Issue { transfer_id: verify_intent(&intent).unwrap().0 };
    let identity = issue.identity(source_cfg.commitment()).unwrap();
    match source.command_status(&identity).unwrap() {
        CommandStatus::Missing => {
            assert!(source.head().finalised_height.is_none());
            let p = source.prepare_block(issue_request(&source, vec![intent])).unwrap();
            let signature = proof(&v, &validator, &p);
            stop_at(stage, "before-source-commit");
            source.commit_finalised(p, signature).unwrap();
        }
        CommandStatus::Committed(_) => assert_eq!(source.head().finalised_height, Some(0)),
        CommandStatus::IdentityConflict(_) => panic!("source command identity conflict"),
    }
    stop_at(stage, "after-source-commit");
    let record = store.load_block([3; 32], 0).unwrap();
    assert_eq!(record.issued_handoffs.len(), 1);
    let handoff_id = record.issued_handoffs[0].id();
    let command = TransferCommand::Intake { handoff_id }.identity(target_cfg.commitment()).unwrap();
    match target.command_status(&command).unwrap() {
        CommandStatus::Missing => {
            assert!(target.head().finalised_height.is_none());
            let p = target.prepare_block(intake_request(&target, vec![handoff_id])).unwrap();
            let signature = proof(&v, &validator, &p);
            stop_at(stage, "before-target-commit");
            target.commit_finalised(p, signature).unwrap();
        }
        CommandStatus::Committed(_) => assert_eq!(target.head().finalised_height, Some(0)),
        CommandStatus::IdentityConflict(_) => panic!("target command identity conflict"),
    }
    stop_at(stage, "after-target-commit");
    let pending = store.pending_outbox([4; 32], 10).unwrap();
    stop_at(stage, "before-delivery");
    for message in pending {
        deliver(&path.join("sink"), message.message.id, &message.message.payload);
        stop_at(stage, "after-delivery-before-ack");
        store.acknowledge_outbox([4; 32], message.message.id).unwrap();
    }
    stop_at(stage, "after-ack");
    replay(&source, &store);
    replay(&target, &store);
}

#[test]
fn transfer_crash_child() {
    let Some(path) = std::env::var_os("MYELIN_TRANSFER_CRASH_PATH") else { return };
    let stage = std::env::var("MYELIN_TRANSFER_CRASH_STAGE").unwrap();
    assert!(STAGES.contains(&stage.as_str()));
    let policy = std::env::var("MYELIN_TRANSFER_CRASH_POLICY").unwrap() == "true";
    run_source_and_target(Path::new(&path), policy, &stage);
    panic!("selected crash boundary was not reached");
}

fn matrix(policy_target: bool) {
    for (stage_index, stage) in STAGES.iter().enumerate() {
        let directory = tempfile::tempdir().unwrap();
        fs::create_dir(directory.path().join("sink")).unwrap();
        let store = Arc::new(RocksSessionStore::open(directory.path().join("store")).unwrap());
        let (v, _) = consensus();
        let initial = state(policy(TransferKind::Scarce));
        let cfg = config(&v, 3, &initial);
        let source = create_chain(cfg.clone(), initial, v.clone(), store.clone()).unwrap();
        let mut intake = policy(TransferKind::Scarce);
        intake.accepted_sources.push(SourceIdentity::from_config(&cfg).unwrap());
        let empty = TransferState::new(intake, vec![]).unwrap();
        let target_cfg = config(&v, 4, &empty);
        let target = create_chain(target_cfg.clone(), empty, v, store.clone()).unwrap();
        drop(source);
        drop(target);
        drop(store);

        let mut child = Command::new(std::env::current_exe().unwrap())
            .args(["--exact", "transfer_crash_child", "--nocapture"])
            .env("MYELIN_TRANSFER_CRASH_PATH", directory.path())
            .env("MYELIN_TRANSFER_CRASH_STAGE", stage)
            .env("MYELIN_TRANSFER_CRASH_POLICY", policy_target.to_string())
            .spawn()
            .unwrap();
        let started = std::time::Instant::now();
        let status = loop {
            if let Some(status) = child.try_wait().unwrap() {
                break status;
            }
            if started.elapsed() > Duration::from_secs(30) {
                child.kill().unwrap();
                child.wait().unwrap();
                panic!("child stalled at {stage}");
            }
            std::thread::sleep(Duration::from_millis(10));
        };
        assert_eq!(status.code(), Some(EXIT_AT_BOUNDARY), "boundary {stage}");
        // The old writer's exact process has exited. Only now is Missing a
        // definite uncommitted outcome for this controlled single-writer drill.
        let store = Arc::new(RocksSessionStore::open(directory.path().join("store")).unwrap());
        let source = recover(store.clone(), &cfg);
        let target = recover(store.clone(), &target_cfg);
        assert_eq!(source.head().finalised_height, (stage_index > 0).then_some(0));
        assert_eq!(target.head().finalised_height, (stage_index > 2).then_some(0));
        let source_state = stored_state(&store, [3; 32]);
        let account = source_state.account(ASSET, signer().actor()).unwrap();
        assert_eq!((account.available, account.locked), if stage_index == 0 { (100, 0) } else { (75, 25) });
        let target_state = stored_state(&store, [4; 32]);
        assert_eq!(target_state.receipts().len(), usize::from(stage_index > 2));
        let target_amount = target_state.account(ASSET, recipient()).map_or(0, |a| a.available);
        assert_eq!(target_amount, if stage_index > 2 { 25 } else { 0 });
        // Before intake the outstanding source lock is still the only claim.
        assert_eq!(account.available + target_amount + if stage_index <= 2 { account.locked } else { 0 }, 100);
        assert_eq!(store.pending_outbox([4; 32], 10).unwrap().len(), usize::from(stage_index > 2 && stage_index < 6));
        assert_eq!(fs::read_dir(directory.path().join("sink")).unwrap().count(), usize::from(stage_index >= 5));
        drop(source);
        drop(target);
        drop(store);

        run_source_and_target(directory.path(), policy_target, "resume-without-crash");
        // Repeat after successful commit and ACK: reconstructing the same
        // command yields its old outcome, with no second debit, credit or event.
        run_source_and_target(directory.path(), policy_target, "resume-without-crash");
        let store = RocksSessionStore::open(directory.path().join("store")).unwrap();
        assert_eq!(store.load_head([3; 32]).unwrap().finalised_height, Some(0));
        assert_eq!(store.load_head([4; 32]).unwrap().finalised_height, Some(0));
        let source = stored_state(&store, [3; 32]);
        let target = stored_state(&store, [4; 32]);
        assert_eq!(source.account(ASSET, signer().actor()).unwrap().available, 75);
        assert_eq!(target.account(ASSET, recipient()).unwrap().available, 25);
        assert_eq!(source.pending().len(), 1);
        assert_eq!(target.receipts().len(), 1);
        assert!(store.pending_outbox([4; 32], 10).unwrap().is_empty());
        assert_eq!(fs::read_dir(directory.path().join("sink")).unwrap().count(), 1);
        let record = store.load_block([3; 32], 0).unwrap();
        let status = store.load_handoff(record.issued_handoffs[0].id()).unwrap().unwrap();
        assert_eq!(status.consumption().map(|at| at.session_id), Some([4; 32]));
        assert_eq!(status.consumption().map(|at| at.block_hash), Some(store.load_head([4; 32]).unwrap().block_hash));
    }
}

#[test]
fn explicit_target_preserves_rights_across_commit_delivery_and_ack_crashes() {
    matrix(false);
}

#[test]
fn policy_target_preserves_rights_across_commit_delivery_and_ack_crashes() {
    matrix(true);
}
