use super::*;
use myelin_session_store_rocksdb::{
    ArchiveRecordSource, OffloadOutcome, RetentionHold, RetentionWindow, RocksSessionStore, RocksSessionStoreOptions,
};
use myelin_session_transfer::TransferCommand;

pub(super) fn tick(f: &mut Fixture, source: bool) {
    if source {
        let state = stored_state(&f.store, [3; 32]);
        let version = state.account(ASSET, signer().actor()).unwrap().version;
        let req = issue_request(&f.source, vec![f.intent(version as u8 + 2, version, 1)]);
        commit(&mut f.source, &f.verifier, &f.validator, req);
    } else {
        let block = f.store.load_block([3; 32], f.source.head().finalised_height.unwrap()).unwrap();
        let req = intake_request(&f.target, vec![block.issued_handoffs[0].id()]);
        commit(&mut f.target, &f.verifier, &f.validator, req);
    }
}
pub(super) fn retired() -> Fixture {
    let mut f = transfer();
    let id = f.store.load_block([3; 32], 0).unwrap().issued_handoffs[0].id();
    let consumed_at = HandoffBlockReference::from_block(&f.store.load_block([4; 32], 0).unwrap().block);
    let mut req = request(&f.source, vec![TransferCommand::Retire { handoff_id: id }], vec![], vec![]);
    req.handoff_actions.push(HandoffAction::Retire { handoff_id: id, consumed_at });
    commit(&mut f.source, &f.verifier, &f.validator, req);
    tick(&mut f, true);
    f
}
pub(super) fn acknowledge(f: &Fixture, id: Hash32) {
    for pending in f.store.pending_outbox(id, 1024).unwrap() {
        f.store.acknowledge_outbox(id, pending.message.id).unwrap();
    }
}
pub(super) fn register(f: &Fixture, registry: &BundleArchives, root: &Path, name: &str) -> (Hash32, std::path::PathBuf) {
    let archive = root.join(name);
    let manifest = export_store(f.store.clone(), &plan(f, root), &archive).unwrap();
    let (store, _) = open(&archive, &anchors(&manifest)).unwrap();
    (registry.register(store).unwrap(), archive)
}
pub(super) fn reopen(f: Fixture, provider: Arc<dyn ArchiveRecordSource>) -> Fixture {
    let Fixture { directory, store, source, target, verifier, validator } = f;
    drop(source);
    drop(target);
    drop(store);
    let store = Arc::new(RocksSessionStore::open_archived(directory.path(), RocksSessionStoreOptions::default(), provider).unwrap());
    let source = recover_chain(&store, &verifier, 3);
    let target = recover_chain(&store, &verifier, 4);
    Fixture { directory, store, source, target, verifier, validator }
}
pub(super) fn window() -> RetentionWindow {
    RetentionWindow { hot_from_height: u64::MAX, protected: vec![] }
}

