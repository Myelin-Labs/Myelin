#[allow(dead_code)]
#[path = "../../session-transfer/tests/support/mod.rs"]
mod support;
use myelin_session::*;
use myelin_session_bundle::*;
use myelin_session_transfer::{restore_executor, TransferKind};
use std::{collections::BTreeSet, fs, path::Path, sync::Arc};
use support::*;

fn plan(f: &Fixture, input: &Path) -> ExportPlan {
    plan_for(&[f.source.config(), f.target.config()], &f.validator, input)
}
fn plan_for(configs: &[&SessionConfig], validator: &myelin_consensus::CommitteeSigner, input: &Path) -> ExportPlan {
    let executable = std::env::current_exe().unwrap();
    static DIGEST: std::sync::OnceLock<BlobId> = std::sync::OnceLock::new();
    let program = *DIGEST.get_or_init(|| {
        use std::io::Read;
        let mut file = fs::File::open(&executable).unwrap();
        let mut hash = blake3::Hasher::new();
        let mut bytes = [0; 65536];
        loop {
            let n = file.read(&mut bytes).unwrap();
            if n == 0 {
                break;
            }
            hash.update(&bytes[..n]);
        }
        BlobId(*hash.finalize().as_bytes())
    });
    let mut artifacts = vec![ArtifactSource { id: program, path: executable, dependencies: vec![] }];
    let mut add = |name: &str, value: &[u8]| {
        let path = input.join(name);
        fs::write(&path, value).unwrap();
        let id = BlobId::of(value);
        artifacts.push(ArtifactSource { id, path, dependencies: vec![] });
        id
    };
    // Test labels make no complete-build/reproducibility claim. The actual test
    // executable is pinned and checked before its compiled restore hook runs.
    let rules = add("rules", b"test reference-transfer rules");
    let mut package = |name: &str| {
        let bytes = FilePackage {
            format: "myelin-file-package-v1".into(),
            files: vec![PackageFile { name: name.into(), bytes: 0, digest: BlobId::of(b""), chunks: vec![] }],
        }
        .encode()
        .unwrap();
        add(name, &bytes)
    };
    let build_inputs = package("synthetic-inputs.txt");
    let toolchain = package("synthetic-toolchain.txt");
    let consensus_toml = consensus_toml(validator);
    let sessions = configs
        .iter()
        .map(|cfg| ExportSession {
            session_id: cfg.session_id,
            network: [90; 32],
            consensus_toml: consensus_toml.clone(),
            execution: ExecutionArtifacts {
                program,
                replayer: program,
                rules,
                build_inputs,
                toolchain,
                runtime_dependencies: vec![],
                target: std::env::consts::ARCH.into(),
                runner_abi: "test-compiled-reference-transfer".into(),
                runtime: None,
            },
        })
        .collect();
    ExportPlan { remote_sessions: vec![], roots: vec![configs.last().unwrap().session_id], sessions, artifacts, limits: test_limits() }
}

fn consensus_toml(validator: &myelin_consensus::CommitteeSigner) -> String {
    let key: String = validator.ckb_public_key().iter().map(|v| format!("{v:02x}")).collect();
    format!(
        "kind = \"proof-of-authority\"\n[[proof_of_authority.authorities]]\nid = \"{}\"\npublic_key = \"{key}\"\n",
        validator.validator_id()
    )
}

#[path = "support/dependencies.rs"]
mod dependencies;
#[path = "support/remote.rs"]
mod remote;

#[cfg(target_os = "linux")]
#[path = "support/dependency_measurements.rs"]
mod dependency_measurements;

#[path = "support/work.rs"]
mod work;

#[path = "support/backup.rs"]
mod backup;

#[path = "support/retention.rs"]
mod retention;

#[cfg(target_os = "linux")]
#[path = "support/collection.rs"]
mod collection;

