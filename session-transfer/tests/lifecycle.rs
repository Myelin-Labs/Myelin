#[allow(dead_code)]
mod support;
use myelin_session::{BlockRequest, CommandStatus, HandoffAction, HandoffTerminal, SessionChain, SessionStore};
use myelin_session_store_rocksdb::RocksSessionStore;
use myelin_session_transfer::*;
use std::sync::{Arc, Barrier};
use support::*;

fn issue(f: &mut Fixture) -> myelin_session::Hash32 {
    let request = issue_request(&f.source, vec![f.intent(1, 0, 25)]);
    commit(&mut f.source, &f.verifier, &f.validator, request)[0]
}
fn cancel(f: &Fixture, id: myelin_session::Hash32) -> BlockRequest {
    let handoff = f.store.load_handoff(id).unwrap().unwrap().handoff;
    let command = signer().cancel(f.source.config(), &handoff).unwrap();
    let mut req = request(&f.source, vec![command], vec![], vec![]);
    req.handoff_actions.push(HandoffAction::Cancel { handoff_id: id });
    req
}
fn reclaim(f: &Fixture, id: myelin_session::Hash32) -> BlockRequest {
    let mut req = request(&f.source, vec![TransferCommand::Reclaim { handoff_id: id }], vec![], vec![]);
    req.handoff_actions.push(HandoffAction::Reclaim { handoff_id: id });
    req
}

#[test]
fn committed_cancellation_precedes_refund_and_blocks_backdated_intake_after_restart() {
    let mut f = Fixture::new(TransferKind::Scarce);
    let id = issue(&mut f);
    assert!(f.source.prepare_block(reclaim(&f, id)).is_err());
    let prepared_target = f.target.prepare_block(intake_request(&f.target, vec![id])).unwrap();
    let target_proof = proof(&f.verifier, &f.validator, &prepared_target);
    let mut req = cancel(&f, id);
    req.timestamp_ms = 101; // Expiry alone has not refunded anything.
    req.local_now_ms = 101;
    commit(&mut f.source, &f.verifier, &f.validator, req);
    let source = stored_state(&f.store, [3; 32]);
    let account = source.account(ASSET, signer().actor()).unwrap();
    assert_eq!((account.available, account.locked), (75, 25));
    let cancelled = f.store.load_handoff(id).unwrap().unwrap();
    let HandoffTerminal::Cancelled { at } = &cancelled.lifecycle.terminal else { panic!() };
    assert_eq!(at.height, 1);
    assert_eq!(at.block_hash, f.source.head().block_hash);
    // Prepared at time 10 while issued. Commit must recheck the same key.
    assert!(f.target.commit_finalised(prepared_target, target_proof).is_err());
    assert!(f.target.prepare_block(intake_request(&f.target, vec![id])).is_err());
    assert_eq!(f.target.head().finalised_height, None);
    let req = reclaim(&f, id);
    let command = req.frame_input.commands[0].clone();
    commit(&mut f.source, &f.verifier, &f.validator, req);
    assert!(matches!(f.source.command_status(&command).unwrap(), CommandStatus::Committed(_)));
    let state = stored_state(&f.store, [3; 32]);
    let a = state.account(ASSET, signer().actor()).unwrap();
    assert_eq!((a.available, a.locked, a.version), (100, 0, 2));
    assert!(state.pending().is_empty());
    assert!(stored_state(&f.store, [4; 32]).accounts().is_empty());
    assert!(f.source.prepare_block(reclaim(&f, id)).is_err());
    let reused = f.intent(1, 2, 25);
    assert!(f.source.prepare_block(issue_request(&f.source, vec![reused])).is_err());
    replay(&f.source, &f.store);
    let Fixture { directory, store, source, target, verifier, .. } = f;
    let cfg = source.config().clone();
    drop(source);
    drop(target);
    drop(store);
    let store = Arc::new(RocksSessionStore::open(directory.path()).unwrap());
    let snapshot = store.load_checkpoint(cfg.session_id).unwrap().state_snapshot;
    let restored = restore_executor(&cfg, &snapshot, store.as_ref()).unwrap();
    let recovered = SessionChain::recover(cfg.session_id, &cfg.application_profile, verifier, restored, store.clone()).unwrap();
    replay(&recovered, &store);
    let status = store.load_handoff(id).unwrap().unwrap();
    let HandoffTerminal::Reclaimed { cancelled_at, reclaimed_at } = status.lifecycle.terminal.clone() else { panic!() };
    assert_eq!((cancelled_at.height, reclaimed_at.height), (1, 2));
    assert_eq!(reclaimed_at.block_hash, recovered.head().block_hash);
    assert!(!status.is_consumed());
}

