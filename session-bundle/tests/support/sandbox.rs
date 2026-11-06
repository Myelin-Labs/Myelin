use super::*;
use myelin_session_bundle::runner::*;
use std::{collections::BTreeMap, os::fd::AsRawFd, process::Command};

fn policy() -> SandboxPolicy {
    let launcher = std::env::var_os("MYELIN_BWRAP").map(std::path::PathBuf::from).unwrap_or_else(|| "/usr/bin/bwrap".into());
    let launcher_digest =
        BlobId::of(&fs::read(&launcher).expect("sandbox test lane requires bubblewrap; set MYELIN_BWRAP to its absolute path"));
    SandboxPolicy {
        launcher,
        launcher_digest,
        wall_time_ms: 10_000,
        cpu_seconds: 5,
        address_space_bytes: 1024 * 1024 * 1024,
        output_bytes: 128 * 1024,
        stderr_bytes: 64 * 1024,
    }
}
fn transfer_for(executable: &Path) -> Fixture {
    let mut f = Fixture::with_program(TransferKind::Scarce, BlobId::of(&fs::read(executable).unwrap()).0);
    let req = issue_request(&f.source, vec![f.intent(1, 0, 25)]);
    let ids = commit(&mut f.source, &f.verifier, &f.validator, req);
    let req = intake_request(&f.target, ids);
    commit(&mut f.target, &f.verifier, &f.validator, req);
    f
}
fn sandbox_plan(f: &Fixture, input: &Path, executable: &str, rules: serde_json::Value) -> ExportPlan {
    sandbox_artifacts(plan(f, input), input, executable, rules)
}
fn sandbox_artifacts(mut plan: ExportPlan, input: &Path, executable: &str, rules: serde_json::Value) -> ExportPlan {
    // This plan replaces the current-process replayer and its rules. Keeping
    // the old debug test executable would archive hundreds of unused MiB per
    // case and multiply temporary-storage requirements during parallel tests.
    let replaced = [plan.sessions[0].execution.program, plan.sessions[0].execution.rules];
    plan.artifacts.retain(|source| !replaced.contains(&source.id));
    let path = input.join("historical-replayer");
    fs::copy(executable, &path).unwrap();
    let program = BlobId::of(&fs::read(&path).unwrap());
    plan.artifacts.push(ArtifactSource { id: program, path, dependencies: vec![] });
    let rules = serde_json::to_vec(&rules).unwrap();
    let rules_id = BlobId::of(&rules);
    fs::write(input.join("sandbox-rules"), rules).unwrap();
    plan.artifacts.push(ArtifactSource { id: rules_id, path: input.join("sandbox-rules"), dependencies: vec![] });
    // ldd is used ONLY on these freshly built, trusted test binaries. The
    // production importer never executes ldd on a supplied executable.
    let output = Command::new("ldd").arg(executable).output().unwrap();
    assert!(output.status.success());
    let mut libraries = BTreeMap::new();
    for line in std::str::from_utf8(&output.stdout).unwrap().lines() {
        if let Some(path) = line.split_whitespace().find(|p| p.starts_with('/')) {
            let id = BlobId::of(&fs::read(path).unwrap());
            libraries.insert(path.to_owned(), id);
            if !plan.artifacts.iter().any(|s| s.id == id) {
                plan.artifacts.push(ArtifactSource { id, path: path.into(), dependencies: vec![] });
            }
        }
    }
    assert!(!libraries.is_empty());
    let dependencies: BTreeSet<_> = libraries.values().copied().collect();
    for s in &mut plan.sessions {
        s.execution.program = program;
        s.execution.replayer = program;
        s.execution.rules = rules_id;
        s.execution.target = format!("{}-unknown-linux-gnu", std::env::consts::ARCH);
        s.execution.runner_abi = RUNNER_ABI.into();
        s.execution.runtime_dependencies = dependencies.iter().copied().collect();
        s.execution.runtime = Some(RuntimeLayout {
            files: libraries.iter().map(|(path, blob)| RuntimeFile { path: path.clone(), blob: *blob }).collect(),
        });
    }
    plan
}
fn run(
    path: &Path,
    m: &BundleManifest,
    p: &SandboxPolicy,
    cancellation: &Cancellation,
) -> myelin_session_bundle::Result<IsolatedReplayReport> {
    replay_isolated(Arc::new(BundleDirectory::open(path, test_limits()).unwrap()), &anchors(m), [4; 32], p, cancellation)
}
fn retain_bundle_evidence(path: &Path, archive: &Path, manifest: &BundleManifest) {
    fs::create_dir(path).unwrap();
    fs::create_dir(path.join("bundle")).unwrap();
    fs::create_dir(path.join("bundle/blobs")).unwrap();
    fs::copy(archive.join("manifest.json"), path.join("bundle/manifest.json")).unwrap();
    for blob in &manifest.blobs {
        fs::copy(archive.join("blobs").join(blob.id.hex()), path.join("bundle/blobs").join(blob.id.hex())).unwrap();
    }
}