#[cfg(all(target_os = "linux", any(target_arch = "x86_64", target_arch = "aarch64"), feature = "test-fixture"))]
#[path = "support/migration_program.rs"]
mod migration_program;
#[cfg(all(target_os = "linux", any(target_arch = "x86_64", target_arch = "aarch64"), feature = "test-fixture"))]
#[path = "support/sandbox.rs"]
mod sandbox;
// The unstripped debug test executable includes native RocksDB symbols (>256 MiB).
// Exercise the explicit import/export override without weakening the defaults.
fn test_limits() -> BundleLimits {
    BundleLimits { blob_bytes: 512 * 1024 * 1024, ..BundleLimits::default() }
}
fn transfer() -> Fixture {
    let mut f = Fixture::new(TransferKind::Scarce);
    let req = issue_request(&f.source, vec![f.intent(1, 0, 25)]);
    let ids = commit(&mut f.source, &f.verifier, &f.validator, req);
    let req = intake_request(&f.target, ids);
    commit(&mut f.target, &f.verifier, &f.validator, req);
    f
}
fn anchors(m: &BundleManifest) -> Vec<TrustAnchor> {
    m.sessions.iter().map(SessionInventory::proposed_anchor).collect()
}
fn open(path: &Path, anchors: &[TrustAnchor]) -> myelin_session_bundle::Result<(Arc<BundleStore>, FinalityReport)> {
    BundleStore::open(Arc::new(BundleDirectory::open(path, test_limits())?), anchors)
}
fn error<T>(result: myelin_session_bundle::Result<T>, expected: ErrorCode) {
    assert_eq!(result.err().expect("must reject").code, expected);
}
fn copy_bundle(source: &Path, target: &Path) {
    fs::create_dir(target).unwrap();
    fs::create_dir(target.join("blobs")).unwrap();
    fs::copy(source.join("manifest.json"), target.join("manifest.json")).unwrap();
    for entry in fs::read_dir(source.join("blobs")).unwrap() {
        let entry = entry.unwrap();
        fs::hard_link(entry.path(), target.join("blobs").join(entry.file_name())).unwrap();
    }
}
fn replace(root: &Path, manifest: &mut BundleManifest, old: BlobId, bytes: Vec<u8>) {
    let new = BlobId::of(&bytes);
    fs::write(root.join("blobs").join(new.hex()), &bytes).unwrap();
    for blob in &mut manifest.blobs {
        if blob.id == old {
            blob.id = new;
            blob.bytes = bytes.len() as u64;
        }
        for dep in &mut blob.dependencies {
            if *dep == old {
                *dep = new;
            }
        }
        blob.dependencies.sort();
    }
    for s in &mut manifest.sessions {
        for field in [
            &mut s.genesis,
            &mut s.consensus,
            &mut s.head,
            &mut s.checkpoint,
            &mut s.execution.program,
            &mut s.execution.replayer,
            &mut s.execution.rules,
            &mut s.execution.build_inputs,
            &mut s.execution.toolchain,
        ] {
            if *field == old {
                *field = new;
            }
        }
        for list in [&mut s.blocks, &mut s.handoffs, &mut s.evidence, &mut s.execution.runtime_dependencies] {
            for field in list {
                if *field == old {
                    *field = new;
                }
            }
        }
    }
    manifest.blobs.sort_by_key(|b| b.id);
    fs::write(root.join("manifest.json"), manifest.encode().unwrap()).unwrap();
}

#[cfg(target_os = "linux")]
#[test]
fn real_transfer_export_replays_offline_after_original_database_is_removed() {
    let f = transfer();
    let directory = tempfile::tempdir().unwrap();
    let plan = plan(&f, directory.path());
    let expected: Vec<_> = [f.source.head().clone(), f.target.head().clone()].into_iter().collect();
    let source_path = f.directory.path().to_owned();
    let archive = directory.path().join("archive");
    let manifest = export_store(f.store.clone(), &plan, &archive).unwrap();
    drop(f);
    assert!(!source_path.exists());
    let (store, report) = open(&archive, &anchors(&manifest)).unwrap();
    assert_eq!(report.sessions.len(), 2);
    let report = serde_json::to_value(report).unwrap();
    assert_eq!(report["stages"]["finality"], "verified");
    assert_eq!(report["stages"]["host_replay"], "not-run");
    assert_eq!(report["canonical_authority_membership"], "not-run");
    for head in expected {
        let config = store.config(head.session_id).unwrap().clone();
        let receipt = store
            .replay_current_process(head.session_id, move |bytes, store| {
                restore_executor(&config, bytes, store).map_err(|e| SessionError::Replay(e.to_string()))
            })
            .unwrap()
            .unwrap();
        assert_eq!(receipt.state_root_after, head.state_root);
        assert_eq!(receipt.total_cycles, 0);
        assert_eq!(store.load_head(head.session_id).unwrap(), head);
    }
    let h = store.load_head([3; 32]).unwrap();
    assert!(store.create_session(&store.load_genesis([3; 32]).unwrap(), &h).is_err());
    assert!(store.commit_block(&h, &h, &store.load_block([3; 32], 0).unwrap()).is_err());
    assert!(store.acknowledge_outbox([3; 32], [1; 32]).is_err());
    assert!(store.pending_outbox([3; 32], 1).is_err());
}

