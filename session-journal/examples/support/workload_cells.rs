//! A bounded Cell rollover workload. The script checks an input since field;
//! this is VM/state/storage measurement, not game logic or CKB settlement.
use super::*;
use myelin_exec::{
    ckb_cell_data_hash, scripts::TIMELOCK_ABSOLUTE_SCRIPT, serialize_transaction_molecule, CellDep, CellInput, CellOutput, CellTx,
    DepType, OutPoint, ResolvedCell, Script, SimpleDataProvider, TransactionScriptVerifier, VmSemantics,
};
use myelin_session_bundle::BlobId;
use myelin_state::{CellEntry, CellStateTree, ResolvedStateInput};

const SINCE: u64 = (1 << 62) | 1_735_689_600;
const CAPACITY: u64 = 100_000_000_000;
const MAX_TX_CYCLES: u64 = 100_000;
const DEP: OutPoint = OutPoint { tx_hash: [0x72; 32], index: 0 };

#[derive(Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub(super) struct CellPlan {
    sessions: u32,
    first_session: u32,
    transactions: u32,
    batch: usize,
    seed: u64,
    trigger: Trigger,
    arrival_us: u64,
    window_us: u64,
}
impl CellPlan {
    fn validate(&self) -> AnyResult<()> {
        if !(1..=64).contains(&self.sessions)
            || self.first_session.checked_add(self.sessions).is_none()
            || !(2..=4096).contains(&self.transactions)
            || !(1..=64).contains(&self.batch)
            || self.arrival_us > 1_000_000
            || !(1..=1_000_000).contains(&self.window_us)
        {
            return Err("invalid bounded Cell workload plan".into());
        }
        Ok(())
    }
}
fn output() -> CellOutput {
    CellOutput { capacity: CAPACITY, lock: Script::new(ckb_cell_data_hash(TIMELOCK_ABSOLUTE_SCRIPT), 4, vec![]), type_: None }
}
fn initial(plan: &CellPlan, index: u32) -> CellTransitionExecutor {
    let mut state = CellStateTree::new();
    state.insert(initial_outpoint(plan, index), CellEntry::from_output(&output(), &0u64.to_le_bytes(), 0, false).unwrap());
    CellTransitionExecutor::new(state, verifier())
}
fn initial_outpoint(plan: &CellPlan, index: u32) -> OutPoint {
    OutPoint::new(hash(format!("myelin:workload:cells:{}:{index}", plan.seed).as_bytes()), 0)
}
fn verifier() -> Arc<dyn CellTxScriptVerifier> {
    Arc::new(|tx: &CellTx, inputs: &[ResolvedStateInput]| {
        let mut provider = SimpleDataProvider::new();
        provider.add_cell(
            DEP.tx_hash,
            DEP.index,
            ResolvedCell { cell_output: output(), data: Some(TIMELOCK_ABSOLUTE_SCRIPT.to_vec()) },
        );
        for input in inputs {
            provider.add_cell(
                input.outpoint.tx_hash,
                input.outpoint.index,
                ResolvedCell {
                    cell_output: CellOutput {
                        capacity: input.cell.capacity,
                        lock: input.cell.lock_script.clone().ok_or("missing full input lock")?,
                        type_: input.cell.type_script.clone(),
                    },
                    data: Some(input.cell.data.clone().ok_or("missing full input data")?),
                },
            );
        }
        TransactionScriptVerifier::new(Arc::new(tx.clone()), Arc::new(provider))
            .with_semantics(VmSemantics::CkbStrict)
            .with_max_cycles(MAX_TX_CYCLES)
            .verify_with_cycles()
            .map_err(|error| format!("strict VM: {error}"))
    })
}
fn restore(bytes: &[u8]) -> myelin_session::Result<CellTransitionExecutor> {
    CellTransitionExecutor::from_snapshot(bytes, verifier())
}
fn config(plan: &CellPlan, index: u32, program: Hash32) -> AnyResult<SessionConfig> {
    let (consensus, _) = consensus()?;
    let descriptor = consensus.descriptor();
    Ok(SessionConfig {
        session_id: initial_outpoint(plan, index).tx_hash,
        consensus_kind: descriptor.consensus_kind,
        consensus_config_commitment: descriptor.config_commitment,
        consensus_module_commitment: descriptor.commitment(),
        consensus_wal_schema_hash: descriptor.wal_schema_hash,
        application_profile: ApplicationProfile {
            command_identity_required: false,
            application_id: "measurement/ckb-strict-timelock-rollover".into(),
            program_digest: program,
            input_schema_hash: hash(b"CellTx:v0;one-input-one-output;u64-sequence;timelock-absolute-data2"),
            state_codec_hash: hash(b"myelin-full-cell-snapshot-current"),
            logical_time_policy_hash: hash(b"one-logical-step-per-transaction;since-field-fixture-only"),
            entropy_policy_hash: hash(b"no-randomness;seed-identifies-initial-outpoint"),
            vm: ApplicationVmProfile { spawn_ipc_required: false },
            resources: ApplicationResourceEnvelope {
                max_frame_inputs: 64,
                max_frame_input_bytes: 1_000_000,
                max_frame_cycles: 64 * MAX_TX_CYCLES,
                max_logical_time_span: 64,
                max_inspect_query_bytes: 32,
                max_inspect_result_bytes: 4096,
            },
            court_profile_hash: hash(b"full-genesis-cell-reexecution:CKB-strict;immutable-embedded-timelock-dep"),
            handoff_policy_hash: hash(b"no-handoffs"),
        },
        initial_state_root: initial(plan, index).state_root(),
        initial_input_position: 0,
        initial_logical_time: 0,
        initial_timestamp_ms: 0,
        predecessor: None,
        max_block_transactions: 64,
        max_block_bytes: 1_000_000,
        max_future_drift_ms: 1000,
        commit_limits: CommitResourceLimits::default(),
        protocol_effects: ProtocolEffectsPolicy::Disabled,
    })
}
fn transactions(plan: &CellPlan, index: u32) -> Vec<CellTx> {
    let mut previous = initial_outpoint(plan, index);
    (0..plan.transactions)
        .map(|i| {
            let tx = CellTx::new(
                vec![CellInput::new(previous, SINCE)],
                vec![CellDep { out_point: DEP, dep_type: DepType::Code }],
                vec![output()],
                vec![u64::from(i + 1).to_le_bytes().to_vec()],
                vec![],
            )
            .unwrap();
            previous = OutPoint::new(tx.id(), 0);
            tx
        })
        .collect()
}
fn request(head: &SessionHead, txs: &[CellTx]) -> AnyResult<BlockRequest> {
    Ok(BlockRequest {
        transactions: txs.iter().map(serialize_transaction_molecule).collect::<std::result::Result<_, _>>()?,
        frame_input: FrameInput {
            commands: vec![],
            start_position: head.input_position,
            end_position: head.input_position + txs.len() as u64,
            logical_time_start: head.logical_time,
            logical_time_end: head.logical_time + txs.len() as u64,
            payload: txs.iter().flat_map(|tx| tx.id()).collect(),
        },
        scheduler_commitment: hash(b"myelin:measurement:sequential-CellTx"),
        data_commitments: vec![],
        timestamp_ms: head.timestamp_ms + 1,
        local_now_ms: head.timestamp_ms + 1,
        successor: None,
        handoffs: vec![],
        consume_handoffs: vec![],
        handoff_actions: vec![],
    })
}
fn check_state<E: TransitionExecutor, S: SessionStore>(
    chain: &SessionChain<E, S>,
    plan: &CellPlan,
    index: u32,
) -> AnyResult<serde_json::Value> {
    let txs = transactions(plan, index);
    let count = chain.head().input_position;
    if count != u64::from(plan.transactions) {
        return Err("Cell workload did not commit every transaction".into());
    }
    let last = txs.last().ok_or("empty Cell workload")?;
    let mut expected = CellStateTree::new();
    expected.insert(
        OutPoint::new(last.id(), 0),
        CellEntry::from_output(&output(), &count.to_le_bytes(), chain.head().finalised_height.unwrap(), false)?,
    );
    let expected = encode_cell_snapshot(&expected)?;
    let inspector = |_: &InspectContext<'_>, snapshot: &[u8], _: &[u8]| Ok(snapshot.to_vec());
    if chain.inspect_head(&inspector, b"cells")?.result != expected {
        return Err("live cells differ from final rollover output, capacity, sequence or creation height".into());
    }
    Ok(
        serde_json::json!({"live_cells":1,"capacity":CAPACITY,"sequence":count,"outpoint_txid":BlobId(last.id()),"snapshot":BlobId::of(&expected)}),
    )
}
#[derive(Serialize)]
struct CellBatch {
    session: u32,
    height: u64,
    transactions: usize,
    prepare_ns: u64,
    execution: ExecutorCosts,
    local_poa_sign_ns: u64,
    verify_and_sync_commit_ns: u64,
    vm_cycles: u64,
    trigger: String,
}
#[derive(Serialize)]
struct CellSample {
    session: u32,
    sequence: usize,
    txid: BlobId,
    height: u64,
    scheduled_ns: u64,
    admitted_ns: u64,
    simulation_finished_ns: u64,
    committed_ns: u64,
}
fn run_session(root: &Path, plan: &CellPlan, index: u32, program: Hash32, store: Arc<RocksSessionStore>) -> AnyResult<SessionResult> {
    let cfg = config(plan, index, program)?;
    let (verifier, signer) = consensus()?;
    let costs = Arc::new(Mutex::new(ExecutorCosts::default()));
    let executor = Measured { inner: initial(plan, index), costs: costs.clone(), spin: Duration::ZERO };
    let mut chain = SessionChain::create(cfg.clone(), verifier.clone(), executor, store.clone())?;
    let txs = transactions(plan, index);
    // The first candidate executes a valid predecessor then a script-rejected
    // successor. Subsequent production must still consume the original genesis.
    let mut invalid = txs[..2].to_vec();
    invalid[1].inputs[0].since = SINCE - 1;
    let before = chain.head().clone();
    let phase = Instant::now();
    let error = match chain.prepare_block(request(&before, &invalid)?) {
        Ok(_) => return Err("strict timelock accepted an invalid successor".into()),
        Err(error) => error.to_string(),
    };
    if !error.contains("strict VM:")
        || !error.contains("Script exited with code 3")
        || chain.head() != &before
        || store.load_head(cfg.session_id)? != before
    {
        return Err("script rejection did not preserve the canonical head".into());
    }
    save(
        &root.join(format!("rejection-{index}.json")),
        &serde_json::json!({
            "elapsed_ns":ns(phase.elapsed()),"error":error,"candidate_transactions":2,"head_unchanged":true,
        }),
    )?;
    let mut samples = BufWriter::new(File::create(root.join(format!("transactions-{index}.jsonl")))?);
    let mut batches = BufWriter::new(File::create(root.join(format!("batches-{index}.jsonl")))?);
    let mut schedule = ProductionSchedule::new(plan.trigger.production(plan.window_us))?;
    let mut manual_due = Duration::from_micros(plan.window_us);
    let mut next = 0;
    let mut confirmed = 0;
    let mut admitted = VecDeque::new();
    let mut vm_cycles = 0;
    let origin = Instant::now();
    while confirmed < txs.len() {
        if origin.elapsed() > Duration::from_secs(3600) {
            return Err("Cell session exceeded one hour".into());
        }
        while next < txs.len() && admitted.len() < 128 && origin.elapsed() >= Duration::from_micros(next as u64 * plan.arrival_us) {
            admitted.push_back(ns(origin.elapsed()));
            next += 1;
        }
        let now = origin.elapsed();
        let cause = if matches!(plan.trigger, Trigger::Manual) {
            if !admitted.is_empty() && now >= manual_due {
                manual_due = now + Duration::from_micros(plan.window_us);
                Some(ProductionCause::Manual)
            } else {
                None
            }
        } else {
            schedule.observe(now, !admitted.is_empty(), admitted.len() >= plan.batch)
        };
        let Some(cause) = cause else {
            std::thread::sleep(Duration::from_micros(100));
            continue;
        };
        let count = plan.batch.min(admitted.len());
        *costs.lock().unwrap() = ExecutorCosts::default();
        let candidate = request(chain.head(), &txs[confirmed..confirmed + count])?;
        let phase = Instant::now();
        let prepared = chain.prepare_block(candidate)?;
        let prepare_ns = ns(phase.elapsed());
        let simulation_finished_ns = ns(costs.lock().unwrap().finished.ok_or("missing VM completion")?.duration_since(origin));
        let cycles = prepared.frame().resources.cycles;
        if cycles == 0 {
            return Err("strict VM reported no cycles".into());
        }
        vm_cycles += cycles;
        let phase = Instant::now();
        let SelectedConsensus::ProofOfAuthority(engine) = verifier.selected_consensus() else { unreachable!() };
        let proof =
            FinalityProof::ProofOfAuthority(engine.seal_from_signer(prepared.block().hash(), prepared.block().number, &signer)?);
        let local_poa_sign_ns = ns(phase.elapsed());
        let phase = Instant::now();
        let head = chain.commit_finalised(prepared, proof)?;
        let verify_and_sync_commit_ns = ns(phase.elapsed());
        let committed_ns = ns(origin.elapsed());
        let height = head.finalised_height.ok_or("no committed Cell block")?;
        for (sequence, tx) in txs.iter().enumerate().skip(confirmed).take(count) {
            json_line(
                &mut samples,
                &CellSample {
                    session: index,
                    sequence,
                    txid: BlobId(tx.id()),
                    height,
                    scheduled_ns: ns(Duration::from_micros(sequence as u64 * plan.arrival_us)),
                    admitted_ns: admitted.pop_front().ok_or("admitted transaction disappeared")?,
                    simulation_finished_ns,
                    committed_ns,
                },
            )?;
        }
        json_line(
            &mut batches,
            &CellBatch {
                session: index,
                height,
                transactions: count,
                prepare_ns,
                execution: costs.lock().unwrap().clone(),
                local_poa_sign_ns,
                verify_and_sync_commit_ns,
                vm_cycles: cycles,
                trigger: format!("{cause:?}"),
            },
        )?;
        confirmed += count;
    }
    samples.flush()?;
    batches.flush()?;
    let production_elapsed_ns = ns(origin.elapsed());
    let phase = Instant::now();
    let state = check_state(&chain, plan, index)?;
    Ok(SessionResult {
        index,
        genesis: BlobId(cfg.commitment()),
        head: head_view(chain.head())?,
        inputs: plan.transactions,
        vm_cycles,
        unknown_outcomes: 0,
        production_elapsed_ns,
        result_validation_ns: ns(phase.elapsed()),
        state,
    })
}
pub(super) fn run(root: &Path, plan: CellPlan) -> AnyResult<serde_json::Value> {
    plan.validate()?;
    fs::create_dir(root)?;
    save(&root.join("plan.json"), &plan)?;
    let program = games::artifact_digest()?;
    let store = Arc::new(RocksSessionStore::open(root.join("canonical"))?);
    let barrier = Arc::new(Barrier::new(plan.sessions as usize));
    let mut threads = vec![];
    for index in plan.first_session..plan.first_session + plan.sessions {
        let (root, plan, store, barrier) = (root.to_owned(), plan.clone(), store.clone(), barrier.clone());
        threads.push(std::thread::spawn(move || {
            barrier.wait();
            run_session(&root, &plan, index, program, store).map_err(|e| e.to_string())
        }));
    }
    let mut sessions = vec![];
    for thread in threads {
        sessions.push(thread.join().map_err(|_| "Cell workload thread panicked")??);
    }
    save(&root.join("sessions.json"), &sessions)?;
    let storage = storage_measurement(&store)?;
    Ok(serde_json::json!({"stage":"produced","program":BlobId(program),"sessions":sessions,
        "storage":storage,
        "execution":"ckb-strict-cell-rollover","script":BlobId::of(TIMELOCK_ABSOLUTE_SCRIPT),"script_hash_type":"data2",
        "transaction_cycle_limit":MAX_TX_CYCLES,"input_source":"scheduled in-memory transactions; no command journal or RPC",
        "scope":"script checks since field; no chain-time enforcement, game rules, CKB settlement or remote proof collection"}))
}
fn recover<S: SessionStore>(root: &Path, plan: &CellPlan, store: Arc<S>) -> AnyResult<Vec<serde_json::Value>> {
    let expected: Vec<SessionResult> = load(&root.join("sessions.json"))?;
    if expected.len() != plan.sessions as usize {
        return Err("Cell workload session count changed".into());
    }
    let program = games::artifact_digest()?;
    let mut sessions = vec![];
    for session in expected {
        let cfg = config(plan, session.index, program)?;
        let (chain, recovery) = archive::restore_chain(store.clone(), &cfg, restore)?;
        let state = check_state(&chain, plan, session.index)?;
        if session.head != head_view(chain.head())? || session.state != state || session.vm_cycles == 0 {
            return Err("recovered Cell head/state/cycles differ".into());
        }
        let replay = archive::replay(&chain, session.vm_cycles, restore)?;
        sessions
            .push(serde_json::json!({"index":session.index,"head":session.head,"state":state,"recovery":recovery,"replay":replay}));
    }
    Ok(sessions)
}
pub(super) fn resume(op: &str, root: &Path) -> AnyResult<serde_json::Value> {
    let plan: CellPlan = load(&root.join("plan.json"))?;
    plan.validate()?;
    match op {
        "recover" if root.join("canonical/CURRENT").is_file() => {
            let start = Instant::now();
            let store = Arc::new(RocksSessionStore::open(root.join("canonical"))?);
            let open_ns = ns(start.elapsed());
            Ok(serde_json::json!({"stage":"recovered","open_ns":open_ns,"sessions":recover(root,&plan,store)?}))
        }
        "archive" => archive::export_sessions(root, plan.transactions, plan.sessions, |index, program| config(&plan, index, program)),
        "cold" if !root.join("canonical").exists() => {
            let (store, archive_audit_ns, usage) = archive::open_archive(root)?;
            let program = BlobId(games::artifact_digest()?);
            for expected in load::<Vec<SessionResult>>(&root.join("sessions.json"))? {
                archive::check_archive_session(&store, &config(&plan, expected.index, program.0)?, &expected, program)?;
            }
            Ok(serde_json::json!({"stage":"cold-replayed","archive_audit_ns":archive_audit_ns,"work":usage,
                "sessions":recover(root,&plan,store)?,"original_databases_present":false}))
        }
        _ => {
            Err("Cell recovery requires an existing database; cold replay requires its removal; expected recover|archive|cold".into())
        }
    }
}
