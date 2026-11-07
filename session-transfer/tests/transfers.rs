#[allow(dead_code)]
mod support;
use myelin_session::{SessionChain, SessionStore};
use myelin_session_store_rocksdb::RocksSessionStore;
use myelin_session_transfer::*;
use std::sync::{Arc, Barrier};
use support::*;

#[test]
fn real_lock_credit_receipts_and_outbox_survive_restart_and_replay() {
    let mut f = Fixture::new(TransferKind::Scarce);
    let intent = f.intent(1, 0, 25);
    let id = verify_intent(&intent).unwrap().0;
    let req = issue_request(&f.source, vec![intent]);
    let handoffs = commit(&mut f.source, &f.verifier, &f.validator, req);
    let source_state = stored_state(&f.store, [3; 32]);
    let source_account = source_state.account(ASSET, signer().actor()).unwrap();
    assert_eq!((source_account.available, source_account.locked, source_account.version), (75, 25, 1));
    assert_eq!(source_state.pending()[0].transfer_id, id);
    let req = intake_request(&f.target, handoffs.clone());
    let command = req.frame_input.commands[0].clone();
    commit(&mut f.target, &f.verifier, &f.validator, req);
    let target_state = stored_state(&f.store, [4; 32]);
    assert_eq!(target_state.account(ASSET, recipient()).unwrap().available, 25);
    assert_eq!(target_state.receipts()[0].transfer_id, id);
    assert!(f.store.load_handoff(handoffs[0]).unwrap().unwrap().is_consumed());
    assert!(f.store.load_command_receipt([4; 32], command.id).unwrap().is_some());
    assert_eq!(f.store.pending_outbox([3; 32], 10).unwrap().len(), 1);
    assert_eq!(f.store.pending_outbox([4; 32], 10).unwrap().len(), 1);
    replay(&f.source, &f.store);
    replay(&f.target, &f.store);
    let Fixture { directory, store, verifier, validator: _, source, target } = f;
    let cfg = target.config().clone();
    let head = target.head().clone();
    drop(source);
    drop(target);
    drop(store);
    let store = Arc::new(RocksSessionStore::open(directory.path()).unwrap());
    let snapshot = store.load_checkpoint(cfg.session_id).unwrap().state_snapshot;
    let restored = restore_executor(&cfg, &snapshot, store.as_ref()).unwrap();
    let recovered = SessionChain::recover(cfg.session_id, &cfg.application_profile, verifier, restored, store.clone()).unwrap();
    assert_eq!(*recovered.head(), head);
    replay(&recovered, &store);
    assert!(recovered.prepare_block(intake_request(&recovered, handoffs)).is_err());
}

#[test]
fn copyable_policy_preserves_source_and_never_creates_scarce_target_balances() {
    let mut f = Fixture::new(TransferKind::CopyableReceipt);
    for (version, nonce) in [(0, 1), (1, 2)] {
        let intent = f.intent(nonce, version, 1);
        let req = issue_request(&f.source, vec![intent]);
        let ids = commit(&mut f.source, &f.verifier, &f.validator, req);
        let req = intake_request(&f.target, ids);
        commit(&mut f.target, &f.verifier, &f.validator, req);
    }
    let source = stored_state(&f.store, [3; 32]);
    let a = source.account(ASSET, signer().actor()).unwrap();
    assert_eq!((a.available, a.locked, a.version), (100, 0, 2));
    assert!(source.pending().is_empty());
    let target = stored_state(&f.store, [4; 32]);
    assert!(target.accounts().is_empty());
    assert_eq!(target.receipts().len(), 2);
    assert!(target.receipts().iter().all(|r| r.kind == TransferKind::CopyableReceipt));
    replay(&f.source, &f.store);
    replay(&f.target, &f.store);
}

