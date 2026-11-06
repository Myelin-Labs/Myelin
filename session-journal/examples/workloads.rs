//! Reproducible host-game and strict CellTx measurements; no deployment SLO.
#[path = "support/workload_archive.rs"]
mod archive;
#[path = "support/workload_cells.rs"]
mod cells;
#[path = "support/workload_evidence.rs"]
mod evidence;
#[path = "support/command_games.rs"]
mod games;
#[path = "support/workload_rewards.rs"]
mod rewards;

use games::{AnyResult, Auth, Game, Kind};
use myelin_consensus::{Authority, CommitteeSigner, ConsensusConfig, FinalityProof, ProofOfAuthorityConfig, SelectedConsensus};
use myelin_session::*;
use myelin_session_journal::{application::*, *};
use myelin_session_producer::{ProductionCause, ProductionSchedule, ProductionTrigger};
use myelin_session_runtime::RegisteredFinalityVerifier;
use myelin_session_store_rocksdb::{RocksSessionStore, RocksSessionStoreOptions};
use serde::{Deserialize, Serialize};
use std::{
    collections::VecDeque,
    fs::{self, File},
    io::{BufWriter, Read, Write},
    path::{Path, PathBuf},
    sync::{Arc, Barrier, Mutex},
    time::{Duration, Instant},
};