#[test]
fn reclaimed_handoff_payload_compacts_without_releasing_its_permanent_identity() {
    let mut f = Fixture::new(TransferKind::Scarce);
    let req = issue_request(&f.source, vec![f.intent(1, 0, 25)]);
    let id = commit(&mut f.source, &f.verifier, &f.validator, req)[0];
    let handoff = f.store.load_handoff(id).unwrap().unwrap().handoff;
    let mut req = request(&f.source, vec![signer().cancel(f.source.config(), &handoff).unwrap()], vec![], vec![]);
    req.handoff_actions.push(HandoffAction::Cancel { handoff_id: id });
    commit(&mut f.source, &f.verifier, &f.validator, req);
    let root = tempfile::tempdir().unwrap();
    let registry = Arc::new(BundleArchives::default());
    let (archive, _) = register(&f, &registry, root.path(), "cancelled");
    f = reopen(f, registry.clone());
    acknowledge(&f, [3; 32]);
    let reference = registry.reference(archive, [3; 32], 0).unwrap();
    assert_eq!(
        f.store.offload_block(f.source.head(), 0, reference, &window(), &RetainUntilTerminal, &|_| Ok(())).unwrap(),
        OffloadOutcome::Held(RetentionHold::OutstandingHandoff)
    );
    let mut req = request(&f.source, vec![TransferCommand::Reclaim { handoff_id: id }], vec![], vec![]);
    req.handoff_actions.push(HandoffAction::Reclaim { handoff_id: id });
    commit(&mut f.source, &f.verifier, &f.validator, req);
    let expected = f.store.load_handoff(id).unwrap().unwrap();
    let expected_head = f.source.head().clone();
    let expected_checkpoint = f.store.load_checkpoint([3; 32]).unwrap();
    let OffloadOutcome::Archived { handoffs, .. } =
        f.store.offload_block(f.source.head(), 0, reference, &window(), &RetainUntilTerminal, &|_| Ok(())).unwrap()
    else {
        panic!("reclaimed handoff should archive and compact")
    };
    assert_eq!(handoffs.records, 1);
    assert!(handoffs.bytes_removed > 0);
    assert_eq!(f.store.load_handoff(id).unwrap().unwrap(), expected);
    let Fixture { directory, store, source, target, verifier, validator } = f;
    drop(source);
    drop(target);
    drop(store);
    let raw: rocksdb::OptimisticTransactionDB =
        rocksdb::OptimisticTransactionDB::open(&rocksdb::Options::default(), directory.path()).unwrap();
    let mut full_key = vec![b'j'];
    full_key.extend(id);
    let mut compact_key = vec![b't'];
    compact_key.extend(id);
    assert!(raw.get(full_key).unwrap().is_none());
    let compact = raw.get(compact_key).unwrap().unwrap();
    assert!(compact.len() <= 256);
    assert_eq!(expected.encode().unwrap().len() as u64 - compact.len() as u64, handoffs.bytes_removed);
    drop(raw);
    let store = Arc::new(RocksSessionStore::open_archived(directory.path(), RocksSessionStoreOptions::default(), registry).unwrap());
    let source = recover_chain(&store, &verifier, 3);
    let target = recover_chain(&store, &verifier, 4);
    let f = Fixture { directory, store, source, target, verifier, validator };
    assert_eq!(f.source.head(), &expected_head);
    assert_eq!(f.store.load_checkpoint([3; 32]).unwrap(), expected_checkpoint);
    assert_eq!(f.store.load_handoff(id).unwrap().unwrap(), expected);
    assert!(f.target.prepare_block(intake_request(&f.target, vec![id])).is_err());
    let mut retry = request(&f.source, vec![TransferCommand::Reclaim { handoff_id: id }], vec![], vec![]);
    retry.handoff_actions.push(HandoffAction::Reclaim { handoff_id: id });
    assert!(f.source.prepare_block(retry).is_err());
    replay(&f.source, &f.store);
    // Export reconstructs the same full semantic record from the compact index.
    let next = root.path().join("after-compaction");
    let manifest = export_store(f.store.clone(), &plan(&f, root.path()), &next).unwrap();
    let (archived, _) = open(&next, &anchors(&manifest)).unwrap();
    assert_eq!(archived.load_handoff(id).unwrap().unwrap(), expected);
}

#[test]
fn compact_handoff_corruption_cannot_make_recovery_or_duplicate_intake_succeed() {
    let mut f = retired();
    let root = tempfile::tempdir().unwrap();
    let registry = Arc::new(BundleArchives::default());
    let (archive, _) = register(&f, &registry, root.path(), "retired");
    f = reopen(f, registry.clone());
    acknowledge(&f, [3; 32]);
    let id = f.store.load_block([3; 32], 0).unwrap().issued_handoffs[0].id();
    let expected = f.store.load_handoff(id).unwrap().unwrap();
    let config = f.source.config().clone();
    let reference = registry.reference(archive, [3; 32], 0).unwrap();
    f.store.offload_block(f.source.head(), 0, reference, &window(), &RetainUntilTerminal, &|_| Ok(())).unwrap();
    let Fixture { directory, store, source, target, verifier, validator: _ } = f;
    drop(source);
    drop(target);
    drop(store);
    let mut full = vec![b'j'];
    full.extend(id);
    let mut compact = vec![b't'];
    compact.extend(id);
    let raw: rocksdb::OptimisticTransactionDB =
        rocksdb::OptimisticTransactionDB::open(&rocksdb::Options::default(), directory.path()).unwrap();
    assert!(raw.get(&full).unwrap().is_none());
    let original = raw.get(&compact).unwrap().unwrap();
    drop(raw);
    for corruption in ["missing", "overlap", "issuer", "terminal", "open", "trailing", "oversized"] {
        let raw: rocksdb::OptimisticTransactionDB =
            rocksdb::OptimisticTransactionDB::open(&rocksdb::Options::default(), directory.path()).unwrap();
        let mut changed = original.clone();
        match corruption {
            "missing" => {
                raw.delete(&compact).unwrap();
            }
            "overlap" => {
                raw.put(&full, expected.encode().unwrap()).unwrap();
            }
            "issuer" => {
                changed[5] ^= 1;
                raw.put(&compact, changed).unwrap();
            }
            "terminal" => {
                let last = changed.len() - 1;
                changed[last] ^= 1;
                raw.put(&compact, changed).unwrap();
            }
            "open" => {
                // The issuer occupies 32 + varint(0) + 32 bytes. Replace the
                // terminal with Issued: a well-formed lifecycle, forbidden here.
                changed.truncate(71);
                changed[70] = 0;
                raw.put(&compact, changed).unwrap();
            }
            "trailing" => {
                changed.push(0);
                raw.put(&compact, changed).unwrap();
            }
            "oversized" => {
                changed.resize(257, 0);
                raw.put(&compact, changed).unwrap();
            }
            _ => unreachable!(),
        }
        drop(raw);
        let store = Arc::new(
            RocksSessionStore::open_archived(directory.path(), RocksSessionStoreOptions::default(), registry.clone()).unwrap(),
        );
        let audit = RecoveryAudit::new(config.session_id, config.application_profile.clone(), verifier.clone(), store.clone());
        assert!(audit.advance(RecoveryBudget::default(), &RecoveryCancellation::default()).is_err(), "must reject {corruption}");
        drop(store);
        let raw: rocksdb::OptimisticTransactionDB =
            rocksdb::OptimisticTransactionDB::open(&rocksdb::Options::default(), directory.path()).unwrap();
        raw.delete(&full).unwrap();
        raw.put(&compact, &original).unwrap();
        drop(raw);
    }
    let store = Arc::new(RocksSessionStore::open_archived(directory.path(), RocksSessionStoreOptions::default(), registry).unwrap());
    assert_eq!(store.load_handoff(id).unwrap().unwrap(), expected);
    let source = recover_chain(&store, &verifier, 3);
    let target = recover_chain(&store, &verifier, 4);
    assert!(target.prepare_block(intake_request(&target, vec![id])).is_err());
    replay(&source, &store);
    replay(&target, &store);
}