#[test]
fn cancellation_requires_exact_owner_and_source_and_cannot_batch_refund() {
    let mut f = Fixture::new(TransferKind::Scarce);
    let id = issue(&mut f);
    let handoff = f.store.load_handoff(id).unwrap().unwrap().handoff;
    assert!(TransferSigner::new([9; 32]).unwrap().cancel(f.source.config(), &handoff).is_err());
    assert!(signer().cancel(f.target.config(), &handoff).is_err());
    let mut bad = cancel(&f, id);
    let command = TransferCommand::Cancel { handoff_id: id, authorization: vec![1; 64] };
    bad.frame_input = command_frame(f.source.config(), 1, 1, vec![command]).unwrap();
    assert!(f.source.prepare_block(bad).err().unwrap().to_string().contains("signature"));
    let mut same_block = cancel(&f, id);
    same_block.handoff_actions.push(HandoffAction::Reclaim { handoff_id: id });
    assert!(f.source.prepare_block(same_block).is_err());
    let mut duplicate = cancel(&f, id);
    duplicate.handoff_actions.push(HandoffAction::Cancel { handoff_id: id });
    assert!(f.source.prepare_block(duplicate).is_err());
    assert_eq!(f.store.load_head([3; 32]).unwrap().finalised_height, Some(0));
    assert!(f.store.load_handoff(id).unwrap().unwrap().is_available());
    assert_eq!(f.store.pending_outbox([3; 32], 10).unwrap().len(), 1);
}

#[test]
fn cancellation_and_consumption_have_one_transactional_terminal_outcome() {
    // Ordered variants prove each possible winner; barrier variants race actual
    // RocksDB transactions after both applications prepared a valid candidate.
    for mode in 0..3 {
        let mut f = Fixture::new(TransferKind::Scarce);
        let id = issue(&mut f);
        let cancel_candidate = f.source.prepare_block(cancel(&f, id)).unwrap();
        let intake_candidate = f.target.prepare_block(intake_request(&f.target, vec![id])).unwrap();
        let cancel_proof = proof(&f.verifier, &f.validator, &cancel_candidate);
        let intake_proof = proof(&f.verifier, &f.validator, &intake_candidate);
        let (cancel_won, intake_won) = if mode < 2 {
            if mode == 0 {
                (
                    f.source.commit_finalised(cancel_candidate, cancel_proof).is_ok(),
                    f.target.commit_finalised(intake_candidate, intake_proof).is_ok(),
                )
            } else {
                let intake = f.target.commit_finalised(intake_candidate, intake_proof).is_ok();
                (f.source.commit_finalised(cancel_candidate, cancel_proof).is_ok(), intake)
            }
        } else {
            let barrier = Barrier::new(2);
            std::thread::scope(|scope| {
                let source = &mut f.source;
                let target = &mut f.target;
                let b = &barrier;
                let c = scope.spawn(move || {
                    b.wait();
                    source.commit_finalised(cancel_candidate, cancel_proof).is_ok()
                });
                let t = scope.spawn(move || {
                    b.wait();
                    target.commit_finalised(intake_candidate, intake_proof).is_ok()
                });
                (c.join().unwrap(), t.join().unwrap())
            })
        };
        assert_ne!(cancel_won, intake_won);
        if mode == 0 {
            assert!(cancel_won);
        }
        if mode == 1 {
            assert!(intake_won);
        }
        if cancel_won {
            let req = reclaim(&f, id);
            commit(&mut f.source, &f.verifier, &f.validator, req);
            assert!(f.target.prepare_block(intake_request(&f.target, vec![id])).is_err());
        } else {
            assert!(f.source.prepare_block(reclaim(&f, id)).is_err());
            assert!(f.source.prepare_block(cancel(&f, id)).is_err());
            replay(&f.target, &f.store);
        }
        let source = stored_state(&f.store, [3; 32]);
        let target = stored_state(&f.store, [4; 32]);
        assert_eq!(
            source.account(ASSET, signer().actor()).unwrap().available + target.account(ASSET, recipient()).map_or(0, |a| a.available),
            100
        );
        assert_eq!(target.receipts().len(), usize::from(intake_won));
        replay(&f.source, &f.store);
    }
}