#[test]
fn migration_artifact_is_executed_offline_in_isolation_and_signed_forged_output_is_rejected() {
    use super::migration_program::{self as migration, Converting, PROGRAM};
    use myelin_consensus::{FinalityProof, SelectedConsensus};
    let executable = env!("CARGO_BIN_EXE_myelin-bundle-sandbox-fixture");
    let artifact = BlobId::of(PROGRAM);
    for forged in [false, true] {
        let f = Fixture::with_program(TransferKind::Scarce, BlobId::of(&fs::read(executable).unwrap()).0);
        let database = f.directory.path().to_owned();
        let executor = Converting::restore(b"archive state", PROGRAM.to_vec(), forged).unwrap();
        let mut cfg = f.source.config().clone();
        cfg.session_id = [10; 32];
        cfg.application_profile.command_identity_required = false;
        cfg.protocol_effects = migration::policy();
        cfg.initial_state_root = executor.state_root();
        let converted: &[u8] = if forged { b"archive state-converted!" } else { b"archive state-converted" };
        let mut target = cfg.clone();
        target.session_id = [11; 32];
        target.application_profile.state_codec_hash = migration::target_codec();
        target.initial_state_root = migration::digest(converted);
        target.initial_input_position = 1;
        target.initial_logical_time = 1;
        target.initial_timestamp_ms = 10;
        target.predecessor = Some(PredecessorReference {
            session_id: cfg.session_id,
            final_height: 0,
            final_state_root: cfg.initial_state_root,
            application_profile_commitment: cfg.application_profile.commitment(),
        });
        let mut chain = SessionChain::create(cfg.clone(), f.verifier.clone(), executor, f.store.clone()).unwrap();
        let prepared = chain
            .prepare_block(BlockRequest {
                transactions: vec![],
                frame_input: FrameInput {
                    commands: vec![],
                    start_position: 0,
                    end_position: 1,
                    logical_time_start: 0,
                    logical_time_end: 1,
                    payload: b"-converted".to_vec(),
                },
                scheduler_commitment: [5; 32],
                data_commitments: vec![],
                timestamp_ms: 10,
                local_now_ms: 10,
                successor: Some(target.clone()),
                handoffs: vec![],
                consume_handoffs: vec![],
                handoff_actions: vec![],
            })
            .unwrap();
        let SelectedConsensus::ProofOfAuthority(engine) = f.verifier.selected_consensus() else { unreachable!() };
        let proof = FinalityProof::ProofOfAuthority(engine.seal_from_signer(prepared.block().hash(), 0, &f.validator).unwrap());
        chain.commit_finalised(prepared, proof).unwrap();
        let dir = tempfile::tempdir().unwrap();
        let mut plan = sandbox_artifacts(
            plan_for(&[&cfg, &target], &f.validator, dir.path()),
            dir.path(),
            executable,
            serde_json::json!({"mode":"migration", "artifact":artifact}),
        );
        error(export_store(f.store.clone(), &plan, dir.path().join("missing-program")), ErrorCode::MissingDependency);
        let program_file = dir.path().join("migration");
        fs::write(&program_file, PROGRAM).unwrap();
        plan.artifacts.push(ArtifactSource { id: artifact, path: program_file.clone(), dependencies: vec![] });
        let archive = dir.path().join("archive");
        let mut manifest = export_store(f.store.clone(), &plan, &archive).unwrap();
        // Trust comes from this controlled source/export, before source removal.
        let trusted = anchors(&manifest);
        let source_block = manifest.sessions.iter().find(|s| s.session_id == cfg.session_id).unwrap().blocks[0];
        assert!(manifest.blobs.iter().find(|b| b.id == source_block).unwrap().dependencies.contains(&artifact));
        drop(chain);
        drop(f);
        assert!(!database.exists());
        fs::remove_file(program_file).unwrap();
        fs::remove_file(dir.path().join("historical-replayer")).unwrap();
        let (store, audit) = open(&archive, &trusted).unwrap();
        assert_eq!(serde_json::to_value(&audit).unwrap()["stages"]["finality"], "verified");
        assert_eq!(store.load_genesis(target.session_id).unwrap().state_snapshot, converted);
        let sandbox_policy = policy();
        let replay = |session| {
            replay_isolated(
                Arc::new(BundleDirectory::open(&archive, test_limits()).unwrap()),
                &trusted,
                session,
                &sandbox_policy,
                &Cancellation::default(),
            )
        };
        let evidence = |report: serde_json::Value| {
            if let Some(root) = std::env::var_os("MYELIN_BUNDLE_MIGRATION_EVIDENCE_OUT") {
                let root = std::path::PathBuf::from(root);
                fs::create_dir_all(&root).unwrap();
                let path = root.join(if forged { "signed-invalid-output" } else { "valid" });
                retain_bundle_evidence(&path, &archive, &manifest);
                for (name, value) in [
                    ("trust.json", serde_json::to_value(&trusted).unwrap()),
                    ("policy.json", serde_json::to_value(&sandbox_policy).unwrap()),
                    ("limits.json", serde_json::to_value(test_limits()).unwrap()),
                    ("report.json", report),
                    (
                        "case.json",
                        serde_json::json!({"source":BlobId(cfg.session_id), "target":BlobId(target.session_id),
                            "migration_program":artifact, "source_database_removed":!database.exists(),
                            "source_root":BlobId(cfg.initial_state_root), "target_root":BlobId(target.initial_state_root),
                            "signed_invalid_output":forged, "scope":"development bytecode fixture; synthetic build packages; no custody or production recognition claim"}),
                    ),
                ] {
                    fs::write(path.join(name), serde_json::to_vec_pretty(&value).unwrap()).unwrap();
                }
                if !forged {
                    fs::copy(env!("CARGO_BIN_EXE_myelin-session-bundle"), root.join("myelin-session-bundle")).unwrap();
                }
            }
        };
        if forged {
            let error = replay(cfg.session_id).unwrap_err();
            assert_eq!(error.code, ErrorCode::Sandbox);
            let execution = error.execution.as_ref().unwrap();
            assert_eq!(execution.exit_code, Some(2));
            assert!(execution.stderr.contains("migration program output differs from target root"), "{execution:?}");
            evidence(serde_json::json!({"status":"rejected", "audit":audit, "error":error}));
            continue;
        }
        let replayed = replay(cfg.session_id).unwrap();
        assert_eq!(replayed.result.state_root_after, cfg.initial_state_root);
        assert_eq!(replayed.result.receipt.as_ref().unwrap().frame_count, 1);
        let target_report = replay(target.session_id).unwrap();
        assert_eq!(target_report.result.state_root_after, target.initial_state_root);
        assert!(target_report.result.receipt.is_none());
        for (name, value) in [
            ("trust.json", serde_json::to_value(&trusted).unwrap()),
            ("policy.json", serde_json::to_value(&sandbox_policy).unwrap()),
            ("limits.json", serde_json::to_value(test_limits()).unwrap()),
        ] {
            fs::write(dir.path().join(name), serde_json::to_vec(&value).unwrap()).unwrap();
        }
        let output = Command::new(env!("CARGO_BIN_EXE_myelin-session-bundle"))
            .arg("replay")
            .arg(&archive)
            .arg(dir.path().join("trust.json"))
            .arg(BlobId(cfg.session_id).hex())
            .arg(dir.path().join("policy.json"))
            .arg(dir.path().join("limits.json"))
            .output()
            .unwrap();
        assert!(output.status.success(), "{}", String::from_utf8_lossy(&output.stderr));
        let cli: serde_json::Value = serde_json::from_slice(&output.stdout).unwrap();
        assert_eq!(cli["result"], serde_json::to_value(&replayed.result).unwrap());
        evidence(serde_json::json!({"status":"passed", "source_replay":replayed, "target_replay":target_report, "cli":cli}));

        // Reads must retain their own length/hash/path boundary even after audit.
        assert_eq!(store.read_artifact(artifact, PROGRAM.len() as u64).unwrap(), PROGRAM);
        error(store.read_artifact(artifact, 0), ErrorCode::Limit);
        error(store.read_artifact(BlobId([0; 32]), 64), ErrorCode::MissingDependency);
        let blob_path = archive.join("blobs").join(artifact.hex());
        fs::write(&blob_path, b"MMIG\x01\x02").unwrap();
        error(store.read_artifact(artifact, 64), ErrorCode::Integrity);
        error(replay(cfg.session_id), ErrorCode::Integrity);
        fs::remove_file(&blob_path).unwrap();
        error(store.read_artifact(artifact, 64), ErrorCode::MissingArtifact);
        fs::write(&blob_path, PROGRAM).unwrap();
        manifest.blobs.iter_mut().find(|b| b.id == source_block).unwrap().dependencies.retain(|id| *id != artifact);
        // Still reachable elsewhere: a substitute edge cannot authorize migration.
        let genesis = manifest.sessions.iter().find(|s| s.session_id == target.session_id).unwrap().genesis;
        let deps = &mut manifest.blobs.iter_mut().find(|b| b.id == genesis).unwrap().dependencies;
        deps.push(artifact);
        deps.sort();
        deps.dedup();
        fs::write(archive.join("manifest.json"), manifest.encode().unwrap()).unwrap();
        error(open(&archive, &trusted), ErrorCode::MissingDependency);
    }
}

