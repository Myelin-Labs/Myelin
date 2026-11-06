//! Dedicated continuity gates: every negative fixture violates one condition.
use super::*;
use myelin_session::{
    CommandIdentity, CommandStatus, EvidencePipelineDescriptor, EvidenceReceipt, EvidenceRecord, EvidenceStage, Hash32, OutboxMessage,
    PreparedBlock, SessionError, SessionHead, StoredHandoff,
};
use std::{path::Path, process::Command, sync::Barrier};

type Chain = SessionChain<TestExecutor, RocksSessionStore>;

struct HandoffIds {
    ordinary: Hash32,
    gated: Hash32,
    expiring: Hash32,
    policy: Hash32,
}

struct Fixture {
    store: Arc<RocksSessionStore>,
    consensus: Arc<RegisteredFinalityVerifier>,
    signer: CommitteeSigner,
    target: Chain,
    ids: HandoffIds,
    evidence_message: OutboxMessage,
    pipeline: EvidencePipelineDescriptor,
}

fn command(position: u64) -> CommandIdentity {
    CommandIdentity { id: [40 + position as u8; 32], payload_hash: [80 + position as u8; 32] }
}

fn request(position: u64, timestamp_ms: u64, consume_handoffs: Vec<Hash32>) -> BlockRequest {
    let mut input = frame_input(position);
    input.commands = vec![command(position)];
    BlockRequest {
        transactions: vec![transaction(position as u8)],
        frame_input: input,
        scheduler_commitment: [5; 32],
        data_commitments: vec![],
        timestamp_ms,
        local_now_ms: timestamp_ms,
        successor: None,
        handoffs: vec![],
        consume_handoffs,
        handoff_actions: vec![],
    }
}

fn proof(fixture: &Fixture, prepared: &PreparedBlock<TestExecutor>) -> FinalityProof {
    seal(&fixture.consensus, &fixture.signer, prepared)
}

fn seal(consensus: &RegisteredFinalityVerifier, signer: &CommitteeSigner, prepared: &PreparedBlock<TestExecutor>) -> FinalityProof {
    let myelin_consensus::SelectedConsensus::ProofOfAuthority(engine) = consensus.selected_consensus() else { unreachable!() };
    FinalityProof::ProofOfAuthority(engine.seal_from_signer(prepared.block().hash(), prepared.block().number, signer).unwrap())
}

impl Fixture {
    fn open(path: &Path) -> Self {
        let store = Arc::new(RocksSessionStore::open(path).unwrap());
        let (consensus, signer) = consensus();
        let mut cfg = config(&consensus);
        cfg.application_profile.command_identity_required = true;
        let mut source = SessionChain::create(cfg.clone(), consensus.clone(), TestExecutor { root: [1; 32] }, store.clone()).unwrap();
        cfg.session_id = [4; 32];
        let target = SessionChain::create(cfg, consensus.clone(), TestExecutor { root: [1; 32] }, store.clone()).unwrap();

        // A real source outbox message exists before the handoff binds its ID.
        let bootstrap = source.prepare_block(request(0, 10, vec![])).unwrap();
        let evidence_message = bootstrap.outbox()[0].clone();
        let bootstrap_proof = seal(&consensus, &signer, &bootstrap);
        source.commit_finalised(bootstrap, bootstrap_proof).unwrap();
        let pipeline = EvidencePipelineDescriptor::data_availability(evidence_message.topic.clone(), [[91; 32], [92; 32]]).unwrap();
        let intent = |payload: &[u8], expires_at_ms, target, evidence_requirement| HandoffIntent {
            target,
            expires_at_ms,
            payload: payload.to_vec(),
            authorization: b"test-application-policy".to_vec(),
            evidence_requirement,
        };
        let mut issue = request(1, 11, vec![]);
        issue.handoffs = vec![
            intent(b"ordinary", 100, HandoffTarget::Session([4; 32]), None),
            intent(
                b"gated",
                100,
                HandoffTarget::Session([4; 32]),
                Some(EvidenceRequirement {
                    message_id: evidence_message.id,
                    pipeline_commitment: pipeline.commitment(),
                    minimum_stage_index: 1,
                }),
            ),
            intent(b"expiring", 15, HandoffTarget::Session([4; 32]), None),
            intent(b"policy", 100, HandoffTarget::Policy(target.config().application_profile.handoff_policy_hash), None),
        ];
        let prepared = source.prepare_block(issue).unwrap();
        let named_id = |name: &[u8]| prepared.issued_handoffs().iter().find(|h| h.payload == name).unwrap().id();
        let ids = HandoffIds {
            ordinary: named_id(b"ordinary"),
            gated: named_id(b"gated"),
            expiring: named_id(b"expiring"),
            policy: named_id(b"policy"),
        };
        let source_proof = seal(&consensus, &signer, &prepared);
        source.commit_finalised(prepared, source_proof).unwrap();
        Self { store, consensus, signer, target, ids, evidence_message, pipeline }
    }