fn lifecycle_child_run(path: &std::path::Path, stage: &str) {
    let store = Arc::new(RocksSessionStore::open(path).unwrap());
    let (v, validator) = consensus();
    let mut source = recover_chain(&store, &v, 3);
    let handoff = store.load_block([3; 32], 0).unwrap().issued_handoffs[0].clone();
    let id = handoff.id();
    let stop = |boundary| {
        if stage == boundary {
            std::process::exit(72)
        }
    };
    if source.head().sealed_by_successor.is_none() {
        let req = continuation_request(&source, &store, 5, &[signer()]);
        let p = source.prepare_block(req).unwrap();
        let signed = proof(&v, &validator, &p);
        stop("before-seal");
        source.commit_finalised(p, signed).unwrap();
    }
    stop("after-seal");
    let mut chain = recover_chain(&store, &v, 5);
    if chain.head().finalised_height.is_none() {
        let mut req = request(&chain, vec![signer().cancel(chain.config(), &handoff).unwrap()], vec![], vec![]);
        req.handoff_actions.push(HandoffAction::Cancel { handoff_id: id });
        let p = chain.prepare_block(req).unwrap();
        let signed = proof(&v, &validator, &p);
        stop("before-cancel");
        chain.commit_finalised(p, signed).unwrap();
    }
    stop("after-cancel");
    if chain.head().finalised_height == Some(0) {
        let mut req = request(&chain, vec![TransferCommand::Reclaim { handoff_id: id }], vec![], vec![]);
        req.handoff_actions.push(HandoffAction::Reclaim { handoff_id: id });
        let p = chain.prepare_block(req).unwrap();
        let signed = proof(&v, &validator, &p);
        stop("before-refund");
        chain.commit_finalised(p, signed).unwrap();
    }
    stop("after-refund");
    assert_eq!(chain.head().finalised_height, Some(1));
    replay(&source, &store);
    replay(&chain, &store);
}

#[test]
fn lifecycle_crash_child() {
    let Some(path) = std::env::var_os("MYELIN_LIFECYCLE_CRASH_PATH") else { return };
    let stage = std::env::var("MYELIN_LIFECYCLE_CRASH_STAGE").unwrap();
    assert!(["before-seal", "after-seal", "before-cancel", "after-cancel", "before-refund", "after-refund"].contains(&stage.as_str()));
    lifecycle_child_run(std::path::Path::new(&path), &stage);
    panic!("crash boundary not reached");
}

#[test]
fn sealing_cancellation_and_refund_survive_real_process_exit_without_duplicate_rights() {
    for policy_target in [false, true] {
        for (index, stage) in
            ["before-seal", "after-seal", "before-cancel", "after-cancel", "before-refund", "after-refund"].iter().enumerate()
        {
            let mut f = Fixture::new(TransferKind::Scarce);
            let mut payload = TransferPayload::decode(&f.intent(1, 0, 25).payload).unwrap();
            if policy_target {
                payload.target =
                    TransferTarget::SharedStorePolicy { policy_commitment: f.target.config().application_profile.handoff_policy_hash };
            }
            let req = issue_request(&f.source, vec![signer().sign(payload, 100, None).unwrap()]);
            let id = commit(&mut f.source, &f.verifier, &f.validator, req)[0];
            let Fixture { directory, source, target, store, .. } = f;
            drop(source);
            drop(target);
            drop(store);
            let mut child = std::process::Command::new(std::env::current_exe().unwrap())
                .args(["--exact", "lifecycle_crash_child", "--nocapture"])
                .env("MYELIN_LIFECYCLE_CRASH_PATH", directory.path())
                .env("MYELIN_LIFECYCLE_CRASH_STAGE", stage)
                .spawn()
                .unwrap();
            let started = std::time::Instant::now();
            let status = loop {
                if let Some(status) = child.try_wait().unwrap() {
                    break status;
                }
                if started.elapsed() > std::time::Duration::from_secs(30) {
                    child.kill().unwrap();
                    child.wait().unwrap();
                    panic!("lifecycle child stalled");
                }
                std::thread::sleep(std::time::Duration::from_millis(10));
            };
            assert_eq!(status.code(), Some(72));
            let store = RocksSessionStore::open(directory.path()).unwrap();
            let state = stored_state(&store, [if index == 0 { 3 } else { 5 }; 32]);
            let account = state.account(ASSET, signer().actor()).unwrap();
            assert_eq!((account.available, account.locked), if index == 5 { (100, 0) } else { (75, 25) });
            assert_eq!(store.load_head([3; 32]).unwrap().sealed_by_successor, if index == 0 { None } else { Some([5; 32]) });
            if index == 0 {
                assert!(store.load_genesis([5; 32]).is_err());
            }
            let terminal = store.load_handoff(id).unwrap().unwrap().lifecycle.terminal;
            match index {
                0..=2 => assert!(matches!(terminal, HandoffTerminal::Issued)),
                3 | 4 => assert!(matches!(terminal, HandoffTerminal::Cancelled { .. })),
                5 => assert!(matches!(terminal, HandoffTerminal::Reclaimed { .. })),
                _ => unreachable!(),
            }
            assert!(stored_state(&store, [4; 32]).receipts().is_empty());
            drop(store);
            lifecycle_child_run(directory.path(), "resume");
            lifecycle_child_run(directory.path(), "resume");
            let store = RocksSessionStore::open(directory.path()).unwrap();
            let state = stored_state(&store, [5; 32]);
            let account = state.account(ASSET, signer().actor()).unwrap();
            assert_eq!((account.available, account.locked), (100, 0));
            assert_eq!(store.load_head([5; 32]).unwrap().finalised_height, Some(1));
            assert_eq!(store.pending_outbox([3; 32], 10).unwrap().len(), 2);
            assert_eq!(store.pending_outbox([5; 32], 10).unwrap().len(), 2);
            assert!(stored_state(&store, [4; 32]).receipts().is_empty());
        }
    }
}