#[test]
fn copyable_history_moves_to_cold_storage_without_resetting_nullifiers_or_recovery() {
    let mut f = Fixture::new(TransferKind::CopyableReceipt);
    let issue = issue_request(&f.source, vec![f.intent(1, 0, 1)]);
    let command = issue.frame_input.commands[0].clone();
    let id = commit(&mut f.source, &f.verifier, &f.validator, issue)[0];
    tick(&mut f, true);
    let root = tempfile::tempdir().unwrap();
    let registry = Arc::new(BundleArchives::default());
    let (archive, _) = register(&f, &registry, root.path(), "before-intake");
    f = reopen(f, registry.clone());
    acknowledge(&f, [3; 32]);
    let reference = registry.reference(archive, [3; 32], 0).unwrap();
    let policy = stored_state(&f.store, [3; 32]).policy().clone();
    assert_eq!(
        f.store.offload_block(f.source.head(), 0, reference, &window(), &policy, &|_| Ok(())).unwrap(),
        OffloadOutcome::Held(RetentionHold::OutstandingHandoff)
    );
    let req = intake_request(&f.target, vec![id]);
    commit(&mut f.target, &f.verifier, &f.validator, req);
    tick(&mut f, false);
    assert_eq!(
        f.store.offload_block(f.source.head(), 0, reference, &window(), &RetainUntilTerminal, &|_| Ok(())).unwrap(),
        OffloadOutcome::Held(RetentionHold::OutstandingHandoff)
    );
    let record = f.store.load_block([3; 32], 0).unwrap().encode().unwrap();
    let expected = [f.source.head().clone(), f.target.head().clone()];
    assert_eq!(
        f.store.offload_block(f.source.head(), 0, reference, &window(), &policy, &|_| Ok(())).unwrap(),
        OffloadOutcome::Archived { hot_record_bytes_removed: record.len() as u64, handoffs: Default::default() }
    );
    assert_eq!(
        f.store.offload_block(f.source.head(), 0, reference, &window(), &policy, &|_| Ok(())).unwrap(),
        OffloadOutcome::AlreadyArchived { handoffs: Default::default() }
    );
    assert_eq!(f.store.load_block([3; 32], 0).unwrap().encode().unwrap(), record);
    assert_eq!(f.store.load_checkpoint_before([3; 32], 1).unwrap().finalised_height, Some(0));
    assert_eq!(f.store.load_chain_page([3; 32], 0, 100).unwrap().len(), 2);
    assert!(matches!(f.source.command_status(&command).unwrap(), CommandStatus::Committed(_)));
    assert!(f.target.prepare_block(intake_request(&f.target, vec![id])).is_err());
    // The next full archive reads the cold source and the newer hot target.
    let (new_archive, _) = register(&f, &registry, root.path(), "after-intake");
    acknowledge(&f, [4; 32]);
    let target_ref = registry.reference(new_archive, [4; 32], 0).unwrap();
    assert!(matches!(
        f.store.offload_block(f.target.head(), 0, target_ref, &window(), &policy, &|_| Ok(())).unwrap(),
        OffloadOutcome::Archived { .. }
    ));
    let pins = f.store.archived_blocks(None, 1).unwrap();
    assert_eq!(pins.len(), 1);
    assert_eq!(pins[0].reference, reference);
    let next = f.store.archived_blocks(Some((pins[0].session_id, pins[0].height)), 1).unwrap();
    assert_eq!(next.len(), 1);
    assert_eq!(next[0].reference, target_ref);
    f = reopen(f, registry);
    assert_eq!([f.source.head().clone(), f.target.head().clone()], expected);
    assert!(matches!(f.source.command_status(&command).unwrap(), CommandStatus::Committed(_)));
    replay(&f.source, &f.store);
    replay(&f.target, &f.store);
    tick(&mut f, true);
    assert_eq!(f.source.head().finalised_height, Some(2));
}

