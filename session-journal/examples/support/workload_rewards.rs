//! Authorized match-completion rewards, from source escrow to inventory.
//! Match completion is supplied input, not a claim that transfer code judges games.
#[path = "workload_quota.rs"]
mod quota;
use super::*;
use myelin_session_bundle::BlobId;
use myelin_session_transfer::{
    command_frame, input_schema, restore_executor, state_codec, verify_intent, AssetAccount, AssetId, SourceBindings, SourceIdentity,
    TransferCommand, TransferExecutor, TransferKind, TransferLimits, TransferPayload, TransferPolicy, TransferSigner, TransferState,
    TransferTarget,
};

const ASSET: AssetId = AssetId { namespace: [80; 32], object: [81; 32] };
const COMPONENTS: &[u8] = b"authorized-match-completion-reward";
type Chain = SessionChain<Measured<TransferExecutor>, RocksSessionStore>;

#[derive(Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub(super) struct RewardPlan {
    pairs: u32,
    first_pair: u32,
    rewards: u32,
    batch: usize,
    seed: u64,
    trigger: Trigger,
    arrival_us: u64,
    window_us: u64,
    cancel_every: u32,
    crash_after_credit: bool,
    evidence: evidence::Mode,
}
impl RewardPlan {
    fn validate(&self) -> AnyResult<()> {
        if !(1..=32).contains(&self.pairs)
            || self.first_pair.checked_add(self.pairs).and_then(|n| n.checked_mul(2)).is_none()
            || !(2..=4096).contains(&self.rewards)
            || !(1..=64).contains(&self.batch)
            || self.arrival_us > 1_000_000
            || !(1..=1_000_000).contains(&self.window_us)
            || self.cancel_every == 1
            || self.crash_after_credit && (self.pairs != 1 || self.batch != 1 || self.cancel_every != 0)
            || self.evidence != evidence::Mode::Disabled && (self.crash_after_credit || self.rewards > 128)
        {
            return Err("invalid bounded reward workload plan".into());
        }
        Ok(())
    }
    fn cancelled(&self, sequence: u32) -> bool {
        self.cancel_every > 0 && (sequence + 1).is_multiple_of(self.cancel_every)
    }
    fn refunded(&self) -> u64 {
        if self.cancel_every == 0 {
            0
        } else {
            u64::from(self.rewards / self.cancel_every)
        }
    }
}
fn owner() -> AnyResult<TransferSigner> {
    Ok(TransferSigner::new([3; 32])?)
}
fn recipient() -> AnyResult<Hash32> {
    Ok(TransferSigner::new([4; 32])?.actor())
}
fn match_id(plan: &RewardPlan, pair: u32, sequence: u32) -> Hash32 {
    hash(format!("myelin:measurement:match-reward:{}:{pair}:{sequence}", plan.seed).as_bytes())
}
fn config(plan: &RewardPlan, index: u32, initial: &TransferState, program: Hash32) -> AnyResult<SessionConfig> {
    let (v, _) = consensus()?;
    let d = v.descriptor();
    let mut cfg = SessionConfig {
        session_id: hash(format!("myelin:measurement:reward-ledger:{}:{index}", plan.seed).as_bytes()),
        consensus_kind: d.consensus_kind,
        consensus_config_commitment: d.config_commitment,
        consensus_module_commitment: d.commitment(),
        consensus_wal_schema_hash: d.wal_schema_hash,
        application_profile: ApplicationProfile {
            application_id: "measurement/match-reward-ledger".into(),
            command_identity_required: true,
            program_digest: program,
            input_schema_hash: input_schema(),
            state_codec_hash: state_codec(),
            logical_time_policy_hash: hash(b"one-step-per-authorized-transfer-command"),
            entropy_policy_hash: hash(b"no-randomness"),
            vm: ApplicationVmProfile { spawn_ipc_required: false },
            resources: ApplicationResourceEnvelope {
                max_frame_inputs: 64,
                max_frame_input_bytes: 1_000_000,
                max_frame_cycles: 1,
                max_logical_time_span: 64,
                max_inspect_query_bytes: 32,
                max_inspect_result_bytes: 8 * 1024 * 1024,
            },
            court_profile_hash: hash(b"host-reference-transfer-replay;no-CKB-settlement"),
            handoff_policy_hash: initial.policy().intake_policy()?,
        },
        initial_state_root: initial.root()?,
        initial_input_position: 0,
        initial_logical_time: 0,
        initial_timestamp_ms: 0,
        predecessor: None,
        max_block_transactions: 64,
        max_block_bytes: 1_000_000,
        max_future_drift_ms: 1000,
        commit_limits: CommitResourceLimits::default(),
        protocol_effects: ProtocolEffectsPolicy::Disabled,
    };
    initial.policy().bind_config(&mut cfg)?;
    Ok(cfg)
}
fn settings(plan: &RewardPlan, pair: u32, program: Hash32) -> AnyResult<[(SessionConfig, TransferState); 2]> {
    let policy = TransferPolicy {
        deployment_id: hash(b"myelin:measurement:reward-deployment"),
        uniqueness_domain: hash(format!("myelin:measurement:reward-domain:{}:{pair}", plan.seed).as_bytes()),
        kind: TransferKind::Scarce,
        limits: TransferLimits::default(),
        accepted_sources: vec![],
    };
    let source = TransferState::new(
        policy.clone(),
        vec![AssetAccount {
            asset: ASSET,
            owner: owner()?.actor(),
            available: u64::from(plan.rewards),
            locked: 0,
            version: 0,
            components: COMPONENTS.to_vec(),
        }],
    )?;
    let source_cfg = config(plan, pair * 2, &source, program)?;
    let mut intake = policy;
    intake.accepted_sources.push(SourceIdentity::from_config(&source_cfg)?);
    let target = TransferState::new(intake, vec![])?;
    let target_cfg = config(plan, pair * 2 + 1, &target, program)?;
    Ok([(source_cfg, source), (target_cfg, target)])
}
fn request<E: TransitionExecutor, S: SessionStore>(
    chain: &SessionChain<E, S>,
    commands: Vec<TransferCommand>,
) -> AnyResult<BlockRequest> {
    Ok(BlockRequest {
        transactions: vec![],
        frame_input: command_frame(chain.config(), chain.head().input_position, chain.head().logical_time, commands)?,
        scheduler_commitment: hash(b"myelin:measurement:reward-sequential"),
        data_commitments: vec![],
        timestamp_ms: chain.head().timestamp_ms + 1,
        local_now_ms: chain.head().timestamp_ms + 1,
        successor: None,
        handoffs: vec![],
        consume_handoffs: vec![],
        handoff_actions: vec![],
    })
}
struct Step {
    simulation_ns: u64,
    committed_ns: u64,
    issued: Vec<SessionHandoff>,
    messages: Vec<Hash32>,
}
fn bind_messages(
    bindings: &mut std::collections::BTreeMap<Hash32, Vec<BlobId>>,
    handoffs: &[SessionHandoff],
    step: &Step,
) -> AnyResult<()> {
    if handoffs.len() != step.messages.len() {
        return Err("reward outbox count differs from executed commands".into());
    }
    for (handoff, message) in handoffs.iter().zip(&step.messages) {
        bindings.entry(handoff.id()).or_default().push(BlobId(*message));
    }
    Ok(())
}
struct Pair {
    chains: [Chain; 2],
    costs: [Arc<Mutex<ExecutorCosts>>; 2],
    store: Arc<RocksSessionStore>,
    verifier: Arc<RegisteredFinalityVerifier>,
    signer: CommitteeSigner,
    origin: Instant,
    pair: u32,
    batches: BufWriter<File>,
    evidence_commits: evidence::Commits,
    evidence_worker: Option<evidence::Worker>,
}
impl Pair {
    fn open(
        root: &Path,
        plan: &RewardPlan,
        pair: u32,
        program: Hash32,
        store: Arc<RocksSessionStore>,
        recovering: bool,
    ) -> AnyResult<Self> {
        let (verifier, signer) = consensus()?;
        let costs = std::array::from_fn(|_| Arc::new(Mutex::new(ExecutorCosts::default())));
        let mut chains = vec![];
        for (side, (cfg, initial)) in settings(plan, pair, program)?.into_iter().enumerate() {
            let measured = |inner| Measured { inner, costs: costs[side].clone(), spin: Duration::ZERO };
            let chain = if recovering {
                archive::restore_chain(store.clone(), &cfg, |bytes| {
                    restore_executor(&cfg, bytes, store.as_ref()).map(measured).map_err(|e| SessionError::Execution(e.to_string()))
                })?
                .0
            } else {
                let sources = SourceBindings::from_store(initial.policy(), store.as_ref())?;
                SessionChain::create(
                    cfg.clone(),
                    verifier.clone(),
                    measured(TransferExecutor::new(&cfg, initial, sources)?),
                    store.clone(),
                )?
            };
            chains.push(chain);
        }
        let chains: [Chain; 2] = chains.try_into().map_err(|_| "reward pair needs two ledgers")?;
        let prefix = if recovering { "recovered-" } else { "" };
        let origin = Instant::now();
        let evidence_commits = evidence::Commits::default();
        let evidence_worker = if plan.evidence == evidence::Mode::Disabled {
            None
        } else {
            Some(evidence::Worker::start(
                root,
                pair,
                std::array::from_fn(|i| chains[i].config().session_id),
                plan.evidence,
                store.clone(),
                evidence_commits.clone(),
                origin,
            )?)
        };
        Ok(Self {
            chains,
            costs,
            store,
            verifier,
            signer,
            origin,
            pair,
            batches: BufWriter::new(File::create(root.join(format!("{prefix}batches-{pair}.jsonl")))?),
            evidence_commits,
            evidence_worker,
        })
    }
    fn commit(&mut self, side: usize, phase: &str, request: BlockRequest) -> AnyResult<Step> {
        let count = request.frame_input.commands.len();
        *self.costs[side].lock().unwrap() = ExecutorCosts::default();
        let start = Instant::now();
        let prepared = self.chains[side].prepare_block(request)?;
        let prepare_ns = ns(start.elapsed());
        let simulation_ns =
            ns(self.costs[side].lock().unwrap().finished.ok_or("missing reward execution timing")?.duration_since(self.origin));
        if prepared.frame().resources.cycles != 0 {
            return Err("host reward executor returned VM cycles".into());
        }
        let issued = prepared.issued_handoffs().to_vec();
        let start = Instant::now();
        let SelectedConsensus::ProofOfAuthority(engine) = self.verifier.selected_consensus() else { unreachable!() };
        let proof = FinalityProof::ProofOfAuthority(engine.seal_from_signer(
            prepared.block().hash(),
            prepared.block().number,
            &self.signer,
        )?);
        let local_poa_sign_ns = ns(start.elapsed());
        let start = Instant::now();
        let head = self.chains[side].commit_finalised(prepared, proof)?;
        let verify_and_sync_commit_ns = ns(start.elapsed());
        let committed_ns = ns(self.origin.elapsed());
        let height = head.finalised_height.ok_or("reward commit without height")?;
        let start = Instant::now();
        let outbox = self.store.load_block(head.session_id, height)?.outbox;
        let messages = outbox.iter().map(|m| m.id).collect();
        for message in outbox {
            if self.evidence_worker.is_some() {
                self.evidence_commits.lock().unwrap().insert(message.id, committed_ns);
            } else {
                self.store.acknowledge_outbox(head.session_id, message.id)?;
            }
        }
        json_line(
            &mut self.batches,
            &serde_json::json!({
                "pair":self.pair,"session":self.pair * 2 + side as u32,"phase":phase,"height":height,"commands":count,
                "prepare_ns":prepare_ns,"execution":self.costs[side].lock().unwrap().clone(),"local_poa_sign_ns":local_poa_sign_ns,
                "verify_and_sync_commit_ns":verify_and_sync_commit_ns,
                "local_outbox_ack_ns":if self.evidence_worker.is_none() { ns(start.elapsed()) } else { 0 },
                "evidence_enqueue_ns":if self.evidence_worker.is_some() { ns(start.elapsed()) } else { 0 },"vm_cycles":0,
            }),
        )?;
        self.batches.flush()?;
        Ok(Step { simulation_ns, committed_ns, issued, messages })
    }
    fn retire(&mut self, handoffs: &[SessionHandoff]) -> AnyResult<Step> {
        let commands = handoffs.iter().map(|h| TransferCommand::Retire { handoff_id: h.id() }).collect();
        let mut req = request(&self.chains[0], commands)?;
        for h in handoffs {
            let stored = self.store.load_handoff(h.id())?.ok_or("consumed reward missing")?;
            let HandoffTerminal::Consumed { at } = stored.lifecycle.terminal else {
                return Err("reward has no canonical consumption".into());
            };
            req.handoff_actions.push(HandoffAction::Retire { handoff_id: h.id(), consumed_at: at });
        }
        self.commit(0, "retire", req)
    }
    fn reconcile_crash(&mut self) -> AnyResult<serde_json::Value> {
        let started = Instant::now();
        if self.chains[0].head().input_position != 1 || self.chains[1].head().input_position != 1 {
            return Err("reward crash did not leave exactly one issue and one intake".into());
        }
        let source = self.store.load_block(self.chains[0].config().session_id, 0)?;
        if source.issued_handoffs.len() != 1 {
            return Err("crashed reward lost its original handoff".into());
        }
        let command = TransferCommand::Intake { handoff_id: source.issued_handoffs[0].id() };
        let identity = command.identity(self.chains[1].config().commitment())?;
        let CommandStatus::Committed(receipt) = self.chains[1].command_status(&identity)? else {
            return Err("unknown credit outcome did not recover its original command".into());
        };
        let before = self.chains[1].head().clone();
        let mut duplicate = request(&self.chains[1], vec![command])?;
        duplicate.consume_handoffs.push(source.issued_handoffs[0].id());
        if self.chains[1].prepare_block(duplicate).is_ok() || self.chains[1].head() != &before {
            return Err("lost-response retry created another reward".into());
        }
        self.retire(&source.issued_handoffs)?;
        if self.chains[1].head() != &before {
            return Err("reconciliation changed the credited inventory head".into());
        }
        Ok(serde_json::json!({"elapsed_ns":ns(started.elapsed()),"original_receipt":receipt,
            "handoff":BlobId(source.issued_handoffs[0].id()),"duplicate_credit_rejected":true,
            "inventory_head_unchanged":true,"source_after_retirement":head_view(self.chains[0].head())?}))
    }
    fn issue_request(
        &self,
        plan: &RewardPlan,
        confirmed: u32,
        count: u32,
        policy: &TransferPolicy,
        owner: &TransferSigner,
        recipient: Hash32,
    ) -> AnyResult<BlockRequest> {
        let source = &self.chains[0];
        let target = self.chains[1].config();
        let mut intents = vec![];
        let source_id = SourceIdentity::from_config(source.config())?;
        for sequence in confirmed..confirmed + count {
            intents.push(owner.sign(
                TransferPayload {
                    deployment_id: policy.deployment_id,
                    uniqueness_domain: policy.uniqueness_domain,
                    kind: TransferKind::Scarce,
                    intake_schema: TransferKind::Scarce.intake_schema(),
                    source: source_id.clone(),
                    asset: ASSET,
                    owner: owner.actor(),
                    recipient,
                    source_version: u64::from(confirmed + sequence),
                    amount: 1,
                    components: COMPONENTS.to_vec(),
                    nonce: match_id(plan, self.pair, sequence),
                    target: TransferTarget::Session { session_id: target.session_id, genesis_commitment: target.commitment() },
                },
                u64::MAX,
                None,
            )?);
        }
        let commands = intents
            .iter()
            .map(|intent| verify_intent(intent).map(|(transfer_id, _)| TransferCommand::Issue { transfer_id }))
            .collect::<std::result::Result<_, _>>()?;
        let mut req = request(source, commands)?;
        req.handoffs = intents;
        Ok(req)
    }
    fn produce(&mut self, root: &Path, plan: &RewardPlan, recovering: bool) -> AnyResult<()> {
        let first = if recovering {
            let reconciliation = self.reconcile_crash()?;
            save(&root.join(format!("reconciled-credit-{}.json", self.pair)), &reconciliation)?;
            1
        } else {
            0
        };
        self.produce_from(root, plan, first, recovering)
    }
    fn produce_from(&mut self, root: &Path, plan: &RewardPlan, first: u32, recovering: bool) -> AnyResult<()> {
        let prefix = if recovering { "recovered-" } else { "" };
        let mut samples = BufWriter::new(File::create(root.join(format!("{prefix}rewards-{}.jsonl", self.pair)))?);
        let mut schedule = ProductionSchedule::new(plan.trigger.production(plan.window_us))?;
        let mut manual_due = Duration::from_micros(plan.window_us);
        let owner = owner()?;
        let recipient = recipient()?;
        let policy = inspect(&self.chains[0])?.policy().clone();
        // Recovery retirement has its own timing; resumed arrivals start here.
        let arrival_origin = self.origin.elapsed();
        let mut confirmed = first;
        let mut next = first;
        let mut admitted = VecDeque::new();
        while confirmed < plan.rewards {
            if self.origin.elapsed() > Duration::from_secs(3600) {
                return Err("reward workload exceeded one hour".into());
            }
            while next < plan.rewards && admitted.len() < 128 {
                let due = arrival_origin + Duration::from_micros(u64::from(next - first) * plan.arrival_us);
                if self.origin.elapsed() < due {
                    break;
                }
                admitted.push_back((ns(due), ns(self.origin.elapsed())));
                next += 1;
            }
            let now = self.origin.elapsed() - arrival_origin;
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
            let count = plan.batch.min(admitted.len()) as u32;
            let req = self.issue_request(plan, confirmed, count, &policy, &owner, recipient)?;
            let issued = self.commit(0, "issue", req)?;
            let mut publications = std::collections::BTreeMap::new();
            bind_messages(&mut publications, &issued.issued, &issued)?;
            let (cancelled, delivered): (Vec<_>, Vec<_>) =
                issued.issued.iter().enumerate().partition(|(i, _)| plan.cancelled(confirmed + *i as u32));
            let delivered: Vec<_> = delivered.into_iter().map(|(_, h)| h.clone()).collect();
            let cancelled: Vec<_> = cancelled.into_iter().map(|(_, h)| h.clone()).collect();
            let mut credited = None;
            let mut resolved_credit = None;
            let mut resolved_refund = None;
            if !delivered.is_empty() {
                let mut req =
                    request(&self.chains[1], delivered.iter().map(|h| TransferCommand::Intake { handoff_id: h.id() }).collect())?;
                req.consume_handoffs = delivered.iter().map(SessionHandoff::id).collect();
                let intake = self.commit(1, "intake", req)?;
                bind_messages(&mut publications, &delivered, &intake)?;
                credited = Some(intake.committed_ns);
                if plan.crash_after_credit && !recovering {
                    std::process::exit(76);
                }
                let retired = self.retire(&delivered)?;
                bind_messages(&mut publications, &delivered, &retired)?;
                resolved_credit = Some(retired.committed_ns);
            }
            if !cancelled.is_empty() {
                let mut req = request(
                    &self.chains[0],
                    cancelled
                        .iter()
                        .map(|h| owner.cancel(self.chains[0].config(), h).map_err(Into::into))
                        .collect::<AnyResult<_>>()?,
                )?;
                req.handoff_actions = cancelled.iter().map(|h| HandoffAction::Cancel { handoff_id: h.id() }).collect();
                let cancellation = self.commit(0, "cancel", req)?;
                bind_messages(&mut publications, &cancelled, &cancellation)?;
                let mut req =
                    request(&self.chains[0], cancelled.iter().map(|h| TransferCommand::Reclaim { handoff_id: h.id() }).collect())?;
                req.handoff_actions = cancelled.iter().map(|h| HandoffAction::Reclaim { handoff_id: h.id() }).collect();
                let reclaimed = self.commit(0, "reclaim", req)?;
                bind_messages(&mut publications, &cancelled, &reclaimed)?;
                resolved_refund = Some(reclaimed.committed_ns);
            }
            for (offset, handoff) in issued.issued.iter().enumerate() {
                let sequence = confirmed + offset as u32;
                let cancelled = plan.cancelled(sequence);
                let (scheduled_ns, admitted_ns) = admitted.pop_front().ok_or("reward admission disappeared")?;
                json_line(
                    &mut samples,
                    &serde_json::json!({
                        "pair":self.pair,"sequence":sequence,"match_id":BlobId(match_id(plan,self.pair,sequence)),
                        "handoff":BlobId(handoff.id()),"outcome":if cancelled { "refunded" } else { "credited" },
                        "outbox_messages":publications.get(&handoff.id()).ok_or("reward publication identities absent")?,
                        "scheduled_ns":scheduled_ns,"admitted_ns":admitted_ns,"simulation_finished_ns":issued.simulation_ns,
                        "issued_ns":issued.committed_ns,"credited_ns":if cancelled { None } else { credited },
                        "resolved_ns":if cancelled { resolved_refund } else { resolved_credit },"trigger":format!("{cause:?}"),
                    }),
                )?;
            }
            confirmed += count;
            samples.flush()?;
        }
        Ok(())
    }
}

