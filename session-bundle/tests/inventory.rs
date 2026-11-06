use myelin_session::SessionGenesis;
use myelin_session_bundle::*;
use std::{collections::BTreeMap, fs, path::Path};

#[path = "support/packages.rs"]
mod packages;

fn unhex(text: &str) -> Vec<u8> {
    (0..text.len()).step_by(2).map(|i| u8::from_str_radix(&text[i..i + 2], 16).unwrap()).collect()
}

#[allow(dead_code)]
#[path = "../../session-transfer/tests/support/mod.rs"]
mod support;

// Current genesis with synthetic inventory members; no finality/replay claim.
fn fixture() -> (BundleManifest, BTreeMap<BlobId, Vec<u8>>) {
    let initial = support::state(support::policy(myelin_session_transfer::TransferKind::Scarce));
    let mut config = support::config(&support::consensus().0, 3, &initial);
    config.application_profile.program_digest = BlobId::of(b"synthetic historical executable; never executed").0;
    let bytes = SessionGenesis { config: config.clone(), state_snapshot: initial.encode().unwrap() }.encode().unwrap();
    let mut data = BTreeMap::new();
    let mut blobs = vec![];
    let mut add = |bytes: Vec<u8>, roles: Vec<BlobKind>| {
        let id = BlobId::of(&bytes);
        blobs.push(Blob { id, bytes: bytes.len() as u64, roles, dependencies: vec![] });
        data.insert(id, bytes);
        id
    };
    let genesis = add(bytes, vec![BlobKind::Genesis]);
    let consensus = add(b"synthetic finality config; not verified".to_vec(), vec![BlobKind::Consensus]);
    let head = add(b"synthetic head; not audited".to_vec(), vec![BlobKind::Head]);
    let checkpoint = add(b"synthetic checkpoint; not replayed".to_vec(), vec![BlobKind::Checkpoint]);
    let program = add(b"synthetic historical executable; never executed".to_vec(), vec![BlobKind::Program, BlobKind::Replayer]);
    let rules = add(b"synthetic rules".to_vec(), vec![BlobKind::Rules]);
    let mut package = |name: &str, role| {
        add(
            FilePackage {
                format: "myelin-file-package-v1".into(),
                files: vec![PackageFile { name: name.into(), bytes: 0, digest: BlobId::of(b""), chunks: vec![] }],
            }
            .encode()
            .unwrap(),
            vec![role],
        )
    };
    let build_inputs = package("synthetic-inputs.txt", BlobKind::BuildInputs);
    let toolchain = package("synthetic-toolchain.txt", BlobKind::Toolchain);
    let execution = ExecutionArtifacts {
        runtime: None,

        program,
        replayer: program,
        rules,
        build_inputs,
        toolchain,
        runtime_dependencies: vec![],
        target: "fixture-only".into(),
        runner_abi: "not-executable".into(),
    };
    let session = SessionInventory {
        network: [91; 32],
        session_id: config.session_id,
        genesis_commitment: config.commitment(),
        profile: config.application_profile,
        genesis,
        consensus,
        head,
        checkpoint,
        blocks: vec![],
        handoffs: vec![],
        evidence: vec![],
        execution,
    };
    let program_node = blobs.iter_mut().find(|b| b.id == program).unwrap();
    program_node.dependencies = vec![rules, build_inputs, toolchain];
    program_node.dependencies.sort();
    blobs.sort_by_key(|b| b.id);
    (
        BundleManifest {
            format: FORMAT.into(),
            myelin_version: "0.31.0".into(),
            record_versions: vec![myelin_session::RECORD_FORMAT_VERSION],
            roots: vec![session.session_id],
            blobs,
            sessions: vec![session],
        },
        data,
    )
}
fn write(root: &Path, manifest: &BundleManifest, data: &BTreeMap<BlobId, Vec<u8>>) {
    fs::create_dir_all(root.join("blobs")).unwrap();
    fs::write(root.join("manifest.json"), manifest.encode().unwrap()).unwrap();
    for (id, bytes) in data {
        fs::write(root.join("blobs").join(id.hex()), bytes).unwrap();
    }
}
fn error<T>(result: Result<T>, code: ErrorCode) {
    assert_eq!(result.err().expect("must reject").code, code);
}

