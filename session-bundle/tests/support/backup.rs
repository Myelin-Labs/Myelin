use super::retention::{acknowledge, grant, register, reopen, retired, tick, window};
use super::*;
use myelin_session_network::{DurableEnvelope, MessageClass, MessageType, NetworkBinding, NetworkSigner, NetworkStore};
use myelin_session_store_rocksdb::{RocksSessionStore, SnapshotLimits};

fn snapshot(store: &RocksSessionStore) -> myelin_session_store_rocksdb::SnapshotInventory {
    store
        .snapshot_inventory(
            SnapshotLimits { max_entries: 262_144, max_bytes: 8 * 1024 * 1024 * 1024, max_sessions: 256, max_archives: 1024 },
            &|_| Ok(()),
        )
        .unwrap()
}
fn finish(task: ArchiveTask) -> ArchiveOutput {
    let ArchiveOutcome::Complete { output, .. } = task.advance(grant(100_000), &RecoveryCancellation::default()).unwrap() else {
        panic!("operation unexpectedly paused")
    };
    *output
}
fn pause_before_files(task: ArchiveTask) -> Box<PausedArchive> {
    let mut outcome = task.advance(grant(1), &RecoveryCancellation::default()).unwrap();
    for _ in 0..50_000 {
        let ArchiveOutcome::Paused(cursor) = outcome else { panic!("backup published before file checkpoint") };
        if cursor.progress().stage == ArchiveStage::BackupFiles {
            return cursor;
        }
        outcome = cursor.resume(grant(1), &RecoveryCancellation::default()).unwrap();
    }
    panic!("backup made no bounded progress");
}
#[test]
fn online_backup_restores_the_capture_with_cold_dependencies_and_operational_identities() {
    let mut f = retired();
    let root = tempfile::tempdir().unwrap();
    let registry = Arc::new(BundleArchives::default());
    let (id, archive) = register(&f, &registry, root.path(), "source-archive");
    let trust = anchors(BundleDirectory::open(&archive, test_limits()).unwrap().manifest());
    f = reopen(f, registry);
    acknowledge(&f, [3; 32]);
    let old = f.store.load_block([3; 32], 0).unwrap();
    let command = old.frame_input.commands[0].clone();
    let handoff = old.issued_handoffs[0].id();
    let reference = myelin_session_store_rocksdb::ArchiveReference {
        archive: id,
        blob: BlobId::of(&old.encode().unwrap()).0,
        bytes: old.encode().unwrap().len() as u64,
    };
    assert!(matches!(
        f.store.offload_block(f.source.head(), 0, reference, &window(), &RetainUntilTerminal, &|_| Ok(())).unwrap(),
        myelin_session_store_rocksdb::OffloadOutcome::Archived {
            handoffs: myelin_session_store_rocksdb::HandoffCompaction { records: 1, .. },
            ..
        }
    ));
    let network_signer = NetworkSigner::new("source", [7; 32]).unwrap();
    let binding = NetworkBinding { session_id: [3; 32], consensus_module_commitment: f.verifier.descriptor().commitment() };
    let envelope = |sequence| {
        DurableEnvelope::new(
            network_signer
                .sign(binding, "target", sequence, 100, MessageType::new(MessageClass::BlockCandidate, 1, 0), vec![1])
                .unwrap(),
        )
    };
    let queued = envelope(0);
    f.store.enqueue_outbound(&queued).unwrap();
    let captured = snapshot(&f.store);
    let head = f.source.head().clone();
    let pending: Vec<_> = f.store.pending_outbox([4; 32], 1024).unwrap().into_iter().map(|p| p.message.id).collect();
    assert!(!pending.is_empty());
    let missing = root.path().join("missing-catalog");
    let task = ArchiveTask::backup(f.store.clone(), BackupPlan { archives: vec![], limits: test_limits() }, missing.clone()).unwrap();
    assert!(task.advance(grant(100_000), &RecoveryCancellation::default()).is_err());
    assert!(!missing.join("backup.json").exists());
    let plan = BackupPlan {
        archives: vec![ArchiveCatalog { path: archive.clone(), commitment: BlobId(id), trust: trust.clone() }],
        limits: test_limits(),
    };
    let backup = root.path().join("backup");
    let task = ArchiveTask::backup(f.store.clone(), plan, backup.clone()).unwrap();
    let cursor = pause_before_files(task);
    assert!(!backup.join("backup.json").exists());
    // The capture is fixed. Live writers, ACKs and network counters continue.
    tick(&mut f, true);
    acknowledge(&f, [4; 32]);
    f.store.acknowledge_outbound([3; 32], queued.message_id).unwrap();
    f.store.enqueue_outbound(&envelope(1)).unwrap();
    assert_eq!(f.store.reserve_outbound_sequence([3; 32], "source", "target").unwrap(), 2);
    assert_ne!(snapshot(&f.store), captured);
    let ArchiveOutcome::Complete { output, .. } = cursor.resume(grant(100_000), &RecoveryCancellation::default()).unwrap() else {
        panic!("backup must finish")
    };
    let ArchiveOutput::BackedUp(manifest) = *output else { panic!("wrong backup output") };
    assert_eq!(manifest.storage, captured);
    assert_eq!(manifest.storage.cold_blocks, 1);
    let commitment = manifest.commitment().unwrap();
    let Fixture { directory, store, source, target, verifier, validator: _ } = f;
    drop(source);
    drop(target);
    drop(store);
    drop(directory);
    fs::remove_dir_all(archive).unwrap();
    let restored = root.path().join("restored");
    let ArchiveOutput::Restored { store, report } =
        finish(ArchiveTask::restore(backup, commitment, trust, test_limits(), restored.clone()).unwrap())
    else {
        panic!("wrong restore output")
    };
    assert_eq!(snapshot(&store), captured);
    assert_eq!(serde_json::to_value(&report.writer_authority).unwrap(), "not-run");
    assert_eq!(serde_json::to_value(&report.application_replay).unwrap(), "not-run");
    assert!(restored.join("restore.json").is_file());
    let source = recover_chain(&store, &verifier, 3);
    let target = recover_chain(&store, &verifier, 4);
    assert_eq!(source.head(), &head);
    assert_eq!(store.pending_outbox([4; 32], 1024).unwrap().into_iter().map(|p| p.message.id).collect::<Vec<_>>(), pending);
    assert_eq!(store.reserve_outbound_sequence([3; 32], "source", "target").unwrap(), 1);
    assert_eq!(store.pending_outbound([3; 32], "target", 10).unwrap(), vec![queued]);
    assert!(matches!(source.command_status(&command).unwrap(), CommandStatus::Committed(_)));
    assert!(target.prepare_block(intake_request(&target, vec![handoff])).is_err());
    assert!(matches!(store.load_handoff(handoff).unwrap().unwrap().lifecycle.terminal, HandoffTerminal::Retired { .. }));
    replay(&source, &store);
    replay(&target, &store);
}

