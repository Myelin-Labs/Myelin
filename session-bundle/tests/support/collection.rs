use super::*;
use myelin_session_store_rocksdb::{RocksSessionStore, RocksSessionStoreOptions, SnapshotInventory, SnapshotLimits};
use std::{
    io::{BufRead, BufReader, Write},
    process::{Command, Stdio},
};

fn catalog(path: &Path, manifest: &BundleManifest) -> ArchiveCatalog {
    ArchiveCatalog { path: path.into(), commitment: BlobId(manifest.commitment().unwrap()), trust: anchors(manifest) }
}
fn collect(plan: CollectionPlan) -> myelin_session_bundle::Result<CollectionReport> {
    let mut result = ArchiveTask::collect(plan)?.advance(ArchiveBudget::default(), &RecoveryCancellation::default())?;
    for _ in 0..16 {
        match result {
            ArchiveOutcome::Complete { output, .. } => {
                return match *output {
                    ArchiveOutput::Collected(report) => Ok(report),
                    _ => panic!("wrong task output"),
                }
            }
            ArchiveOutcome::Paused(cursor) => result = cursor.resume(ArchiveBudget::default(), &RecoveryCancellation::default())?,
        }
    }
    panic!("collection exceeded fixture work bound")
}
fn evidence(name: &str, value: serde_json::Value) {
    if let Some(path) = std::env::var_os("MYELIN_BUNDLE_COLLECTION_EVIDENCE_OUT") {
        let path = std::path::PathBuf::from(path);
        fs::create_dir_all(&path).unwrap();
        fs::write(path.join(name), serde_json::to_vec_pretty(&value).unwrap()).unwrap();
    }
}
fn inventory(store: &RocksSessionStore) -> SnapshotInventory {
    store
        .snapshot_inventory(
            SnapshotLimits { max_entries: 100_000, max_bytes: 1024 * 1024 * 1024, max_sessions: 10, max_archives: 10 },
            &|_| Ok(()),
        )
        .unwrap()
}

