//! Aggregate budgets are tested against real durable state, including requests
//! with zero CellTx values. Every failure must preserve all atomic side effects.
use super::*;
use myelin_session::{
    CommandIdentity, CommandStatus, CommitResourceLimits, FinalisedBlockRecord, PausedRecovery, RecoveryAudit, RecoveryBudget,
    RecoveryCancellation, RecoveryOutcome, RecoveryPauseReason, ReplayAnchor, ReplayOutcome, SessionError,
};
use std::sync::atomic::{AtomicUsize, Ordering};
use std::time::Duration;

#[derive(Clone)]
struct BudgetExecutor {
    inner: TestExecutor,
    calls: Arc<AtomicUsize>,
    messages: usize,
    payload_bytes: usize,
    snapshot_bytes: usize,
    executed: bool,
    cancel_on_execute: Option<RecoveryCancellation>,
}

impl BudgetExecutor {
    fn new() -> Self {
        Self {
            inner: TestExecutor { root: [1; 32] },
            calls: Arc::new(AtomicUsize::new(0)),
            messages: 1,
            payload_bytes: 2,
            snapshot_bytes: 32,
            executed: false,
            cancel_on_execute: None,
        }
    }
}

impl TransitionExecutor for BudgetExecutor {
    fn state_root(&self) -> [u8; 32] {
        self.inner.root
    }
    fn execute_block_with_context(
        &mut self,
        context: &myelin_session::ProtocolEffectsContext,
        input: &FrameInput,
        handoffs: &[myelin_session::SessionHandoff],
        transactions: &[Vec<u8>],
    ) -> Result<ExecutionOutcome, String> {
        context.require_authority_metadata([99; 32])?;
        self.execute_block(context.height, input, handoffs, transactions)
    }
    fn execute_block(
        &mut self,
        height: u64,
        frame: &FrameInput,
        handoffs: &[myelin_session::SessionHandoff],
        txs: &[Vec<u8>],
    ) -> Result<ExecutionOutcome, String> {
        self.calls.fetch_add(1, Ordering::SeqCst);
        self.executed = true;
        if let Some(cancellation) = &self.cancel_on_execute {
            cancellation.cancel();
        }
        let mut result = self.inner.execute_block(height, frame, handoffs, txs)?;
        result.outbox =
            (0..self.messages).map(|_| PendingOutboxMessage { topic: "t".into(), payload: vec![9; self.payload_bytes] }).collect();
        Ok(result)
    }
    fn snapshot(&self) -> Result<Vec<u8>, String> {
        let mut bytes = self.inner.root.to_vec();
        if self.executed {
            bytes.resize(self.snapshot_bytes, 0);
        }
        Ok(bytes)
    }
}