#[test]
fn backup_restore_rejects_wrong_commitments_tampered_files_links_and_existing_destinations() {
    let f = retired();
    let root = tempfile::tempdir().unwrap();
    let backup = root.path().join("backup");
    let ArchiveOutput::BackedUp(manifest) =
        finish(ArchiveTask::backup(f.store.clone(), BackupPlan { archives: vec![], limits: test_limits() }, backup.clone()).unwrap())
    else {
        panic!("wrong output")
    };
    let commitment = manifest.commitment().unwrap();
    let wrong = root.path().join("wrong");
    let task = ArchiveTask::restore(backup.clone(), BlobId([99; 32]), vec![], test_limits(), wrong.clone()).unwrap();
    assert!(task.advance(grant(100_000), &RecoveryCancellation::default()).is_err());
    assert!(!wrong.exists());
    let existing = root.path().join("existing");
    fs::create_dir(&existing).unwrap();
    fs::write(existing.join("keep"), b"existing data").unwrap();
    let task = ArchiveTask::restore(backup.clone(), commitment, vec![], test_limits(), existing.clone()).unwrap();
    assert!(task.advance(grant(100_000), &RecoveryCancellation::default()).is_err());
    assert_eq!(fs::read(existing.join("keep")).unwrap(), b"existing data");
    let file = manifest.files.iter().find(|f| f.name.ends_with(".sst") && f.bytes != 0).unwrap();
    let path = backup.join(&file.name);
    let original = fs::read(&path).unwrap();
    let mut changed = original.clone();
    changed[0] ^= 1;
    fs::write(&path, changed).unwrap();
    let bad = root.path().join("tampered");
    let task = ArchiveTask::restore(backup.clone(), commitment, vec![], test_limits(), bad.clone()).unwrap();
    assert!(task.advance(grant(100_000), &RecoveryCancellation::default()).is_err());
    assert!(!bad.join("restore.json").exists());
    fs::write(&path, original).unwrap();
    // File hashes alone do not establish the captured operational keyspace.
    let mut forged = manifest.clone();
    forged.storage.fingerprint[0] ^= 1;
    fs::write(backup.join("backup.json"), serde_json::to_vec(&forged).unwrap()).unwrap();
    let mismatched = root.path().join("mismatched-state");
    let task = ArchiveTask::restore(backup.clone(), forged.commitment().unwrap(), vec![], test_limits(), mismatched.clone()).unwrap();
    let error = task.advance(grant(100_000), &RecoveryCancellation::default()).err().unwrap();
    assert!(error.message.contains("restored keyspace differs"));
    assert!(!mismatched.join("restore.json").exists());
    fs::write(backup.join("backup.json"), serde_json::to_vec(&manifest).unwrap()).unwrap();
    #[cfg(unix)]
    {
        fs::remove_file(&path).unwrap();
        let external = root.path().join("external");
        fs::write(&external, b"do not follow").unwrap();
        std::os::unix::fs::symlink(&external, &path).unwrap();
        let linked = root.path().join("linked");
        let task = ArchiveTask::restore(backup, commitment, vec![], test_limits(), linked.clone()).unwrap();
        assert!(task.advance(grant(100_000), &RecoveryCancellation::default()).is_err());
        assert!(!linked.join("restore.json").exists());
        assert_eq!(fs::read(external).unwrap(), b"do not follow");
    }
}