#[test]
fn policy_targets_compete_in_one_store_and_only_one_receives_value() {
    let mut f = Fixture::new(TransferKind::Scarce);
    let target_state = stored_state(&f.store, [4; 32]);
    let mut other_cfg = f.target.config().clone();
    other_cfg.session_id = [5; 32];
    let other = create_chain(other_cfg, target_state.clone(), f.verifier.clone(), f.store.clone()).unwrap();
    let p = stored_state(&f.store, [3; 32]).policy().clone();
    let intent = signer()
        .sign(
            payload(
                f.source.config(),
                &p,
                TransferTarget::SharedStorePolicy { policy_commitment: target_state.policy().intake_policy().unwrap() },
                1,
                0,
                40,
            ),
            100,
            None,
        )
        .unwrap();
    let req = issue_request(&f.source, vec![intent]);
    let ids = commit(&mut f.source, &f.verifier, &f.validator, req);
    let first_prepared = f.target.prepare_block(intake_request(&f.target, ids.clone())).unwrap();
    let second_prepared = other.prepare_block(intake_request(&other, ids.clone())).unwrap();
    let barrier = Arc::new(Barrier::new(2));
    let results = std::thread::scope(|scope| {
        let joins = [(f.target, first_prepared), (other, second_prepared)]
            .into_iter()
            .map(|(mut chain, p)| {
                let barrier = barrier.clone();
                let signed = proof(&f.verifier, &f.validator, &p);
                scope.spawn(move || {
                    barrier.wait();
                    let result = chain.commit_finalised(p, signed);
                    (chain, result)
                })
            })
            .collect::<Vec<_>>();
        joins.into_iter().map(|j| j.join().unwrap()).collect::<Vec<_>>()
    });
    assert_eq!(results.iter().filter(|(_, r)| r.is_ok()).count(), 1);
    let credited = [4, 5]
        .into_iter()
        .map(|id| stored_state(&f.store, [id; 32]).account(ASSET, recipient()).map_or(0, |a| a.available))
        .sum::<u64>();
    assert_eq!(credited, 40);
    for (chain, result) in &results {
        if result.is_ok() {
            replay(chain, &f.store);
        } else {
            assert_eq!(chain.head().finalised_height, None);
        }
    }
    assert_eq!(stored_state(&f.store, [3; 32]).account(ASSET, signer().actor()).unwrap().available + credited, 100);
}

#[test]
fn changed_signed_fields_wrong_commands_and_duplicate_nonces_are_atomic_failures() {
    let mut f = Fixture::new(TransferKind::Scarce);
    let original = f.intent(1, 0, 10);
    let valid = issue_request(&f.source, vec![original.clone()]);
    for mutation in 0..11 {
        let mut req = valid.clone();
        match mutation {
            0..=6 => {
                let mut p = TransferPayload::decode(&req.handoffs[0].payload).unwrap();
                match mutation {
                    0 => p.recipient = TransferSigner::new([5; 32]).unwrap().actor(),
                    1 => p.amount += 1,
                    2 => p.asset.object[0] ^= 1,
                    3 => p.source_version += 1,
                    4 => p.nonce[0] ^= 1,
                    5 => p.deployment_id[0] ^= 1,
                    6 => p.source.genesis_commitment[0] ^= 1,
                    _ => unreachable!(),
                }
                req.handoffs[0].payload = p.encode().unwrap();
            }
            7 => req.handoffs[0].expires_at_ms += 1,
            8 => req.frame_input.commands[0].id[0] ^= 1,
            9 => req.handoffs.push(original.clone()),
            10 => req.handoffs[0].authorization = vec![3; 20],
            _ => unreachable!(),
        }
        assert!(f.source.prepare_block(req).is_err(), "mutation {mutation}");
        assert_eq!(f.store.load_head([3; 32]).unwrap().finalised_height, None);
        assert!(f.store.pending_outbox([3; 32], 10).unwrap().is_empty());
    }
    let req = issue_request(&f.source, vec![original]);
    commit(&mut f.source, &f.verifier, &f.validator, req);
    let repeat_nonce = f.intent(1, 1, 10);
    let req = issue_request(&f.source, vec![repeat_nonce]);
    assert!(f.source.prepare_block(req).err().unwrap().to_string().contains("nonce already used"));
    let old_version = f.intent(2, 0, 10);
    let req = issue_request(&f.source, vec![old_version]);
    assert!(f.source.prepare_block(req).err().unwrap().to_string().contains("source object version"));
    assert_eq!(f.store.load_head([3; 32]).unwrap().finalised_height, Some(0));
}

