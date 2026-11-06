#[allow(dead_code)]
mod support;
use myelin_session::{
    BlockRequest, CommandStatus, FinalisedBlockRecord, HandoffAction, HandoffBlockReference, HandoffTerminal, SessionChain,
    SessionStore, StoredHandoff,
};
use myelin_session_store_rocksdb::RocksSessionStore;
use myelin_session_transfer::*;
use std::sync::{Arc, Barrier};
use support::*;

fn issued(kind: TransferKind) -> (Fixture, [u8; 32]) {
    let mut f = Fixture::new(kind);
    let amount = if kind == TransferKind::Scarce { 25 } else { 1 };
    let req = issue_request(&f.source, vec![f.intent(1, 0, amount)]);
    let id = commit(&mut f.source, &f.verifier, &f.validator, req)[0];
    (f, id)
}
fn consume(f: &mut Fixture, id: [u8; 32]) -> HandoffBlockReference {
    let req = intake_request(&f.target, vec![id]);
    commit(&mut f.target, &f.verifier, &f.validator, req);
    HandoffBlockReference::from_block(&f.store.load_block([4; 32], 0).unwrap().block)
}
fn retire(chain: &Chain, id: [u8; 32], at: HandoffBlockReference) -> BlockRequest {
    let mut req = request(chain, vec![TransferCommand::Retire { handoff_id: id }], vec![], vec![]);
    req.handoff_actions.push(HandoffAction::Retire { handoff_id: id, consumed_at: at });
    req
}
fn recover(store: Arc<RocksSessionStore>, id: u8) -> Chain {
    let cfg = store.load_genesis([id; 32]).unwrap().config;
    let snapshot = store.load_checkpoint([id; 32]).unwrap().state_snapshot;
    let exec = restore_executor(&cfg, &snapshot, store.as_ref()).unwrap();
    SessionChain::recover([id; 32], &cfg.application_profile, consensus().0, exec, store).unwrap()
}
fn balances(store: &RocksSessionStore, retired: bool) {
    let source = stored_state(store, [3; 32]);
    let target = stored_state(store, [4; 32]);
    let a = source.account(ASSET, signer().actor()).unwrap();
    assert_eq!((a.available, a.locked, a.version), (75, if retired { 0 } else { 25 }, if retired { 2 } else { 1 }));
    assert_eq!(source.pending().len(), usize::from(!retired));
    assert_eq!(target.account(ASSET, recipient()).unwrap().available, 25);
    assert_eq!(target.receipts().len(), 1);
    assert_eq!(a.available + target.account(ASSET, recipient()).unwrap().available, 100);
}