#[test]
fn recovery_rejects_missing_reset_and_forged_lifecycle_records() {
    for corruption in ["missing", "reset", "wrong-cancel-block", "wrong-refund-block"] {
        let mut f = Fixture::new(TransferKind::Scarce);
        let id = issue(&mut f);
        let req = cancel(&f, id);
        commit(&mut f.source, &f.verifier, &f.validator, req);
        let req = reclaim(&f, id);
        commit(&mut f.source, &f.verifier, &f.validator, req);
        let mut status = f.store.load_handoff(id).unwrap().unwrap();
        let cfg = f.source.config().clone();
        let snapshot = f.store.load_checkpoint([3; 32]).unwrap().state_snapshot;
        let Fixture { directory, source, target, store, verifier, .. } = f;
        drop(source);
        drop(target);
        drop(store);
        let db: rocksdb::OptimisticTransactionDB =
            rocksdb::OptimisticTransactionDB::open(&rocksdb::Options::default(), directory.path()).unwrap();
        let mut key = vec![b'j'];
        key.extend_from_slice(&id);
        if corruption == "missing" {
            db.delete(&key).unwrap();
        } else {
            match corruption {
                "reset" => status.lifecycle.terminal = HandoffTerminal::Issued,
                "wrong-cancel-block" => {
                    let HandoffTerminal::Reclaimed { cancelled_at, .. } = &mut status.lifecycle.terminal else { panic!() };
                    cancelled_at.block_hash = [99; 32];
                }
                "wrong-refund-block" => {
                    let HandoffTerminal::Reclaimed { reclaimed_at, .. } = &mut status.lifecycle.terminal else { panic!() };
                    reclaimed_at.block_hash = [99; 32];
                }
                _ => unreachable!(),
            }
            db.put(&key, status.encode().unwrap()).unwrap();
        }
        drop(db);
        let store = Arc::new(RocksSessionStore::open(directory.path()).unwrap());
        let executor = restore_executor(&cfg, &snapshot, store.as_ref()).unwrap();
        assert!(SessionChain::recover([3; 32], &cfg.application_profile, verifier, executor, store).is_err(), "{corruption}");
    }
}

#[derive(Clone)]
struct WithoutLifecycle(TransferExecutor);
impl myelin_session::TransitionExecutor for WithoutLifecycle {
    fn state_root(&self) -> myelin_session::Hash32 {
        self.0.state_root()
    }
    fn snapshot(&self) -> std::result::Result<Vec<u8>, String> {
        self.0.snapshot()
    }
    fn execute_block(
        &mut self,
        h: u64,
        f: &myelin_session::FrameInput,
        c: &[myelin_session::SessionHandoff],
        t: &[Vec<u8>],
    ) -> std::result::Result<myelin_session::ExecutionOutcome, String> {
        self.0.execute_block(h, f, c, t)
    }
    fn execute_block_with_context(
        &mut self,
        c: &myelin_session::ProtocolEffectsContext,
        f: &myelin_session::FrameInput,
        h: &[myelin_session::SessionHandoff],
        t: &[Vec<u8>],
    ) -> std::result::Result<myelin_session::ExecutionOutcome, String> {
        self.0.execute_block_with_context(c, f, h, t)
    }
}

