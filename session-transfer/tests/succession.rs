#[allow(dead_code)]
mod support;
use myelin_session::{HandoffAction, HandoffTerminal, SessionStore, MAX_HANDOFF_OWNER_DEPTH};
use myelin_session_transfer::*;
use support::*;

fn issue(f: &mut Fixture) -> myelin_session::Hash32 {
    let req = issue_request(&f.source, vec![f.intent(1, 0, 25)]);
    commit(&mut f.source, &f.verifier, &f.validator, req)[0]
}

fn continue_source(f: &mut Fixture, id: u8) -> Chain {
    let req = continuation_request(&f.source, &f.store, id, &[signer()]);
    commit(&mut f.source, &f.verifier, &f.validator, req);
    recover_chain(&f.store, &f.verifier, id)
}

fn cancel(chain: &Chain, f: &Fixture, id: myelin_session::Hash32) -> myelin_session::BlockRequest {
    let handoff = f.store.load_handoff(id).unwrap().unwrap().handoff;
    let mut req = request(chain, vec![signer().cancel(chain.config(), &handoff).unwrap()], vec![], vec![]);
    req.handoff_actions.push(HandoffAction::Cancel { handoff_id: id });
    req
}

fn refund(chain: &Chain, id: myelin_session::Hash32) -> myelin_session::BlockRequest {
    let mut req = request(chain, vec![TransferCommand::Reclaim { handoff_id: id }], vec![], vec![]);
    req.handoff_actions.push(HandoffAction::Reclaim { handoff_id: id });
    req
}

#[test]
fn sealed_source_obligations_follow_continuations_with_fresh_authorization() {
    let mut f = Fixture::new(TransferKind::Scarce);
    let id = issue(&mut f);
    let handoff = f.store.load_handoff(id).unwrap().unwrap().handoff;
    let old_cancel = signer().cancel(f.source.config(), &handoff).unwrap();
    let stale = f.source.prepare_block(cancel(&f.source, &f, id)).unwrap();
    let stale_proof = proof(&f.verifier, &f.validator, &stale);
    let original_snapshot = f.store.load_checkpoint([3; 32]).unwrap().state_snapshot;
    let original_outbox = f.store.pending_outbox([3; 32], 10).unwrap()[0].clone();
    let mut next = continue_source(&mut f, 5);
    assert_eq!(f.store.load_genesis([5; 32]).unwrap().state_snapshot, original_snapshot);
    assert!(f.source.commit_finalised(stale, stale_proof).is_err());
    let mut old_req = request(&next, vec![old_cancel], vec![], vec![]);
    old_req.handoff_actions.push(HandoffAction::Cancel { handoff_id: id });
    assert!(next.prepare_block(old_req).err().unwrap().to_string().contains("signature"));
    // Same application and balances do not grant an unrelated session ownership.
    let state = stored_state(&f.store, [3; 32]);
    let mut cfg = f.source.config().clone();
    cfg.session_id = [9; 32];
    cfg.initial_state_root = state.root().unwrap();
    let unrelated = create_chain(cfg, state, f.verifier.clone(), f.store.clone()).err().unwrap();
    assert!(unrelated.to_string().contains("successor"));
    let req = cancel(&next, &f, id);
    commit(&mut next, &f.verifier, &f.validator, req);
    let req = continuation_request(&next, &f.store, 6, &[signer()]);
    commit(&mut next, &f.verifier, &f.validator, req);
    let mut last = recover_chain(&f.store, &f.verifier, 6);
    let req = refund(&last, id);
    commit(&mut last, &f.verifier, &f.validator, req);
    assert!(f.target.prepare_block(intake_request(&f.target, vec![id])).is_err());
    let state = stored_state(&f.store, [6; 32]);
    let account = state.account(ASSET, signer().actor()).unwrap();
    assert_eq!((account.available, account.locked, account.version), (100, 0, 2));
    let reused = signer()
        .sign(
            payload(
                last.config(),
                state.policy(),
                TransferTarget::Session {
                    session_id: f.target.config().session_id,
                    genesis_commitment: f.target.config().commitment(),
                },
                1,
                2,
                25,
            ),
            100,
            None,
        )
        .unwrap();
    assert!(last.prepare_block(issue_request(&last, vec![reused])).err().unwrap().to_string().contains("nonce"));
    let status = f.store.load_handoff(id).unwrap().unwrap();
    assert!(matches!(status.lifecycle.terminal, HandoffTerminal::Reclaimed { cancelled_at, reclaimed_at }
        if cancelled_at.session_id == [5; 32] && reclaimed_at.session_id == [6; 32]));
    assert!(f.store.pending_outbox([3; 32], 10).unwrap().contains(&original_outbox));
    assert!(f.store.pending_outbox([5; 32], 10).unwrap().iter().all(|m| m.message.id != original_outbox.message.id));
    replay(&f.source, &f.store);
    replay(&next, &f.store);
    replay(&last, &f.store);
}