#[test]
fn inventory_requires_external_recognition_without_claiming_execution() {
    let (manifest, data) = fixture();
    let root = tempfile::tempdir().unwrap();
    write(root.path(), &manifest, &data);
    let bundle = BundleDirectory::open(root.path(), BundleLimits::default()).unwrap();
    error(bundle.verify_inventory(&[]), ErrorCode::UnknownTrustAnchor);
    let report = bundle.verify_inventory(&[manifest.sessions[0].proposed_anchor()]).unwrap();
    assert_eq!(report.inventory.blobs, 8);
    assert_eq!(report.inventory.dependency_depth, 2);
    let json = serde_json::to_value(report).unwrap();
    for key in ["host_replay", "ckb_vm", "finality", "data_availability", "ckb_inclusion", "court_adjudication"] {
        assert_eq!(json[key], "not-run");
    }
    assert_eq!(bundle.read_blob(manifest.sessions[0].execution.rules, 100).unwrap(), b"synthetic rules");
}

#[test]
fn tampering_missing_artifacts_and_missing_descriptors_are_distinct() {
    let (mut manifest, data) = fixture();
    let root = tempfile::tempdir().unwrap();
    write(root.path(), &manifest, &data);
    let bundle = BundleDirectory::open(root.path(), BundleLimits::default()).unwrap();
    let anchor = manifest.sessions[0].proposed_anchor();
    let rules = manifest.sessions[0].execution.rules;
    let path = root.path().join("blobs").join(rules.hex());
    fs::write(&path, vec![0; data[&rules].len()]).unwrap();
    error(bundle.verify_inventory(std::slice::from_ref(&anchor)), ErrorCode::Integrity);
    error(bundle.read_blob(rules, 100), ErrorCode::Integrity);
    fs::remove_file(path).unwrap();
    error(bundle.verify_inventory(&[anchor]), ErrorCode::MissingArtifact);
    manifest.blobs.retain(|b| b.id != rules);
    error(manifest.validate_structure(&BundleLimits::default()), ErrorCode::MissingDependency);
}

#[test]
fn changed_network_profile_authority_or_program_cannot_reuse_recognition() {
    let (manifest, data) = fixture();
    let anchor = manifest.sessions[0].proposed_anchor();
    for field in 0..4 {
        let mut changed = manifest.clone();
        let session = &mut changed.sessions[0];
        match field {
            0 => session.network[0] ^= 1,
            1 => session.profile.input_schema_hash[0] ^= 1,
            2 => session.genesis_commitment[0] ^= 1,
            _ => session.execution.runner_abi.push_str("-new"),
        }
        error(changed.recognize(std::slice::from_ref(&anchor)), ErrorCode::UnknownTrustAnchor);
    }
    // Merely copying the public recognized fields cannot bless another genesis.
    let root = tempfile::tempdir().unwrap();
    let mut changed = manifest.clone();
    let mut values = data;
    let old = changed.sessions[0].genesis;
    let mut actual = SessionGenesis::decode(&values.remove(&old).unwrap()).unwrap();
    actual.config.session_id[0] ^= 1;
    let bytes = actual.encode().unwrap();
    let new = BlobId::of(&bytes);
    let descriptor = changed.blobs.iter_mut().find(|b| b.id == old).unwrap();
    descriptor.id = new;
    descriptor.bytes = bytes.len() as u64;
    changed.blobs.sort_by_key(|b| b.id);
    changed.sessions[0].genesis = new;
    values.insert(new, bytes);
    write(root.path(), &changed, &values);
    let bundle = BundleDirectory::open(root.path(), BundleLimits::default()).unwrap();
    error(bundle.verify_inventory(&[anchor]), ErrorCode::Integrity);
}

#[test]
fn canonical_format_and_all_import_budgets_reject_before_any_execution() {
    let (manifest, _) = fixture();
    let limits = BundleLimits::default();
    let mut bytes = manifest.encode().unwrap();
    bytes.push(b'\n');
    error(BundleManifest::decode(&bytes, &limits), ErrorCode::Format);
    let mut json = serde_json::to_value(&manifest).unwrap();
    json["verified"] = serde_json::json!(true);
    error(BundleManifest::decode(&serde_json::to_vec(&json).unwrap(), &limits), ErrorCode::Format);
    for name in ["../secret", "/etc/passwd", "0", &"A".repeat(64)] {
        assert!(serde_json::from_value::<BlobId>(serde_json::json!(name)).is_err());
    }
    for choice in 0..6 {
        let mut cap = limits.clone();
        match choice {
            0 => cap.manifest_bytes = 1,
            1 => cap.blob_bytes = 1,
            2 => cap.total_bytes = 1,
            3 => {
                cap.nodes = 1;
                cap.blocks = 1;
            }
            4 => cap.edges = 1,
            _ => cap.dependency_depth = 1,
        }
        error(manifest.validate_structure(&cap), ErrorCode::Limit);
    }
    let mut changed = manifest.clone();
    changed.sessions[0].profile.program_digest[0] ^= 1;
    error(changed.validate_structure(&limits), ErrorCode::Integrity);
    let mut changed = manifest;
    changed.blobs.push(changed.blobs[0].clone());
    error(changed.validate_structure(&limits), ErrorCode::Format);
}