#[test]
fn collection_requires_pin_rebinding_and_closed_readers_then_recovers_exact_storage() {
    let mut f = retention::retired();
    let root = tempfile::tempdir().unwrap();
    let old_path = root.path().join("old");
    let old = export_store(f.store.clone(), &plan(&f, root.path()), &old_path).unwrap();
    retention::tick(&mut f, true);
    let new_path = root.path().join("new");
    let new = export_store(f.store.clone(), &plan(&f, root.path()), &new_path).unwrap();
    let old_catalog = catalog(&old_path, &old);
    let new_catalog = catalog(&new_path, &new);
    let registry = Arc::new(BundleArchives::default());
    registry.register(open(&old_path, &old_catalog.trust).unwrap().0).unwrap();
    registry.register(open(&new_path, &new_catalog.trust).unwrap().0).unwrap();
    f = retention::reopen(f, registry.clone());
    retention::acknowledge(&f, [3; 32]);
    let reference = registry.reference(old_catalog.commitment.0, [3; 32], 0).unwrap();
    f.store.offload_block(f.source.head(), 0, reference, &retention::window(), &RetainUntilTerminal, &|_| Ok(())).unwrap();
    let checkpoint = root.path().join("database-checkpoint");
    f.store.create_checkpoint(&checkpoint).unwrap();
    let plan = CollectionPlan {
        databases: vec![f.directory.path().into(), checkpoint.clone()],
        archives: vec![new_catalog.clone()],
        candidates: vec![old_catalog.clone()],
        protected: vec![],
        limits: test_limits(),
    };
    error(collect(plan.clone()), ErrorCode::Io); // live writer owns RocksDB
    let Fixture { directory, store, source, target, verifier, validator: _ } = f;
    drop(source);
    drop(target);
    drop(store);
    error(collect(plan.clone()), ErrorCode::MissingDependency); // persistent old pin
    assert!(old_path.join("manifest.json").is_file());

    let store =
        Arc::new(RocksSessionStore::open_archived(directory.path(), RocksSessionStoreOptions::default(), registry.clone()).unwrap());
    let source = recover_chain(&store, &verifier, 3);
    let new_reference = registry.reference(new_catalog.commitment.0, [3; 32], 0).unwrap();
    store.offload_block(source.head(), 0, new_reference, &retention::window(), &RetainUntilTerminal, &|_| Ok(())).unwrap();
    let expected = inventory(&store);
    let block = store.load_block([3; 32], 0).unwrap();
    let handoff = store.load_handoff(block.issued_handoffs[0].id()).unwrap().unwrap();
    drop(source);
    drop(store);
    error(collect(plan.clone()), ErrorCode::MissingDependency); // retained checkpoint still pins the old archive
    let backup = RocksSessionStore::open_archived(&checkpoint, RocksSessionStoreOptions::default(), registry.clone()).unwrap();
    backup
        .offload_block(&backup.load_head([3; 32]).unwrap(), 0, new_reference, &retention::window(), &RetainUntilTerminal, &|_| Ok(()))
        .unwrap();
    let expected_backup = inventory(&backup);
    drop(backup);
    error(collect(plan.clone()), ErrorCode::SourceChanged); // catalog still has a reader
    drop(registry);

    // A reader reached through a separately mounted archive root still pins
    // this same payload inode; the root lock alone cannot represent it.
    let payload_reader = fs::File::open(old_path.join("blobs")).unwrap();
    use std::os::fd::AsRawFd;
    // SAFETY: this test owns the live directory descriptor.
    assert_eq!(unsafe { libc::flock(payload_reader.as_raw_fd(), libc::LOCK_SH | libc::LOCK_NB) }, 0);
    error(collect(plan.clone()), ErrorCode::SourceChanged);
    drop(payload_reader);

    let reader = Arc::new(BundleDirectory::open(&old_path, test_limits()).unwrap());
    let task = ArchiveTask::inventory(reader.clone(), old_catalog.trust.clone()).unwrap();
    let ArchiveOutcome::Paused(cursor) =
        task.advance(ArchiveBudget { max_steps: 0, ..ArchiveBudget::default() }, &RecoveryCancellation::default()).unwrap()
    else {
        panic!("inventory must park");
    };
    drop(reader); // parked worker owns the last reader
    error(collect(plan.clone()), ErrorCode::SourceChanged);
    let ArchiveOutcome::Complete { .. } = cursor.resume(ArchiveBudget::default(), &RecoveryCancellation::default()).unwrap() else {
        panic!("inventory must finish");
    };
    let mut protected = plan.clone();
    protected.protected = vec![old_catalog.commitment];
    error(collect(protected), ErrorCode::MissingDependency);

    let report = collect(plan).unwrap();
    assert_eq!(report.storage, vec![expected.clone(), expected_backup]);
    assert_eq!(report.progress.completed, 1);
    assert_eq!(report.progress.blobs_removed, old.blobs.len() as u64);
    assert_eq!(report.progress.unlinked_bytes, old.blobs.iter().map(|b| b.bytes).sum::<u64>());
    assert_eq!(fs::read_dir(old_path.join("blobs")).unwrap().count(), 0);
    assert!(old_path.join("gc.pending").is_file());
    error(BundleDirectory::open(&old_path, test_limits()), ErrorCode::MissingArtifact);
    let registry = Arc::new(BundleArchives::default());
    registry.register(open(&new_path, &new_catalog.trust).unwrap().0).unwrap();
    let store = Arc::new(RocksSessionStore::open_archived(directory.path(), RocksSessionStoreOptions::default(), registry).unwrap());
    assert_eq!(inventory(&store), expected);
    assert_eq!(store.load_block([3; 32], 0).unwrap(), block);
    assert_eq!(store.load_handoff(handoff.handoff.id()).unwrap().unwrap(), handoff);
    for id in [3, 4] {
        let chain = recover_chain(&store, &verifier, id);
        replay(&chain, &store);
    }
    evidence(
        "storage-and-readers.json",
        serde_json::json!({
            "candidate":old_catalog.commitment, "keeper":new_catalog.commitment,
            "collection":report, "storage_after":inventory(&store),
            "checked_boundaries":["live-writer", "live-cold-pin", "checkpoint-cold-pin", "catalog-reader", "payload-directory-reader", "parked-reader", "protected-root"],
            "replayed_sessions":[BlobId([3; 32]), BlobId([4; 32])], "fixture":"compiled reference transfer; synthetic build inputs",
        }),
    );
}