#[test]
fn old_policy_override_cannot_silently_drop_lifecycle_during_replay() {
    let mut f = Fixture::new(TransferKind::Scarce);
    let id = issue(&mut f);
    let req = cancel(&f, id);
    commit(&mut f.source, &f.verifier, &f.validator, req);
    let cfg = f.source.config().clone();
    let replay_store = f.store.clone();
    let replay_config = cfg.clone();
    let result = f
        .source
        .replay_range(0, 1, move |b| Ok(WithoutLifecycle(restore_executor(&replay_config, b, replay_store.as_ref()).unwrap())));
    assert!(result.unwrap_err().to_string().contains("no lifecycle policy"));
    let mut record = f.store.load_block([3; 32], 1).unwrap();
    let bytes = record.encode().unwrap();
    assert_eq!(&bytes[4..6], &myelin_session::RECORD_FORMAT_VERSION.to_le_bytes());
    let mut downgraded = bytes;
    downgraded[4..6].copy_from_slice(&9u16.to_le_bytes());
    assert!(myelin_session::FinalisedBlockRecord::decode(&downgraded).is_err());
    record.handoff_actions[0].handoff.payload.push(0);
    assert!(record.validate_resources(&cfg).is_err());
}

#[test]
fn lifecycle_witnesses_share_aggregate_counts_bytes_and_exact_record_budget() {
    let directory = tempfile::tempdir().unwrap();
    let store = Arc::new(RocksSessionStore::open(directory.path()).unwrap());
    let (v, validator) = consensus();
    let initial = state(policy(TransferKind::Scarce));
    let mut cfg = config(&v, 3, &initial);
    let destination = TransferTarget::Session { session_id: [4; 32], genesis_commitment: [5; 32] };
    let probe = signer().sign(payload(&cfg, initial.policy(), destination.clone(), 1, 0, 25), 100, None).unwrap();
    let cap = probe.payload.len() as u64;
    cfg.commit_limits.max_handoff_payload_bytes = cap;
    cfg.commit_limits.max_consumed_handoffs = 1;
    let intent = signer().sign(payload(&cfg, initial.policy(), destination.clone(), 1, 0, 25), 100, None).unwrap();
    let later = signer().sign(payload(&cfg, initial.policy(), destination, 2, 1, 10), 100, None).unwrap();
    let mut chain = create_chain(cfg.clone(), initial, v.clone(), store.clone()).unwrap();
    let req = issue_request(&chain, vec![intent]);
    let id = commit(&mut chain, &v, &validator, req)[0];
    let handoff = store.load_handoff(id).unwrap().unwrap().handoff;
    let command = signer().cancel(&cfg, &handoff).unwrap();
    let mut req = request(
        &chain,
        vec![TransferCommand::Issue { transfer_id: verify_intent(&later).unwrap().0 }, command.clone()],
        vec![later],
        vec![],
    );
    req.handoff_actions.push(HandoffAction::Cancel { handoff_id: id });
    assert!(matches!(
        chain.prepare_block(req),
        Err(myelin_session::SessionError::ResourceLimit { resource: "handoff_payload_bytes", .. })
    ));
    let mut req = request(&chain, vec![command], vec![], vec![]);
    req.handoff_actions = vec![HandoffAction::Cancel { handoff_id: id }; 2];
    assert!(matches!(
        chain.prepare_block(req.clone()),
        Err(myelin_session::SessionError::ResourceLimit { resource: "consumed_handoffs", .. })
    ));
    req.handoff_actions.pop();
    commit(&mut chain, &v, &validator, req);
    let mut record = store.load_block([3; 32], 1).unwrap();
    let original_size = record.encoded_size_with_limit(u64::MAX).unwrap();
    // Non-archival records omit snapshots, but budgets charge their original
    // complete form. Restore that retained checkpoint for the exact byte check.
    record.state_snapshot = store.load_checkpoint([3; 32]).unwrap().state_snapshot;
    let bytes = record.encode().unwrap();
    assert_eq!(original_size, bytes.len() as u64);
    assert_eq!(record.encoded_size_with_limit(u64::MAX).unwrap(), original_size);
    assert!(matches!(
        record.encoded_size_with_limit(original_size - 1),
        Err(myelin_session::SessionError::ResourceLimit { resource: "record_bytes", .. })
    ));
    let mut bounded = cfg;
    bounded.commit_limits.max_record_bytes = bytes.len() as u64 - 1;
    assert!(myelin_session::FinalisedBlockRecord::decode_for_config(&bytes, &bounded).is_err());
    replay(&chain, &store);
}