#[test]
fn scarce_consumption_stays_hot_until_retirement_and_operational_holds_are_enforced() {
    let mut f = transfer();
    tick(&mut f, true);
    let root = tempfile::tempdir().unwrap();
    let registry = Arc::new(BundleArchives::default());
    let (archive, _) = register(&f, &registry, root.path(), "consumed");
    f = reopen(f, registry.clone());
    let reference = registry.reference(archive, [3; 32], 0).unwrap();
    let policy = stored_state(&f.store, [3; 32]).policy().clone();
    assert_eq!(
        f.store.offload_block(f.source.head(), 1, reference, &window(), &policy, &|_| Ok(())).unwrap(),
        OffloadOutcome::Held(RetentionHold::CurrentTip)
    );
    assert_eq!(
        f.store
            .offload_block(
                f.source.head(),
                0,
                reference,
                &RetentionWindow { hot_from_height: 0, protected: vec![] },
                &policy,
                &|_| Ok(())
            )
            .unwrap(),
        OffloadOutcome::Held(RetentionHold::RecoveryWindow)
    );
    assert_eq!(
        f.store
            .offload_block(
                f.source.head(),
                0,
                reference,
                &RetentionWindow { hot_from_height: 1, protected: vec![0..=0] },
                &policy,
                &|_| Ok(())
            )
            .unwrap(),
        OffloadOutcome::Held(RetentionHold::ProtectedRange)
    );
    assert_eq!(
        f.store.offload_block(f.source.head(), 0, reference, &window(), &policy, &|_| Ok(())).unwrap(),
        OffloadOutcome::Held(RetentionHold::PendingDelivery)
    );
    acknowledge(&f, [3; 32]);
    assert_eq!(
        f.store.offload_block(f.source.head(), 0, reference, &window(), &policy, &|_| Ok(())).unwrap(),
        OffloadOutcome::Held(RetentionHold::OutstandingHandoff)
    );
    let stored = f.store.load_block([3; 32], 0).unwrap();
    let id = stored.issued_handoffs[0].id();
    let consumed_at = HandoffBlockReference::from_block(&f.store.load_block([4; 32], 0).unwrap().block);
    let mut req = request(&f.source, vec![TransferCommand::Retire { handoff_id: id }], vec![], vec![]);
    req.handoff_actions.push(HandoffAction::Retire { handoff_id: id, consumed_at });
    commit(&mut f.source, &f.verifier, &f.validator, req);
    assert!(matches!(
        f.store.offload_block(f.source.head(), 0, reference, &window(), &policy, &|_| Ok(())).unwrap(),
        OffloadOutcome::Archived { .. }
    ));
    let before = f.source.head().clone();
    f = reopen(f, registry);
    assert_eq!(f.source.head(), &before);
    let state = stored_state(&f.store, [3; 32]);
    let account = state.account(ASSET, signer().actor()).unwrap();
    assert_eq!((account.available, account.locked), (74, 1));
    assert!(matches!(f.store.load_handoff(id).unwrap().unwrap().lifecycle.terminal, HandoffTerminal::Retired { .. }));
    replay(&f.source, &f.store);
}