#[test]
fn backup_and_restore_cli_share_resumable_grants_and_require_an_independent_commitment() {
    use std::io::{BufRead, BufReader, Write};
    use std::process::{Command, Stdio};
    fn command(args: &[&str]) -> serde_json::Value {
        let mut child = Command::new(env!("CARGO_BIN_EXE_myelin-session-bundle"))
            .arg("work")
            .args(args)
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::inherit())
            .spawn()
            .unwrap();
        let mut input = child.stdin.take().unwrap();
        let mut output = BufReader::new(child.stdout.take().unwrap());
        let mut report = None;
        for cancelled in [true, false] {
            writeln!(input, "{}", serde_json::json!({"budget":grant(100_000), "cancelled":cancelled})).unwrap();
            input.flush().unwrap();
            let mut line = String::new();
            output.read_line(&mut line).unwrap();
            let value: serde_json::Value = serde_json::from_str(&line).unwrap();
            if cancelled {
                assert_eq!(value["status"], "paused");
                assert!(child.try_wait().unwrap().is_none());
            } else {
                assert_eq!(value["status"], "complete");
                report = Some(value["report"].clone());
            }
        }
        drop(input);
        assert!(child.wait().unwrap().success());
        report.unwrap()
    }
    let f = retired();
    let root = tempfile::tempdir().unwrap();
    let plan = root.path().join("plan.json");
    fs::write(&plan, serde_json::to_vec(&BackupPlan { archives: vec![], limits: test_limits() }).unwrap()).unwrap();
    let trust = root.path().join("trust.json");
    fs::write(&trust, b"[]").unwrap();
    let backup = root.path().join("backup");
    let expected = f.source.head().clone();
    let Fixture { directory, store, source, target, verifier, validator: _ } = f;
    drop(source);
    drop(target);
    drop(store);
    let report = command(&["backup", directory.path().to_str().unwrap(), plan.to_str().unwrap(), backup.to_str().unwrap()]);
    assert_eq!(report["stage"], "backed-up");
    drop(directory);
    let restored = root.path().join("restored");
    let report = command(&[
        "restore",
        backup.to_str().unwrap(),
        report["commitment"].as_str().unwrap(),
        trust.to_str().unwrap(),
        restored.to_str().unwrap(),
    ]);
    assert_eq!(report["stage"], "restored");
    let store = Arc::new(RocksSessionStore::open(restored.join("db")).unwrap());
    let source = recover_chain(&store, &verifier, 3);
    assert_eq!(source.head(), &expected);
    replay(&source, &store);
}
