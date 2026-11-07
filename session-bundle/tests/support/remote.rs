use super::*;
use myelin_session_store_rocksdb::{RemoteInboxLimits, RocksSessionStore};
use myelin_session_transfer::{SourceBindings, TransferExecutor, TransferImportVerifier};

#[test]
fn remote_target_archive_retains_proofs_and_source_artifacts_after_both_databases_are_deleted() {
    let mut source = Fixture::new(TransferKind::CopyableReceipt);
    let source_cfg = source.source.config().clone();
    let target_cfg = source.target.config().clone();
    let target_state = stored_state(&source.store, target_cfg.session_id);
    let policy = HandoffImportPolicy::new(&source_cfg, &target_cfg, 1_000_000).unwrap();
    let application = TransferImportVerifier::new(
        target_state.policy().clone(),
        vec![stored_state(&source.store, source_cfg.session_id).policy().clone()],
        Arc::new(|_, _, _, _| Err(SessionError::Evidence("no external evidence adapter configured".into()))),
    )
    .unwrap();
    let mut registry = HandoffImportRegistry::default();
    let policy_id = registry
        .register(
            policy,
            source_cfg.clone(),
            target_cfg.clone(),
            myelin_session::RemoteFinalityVerifiers { source: source.verifier.clone(), target: source.verifier.clone() },
            Arc::new(application),
        )
        .unwrap();
    let registry = Arc::new(registry);
    let root = tempfile::tempdir().unwrap();
    let database = root.path().join("target-db");
    let input = root.path().join("inputs");
    fs::create_dir(&input).unwrap();
    let store = Arc::new(
        RocksSessionStore::open(&database)
            .unwrap()
            .with_handoff_imports(registry.clone(), RemoteInboxLimits { max_entries: 8, max_bytes: 8_000_000 })
            .unwrap(),
    );
    let bindings = SourceBindings::from_store(target_state.policy(), store.as_ref()).unwrap();
    let executor = TransferExecutor::new(&target_cfg, target_state, bindings).unwrap();
    let mut target = SessionChain::create(target_cfg.clone(), source.verifier.clone(), executor, store.clone()).unwrap();
    let intent = source.intent(1, 0, 1);
    let request = issue_request(&source.source, vec![intent]);
    let ids = commit(&mut source.source, &source.verifier, &source.validator, request);
    let proof = HandoffProofBundle::export(source.store.as_ref(), ids[0], &target_cfg).unwrap();
    store.import_handoff(&HandoffInboxRecord { policy: policy_id, proof }).unwrap();
    let request = intake_request(&target, ids.clone());
    commit(&mut target, &source.verifier, &source.validator, request);
    #[cfg(target_os = "linux")]
    let expected = target.head().clone();
    let mut plan = plan_for(&[&target_cfg], &source.validator, &input);
    plan.remote_sessions.push(ExportSession {
        session_id: source_cfg.session_id,
        network: [90; 32],
        consensus_toml: consensus_toml(&source.validator),
        execution: plan.sessions[0].execution.clone(),
    });
    let archive = root.path().join("archive");
    let manifest = export_store(store.clone(), &plan, &archive).unwrap();
    assert_eq!(manifest.sessions.len(), 1);
    assert_eq!(manifest.sessions[0].session_id, target_cfg.session_id);
    let imports = &manifest.sessions[0].remote_handoffs;
    assert_eq!(imports.len(), 1);
    let proof_blob = imports[0].proof;
    let artifact = manifest.blobs.iter().find(|b| b.id == proof_blob).unwrap();
    for dependency in [
        imports[0].source.consensus,
        imports[0].source.execution.program,
        imports[0].source.execution.rules,
        imports[0].source.execution.build_inputs,
        imports[0].source.execution.toolchain,
    ] {
        assert!(artifact.dependencies.contains(&dependency));
    }
    let anchors = vec![manifest.sessions[0].proposed_anchor(), imports[0].source.proposed_anchor()];
    let directory = BundleDirectory::open(&archive, test_limits()).unwrap().with_handoff_imports(registry.clone());
    error(directory.verify_inventory(&anchors[..1]), ErrorCode::UnknownTrustAnchor);
    drop(directory);
    drop(target);
    drop(store);
    drop(source);
    fs::remove_dir_all(&database).unwrap();
    error(
        BundleStore::open(Arc::new(BundleDirectory::open(&archive, test_limits()).unwrap()), &anchors),
        ErrorCode::UnknownTrustAnchor,
    );
    let directory = Arc::new(BundleDirectory::open(&archive, test_limits()).unwrap().with_handoff_imports(registry.clone()));
    let (bundle, _) = BundleStore::open(directory, &anchors).unwrap();
    assert!(bundle.load_genesis(source_cfg.session_id).is_err());
    assert!(bundle.load_block(source_cfg.session_id, 0).is_err());
    let cfg = target_cfg.clone();
    let result = bundle.replay_current_process(target_cfg.session_id, move |bytes, store| {
        restore_executor(&cfg, bytes, store).map_err(|e| SessionError::Replay(e.to_string()))
    });
    #[cfg(target_os = "linux")]
    assert_eq!(result.unwrap().unwrap().state_root_after, expected.state_root);
    #[cfg(not(target_os = "linux"))]
    error(result, ErrorCode::UnsupportedPlatform);
    drop(bundle);
    let member = archive.join("blobs").join(proof_blob.hex());
    let saved = fs::read(&member).unwrap();
    fs::remove_file(&member).unwrap();
    error(
        BundleStore::open(Arc::new(BundleDirectory::open(&archive, test_limits()).unwrap().with_handoff_imports(registry)), &anchors),
        ErrorCode::MissingArtifact,
    );
    fs::write(member, saved).unwrap();
}