#[test]
fn cross_session_inventories_require_each_anchor_and_dependency_graph_is_acyclic() {
    let (mut manifest, _) = fixture();
    let mut other = manifest.sessions[0].clone();
    other.session_id = [250; 32];
    other.network = [251; 32];
    let first = manifest.sessions[0].proposed_anchor();
    let second = other.proposed_anchor();
    manifest.sessions.push(other);
    manifest.validate_structure(&BundleLimits::default()).unwrap();
    error(manifest.recognize(std::slice::from_ref(&first)), ErrorCode::UnknownTrustAnchor);
    manifest.recognize(&[first, second]).unwrap();
    let program = manifest.sessions[0].execution.program;
    let rules = manifest.sessions[0].execution.rules;
    manifest.blobs.iter_mut().find(|b| b.id == rules).unwrap().dependencies = vec![program];
    error(manifest.validate_structure(&BundleLimits::default()), ErrorCode::DependencyCycle);
    manifest.blobs.iter_mut().find(|b| b.id == rules).unwrap().dependencies = vec![BlobId([123; 32])];
    error(manifest.validate_structure(&BundleLimits::default()), ErrorCode::MissingDependency);
}

#[cfg(unix)]
#[test]
fn pinned_directory_handles_reject_symlinks_and_survive_path_replacement() {
    use std::os::unix::fs::symlink;
    let (manifest, data) = fixture();
    let base = tempfile::tempdir().unwrap();
    let root = base.path().join("bundle");
    write(&root, &manifest, &data);
    let bundle = BundleDirectory::open(&root, BundleLimits::default()).unwrap();
    let rules = manifest.sessions[0].execution.rules;
    fs::rename(&root, base.path().join("moved")).unwrap();
    fs::create_dir(&root).unwrap();
    assert_eq!(bundle.read_blob(rules, 100).unwrap(), data[&rules]);
    let blob = base.path().join("moved/blobs").join(rules.hex());
    fs::remove_file(&blob).unwrap();
    let outside = base.path().join("outside");
    fs::write(&outside, &data[&rules]).unwrap();
    symlink(&outside, &blob).unwrap();
    error(bundle.read_blob(rules, 100), ErrorCode::UnsafePath);
    symlink(base.path().join("moved"), base.path().join("linked")).unwrap();
    error(BundleDirectory::open(base.path().join("linked"), BundleLimits::default()), ErrorCode::UnsafePath);
}

#[test]
fn cli_inventory_checks_report_machine_readable_errors_and_never_claim_replay() {
    let (manifest, data) = fixture();
    let root = tempfile::tempdir().unwrap();
    write(root.path(), &manifest, &data);
    let trust = root.path().join("external-trust.json");
    fs::write(&trust, serde_json::to_vec(&vec![manifest.sessions[0].proposed_anchor()]).unwrap()).unwrap();
    let run = |operation: &str, path: Option<&Path>| {
        let mut command = std::process::Command::new(env!("CARGO_BIN_EXE_myelin-session-bundle"));
        command.arg(operation).arg(root.path());
        if let Some(path) = path {
            command.arg(path);
        }
        command.output().unwrap()
    };
    let inspection = run("inspect", None);
    assert!(inspection.status.success());
    assert_eq!(serde_json::from_slice::<serde_json::Value>(&inspection.stdout).unwrap()["stage"], "unverified-inventory");
    let verified = run("verify-inventory", Some(&trust));
    assert!(verified.status.success(), "{}", String::from_utf8_lossy(&verified.stderr));
    let report: serde_json::Value = serde_json::from_slice(&verified.stdout).unwrap();
    assert_eq!(report["host_replay"], "not-run");
    assert_eq!(report["finality"], "not-run");
    fs::write(&trust, b"[]").unwrap();
    let rejected = run("verify-inventory", Some(&trust));
    assert_eq!(rejected.status.code(), Some(2));
    assert_eq!(serde_json::from_slice::<serde_json::Value>(&rejected.stderr).unwrap()["code"], "unknown-trust-anchor");
    assert_eq!(run("export", None).status.code(), Some(2));
}