#[test]
fn retirement_archive_retains_consuming_target_and_replays_without_source_database() {
    let mut f = Fixture::new(TransferKind::Scarce);
    let req = issue_request(&f.source, vec![f.intent(1, 0, 25)]);
    let id = commit(&mut f.source, &f.verifier, &f.validator, req)[0];
    let req = intake_request(&f.target, vec![id]);
    commit(&mut f.target, &f.verifier, &f.validator, req);
    let consumed_at = HandoffBlockReference::from_block(&f.store.load_block([4; 32], 0).unwrap().block);
    let mut req = request(&f.source, vec![myelin_session_transfer::TransferCommand::Retire { handoff_id: id }], vec![], vec![]);
    req.handoff_actions.push(HandoffAction::Retire { handoff_id: id, consumed_at });
    commit(&mut f.source, &f.verifier, &f.validator, req);
    let directory = tempfile::tempdir().unwrap();
    let archive = directory.path().join("retired");
    let mut p = plan(&f, directory.path());
    p.roots = vec![[3; 32]];
    let mut manifest = export_store(f.store.clone(), &p, &archive).unwrap();
    assert!(manifest.record_versions == [myelin_session::RECORD_FORMAT_VERSION]);
    let expected = [f.source.head().clone(), f.target.head().clone()];
    let source_path = f.directory.path().to_owned();
    drop(f);
    assert!(!source_path.exists());
    let trusted = anchors(&manifest);
    let (store, _) = open(&archive, &trusted).unwrap();
    for head in expected {
        let cfg = store.config(head.session_id).unwrap().clone();
        let result = store.replay_current_process(head.session_id, move |bytes, store| {
            restore_executor(&cfg, bytes, store).map_err(|e| SessionError::Replay(e.to_string()))
        });
        // Pinning the running executable is Linux-only. Keep the retirement
        // dependency checks below portable, and require explicit refusal here
        // on other platforms instead of accepting an unpinned replay.
        #[cfg(target_os = "linux")]
        assert_eq!(result.unwrap().unwrap().state_root_after, head.state_root);
        #[cfg(not(target_os = "linux"))]
        error(result, ErrorCode::UnsupportedPlatform);
    }
    assert!(matches!(store.load_handoff(id).unwrap().unwrap().lifecycle.terminal, HandoffTerminal::Retired { .. }));
    let source = manifest.sessions.iter().find(|s| s.session_id == [3; 32]).unwrap();
    let retirement = source.blocks[1];
    let target = manifest.sessions.iter().find(|s| s.session_id == [4; 32]).unwrap().blocks[0];
    let deps = &mut manifest.blobs.iter_mut().find(|b| b.id == retirement).unwrap().dependencies;
    assert!(deps.contains(&target));
    deps.retain(|id| *id != target);
    // Target remains reachable via its inventory and the terminal index. It is
    // the retirement block's mandatory causal edge that is now missing.
    fs::write(archive.join("manifest.json"), manifest.encode().unwrap()).unwrap();
    error(open(&archive, &trusted), ErrorCode::MissingDependency);
}

#[test]
fn audit_follows_short_pages_and_checks_the_final_tail() {
    let mut f = transfer();
    for i in 0..70 {
        let request = issue_request(&f.source, vec![f.intent(i + 2, i as u64 + 1, 1)]);
        commit(&mut f.source, &f.verifier, &f.validator, request);
    }
    let directory = tempfile::tempdir().unwrap();
    let plan = plan(&f, directory.path());
    let archive = directory.path().join("archive");
    let pause_error =
        export_store(f.store.clone(), &plan, &archive).expect_err("large capture exceeds the default reservation budget");
    assert_eq!(pause_error.code, ErrorCode::Paused);
    assert!(!archive.join("manifest.json").exists());
    let cursor = pause_error.paused.unwrap();
    assert_eq!(cursor.reason(), ArchivePauseReason::Bytes);
    let ArchiveOutcome::Complete { output, .. } = cursor
        .resume(ArchiveBudget { max_io_bytes: 32 * 1024 * 1024 * 1024, ..ArchiveBudget::default() }, &RecoveryCancellation::default())
        .unwrap()
    else {
        panic!("explicit larger source-read budget must complete");
    };
    let ArchiveOutput::Exported(mut manifest) = *output else {
        panic!("wrong operation");
    };
    let trusted = anchors(&manifest);
    let (store, report) = open(&archive, &trusted).unwrap();
    assert_eq!(store.load_chain_page([3; 32], 0, 1024).unwrap().len(), 64);
    assert_eq!(report.sessions[0].finalised_height, Some(70));
    let old = manifest.sessions[0].blocks[70];
    let mut record = store.load_block([3; 32], 70).unwrap();
    record.block.state_root_after[0] ^= 1;
    replace(&archive, &mut manifest, old, record.encode().unwrap());
    error(open(&archive, &trusted), ErrorCode::InvalidHistory);
}

