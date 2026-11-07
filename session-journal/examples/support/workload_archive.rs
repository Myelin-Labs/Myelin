use super::*;
use myelin_session_bundle::*;

fn plan(root: &Path) -> AnyResult<Plan> {
    let plan: Plan = load(&root.join("plan.json"))?;
    plan.validate()?;
    Ok(plan)
}
fn check_state<S: SessionStore>(chain: &SessionChain<CommandExecutor<Game>, S>, plan: &Plan) -> AnyResult<serde_json::Value> {
    let count = chain.head().input_position;
    if count > u64::from(plan.commands) {
        return Err("recovered input cursor exceeds supplied workload".into());
    }
    let mut model = Game::new(plan.kind);
    for i in 0..count {
        let original = command(plan, chain.config(), i as u32);
        model.apply_rule(i, i, &original.canonical_command)?.map_err(|e| e.to_string())?;
        if !matches!(chain.command_status(&original.identity)?, CommandStatus::Committed(_)) {
            return Err("recovered command identity is missing or conflicting".into());
        }
    }
    let cfg = chain.config().clone();
    let kind = plan.kind;
    let port = command_inspector(cfg, Arc::new(Auth(kind)), move |bytes| Game::restore(kind, bytes));
    let actual = chain.inspect_head(&port, b"state")?.result;
    if model.snapshot()? != actual {
        return Err("recovered game state differs from original input fold".into());
    }
    Ok(serde_json::from_slice(&actual)?)
}
pub(super) fn restore_chain<E: TransitionExecutor, S: SessionStore>(
    store: Arc<S>,
    cfg: &SessionConfig,
    restore: impl FnOnce(&[u8]) -> myelin_session::Result<E>,
) -> AnyResult<(SessionChain<E, S>, serde_json::Value)> {
    if store.load_genesis(cfg.session_id)?.config != *cfg {
        return Err("stored genesis differs from workload/program".into());
    }
    let (verifier, _) = consensus()?;
    let start = Instant::now();
    let mut result = RecoveryAudit::new(cfg.session_id, cfg.application_profile.clone(), verifier.clone(), store.clone())
        .advance(RecoveryBudget::default(), &RecoveryCancellation::default())?;
    let mut pauses = 0;
    let audited = loop {
        match result {
            RecoveryOutcome::Complete(audited) => break audited,
            RecoveryOutcome::Paused(cursor) => {
                pauses += 1;
                result = cursor.resume(RecoveryBudget::default(), &RecoveryCancellation::default())?;
            }
        }
    };
    let audit_ns = ns(start.elapsed());
    let usage = audited.usage();
    let start = Instant::now();
    let executor = restore(audited.state_snapshot())?;
    let chain = SessionChain::recover_audited(*audited, &cfg.application_profile, verifier, executor, store)?;
    Ok((chain, serde_json::json!({"audit_ns":audit_ns,"restore_and_attach_ns":ns(start.elapsed()),"usage":usage,"pauses":pauses})))
}
pub(super) fn replay<E: TransitionExecutor, S: SessionStore>(
    chain: &SessionChain<E, S>,
    expected_cycles: u64,
    restore: impl FnOnce(&[u8]) -> myelin_session::Result<E> + Send + Sync + 'static,
) -> AnyResult<serde_json::Value> {
    let Some(end) = chain.head().finalised_height else {
        return Ok(serde_json::json!({"frames":0}));
    };
    let start = Instant::now();
    let mut outcome =
        chain.start_replay_range(0, end, None, restore).advance(RecoveryBudget::default(), &RecoveryCancellation::default())?;
    let mut pauses = 0;
    loop {
        match outcome {
            ReplayOutcome::Complete { receipt, usage } => {
                if receipt.state_root_after != chain.head().state_root || receipt.total_cycles != expected_cycles {
                    return Err("replay root or measured cycle total differs".into());
                }
                return Ok(serde_json::json!({"elapsed_ns":ns(start.elapsed()),"receipt":receipt,"usage":usage,"pauses":pauses}));
            }
            ReplayOutcome::Paused(cursor) => {
                pauses += 1;
                outcome = cursor.resume(RecoveryBudget::default(), &RecoveryCancellation::default())?;
            }
        }
    }
}
pub(super) fn recover(root: &Path) -> AnyResult<serde_json::Value> {
    let plan = plan(root)?;
    if !root.join("canonical/CURRENT").is_file() {
        return Err("recovery requires an existing store".into());
    }
    let start = Instant::now();
    let store = Arc::new(RocksSessionStore::open(root.join("canonical"))?);
    let open_ns = ns(start.elapsed());
    let program = games::artifact_digest()?;
    let mut reports = vec![];
    for index in plan.first_session..plan.first_session + plan.sessions {
        let cfg = session_config(&plan, index, program);
        let (chain, recovery) = restore_chain(store.clone(), &cfg, |bytes| games::restore(&cfg, plan.kind, bytes))?;
        if !journal_path(root, index).join("CURRENT").is_file() {
            return Err("original journal is missing".into());
        }
        let start = Instant::now();
        let journal = CommandJournal::open(journal_path(root, index), &cfg, JournalLimits::default())?;
        let changed = journal.reconcile(&chain, business_result)?;
        for entry in &changed {
            if matches!(entry.status, EntryStatus::Committed { .. }) {
                journal.acknowledge(entry.original.identity.id)?;
            }
        }
        let journal_recovery_ns = ns(start.elapsed());
        let mut original_entries_checked = 0;
        let mut sequence = 0;
        loop {
            let page = journal.entries(sequence, 256)?;
            if page.is_empty() {
                break;
            }
            for entry in page {
                if entry.original != command(&plan, &cfg, entry.sequence as u32) {
                    return Err("original command bytes changed after restart".into());
                }
                if entry.sequence < chain.head().input_position
                    && !matches!(entry.status, EntryStatus::Committed { outcome: Some(BusinessOutcome::Succeeded(_)), .. })
                {
                    return Err("committed original has no recovered business result".into());
                }
                sequence = entry.sequence + 1;
                original_entries_checked += 1;
            }
        }
        let mut count = 0;
        let retries: std::collections::BTreeSet<_> =
            [0, chain.head().input_position.saturating_sub(1)].into_iter().filter(|i| *i < chain.head().input_position).collect();
        for i in retries {
            let original = command(&plan, &cfg, i as u32);
            let outcome = journal.acknowledge(original.identity.id)?;
            let before = chain.head().clone();
            journal.enqueue(original.clone(), &Auth(plan.kind))?;
            if journal.acknowledge(original.identity.id)? != outcome || chain.head() != &before {
                return Err("recovered original retry changed its outcome or head".into());
            }
            count += 1;
        }
        let state = check_state(&chain, &plan)?;
        let replay = replay(&chain, 0, move |bytes| games::restore(&cfg, plan.kind, bytes))?;
        reports.push(serde_json::json!({"index":index,"head":head_view(chain.head())?,"state":state,"recovery":recovery,
            "journal_recovery_ns":journal_recovery_ns,"reconciled_entries":changed.len(),"original_entries_checked":original_entries_checked,"original_retries_checked":count,
            "replay":replay}));
    }
    Ok(serde_json::json!({"stage":"recovered","open_ns":open_ns,"sessions":reports}))
}
fn drive(task: ArchiveTask, log: &Path) -> AnyResult<(ArchiveOutput, ArchiveUsage)> {
    let mut budget = ArchiveBudget { max_io_bytes: 64 * 1024 * 1024 * 1024, ..ArchiveBudget::default() };
    let mut log = BufWriter::new(File::options().write(true).create_new(true).open(log)?);
    json_line(&mut log, &serde_json::json!({"initial_budget":budget}))?;
    let mut outcome = task.advance(budget.clone(), &RecoveryCancellation::default())?;
    loop {
        match outcome {
            ArchiveOutcome::Complete { output, usage } => {
                json_line(&mut log, &serde_json::json!({"completed":usage}))?;
                log.flush()?;
                return Ok((*output, usage));
            }
            ArchiveOutcome::Paused(cursor) => {
                if let Some(required) = cursor.progress().next_operation {
                    budget.max_steps = budget.max_steps.max(required.steps);
                    budget.max_io_bytes = budget.max_io_bytes.max(required.io_bytes);
                }
                json_line(&mut log, &serde_json::json!({"reason":cursor.reason(),"progress":cursor.progress(),"next_budget":budget}))?;
                log.flush()?;
                outcome = cursor.resume(budget.clone(), &RecoveryCancellation::default())?;
            }
        }
    }
}
pub(super) fn export(root: &Path) -> AnyResult<serde_json::Value> {
    let plan = plan(root)?;
    export_sessions(root, plan.commands, plan.sessions, |index, program| Ok(session_config(&plan, index, program)))
}
pub(super) fn export_sessions(
    root: &Path,
    inputs: u32,
    session_count: u32,
    config: impl Fn(u32, Hash32) -> AnyResult<SessionConfig>,
) -> AnyResult<serde_json::Value> {
    let completed: Vec<SessionResult> = load(&root.join("sessions.json"))?;
    if completed.len() != session_count as usize || completed.is_empty() || !root.join("canonical/CURRENT").is_file() {
        return Err("archive requires a completed workload".into());
    }
    let program = BlobId(games::artifact_digest()?);
    let store = Arc::new(RocksSessionStore::open(root.join("canonical"))?);
    // History edges include every preceding block plus the profile/genesis/head
    // closure. Keep an explicit finite allowance for the selected input trace.
    let limits = BundleLimits { blob_bytes: 512 * 1024 * 1024, dependency_depth: inputs as usize + 8, ..BundleLimits::default() };
    // This measurement capsule retains parameters and executable. It is explicitly
    // not a complete source/toolchain kit or a claim of reproducible compilation.
    let packed = pack_files(
        &PackPlan {
            files: vec![FileSource { name: "measurement-plan.json".into(), path: root.join("plan.json") }],
            chunk_bytes: 65536,
            limits: limits.clone(),
        },
        root.join("parameters"),
    )?;
    let mut artifacts = packed.artifacts;
    artifacts.push(ArtifactSource { id: program, path: std::env::current_exe()?, dependencies: vec![] });
    let (_, signer) = consensus()?;
    let key: String = signer.ckb_public_key().iter().map(|b| format!("{b:02x}")).collect();
    let consensus_toml = format!(
        "kind = \"proof-of-authority\"\n[[proof_of_authority.authorities]]\nid = \"{}\"\npublic_key = \"{key}\"\n",
        signer.validator_id()
    );
    let network = hash(b"myelin:workload-development-network:v1");
    let mut sessions = vec![];
    let mut trust = vec![];
    let mut profiles = std::collections::BTreeSet::new();
    for expected in &completed {
        let cfg = config(expected.index, program.0)?;
        if expected.genesis != BlobId(cfg.commitment()) || head_view(&store.load_head(cfg.session_id)?)? != expected.head {
            return Err("export source differs from measured workload".into());
        }
        let mut bytes = serde_json::to_vec_pretty(&cfg.application_profile)?;
        bytes.push(b'\n');
        let rules = BlobId::of(&bytes);
        if profiles.insert(rules) {
            let path = root.join(format!("profile-{}.json", expected.index));
            save(&path, &cfg.application_profile)?;
            artifacts.push(ArtifactSource { id: rules, path, dependencies: vec![] });
        }
        let execution = ExecutionArtifacts {
            program,
            replayer: program,
            rules,
            build_inputs: packed.root,
            toolchain: packed.root,
            runtime_dependencies: vec![],
            target: format!("{}-{}", std::env::consts::ARCH, std::env::consts::OS),
            runner_abi: "measurement-current-process-v1".into(),
            runtime: None,
        };
        sessions.push(ExportSession {
            session_id: cfg.session_id,
            network,
            consensus_toml: consensus_toml.clone(),
            execution: execution.clone(),
        });
        trust.push(TrustAnchor {
            network,
            session_id: cfg.session_id,
            genesis_commitment: cfg.commitment(),
            profile_commitment: cfg.application_profile.commitment(),
            execution_commitment: execution.commitment(),
        });
    }
    sessions.sort_by_key(|s| s.session_id);
    trust.sort_by_key(|s| s.session_id);
    let export = ExportPlan {
        remote_sessions: vec![],
        roots: sessions.iter().map(|s| s.session_id).collect(),
        sessions,
        artifacts,
        limits: limits.clone(),
    };
    let start = Instant::now();
    let (output, usage) = drive(ArchiveTask::export(store, export, root.join("archive"))?, &root.join("archive-work.jsonl"))?;
    let ArchiveOutput::Exported(manifest) = output else { unreachable!() };
    save(&root.join("trust.json"), &trust)?;
    save(&root.join("archive-limits.json"), &limits)?;
    let report = serde_json::json!({"stage":"archived","elapsed_ns":ns(start.elapsed()),"work":usage,
        "manifest":BlobId(manifest.commitment()?),"blobs":manifest.blobs.len(),"blob_bytes":manifest.blobs.iter().map(|b|b.bytes).sum::<u64>(),
        "dependency_index":{
            "manifest_file_bytes":fs::metadata(root.join("archive/manifest.json"))?.len(),
            "blob_index_json_bytes":serde_json::to_vec(&manifest.blobs)?.len(),
            "session_catalog_json_bytes":serde_json::to_vec(&manifest.sessions)?.len(),
            "dependency_references":manifest.blobs.iter().map(|b|b.dependencies.len()).sum::<usize>(),
            "block_references":manifest.sessions.iter().map(|s|s.blocks.len()).sum::<usize>(),
            "handoff_references":manifest.sessions.iter().map(|s|s.handoffs.len()).sum::<usize>()},
        "build_provenance":"measurement parameters only; complete source/toolchain reproduction not run"});
    save(&root.join("archive-report.json"), &report)?;
    Ok(report)
}
pub(super) fn cold(root: &Path) -> AnyResult<serde_json::Value> {
    let plan = plan(root)?;
    if root.join("canonical").exists()
        || (plan.first_session..plan.first_session + plan.sessions).any(|i| journal_path(root, i).exists())
    {
        return Err("cold drill requires removal of the original canonical store and journals".into());
    }
    let expected: Vec<SessionResult> = load(&root.join("sessions.json"))?;
    let (store, archive_audit_ns, usage) = open_archive(root)?;
    let program = BlobId(games::artifact_digest()?);
    let mut sessions = vec![];
    for session in expected {
        let cfg = session_config(&plan, session.index, program.0);
        check_archive_session(&store, &cfg, &session, program)?;
        let (chain, recovery) = restore_chain(store.clone(), &cfg, |bytes| games::restore(&cfg, plan.kind, bytes))?;
        let state = check_state(&chain, &plan)?;
        if state != session.state {
            return Err("cold state differs from live result".into());
        }
        let replay = replay(&chain, 0, move |bytes| games::restore(&cfg, plan.kind, bytes))?;
        sessions.push(serde_json::json!({"index":session.index,"head":head_view(chain.head())?,"state":state,"recovery":recovery,"replay":replay}));
    }
    Ok(serde_json::json!({"stage":"cold-replayed","archive_audit_ns":archive_audit_ns,"work":usage,"sessions":sessions,
        "original_databases_present":false,"assurance":"trusted current executable; full genesis history audit and application reexecution; no isolated runner/build reproduction claim"}))
}
pub(super) fn open_archive(root: &Path) -> AnyResult<(Arc<BundleStore>, u64, ArchiveUsage)> {
    let trust: Vec<TrustAnchor> = load(&root.join("trust.json"))?;
    let limits: BundleLimits = load(&root.join("archive-limits.json"))?;
    let report: serde_json::Value = load(&root.join("archive-report.json"))?;
    let start = Instant::now();
    let directory = Arc::new(BundleDirectory::open(root.join("archive"), limits)?);
    if serde_json::to_value(BlobId(directory.manifest().commitment()?))? != report["manifest"] {
        return Err("manifest identity changed".into());
    }
    let (output, usage) = drive(ArchiveTask::verify(directory, trust)?, &root.join("cold-work.jsonl"))?;
    let ArchiveOutput::Verified { store, .. } = output else { unreachable!() };
    Ok((store, ns(start.elapsed()), usage))
}
pub(super) fn check_archive_session(
    store: &BundleStore,
    cfg: &SessionConfig,
    expected: &SessionResult,
    program: BlobId,
) -> AnyResult<()> {
    let declared = store.manifest().sessions.iter().find(|s| s.session_id == cfg.session_id).ok_or("missing session")?;
    if declared.execution.replayer != program || head_view(&store.load_head(cfg.session_id)?)? != expected.head {
        return Err("historical program or head changed".into());
    }
    Ok(())
}
