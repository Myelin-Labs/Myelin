#[allow(dead_code)]
#[path = "../examples/support/remote_transfer.rs"]
mod fixture;
use fixture::*;
use myelin_session::*;
use myelin_session_transfer::*;
use std::sync::Arc;

fn source_state(store: &myelin_session_store_rocksdb::RocksSessionStore, d: &Deployment) -> TransferState {
    TransferState::decode(&store.load_checkpoint(d.source.session_id).unwrap().state_snapshot).unwrap()
}
fn cancel_authorization(d: &Deployment, h: &SessionHandoff) -> Vec<u8> {
    let TransferCommand::Cancel { authorization, .. } = owner().cancel(&d.source, h).unwrap() else { unreachable!() };
    authorization
}
fn source_commit(
    d: &Deployment,
    source: &mut transfer::Chain,
    command: TransferCommand,
    action: HandoffAction,
) -> myelin_session_transfer::Result<()> {
    let mut request = transfer::req(source, vec![command]).map_err(|e| Error::State(e.to_string()))?;
    request.handoff_actions.push(action);
    let (_, signer) = transfer::consensus("remote-source", [7; 32]).unwrap();
    transfer::commit(source, &d.source_finality, &signer, request).map_err(|e| Error::State(e.to_string()))
}
fn refund(
    d: &Deployment,
    source: &mut transfer::Chain,
    h: &SessionHandoff,
    certificate: &RemoteTerminalCertificate,
) -> myelin_session_transfer::Result<()> {
    let authorization = cancel_authorization(d, h);
    source_commit(
        d,
        source,
        TransferCommand::RemoteCancel { handoff_id: h.id(), authorization, certificate: certificate.encode().unwrap() },
        HandoffAction::Cancel { handoff_id: h.id() },
    )?;
    source_commit(d, source, TransferCommand::Reclaim { handoff_id: h.id() }, HandoffAction::Reclaim { handoff_id: h.id() })
}

#[test]
fn lost_consumption_ack_reopens_original_candidate_and_never_creates_a_refund_right() {
    let root = tempfile::tempdir().unwrap();
    let root = root.path();
    let d = Deployment::new(program_digest().unwrap()).unwrap();
    let (mut source, store) = d.source_chain(&root.join("source")).unwrap();
    let id = d.issue(&mut source, 1, 0).unwrap();
    let handoff = store.load_handoff(id).unwrap().unwrap().handoff;
    let proof = d.proof(&store, id).unwrap();
    let target = d.target_authority(&root.join("target")).unwrap();
    target.ledger.import_handoff(&proof).unwrap();
    assert!(target.authority.head().unwrap().finalised_height.is_none());
    let candidate = target.prepare(&d.target, id, 1).unwrap();
    assert_eq!(target.authority.pending_remote_candidate().unwrap().unwrap(), candidate);
    // While the external decision is unknown, no replacement writes or grants.
    assert!(target.prepare(&d.target, id, 2).is_err());
    let grant = authority_key(11)
        .authorize_grant(target.authority.policy(), 1, authority_key(14).public_key(), &target.authority.head().unwrap())
        .unwrap();
    assert!(target.authority.grant(grant).is_err());
    drop(target);
    let terminal = d.terminal_create(&root.join("terminal")).unwrap();
    let lost_reply = terminal.authorize_consume(&proof, &candidate).unwrap();
    drop(terminal); // durable decision exists, client has no usable reply
    let target = d.target_authority(&root.join("target")).unwrap();
    assert_eq!(target.authority.pending_remote_candidate().unwrap().unwrap(), candidate);
    let terminal = d.terminal_open(&root.join("terminal")).unwrap();
    let certificate = terminal.lookup(id).unwrap().unwrap();
    assert_eq!(certificate, lost_reply);
    assert!(terminal.authorize_cancel(&proof, &cancel_authorization(&d, &handoff)).is_err());
    let original_receipt = target.authority.commit_remote(&certificate).unwrap();
    original_receipt.verify(target.authority.policy()).unwrap();
    assert!(target.authority.pending_remote_candidate().unwrap().is_none());
    let record = target.authority.block(0).unwrap();
    assert_eq!(TransferState::decode(&record.state_snapshot).unwrap().accounts()[0].available, 25);
    drop(target); // lose the actual target commit ACK
    let target = d.target_authority(&root.join("target")).unwrap();
    assert_eq!(target.authority.commit_remote(&certificate).unwrap(), original_receipt);
    assert!(refund(&d, &mut source, &handoff, &certificate).is_err());
    assert!(source_commit(&d, &mut source, owner().cancel(&d.source, &handoff).unwrap(), HandoffAction::Cancel { handoff_id: id })
        .is_err());
    let locked = source_state(&store, &d);
    assert_eq!((locked.accounts()[0].available, locked.accounts()[0].locked), (75, 25));
    // Complete with retained foreign evidence, after target is unavailable.
    let completion = RemoteConsumptionProof { import_policy: d.import_policy, target_record: record.encode().unwrap() };
    drop(target);
    std::fs::remove_dir_all(root.join("target")).unwrap();
    source_commit(
        &d,
        &mut source,
        TransferCommand::Retire { handoff_id: id },
        HandoffAction::RemoteRetire { handoff_id: id, proof: completion },
    )
    .unwrap();
    let finished = source_state(&store, &d);
    assert_eq!((finished.accounts()[0].available, finished.accounts()[0].locked), (75, 0));
    assert!(finished.pending().is_empty());
    let cfg = d.source.clone();
    let registry = d.registry.clone();
    let replay = source
        .replay_range(0, 1, move |bytes| {
            let state = TransferState::decode(bytes).map_err(|e| SessionError::Replay(e.to_string()))?;
            TransferExecutor::restore(
                &cfg,
                bytes,
                SourceBindings::from_registry(state.policy(), registry).map_err(|e| SessionError::Replay(e.to_string()))?,
            )
            .map_err(|e| SessionError::Replay(e.to_string()))
        })
        .unwrap();
    assert_eq!(replay.state_root_after, source.head().state_root);
}