#[test]
fn collection_keeps_dependency_closure_and_reaudits_after_process_death() {
    let mut f = work::work_transfer();
    let root = tempfile::tempdir().unwrap();
    let mut export = work::small_plan(&f, root.path());
    let dependency_bytes = b"historical interpreter dependency";
    let dependency = BlobId::of(dependency_bytes);
    let dependency_path = root.path().join("dependency");
    fs::write(&dependency_path, dependency_bytes).unwrap();
    let rules = export.sessions[0].execution.rules;
    export.artifacts.iter_mut().find(|a| a.id == rules).unwrap().dependencies = vec![dependency];
    export.artifacts.push(ArtifactSource { id: dependency, path: dependency_path, dependencies: vec![] });
    let old_path = root.path().join("old");
    let old = export_store(f.store.clone(), &export, &old_path).unwrap();
    retention::tick(&mut f, true);
    let incomplete_path = root.path().join("incomplete");
    let incomplete = export_store(f.store.clone(), &work::small_plan(&f, root.path()), &incomplete_path).unwrap();
    let new_path = root.path().join("new");
    let new = export_store(f.store.clone(), &export, &new_path).unwrap();
    let expected = inventory(&f.store);
    let Fixture { directory, store, source, target, .. } = f;
    drop(source);
    drop(target);
    drop(store);
    let plan = CollectionPlan {
        databases: vec![directory.path().into()],
        archives: vec![catalog(&new_path, &new)],
        candidates: vec![catalog(&old_path, &old)],
        protected: vec![],
        limits: test_limits(),
    };
    let mut lost_dependency = plan.clone();
    lost_dependency.archives = vec![catalog(&incomplete_path, &incomplete)];
    error(collect(lost_dependency), ErrorCode::MissingDependency);
    let mut lost_history = plan.clone();
    std::mem::swap(&mut lost_history.archives, &mut lost_history.candidates);
    error(collect(lost_history), ErrorCode::MissingDependency);
    assert!(old_path.join("manifest.json").is_file());
    // Unknown files and symlinks cannot become recursive deletion targets.
    let extra = old_path.join("blobs").join("operator-note");
    fs::write(&extra, b"keep me").unwrap();
    error(collect(plan.clone()), ErrorCode::UnsafePath);
    assert_eq!(fs::read(&extra).unwrap(), b"keep me");
    fs::remove_file(extra).unwrap();
    let dep_file = old_path.join("blobs").join(dependency.hex());
    fs::remove_file(&dep_file).unwrap();
    let outside = root.path().join("outside");
    fs::write(&outside, dependency_bytes).unwrap();
    std::os::unix::fs::symlink(&outside, &dep_file).unwrap();
    error(collect(plan.clone()), ErrorCode::UnsafePath);
    assert_eq!(fs::read(&outside).unwrap(), dependency_bytes);
    fs::remove_file(&dep_file).unwrap();
    fs::write(&dep_file, dependency_bytes).unwrap();

    let plan_path = root.path().join("collect.json");
    let mut cli_plan = plan.clone();
    cli_plan.archives[0].path = "new".into();
    cli_plan.candidates[0].path = "old".into();
    fs::write(&plan_path, serde_json::to_vec(&cli_plan).unwrap()).unwrap();
    let executable = env!("CARGO_BIN_EXE_myelin-session-bundle");
    let mut child = Command::new(executable)
        .args(["work", "collect"])
        .arg(&plan_path)
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .unwrap();
    let mut input = child.stdin.take().unwrap();
    let mut output = BufReader::new(child.stdout.take().unwrap());
    let grant = serde_json::json!({"budget":ArchiveBudget { max_steps: 1, ..ArchiveBudget::default() }, "cancelled":false});
    let mut removed_before_death = 0;
    for _ in 0..2000 {
        writeln!(input, "{grant}").unwrap();
        input.flush().unwrap();
        let mut line = String::new();
        assert!(output.read_line(&mut line).unwrap() > 0, "collector exited before its deletion boundary");
        let event: serde_json::Value = serde_json::from_str(&line).unwrap();
        assert_eq!(event["status"], "paused", "{event}");
        removed_before_death = event["progress"]["collection"]["blobs_removed"].as_u64().unwrap();
        if removed_before_death > 0 {
            break;
        }
    }
    assert!(removed_before_death > 0);
    assert!(RocksSessionStore::open(directory.path()).is_err());
    error(BundleDirectory::open(&old_path, test_limits()), ErrorCode::SourceChanged);
    child.kill().unwrap();
    assert!(!child.wait().unwrap().success());
    drop(input);
    drop(output);
    let remaining = fs::read_dir(old_path.join("blobs")).unwrap().count();
    assert_eq!(remaining + removed_before_death as usize, old.blobs.len());
    assert!(old_path.join("gc.pending").is_file());
    assert!(!old_path.join("manifest.json").exists());
    // Restart must audit current keeper bytes, even with a valid pending marker.
    let kept_dependency = new_path.join("blobs").join(dependency.hex());
    fs::write(&kept_dependency, b"tampered").unwrap();
    error(collect(plan.clone()), ErrorCode::Integrity);
    assert_eq!(fs::read_dir(old_path.join("blobs")).unwrap().count(), remaining);
    fs::write(&kept_dependency, dependency_bytes).unwrap();
    let output = Command::new(executable).arg("collect").arg(&plan_path).output().unwrap();
    assert!(output.status.success(), "{}", String::from_utf8_lossy(&output.stderr));
    let value: serde_json::Value = serde_json::from_slice(&output.stdout).unwrap();
    assert_eq!(value["report"]["progress"]["already_missing"], removed_before_death);
    assert_eq!(value["report"]["progress"]["blobs_removed"], remaining);
    assert_eq!(fs::read_dir(old_path.join("blobs")).unwrap().count(), 0);
    let report = collect(plan).unwrap();
    assert_eq!(report.progress.blobs_removed, 0);
    assert_eq!(report.progress.already_missing, old.blobs.len() as u64);
    assert_eq!(report.storage, vec![expected]);
    open(&new_path, &anchors(&new)).unwrap();
    evidence(
        "process-restart.json",
        serde_json::json!({
            "candidate":BlobId(old.commitment().unwrap()), "keeper":BlobId(new.commitment().unwrap()),
            "removed_before_sigkill":removed_before_death, "resumed_cli":value, "idempotent_retry":report,
            "checked_boundaries":["missing-transitive-artifact", "lost-history-prefix", "unexpected-file", "symlink", "locked-database", "locked-reader", "tampered-keeper-on-restart"],
            "fixture":"finality/collection only; synthetic program and build inputs",
        }),
    );
}