#[test]
fn exact_target_genesis_expiry_and_pinned_sources_fail_closed() {
    let mut f = Fixture::new(TransferKind::Scarce);
    let mut p = TransferPayload::decode(&f.intent(1, 0, 10).payload).unwrap();
    let TransferTarget::Session { genesis_commitment, .. } = &mut p.target else { unreachable!() };
    *genesis_commitment = [99; 32];
    let intent = signer().sign(p, 100, None).unwrap();
    let req = issue_request(&f.source, vec![intent]);
    let ids = commit(&mut f.source, &f.verifier, &f.validator, req);
    assert!(f.target.prepare_block(intake_request(&f.target, ids.clone())).err().unwrap().to_string().contains("target genesis"));
    let mut expired = intake_request(&f.target, ids);
    expired.timestamp_ms = 101;
    expired.local_now_ms = 101;
    assert!(f.target.prepare_block(expired).err().unwrap().to_string().contains("expired"));
    let mut wrong_policy = stored_state(&f.store, [4; 32]).policy().clone();
    wrong_policy.accepted_sources[0].genesis_commitment[0] ^= 1;
    assert!(SourceBindings::from_store(&wrong_policy, f.store.as_ref()).is_err());
}

#[test]
fn policy_mismatch_and_copy_to_scarce_promotion_are_rejected() {
    let mut f = Fixture::new(TransferKind::CopyableReceipt);
    let mut scarce_policy = policy(TransferKind::Scarce);
    scarce_policy.accepted_sources.push(SourceIdentity::from_config(f.source.config()).unwrap());
    let state = TransferState::new(scarce_policy, vec![]).unwrap();
    let cfg = config(&f.verifier, 6, &state);
    let target = create_chain(cfg, state, f.verifier.clone(), f.store.clone()).unwrap();
    let source_policy = stored_state(&f.store, [3; 32]).policy().clone();
    let intent = signer()
        .sign(
            payload(
                f.source.config(),
                &source_policy,
                TransferTarget::Session { session_id: [6; 32], genesis_commitment: target.config().commitment() },
                1,
                0,
                1,
            ),
            100,
            None,
        )
        .unwrap();
    let req = issue_request(&f.source, vec![intent]);
    let ids = commit(&mut f.source, &f.verifier, &f.validator, req);
    assert!(target.prepare_block(intake_request(&target, ids)).err().unwrap().to_string().contains("transfer kind"));
    assert!(stored_state(&f.store, [6; 32]).accounts().is_empty());
    let state = stored_state(&f.store, [3; 32]);
    let mut changed = f.source.config().clone();
    changed.protocol_effects = myelin_session::ProtocolEffectsPolicy::Application { policy_hash: [99; 32] };
    assert!(restore_executor(&changed, &state.encode().unwrap(), f.store.as_ref()).is_err());
}

#[test]
fn state_caps_and_late_batch_failure_do_not_leak_partial_updates() {
    let f = Fixture::new(TransferKind::Scarce);
    let first = f.intent(1, 0, 10);
    let second = f.intent(2, 1, 200);
    let req = issue_request(&f.source, vec![first, second]);
    let before = f.source.head().clone();
    assert!(f.source.prepare_block(req).is_err());
    assert_eq!(*f.source.head(), before);
    assert_eq!(stored_state(&f.store, [3; 32]).account(ASSET, signer().actor()).unwrap().available, 100);
    let dir = tempfile::tempdir().unwrap();
    let store = Arc::new(RocksSessionStore::open(dir.path()).unwrap());
    let (v, validator) = consensus();
    let mut p = policy(TransferKind::Scarce);
    p.limits.max_pending = 1;
    p.limits.max_nonces = 1;
    let state = state(p.clone());
    let cfg = config(&v, 3, &state);
    let mut source = create_chain(cfg.clone(), state, v.clone(), store.clone()).unwrap();
    let target = TransferTarget::Session { session_id: [4; 32], genesis_commitment: [5; 32] };
    let intent = signer().sign(payload(&cfg, &p, target.clone(), 1, 0, 10), 100, None).unwrap();
    let req = issue_request(&source, vec![intent]);
    commit(&mut source, &v, &validator, req);
    let intent = signer().sign(payload(&cfg, &p, target, 2, 1, 10), 100, None).unwrap();
    let req = issue_request(&source, vec![intent]);
    assert!(source.prepare_block(req).err().unwrap().to_string().contains("bound exceeded"));
    assert_eq!(stored_state(&store, [3; 32]).account(ASSET, signer().actor()).unwrap().available, 90);
}