#[test]
fn cancellation_is_terminal_even_for_old_timestamps_delayed_proofs_and_target_restart() {
    let root = tempfile::tempdir().unwrap();
    let root = root.path();
    let d = Deployment::new(program_digest().unwrap()).unwrap();
    let (mut source, store) = d.source_chain(&root.join("source")).unwrap();
    let id = d.issue(&mut source, 1, 0).unwrap();
    let handoff = store.load_handoff(id).unwrap().unwrap().handoff;
    let proof = d.proof(&store, id).unwrap();
    let target = d.target_authority(&root.join("target")).unwrap();
    target.ledger.import_handoff(&proof).unwrap();
    let candidate = target.prepare(&d.target, id, 1).unwrap();
    let terminal = d.terminal_create(&root.join("terminal")).unwrap();
    let mut wrong_owner = cancel_authorization(&d, &handoff);
    wrong_owner[0] ^= 1;
    assert!(terminal.authorize_cancel(&proof, &wrong_owner).is_err());
    assert!(terminal.lookup(id).unwrap().is_none());
    let cancel = terminal.authorize_cancel(&proof, &cancel_authorization(&d, &handoff)).unwrap();
    assert!(terminal.authorize_consume(&proof, &candidate).is_err());
    assert!(target.authority.commit_remote(&cancel).is_err());
    target.authority.release_cancelled_remote(&cancel).unwrap();
    drop(target);
    drop(terminal);
    let terminal = d.terminal_open(&root.join("terminal")).unwrap();
    assert_eq!(terminal.authorize_cancel(&proof, &cancel_authorization(&d, &handoff)).unwrap(), cancel);
    let target = d.target_authority(&root.join("target")).unwrap();
    assert!(target.authority.head().unwrap().finalised_height.is_none());
    target.ledger.import_handoff(&proof).unwrap(); // delayed at-least-once relay
    let delayed = target.prepare(&d.target, id, 1).unwrap();
    assert_eq!(candidate, delayed);
    assert!(terminal.authorize_consume(&proof, &delayed).is_err());
    target.authority.release_cancelled_remote(&cancel).unwrap();
    refund(&d, &mut source, &handoff, &cancel).unwrap();
    let state = source_state(&store, &d);
    assert_eq!((state.accounts()[0].available, state.accounts()[0].locked), (100, 0));
    assert!(refund(&d, &mut source, &handoff, &cancel).is_err());
    // A missing coordinator is an outage, never a fresh empty authority.
    assert!(d.terminal_open(&root.join("missing")).is_err());
    assert!(!root.join("missing").exists());
    assert!(d.terminal_create(&root.join("terminal")).is_err());
}