    fn assert_uncommitted(&self, target: &Chain, id: Hash32) {
        let session = target.config().session_id;
        assert_eq!(target.head().finalised_height, None);
        assert_eq!(self.store.load_head(session).unwrap(), *target.head());
        assert_eq!(self.store.load_checkpoint(session).unwrap().state_root, [1; 32]);
        assert!(self.store.load_chain_page(session, 0, 2).unwrap().is_empty());
        assert_eq!(target.command_status(&command(0)).unwrap(), CommandStatus::Missing);
        assert!(self.store.pending_outbox(session, 10).unwrap().is_empty());
        assert!(!self.store.load_handoff(id).unwrap().unwrap().is_consumed());
    }

    fn reject(&self, id: Hash32, timestamp: u64, reason: &str) {
        let error = self.target.prepare_block(request(0, timestamp, vec![id])).err().expect("intake must reject");
        assert!(matches!(error, SessionError::Continuity(ref actual) if actual == reason), "{error}");
        self.assert_uncommitted(&self.target, id);
    }

    // This fixture models records already checked by an evidence worker. It
    // tests the consumption gate, not a DA provider or a deployed L1 verifier.
    fn publish_stages(&self, descriptor: &EvidencePipelineDescriptor, count: usize) -> EvidenceRecord {
        let mut current = self.store.load_evidence([3; 32], self.evidence_message.id).unwrap();
        for index in current.as_ref().map_or(0, |record| record.receipts.len())..count {
            let previous = current
                .as_ref()
                .and_then(EvidenceRecord::latest_receipt)
                .map_or([0; 32], |receipt| receipt.commitment(descriptor.commitment(), self.evidence_message.commitment()));
            let receipt = EvidenceReceipt::new(descriptor, index, previous, vec![index as u8 + 1]).unwrap();
            let next = EvidenceRecord::append(current.as_ref(), [3; 32], &self.evidence_message, descriptor, receipt).unwrap();
            self.store.compare_and_set_evidence(current.as_ref().map(|record| record.revision), &next).unwrap();
            current = Some(next);
        }
        current.unwrap()
    }

    fn consume(&mut self, id: Hash32, timestamp: u64) -> SessionHead {
        let prepared = self.target.prepare_block(request(0, timestamp, vec![id])).unwrap();
        let expected_root = prepared.block().state_root_after;
        let expected_outbox = prepared.outbox().to_vec();
        let signed = proof(self, &prepared);
        let head = self.target.commit_finalised(prepared, signed).unwrap();
        assert_committed(&self.store, &self.target, id, &head);
        assert_eq!(head.state_root, expected_root);
        assert_eq!(
            self.store.pending_outbox(head.session_id, 10).unwrap().into_iter().map(|entry| entry.message).collect::<Vec<_>>(),
            expected_outbox
        );
        head
    }
}

fn assert_committed(store: &RocksSessionStore, target: &Chain, id: Hash32, head: &SessionHead) {
    assert_eq!(target.head(), head);
    assert_eq!(store.load_head(head.session_id).unwrap(), *head);
    let checkpoint = store.load_checkpoint(head.session_id).unwrap();
    assert_eq!(checkpoint.state_root, head.state_root);
    assert_eq!(checkpoint.state_snapshot, head.state_root);
    let stored = store.load_handoff(id).unwrap().unwrap();
    assert_eq!(stored.consumption().map(|at| at.session_id), Some(head.session_id));
    assert_eq!(stored.consumption().map(|at| at.height), Some(0));
    assert_eq!(stored.consumption().map(|at| at.block_hash), Some(head.block_hash));
    let CommandStatus::Committed(receipt) = target.command_status(&command(0)).unwrap() else {
        panic!("consumption must commit its command receipt")
    };
    assert_eq!(receipt.block_hash, head.block_hash);
    assert_eq!(store.load_chain_page(head.session_id, 0, 2).unwrap().len(), 1);
    assert_eq!(store.pending_outbox(head.session_id, 10).unwrap().len(), 1);
}

