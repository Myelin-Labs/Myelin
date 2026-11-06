use super::*;
use myelin_consensus::{Authority, CommitteeSigner, ConsensusConfig, ProofOfAuthorityConfig};
use myelin_session_runtime::RegisteredFinalityVerifier;
use myelin_session_transfer::TransferCommand;
use std::{collections::BTreeMap, os::unix::fs::MetadataExt, path::PathBuf, time::Instant};

fn phase(start: Instant) -> serde_json::Value {
    let status = fs::read_to_string("/proc/self/status").unwrap();
    let value = |key: &str| {
        status.lines().find_map(|line| line.strip_prefix(key)).unwrap().split_whitespace().next().unwrap().parse::<u64>().unwrap()
            * 1024
    };
    serde_json::json!({"elapsed_ns":start.elapsed().as_nanos(),"rss_after_bytes":value("VmRSS:")})
}

fn disk(path: &Path) -> serde_json::Value {
    let mut pending = vec![path.to_path_buf()];
    let (mut logical, mut allocated, mut files) = (0u64, 0u64, 0usize);
    let mut inodes = BTreeSet::new();
    while let Some(path) = pending.pop() {
        let metadata = fs::symlink_metadata(&path).unwrap();
        if metadata.is_dir() {
            pending.extend(fs::read_dir(&path).unwrap().map(|entry| entry.unwrap().path()));
        } else {
            assert!(metadata.is_file(), "unexpected measurement file type");
            files += 1;
            logical += metadata.len();
            if inodes.insert((metadata.dev(), metadata.ino())) {
                allocated += metadata.blocks() * 512;
            }
        }
    }
    serde_json::json!({"logical_file_bytes":logical,"allocated_file_bytes":allocated,"files":files})
}

fn finish(task: ArchiveTask) -> myelin_session_bundle::Result<(ArchiveOutput, ArchiveUsage, usize)> {
    let budget = || ArchiveBudget { max_io_bytes: 64 * 1024 * 1024 * 1024, ..ArchiveBudget::default() };
    let mut result = task.advance(budget(), &RecoveryCancellation::default())?;
    let mut pauses = 0;
    loop {
        match result {
            ArchiveOutcome::Complete { output, usage } => return Ok((*output, usage, pauses)),
            ArchiveOutcome::Paused(cursor) => {
                pauses += 1;
                assert!(pauses < 1024, "finite fixture must progress");
                result = cursor.resume(budget(), &RecoveryCancellation::default())?;
            }
        }
    }
}

fn replay_all<S: SessionStore + 'static>(
    store: Arc<S>,
    configs: &[SessionConfig],
    verifiers: &BTreeMap<Hash32, Arc<RegisteredFinalityVerifier>>,
    expected: &BTreeMap<Hash32, SessionHead>,
) -> Vec<serde_json::Value> {
    configs
        .iter()
        .map(|cfg| {
            assert_eq!(store.load_genesis(cfg.session_id).unwrap().config, *cfg);
            assert_eq!(cfg.application_profile.program_digest, program_digest());
            let snapshot = store.load_checkpoint(cfg.session_id).unwrap().state_snapshot;
            let executor = restore_executor(cfg, &snapshot, store.as_ref()).unwrap();
            let chain = SessionChain::recover(
                cfg.session_id,
                &cfg.application_profile,
                verifiers[&cfg.session_id].clone(),
                executor,
                store.clone(),
            )
            .unwrap();
            assert_eq!(chain.head(), &expected[&cfg.session_id]);
            let replay_cfg = cfg.clone();
            let replay_store = store.clone();
            let receipt = chain
                .replay_range(0, chain.head().finalised_height.unwrap(), move |bytes| {
                    restore_executor(&replay_cfg, bytes, replay_store.as_ref()).map_err(|e| SessionError::Replay(e.to_string()))
                })
                .unwrap();
            assert_eq!(receipt.total_cycles, 0);
            serde_json::json!({"session":BlobId(cfg.session_id),"genesis":BlobId(cfg.commitment()),
                "authority":BlobId(cfg.consensus_module_commitment),"head":BlobId::of(&chain.head().encode().unwrap()),
                "state_root":BlobId(chain.head().state_root),"receipt":BlobId(receipt.commitment())})
        })
        .collect()
}