#[test]
fn two_target_copies_and_owner_cancel_race_at_one_durable_irreversible_decision() {
    for first in 0..3 {
        let root = tempfile::tempdir().unwrap();
        let root = root.path();
        let d = Deployment::new(program_digest().unwrap()).unwrap();
        let (mut source, store) = d.source_chain(&root.join("source")).unwrap();
        let id = d.issue(&mut source, 1, 0).unwrap();
        let h = store.load_handoff(id).unwrap().unwrap().handoff;
        let proof = Arc::new(d.proof(&store, id).unwrap());
        let a = d.target_authority(&root.join("target-a")).unwrap();
        let b = d.target_authority(&root.join("target-b")).unwrap();
        a.ledger.import_handoff(&proof).unwrap();
        b.ledger.import_handoff(&proof).unwrap();
        let candidates = [a.prepare(&d.target, id, 1).unwrap(), b.prepare(&d.target, id, 2).unwrap()];
        assert_ne!(candidates[0], candidates[1]);
        let terminal = Arc::new(d.terminal_create(&root.join("terminal")).unwrap());
        let authorization = Arc::new(cancel_authorization(&d, &h));
        // Cover both forced orderings plus a real simultaneous three-way race.
        if first == 0 {
            terminal.authorize_consume(&proof, &candidates[0]).unwrap();
        }
        if first == 1 {
            terminal.authorize_cancel(&proof, &authorization).unwrap();
        }
        let barrier = Arc::new(std::sync::Barrier::new(3));
        let results = std::thread::scope(|scope| {
            let mut jobs = vec![];
            for (i, candidate) in candidates.iter().enumerate() {
                let terminal = terminal.clone();
                let proof = proof.clone();
                let barrier = barrier.clone();
                jobs.push(scope.spawn(move || {
                    barrier.wait();
                    (i, terminal.authorize_consume(&proof, candidate))
                }));
            }
            let terminal = terminal.clone();
            let proof = proof.clone();
            let barrier = barrier.clone();
            let authorization = authorization.clone();
            jobs.push(scope.spawn(move || {
                barrier.wait();
                (2, terminal.authorize_cancel(&proof, &authorization))
            }));
            jobs.into_iter().map(|job| job.join().unwrap()).collect::<Vec<_>>()
        });
        assert_eq!(results.iter().filter(|(_, result)| result.is_ok()).count(), 1);
        let original = terminal.lookup(id).unwrap().unwrap();
        drop(terminal);
        let terminal = d.terminal_open(&root.join("terminal")).unwrap();
        assert_eq!(terminal.lookup(id).unwrap().unwrap(), original);
        match &original.decision {
            RemoteTerminalDecision::Cancelled { .. } => {
                assert!(a.authority.commit_remote(&original).is_err());
                assert!(b.authority.commit_remote(&original).is_err());
                refund(&d, &mut source, &h, &original).unwrap();
                assert_eq!(source_state(&store, &d).accounts()[0].available, 100);
            }
            RemoteTerminalDecision::ConsumeAuthorized { candidate } => {
                let index = candidates
                    .iter()
                    .position(|bytes| FinalisedBlockRecord::decode(bytes).unwrap().block.hash() == candidate.block_hash)
                    .unwrap();
                let targets = [&a, &b];
                targets[index].authority.commit_remote(&original).unwrap();
                assert!(targets[1 - index].authority.commit_remote(&original).is_err());
                assert!(refund(&d, &mut source, &h, &original).is_err());
                assert_eq!(source_state(&store, &d).accounts()[0].available, 75);
            }
        }
    }
}

