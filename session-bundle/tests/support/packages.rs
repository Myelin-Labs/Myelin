use super::*;
use std::process::Command;

fn package_file(name: &str, parts: &[&[u8]]) -> PackageFile {
    let bytes: Vec<_> = parts.iter().flat_map(|p| p.iter().copied()).collect();
    PackageFile {
        name: name.into(),
        bytes: bytes.len() as u64,
        digest: BlobId::of(&bytes),
        chunks: parts.iter().map(|p| FileChunk { id: BlobId::of(p), bytes: p.len() as u64 }).collect(),
    }
}
fn simple() -> FilePackage {
    FilePackage { format: "myelin-file-package-v1".into(), files: vec![package_file("source.tar", &[b"abc", b"def", b"abc"])] }
}
fn attach(package: &FilePackage) -> (BundleManifest, BTreeMap<BlobId, Vec<u8>>) {
    attach_encoded(package, package.encode().unwrap())
}
fn attach_encoded(package: &FilePackage, bytes: Vec<u8>) -> (BundleManifest, BTreeMap<BlobId, Vec<u8>>) {
    let (mut manifest, mut data) = fixture();
    let old = manifest.sessions[0].execution.build_inputs;
    manifest.blobs.retain(|b| b.id != old);
    data.remove(&old);
    let root = BlobId::of(&bytes);
    let children: std::collections::BTreeSet<_> = package.files.iter().flat_map(|f| f.chunks.iter().map(|c| c.id)).collect();
    data.insert(root, bytes.clone());
    manifest.blobs.push(Blob {
        id: root,
        bytes: bytes.len() as u64,
        roles: vec![BlobKind::BuildInputs],
        dependencies: children.into_iter().collect(),
    });
    manifest.sessions[0].execution.build_inputs = root;
    for blob in &mut manifest.blobs {
        for dep in &mut blob.dependencies {
            if *dep == old {
                *dep = root;
            }
        }
        blob.dependencies.sort();
    }
    for bytes in [b"abc".as_slice(), b"def".as_slice()] {
        let id = BlobId::of(bytes);
        data.insert(id, bytes.to_vec());
        manifest.blobs.push(Blob { id, bytes: bytes.len() as u64, roles: vec![BlobKind::BuildInputs], dependencies: vec![] });
    }
    manifest.blobs.sort_by_key(|b| b.id);
    (manifest, data)
}

#[test]
fn typed_packages_reconstruct_ordered_repeated_chunks_without_claiming_build_or_replay() {
    let package = simple();
    let vector: serde_json::Value = serde_json::from_str(include_str!("../../../fixtures/protocol/file-package-v1.json")).unwrap();
    assert_eq!(package.encode().unwrap(), unhex(vector["canonical_package_hex"].as_str().unwrap()));
    assert_eq!(BlobId::of(&package.encode().unwrap()).hex(), vector["package_digest"]);
    let (manifest, data) = attach(&package);
    let dir = tempfile::tempdir().unwrap();
    let archive = dir.path().join("archive");
    write(&archive, &manifest, &data);
    let bundle = std::sync::Arc::new(BundleDirectory::open(&archive, BundleLimits::default()).unwrap());
    let anchors = [manifest.sessions[0].proposed_anchor()];
    let inventory = bundle.verify_inventory(&anchors).unwrap();
    assert_eq!(inventory.file_packages.len(), 2);
    let report = inventory.file_packages.iter().find(|p| p.root == manifest.sessions[0].execution.build_inputs).unwrap();
    assert_eq!(report.expanded_bytes, 9);
    assert_eq!(report.chunk_references, 3);
    assert_eq!(serde_json::to_value(inventory).unwrap()["host_replay"], "not-run");
    let target = dir.path().join("restored");
    let root = manifest.sessions[0].execution.build_inputs;
    error(bundle.assemble_file_package(&[], root, &target), ErrorCode::UnknownTrustAnchor);
    assert!(!target.exists());
    let budget = ArchiveBudget { max_steps: 1, ..ArchiveBudget::default() };
    let token = myelin_session::RecoveryCancellation::default();
    let mut outcome = ArchiveTask::assemble(bundle.clone(), anchors.to_vec(), root, target.clone())
        .unwrap()
        .advance(budget.clone(), &token)
        .unwrap();
    let mut pauses = 0;
    loop {
        match outcome {
            ArchiveOutcome::Paused(cursor) => {
                pauses += 1;
                assert!(pauses < 1000);
                outcome = cursor.resume(budget.clone(), &token).unwrap();
            }
            ArchiveOutcome::Complete { output, .. } => {
                let ArchiveOutput::Assembled(report) = *output else {
                    panic!("wrong operation");
                };
                assert_eq!(report.expanded_bytes, 9);
                break;
            }
        }
    }
    assert!(pauses > 1);
    assert_eq!(fs::read(target.join("source.tar")).unwrap(), b"abcdefabc");
    fs::write(target.join("owned"), b"keep").unwrap();
    error(bundle.assemble_file_package(&anchors, root, &target), ErrorCode::Io);
    assert_eq!(fs::read(target.join("owned")).unwrap(), b"keep");
    error(
        bundle.assemble_file_package(&anchors, manifest.sessions[0].execution.rules, dir.path().join("not-a-package")),
        ErrorCode::Format,
    );
    assert!(!dir.path().join("not-a-package").exists());
}