#[test]
fn consumed_lock_retirement_never_refunds_and_replays_after_reopen() {
    let (mut f, id) = issued(TransferKind::Scarce);
    // Source and target use the current command schema.
    assert_eq!(f.target.config().application_profile.input_schema_hash, input_schema());
    let at = consume(&mut f, id);
    balances(&f.store, false);
    let req = retire(&f.source, id, at.clone());
    let command = req.frame_input.commands[0].clone();
    commit(&mut f.source, &f.verifier, &f.validator, req);
    balances(&f.store, true);
    assert!(matches!(f.source.command_status(&command).unwrap(), CommandStatus::Committed(_)));
    let mut record = f.store.load_block([3; 32], 1).unwrap();
    // Stored blocks omit the checkpoint bytes, but the resource cap still counts them.
    let original_size = record.encoded_size_with_limit(u64::MAX).unwrap();
    record.state_snapshot = f.store.load_checkpoint([3; 32]).unwrap().state_snapshot;
    assert_eq!(record.block.version, myelin_session::SESSION_BLOCK_VERSION);
    let encoded = record.encode().unwrap();
    assert_eq!(&encoded[4..6], &myelin_session::RECORD_FORMAT_VERSION.to_le_bytes());
    assert_eq!(record.encoded_size_with_limit(u64::MAX).unwrap(), encoded.len() as u64);
    assert_eq!(original_size, encoded.len() as u64);
    assert_eq!(FinalisedBlockRecord::decode(&encoded).unwrap(), record);
    // Codec-only check of the typed successor receipt. This does
    // not claim that the reference transfer policy authorizes succession.
    let mut with_successor = record.clone();
    let mut target_config = f.source.config().clone();
    target_config.session_id = [9; 32];
    target_config.initial_state_root = record.block.state_root_after;
    target_config.predecessor = Some(myelin_session::PredecessorReference {
        session_id: [3; 32],
        final_height: 1,
        final_state_root: record.block.state_root_after,
        application_profile_commitment: record.block.application_profile_commitment,
    });
    with_successor.successor = Some(myelin_session::SuccessorDeclaration {
        source_session_id: [3; 32],
        source_height: 1,
        source_state_root: record.block.state_root_after,
        source_application_profile_commitment: record.block.application_profile_commitment,
        target_config,
        transition: myelin_session::SuccessorTransition {
            source_genesis_commitment: f.source.config().commitment(),
            input_root: record.frame_input.root(),
            source_snapshot_hash: *blake3::hash(&record.state_snapshot).as_bytes(),
            state: myelin_session::SuccessorState::Continue,
        },
    });
    let bytes = with_successor.encode().unwrap();
    assert_eq!(with_successor.encoded_size_with_limit(u64::MAX).unwrap(), bytes.len() as u64);
    assert_eq!(FinalisedBlockRecord::decode(&bytes).unwrap(), with_successor);
    let status = f.store.load_handoff(id).unwrap().unwrap();
    assert!(status.is_consumed() && !status.is_available());
    assert_eq!(&status.encode().unwrap()[4..6], &myelin_session::RECORD_FORMAT_VERSION.to_le_bytes());
    assert_eq!(StoredHandoff::decode(&status.encode().unwrap()).unwrap(), status);
    assert!(matches!(&status.lifecycle.terminal,
        HandoffTerminal::Retired { consumed_at, retired_at } if consumed_at == &at && retired_at.block_hash == record.block.hash()));
    assert_eq!(record.outbox[0].topic, "myelin/reference-transfer/v2");
    assert!(record.outbox[0].payload.starts_with(b"MTRF"));
    assert!(f.source.prepare_block(retire(&f.source, id, at)).is_err());
    for (command, action) in [
        (signer().cancel(f.source.config(), &status.handoff).unwrap(), HandoffAction::Cancel { handoff_id: id }),
        (TransferCommand::Reclaim { handoff_id: id }, HandoffAction::Reclaim { handoff_id: id }),
    ] {
        let mut req = request(&f.source, vec![command], vec![], vec![]);
        req.handoff_actions.push(action);
        assert!(f.source.prepare_block(req).is_err());
    }
    assert!(f.target.prepare_block(intake_request(&f.target, vec![id])).is_err());
    // Removing a pending lock must not remove the nonce which prevents reissuance.
    let reused = f.intent(1, 2, 25);
    assert!(f.source.prepare_block(issue_request(&f.source, vec![reused])).is_err());
    let fresh = f.intent(2, 2, 25);
    assert!(f.source.prepare_block(issue_request(&f.source, vec![fresh])).is_ok());
    let Fixture { directory, source, target, store, .. } = f;
    drop(source);
    drop(target);
    drop(store);
    let store = Arc::new(RocksSessionStore::open(directory.path()).unwrap());
    for id in [3, 4] {
        replay(&recover(store.clone(), id), &store);
    }
    balances(&store, true);
}