#[test]
fn multi_epoch_dependencies_replay_without_the_original_store() {
    let epochs: usize = std::env::var("MYELIN_DEPENDENCY_EPOCHS").map_or(3, |value| value.parse().expect("integer epoch count"));
    assert!((2..=MAX_HANDOFF_OWNER_DEPTH).contains(&epochs));
    let destination = std::env::var_os("MYELIN_DEPENDENCY_EVIDENCE_OUT").map(PathBuf::from);
    let temporary = destination.is_none().then(|| tempfile::tempdir().unwrap());
    let root = destination.unwrap_or_else(|| temporary.as_ref().unwrap().path().to_path_buf());
    if temporary.is_none() {
        fs::create_dir(&root).expect("evidence destination must be new with an existing parent");
    }
    let total_start = Instant::now();
    let start = Instant::now();
    let mut f = Fixture::new(TransferKind::Scarce);
    let mut configs = vec![f.source.config().clone(), f.target.config().clone()];
    let mut verifiers = BTreeMap::from([([3; 32], f.verifier.clone()), ([4; 32], f.verifier.clone())]);
    let mut authorities = BTreeMap::from([([3; 32], consensus_toml(&f.validator)), ([4; 32], consensus_toml(&f.validator))]);
    let mut writer_verifier = f.verifier.clone();
    let mut writer_signer = f.validator.clone();
    let req = issue_request(&f.source, vec![f.intent(1, 0, 25), f.intent(2, 1, 25)]);
    let ids = commit(&mut f.source, &writer_verifier, &writer_signer, req);
    let mut seals = vec![];
    let mut phases = BTreeMap::new();

    for epoch in 1..=epochs {
        let id = 4 + epoch as u8;
        let next_signer = CommitteeSigner::new(format!("dependency-epoch-{epoch}"), [128 + epoch as u8; 32]).unwrap();
        let next_verifier = Arc::new(
            RegisteredFinalityVerifier::new(ConsensusConfig::proof_of_authority(ProofOfAuthorityConfig {
                authorities: vec![Authority { id: next_signer.validator_id().into(), public_key: next_signer.ckb_public_key() }],
            }))
            .unwrap(),
        );
        let state = stored_state(&f.store, f.source.config().session_id);
        let mut next = f.source.config().clone();
        next.session_id = [id; 32];
        next.initial_state_root = f.source.head().state_root;
        next.initial_input_position = f.source.head().input_position + 1;
        next.initial_logical_time = f.source.head().logical_time + 1;
        next.initial_timestamp_ms = 10;
        next.predecessor = Some(PredecessorReference {
            session_id: f.source.config().session_id,
            final_height: f.source.head().next_height().unwrap(),
            final_state_root: f.source.head().state_root,
            application_profile_commitment: f.source.config().application_profile.commitment(),
        });
        let descriptor = next_verifier.descriptor();
        next.consensus_kind = descriptor.consensus_kind;
        next.consensus_config_commitment = descriptor.config_commitment;
        next.consensus_module_commitment = descriptor.commitment();
        next.consensus_wal_schema_hash = descriptor.wal_schema_hash;
        let approval = signer().continue_session(f.source.config(), &state, &next).unwrap();
        let mut req = request(
            &f.source,
            vec![TransferCommand::Continue { target_genesis: next.commitment(), approvals: vec![approval] }],
            vec![],
            vec![],
        );
        req.successor = Some(next.clone());
        commit(&mut f.source, &writer_verifier, &writer_signer, req);
        let seal = f.store.load_block(f.source.config().session_id, f.source.head().finalised_height.unwrap()).unwrap();
        seals.push(BlobId::of(&seal.encode().unwrap()));
        f.source = recover_chain(&f.store, &next_verifier, id);
        configs.push(next);
        authorities.insert([id; 32], consensus_toml(&next_signer));
        verifiers.insert([id; 32], next_verifier.clone());
        writer_verifier = next_verifier;
        writer_signer = next_signer;

        if epoch == 1 {
            let handoff = f.store.load_handoff(ids[1]).unwrap().unwrap().handoff;
            let mut req = request(&f.source, vec![signer().cancel(f.source.config(), &handoff).unwrap()], vec![], vec![]);
            req.handoff_actions.push(HandoffAction::Cancel { handoff_id: ids[1] });
            commit(&mut f.source, &writer_verifier, &writer_signer, req);
            let req = intake_request(&f.target, vec![ids[0]]);
            commit(&mut f.target, &f.verifier, &f.validator, req);
        }
    }
    let mut refund = request(&f.source, vec![TransferCommand::Reclaim { handoff_id: ids[1] }], vec![], vec![]);
    refund.handoff_actions.push(HandoffAction::Reclaim { handoff_id: ids[1] });
    commit(&mut f.source, &writer_verifier, &writer_signer, refund);
    let consumed = f.store.load_handoff(ids[0]).unwrap().unwrap().consumption().unwrap().clone();
    let mut retire = request(&f.source, vec![TransferCommand::Retire { handoff_id: ids[0] }], vec![], vec![]);
    retire.handoff_actions.push(HandoffAction::Retire { handoff_id: ids[0], consumed_at: consumed });
    commit(&mut f.source, &writer_verifier, &writer_signer, retire);
    phases.insert("production", phase(start));

    let final_id = f.source.config().session_id;
    let state = stored_state(&f.store, final_id);
    let account = state.account(ASSET, signer().actor()).unwrap();
    assert_eq!((account.available, account.locked, state.pending().len()), (75, 0, 0));
    let target = stored_state(&f.store, [4; 32]);
    assert_eq!(target.account(ASSET, recipient()).unwrap().available, 25);
    assert_eq!(target.receipts().len(), 1);
    let expected: BTreeMap<_, _> = configs.iter().map(|c| (c.session_id, f.store.load_head(c.session_id).unwrap())).collect();
    let start = Instant::now();
    let live = replay_all(f.store.clone(), &configs, &verifiers, &expected);
    phases.insert("live_audit_and_replay", phase(start));

    let mut export = plan_for(&configs.iter().collect::<Vec<_>>(), &f.validator, &root);
    for session in &mut export.sessions {
        session.consensus_toml = authorities[&session.session_id].clone();
    }
    let archive = root.join("archive");
    let start = Instant::now();
    let (output, export_usage, export_pauses) =
        finish(ArchiveTask::export(f.store.clone(), export, archive.clone()).unwrap()).unwrap();
    let ArchiveOutput::Exported(mut manifest) = output else { panic!("expected export") };
    phases.insert("export", phase(start));
    let trust = anchors(&manifest);
    let manifest_id = BlobId(manifest.commitment().unwrap());
    let db_path = f.directory.path().to_path_buf();
    let Fixture { directory, store, source, target, .. } = f;
    drop(source);
    drop(target);
    drop(store);
    let source_disk = disk(&db_path);
    drop(directory);
    assert!(!db_path.exists());

    let start = Instant::now();
    let (output, verify_usage, verify_pauses) =
        finish(ArchiveTask::verify(Arc::new(BundleDirectory::open(&archive, test_limits()).unwrap()), trust.clone()).unwrap())
            .unwrap();
    let ArchiveOutput::Verified { store, report } = output else { panic!("expected verification") };
    assert_eq!(report.sessions.len(), epochs + 2);
    phases.insert("archive_audit", phase(start));
    let start = Instant::now();
    let replayed = replay_all(store.clone(), &configs, &verifiers, &expected);
    assert_eq!(replayed, live);
    phases.insert("archive_restore_and_replay", phase(start));
    assert!(matches!(store.load_handoff(ids[0]).unwrap().unwrap().lifecycle.terminal, HandoffTerminal::Retired { .. }));
    assert!(matches!(store.load_handoff(ids[1]).unwrap().unwrap().lifecycle.terminal, HandoffTerminal::Reclaimed { .. }));
    drop(store);

    // Removing an intermediate epoch edge must fail even though all blob bytes
    // and each independently signed session remain present.
    let final_session = manifest.sessions.iter().find(|s| s.session_id == final_id).unwrap();
    let refund = final_session.blocks[0];
    let saved = manifest.encode().unwrap();
    let deps = &mut manifest.blobs.iter_mut().find(|b| b.id == refund).unwrap().dependencies;
    assert!(seals.iter().all(|seal| deps.contains(seal)));
    deps.retain(|id| *id != seals[epochs / 2]);
    fs::write(archive.join("manifest.json"), manifest.encode().unwrap()).unwrap();
    error(
        finish(ArchiveTask::verify(Arc::new(BundleDirectory::open(&archive, test_limits()).unwrap()), trust.clone()).unwrap()),
        ErrorCode::MissingDependency,
    );
    fs::write(archive.join("manifest.json"), &saved).unwrap();
    fs::write(root.join("trust.json"), serde_json::to_vec_pretty(&trust).unwrap()).unwrap();
    let report = serde_json::json!({"schema":"myelin-multi-epoch-measurement-v1","status":"passed","authority_rotations":epochs,
        "sessions":configs.len(),"blocks":expected.values().map(|h|h.next_height().unwrap()).sum::<u64>(),
        "program":BlobId(program_digest()),"manifest":manifest_id,"phases":phases,"total":phase(total_start),
        "source_disk_after_close":source_disk,"archive_disk":disk(&archive),"inventory":manifest.validate_structure(&test_limits()).unwrap(),
        "export_work":export_usage,"export_pauses":export_pauses,"verify_work":verify_usage,"verify_pauses":verify_pauses,
        "replayed":replayed,"active_source_available":75,"target_available":25,"locked":0,
        "consumed_and_retired":BlobId(ids[0]),"cancelled_and_refunded":BlobId(ids[1]),
        "original_database_present":false,"missing_intermediate_epoch_rejected":true,
        "scope":"trusted compiled host replay in one process after original DB deletion; synthetic build packages; no OS-cache purge, sandbox, complete-build or deployment RPO/RTO claim",
        "rss_method":"Linux /proc/self/status VmRSS sampled after each phase; samples may miss peaks; record lifetime peak RSS in the external runner"});
    fs::write(root.join("report.json"), serde_json::to_vec_pretty(&report).unwrap()).unwrap();
}