#[test]
fn missing_or_changed_cold_data_fails_explicitly_and_a_wrong_reference_cannot_remove_hot_bytes() {
    let mut f = retired();
    let root = tempfile::tempdir().unwrap();
    let registry = Arc::new(BundleArchives::default());
    let (archive, path) = register(&f, &registry, root.path(), "archive");
    f = reopen(f, registry.clone());
    acknowledge(&f, [3; 32]);
    let reference = registry.reference(archive, [3; 32], 0).unwrap();
    let wrong = registry.reference(archive, [3; 32], 1).unwrap();
    let original = f.store.load_block([3; 32], 0).unwrap().encode().unwrap();
    let handoff = f.store.load_block([3; 32], 0).unwrap().issued_handoffs[0].id();
    let original_handoff = f.store.load_handoff(handoff).unwrap().unwrap();
    assert!(f.store.offload_block(f.source.head(), 0, wrong, &window(), &RetainUntilTerminal, &|_| Ok(())).is_err());
    assert!(f.store.archived_blocks(None, 10).unwrap().is_empty());
    assert_eq!(f.store.load_block([3; 32], 0).unwrap().encode().unwrap(), original);
    assert_eq!(f.store.load_handoff(handoff).unwrap().unwrap(), original_handoff);
    let abort_commit = |bytes| {
        if bytes == 0 {
            Err(StoreError::Backend("cancelled at commit boundary".into()))
        } else {
            Ok(())
        }
    };
    assert!(f.store.offload_block(f.source.head(), 0, reference, &window(), &RetainUntilTerminal, &abort_commit).is_err());
    assert!(f.store.archived_blocks(None, 10).unwrap().is_empty());
    assert_eq!(f.store.load_block([3; 32], 0).unwrap().encode().unwrap(), original);
    assert_eq!(f.store.load_handoff(handoff).unwrap().unwrap(), original_handoff);
    assert!(matches!(
        f.store.offload_block(f.source.head(), 0, reference, &window(), &RetainUntilTerminal, &|_| Ok(())).unwrap(),
        OffloadOutcome::Archived { handoffs: myelin_session_store_rocksdb::HandoffCompaction { records: 1, .. }, .. }
    ));
    let Fixture { directory, store, source, target, verifier, validator } = f;
    drop(source);
    drop(target);
    drop(store);
    let store = Arc::new(RocksSessionStore::open(directory.path()).unwrap());
    assert!(matches!(store.load_block([3; 32], 0), Err(StoreError::NotFound(_))));
    assert!(matches!(store.load_handoff(handoff), Err(StoreError::NotFound(_))));
    assert_eq!(store.archived_blocks(None, 10).unwrap().len(), 1);
    drop(store);
    // Recovery must not hide cold entries beyond the durable head. Reuse the
    // real reference at an impossible key, then remove only that injected key.
    let mut key = vec![b'd'];
    key.extend([3; 32]);
    key.extend(0u64.to_be_bytes());
    let mut orphan = vec![b'd'];
    orphan.extend([3; 32]);
    orphan.extend(3u64.to_be_bytes());
    let options = rocksdb::Options::default();
    let raw: rocksdb::OptimisticTransactionDB = rocksdb::OptimisticTransactionDB::open(&options, directory.path()).unwrap();
    raw.put(&orphan, raw.get(&key).unwrap().unwrap()).unwrap();
    drop(raw);
    let store = RocksSessionStore::open_archived(directory.path(), RocksSessionStoreOptions::default(), registry.clone()).unwrap();
    assert!(matches!(store.load_chain_page([3; 32], 3, 1), Err(StoreError::Corrupt(_))));
    drop(store);
    let raw: rocksdb::OptimisticTransactionDB = rocksdb::OptimisticTransactionDB::open(&options, directory.path()).unwrap();
    raw.delete(orphan).unwrap();
    drop(raw);
    let store = Arc::new(RocksSessionStore::open_archived(directory.path(), RocksSessionStoreOptions::default(), registry).unwrap());
    let source = recover_chain(&store, &verifier, 3);
    let target = recover_chain(&store, &verifier, 4);
    let f = Fixture { directory, store, source, target, verifier, validator };
    let blob = path.join("blobs").join(BlobId(reference.blob).hex());
    fs::write(&blob, b"changed").unwrap();
    assert!(f.store.load_block([3; 32], 0).is_err());
    assert!(f.store.load_handoff(handoff).is_err());
    fs::remove_file(blob).unwrap();
    assert!(matches!(f.store.load_block([3; 32], 0), Err(StoreError::NotFound(_))));
    assert!(matches!(f.store.load_handoff(handoff), Err(StoreError::NotFound(_))));
    let cfg = f.source.config();
    assert!(RecoveryAudit::new(cfg.session_id, cfg.application_profile.clone(), f.verifier.clone(), f.store.clone())
        .advance(RecoveryBudget::default(), &RecoveryCancellation::default())
        .is_err());
    assert_eq!(f.store.load_head([3; 32]).unwrap(), *f.source.head());
}