#[test]
fn isolated_historical_transfer_replays_with_only_archived_program_libraries_and_data() {
    let f = transfer_for(Path::new(env!("CARGO_BIN_EXE_myelin-reference-transfer-replayer")));
    let dir = tempfile::tempdir().unwrap();
    let plan = sandbox_plan(
        &f,
        dir.path(),
        env!("CARGO_BIN_EXE_myelin-reference-transfer-replayer"),
        serde_json::json!({"mode":"reference-transfer"}),
    );
    let archive = dir.path().join("archive");
    let m = export_store(f.store.clone(), &plan, &archive).unwrap();
    let expected = f.target.head().state_root;
    drop(f);
    fs::remove_file(dir.path().join("historical-replayer")).unwrap();
    let p = policy();
    let report = run(&archive, &m, &p, &Cancellation::default()).unwrap();
    assert_eq!(report.result.state_root_after, expected);
    assert_eq!(report.result.receipt.as_ref().unwrap().total_cycles, 0);
    assert!(report.execution.user_cpu_us + report.execution.system_cpu_us > 0);
    assert!(report.execution.lifetime_peak_rss_bytes > 0);
    assert_eq!(serde_json::to_value(&report).unwrap()["host_replay"], "verified");
    assert_eq!(serde_json::to_value(&report).unwrap()["audit"]["stages"]["host_replay"], "not-run");
    for (name, bytes) in [
        ("trust.json", serde_json::to_vec(&anchors(&m)).unwrap()),
        ("policy.json", serde_json::to_vec(&p).unwrap()),
        ("limits.json", serde_json::to_vec(&test_limits()).unwrap()),
    ] {
        fs::write(dir.path().join(name), bytes).unwrap();
    }
    let output = Command::new(env!("CARGO_BIN_EXE_myelin-session-bundle"))
        .arg("replay")
        .arg(&archive)
        .arg(dir.path().join("trust.json"))
        .arg(BlobId([4; 32]).hex())
        .arg(dir.path().join("policy.json"))
        .arg(dir.path().join("limits.json"))
        .output()
        .unwrap();
    assert!(output.status.success(), "{}", String::from_utf8_lossy(&output.stderr));
    let result: serde_json::Value = serde_json::from_slice(&output.stdout).unwrap();
    assert_eq!(result["result"], serde_json::to_value(&report.result).unwrap());
    if let Some(path) = std::env::var_os("MYELIN_BUNDLE_RUNNER_EVIDENCE_OUT") {
        let path = std::path::PathBuf::from(path);
        retain_bundle_evidence(&path, &archive, &m);
        for name in ["trust.json", "policy.json", "limits.json"] {
            fs::copy(dir.path().join(name), path.join(name)).unwrap();
        }
        fs::copy(env!("CARGO_BIN_EXE_myelin-session-bundle"), path.join("myelin-session-bundle")).unwrap();
        fs::write(path.join("report.json"), serde_json::to_vec_pretty(&report).unwrap()).unwrap();
        fs::write(path.join("cli-report.json"), output.stdout).unwrap();
        fs::write(path.join("README.txt"), b"Controlled development-key transfer fixture. Rules/build/toolchain labels are synthetic. This archive demonstrates isolated replay of its retained executable and libraries, not reproducible builds or official deployment recognition.\n").unwrap();
    }
}