#[derive(Clone, Copy, Debug, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
enum Trigger {
    Instant,
    Interval,
    Open,
    Manual,
}
impl Trigger {
    fn production(self, window_us: u64) -> ProductionTrigger {
        match self {
            Self::Instant => ProductionTrigger::Instant,
            Self::Interval => ProductionTrigger::Interval { block_time: Duration::from_micros(window_us), produce_empty: false },
            Self::Open => ProductionTrigger::Open { period: Duration::from_micros(window_us) },
            Self::Manual => ProductionTrigger::Never,
        }
    }
}
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct Plan {
    kind: Kind,
    sessions: u32,
    first_session: u32,
    commands: u32,
    batch: usize,
    seed: u64,
    trigger: Trigger,
    arrival_us: u64,
    window_us: u64,
    slow_first_session_us: u64,
    unknown_every_blocks: u64,
    crash_after_first_commit: bool,
}
impl Plan {
    fn validate(&self) -> AnyResult<()> {
        if !(1..=64).contains(&self.sessions)
            || self.first_session.checked_add(self.sessions).is_none()
            || !(1..=4096).contains(&self.commands)
            || self.kind == Kind::Turns && self.commands > 9
            || !(1..=64).contains(&self.batch)
            || self.arrival_us > 1_000_000
            || !(1..=1_000_000).contains(&self.window_us)
            || self.slow_first_session_us > 1_000_000
            || self.crash_after_first_commit && self.sessions != 1
        {
            return Err("invalid bounded workload plan".into());
        }
        Ok(())
    }
}
fn load<T: serde::de::DeserializeOwned>(path: &Path) -> AnyResult<T> {
    let mut bytes = vec![];
    File::open(path)?.take(8 * 1024 * 1024 + 1).read_to_end(&mut bytes)?;
    if bytes.len() > 8 * 1024 * 1024 {
        return Err("measurement JSON exceeds limit".into());
    }
    Ok(serde_json::from_slice(&bytes)?)
}
fn save(path: &Path, value: &impl Serialize) -> AnyResult<()> {
    let mut output = File::options().write(true).create_new(true).open(path)?;
    serde_json::to_writer_pretty(&mut output, value)?;
    output.write_all(b"\n")?;
    output.sync_all()?;
    Ok(())
}
fn hash(bytes: &[u8]) -> Hash32 {
    *blake3::hash(bytes).as_bytes()
}
fn session_config(plan: &Plan, index: u32, program: Hash32) -> SessionConfig {
    let mut cfg = games::config(plan.kind, program);
    cfg.session_id = hash(format!("myelin:workload:v1:{}:{}:{index}", plan.kind.name(), plan.seed).as_bytes());
    cfg
}
fn command(plan: &Plan, cfg: &SessionConfig, i: u32) -> OriginalCommand {
    let random = hash(&[plan.seed.to_le_bytes().as_slice(), i.to_le_bytes().as_slice()].concat());
    let (action, actor) = match plan.kind {
        Kind::Inventory if i == 0 => (games::Action::Buy { quantity: 1, expected_revision: 0 }, 3),
        Kind::Inventory => (games::Action::Save { checkpoint: u32::from(random[0]) % 101, expected_revision: u64::from(i) }, 3),
        Kind::Turns => (
            games::Action::Move { seat: (i % 2) as u8, square: [0, 1, 2, 4, 3, 5, 7, 6, 8][i as usize], expected_turn: u64::from(i) },
            3 + (i % 2) as u8,
        ),
        Kind::Ticks => {
            use games::tick_game::{Event, Tick};
            let phase = i % 32;
            let events = match phase {
                2 => vec![Event::Disconnect { seat: 1 }],
                4 => vec![Event::Takeover { seat: 1 }],
                8 => vec![Event::Reconnect { seat: 1 }],
                _ => vec![],
            };
            (
                games::Action::Tick(Tick {
                    tick: u64::from(i),
                    events,
                    inputs: [(random[0] % 3) as i8 - 1, if (2..8).contains(&phase) { 0 } else { (random[1] % 3) as i8 - 1 }],
                }),
                5,
            )
        }
    };
    let id = hash(&[b"myelin:workload-command:v1".as_slice(), &cfg.session_id, &i.to_le_bytes()].concat());
    games::signed(cfg, id, action, actor)
}
fn consensus() -> AnyResult<(Arc<RegisteredFinalityVerifier>, CommitteeSigner)> {
    let signer = CommitteeSigner::new("command-game-validator", [7; 32])?;
    let config = ConsensusConfig::proof_of_authority(ProofOfAuthorityConfig {
        authorities: vec![Authority { id: signer.validator_id().into(), public_key: signer.ckb_public_key() }],
    });
    Ok((Arc::new(RegisteredFinalityVerifier::new(config)?), signer))
}
fn ns(duration: Duration) -> u64 {
    duration.as_nanos().try_into().unwrap_or(u64::MAX)
}
#[derive(Clone, Default, Serialize)]
struct ExecutorCosts {
    outer_clone_ns: u64,
    execute_ns: u64,
    snapshot_ns: u64,
    #[serde(skip)]
    finished: Option<Instant>,
}
struct Measured<E> {
    inner: E,
    costs: Arc<Mutex<ExecutorCosts>>,
    spin: Duration,
}
impl<E: Clone> Clone for Measured<E> {
    fn clone(&self) -> Self {
        let start = Instant::now();
        let inner = self.inner.clone();
        self.costs.lock().unwrap().outer_clone_ns += ns(start.elapsed());
        Self { inner, costs: self.costs.clone(), spin: self.spin }
    }
}
impl<E> Measured<E> {
    fn measure(
        &mut self,
        execute: impl FnOnce(&mut E) -> std::result::Result<ExecutionOutcome, String>,
    ) -> std::result::Result<ExecutionOutcome, String> {
        let start = Instant::now();
        while start.elapsed() < self.spin {
            std::hint::spin_loop();
        }
        let result = execute(&mut self.inner);
        let finished = Instant::now();
        let mut costs = self.costs.lock().unwrap();
        costs.execute_ns += ns(finished.duration_since(start));
        costs.finished = Some(finished);
        result
    }
}
impl<E: TransitionExecutor> TransitionExecutor for Measured<E> {
    fn state_root(&self) -> Hash32 {
        self.inner.state_root()
    }
    fn snapshot(&self) -> std::result::Result<Vec<u8>, String> {
        let start = Instant::now();
        let result = self.inner.snapshot();
        self.costs.lock().unwrap().snapshot_ns += ns(start.elapsed());
        result
    }
    fn execute_block(
        &mut self,
        height: u64,
        frame: &FrameInput,
        handoffs: &[SessionHandoff],
        txs: &[Vec<u8>],
    ) -> std::result::Result<ExecutionOutcome, String> {
        self.measure(|inner| inner.execute_block(height, frame, handoffs, txs))
    }
    fn execute_block_with_lifecycle(
        &mut self,
        context: &ProtocolEffectsContext,
        actions: &[ResolvedHandoffAction],
        frame: &FrameInput,
        handoffs: &[SessionHandoff],
        txs: &[Vec<u8>],
    ) -> std::result::Result<ExecutionOutcome, String> {
        self.measure(|inner| inner.execute_block_with_lifecycle(context, actions, frame, handoffs, txs))
    }
    fn execute_block_with_context(
        &mut self,
        context: &ProtocolEffectsContext,
        frame: &FrameInput,
        handoffs: &[SessionHandoff],
        txs: &[Vec<u8>],
    ) -> std::result::Result<ExecutionOutcome, String> {
        self.measure(|inner| inner.execute_block_with_context(context, frame, handoffs, txs))
    }
}
#[derive(Serialize)]
struct Sample {
    session: u32,
    sequence: u32,
    command: myelin_session_bundle::BlobId,
    height: u64,
    batch: usize,
    scheduled_ns: u64,
    enqueue_started_ns: u64,
    enqueued_ns: u64,
    simulation_finished_ns: u64,
    committed_ns: u64,
    response_ns: u64,
    recovered_unknown_outcome: bool,
}
#[derive(Serialize)]
struct Batch {
    height: u64,
    commands: usize,
    trigger: String,
    reserve_ns: u64,
    prepare_ns: u64,
    execution: ExecutorCosts,
    local_poa_sign_ns: u64,
    verify_and_sync_commit_ns: u64,
    reconcile_and_ack_ns: u64,
    vm_cycles: u64,
    state_root: myelin_session_bundle::BlobId,
}
#[derive(Serialize, Deserialize)]
struct SessionResult {
    index: u32,
    genesis: myelin_session_bundle::BlobId,
    head: HeadView,
    inputs: u32,
    vm_cycles: u64,
    unknown_outcomes: u64,
    production_elapsed_ns: u64,
    result_validation_ns: u64,
    state: serde_json::Value,
}
#[derive(Serialize, Deserialize, PartialEq, Eq)]
struct HeadView {
    record: myelin_session_bundle::BlobId,
    height: Option<u64>,
    block_hash: myelin_session_bundle::BlobId,
    state_root: myelin_session_bundle::BlobId,
    input_position: u64,
}
fn head_view(head: &SessionHead) -> AnyResult<HeadView> {
    use myelin_session_bundle::BlobId;
    Ok(HeadView {
        record: BlobId::of(&head.encode()?),
        height: head.finalised_height,
        block_hash: BlobId(head.block_hash),
        state_root: BlobId(head.state_root),
        input_position: head.input_position,
    })
}
fn json_line(output: &mut impl Write, value: &impl Serialize) -> AnyResult<()> {
    serde_json::to_writer(&mut *output, value)?;
    output.write_all(b"\n")?;
    Ok(())
}
fn journal_path(root: &Path, index: u32) -> PathBuf {
    root.join(format!("journal-{index}"))
}
fn run_session(root: &Path, plan: &Plan, index: u32, program: Hash32, store: Arc<RocksSessionStore>) -> AnyResult<SessionResult> {
    let cfg = session_config(plan, index, program);
    let (verifier, signer) = consensus()?;
    let costs = Arc::new(Mutex::new(ExecutorCosts::default()));
    let spin = if index == plan.first_session { Duration::from_micros(plan.slow_first_session_us) } else { Duration::ZERO };
    let executor = Measured {
        inner: CommandExecutor::new(cfg.clone(), Game::new(plan.kind), Arc::new(Auth(plan.kind)))?,
        costs: costs.clone(),
        spin,
    };
    let mut chain = SessionChain::create(cfg.clone(), verifier.clone(), executor, store.clone())?;
    let journal = CommandJournal::open(journal_path(root, index), &cfg, JournalLimits::default())?;
    let commands: Vec<_> = (0..plan.commands).map(|i| command(plan, &cfg, i)).collect();
    let mut samples = BufWriter::new(File::create(root.join(format!("commands-{index}.jsonl")))?);
    let mut batches = BufWriter::new(File::create(root.join(format!("batches-{index}.jsonl")))?);
    let mut schedule = ProductionSchedule::new(plan.trigger.production(plan.window_us))?;
    let mut manual_due = Duration::from_micros(plan.window_us);
    let mut next = 0u32;
    let mut pending = VecDeque::new();
    let mut confirmed = 0u32;
    let mut unknown_outcomes = 0;
    let origin = Instant::now();
    while confirmed < plan.commands {
        if origin.elapsed() > Duration::from_secs(3600) {
            return Err("workload session exceeded one hour".into());
        }
        while next < plan.commands && pending.len() < 128 {
            let due = Duration::from_micros(u64::from(next) * plan.arrival_us);
            if origin.elapsed() < due {
                break;
            }
            let started = ns(origin.elapsed());
            journal.enqueue(commands[next as usize].clone(), &Auth(plan.kind))?;
            pending.push_back((next, ns(due), started, ns(origin.elapsed())));
            next += 1;
        }
        let now = origin.elapsed();
        let cause = if matches!(plan.trigger, Trigger::Manual) {
            if !pending.is_empty() && now >= manual_due {
                manual_due = now + Duration::from_micros(plan.window_us);
                Some(ProductionCause::Manual)
            } else {
                None
            }
        } else {
            schedule.observe(now, !pending.is_empty(), pending.len() >= plan.batch)
        };
        let Some(cause) = cause else {
            std::thread::sleep(Duration::from_micros(100));
            continue;
        };
        let phase = Instant::now();
        let reservation = journal
            .reserve_commands(chain.head(), plan.batch, 1_000_000, &Auth(plan.kind))?
            .ok_or("ready source had no reservation")?;
        let reserve_ns = ns(phase.elapsed());
        let count = reservation.command_ids.len();
        journal.begin_commit(reservation.token, chain.head())?;
        let timestamp = chain.head().timestamp_ms + 1;
        let request = request_from_reservation(&cfg, &reservation, timestamp, timestamp)?;
        *costs.lock().unwrap() = ExecutorCosts::default();
        let phase = Instant::now();
        let prepared = chain.prepare_block(request)?;
        let prepare_ns = ns(phase.elapsed());
        let simulation_finished_ns =
            ns(costs.lock().unwrap().finished.ok_or("executor did not record completion")?.duration_since(origin));
        let vm_cycles = prepared.frame().resources.cycles;
        let phase = Instant::now();
        let SelectedConsensus::ProofOfAuthority(engine) = verifier.selected_consensus() else { unreachable!() };
        let proof =
            FinalityProof::ProofOfAuthority(engine.seal_from_signer(prepared.block().hash(), prepared.block().number, &signer)?);
        let local_poa_sign_ns = ns(phase.elapsed());
        let phase = Instant::now();
        let head = chain.commit_finalised(prepared, proof)?;
        let verify_and_sync_commit_ns = ns(phase.elapsed());
        let committed_ns = ns(origin.elapsed());
        if plan.crash_after_first_commit {
            std::process::exit(76);
        }
        let height = head.finalised_height.ok_or("committed head has no height")?;
        let recovered_unknown_outcome = plan.unknown_every_blocks != 0 && (height + 1) % plan.unknown_every_blocks == 0;
        let phase = Instant::now();
        if recovered_unknown_outcome {
            journal.mark_unknown(reservation.token)?;
            unknown_outcomes += 1;
        }
        journal.reconcile(&chain, business_result)?;
        let mut measured_commands = vec![];
        for expected_id in &reservation.command_ids {
            let (sequence, scheduled_ns, enqueue_started_ns, enqueued_ns) =
                pending.pop_front().ok_or("pending command disappeared")?;
            let original = &commands[sequence as usize];
            if original.identity.id != *expected_id {
                return Err("reservation reordered commands".into());
            }
            let outcome = journal.acknowledge(*expected_id)?;
            if !matches!(outcome, BusinessOutcome::Succeeded(_)) {
                return Err("valid workload action was rejected".into());
            }
            if recovered_unknown_outcome {
                // Lost client response: retry the same original identity after ACK.
                let before = chain.head().clone();
                journal.enqueue(original.clone(), &Auth(plan.kind))?;
                if journal.acknowledge(*expected_id)? != outcome || chain.head() != &before {
                    return Err("retry changed a committed outcome".into());
                }
            }
            measured_commands.push(Sample {
                session: index,
                sequence,
                command: myelin_session_bundle::BlobId(*expected_id),
                height,
                batch: count,
                scheduled_ns,
                enqueue_started_ns,
                enqueued_ns,
                simulation_finished_ns,
                committed_ns,
                response_ns: ns(origin.elapsed()),
                recovered_unknown_outcome,
            });
            confirmed += 1;
        }
        for message in store.load_block(cfg.session_id, height)?.outbox {
            store.acknowledge_outbox(cfg.session_id, message.id)?;
        }
        let reconcile_and_ack_ns = ns(phase.elapsed());
        for sample in measured_commands {
            json_line(&mut samples, &sample)?;
        }
        json_line(
            &mut batches,
            &Batch {
                height,
                commands: count,
                trigger: format!("{cause:?}"),
                reserve_ns,
                prepare_ns,
                execution: costs.lock().unwrap().clone(),
                local_poa_sign_ns,
                verify_and_sync_commit_ns,
                reconcile_and_ack_ns,
                vm_cycles,
                state_root: myelin_session_bundle::BlobId(head.state_root),
            },
        )?;
    }
    samples.flush()?;
    batches.flush()?;
    let production_elapsed_ns = ns(origin.elapsed());
    let validation_start = Instant::now();
    let port = command_inspector(cfg.clone(), Arc::new(Auth(plan.kind)), {
        let kind = plan.kind;
        move |bytes| Game::restore(kind, bytes)
    });
    let inspection = chain.inspect_head(&port, b"state")?;
    let mut expected = Game::new(plan.kind);
    for (i, original) in commands.iter().enumerate() {
        expected.apply_rule(i as u64, i as u64, &original.canonical_command)?.map_err(|e| e.to_string())?;
    }
    if expected.snapshot()? != inspection.result || chain.head().input_position != u64::from(plan.commands) {
        return Err("durable game state or input count differs from the unbatched input fold".into());
    }
    Ok(SessionResult {
        index,
        genesis: myelin_session_bundle::BlobId(cfg.commitment()),
        head: head_view(chain.head())?,
        inputs: confirmed,
        vm_cycles: 0,
        unknown_outcomes,
        production_elapsed_ns,
        result_validation_ns: ns(validation_start.elapsed()),
        state: serde_json::from_slice(&inspection.result)?,
    })
}
fn storage_inventory(store: &RocksSessionStore) -> AnyResult<myelin_session_store_rocksdb::SnapshotInventory> {
    Ok(store.snapshot_inventory(
        myelin_session_store_rocksdb::SnapshotLimits {
            max_entries: 2_000_000,
            max_bytes: 64 * 1024 * 1024 * 1024,
            max_sessions: 64,
            max_archives: 1024,
        },
        &|_| Ok(()),
    )?)
}
// Inspect actual stored values after all producers have joined. Re-encoding uses
// the same current codecs as RocksDB; these are logical values, not SST bytes or
// device traffic. Keep this scan separate from production latency samples.
fn storage_measurement(store: &RocksSessionStore) -> AnyResult<serde_json::Value> {
    #[derive(Default, Serialize)]
    struct Values {
        records: u64,
        encoded_value_bytes: u64,
    }
    impl Values {
        fn add(&mut self, bytes: Vec<u8>) {
            self.records += 1;
            self.encoded_value_bytes += bytes.len() as u64;
        }
    }
    let start = Instant::now();
    let inventory = storage_inventory(store)?;
    let mut sessions = vec![];
    for head in &inventory.heads {
        let (mut blocks, mut receipts, mut consensus_wal) = (Values::default(), Values::default(), Values::default());
        if let Some(last) = head.finalised_height {
            for height in 0..=last {
                let record = store.load_block(head.session_id, height)?;
                blocks.add(record.encode()?);
                for command in &record.frame_input.commands {
                    receipts.add(
                        store
                            .load_command_receipt(head.session_id, command.id)?
                            .ok_or("storage measurement found a missing receipt")?
                            .encode()?,
                    );
                }
            }
        }
        if let Some(wal) = store.load_consensus_wal(head.session_id)? {
            consensus_wal.add(wal.encode()?);
        }
        sessions.push(serde_json::json!({"session":myelin_session_bundle::BlobId(head.session_id),
            "blocks":blocks,"command_receipts":receipts,"consensus_wal":consensus_wal}));
    }
    let mut report = serde_json::to_value(inventory)?;
    report["record_storage"] = serde_json::json!({"sessions":sessions,"scan_elapsed_ns":ns(start.elapsed()),
        "scope":"current encoded values; excludes keys, RocksDB WAL/SST framing, compression and device write amplification"});
    Ok(report)
}
fn run(root: &Path, plan: Plan) -> AnyResult<serde_json::Value> {
    plan.validate()?;
    fs::create_dir(root)?;
    save(&root.join("plan.json"), &plan)?;
    let program = games::artifact_digest()?;
    let store = Arc::new(RocksSessionStore::open_with_options(root.join("canonical"), RocksSessionStoreOptions::default())?);
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
        sessions.push(thread.join().map_err(|_| "workload thread panicked")??);
    }
    save(&root.join("sessions.json"), &sessions)?;
    let storage = storage_measurement(&store)?;
    Ok(serde_json::json!({"stage":"produced", "program":myelin_session_bundle::BlobId(program), "sessions":sessions,
        "storage":storage,
        "writer_model":"local-single-writer-per-session; shared RocksDB; development PoA keys",
        "execution":"host-command-adapter", "vm_cycles":0, "remote_proof_collection":"not-run", "external_evidence_publication":"not-run"}))
}
fn main() -> AnyResult<()> {
    let args: Vec<_> = std::env::args().skip(1).collect();
    let report = match args.as_slice() {
        [kind, root, mode] if kind == "evidence-provider" => evidence::serve(Path::new(root), mode)?,
        [kind, op, plan, root, marker] if kind == "rewards" && op == "run-after-quota" => {
            rewards::run_after_quota(Path::new(root), load(Path::new(plan))?, Path::new(marker))?
        }
        [kind, op, plan, root] if kind == "rewards" && op == "quota" => rewards::quota_run(Path::new(root), load(Path::new(plan))?)?,
        [kind, op, plan, root] if kind == "rewards" && op == "run" => rewards::run(Path::new(root), load(Path::new(plan))?)?,
        [kind, op, root] if kind == "rewards" => rewards::resume(op, Path::new(root))?,
        [kind, op, plan, root] if kind == "cells" && op == "run" => cells::run(Path::new(root), load(Path::new(plan))?)?,
        [kind, op, root] if kind == "cells" => cells::resume(op, Path::new(root))?,
        [op, plan, root] if op == "run" => run(Path::new(root), load(Path::new(plan))?)?,
        [op, root] if op == "recover" => archive::recover(Path::new(root))?,
        [op, root] if op == "archive" => archive::export(Path::new(root))?,
        [op, root] if op == "cold" => archive::cold(Path::new(root))?,
        _ => return Err(
            "usage: workloads [cells|rewards] run <plan.json> <new-directory> | [cells|rewards] recover|archive|cold <run-directory>"
                .into(),
        ),
    };
    println!("{}", serde_json::to_string(&report)?);
    Ok(())
}