#[test]
fn typed_packages_detect_missing_chunks_even_when_the_manifest_graph_was_rewritten() {
    let (mut manifest, mut data) = attach(&simple());
    let chunk = BlobId::of(b"def");
    manifest.blobs.retain(|b| b.id != chunk);
    data.remove(&chunk);
    for blob in &mut manifest.blobs {
        blob.dependencies.retain(|id| *id != chunk);
    }
    let dir = tempfile::tempdir().unwrap();
    write(dir.path(), &manifest, &data);
    let bundle = BundleDirectory::open(dir.path(), BundleLimits::default()).unwrap();
    error(bundle.verify_inventory(&[manifest.sessions[0].proposed_anchor()]), ErrorCode::MissingDependency);
}

#[test]
fn invalid_packages_or_chunks_fail_before_output_creation() {
    for choice in 0..5 {
        let mut package = simple();
        match choice {
            1 => package.files[0].chunks.swap(0, 1),
            2 => package.files[0].digest = BlobId::of(b"wrong whole file"),
            3 => package.files[0].chunks[0].bytes += 1,
            _ => {}
        }
        let (manifest, mut data) = if choice == 4 {
            // Even independently recognized opaque roots cannot bypass package checks.
            attach_encoded(&package, b"opaque build metadata".to_vec())
        } else {
            attach(&package)
        };
        if choice == 0 {
            data.insert(BlobId::of(b"def"), b"bad".to_vec());
        }
        let dir = tempfile::tempdir().unwrap();
        let archive = dir.path().join("archive");
        write(&archive, &manifest, &data);
        let bundle = BundleDirectory::open(&archive, BundleLimits::default()).unwrap();
        let target = dir.path().join("restored");
        error(
            bundle.assemble_file_package(
                &[manifest.sessions[0].proposed_anchor()],
                manifest.sessions[0].execution.build_inputs,
                &target,
            ),
            if choice == 4 { ErrorCode::Format } else { ErrorCode::Integrity },
        );
        assert!(!target.exists());
    }
}

#[test]
fn package_paths_canonical_encoding_and_expansion_budgets_are_enforced() {
    for name in ["../escape", "/etc/passwd", "a/../b", "a//b", "a\\b", "C:/file", "a/.", "a.", "CON", "com1.txt", ""] {
        let mut package = simple();
        package.files[0].name = name.into();
        error(package.validate(&BundleLimits::default()), ErrorCode::UnsafePath);
    }
    for pair in [["a", "a/b"], ["File", "file"], ["A/b", "a"], ["A/b", "a/c"]] {
        let mut package = simple();
        package.files = pair.iter().map(|name| package_file(name, &[b"abc"])).collect();
        package.files.sort_by(|a, b| a.name.cmp(&b.name));
        error(package.validate(&BundleLimits::default()), ErrorCode::UnsafePath);
    }
    let package = simple();
    let mut bytes = package.encode().unwrap();
    bytes.push(b'\n');
    error(FilePackage::decode(&bytes, &BundleLimits::default()), ErrorCode::Format);
    let mut unknown = package.clone();
    unknown.format.push_str("-unknown");
    error(FilePackage::decode(&unknown.encode().unwrap(), &BundleLimits::default()), ErrorCode::Format);
    for limits in [
        BundleLimits { total_bytes: 8, ..BundleLimits::default() },
        BundleLimits { edges: 2, ..BundleLimits::default() },
        BundleLimits { manifest_bytes: 8, ..BundleLimits::default() },
    ] {
        error(package.validate(&limits), ErrorCode::Limit);
    }
    let mut empty = simple();
    empty.files = vec![package_file("empty", &[])];
    empty.validate(&BundleLimits::default()).unwrap();
    empty.files[0].digest = BlobId::of(b"not empty");
    error(empty.validate(&BundleLimits::default()), ErrorCode::Integrity);
}