#[test]
fn concurrent_head_advancement_rejects_the_atomic_hot_to_cold_swap() {
    use std::sync::{
        atomic::{AtomicBool, Ordering},
        Barrier,
    };
    struct Blocking {
        inner: Arc<BundleArchives>,
        pause: AtomicBool,
        entered: Barrier,
        resume: Barrier,
    }
    impl ArchiveRecordSource for Blocking {
        fn read_record(&self, a: Hash32, b: Hash32, n: u64) -> std::result::Result<Vec<u8>, StoreError> {
            let bytes = self.inner.read_record(a, b, n)?;
            if self.pause.swap(false, Ordering::SeqCst) {
                self.entered.wait();
                self.resume.wait();
            }
            Ok(bytes)
        }
    }
    let mut f = retired();
    let root = tempfile::tempdir().unwrap();
    let registry = Arc::new(BundleArchives::default());
    let (archive, _) = register(&f, &registry, root.path(), "archive");
    let reference = registry.reference(archive, [3; 32], 0).unwrap();
    let blocker =
        Arc::new(Blocking { inner: registry, pause: AtomicBool::new(false), entered: Barrier::new(2), resume: Barrier::new(2) });
    f = reopen(f, blocker.clone());
    acknowledge(&f, [3; 32]);
    let store = f.store.clone();
    let expected = f.source.head().clone();
    blocker.pause.store(true, Ordering::SeqCst);
    std::thread::scope(|scope| {
        let worker = scope.spawn(|| store.offload_block(&expected, 0, reference, &window(), &RetainUntilTerminal, &|_| Ok(())));
        blocker.entered.wait();
        tick(&mut f, true);
        blocker.resume.wait();
        assert!(matches!(worker.join().unwrap(), Err(StoreError::Conflict(_))));
    });
    assert!(f.store.archived_blocks(None, 10).unwrap().is_empty());
    assert_eq!(f.store.load_chain_page([3; 32], 0, 10).unwrap().len(), 4);
    assert_eq!(recover_chain(&f.store, &f.verifier, 3).head(), f.source.head());
    assert!(matches!(
        f.store.offload_block(f.source.head(), 0, reference, &window(), &RetainUntilTerminal, &|_| Ok(())).unwrap(),
        OffloadOutcome::Archived { handoffs: myelin_session_store_rocksdb::HandoffCompaction { records: 1, .. }, .. }
    ));
}

fn retention_plan(archive: Hash32, path: &Path) -> RetentionPlan {
    let directory = BundleDirectory::open(path, test_limits()).unwrap();
    RetentionPlan {
        archives: vec![ArchiveCatalog { path: path.into(), commitment: BlobId(archive), trust: anchors(directory.manifest()) }],
        ranges: vec![
            RetentionRange {
                session: BlobId([3; 32]),
                first: 0,
                last: 2,
                archive: BlobId(archive),
                hot_from_height: u64::MAX,
                protected: vec![],
            },
            RetentionRange {
                session: BlobId([4; 32]),
                first: 0,
                last: 0,
                archive: BlobId(archive),
                hot_from_height: u64::MAX,
                protected: vec![],
            },
        ],
        limits: test_limits(),
    }
}
pub(super) fn grant(steps: u64) -> ArchiveBudget {
    ArchiveBudget { max_steps: steps, max_io_bytes: 64 * 1024 * 1024 * 1024, max_elapsed_ms: 60_000 }
}
fn complete(outcome: ArchiveOutcome) -> RetentionProgress {
    let ArchiveOutcome::Complete { output, .. } = outcome else { panic!("retention unexpectedly paused") };
    let ArchiveOutput::Retained(report) = *output else { panic!("wrong retention output") };
    report
}
fn pause_after_first_block(task: ArchiveTask) -> Box<PausedArchive> {
    let cancellation = RecoveryCancellation::default();
    let mut outcome = task.advance(grant(1), &cancellation).unwrap();
    for _ in 0..50_000 {
        let ArchiveOutcome::Paused(cursor) = outcome else { panic!("task completed without a per-block checkpoint") };
        assert_eq!(cursor.progress().step.steps, 1);
        if cursor.progress().retention.as_ref().is_some_and(|p| p.processed == 1) {
            return cursor;
        }
        outcome = cursor.resume(grant(1), &cancellation).unwrap();
    }
    panic!("retention made no bounded progress");
}