fn inspect<E: TransitionExecutor, S: SessionStore>(chain: &SessionChain<E, S>) -> AnyResult<TransferState> {
    let port = |_: &InspectContext<'_>, bytes: &[u8], _: &[u8]| Ok(bytes.to_vec());
    let result = chain.inspect_head(&port, b"state")?;
    let state = TransferState::decode(&result.result)?;
    if state.root()? != chain.head().state_root {
        return Err("reward state is not committed by head".into());
    }
    Ok(state)
}
fn summary(state: &TransferState) -> AnyResult<serde_json::Value> {
    Ok(serde_json::json!({"snapshot":BlobId::of(&state.encode()?),"accounts":state.accounts(),
        "pending":state.pending().len(),"receipts":state.receipts().len()}))
}
fn check_pair<E: TransitionExecutor, S: SessionStore>(
    plan: &RewardPlan,
    pair: u32,
    chains: &[SessionChain<E, S>; 2],
    store: &S,
) -> AnyResult<[serde_json::Value; 2]> {
    let source = inspect(&chains[0])?;
    let target = inspect(&chains[1])?;
    let refunded = plan.refunded();
    let credited = u64::from(plan.rewards) - refunded;
    let a = source.account(ASSET, owner()?.actor()).ok_or("source reward account missing")?;
    let b = target.account(ASSET, recipient()?).ok_or("inventory reward account missing")?;
    if source.accounts().len() != 1
        || target.accounts().len() != 1
        || !source.pending().is_empty()
        || !target.pending().is_empty()
        || !source.receipts().is_empty()
        || target.receipts().len() as u64 != credited
        || a.available != refunded
        || b.available != credited
        || a.locked != 0
        || b.locked != 0
        || a.version != 2 * u64::from(plan.rewards)
        || b.version != credited - 1
        || a.components != COMPONENTS
        || b.components != COMPONENTS
        || chains[0].head().input_position != 2 * u64::from(plan.rewards) + refunded
        || chains[1].head().input_position != credited
    {
        return Err("reward balance, lock, receipt, version or command conservation failed".into());
    }
    let mut sequence = 0u32;
    let mut checked_commands = 0u64;
    for side in 0..2 {
        let chain = &chains[side];
        for height in 0..=chain.head().finalised_height.ok_or("reward ledger has no history")? {
            let record = store.load_block(chain.config().session_id, height)?;
            if plan.evidence != evidence::Mode::Disabled {
                evidence::check(store, chain.config().session_id, &record.outbox)?;
            }
            for (i, command) in record.frame_input.commands.iter().enumerate() {
                if chain.command_status(command)? != CommandStatus::Committed(CommandReceipt::for_record(&record, i)?) {
                    return Err("reward command lost its original canonical receipt".into());
                }
                checked_commands += 1;
            }
            for h in record.issued_handoffs {
                let payload = TransferPayload::decode(&h.payload)?;
                if side != 0
                    || sequence >= plan.rewards
                    || payload.nonce != match_id(plan, pair, sequence)
                    || payload.amount != 1
                    || payload.asset != ASSET
                    || payload.recipient != recipient()?
                {
                    return Err("issued reward differs from authorized match input".into());
                }
                let stored = store.load_handoff(h.id())?.ok_or("reward lifecycle missing")?;
                let receipt = target.receipts().iter().find(|r| r.handoff_id == h.id());
                match (&stored.lifecycle.terminal, plan.cancelled(sequence), receipt) {
                    (HandoffTerminal::Reclaimed { .. }, true, None) => {}
                    (HandoffTerminal::Retired { consumed_at, .. }, false, Some(receipt))
                        if consumed_at.session_id == chains[1].config().session_id
                            && receipt.amount == 1
                            && receipt.source_height == h.source_height
                            && receipt.source_state_root == h.source_state_root => {}
                    _ => return Err("reward terminal/receipt permits duplicate credit or refund".into()),
                }
                sequence += 1;
            }
        }
    }
    if sequence != plan.rewards || checked_commands != 3 * u64::from(plan.rewards) {
        // Cancellation adds a source command and removes one target command.
        return Err("reward input/command identity count differs".into());
    }
    Ok([summary(&source)?, summary(&target)?])
}
fn run_pair(
    root: &Path,
    plan: &RewardPlan,
    index: u32,
    program: Hash32,
    store: Arc<RocksSessionStore>,
    recovering: bool,
) -> AnyResult<Vec<SessionResult>> {
    let mut pair = Pair::open(root, plan, index, program, store, recovering)?;
    pair.produce(root, plan, recovering)?;
    finish_pair(pair, plan, recovering)
}
fn finish_pair(mut pair: Pair, plan: &RewardPlan, recovering: bool) -> AnyResult<Vec<SessionResult>> {
    let index = pair.pair;
    let production_elapsed_ns = ns(pair.origin.elapsed());
    if let Some(worker) = pair.evidence_worker.take() {
        worker.finish()?;
    }
    for chain in &pair.chains {
        if !pair.store.pending_outbox(chain.config().session_id, 1)?.is_empty() {
            return Err("reward outbox not fully delivered".into());
        }
    }
    let start = Instant::now();
    let states = check_pair(plan, index, &pair.chains, pair.store.as_ref())?;
    let result_validation_ns = ns(start.elapsed());
    pair.chains
        .iter()
        .zip(states)
        .enumerate()
        .map(|(side, (chain, state))| {
            Ok(SessionResult {
                index: index * 2 + side as u32,
                genesis: BlobId(chain.config().commitment()),
                head: head_view(chain.head())?,
                inputs: chain.head().input_position.try_into()?,
                vm_cycles: 0,
                unknown_outcomes: u64::from(recovering && side == 1),
                production_elapsed_ns,
                result_validation_ns,
                state,
            })
        })
        .collect()
}
fn produce(root: &Path, plan: &RewardPlan, recovering: bool) -> AnyResult<serde_json::Value> {
    let program = games::artifact_digest()?;
    let store = Arc::new(RocksSessionStore::open(root.join("canonical"))?);
    let barrier = Arc::new(Barrier::new(plan.pairs as usize));
    let mut threads = vec![];
    for index in plan.first_pair..plan.first_pair + plan.pairs {
        let (root, plan, store, barrier) = (root.to_owned(), plan.clone(), store.clone(), barrier.clone());
        threads.push(std::thread::spawn(move || {
            barrier.wait();
            run_pair(&root, &plan, index, program, store, recovering).map_err(|e| e.to_string())
        }));
    }
    let mut sessions = vec![];
    for thread in threads {
        sessions.extend(thread.join().map_err(|_| "reward worker panicked")??);
    }
    save(&root.join("sessions.json"), &sessions)?;
    Ok(serde_json::json!({"stage":if recovering { "resumed" } else { "produced" },"sessions":sessions,
        "storage":storage_measurement(&store)?,"reconciled_rewards":u64::from(recovering),"execution":"host-reference-transfer",
        "evidence_mode":plan.evidence,
        "scope":"authorized match-completion input; shared-store scarce reward; optional controlled loopback signed-blob provider; no game judgement or remote claim consensus"}))
}
pub(super) fn quota_run(root: &Path, plan: RewardPlan) -> AnyResult<serde_json::Value> {
    quota::run(root, plan)
}
pub(super) fn run_after_quota(root: &Path, plan: RewardPlan, marker: &Path) -> AnyResult<serde_json::Value> {
    let start = Instant::now();
    let fault = loop {
        if let Ok(fault) = load::<serde_json::Value>(marker) {
            if !fault["error"].as_str().is_some_and(|e| e.contains("No space left on device")) {
                return Err("independent worker did not observe the expected quota fault".into());
            }
            break fault;
        }
        if start.elapsed() > Duration::from_secs(30) {
            return Err("quota fault did not become active".into());
        }
        std::thread::sleep(Duration::from_millis(2));
    };
    let mut report = run(root, plan)?;
    report["quota_fault_observed_before_production"] = fault;
    Ok(report)
}
pub(super) fn run(root: &Path, plan: RewardPlan) -> AnyResult<serde_json::Value> {
    plan.validate()?;
    fs::create_dir(root)?;
    save(&root.join("plan.json"), &plan)?;
    produce(root, &plan, false)
}
fn recover<S: SessionStore>(root: &Path, plan: &RewardPlan, store: Arc<S>) -> AnyResult<serde_json::Value> {
    let expected: Vec<SessionResult> = load(&root.join("sessions.json"))?;
    if expected.len() != plan.pairs as usize * 2 {
        return Err("reward session count changed".into());
    }
    let program = games::artifact_digest()?;
    let mut sessions = vec![];
    for pair in plan.first_pair..plan.first_pair + plan.pairs {
        let mut chains = vec![];
        let mut recovery = vec![];
        for (cfg, _) in settings(plan, pair, program)? {
            let (chain, report) = archive::restore_chain(store.clone(), &cfg, |bytes| {
                restore_executor(&cfg, bytes, store.as_ref()).map_err(|e| SessionError::Execution(e.to_string()))
            })?;
            chains.push(chain);
            recovery.push(report);
        }
        let chains: [_; 2] = chains.try_into().map_err(|_| "reward recovery needs two ledgers")?;
        let states = check_pair(plan, pair, &chains, store.as_ref())?;
        for (side, (chain, state)) in chains.iter().zip(states).enumerate() {
            let index = pair * 2 + side as u32;
            let before = expected.iter().find(|s| s.index == index).ok_or("original reward session absent")?;
            if before.head != head_view(chain.head())? || before.state != state {
                return Err("reward head/state changed after restart".into());
            }
            let cfg = chain.config().clone();
            let replay_store = store.clone();
            let replay = archive::replay(chain, 0, move |bytes| {
                restore_executor(&cfg, bytes, replay_store.as_ref()).map_err(|e| SessionError::Replay(e.to_string()))
            })?;
            sessions
                .push(serde_json::json!({"index":index,"head":before.head,"state":state,"recovery":recovery[side],"replay":replay}));
        }
    }
    Ok(serde_json::json!({"sessions":sessions,"reconciled_rewards":0}))
}
pub(super) fn resume(op: &str, root: &Path) -> AnyResult<serde_json::Value> {
    let plan: RewardPlan = load(&root.join("plan.json"))?;
    plan.validate()?;
    match op {
        "quota-recover" => quota::recover(root, &plan),
        "recover" if root.join("canonical/CURRENT").is_file() => {
            if !root.join("sessions.json").exists() && plan.crash_after_credit {
                let resumed = produce(root, &plan, true)?;
                let mut report = recover(root, &plan, Arc::new(RocksSessionStore::open(root.join("canonical"))?))?;
                report["reconciled_rewards"] = resumed["reconciled_rewards"].clone();
                report["resumed_production"] = resumed;
                Ok(report)
            } else {
                recover(root, &plan, Arc::new(RocksSessionStore::open(root.join("canonical"))?))
            }
        }
        "archive" => archive::export_sessions(root, plan.rewards * 4 + 16, plan.pairs * 2, |index, program| {
            Ok(settings(&plan, index / 2, program)?[index as usize % 2].0.clone())
        }),
        "cold" if !root.join("canonical").exists() => {
            let (store, audit_ns, usage) = archive::open_archive(root)?;
            let program = BlobId(games::artifact_digest()?);
            for s in load::<Vec<SessionResult>>(&root.join("sessions.json"))? {
                let cfg = settings(&plan, s.index / 2, program.0)?[s.index as usize % 2].0.clone();
                archive::check_archive_session(&store, &cfg, &s, program)?;
            }
            let mut report = recover(root, &plan, store)?;
            report["archive_audit_ns"] = audit_ns.into();
            report["work"] = serde_json::to_value(usage)?;
            report["original_databases_present"] = false.into();
            Ok(report)
        }
        _ => Err("reward recovery needs existing databases; cold replay requires removal; expected recover|archive|cold".into()),
    }
}