#[test]
fn typed_build_packages_survive_export_reconstruction_and_isolated_historical_replay() {
    let dir = tempfile::tempdir().unwrap();
    // The optional acceptance lane supplies already verified build outputs and
    // a complete packed kit. Ordinary tests exercise the same package/export
    // path with explicitly synthetic inputs and the Cargo-built replayer.
    let outputs = std::env::var_os("MYELIN_BUNDLE_REPRODUCED_OUTPUTS").map(std::path::PathBuf::from);
    let executable = outputs
        .as_ref()
        .map(|p| p.join("myelin-reference-transfer-replayer"))
        .unwrap_or_else(|| env!("CARGO_BIN_EXE_myelin-reference-transfer-replayer").into());
    let f = transfer_for(&executable);
    let mut plan = sandbox_plan(&f, dir.path(), executable.to_str().unwrap(), serde_json::json!({"mode":"reference-transfer"}));
    if let Some(outputs) = &outputs {
        let inventory: serde_json::Value = serde_json::from_slice(&fs::read(outputs.join("outputs.json")).unwrap()).unwrap();
        let old = plan.sessions[0].execution.runtime_dependencies.clone();
        plan.artifacts.retain(|s| !old.contains(&s.id));
        let mut files = vec![];
        for name in inventory["files"].as_object().unwrap().keys().filter(|s| s.starts_with("runtime/")) {
            let path = outputs.join(name);
            let blob = BlobId::of(&fs::read(&path).unwrap());
            plan.artifacts.push(ArtifactSource { id: blob, path, dependencies: vec![] });
            files.push(RuntimeFile { path: format!("/{}", name.strip_prefix("runtime/").unwrap()), blob });
        }
        files.sort_by(|a, b| a.path.cmp(&b.path));
        let ids: BTreeSet<_> = files.iter().map(|f| f.blob).collect();
        for s in &mut plan.sessions {
            s.execution.runtime_dependencies = ids.iter().copied().collect();
            s.execution.runtime = Some(RuntimeLayout { files: files.clone() });
        }
    }
    let packed = if let Some(path) = std::env::var_os("MYELIN_BUNDLE_PACKED_KIT") {
        assert!(outputs.is_some(), "complete-kit lane must also supply its reproduced binary and libraries");
        serde_json::from_slice::<PackedFiles>(&fs::read(path).unwrap()).unwrap()
    } else {
        assert!(outputs.is_none(), "complete-kit lane must supply both artifact inputs");
        let path = dir.path().join("synthetic-build-input");
        fs::write(&path, b"synthetic package fixture; not complete build inputs").unwrap();
        pack_files(
            &PackPlan {
                files: vec![FileSource { name: "fixture.txt".into(), path }],
                chunk_bytes: 16,
                limits: BundleLimits::default(),
            },
            dir.path().join("packed"),
        )
        .unwrap()
    };
    for s in &mut plan.sessions {
        s.execution.build_inputs = packed.root;
        s.execution.toolchain = packed.root;
    }
    plan.artifacts.extend(packed.artifacts);
    let archive = dir.path().join("archive");
    let m = export_store(f.store.clone(), &plan, &archive).unwrap();
    let expected = f.target.head().state_root;
    drop(f);
    fs::remove_file(dir.path().join("historical-replayer")).unwrap();
    let trusted = anchors(&m);
    let directory = BundleDirectory::open(&archive, test_limits()).unwrap();
    let inventory = directory.verify_inventory(&trusted).unwrap();
    assert_eq!(inventory.file_packages.len(), 1);
    let restored = dir.path().join("restored-kit");
    directory.assemble_file_package(&trusted, packed.root, &restored).unwrap();
    let mut p = policy();
    p.wall_time_ms = 30_000;
    p.cpu_seconds = 20;
    let report = run(&archive, &m, &p, &Cancellation::default()).unwrap();
    assert_eq!(report.result.state_root_after, expected);
    if let Some(path) = std::env::var_os("MYELIN_BUNDLE_BUILD_EVIDENCE_OUT") {
        assert!(outputs.is_some(), "retained full-build evidence requires the explicit complete-kit lane");
        let path = std::path::PathBuf::from(path);
        fs::create_dir(&path).unwrap();
        fs::rename(&archive, path.join("bundle")).unwrap();
        fs::rename(&restored, path.join("restored-kit")).unwrap();
        for (name, bytes) in [
            ("trust.json", serde_json::to_vec(&trusted).unwrap()),
            ("policy.json", serde_json::to_vec(&p).unwrap()),
            ("limits.json", serde_json::to_vec(&test_limits()).unwrap()),
            ("inventory.json", serde_json::to_vec_pretty(&inventory).unwrap()),
            ("replay.json", serde_json::to_vec_pretty(&report).unwrap()),
        ] {
            fs::write(path.join(name), bytes).unwrap();
        }
        fs::copy(env!("CARGO_BIN_EXE_myelin-session-bundle"), path.join("myelin-session-bundle")).unwrap();
        fs::write(path.join("README.txt"),b"Controlled development-key source/target transfer. This archive pins a reproduced historical native replayer and its runtime, plus complete typed build inputs. See the separately retained kit manifest and offline reproduction reports; this fixture is not an official deployment or physical second-machine acceptance.\n").unwrap();
    }
}