#[test]
fn source_state_is_independent_of_batch_boundaries_and_imports_are_canonical() {
    let mut roots = vec![];
    for batched in [false, true] {
        let mut f = Fixture::new(TransferKind::Scarce);
        let first = f.intent(1, 0, 10);
        let second = f.intent(2, 1, 20);
        if batched {
            let req = issue_request(&f.source, vec![first, second]);
            commit(&mut f.source, &f.verifier, &f.validator, req);
        } else {
            for intent in [first, second] {
                let req = issue_request(&f.source, vec![intent]);
                commit(&mut f.source, &f.verifier, &f.validator, req);
            }
        }
        roots.push(f.source.head().state_root);
        replay(&f.source, &f.store);
        let state = stored_state(&f.store, [3; 32]);
        let bytes = state.encode().unwrap();
        assert_eq!(TransferState::decode(&bytes).unwrap(), state);
        let mut bad = bytes.clone();
        bad.push(0);
        assert!(TransferState::decode(&bad).is_err());
        let mut bad = bytes;
        bad[4] = bad[4].checked_add(1).unwrap();
        assert!(TransferState::decode(&bad).is_err());
    }
    assert_eq!(roots[0], roots[1]);
}

#[test]
fn alternative_owner_signature_or_handoff_identity_cannot_duplicate_a_transfer_right() {
    use myelin_session::{ProtocolEffectsContext, TransitionExecutor};
    use secp256k1::{Keypair, Message, Secp256k1, SecretKey};
    let mut f = Fixture::new(TransferKind::Scarce);
    let intent = f.intent(1, 0, 10);
    let transfer_id = verify_intent(&intent).unwrap().0;
    let mut alternative = intent.clone();
    let secp = Secp256k1::new();
    let key = Keypair::from_secret_key(&secp, &SecretKey::from_slice(&[3; 32]).unwrap());
    let signature = secp.sign_schnorr_with_aux_rand(&Message::from_digest(transfer_id), &key, &[42; 32]).serialize();
    let start = alternative.authorization.len() - 64;
    alternative.authorization[start..].copy_from_slice(&signature);
    let hash = *blake3::hash(&alternative.authorization[46..]).as_bytes();
    alternative.authorization[14..46].copy_from_slice(&hash);
    assert_ne!(alternative.authorization, intent.authorization);
    assert_eq!(verify_intent(&alternative).unwrap().0, transfer_id);
    let req = issue_request(&f.source, vec![intent]);
    let ids = commit(&mut f.source, &f.verifier, &f.validator, req);
    let req = intake_request(&f.target, ids.clone());
    commit(&mut f.target, &f.verifier, &f.validator, req);
    assert!(f.source.prepare_block(issue_request(&f.source, vec![alternative.clone()])).is_err());
    let mut aliased = f.store.load_handoff(ids[0]).unwrap().unwrap().handoff;
    aliased.authorization = alternative.authorization;
    assert_ne!(aliased.id(), ids[0]);
    // Exercise the application's additional nullifier defense. Normal session
    // admission also requires the exact source block/marker for this new ID.
    let cfg = f.target.config();
    let snapshot = f.store.load_checkpoint(cfg.session_id).unwrap().state_snapshot;
    let mut executor = restore_executor(cfg, &snapshot, f.store.as_ref()).unwrap();
    let input = command_frame(cfg, 1, 1, vec![TransferCommand::Intake { handoff_id: aliased.id() }]).unwrap();
    let context = ProtocolEffectsContext {
        source_genesis_commitment: cfg.commitment(),
        source_session_id: cfg.session_id,
        policy: cfg.protocol_effects.clone(),
        height: 1,
        timestamp_ms: 10,
        pre_state_root: executor.state_root(),
        input_root: input.root(),
        issued_handoffs: vec![],
        successor_config: vec![],
    };
    let before = executor.snapshot().unwrap();
    assert!(executor
        .execute_transfer(&context, &input, &[aliased], &[])
        .unwrap_err()
        .to_string()
        .contains("nullifier already credited"));
    assert_eq!(executor.snapshot().unwrap(), before);
}