#[test]
fn pack_cli_captures_large_files_empty_files_and_relative_source_paths() {
    let dir = tempfile::tempdir().unwrap();
    let original: Vec<_> = (0..200_005).map(|i| (i % 251) as u8).collect();
    fs::write(dir.path().join("large"), &original).unwrap();
    fs::write(dir.path().join("empty"), b"").unwrap();
    let plan = PackPlan {
        files: vec![
            FileSource { name: "nested/source.tar".into(), path: "large".into() },
            FileSource { name: "empty".into(), path: "empty".into() },
        ],
        chunk_bytes: 65_536,
        limits: BundleLimits::default(),
    };
    let plan_path = dir.path().join("plan.json");
    fs::write(&plan_path, serde_json::to_vec(&plan).unwrap()).unwrap();
    let output = Command::new(env!("CARGO_BIN_EXE_myelin-session-bundle"))
        .args(["pack-files"])
        .arg(&plan_path)
        .arg(dir.path().join("packed"))
        .current_dir("/")
        .output()
        .unwrap();
    assert!(output.status.success(), "{}", String::from_utf8_lossy(&output.stderr));
    let packed: PackedFiles = serde_json::from_slice(&output.stdout).unwrap();
    let root = packed.artifacts.iter().find(|s| s.id == packed.root).unwrap();
    let package = FilePackage::decode(&fs::read(&root.path).unwrap(), &plan.limits).unwrap();
    assert_eq!(package.files[0].bytes, 0);
    assert_eq!(package.files[1].chunks.len(), 4);
    assert_eq!(package.files[1].digest, BlobId::of(&original));
    let mut restored = vec![];
    for chunk in &package.files[1].chunks {
        let source = packed.artifacts.iter().find(|s| s.id == chunk.id).unwrap();
        let bytes = fs::read(&source.path).unwrap();
        assert_eq!(BlobId::of(&bytes), chunk.id);
        restored.extend(bytes);
    }
    assert_eq!(restored, original);
    let mut bad = plan;
    bad.files[0].name = "../escape".into();
    let target = dir.path().join("invalid");
    error(pack_files(&bad, &target), ErrorCode::UnsafePath);
    assert!(!target.exists());
}

#[test]
fn assemble_cli_requires_external_recognition_and_writes_only_checked_package_files() {
    let (manifest, data) = attach(&simple());
    let dir = tempfile::tempdir().unwrap();
    let archive = dir.path().join("archive");
    write(&archive, &manifest, &data);
    let trust = dir.path().join("trust.json");
    fs::write(&trust, serde_json::to_vec(&[manifest.sessions[0].proposed_anchor()]).unwrap()).unwrap();
    let target = dir.path().join("restored");
    let run = || {
        Command::new(env!("CARGO_BIN_EXE_myelin-session-bundle"))
            .arg("assemble-files")
            .arg(&archive)
            .arg(&trust)
            .arg(manifest.sessions[0].execution.build_inputs.hex())
            .arg(&target)
            .output()
            .unwrap()
    };
    let output = run();
    assert!(output.status.success(), "{}", String::from_utf8_lossy(&output.stderr));
    assert_eq!(fs::read(target.join("source.tar")).unwrap(), b"abcdefabc");
    fs::remove_dir_all(&target).unwrap();
    fs::write(&trust, b"[]").unwrap();
    let output = run();
    assert_eq!(output.status.code(), Some(2));
    assert_eq!(serde_json::from_slice::<serde_json::Value>(&output.stderr).unwrap()["code"], "unknown-trust-anchor");
    assert!(!target.exists());
}