#[test]
fn sandbox_denies_host_files_writes_network_descendants_and_inherited_descriptors() {
    let f = transfer_for(Path::new(env!("CARGO_BIN_EXE_myelin-bundle-sandbox-fixture")));
    let dir = tempfile::tempdir().unwrap();
    let private = dir.path().join("private-host-file");
    fs::write(&private, b"private content must not be read or changed").unwrap();
    let fd = fs::File::open(&private).unwrap();
    // An intentionally inheritable high-numbered descriptor is closed before
    // the launcher; keep RAII ownership so even a failed assertion closes it.
    use std::os::fd::FromRawFd;
    let leaked = unsafe { libc::fcntl(fd.as_raw_fd(), libc::F_DUPFD, 800) };
    assert!(leaked >= 800);
    let _inheritable = unsafe { fs::File::from_raw_fd(leaked) };
    let plan = sandbox_plan(
        &f,
        dir.path(),
        env!("CARGO_BIN_EXE_myelin-bundle-sandbox-fixture"),
        serde_json::json!({"mode":"probe","secret":private,"fd":leaked}),
    );
    let archive = dir.path().join("archive");
    let m = export_store(f.store.clone(), &plan, &archive).unwrap();
    let original = fs::read(archive.join("manifest.json")).unwrap();
    let error = run(&archive, &m, &policy(), &Cancellation::default()).unwrap_err();
    assert_eq!(error.code, ErrorCode::ReplayMismatch, "{error:?}");
    assert!(error.execution.unwrap().stderr.contains("sandbox probes passed"));
    assert_eq!(fs::read(private).unwrap(), b"private content must not be read or changed");
    assert_eq!(fs::read(archive.join("manifest.json")).unwrap(), original);
    for (name, bytes) in [
        ("trust.json", serde_json::to_vec(&anchors(&m)).unwrap()),
        ("policy.json", serde_json::to_vec(&policy()).unwrap()),
        ("limits.json", serde_json::to_vec(&test_limits()).unwrap()),
    ] {
        fs::write(dir.path().join(name), bytes).unwrap();
    }
    let output = Command::new(env!("CARGO_BIN_EXE_myelin-session-bundle"))
        .env("MYELIN_SANDBOX_PRIVATE_SENTINEL", "private-parent-environment")
        .arg("replay")
        .arg(&archive)
        .arg(dir.path().join("trust.json"))
        .arg(BlobId([4; 32]).hex())
        .arg(dir.path().join("policy.json"))
        .arg(dir.path().join("limits.json"))
        .output()
        .unwrap();
    assert_eq!(output.status.code(), Some(2));
    let error: serde_json::Value = serde_json::from_slice(&output.stderr).unwrap();
    assert_eq!(error["code"], "replay-mismatch");
    assert!(error["execution"]["stderr"].as_str().unwrap().contains("sandbox probes passed"));
}