fn request(position: u64) -> BlockRequest {
    let mut input = frame_input(position);
    input.commands = vec![CommandIdentity { id: [40 + position as u8; 32], payload_hash: [80; 32] }];
    BlockRequest {
        transactions: vec![],
        frame_input: input,
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

fn intent(seed: u8) -> HandoffIntent {
    HandoffIntent {
        target: HandoffTarget::Session([4; 32]),
        expires_at_ms: 100,
        payload: vec![seed; 2],
        authorization: vec![1; 2],
        evidence_requirement: None,
    }
}

fn seal<E: TransitionExecutor>(
    verifier: &RegisteredFinalityVerifier,
    signer: &CommitteeSigner,
    prepared: &myelin_session::PreparedBlock<E>,
) -> FinalityProof {
    let myelin_consensus::SelectedConsensus::ProofOfAuthority(engine) = verifier.selected_consensus() else { unreachable!() };
    FinalityProof::ProofOfAuthority(engine.seal_from_signer(prepared.block().hash(), prepared.block().number, signer).unwrap())
}

fn assert_limit(error: SessionError, resource: &str) {
    assert!(
        matches!(&error, SessionError::ResourceLimit { resource: actual, used, limit } if *actual == resource && used > limit),
        "{error}"
    );
}

fn assert_genesis_unchanged(store: &RocksSessionStore, chain: &SessionChain<BudgetExecutor, RocksSessionStore>, input: &FrameInput) {
    assert_eq!(chain.head(), &store.load_head([3; 32]).unwrap());
    assert_eq!(chain.head().finalised_height, None);
    assert_eq!(store.load_checkpoint([3; 32]).unwrap().state_snapshot, vec![1; 32]);
    assert!(store.load_chain_page([3; 32], 0, 2).unwrap().is_empty());
    assert!(store.pending_outbox([3; 32], 100).unwrap().is_empty());
    assert_eq!(chain.command_status(&input.commands[0]).unwrap(), CommandStatus::Missing);
}

#[test]
fn zero_transaction_requests_enforce_all_predictable_aggregate_budgets_before_execution() {
    for resource in
        ["issued_handoffs", "consumed_handoffs", "handoff_payload_bytes", "authorization_bytes", "frame_bytes", "data_commitments"]
    {
        let dir = tempfile::tempdir().unwrap();
        let store = Arc::new(RocksSessionStore::open(dir.path()).unwrap());
        let (verifier, _) = consensus();
        let mut cfg = config(&verifier);
        let limits = &mut cfg.commit_limits;
        let mut req = request(0);
        match resource {
            "issued_handoffs" => {
                limits.max_issued_handoffs = 1;
                req.handoffs = vec![intent(1), intent(2)];
            }
            "consumed_handoffs" => {
                limits.max_consumed_handoffs = 1;
                req.consume_handoffs = vec![[1; 32], [2; 32]];
            }
            "handoff_payload_bytes" => {
                limits.max_handoff_payload_bytes = 3;
                req.handoffs = vec![intent(1), intent(2)];
            }
            "authorization_bytes" => {
                limits.max_authorization_bytes = 3;
                req.handoffs = vec![intent(1), intent(2)];
            }
            // One payload byte fits the old input-payload check; command ids do not fit the whole frame.
            "frame_bytes" => limits.max_frame_bytes = 64,
            "data_commitments" => {
                limits.max_data_commitments = 1;
                req.handoffs = vec![intent(1)];
            }
            _ => unreachable!(),
        }
        let executor = BudgetExecutor::new();
        let calls = executor.calls.clone();
        let chain = SessionChain::create(cfg, verifier, executor, store.clone()).unwrap();
        let input = req.frame_input.clone();
        assert_limit(chain.prepare_block(req).err().unwrap(), resource);
        assert_eq!(calls.load(Ordering::SeqCst), 0);
        assert_genesis_unchanged(&store, &chain, &input);
    }
}

#[test]
fn oversized_executor_outputs_never_advance_state_receipts_or_outbox() {
    for resource in ["outbox_messages", "outbox_bytes", "snapshot_bytes", "data_commitments", "record_bytes"] {
        let dir = tempfile::tempdir().unwrap();
        let store = Arc::new(RocksSessionStore::open(dir.path()).unwrap());
        let (verifier, _) = consensus();
        let mut cfg = config(&verifier);
        let limits = &mut cfg.commit_limits;
        let mut executor = BudgetExecutor::new();
        match resource {
            "outbox_messages" => {
                limits.max_outbox_messages = 1;
                executor.messages = 2;
            }
            "outbox_bytes" => limits.max_outbox_bytes = 2, // topic + payload = 3
            "snapshot_bytes" => {
                limits.max_snapshot_bytes = 32;
                executor.snapshot_bytes = 33;
            }
            "data_commitments" => limits.max_data_commitments = 2, // outbox plus context/resource commitments
            "record_bytes" => limits.max_record_bytes = 100,
            _ => unreachable!(),
        }
        let calls = executor.calls.clone();
        let chain = SessionChain::create(cfg, verifier, executor, store.clone()).unwrap();
        assert_limit(chain.prepare_block(request(0)).err().unwrap(), resource);
        assert_eq!(calls.load(Ordering::SeqCst), 1);
        assert_genesis_unchanged(&store, &chain, &request(0).frame_input);
    }
}

#[test]
fn inclusive_budgets_and_full_record_size_survive_snapshot_elision_and_replay() {
    let dir = tempfile::tempdir().unwrap();
    let store = Arc::new(
        RocksSessionStore::open_with_options(
            dir.path(),
            RocksSessionStoreOptions { archival_checkpoint_interval: 100, ..Default::default() },
        )
        .unwrap(),
    );
    let (verifier, signer) = consensus();
    let mut cfg = config(&verifier);
    let limits = &mut cfg.commit_limits;
    limits.max_outbox_messages = 1;
    limits.max_outbox_bytes = 3;
    limits.max_snapshot_bytes = 32;
    limits.max_data_commitments = 3;
    let mut chain = SessionChain::create(cfg.clone(), verifier.clone(), BudgetExecutor::new(), store.clone()).unwrap();
    for position in 0..3 {
        let prepared = chain.prepare_block(request(position)).unwrap();
        let proof = seal(&verifier, &signer, &prepared);
        chain.commit_finalised(prepared, proof).unwrap();
    }
    let mut record = store.load_block([3; 32], 1).unwrap();
    assert!(record.state_snapshot.is_empty());
    let full_size = record.encoded_size_with_limit(u64::MAX).unwrap();
    let pruned_size = record.encode().unwrap().len() as u64;
    assert_eq!(full_size, pruned_size + 32);
    assert_eq!(record.encoded_size_with_limit(full_size).unwrap(), full_size);
    assert_limit(record.encoded_size_with_limit(full_size - 1).unwrap_err(), "record_bytes");
    let mut altered = cfg.clone();
    altered.commit_limits.max_record_bytes -= 1;
    assert!(record.validate_resources(&altered).is_err());
    record.resource_snapshot_bytes = 31;
    assert!(record.validate_resources(&cfg).is_err());
    SessionChain::<BudgetExecutor, _>::audit_recovery([3; 32], &cfg.application_profile, verifier.clone(), store.clone()).unwrap();
    chain
        .replay_range(0, 2, |bytes| {
            let mut executor = BudgetExecutor::new();
            executor.inner = TestExecutor::from_snapshot(bytes);
            Ok(executor)
        })
        .unwrap();
}

#[test]
fn issued_and_consumed_payloads_share_one_budget_and_failure_preserves_marker() {
    let dir = tempfile::tempdir().unwrap();
    let store = Arc::new(RocksSessionStore::open(dir.path()).unwrap());
    let (verifier, signer) = consensus();
    let mut source = SessionChain::create(config(&verifier), verifier.clone(), BudgetExecutor::new(), store.clone()).unwrap();
    let mut issue = request(0);
    issue.handoffs = vec![intent(1)];
    let prepared = source.prepare_block(issue).unwrap();
    let id = prepared.issued_handoffs()[0].id();
    let proof = seal(&verifier, &signer, &prepared);
    source.commit_finalised(prepared, proof).unwrap();
    let mut cfg = config(&verifier);
    cfg.session_id = [4; 32];
    cfg.commit_limits.max_handoff_payload_bytes = 3;
    let executor = BudgetExecutor::new();
    let calls = executor.calls.clone();
    let target = SessionChain::create(cfg, verifier, executor, store.clone()).unwrap();
    let mut request = request(0);
    request.handoffs = vec![intent(2)];
    request.consume_handoffs = vec![id];
    assert_limit(target.prepare_block(request).err().unwrap(), "handoff_payload_bytes");
    assert_eq!(calls.load(Ordering::SeqCst), 0);
    assert!(!store.load_handoff(id).unwrap().unwrap().is_consumed());
    assert_eq!(store.load_head([4; 32]).unwrap().finalised_height, None);
    assert!(store.pending_outbox([4; 32], 10).unwrap().is_empty());
}

#[test]
fn direct_store_writes_and_offline_decode_cannot_bypass_genesis_limits() {
    let dir = tempfile::tempdir().unwrap();
    let store = Arc::new(RocksSessionStore::open(dir.path()).unwrap());
    let (verifier, signer) = consensus();
    let mut cfg = config(&verifier);
    cfg.commit_limits.max_outbox_bytes = 3;
    let mut chain = SessionChain::create(cfg.clone(), verifier.clone(), BudgetExecutor::new(), store.clone()).unwrap();
    let genesis_head = chain.head().clone();
    let prepared = chain.prepare_block(request(0)).unwrap();
    let proof = seal(&verifier, &signer, &prepared);
    chain.commit_finalised(prepared, proof).unwrap();
    let mut record = store.load_block([3; 32], 0).unwrap();
    let full_size = record.encode().unwrap().len() as u64;
    assert_eq!(record.encoded_size_with_limit(full_size).unwrap(), full_size);
    record.outbox[0].payload.push(0);
    let corrupt_bytes = record.encode().unwrap();
    assert_limit(FinalisedBlockRecord::decode_for_config(&corrupt_bytes, &cfg).unwrap_err(), "outbox_bytes");
    let err = store.commit_block(&genesis_head, chain.head(), &record).unwrap_err();
    assert!(err.to_string().contains("outbox_bytes"), "{err}");
    assert_eq!(store.load_head([3; 32]).unwrap(), *chain.head());
    assert_eq!(store.pending_outbox([3; 32], 10).unwrap()[0].message.payload.len(), 2);
}

#[test]
fn every_aggregate_limit_changes_the_genesis_commitment() {
    let (verifier, _) = consensus();
    let cfg = config(&verifier);
    let encoded = cfg.encode().unwrap();
    assert_eq!(SessionConfig::decode(&encoded).unwrap(), cfg);
    let original = cfg.commitment();
    for field in 0..10 {
        let mut changed = cfg.clone();
        let limits = &mut changed.commit_limits;
        let value = match field {
            0 => &mut limits.max_frame_bytes,
            1 => &mut limits.max_issued_handoffs,
            2 => &mut limits.max_consumed_handoffs,
            3 => &mut limits.max_handoff_payload_bytes,
            4 => &mut limits.max_authorization_bytes,
            5 => &mut limits.max_data_commitments,
            6 => &mut limits.max_outbox_messages,
            7 => &mut limits.max_outbox_bytes,
            8 => &mut limits.max_snapshot_bytes,
            _ => &mut limits.max_record_bytes,
        };
        *value -= 1;
        assert_ne!(changed.commitment(), original);
    }
    assert!(CommitResourceLimits { max_issued_handoffs: u64::MAX, ..Default::default() }.validate().is_err());
}

#[test]
fn complete_record_budget_includes_finality_proof_and_is_inclusive() {
    let (verifier, signer) = consensus();
    for bounded in [false, true] {
        let dir = tempfile::tempdir().unwrap();
        let store = Arc::new(RocksSessionStore::open(dir.path()).unwrap());
        let mut cfg = config(&verifier);
        if bounded {
            cfg.commit_limits.max_record_bytes = 1024;
        }
        let mut chain = SessionChain::create(cfg, verifier.clone(), BudgetExecutor::new(), store.clone()).unwrap();
        // The predictable body fits both profiles; finality adds bytes.
        let prepared = chain.prepare_block(request(0)).unwrap();
        let proof = seal(&verifier, &signer, &prepared);
        let result = chain.commit_finalised(prepared, proof);
        if bounded {
            assert_limit(result.unwrap_err(), "record_bytes");
            assert_genesis_unchanged(&store, &chain, &request(0).frame_input);
        } else {
            result.unwrap();
            let record = store.load_block([3; 32], 0).unwrap();
            let exact_size = record.encode().unwrap().len() as u64;
            assert_eq!(record.encoded_size_with_limit(exact_size).unwrap(), exact_size);
            assert_limit(record.encoded_size_with_limit(exact_size - 1).unwrap_err(), "record_bytes");
        }
    }
}

#[test]
fn pruned_record_size_restores_snapshot_varint_prefix_at_boundaries() {
    for snapshot_bytes in [127, 128, 16_383, 16_384] {
        let dir = tempfile::tempdir().unwrap();
        let store = Arc::new(RocksSessionStore::open(dir.path()).unwrap());
        let (verifier, signer) = consensus();
        let mut cfg = config(&verifier);
        cfg.commit_limits.max_snapshot_bytes = snapshot_bytes as u64;
        let mut executor = BudgetExecutor::new();
        executor.snapshot_bytes = snapshot_bytes;
        let mut chain = SessionChain::create(cfg.clone(), verifier.clone(), executor, store.clone()).unwrap();
        for position in 0..2 {
            let prepared = chain.prepare_block(request(position)).unwrap();
            let proof = seal(&verifier, &signer, &prepared);
            chain.commit_finalised(prepared, proof).unwrap();
        }
        let mut record = store.load_block([3; 32], 1).unwrap();
        assert!(record.state_snapshot.is_empty());
        let charged = record.encoded_size_with_limit(u64::MAX).unwrap();
        record.state_snapshot = store.load_checkpoint([3; 32]).unwrap().state_snapshot;
        assert_eq!(charged, record.encode().unwrap().len() as u64);
        assert_eq!(charged, record.encoded_size_with_limit(u64::MAX).unwrap());
        record.validate_resources(&cfg).unwrap();
        record.state_snapshot.clear();
        record.validate_resources(&cfg).unwrap();
    }
}

#[test]
fn replacing_genesis_limits_cannot_reinterpret_an_existing_signed_history() {
    let dir = tempfile::tempdir().unwrap();
    let store = Arc::new(RocksSessionStore::open(dir.path()).unwrap());
    let (verifier, signer) = consensus();
    let cfg = config(&verifier);
    let mut chain = SessionChain::create(cfg.clone(), verifier.clone(), BudgetExecutor::new(), store.clone()).unwrap();
    let prepared = chain.prepare_block(request(0)).unwrap();
    let proof = seal(&verifier, &signer, &prepared);
    chain.commit_finalised(prepared, proof).unwrap();
    let mut genesis = store.load_genesis([3; 32]).unwrap();
    genesis.config.commit_limits.max_issued_handoffs += 1;
    drop(chain);
    drop(store);
    let db: OptimisticTransactionDB = OptimisticTransactionDB::open(&Options::default(), dir.path()).unwrap();
    let mut key = vec![b'g'];
    key.extend_from_slice(&[3; 32]);
    db.put(key, genesis.encode().unwrap()).unwrap();
    drop(db);
    let store = RocksSessionStore::open(dir.path()).unwrap();
    let error =
        SessionChain::<BudgetExecutor, _>::audit_recovery([3; 32], &cfg.application_profile, verifier.clone(), Arc::new(store))
            .err()
            .unwrap();
    assert!(error.to_string().contains("resource budget"), "{error}");
}

#[test]
fn original_megabyte_probe_many_small_handoffs_and_large_outputs_are_bounded() {
    for case in 0..4 {
        let dir = tempfile::tempdir().unwrap();
        let store = Arc::new(RocksSessionStore::open(dir.path()).unwrap());
        let (verifier, _) = consensus();
        let mut cfg = config(&verifier);
        let mut executor = BudgetExecutor::new();
        let mut req = request(0);
        let resource = match case {
            0 => {
                cfg.commit_limits.max_handoff_payload_bytes = 1_000_000;
                let mut handoff = intent(1);
                handoff.payload.resize(1_000_001, 1);
                req.handoffs.push(handoff);
                "handoff_payload_bytes"
            }
            1 => {
                req.handoffs = (0..65).map(intent).collect();
                "issued_handoffs"
            }
            2 => {
                executor.messages = 1025;
                executor.payload_bytes = 1024;
                "outbox_messages"
            }
            _ => {
                cfg.commit_limits.max_snapshot_bytes = 1024 * 1024;
                executor.snapshot_bytes = 1024 * 1024 + 1;
                "snapshot_bytes"
            }
        };
        let calls = executor.calls.clone();
        let chain = SessionChain::create(cfg, verifier, executor, store.clone()).unwrap();
        assert_limit(chain.prepare_block(req).err().unwrap(), resource);
        assert_eq!(calls.load(Ordering::SeqCst), usize::from(case >= 2));
        assert_genesis_unchanged(&store, &chain, &request(0).frame_input);
    }
}

fn commit_budget_block(
    chain: &mut SessionChain<BudgetExecutor, RocksSessionStore>,
    verifier: &RegisteredFinalityVerifier,
    signer: &CommitteeSigner,
    position: u64,
) {
    let mut req = request(position);
    if position == 1 {
        req.handoffs = vec![intent(1)];
    }
    let prepared = chain.prepare_block(req).unwrap();
    let proof = seal(verifier, signer, &prepared);
    chain.commit_finalised(prepared, proof).unwrap();
}

fn paused(outcome: RecoveryOutcome, reason: RecoveryPauseReason, next_height: u64) -> Box<PausedRecovery> {
    let RecoveryOutcome::Paused(cursor) = outcome else { panic!("incomplete audit was marked complete") };
    assert_eq!(cursor.reason(), reason);
    assert_eq!(cursor.progress().next_height, next_height);
    cursor
}

#[test]
fn recovery_resumes_after_each_budget_and_cancellation_without_granting_early_write_access() {
    let dir = tempfile::tempdir().unwrap();
    let store = Arc::new(RocksSessionStore::open(dir.path()).unwrap());
    let (verifier, signer) = consensus();
    let cfg = config(&verifier);
    let mut chain = SessionChain::create(cfg.clone(), verifier.clone(), BudgetExecutor::new(), store.clone()).unwrap();
    for position in 0..5 {
        commit_budget_block(&mut chain, &verifier, &signer, position);
    }
    let cancel = RecoveryCancellation::default();
    let budget = RecoveryBudget { max_blocks: 2, ..Default::default() };
    let audit = RecoveryAudit::new([3; 32], cfg.application_profile.clone(), verifier.clone(), store.clone());
    let mut cursor = paused(audit.advance(budget.clone(), &cancel).unwrap(), RecoveryPauseReason::Blocks, 2);
    assert_eq!(cursor.progress().target_height, Some(4));
    assert_eq!(cursor.progress().captured_head, Some(chain.head().block_hash));
    assert_eq!(cursor.progress().total.blocks, 2);

    let cancelled = RecoveryCancellation::default();
    cancelled.cancel();
    cursor = paused(cursor.resume(budget.clone(), &cancelled).unwrap(), RecoveryPauseReason::Cancelled, 2);
    assert_eq!(cursor.progress().step.store_reads, 0);
    for (limited, reason) in [
        (RecoveryBudget { max_elapsed: Duration::ZERO, ..budget.clone() }, RecoveryPauseReason::Deadline),
        (RecoveryBudget { max_store_reads: 0, ..budget.clone() }, RecoveryPauseReason::StoreReads),
        (RecoveryBudget { max_read_bytes: 0, ..budget.clone() }, RecoveryPauseReason::ReadBytes),
    ] {
        cursor = paused(cursor.resume(limited, &cancel).unwrap(), reason, 2);
        assert_eq!(cursor.progress().step.store_reads, 0);
        assert_eq!(cursor.progress().total.blocks, 2);
    }
    // A point read fits, but there is not enough remaining space to reserve a
    // variable-size page. No page is loaded and no block advances.
    cursor = paused(
        cursor.resume(RecoveryBudget { max_read_bytes: 128 * 1024 * 1024, ..budget.clone() }, &cancel).unwrap(),
        RecoveryPauseReason::ReadBytes,
        2,
    );
    assert_eq!(cursor.progress().step.store_reads, 1);
    assert!(cursor.progress().step.read_bytes > 0);
    cursor = paused(cursor.resume(budget.clone(), &cancel).unwrap(), RecoveryPauseReason::Blocks, 4);
    assert_eq!(cursor.progress().step.blocks, 2);
    let RecoveryOutcome::Complete(audited) = cursor.resume(budget, &cancel).unwrap() else { panic!("audit did not finish") };
    assert_eq!(audited.usage().blocks, 5);
    assert_eq!(audited.head(), chain.head());
    let mut executor = BudgetExecutor::new();
    executor.inner = TestExecutor::from_snapshot(audited.state_snapshot());
    let recovered = SessionChain::recover_audited(*audited, &cfg.application_profile, verifier, executor, store.clone()).unwrap();
    assert_eq!(recovered.head(), chain.head());
    assert_eq!(store.pending_outbox([3; 32], 10).unwrap().len(), 5);
}

#[test]
fn recovery_charges_handoff_dependency_reads_and_rechecks_an_interrupted_block() {
    let dir = tempfile::tempdir().unwrap();
    let store = Arc::new(RocksSessionStore::open(dir.path()).unwrap());
    let (verifier, signer) = consensus();
    let cfg = config(&verifier);
    let mut chain = SessionChain::create(cfg.clone(), verifier.clone(), BudgetExecutor::new(), store.clone()).unwrap();
    for position in 0..3 {
        commit_budget_block(&mut chain, &verifier, &signer, position);
    }
    let cancel = RecoveryCancellation::default();
    let cursor = paused(
        RecoveryAudit::new([3; 32], cfg.application_profile.clone(), verifier, store.clone())
            .advance(RecoveryBudget { max_store_reads: 7, ..Default::default() }, &cancel)
            .unwrap(),
        RecoveryPauseReason::StoreReads,
        1,
    );
    assert_eq!(cursor.progress().step.store_reads, 7);
    assert_eq!(cursor.progress().step.blocks, 1);
    let RecoveryOutcome::Complete(audited) = cursor.resume(RecoveryBudget::default(), &cancel).unwrap() else {
        panic!("dependency audit did not finish")
    };
    assert_eq!(audited.usage().blocks, 3);
    assert!(audited.usage().store_reads > 7);
    assert_eq!(audited.head(), chain.head());
}

#[test]
fn recovery_rejects_a_changed_head_and_completed_audit_cannot_move_to_another_store() {
    let dir = tempfile::tempdir().unwrap();
    let store = Arc::new(RocksSessionStore::open(dir.path().join("source")).unwrap());
    let other = Arc::new(RocksSessionStore::open(dir.path().join("other")).unwrap());
    let (verifier, signer) = consensus();
    let cfg = config(&verifier);
    let mut chain = SessionChain::create(cfg.clone(), verifier.clone(), BudgetExecutor::new(), store.clone()).unwrap();
    let mut other_chain = SessionChain::create(cfg.clone(), verifier.clone(), BudgetExecutor::new(), other.clone()).unwrap();
    for position in 0..3 {
        commit_budget_block(&mut chain, &verifier, &signer, position);
        commit_budget_block(&mut other_chain, &verifier, &signer, position);
    }
    let audit = || RecoveryAudit::new([3; 32], cfg.application_profile.clone(), verifier.clone(), store.clone());
    let cancel = RecoveryCancellation::default();
    let ReplayOutcome::Paused(replay_cursor) = chain
        .start_replay_range(0, 2, None, restore_budget)
        .advance(RecoveryBudget { max_blocks: 1, ..Default::default() }, &cancel)
        .unwrap()
    else {
        panic!("replay unexpectedly completed")
    };
    let cursor = paused(
        audit().advance(RecoveryBudget { max_blocks: 1, ..Default::default() }, &cancel).unwrap(),
        RecoveryPauseReason::Blocks,
        1,
    );
    for (target, advance) in [(other, false), (store.clone(), true)] {
        let RecoveryOutcome::Complete(audited) = audit().advance(RecoveryBudget::default(), &cancel).unwrap() else {
            panic!("audit did not finish")
        };
        let mut executor = BudgetExecutor::new();
        executor.inner = TestExecutor::from_snapshot(audited.state_snapshot());
        assert_eq!(target.load_head([3; 32]).unwrap(), *audited.head());
        if advance {
            commit_budget_block(&mut chain, &verifier, &signer, 3);
        }
        let error =
            SessionChain::recover_audited(*audited, &cfg.application_profile, verifier.clone(), executor, target).err().unwrap();
        assert!(error.to_string().contains("another store or a changed head"), "{error}");
    }
    let error = cursor.resume(RecoveryBudget::default(), &cancel).err().unwrap();
    assert!(error.to_string().contains("captured head changed"), "{error}");
    let error = replay_cursor.resume(RecoveryBudget::default(), &cancel).err().unwrap();
    assert!(error.to_string().contains("captured head changed"), "{error}");
}

#[test]
fn durable_pages_stop_at_bytes_even_when_more_records_were_requested() {
    let dir = tempfile::tempdir().unwrap();
    let store = Arc::new(
        RocksSessionStore::open_with_options(
            dir.path(),
            RocksSessionStoreOptions { archival_checkpoint_interval: 1, ..Default::default() },
        )
        .unwrap(),
    );
    let (verifier, signer) = consensus();
    let cfg = config(&verifier);
    let mut executor = BudgetExecutor::new();
    executor.snapshot_bytes = 9 * 1024 * 1024;
    let mut chain = SessionChain::create(cfg.clone(), verifier.clone(), executor, store.clone()).unwrap();
    for position in 0..3 {
        commit_budget_block(&mut chain, &verifier, &signer, position);
    }
    for height in 0..3 {
        let page = store.load_chain_page([3; 32], height, 100).unwrap();
        assert_eq!(page.len(), 1);
        assert_eq!(page[0].block.number, height);
    }
    let audited = SessionChain::<BudgetExecutor, _>::audit_recovery([3; 32], &cfg.application_profile, verifier, store).unwrap();
    assert_eq!(audited.head(), chain.head());
    assert_eq!(audited.usage().blocks, 3);
    assert!(audited.usage().read_bytes > 36 * 1024 * 1024);
}

fn restore_budget(bytes: &[u8]) -> myelin_session::Result<BudgetExecutor> {
    let mut executor = BudgetExecutor::new();
    executor.inner = TestExecutor::from_snapshot(bytes);
    Ok(executor)
}

#[test]
fn replay_budget_includes_warmup_and_resume_preserves_exact_receipt_totals() {
    let dir = tempfile::tempdir().unwrap();
    let store = Arc::new(RocksSessionStore::open(dir.path()).unwrap());
    let (verifier, signer) = consensus();
    let mut chain = SessionChain::create(config(&verifier), verifier.clone(), BudgetExecutor::new(), store.clone()).unwrap();
    for position in 0..5 {
        commit_budget_block(&mut chain, &verifier, &signer, position);
    }
    let expected = chain.replay_range(3, 4, restore_budget).unwrap();
    assert_eq!(expected.checkpoint_height, None);
    assert_eq!(expected.frame_count, 2);
    assert_eq!(expected.state_root_before, store.load_block([3; 32], 2).unwrap().block.state_root_after);
    let cancel = RecoveryCancellation::default();
    let replay = chain.start_replay_range(3, 4, None, restore_budget);
    let ReplayOutcome::Paused(mut cursor) = replay.advance(RecoveryBudget { max_blocks: 2, ..Default::default() }, &cancel).unwrap()
    else {
        panic!("warmup was not budgeted")
    };
    assert_eq!(cursor.reason(), RecoveryPauseReason::Blocks);
    assert_eq!(cursor.progress().next_height, 2);
    for (budget, reason) in [
        (RecoveryBudget { max_store_reads: 0, ..Default::default() }, RecoveryPauseReason::StoreReads),
        (RecoveryBudget { max_read_bytes: 0, ..Default::default() }, RecoveryPauseReason::ReadBytes),
        (RecoveryBudget { max_elapsed: Duration::ZERO, ..Default::default() }, RecoveryPauseReason::Deadline),
    ] {
        let ReplayOutcome::Paused(next) = cursor.resume(budget, &cancel).unwrap() else { panic!("zero budget completed replay") };
        assert_eq!(next.reason(), reason);
        assert_eq!(next.progress().next_height, 2);
        assert_eq!(next.progress().step.store_reads, 0);
        cursor = next;
    }
    let ReplayOutcome::Complete { receipt, usage } = cursor.resume(RecoveryBudget::default(), &cancel).unwrap() else {
        panic!("replay did not finish")
    };
    assert_eq!(*receipt, expected);
    assert_eq!(usage.blocks, 5);
    assert_eq!(store.load_head([3; 32]).unwrap(), *chain.head());
    assert_eq!(store.pending_outbox([3; 32], 10).unwrap().len(), 5);
}

#[test]
fn cancellation_after_execution_discards_candidate_and_restores_only_once() {
    let dir = tempfile::tempdir().unwrap();
    let store = Arc::new(RocksSessionStore::open(dir.path()).unwrap());
    let (verifier, signer) = consensus();
    let mut chain = SessionChain::create(config(&verifier), verifier.clone(), BudgetExecutor::new(), store.clone()).unwrap();
    for position in 0..3 {
        commit_budget_block(&mut chain, &verifier, &signer, position);
    }
    let expected = chain.replay_range(0, 2, restore_budget).unwrap();
    let cancellation = RecoveryCancellation::default();
    let signal = cancellation.clone();
    let calls = Arc::new(AtomicUsize::new(0));
    let executions = calls.clone();
    let restores = Arc::new(AtomicUsize::new(0));
    let restored = restores.clone();
    let replay = chain.start_replay_range(0, 2, None, move |bytes| {
        restored.fetch_add(1, Ordering::SeqCst);
        let mut executor = restore_budget(bytes)?;
        executor.calls = executions;
        executor.cancel_on_execute = Some(signal);
        Ok(executor)
    });
    let ReplayOutcome::Paused(cursor) =
        replay.advance(RecoveryBudget { max_store_reads: 0, ..Default::default() }, &cancellation).unwrap()
    else {
        panic!("initialization was not budgeted")
    };
    assert_eq!(restores.load(Ordering::SeqCst), 0);
    let ReplayOutcome::Paused(cursor) = cursor.resume(RecoveryBudget::default(), &cancellation).unwrap() else {
        panic!("cancelled execution was accepted")
    };
    assert_eq!(cursor.reason(), RecoveryPauseReason::Cancelled);
    assert_eq!(cursor.progress().next_height, 0);
    assert_eq!(cursor.progress().total.blocks, 0);
    assert_eq!(calls.load(Ordering::SeqCst), 1);
    let ReplayOutcome::Complete { receipt, usage } =
        cursor.resume(RecoveryBudget::default(), &RecoveryCancellation::default()).unwrap()
    else {
        panic!("replay did not finish")
    };
    assert_eq!(*receipt, expected);
    assert_eq!(usage.blocks, 3);
    assert_eq!(calls.load(Ordering::SeqCst), 4);
    assert_eq!(restores.load(Ordering::SeqCst), 1);
    assert_eq!(store.load_head([3; 32]).unwrap(), *chain.head());
}

#[test]
fn trusted_replay_anchor_binds_every_identity_and_reports_only_suffix_execution() {
    let dir = tempfile::tempdir().unwrap();
    let store = Arc::new(RocksSessionStore::open(dir.path()).unwrap());
    let (verifier, signer) = consensus();
    let cfg = config(&verifier);
    let mut chain = SessionChain::create(cfg.clone(), verifier.clone(), BudgetExecutor::new(), store.clone()).unwrap();
    for position in 0..5 {
        commit_budget_block(&mut chain, &verifier, &signer, position);
    }
    let record = store.load_block([3; 32], 0).unwrap();
    // This test owns and checked the original history. Production callers must
    // supply these identities from their independently trusted checkpoint.
    let anchor = ReplayAnchor {
        genesis_commitment: cfg.commitment(),
        height: 0,
        block_hash: record.block.hash(),
        state_root: record.block.state_root_after,
        snapshot_hash: *blake3::hash(&record.state_snapshot).as_bytes(),
    };
    let cancellation = RecoveryCancellation::default();
    for field in 0..5 {
        let mut wrong = anchor.clone();
        match field {
            0 => wrong.genesis_commitment[0] ^= 1,
            1 => wrong.height = 3,
            2 => wrong.block_hash[0] ^= 1,
            3 => wrong.state_root[0] ^= 1,
            _ => wrong.snapshot_hash[0] ^= 1,
        }
        assert!(chain
            .start_replay_range(3, 4, Some(wrong), restore_budget)
            .advance(RecoveryBudget::default(), &cancellation)
            .is_err());
    }
    let ReplayOutcome::Complete { receipt, usage } =
        chain.start_replay_range(3, 4, Some(anchor), restore_budget).advance(RecoveryBudget::default(), &cancellation).unwrap()
    else {
        panic!("anchored replay did not finish")
    };
    let genesis_receipt = chain.replay_range(3, 4, restore_budget).unwrap();
    assert_eq!(receipt.checkpoint_height, Some(0));
    assert_eq!(usage.blocks, 4);
    assert_eq!(receipt.frame_count, 2);
    assert_eq!(receipt.total_input_bytes, genesis_receipt.total_input_bytes);
    assert_eq!(receipt.state_root_before, genesis_receipt.state_root_before);
    assert_eq!(receipt.state_root_after, genesis_receipt.state_root_after);
    assert_ne!(receipt.commitment(), genesis_receipt.commitment());
}

#[test]
fn archival_checkpoint_lookup_uses_atomic_index_without_decoding_intervening_history() {
    let dir = tempfile::tempdir().unwrap();
    let store = Arc::new(
        RocksSessionStore::open_with_options(
            dir.path(),
            RocksSessionStoreOptions { archival_checkpoint_interval: 10, ..Default::default() },
        )
        .unwrap(),
    );
    let (verifier, signer) = consensus();
    let cfg = config(&verifier);
    let mut chain = SessionChain::create(cfg.clone(), verifier.clone(), BudgetExecutor::new(), store.clone()).unwrap();
    for position in 0..12 {
        commit_budget_block(&mut chain, &verifier, &signer, position);
    }
    let expected_head = chain.head().clone();
    assert_eq!(store.load_checkpoint_before([3; 32], 10).unwrap().finalised_height, Some(0));
    assert_eq!(store.load_checkpoint_before([3; 32], 12).unwrap().finalised_height, Some(10));
    drop(chain);
    drop(store);
    let db: OptimisticTransactionDB = OptimisticTransactionDB::open(&Options::default(), dir.path()).unwrap();
    let mut key = vec![b'b'];
    key.extend_from_slice(&[3; 32]);
    key.extend_from_slice(&11u64.to_be_bytes());
    db.put(key, b"corrupt unrequested block").unwrap();
    drop(db);
    let store = Arc::new(RocksSessionStore::open(dir.path()).unwrap());
    assert_eq!(store.load_checkpoint_before([3; 32], 12).unwrap().finalised_height, Some(10));
    assert_eq!(store.load_head([3; 32]).unwrap(), expected_head);
    assert!(SessionChain::<BudgetExecutor, _>::audit_recovery([3; 32], &cfg.application_profile, verifier, store.clone()).is_err());
    drop(store);
    let db: OptimisticTransactionDB = OptimisticTransactionDB::open(&Options::default(), dir.path()).unwrap();
    let mut key = vec![b'a'];
    key.extend_from_slice(&[3; 32]);
    key.extend_from_slice(&10u64.to_be_bytes());
    db.put(key, [99; 32]).unwrap();
    drop(db);
    let store = RocksSessionStore::open(dir.path()).unwrap();
    assert!(store.load_checkpoint_before([3; 32], 12).is_err());
    assert_eq!(store.load_head([3; 32]).unwrap(), expected_head);
}