#[test]
fn retirement_requires_exact_consumption_and_scarce_source() {
    let (mut f, id) = issued(TransferKind::Scarce);
    let fabricated = HandoffBlockReference { session_id: [4; 32], height: 0, block_hash: [9; 32] };
    assert!(f.source.prepare_block(retire(&f.source, id, fabricated)).is_err());
    let at = consume(&mut f, id);
    for field in 0..3 {
        let mut altered = at.clone();
        match field {
            0 => altered.session_id = [9; 32],
            1 => altered.height += 1,
            _ => altered.block_hash = [9; 32],
        }
        assert!(f.source.prepare_block(retire(&f.source, id, altered)).is_err());
    }
    let mut duplicate = retire(&f.source, id, at.clone());
    duplicate.handoff_actions.push(duplicate.handoff_actions[0].clone());
    assert!(f.source.prepare_block(duplicate).is_err());
    let mut missing = retire(&f.source, id, at.clone());
    missing.handoff_actions.clear();
    assert!(f.source.prepare_block(missing).is_err());
    let mut wrong = retire(&f.source, id, at.clone());
    wrong.frame_input = command_frame(f.source.config(), 1, 1, vec![TransferCommand::Reclaim { handoff_id: id }]).unwrap();
    assert!(f.source.prepare_block(wrong).is_err());
    // Wrong finality cannot publish retirement, its state, command receipt or outbox.
    let candidate = f.source.prepare_block(retire(&f.source, id, at)).unwrap();
    let bad_proof = f.store.load_block([3; 32], 0).unwrap().proof;
    assert!(f.source.commit_finalised(candidate, bad_proof).is_err());
    balances(&f.store, false);
    assert_eq!(f.store.pending_outbox([3; 32], 10).unwrap().len(), 1);
    let (mut f, id) = issued(TransferKind::CopyableReceipt);
    let at = consume(&mut f, id);
    assert!(f.source.prepare_block(retire(&f.source, id, at)).err().unwrap().to_string().contains("scarce lock"));
}

#[test]
fn concurrent_retirement_commits_once_with_one_receipt_and_outbox() {
    let (mut f, id) = issued(TransferKind::Scarce);
    let at = consume(&mut f, id);
    let mut second = recover(f.store.clone(), 3);
    let a = f.source.prepare_block(retire(&f.source, id, at.clone())).unwrap();
    let b = second.prepare_block(retire(&second, id, at)).unwrap();
    let pa = proof(&f.verifier, &f.validator, &a);
    let pb = proof(&f.verifier, &f.validator, &b);
    let barrier = Barrier::new(2);
    let winners = std::thread::scope(|scope| {
        let x = scope.spawn(|| {
            barrier.wait();
            f.source.commit_finalised(a, pa).is_ok()
        });
        let y = scope.spawn(|| {
            barrier.wait();
            second.commit_finalised(b, pb).is_ok()
        });
        usize::from(x.join().unwrap()) + usize::from(y.join().unwrap())
    });
    assert_eq!(winners, 1);
    balances(&f.store, true);
    assert_eq!(f.store.pending_outbox([3; 32], 10).unwrap().len(), 2);
    replay(&recover(f.store.clone(), 3), &f.store);
}

fn child_run(path: &std::path::Path, stage: &str) {
    let store = Arc::new(RocksSessionStore::open(path).unwrap());
    let mut source = recover(store.clone(), 3);
    let id = store.load_block([3; 32], 0).unwrap().issued_handoffs[0].id();
    if source.head().finalised_height == Some(0) {
        let at = HandoffBlockReference::from_block(&store.load_block([4; 32], 0).unwrap().block);
        let p = source.prepare_block(retire(&source, id, at)).unwrap();
        let (v, signer) = consensus();
        let proof = proof(&v, &signer, &p);
        if stage == "before" {
            std::process::exit(74);
        }
        source.commit_finalised(p, proof).unwrap();
    }
    if stage == "after" {
        std::process::exit(74);
    }
    replay(&source, &store);
    replay(&recover(store.clone(), 4), &store);
}
#[test]
fn retirement_crash_child() {
    if let Ok(path) = std::env::var("MYELIN_RETIREMENT_CHILD_STORE") {
        child_run(std::path::Path::new(&path), &std::env::var("MYELIN_RETIREMENT_CHILD_STAGE").unwrap());
    }
}
#[test]
fn process_exit_before_and_after_retirement_preserves_single_outcome() {
    for stage in ["before", "after"] {
        let (mut f, id) = issued(TransferKind::Scarce);
        consume(&mut f, id);
        let Fixture { directory, source, target, store, .. } = f;
        drop(source);
        drop(target);
        drop(store);
        let status = std::process::Command::new(std::env::current_exe().unwrap())
            .args(["--exact", "retirement_crash_child", "--nocapture"])
            .env("MYELIN_RETIREMENT_CHILD_STORE", directory.path())
            .env("MYELIN_RETIREMENT_CHILD_STAGE", stage)
            .status()
            .unwrap();
        assert_eq!(status.code(), Some(74));
        {
            let store = RocksSessionStore::open(directory.path()).unwrap();
            balances(&store, stage == "after");
            let terminal = store.load_handoff(id).unwrap().unwrap().lifecycle.terminal;
            assert_eq!(matches!(terminal, HandoffTerminal::Retired { .. }), stage == "after");
        }
        child_run(directory.path(), "resume");
        child_run(directory.path(), "resume");
        let store = RocksSessionStore::open(directory.path()).unwrap();
        balances(&store, true);
        assert_eq!(store.pending_outbox([3; 32], 10).unwrap().len(), 2);
    }
}