#[test]
fn hostile_output_cpu_allocation_exit_and_receipt_mismatch_have_bounded_outcomes() {
    let f = transfer_for(Path::new(env!("CARGO_BIN_EXE_myelin-bundle-sandbox-fixture")));
    let dir = tempfile::tempdir().unwrap();
    for mode in ["flood", "loop", "cpu", "allocation", "exit", "wrong-receipt"] {
        let plan = sandbox_plan(&f, dir.path(), env!("CARGO_BIN_EXE_myelin-bundle-sandbox-fixture"), serde_json::json!({"mode":mode}));
        let archive = dir.path().join(mode);
        let m = export_store(f.store.clone(), &plan, &archive).unwrap();
        let mut p = policy();
        if mode == "loop" {
            p.wall_time_ms = 150;
        }
        if mode == "cpu" {
            p.cpu_seconds = 1;
        }
        let start = std::time::Instant::now();
        let error = run(&archive, &m, &p, &Cancellation::default()).unwrap_err();
        let expected = match mode {
            "flood" => ErrorCode::OutputLimit,
            "loop" => ErrorCode::Deadline,
            "cpu" => ErrorCode::CpuLimit,
            "exit" => ErrorCode::Sandbox,
            _ => ErrorCode::ReplayMismatch,
        };
        assert_eq!(error.code, expected, "{mode}: {error:?}");
        assert!(start.elapsed().as_secs() < 10);
        if mode == "allocation" {
            assert!(error.execution.as_ref().unwrap().stderr.contains("allocation denied"));
        }
        assert!(error.execution.unwrap().stderr.len() <= p.stderr_bytes);
        fs::remove_dir_all(archive).unwrap();
    }
    assert_eq!(f.store.load_head([4; 32]).unwrap(), *f.target.head());
}