#[test]
fn handoff_wrong_target_rejects_an_unexpired_ungated_claim() {
    let directory = tempfile::tempdir().unwrap();
    let fixture = Fixture::open(directory.path());
    let mut cfg = fixture.target.config().clone();
    cfg.session_id = [5; 32];
    let wrong = Chain::create(cfg, fixture.consensus.clone(), TestExecutor { root: [1; 32] }, fixture.store.clone()).unwrap();
    let error = wrong.prepare_block(request(0, 20, vec![fixture.ids.ordinary])).err().unwrap();
    assert!(matches!(error, SessionError::Continuity(ref reason) if reason == "handoff target policy mismatch or handoff expired"));
    fixture.assert_uncommitted(&wrong, fixture.ids.ordinary);
}

#[test]
fn handoff_expiry_rejects_only_after_the_inclusive_timestamp_boundary() {
    let directory = tempfile::tempdir().unwrap();
    let mut fixture = Fixture::open(directory.path());
    fixture.reject(fixture.ids.expiring, 16, "handoff target policy mismatch or handoff expired");
    // There is no evidence requirement; only the proposed timestamp differs.
    fixture.consume(fixture.ids.expiring, 15);
}

#[test]
fn handoff_evidence_requires_the_exact_pipeline_and_minimum_stage() {
    let directory = tempfile::tempdir().unwrap();
    let mut fixture = Fixture::open(directory.path());
    fixture.reject(fixture.ids.gated, 20, "required handoff evidence is absent");
    fixture.publish_stages(&fixture.pipeline, 1);
    fixture.reject(fixture.ids.gated, 20, "required handoff evidence stage has not been reached");
    fixture.publish_stages(&fixture.pipeline, 2);
    fixture.consume(fixture.ids.gated, 20);
}

#[test]
fn handoff_sufficient_receipts_from_another_pipeline_do_not_satisfy_the_gate() {
    let directory = tempfile::tempdir().unwrap();
    let fixture = Fixture::open(directory.path());
    let wrong = EvidencePipelineDescriptor::data_availability(fixture.evidence_message.topic.clone(), [[93; 32], [94; 32]]).unwrap();
    fixture.publish_stages(&wrong, 2);
    fixture.reject(fixture.ids.gated, 20, "required handoff evidence stage has not been reached");
}

#[test]
fn handoff_failed_finality_and_duplicate_intake_leave_every_effect_uncommitted() {
    let directory = tempfile::tempdir().unwrap();
    let mut fixture = Fixture::open(directory.path());
    let id = fixture.ids.ordinary;
    let error = fixture.target.prepare_block(request(0, 20, vec![id, id])).err().unwrap();
    assert!(matches!(error, SessionError::Continuity(ref reason) if reason == "duplicate handoff consumption id"));
    let prepared = fixture.target.prepare_block(request(0, 20, vec![id])).unwrap();
    let other = fixture.target.prepare_block(request(0, 21, vec![id])).unwrap();
    let wrong_proof = proof(&fixture, &other);
    assert!(matches!(fixture.target.commit_finalised(prepared, wrong_proof), Err(SessionError::Consensus(_))));
    fixture.assert_uncommitted(&fixture.target, id);
    let head = fixture.consume(id, 20);
    let error = fixture.target.prepare_block(request(1, 21, vec![id])).err().unwrap();
    assert!(matches!(error, SessionError::Continuity(ref reason) if reason == "handoff identity mismatch or already consumed"));
    assert_committed(&fixture.store, &fixture.target, id, &head);
}

#[test]
fn handoff_commit_rechecks_evidence_after_prepare() {
    let directory = tempfile::tempdir().unwrap();
    let mut fixture = Fixture::open(directory.path());
    fixture.publish_stages(&fixture.pipeline, 2);
    let prepared = fixture.target.prepare_block(request(0, 20, vec![fixture.ids.gated])).unwrap();
    let signed = proof(&fixture, &prepared);

    // Simulate replacement by an evidence writer between prepare and commit.
    // A well-formed longer record still belongs to the wrong pipeline.
    let mut changed = fixture.pipeline.clone();
    changed.pipeline_name = "replacement-pipeline".to_owned();
    changed.stages.push(EvidenceStage { name: "extra-stage".to_owned(), verifier_commitment: [95; 32] });
    let mut replacement: Option<EvidenceRecord> = None;
    for index in 0..3 {
        let previous = replacement
            .as_ref()
            .and_then(EvidenceRecord::latest_receipt)
            .map_or([0; 32], |receipt| receipt.commitment(changed.commitment(), fixture.evidence_message.commitment()));
        let receipt = EvidenceReceipt::new(&changed, index, previous, vec![1]).unwrap();
        replacement =
            Some(EvidenceRecord::append(replacement.as_ref(), [3; 32], &fixture.evidence_message, &changed, receipt).unwrap());
    }
    fixture.store.compare_and_set_evidence(Some(1), &replacement.unwrap()).unwrap();
    assert!(matches!(
        fixture.target.commit_finalised(prepared, signed),
        Err(SessionError::Store(StoreError::Conflict(ref reason))) if reason == "required handoff evidence stage has not been reached"
    ));
    fixture.assert_uncommitted(&fixture.target, fixture.ids.gated);
}