#[allow(dead_code)]
#[path = "../../../session-authority/examples/support/remote_transfer.rs"]
mod scarce;
#[test]
fn source_retirement_archive_retains_foreign_target_evidence_and_replays_after_both_services_disappear() {
    let d = scarce::Deployment::new(scarce::program_digest().unwrap()).unwrap();
    let root = tempfile::tempdir().unwrap();
    let root = root.path();
    let (mut source, store) = d.source_chain(&root.join("source")).unwrap();
    let id = d.issue(&mut source, 1, 0).unwrap();
    let proof = d.proof(&store, id).unwrap();
    let target = d.target_authority(&root.join("target")).unwrap();
    target.ledger.import_handoff(&proof).unwrap();
    let candidate = target.prepare(&d.target, id, 1).unwrap();
    let terminal = d.terminal_create(&root.join("terminal")).unwrap();
    let certificate = terminal.authorize_consume(&proof, &candidate).unwrap();
    target.authority.commit_remote(&certificate).unwrap();
    let target_record = target.authority.block(0).unwrap();
    drop(target);
    drop(terminal);
    fs::remove_dir_all(root.join("target")).unwrap();
    fs::remove_dir_all(root.join("terminal")).unwrap();
    let mut request =
        scarce::transfer::req(&source, vec![myelin_session_transfer::TransferCommand::Retire { handoff_id: id }]).unwrap();
    request.handoff_actions.push(HandoffAction::RemoteRetire {
        handoff_id: id,
        proof: RemoteConsumptionProof { import_policy: d.import_policy, target_record: target_record.encode().unwrap() },
    });
    let (_, signer) = scarce::transfer::consensus("remote-source", [7; 32]).unwrap();
    scarce::transfer::commit(&mut source, &d.source_finality, &signer, request).unwrap();
    let expected = source.head().clone();
    let input = root.join("inputs");
    fs::create_dir(&input).unwrap();
    let mut plan = plan_for(&[&d.source], &signer, &input);
    let (_, target_signer) = scarce::transfer::consensus("remote-target", [8; 32]).unwrap();
    plan.remote_sessions.push(ExportSession {
        session_id: d.target.session_id,
        network: [90; 32],
        consensus_toml: consensus_toml(&target_signer),
        execution: plan.sessions[0].execution.clone(),
    });
    let archive = root.join("archive");
    let manifest = export_store(store.clone(), &plan, &archive).unwrap();
    assert_eq!(manifest.sessions.len(), 1);
    assert!(manifest.sessions[0].remote_handoffs.is_empty());
    let completions = &manifest.sessions[0].remote_completions;
    assert_eq!(completions.len(), 1);
    assert_eq!(completions[0].handoff_id, id);
    let anchors = vec![manifest.sessions[0].proposed_anchor(), completions[0].target.proposed_anchor()];
    let directory = BundleDirectory::open(&archive, test_limits()).unwrap().with_handoff_imports(d.registry.clone());
    error(directory.verify_inventory(&anchors[..1]), ErrorCode::UnknownTrustAnchor);
    drop(directory);
    drop(source);
    drop(store);
    fs::remove_dir_all(root.join("source")).unwrap();
    let directory = Arc::new(BundleDirectory::open(&archive, test_limits()).unwrap().with_handoff_imports(d.registry.clone()));
    let (bundle, report) = BundleStore::open(directory, &anchors).unwrap();
    assert_eq!(report.stages.externally_recognized_sessions, 2);
    assert!(bundle.load_genesis(d.target.session_id).is_err());
    assert!(bundle.load_block(d.target.session_id, 0).is_err());
    let cfg = d.source.clone();
    let replay = bundle.replay_current_process(d.source.session_id, move |bytes, store| {
        restore_executor(&cfg, bytes, store).map_err(|e| SessionError::Replay(e.to_string()))
    });
    if cfg!(target_os = "linux") {
        assert_eq!(replay.unwrap().unwrap().state_root_after, expected.state_root);
    } else {
        error(replay, ErrorCode::UnsupportedPlatform);
    }
    let retirement = &manifest.blobs.iter().find(|blob| blob.id == completions[0].retirement).unwrap();
    for peer in [
        completions[0].target.consensus,
        completions[0].target.execution.program,
        completions[0].target.execution.rules,
        completions[0].target.execution.build_inputs,
        completions[0].target.execution.toolchain,
    ] {
        assert!(retirement.dependencies.contains(&peer));
    }
}