#[test]
fn cancellation_unknown_launcher_and_runtime_tampering_never_fall_back_to_host_execution() {
    let f = transfer_for(Path::new(env!("CARGO_BIN_EXE_myelin-reference-transfer-replayer")));
    let dir = tempfile::tempdir().unwrap();
    let plan = sandbox_plan(
        &f,
        dir.path(),
        env!("CARGO_BIN_EXE_myelin-reference-transfer-replayer"),
        serde_json::json!({"mode":"reference-transfer"}),
    );
    let archive = dir.path().join("archive");
    let mut m = export_store(f.store.clone(), &plan, &archive).unwrap();
    let token = Cancellation::default();
    token.cancel();
    error(run(&archive, &m, &policy(), &token), ErrorCode::Cancelled);
    let mut p = policy();
    p.launcher_digest.0[0] ^= 1;
    error(run(&archive, &m, &p, &Cancellation::default()), ErrorCode::Sandbox);
    p = policy();
    p.launcher = dir.path().join("missing-launcher");
    error(run(&archive, &m, &p, &Cancellation::default()), ErrorCode::Sandbox);
    let original_anchors = anchors(&m);
    m.sessions[0].execution.runtime.as_mut().unwrap().files[0].path = "/home/escape".into();
    fs::write(archive.join("manifest.json"), m.encode().unwrap()).unwrap();
    error(BundleDirectory::open(&archive, test_limits()), ErrorCode::UnsafePath);
    m.sessions[0].execution.runtime = plan.sessions[0].execution.runtime.clone();
    // Change a valid layout without changing its byte artifacts. The mapping
    // itself is anchored, so old recognition must fail before executing it.
    m.sessions[0].execution.runtime.as_mut().unwrap().files[0].path.push_str(".changed");
    m.sessions[0].execution.runtime.as_mut().unwrap().files.sort_by(|a, b| a.path.cmp(&b.path));
    fs::write(archive.join("manifest.json"), m.encode().unwrap()).unwrap();
    let directory = Arc::new(BundleDirectory::open(&archive, test_limits()).unwrap());
    error(replay_isolated(directory, &original_anchors, [4; 32], &policy(), &Cancellation::default()), ErrorCode::UnknownTrustAnchor);
    m.sessions[0].execution.runtime = plan.sessions[0].execution.runtime.clone();
    fs::write(archive.join("manifest.json"), m.encode().unwrap()).unwrap();
    let lib = archive.join("blobs").join(m.sessions[0].execution.runtime_dependencies[0].hex());
    let mut bytes = fs::read(&lib).unwrap();
    bytes[0] ^= 1;
    fs::write(lib, bytes).unwrap();
    error(run(&archive, &m, &policy(), &Cancellation::default()), ErrorCode::Integrity);
}