#[test]
fn handoff_policy_targets_compete_atomically_after_both_prepare() {
    let directory = tempfile::tempdir().unwrap();
    let fixture = Fixture::open(directory.path());
    let mut cfg = fixture.target.config().clone();
    cfg.session_id = [6; 32];
    let other = Chain::create(cfg, fixture.consensus.clone(), TestExecutor { root: [1; 32] }, fixture.store.clone()).unwrap();
    let id = fixture.ids.policy;
    let barrier = Arc::new(Barrier::new(2));
    let results = std::thread::scope(|scope| {
        let handles: Vec<_> = [fixture.target, other]
            .into_iter()
            .map(|mut chain| {
                let barrier = barrier.clone();
                let consensus = fixture.consensus.clone();
                let signer = fixture.signer.clone();
                scope.spawn(move || {
                    let prepared = chain.prepare_block(request(0, 20, vec![id])).unwrap();
                    let signed = seal(&consensus, &signer, &prepared);
                    barrier.wait();
                    let result = chain.commit_finalised(prepared, signed);
                    (chain, result)
                })
            })
            .collect();
        handles.into_iter().map(|handle| handle.join().unwrap()).collect::<Vec<_>>()
    });
    assert_eq!(results.iter().filter(|(_, result)| result.is_ok()).count(), 1);
    for (chain, result) in results {
        match result {
            Ok(head) => assert_committed(&fixture.store, &chain, id, &head),
            Err(error) => {
                assert!(matches!(error, SessionError::Store(StoreError::Conflict(_))), "{error}");
                let session = chain.config().session_id;
                assert_eq!(chain.head().finalised_height, None);
                assert_eq!(fixture.store.load_head(session).unwrap(), *chain.head());
                assert_eq!(chain.command_status(&command(0)).unwrap(), CommandStatus::Missing);
                assert!(fixture.store.pending_outbox(session, 10).unwrap().is_empty());
                assert!(fixture.store.load_chain_page(session, 0, 2).unwrap().is_empty());
                assert_eq!(fixture.store.load_checkpoint(session).unwrap().state_root, [1; 32]);
            }
        }
    }
}

#[test]
fn handoff_crash_child() {
    let Some(path) = std::env::var_os("MYELIN_HANDOFF_CRASH_TEST_PATH") else { return };
    let mut fixture = Fixture::open(Path::new(&path));
    let prepared = fixture.target.prepare_block(request(0, 20, vec![fixture.ids.ordinary])).unwrap();
    let signed = proof(&fixture, &prepared);
    if std::env::var("MYELIN_HANDOFF_CRASH_TEST_PHASE").unwrap() == "after-commit" {
        fixture.target.commit_finalised(prepared, signed).unwrap();
    }
    // Exit without running any RocksDB or session destructors, before ACK.
    std::process::exit(73);
}