#[test]
fn inherited_cancel_and_target_consume_still_have_one_atomic_winner() {
    for cancel_first in [true, false] {
        let mut f = Fixture::new(TransferKind::Scarce);
        let id = issue(&mut f);
        let mut next = continue_source(&mut f, 5);
        let cancelled = next.prepare_block(cancel(&next, &f, id)).unwrap();
        let cancellation_proof = proof(&f.verifier, &f.validator, &cancelled);
        let consumed = f.target.prepare_block(intake_request(&f.target, vec![id])).unwrap();
        let consumption_proof = proof(&f.verifier, &f.validator, &consumed);
        if cancel_first {
            next.commit_finalised(cancelled, cancellation_proof).unwrap();
            assert!(f.target.commit_finalised(consumed, consumption_proof).is_err());
            let req = refund(&next, id);
            commit(&mut next, &f.verifier, &f.validator, req);
        } else {
            f.target.commit_finalised(consumed, consumption_proof).unwrap();
            assert!(next.commit_finalised(cancelled, cancellation_proof).is_err());
            let at = f.store.load_handoff(id).unwrap().unwrap().consumption().unwrap().clone();
            let mut req = request(&next, vec![TransferCommand::Retire { handoff_id: id }], vec![], vec![]);
            req.handoff_actions.push(HandoffAction::Retire { handoff_id: id, consumed_at: at });
            commit(&mut next, &f.verifier, &f.validator, req);
            assert!(next.prepare_block(refund(&next, id)).is_err());
            replay(&f.target, &f.store);
        }
        let a = stored_state(&f.store, [5; 32]);
        let b = stored_state(&f.store, [4; 32]);
        let a = a.account(ASSET, signer().actor()).unwrap();
        assert_eq!(a.locked, 0);
        assert_eq!(a.available + b.account(ASSET, recipient()).map_or(0, |a| a.available), 100);
        replay(&next, &f.store);
    }
}

#[test]
fn continuation_requires_every_rightsholder_and_exact_target_configuration() {
    let mut f = Fixture::new(TransferKind::Scarce);
    let id = issue(&mut f);
    let req = intake_request(&f.target, vec![id]);
    commit(&mut f.target, &f.verifier, &f.validator, req);
    let owners = [TransferSigner::new([4; 32]).unwrap()];
    let state = stored_state(&f.store, [4; 32]);
    for bad in 0..5 {
        let mut req = continuation_request(&f.target, &f.store, 5, &owners);
        let mut target = req.successor.take().unwrap();
        let mut approvals = vec![owners[0].continue_session(f.target.config(), &state, &target).unwrap()];
        match bad {
            0 => approvals.clear(),
            1 => approvals.push(approvals[0].clone()),
            2 => approvals[0].signature[0] ^= 1,
            3 => target.commit_limits.max_outbox_messages -= 1,
            _ => {
                target.application_profile.program_digest = [92; 32];
                approvals = vec![owners[0].continue_session(f.target.config(), &state, &target).unwrap()];
            }
        }
        req.frame_input = command_frame(
            f.target.config(),
            f.target.head().input_position,
            f.target.head().logical_time,
            vec![TransferCommand::Continue { target_genesis: target.commitment(), approvals }],
        )
        .unwrap();
        req.successor = Some(target);
        assert!(f.target.prepare_block(req).is_err(), "bad case {bad}");
        assert!(f.store.load_genesis([5; 32]).is_err());
    }
    let req = continuation_request(&f.target, &f.store, 5, &owners);
    commit(&mut f.target, &f.verifier, &f.validator, req);
    let next = recover_chain(&f.store, &f.verifier, 5);
    let copied = stored_state(&f.store, [5; 32]);
    assert_eq!(copied.receipts(), state.receipts());
    assert_eq!(copied.accounts(), state.accounts());
    assert!(next.prepare_block(intake_request(&next, vec![id])).is_err());
    replay(&f.target, &f.store);
}