#[test]
fn cancelling_an_observed_live_sandbox_terminates_its_application() {
    let f = transfer_for(Path::new(env!("CARGO_BIN_EXE_myelin-bundle-sandbox-fixture")));
    let dir = tempfile::tempdir().unwrap();
    let plan = sandbox_plan(
        &f,
        dir.path(),
        env!("CARGO_BIN_EXE_myelin-bundle-sandbox-fixture"),
        serde_json::json!({"mode":"loop", "unique_capture":dir.path()}),
    );
    let archive = dir.path().join("archive");
    let m = export_store(f.store.clone(), &plan, &archive).unwrap();
    let expected_command = format!("/app/replayer\0{}\0", BlobId(m.commitment().unwrap()).hex()).into_bytes();
    let token = Cancellation::default();
    std::thread::scope(|scope| {
        let token_ref = &token;
        let handle = scope.spawn(|| run(&archive, &m, &policy(), token_ref));
        let start = std::time::Instant::now();
        let pid = loop {
            let found = fs::read_dir("/proc").unwrap().filter_map(|e| e.ok()).find_map(|entry| {
                let pid = entry.file_name().to_str()?.parse::<u32>().ok()?;
                let command = fs::read(entry.path().join("cmdline")).ok()?;
                (command == expected_command).then_some(pid)
            });
            if let Some(pid) = found {
                break pid;
            }
            assert!(start.elapsed().as_secs() < 5, "sandbox application was never observed live");
            std::thread::sleep(std::time::Duration::from_millis(10));
        };
        token.cancel();
        let error = handle.join().unwrap().unwrap_err();
        assert_eq!(error.code, ErrorCode::Cancelled);
        assert!(error.execution.is_some());
        let stopped = std::time::Instant::now();
        while fs::read(format!("/proc/{pid}/cmdline")).is_ok_and(|command| command == expected_command) {
            assert!(stopped.elapsed().as_secs() < 2, "cancelled sandbox still executing");
            std::thread::sleep(std::time::Duration::from_millis(10));
        }
    });
}