#[test]
fn handoff_process_exit_before_and_after_commit_recovers_one_atomic_outcome() {
    for phase in ["before-commit", "after-commit"] {
        let directory = tempfile::tempdir().unwrap();
        let output = Command::new(std::env::current_exe().unwrap())
            .args(["--exact", "handoff_regressions::handoff_crash_child", "--nocapture"])
            .env("MYELIN_HANDOFF_CRASH_TEST_PATH", directory.path())
            .env("MYELIN_HANDOFF_CRASH_TEST_PHASE", phase)
            .output()
            .unwrap();
        assert_eq!(output.status.code(), Some(73), "child failed: {output:?}");
        let store = Arc::new(RocksSessionStore::open(directory.path()).unwrap());
        let (consensus, signer) = consensus();
        let genesis = store.load_genesis([4; 32]).unwrap();
        let (_, snapshot) = Chain::recovery_snapshot(&store, [4; 32]).unwrap();
        let mut target = Chain::recover(
            [4; 32],
            &genesis.config.application_profile,
            consensus.clone(),
            TestExecutor::from_snapshot(&snapshot),
            store.clone(),
        )
        .unwrap();
        let source = store.load_block([3; 32], 1).unwrap();
        let id = source.issued_handoffs.iter().find(|handoff| handoff.payload == b"ordinary").unwrap().id();
        if phase == "before-commit" {
            assert_eq!(target.command_status(&command(0)).unwrap(), CommandStatus::Missing);
            assert_eq!(target.head().finalised_height, None);
            assert!(!store.load_handoff(id).unwrap().unwrap().is_consumed());
            assert!(store.pending_outbox([4; 32], 10).unwrap().is_empty());
            let prepared = target.prepare_block(request(0, 20, vec![id])).unwrap();
            let signed = seal(&consensus, &signer, &prepared);
            target.commit_finalised(prepared, signed).unwrap();
        }
        assert_committed(&store, &target, id, target.head());
        let error = target.prepare_block(request(1, 21, vec![id])).err().unwrap();
        assert!(matches!(error, SessionError::Continuity(ref reason) if reason == "handoff identity mismatch or already consumed"));
        let replay = target.replay_range(0, 0, |snapshot| Ok(TestExecutor::from_snapshot(snapshot))).unwrap();
        assert_eq!(replay.state_root_after, target.head().state_root);
    }
}

#[test]
fn handoff_recovery_rejects_missing_reset_and_forged_consumption_references() {
    for corruption in ["missing", "reset", "wrong-block", "wrong-session", "wrong-height"] {
        let directory = tempfile::tempdir().unwrap();
        let mut fixture = Fixture::open(directory.path());
        let id = fixture.ids.ordinary;
        fixture.consume(id, 20);
        let profile = fixture.target.config().application_profile.clone();
        let snapshot = fixture.store.load_checkpoint([4; 32]).unwrap().state_snapshot;
        let mut stored: StoredHandoff = fixture.store.load_handoff(id).unwrap().unwrap();
        drop(fixture);
        let db: OptimisticTransactionDB = OptimisticTransactionDB::open(&Options::default(), directory.path()).unwrap();
        let mut key = vec![b'j'];
        key.extend_from_slice(&id);
        match corruption {
            "missing" => {
                db.delete(&key).unwrap();
            }
            _ => {
                match corruption {
                    "reset" => stored.lifecycle.terminal = myelin_session::HandoffTerminal::Issued,
                    _ => {
                        let myelin_session::HandoffTerminal::Consumed { at } = &mut stored.lifecycle.terminal else { panic!() };
                        match corruption {
                            "wrong-block" => at.block_hash = [99; 32],
                            "wrong-session" => at.session_id = [99; 32],
                            "wrong-height" => at.height = 99,
                            _ => unreachable!(),
                        }
                    }
                }
                let bytes = stored.encode().unwrap();
                db.put(&key, bytes).unwrap();
            }
        }
        drop(db);
        let store = Arc::new(RocksSessionStore::open(directory.path()).unwrap());
        let (consensus, _) = consensus();
        let result = Chain::recover([4; 32], &profile, consensus, TestExecutor::from_snapshot(&snapshot), store);
        assert!(
            matches!(result, Err(SessionError::Recovery(_) | SessionError::Store(StoreError::Corrupt(_)))),
            "recovery must reject {corruption} markers"
        );
    }
}

#[test]
fn handoff_source_recovery_rejects_a_missing_issuance_index() {
    let directory = tempfile::tempdir().unwrap();
    let fixture = Fixture::open(directory.path());
    let id = fixture.ids.ordinary;
    let genesis = fixture.store.load_genesis([3; 32]).unwrap();
    let snapshot = fixture.store.load_checkpoint([3; 32]).unwrap().state_snapshot;
    drop(fixture);
    let db: OptimisticTransactionDB = OptimisticTransactionDB::open(&Options::default(), directory.path()).unwrap();
    let mut key = vec![b'j'];
    key.extend_from_slice(&id);
    db.delete(&key).unwrap();
    drop(db);
    let store = Arc::new(RocksSessionStore::open(directory.path()).unwrap());
    let (consensus, _) = consensus();
    let result =
        Chain::recover([3; 32], &genesis.config.application_profile, consensus, TestExecutor::from_snapshot(&snapshot), store);
    assert!(matches!(
        result,
        Err(SessionError::Recovery(ref reason)) if reason == "issued handoff is absent from the durable index"
    ));
}