#[test]
fn matching_opaque_domain_cannot_import_a_source_with_shared_store_refund_semantics() {
    let root = tempfile::tempdir().unwrap();
    let mut d = Deployment::new(program_digest().unwrap()).unwrap();
    let mut shared = d.source_state.policy().clone();
    shared.coordination = TransferCoordination::SharedStore;
    // The opaque domain stays byte-identical to the remote terminal commitment.
    assert_eq!(shared.uniqueness_domain, d.target_state.policy().uniqueness_domain);
    d.source_state = TransferState::new(shared.clone(), d.source_state.accounts().to_vec()).unwrap();
    d.source = transfer::config(&d.source_finality, 3, &d.source_state, d.source.application_profile.program_digest).unwrap();
    let mut intake = d.target_state.policy().clone();
    intake.accepted_sources = vec![SourceIdentity::from_config(&d.source).unwrap()];
    d.target_state = TransferState::new(intake.clone(), vec![]).unwrap();
    d.target = transfer::config(&d.target_finality, 4, &d.target_state, d.target.application_profile.program_digest).unwrap();
    let store = Arc::new(myelin_session_store_rocksdb::RocksSessionStore::open(root.path()).unwrap());
    let mut source = transfer::open_chain(d.source.clone(), d.source_state.clone(), d.source_finality.clone(), store.clone()).unwrap();
    let id = d.issue(&mut source, 1, 0).unwrap();
    let proof = HandoffProofBundle::export(store.as_ref(), id, &d.target).unwrap();
    assert!(HandoffImportPolicy::new(&d.source, &d.target, PROOF_LIMIT).is_err());
    // A caller can also supply policy fields directly: proof verification must
    // enforce the kernel scope independently of that constructor.
    let policy = HandoffImportPolicy {
        source_genesis: d.source.commitment(),
        target_genesis: d.target.commitment(),
        max_proof_bytes: PROOF_LIMIT as u64,
    };
    let verifier = TransferImportVerifier::new(intake, vec![shared], Arc::new(|_, _, _, _| unreachable!())).unwrap();
    assert!(proof.verify(&policy, &d.source, &d.target, d.source_finality.as_ref(), &verifier).is_err());
    // The source really can cancel under its shared-store semantics. Remote
    // target admission must reject it before credit or coordination is possible.
    let h = store.load_handoff(id).unwrap().unwrap().handoff;
    source_commit(&d, &mut source, owner().cancel(&d.source, &h).unwrap(), HandoffAction::Cancel { handoff_id: id }).unwrap();
    source_commit(&d, &mut source, TransferCommand::Reclaim { handoff_id: id }, HandoffAction::Reclaim { handoff_id: id }).unwrap();
    assert_eq!(source_state(&store, &d).accounts()[0].available, 100);
}

#[test]
fn copied_source_history_in_an_unregistered_target_store_still_requires_terminal_authorization() {
    fn copy_tree(source: &std::path::Path, destination: &std::path::Path) {
        std::fs::create_dir(destination).unwrap();
        for entry in std::fs::read_dir(source).unwrap() {
            let entry = entry.unwrap();
            if entry.file_type().unwrap().is_dir() {
                copy_tree(&entry.path(), &destination.join(entry.file_name()));
            } else {
                std::fs::copy(entry.path(), destination.join(entry.file_name())).unwrap();
            }
        }
    }
    let root = tempfile::tempdir().unwrap();
    let d = Deployment::new(program_digest().unwrap()).unwrap();
    let (mut source, store) = d.source_chain(&root.path().join("source")).unwrap();
    let id = d.issue(&mut source, 1, 0).unwrap();
    let source_proof = d.proof(&store, id).unwrap();
    let handoff = store.load_handoff(id).unwrap().unwrap().handoff;
    drop(source);
    drop(store);
    copy_tree(&root.path().join("source"), &root.path().join("copied-source"));
    // The target has genuine copied source blocks but no configured inbox. A
    // typed remote policy must not silently become shared-store uniqueness.
    let copied = Arc::new(myelin_session_store_rocksdb::RocksSessionStore::open(root.path().join("copied-source")).unwrap());
    assert!(copied.handoff_import_registry().is_none());
    let executor = TransferExecutor::new(
        &d.target,
        d.target_state.clone(),
        SourceBindings::from_registry(d.target_state.policy(), d.registry.clone()).unwrap(),
    )
    .unwrap();
    let mut target = SessionChain::create(d.target.clone(), d.target_finality.clone(), executor, copied).unwrap();
    let mut request = request(&d.target, target.head(), vec![TransferCommand::Intake { handoff_id: id }]).unwrap();
    request.consume_handoffs.push(id);
    let prepared = target.prepare_block(request).unwrap();
    let (_, signer) = transfer::consensus("remote-target", [8; 32]).unwrap();
    use myelin_consensus::{FinalityProof, SelectedConsensus};
    let SelectedConsensus::ProofOfAuthority(engine) = d.target_finality.selected_consensus() else { unreachable!() };
    let proof =
        FinalityProof::ProofOfAuthority(engine.seal_from_signer(prepared.block().hash(), prepared.block().number, &signer).unwrap());
    assert!(target.commit_finalised(prepared, proof).is_err(), "copied source placement must not bypass the terminal certificate");
    assert!(target.head().finalised_height.is_none());
    let terminal = d.terminal_create(&root.path().join("terminal")).unwrap();
    let cancellation = terminal.authorize_cancel(&source_proof, &cancel_authorization(&d, &handoff)).unwrap();
    let (mut source, store) = d.source_chain(&root.path().join("source")).unwrap();
    refund(&d, &mut source, &handoff, &cancellation).unwrap();
    assert_eq!(source_state(&store, &d).accounts()[0].available, 100);
}