#[test]
fn missing_causal_edges_sources_and_forged_terminal_indexes_reject() {
    let f = transfer();
    let directory = tempfile::tempdir().unwrap();
    let plan = plan(&f, directory.path());
    let original = directory.path().join("original");
    let base = export_store(f.store.clone(), &plan, &original).unwrap();
    let trusted = anchors(&base);
    for variant in 0..3 {
        let path = directory.path().join(format!("mutation-{variant}"));
        copy_bundle(&original, &path);
        let mut m = base.clone();
        if variant == 0 {
            let source = m.sessions[0].blocks[0];
            let target = m.sessions[1].blocks[0];
            m.blobs.iter_mut().find(|b| b.id == target).unwrap().dependencies.retain(|d| *d != source);
            fs::write(path.join("manifest.json"), m.encode().unwrap()).unwrap();
            error(open(&path, &trusted), ErrorCode::MissingDependency);
        } else if variant == 1 {
            let source = m.sessions[0].blocks[0];
            m.blobs.retain(|b| b.id != source);
            fs::write(path.join("manifest.json"), m.encode().unwrap()).unwrap();
            error(open(&path, &trusted), ErrorCode::MissingDependency);
        } else {
            let old = m.sessions[0].handoffs[0];
            let mut stored = StoredHandoff::decode(&fs::read(path.join("blobs").join(old.hex())).unwrap()).unwrap();

            stored.lifecycle.terminal = HandoffTerminal::Issued;
            replace(&path, &mut m, old, stored.encode().unwrap());
            error(open(&path, &trusted), ErrorCode::InvalidHistory);
        }
    }
}

#[cfg(target_os = "linux")]
#[test]
fn changed_rules_authority_input_checkpoint_and_replayer_have_separate_failures() {
    let f = transfer();
    let directory = tempfile::tempdir().unwrap();
    let plan = plan(&f, directory.path());
    let original = directory.path().join("original");
    let base = export_store(f.store.clone(), &plan, &original).unwrap();
    let trusted = anchors(&base);
    for variant in 0..5 {
        let path = directory.path().join(format!("mutation-{variant}"));
        copy_bundle(&original, &path);
        let mut m = base.clone();
        match variant {
            0 => {
                let old = m.sessions[0].execution.rules;
                replace(&path, &mut m, old, b"different rules".to_vec());
                error(open(&path, &trusted), ErrorCode::UnknownTrustAnchor);
            }
            1 => {
                let old = m.sessions[0].consensus;
                let config = plan.sessions[0].consensus_toml.replace("transfer-test-validator", "different-authority");
                replace(&path, &mut m, old, config.into_bytes());
                error(open(&path, &trusted), ErrorCode::InvalidHistory);
            }
            2 => {
                let old = m.sessions[0].blocks[0];
                let mut record = f.store.load_block([3; 32], 0).unwrap();
                record.frame_input.payload[0] ^= 1;
                replace(&path, &mut m, old, record.encode().unwrap());
                error(open(&path, &trusted), ErrorCode::InvalidHistory);
            }
            3 => {
                let old = m.sessions[0].checkpoint;
                let mut checkpoint = f.store.load_checkpoint([3; 32]).unwrap();
                checkpoint.state_snapshot[0] ^= 1;
                replace(&path, &mut m, old, checkpoint.encode().unwrap());
                let (store, _) = open(&path, &trusted).unwrap();
                let config = store.config([3; 32]).unwrap().clone();
                assert!(store
                    .replay_current_process([3; 32], move |b, store| restore_executor(&config, b, store)
                        .map_err(|e| SessionError::Replay(e.to_string())))
                    .is_err());
            }
            _ => {
                let old = m.sessions[0].execution.replayer;
                let bytes = b"another historical replayer; must never be loaded";
                let id = BlobId::of(bytes);
                let dependencies = m.blobs.iter().find(|b| b.id == old).unwrap().dependencies.clone();
                for blob in &mut m.blobs {
                    if blob.id == old {
                        blob.roles.retain(|role| *role != BlobKind::Replayer);
                    }
                    if blob.dependencies.contains(&old) {
                        blob.dependencies.push(id);
                        blob.dependencies.sort();
                        blob.dependencies.dedup();
                    }
                }
                m.blobs.push(Blob { id, bytes: bytes.len() as u64, roles: vec![BlobKind::Replayer], dependencies });
                m.blobs.sort_by_key(|b| b.id);
                for session in &mut m.sessions {
                    session.execution.replayer = id;
                }
                fs::write(path.join("blobs").join(id.hex()), bytes).unwrap();
                fs::write(path.join("manifest.json"), m.encode().unwrap()).unwrap();
                error(open(&path, &trusted), ErrorCode::UnknownTrustAnchor);
                let (store, _) = open(&path, &anchors(&m)).unwrap();
                let called = Arc::new(std::sync::atomic::AtomicBool::new(false));
                let replay_called = called.clone();
                let config = store.config([3; 32]).unwrap().clone();
                assert!(store
                    .replay_current_process([3; 32], move |b, store| {
                        replay_called.store(true, std::sync::atomic::Ordering::SeqCst);
                        restore_executor(&config, b, store).map_err(|e| SessionError::Replay(e.to_string()))
                    })
                    .is_err());
                assert!(!called.load(std::sync::atomic::Ordering::SeqCst));
            }
        }
    }
}