#[test]
fn bulk_retention_resumes_partial_commits_and_rebinds_without_old_archive_files() {
    let mut f = retired();
    let root = tempfile::tempdir().unwrap();
    let registry = Arc::new(BundleArchives::default());
    let (archive, path) = register(&f, &registry, root.path(), "first");
    let plan = retention_plan(archive, &path);
    f = reopen(f, registry.clone());
    acknowledge(&f, [3; 32]);
    let heads = [f.source.head().clone(), f.target.head().clone()];
    let block = f.store.load_block([3; 32], 0).unwrap();
    let command = block.frame_input.commands[0].clone();
    let task = ArchiveTask::retain(f.store.clone(), registry.clone(), plan.clone(), Arc::new(RetainUntilTerminal)).unwrap();
    let cursor = pause_after_first_block(task);
    assert_eq!(cursor.progress().retention.as_ref().unwrap().archived, 1);
    assert_eq!(cursor.progress().retention.as_ref().unwrap().handoffs_compacted, 1);
    assert!(cursor.progress().retention.as_ref().unwrap().handoff_bytes_removed > 0);
    assert_eq!(f.store.archived_blocks(None, 10).unwrap().len(), 1);
    let cancelled = RecoveryCancellation::default();
    cancelled.cancel();
    let ArchiveOutcome::Paused(cursor) = cursor.resume(grant(100_000), &cancelled).unwrap() else { panic!("cancel must pause") };
    assert_eq!(cursor.reason(), ArchivePauseReason::Cancelled);
    let ArchiveOutcome::Paused(cursor) =
        cursor.resume(ArchiveBudget { max_io_bytes: 0, ..grant(100_000) }, &RecoveryCancellation::default()).unwrap()
    else {
        panic!("read budget must pause")
    };
    assert_eq!(cursor.reason(), ArchivePauseReason::Bytes);
    assert_eq!(cursor.progress().retention.as_ref().unwrap().processed, 1);
    let report = complete(cursor.resume(grant(100_000), &RecoveryCancellation::default()).unwrap());
    assert_eq!((report.requested, report.processed, report.archived, report.held.current_tip), (4, 4, 2, 2));
    assert!(report.hot_record_bytes_removed >= block.encode().unwrap().len() as u64);
    assert_eq!(report.handoffs_compacted, 1);
    let again = ArchiveTask::retain(f.store.clone(), registry.clone(), plan, Arc::new(RetainUntilTerminal)).unwrap();
    let report = complete(again.advance(grant(100_000), &RecoveryCancellation::default()).unwrap());
    assert_eq!((report.archived, report.already_archived, report.held.current_tip), (0, 2, 2));
    assert_eq!((report.handoffs_compacted, report.handoff_bytes_removed), (0, 0));
    assert_eq!([f.source.head().clone(), f.target.head().clone()], heads);
    assert!(matches!(f.source.command_status(&command).unwrap(), CommandStatus::Committed(_)));

    // A newer complete catalog contains the same blocks and their full closure.
    tick(&mut f, true);
    let (new_archive, new_path) = register(&f, &registry, root.path(), "consolidated");
    assert_ne!(new_archive, archive);
    let mut plan = retention_plan(new_archive, &new_path);
    plan.ranges.truncate(1);
    plan.ranges[0].last = 1;
    plan.ranges[0].hot_from_height = 0;
    plan.ranges[0].protected = vec![RetentionHeights { first: 0, last: 1 }];
    // Limits bound the whole retained catalog set, not each catalog separately.
    let mut excessive = plan.clone();
    excessive.archives.extend(retention_plan(archive, &path).archives);
    excessive.limits.total_bytes = [&path, &new_path]
        .into_iter()
        .map(|p| BundleDirectory::open(p, test_limits()).unwrap().manifest().blobs.iter().map(|b| b.bytes).sum::<u64>())
        .max()
        .unwrap();
    let task = ArchiveTask::retain(f.store.clone(), registry.clone(), excessive, Arc::new(RetainUntilTerminal)).unwrap();
    let error = match task.advance(grant(100_000), &RecoveryCancellation::default()) {
        Err(error) => error,
        Ok(_) => panic!("combined catalog budget must reject before rebinding"),
    };
    assert_eq!(error.code, ErrorCode::Limit);
    assert_eq!(error.progress.unwrap().retention.unwrap().processed, 0);
    assert!(f.store.archived_blocks(None, 10).unwrap().iter().all(|b| b.reference.archive == archive));
    let wrong = registry.reference(new_archive, [3; 32], 1).unwrap();
    assert!(matches!(
        f.store.offload_block(f.source.head(), 0, wrong, &window(), &RetainUntilTerminal, &|_| Ok(())),
        Err(StoreError::Conflict(_))
    ));
    fs::remove_dir_all(&path).unwrap();
    assert!(f.store.load_block([3; 32], 0).is_err());
    let task = ArchiveTask::retain(f.store.clone(), registry, plan, Arc::new(RetainUntilTerminal)).unwrap();
    let report = complete(task.advance(grant(100_000), &RecoveryCancellation::default()).unwrap());
    assert_eq!((report.processed, report.rebound, report.archived), (2, 2, 0));
    assert_eq!(report.held, RetentionHolds::default());
    assert!(f.store.archived_blocks(None, 10).unwrap().iter().all(|b| b.reference.archive == new_archive));
    // Reopen with only the new registry: old manifests and blobs are absent.
    let fresh = Arc::new(BundleArchives::default());
    let (verified, _) = open(&new_path, &anchors(BundleDirectory::open(&new_path, test_limits()).unwrap().manifest())).unwrap();
    fresh.register(verified).unwrap();
    f = reopen(f, fresh);
    assert_eq!(f.store.load_block([3; 32], 0).unwrap().encode().unwrap(), block.encode().unwrap());
    assert!(matches!(f.source.command_status(&command).unwrap(), CommandStatus::Committed(_)));
    replay(&f.source, &f.store);
    replay(&f.target, &f.store);
}