#[test]
fn retirement_index_downgrade_and_forged_terminal_evidence_fail_recovery() {
    for corruption in ["reset", "consumer", "retirement", "missing-target", "downgrade-index", "downgrade-block"] {
        let (mut f, id) = issued(TransferKind::Scarce);
        let at = consume(&mut f, id);
        let req = retire(&f.source, id, at.clone());
        commit(&mut f.source, &f.verifier, &f.validator, req);
        let mut stored = f.store.load_handoff(id).unwrap().unwrap();
        let mut record = f.store.load_block([3; 32], 1).unwrap();
        if corruption == "downgrade-block" {
            record.block.version = 3;
            assert!(record.encode().is_err());
            continue;
        }
        let Fixture { directory, source, target, store, .. } = f;
        drop(source);
        drop(target);
        drop(store);
        let db: rocksdb::OptimisticTransactionDB =
            rocksdb::OptimisticTransactionDB::open(&rocksdb::Options::default(), directory.path()).unwrap();
        let mut key = vec![b'j'];
        key.extend_from_slice(&id);
        match corruption {
            "missing-target" => {
                let mut key = vec![b'b'];
                key.extend_from_slice(&[4; 32]);
                key.extend_from_slice(&0u64.to_be_bytes());
                assert!(db.get(&key).unwrap().is_some());
                db.delete(key).unwrap();
            }
            "downgrade-index" => {
                let mut bytes = stored.encode().unwrap();
                bytes[4..6].copy_from_slice(&10u16.to_le_bytes());
                assert!(StoredHandoff::decode(&bytes).is_err());
                db.put(&key, bytes).unwrap();
            }
            _ => {
                match corruption {
                    "reset" => stored.lifecycle.terminal = HandoffTerminal::Consumed { at },
                    "consumer" => {
                        let HandoffTerminal::Retired { consumed_at, .. } = &mut stored.lifecycle.terminal else { panic!() };
                        consumed_at.block_hash = [99; 32];
                    }
                    "retirement" => {
                        let HandoffTerminal::Retired { retired_at, .. } = &mut stored.lifecycle.terminal else { panic!() };
                        retired_at.block_hash = [99; 32];
                    }
                    _ => unreachable!(),
                }
                db.put(&key, stored.encode().unwrap()).unwrap();
            }
        }
        drop(db);
        let store = Arc::new(RocksSessionStore::open(directory.path()).unwrap());
        let cfg = store.load_genesis([3; 32]).unwrap().config;
        let snap = store.load_checkpoint([3; 32]).unwrap().state_snapshot;
        let exec = restore_executor(&cfg, &snap, store.as_ref()).unwrap();
        assert!(SessionChain::recover([3; 32], &cfg.application_profile, consensus().0, exec, store).is_err(), "{corruption}");
    }
}