#[test]
fn incomplete_catalog_or_budget_never_publishes_a_manifest_and_export_cannot_clobber() {
    let f = transfer();
    let directory = tempfile::tempdir().unwrap();
    let base = plan(&f, directory.path());
    for variant in 0..3 {
        let mut plan = base.clone();
        match variant {
            0 => {
                plan.sessions.remove(0);
            }
            1 => plan.limits.total_bytes = 1,
            _ => {
                plan.artifacts.clear();
            }
        }
        let path = directory.path().join(format!("incomplete-{variant}"));
        assert!(export_store(f.store.clone(), &plan, &path).is_err());
        assert!(!path.join("manifest.json").exists());
    }
    let path = directory.path().join("existing");
    fs::create_dir(&path).unwrap();
    fs::write(path.join("user-file"), b"retain me").unwrap();
    error(export_store(f.store.clone(), &base, &path), ErrorCode::Io);
    assert_eq!(fs::read(path.join("user-file")).unwrap(), b"retain me");
}

#[test]
fn separate_cli_exports_and_verifies_quiescent_rocksdb_without_copying_operational_keys() {
    let f = transfer();
    let directory = tempfile::tempdir().unwrap();
    let plan = plan(&f, directory.path());
    let Fixture { directory: database, store, source, target, .. } = f;
    drop((store, source, target));
    let plan_path = directory.path().join("plan.json");
    fs::write(&plan_path, serde_json::to_vec(&plan).unwrap()).unwrap();
    let archive = directory.path().join("archive");
    let exported = std::process::Command::new(env!("CARGO_BIN_EXE_myelin-session-bundle"))
        .arg("export")
        .arg(database.path())
        .arg(&plan_path)
        .arg(&archive)
        .output()
        .unwrap();
    assert!(exported.status.success(), "{}", String::from_utf8_lossy(&exported.stderr));
    let bundle = BundleDirectory::open(&archive, test_limits()).unwrap();
    let trust = directory.path().join("trust.json");
    fs::write(&trust, serde_json::to_vec(&anchors(bundle.manifest())).unwrap()).unwrap();
    let limits = directory.path().join("limits.json");
    fs::write(&limits, serde_json::to_vec(&test_limits()).unwrap()).unwrap();
    drop(database);
    let verified = std::process::Command::new(env!("CARGO_BIN_EXE_myelin-session-bundle"))
        .arg("verify")
        .arg(&archive)
        .arg(&trust)
        .arg(&limits)
        .output()
        .unwrap();
    assert!(verified.status.success(), "{}", String::from_utf8_lossy(&verified.stderr));
    let report: serde_json::Value = serde_json::from_slice(&verified.stdout).unwrap();
    assert_eq!(report["stages"]["finality"], "verified");
    assert_eq!(report["stages"]["ckb_inclusion"], "not-run");
    let files: BTreeSet<_> = fs::read_dir(&archive).unwrap().map(|e| e.unwrap().file_name()).collect();
    assert_eq!(files, ["blobs", "manifest.json"].into_iter().map(std::ffi::OsString::from).collect());
}