#[test]
fn bulk_retention_reports_partial_progress_on_head_change_and_restarts_idempotently() {
    let mut f = retired();
    let root = tempfile::tempdir().unwrap();
    let registry = Arc::new(BundleArchives::default());
    let (archive, path) = register(&f, &registry, root.path(), "archive");
    let plan = retention_plan(archive, &path);
    f = reopen(f, registry.clone());
    acknowledge(&f, [3; 32]);
    // Catalog identity and source bounds fail before the first mutation.
    for mismatch in [true, false] {
        let mut invalid = plan.clone();
        if mismatch {
            invalid.archives[0].commitment = BlobId([99; 32]);
            for range in &mut invalid.ranges {
                range.archive = BlobId([99; 32]);
            }
        } else {
            invalid.ranges[0].last = 3;
        }
        let task = ArchiveTask::retain(f.store.clone(), registry.clone(), invalid, Arc::new(RetainUntilTerminal)).unwrap();
        let error = match task.advance(grant(100_000), &RecoveryCancellation::default()) {
            Err(error) => error,
            Ok(_) => panic!("invalid plan must fail before mutations"),
        };
        assert_eq!(error.code, if mismatch { ErrorCode::Integrity } else { ErrorCode::Format });
        assert_eq!(error.progress.unwrap().retention.unwrap().processed, 0);
        assert!(f.store.archived_blocks(None, 10).unwrap().is_empty());
    }
    let task = ArchiveTask::retain(f.store.clone(), registry.clone(), plan.clone(), Arc::new(RetainUntilTerminal)).unwrap();
    let cursor = pause_after_first_block(task);
    tick(&mut f, true);
    let error = match cursor.resume(grant(100_000), &RecoveryCancellation::default()) {
        Err(error) => error,
        Ok(_) => panic!("stale captured head must reject continued relocation"),
    };
    assert_eq!(error.code, ErrorCode::SourceChanged);
    let progress = error.progress.unwrap().retention.unwrap();
    assert_eq!((progress.processed, progress.archived), (1, 1));
    assert_eq!(f.store.archived_blocks(None, 10).unwrap().len(), 1);
    let restart = ArchiveTask::retain(f.store.clone(), registry.clone(), plan.clone(), Arc::new(RetainUntilTerminal)).unwrap();
    let cursor = pause_after_first_block(restart);
    assert_eq!(cursor.progress().retention.as_ref().unwrap().already_archived, 1);
    drop(cursor);
    let restart = ArchiveTask::retain(f.store.clone(), registry, plan, Arc::new(RetainUntilTerminal)).unwrap();
    let report = complete(restart.advance(grant(100_000), &RecoveryCancellation::default()).unwrap());
    assert_eq!((report.processed, report.already_archived, report.archived, report.held.outstanding_handoff), (4, 1, 1, 1));
    assert_eq!(f.source.head().finalised_height, Some(3));
    replay(&f.source, &f.store);
}

#[test]
fn retention_cli_uses_the_shared_work_protocol_and_reopens_cold_history() {
    use std::io::{BufRead, BufReader, Write};
    use std::process::{Command, Stdio};
    let f = retired();
    acknowledge(&f, [3; 32]);
    let root = tempfile::tempdir().unwrap();
    let registry = Arc::new(BundleArchives::default());
    let (archive, path) = register(&f, &registry, root.path(), "archive");
    let mut plan = retention_plan(archive, &path);
    plan.archives[0].path = "archive".into(); // Relative to the plan, not process cwd.
    let plan_path = root.path().join("retention.json");
    fs::write(&plan_path, serde_json::to_vec(&plan).unwrap()).unwrap();
    let expected = f.source.head().clone();
    let Fixture { directory, store, source, target, verifier, validator: _ } = f;
    drop(source);
    drop(target);
    drop(store);
    let mut child = Command::new(env!("CARGO_BIN_EXE_myelin-session-bundle"))
        .args(["work", "retain"])
        .arg(directory.path())
        .arg(plan_path)
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::inherit())
        .spawn()
        .unwrap();
    let mut input = child.stdin.take().unwrap();
    let mut output = BufReader::new(child.stdout.take().unwrap());
    for cancelled in [true, false] {
        writeln!(input, "{}", serde_json::json!({"budget":grant(100_000),"cancelled":cancelled})).unwrap();
        input.flush().unwrap();
        let mut line = String::new();
        output.read_line(&mut line).unwrap();
        let value: serde_json::Value = serde_json::from_str(&line).unwrap();
        if cancelled {
            assert_eq!(value["status"], "paused");
            assert_eq!(value["reason"], "cancelled");
            assert!(child.try_wait().unwrap().is_none());
        } else {
            assert_eq!(value["status"], "complete");
            assert_eq!(value["report"]["stage"], "retained");
            assert_eq!(value["report"]["progress"]["archived"], 2);
        }
    }
    drop(input);
    assert!(child.wait().unwrap().success());
    let store = Arc::new(RocksSessionStore::open_archived(directory.path(), RocksSessionStoreOptions::default(), registry).unwrap());
    let source = recover_chain(&store, &verifier, 3);
    assert_eq!(source.head(), &expected);
    assert_eq!(store.archived_blocks(None, 10).unwrap().len(), 2);
    replay(&source, &store);
}