#[test]
fn depth_limit_blocks_sealing_before_an_outstanding_lock_can_be_stranded() {
    let mut f = Fixture::new(TransferKind::Scarce);
    let id = issue(&mut f);
    let mut current = continue_source(&mut f, 5);
    for hop in 1..MAX_HANDOFF_OWNER_DEPTH {
        let next_id = 5 + hop as u8;
        let req = continuation_request(&current, &f.store, next_id, &[signer()]);
        commit(&mut current, &f.verifier, &f.validator, req);
        current = recover_chain(&f.store, &f.verifier, next_id);
    }
    let req = continuation_request(&current, &f.store, 99, &[signer()]);
    assert!(current.prepare_block(req).err().unwrap().to_string().contains("reconcile inherited"));
    assert!(f.store.load_genesis([99; 32]).is_err());
    let req = cancel(&current, &f, id);
    commit(&mut current, &f.verifier, &f.validator, req);
    let req = refund(&current, id);
    commit(&mut current, &f.verifier, &f.validator, req);
    let req = continuation_request(&current, &f.store, 99, &[signer()]);
    commit(&mut current, &f.verifier, &f.validator, req);
    let last = recover_chain(&f.store, &f.verifier, 99);
    assert!(stored_state(&f.store, last.config().session_id).pending().is_empty());
    replay(&current, &f.store);
}

#[test]
fn inherited_lock_restore_rejects_broken_canonical_lineage() {
    for corruption in ["missing-seal", "unsealed-head", "changed-target-snapshot"] {
        let mut f = Fixture::new(TransferKind::Scarce);
        issue(&mut f);
        let next = continue_source(&mut f, 5);
        let cfg = next.config().clone();
        let snapshot = f.store.load_checkpoint([5; 32]).unwrap().state_snapshot;
        let mut parent = f.source.head().clone();
        let mut genesis = f.store.load_genesis([5; 32]).unwrap();
        drop(next);
        let Fixture { directory, source, target, store, .. } = f;
        drop(source);
        drop(target);
        drop(store);
        let db: rocksdb::OptimisticTransactionDB =
            rocksdb::OptimisticTransactionDB::open(&rocksdb::Options::default(), directory.path()).unwrap();
        match corruption {
            "missing-seal" => {
                let key = [vec![b'b'], vec![3; 32], 1u64.to_be_bytes().to_vec()].concat();
                db.delete(key).unwrap();
            }
            "unsealed-head" => {
                parent.sealed_by_successor = None;
                db.put([vec![b'h'], vec![3; 32]].concat(), parent.encode().unwrap()).unwrap();
            }
            _ => {
                genesis.state_snapshot[0] ^= 1;
                db.put([vec![b'g'], vec![5; 32]].concat(), genesis.encode().unwrap()).unwrap();
            }
        }
        drop(db);
        let store = myelin_session_store_rocksdb::RocksSessionStore::open(directory.path()).unwrap();
        assert!(restore_executor(&cfg, &snapshot, &store).is_err(), "{corruption}");
    }
}
