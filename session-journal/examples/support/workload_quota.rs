//! Real filesystem exhaustion, including writes within preallocated WAL space.
//! The driver owns the bounded ext4/FUSE image; no host filesystem is filled.
use super::*;
use std::os::unix::fs::MetadataExt;

#[derive(Clone, Copy, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
enum Phase {
    Issue,
    Intake,
    Retire,
}
impl Phase {
    fn side(self) -> usize {
        usize::from(matches!(self, Self::Intake))
    }
    fn name(self) -> &'static str {
        match self {
            Self::Issue => "issue",
            Self::Intake => "intake",
            Self::Retire => "retire",
        }
    }
}
#[derive(Serialize, Deserialize)]
struct Fault {
    baseline: myelin_session_store_rocksdb::SnapshotInventory,
    command: CommandIdentity,
    phase: Phase,
    sequence: u32,
    handoff: Option<Hash32>,
    acknowledged: Vec<CommandReceipt>,
    failed_writes: Vec<serde_json::Value>,
    filler_bytes: u64,
}
fn fill(root: &Path) -> AnyResult<u64> {
    let volume = root.join("canonical");
    if fs::symlink_metadata(&volume)?.file_type().is_symlink() || volume.metadata()?.dev() == root.metadata()?.dev() {
        return Err("quota fault requires a distinct mounted filesystem".into());
    }
    let mut file = File::options().create_new(true).write(true).open(volume.join("quota-filler"))?;
    let mut written = 0u64;
    let chunk = [0x5a; 64 * 1024];
    while written < 64 * 1024 * 1024 {
        match file.write(&chunk) {
            Ok(0) => return Err("quota fill returned zero without ENOSPC".into()),
            Ok(count) => written += count as u64,
            Err(error) if error.raw_os_error() == Some(28) => return Ok(written),
            Err(error) => return Err(error.into()),
        }
    }
    Err("quota filesystem did not enforce its finite capacity".into())
}
fn request_for(pair: &Pair, plan: &RewardPlan, sequence: u32, phase: Phase, handoff: Option<Hash32>) -> AnyResult<BlockRequest> {
    if matches!(phase, Phase::Issue) {
        let policy = inspect(&pair.chains[0])?.policy().clone();
        return pair.issue_request(plan, sequence, 1, &policy, &owner()?, recipient()?);
    }
    let handoff = handoff.ok_or("reward phase has no source handoff")?;
    let command = match phase {
        Phase::Intake => TransferCommand::Intake { handoff_id: handoff },
        Phase::Retire => TransferCommand::Retire { handoff_id: handoff },
        Phase::Issue => unreachable!(),
    };
    let mut req = request(&pair.chains[phase.side()], vec![command])?;
    if matches!(phase, Phase::Intake) {
        req.consume_handoffs.push(handoff);
    } else {
        let at = match pair.store.load_handoff(handoff)?.ok_or("reward handoff disappeared")?.lifecycle.terminal {
            HandoffTerminal::Consumed { at } | HandoffTerminal::Retired { consumed_at: at, .. } => at,
            _ => return Err("retirement has no canonical intake".into()),
        };
        req.handoff_actions.push(HandoffAction::Retire { handoff_id: handoff, consumed_at: at });
    }
    Ok(req)
}
// Success can consume reserved WAL blocks even after statvfs reports no free
// space. Continue real commands; never assume the next append must fail.
fn attempt(pair: &mut Pair, phase: Phase, req: BlockRequest) -> AnyResult<(bool, serde_json::Value)> {
    let side = phase.side();
    let before = pair.chains[side].head().clone();
    let prepared = pair.chains[side].prepare_block(req)?;
    let SelectedConsensus::ProofOfAuthority(engine) = pair.verifier.selected_consensus() else { unreachable!() };
    let proof =
        FinalityProof::ProofOfAuthority(engine.seal_from_signer(prepared.block().hash(), prepared.block().number, &pair.signer)?);
    let start = Instant::now();
    match pair.chains[side].commit_finalised(prepared, proof) {
        Ok(head) => {
            let record = pair.store.load_block(head.session_id, head.finalised_height.ok_or("committed reward has no height")?)?;
            Ok((
                true,
                serde_json::json!({"phase":phase,"elapsed_ns":ns(start.elapsed()),"receipt":CommandReceipt::for_record(&record,0)?}),
            ))
        }
        Err(error) => {
            let elapsed_ns = ns(start.elapsed());
            if !matches!(error, SessionError::Store(StoreError::Backend(_))) || !error.to_string().contains("No space left on device")
            {
                return Err(format!("expected a real RocksDB ENOSPC write error, got {error}").into());
            }
            if pair.chains[side].head() != &before || pair.store.load_head(before.session_id)? != before {
                return Err("failed write changed in-memory or stored canonical head".into());
            }
            Ok((
                false,
                serde_json::json!({"pair":pair.pair,"phase":phase,"side":side,"elapsed_ns":elapsed_ns,"error":error.to_string(),"head":head_view(&before)?}),
            ))
        }
    }
}
fn latest_handoff(pair: &Pair) -> AnyResult<Hash32> {
    let head = pair.chains[0].head();
    let record = pair.store.load_block(head.session_id, head.finalised_height.ok_or("source has no issued reward")?)?;
    if record.issued_handoffs.len() != 1 {
        return Err("expected one source issuance".into());
    }
    Ok(record.issued_handoffs[0].id())
}
pub(super) fn run(root: &Path, plan: RewardPlan) -> AnyResult<serde_json::Value> {
    plan.validate()?;
    if plan.first_pair != 0 || plan.evidence != evidence::Mode::Disabled || plan.crash_after_credit || plan.cancel_every != 0 {
        return Err("quota drill cannot combine another fault".into());
    }
    save(&root.join("plan.json"), &plan)?;
    let program = games::artifact_digest()?;
    let store = Arc::new(RocksSessionStore::open(root.join("canonical"))?);
    let mut pairs =
        (0..plan.pairs).map(|i| Pair::open(root, &plan, i, program, store.clone(), false)).collect::<AnyResult<Vec<_>>>()?;
    let req = request_for(&pairs[0], &plan, 0, Phase::Issue, None)?;
    pairs[0].commit(0, "issue", req)?;
    let first = store.load_block(pairs[0].chains[0].config().session_id, 0)?;
    let mut acknowledged = vec![CommandReceipt::for_record(&first, 0)?];
    let mut handoff = Some(latest_handoff(&pairs[0])?);
    let filler_bytes = fill(root)?;
    let mut log = File::create(root.join("quota-commits.jsonl"))?;
    let mut failed = None;
    'work: for sequence in 0..plan.rewards {
        for phase in [Phase::Issue, Phase::Intake, Phase::Retire] {
            if sequence == 0 && matches!(phase, Phase::Issue) {
                continue;
            }
            let req = request_for(&pairs[0], &plan, sequence, phase, handoff)?;
            let command = req.frame_input.commands[0].clone();
            let baseline = storage_inventory(&store)?;
            let (success, observation) = attempt(&mut pairs[0], phase, req)?;
            json_line(&mut log, &serde_json::json!({"sequence":sequence,"success":success,"observation":observation}))?;
            log.sync_all()?;
            if !success {
                failed = Some(Fault {
                    baseline,
                    command,
                    phase,
                    sequence,
                    handoff,
                    acknowledged,
                    failed_writes: vec![observation],
                    filler_bytes,
                });
                break 'work;
            }
            acknowledged.push(serde_json::from_value(observation["receipt"].clone())?);
            if matches!(phase, Phase::Issue) {
                handoff = Some(latest_handoff(&pairs[0])?);
            }
            if matches!(phase, Phase::Retire) {
                handoff = None;
            }
        }
    }
    let mut fault = failed.ok_or("bounded workload did not exhaust RocksDB's reserved space")?;
    save(&root.join("fault-active.json"), &fault.failed_writes[0])?;
    for pair in pairs.iter_mut().skip(1) {
        let req = request_for(pair, &plan, 0, Phase::Issue, None)?;
        let (success, observation) = attempt(pair, Phase::Issue, req)?;
        if success {
            return Err("another shared-store write unexpectedly bypassed exhausted WAL".into());
        }
        fault.failed_writes.push(observation);
    }
    if storage_inventory(&store)? != fault.baseline
        || pairs[0].chains[fault.phase.side()].command_status(&fault.command)? != CommandStatus::Missing
    {
        return Err("failed writes changed receipts, locks, outbox or other canonical keys".into());
    }
    save(&root.join("quota-fault.json"), &fault)?;
    Ok(serde_json::json!({"stage":"quota-write-failed","fault":fault,"unchanged_inventory":true,"vm_cycles":0}))
}
pub(super) fn recover(root: &Path, plan: &RewardPlan) -> AnyResult<serde_json::Value> {
    let start = Instant::now();
    let fault: Fault = load(&root.join("quota-fault.json"))?;
    if root.join("canonical/quota-filler").exists() {
        return Err("quota space has not been released".into());
    }
    let store = Arc::new(RocksSessionStore::open(root.join("canonical"))?);
    let program = games::artifact_digest()?;
    let mut pair = Pair::open(root, plan, 0, program, store.clone(), true)?;
    for receipt in &fault.acknowledged {
        if store.load_command_receipt(receipt.session_id, receipt.command.id)?.as_ref() != Some(receipt) {
            return Err("quota recovery lost or changed an acknowledged receipt".into());
        }
    }
    let side = fault.phase.side();
    let was_committed = match pair.chains[side].command_status(&fault.command)? {
        CommandStatus::Missing => {
            if storage_inventory(&store)? != fault.baseline {
                return Err("missing command recovered changed canonical keys".into());
            }
            let req = request_for(&pair, plan, fault.sequence, fault.phase, fault.handoff)?;
            if req.frame_input.commands != [fault.command.clone()] {
                return Err("quota retry changed original identity".into());
            }
            pair.commit(side, fault.phase.name(), req)?;
            false
        }
        CommandStatus::Committed(receipt) => {
            let before = fault.baseline.heads.iter().find(|h| h.session_id == receipt.session_id).ok_or("fault session absent")?;
            let expected_height = before.finalised_height.map_or(0, |h| h + 1);
            let record = store.load_block(receipt.session_id, expected_height)?;
            if record.frame_input.commands != [fault.command.clone()]
                || receipt != CommandReceipt::for_record(&record, 0)?
                || pair.chains[side].head().finalised_height != Some(expected_height)
            {
                return Err("unknown outcome recovered a different command".into());
            }
            true
        }
        _ => return Err("quota recovery has an identity conflict".into()),
    };
    let handoff = if matches!(fault.phase, Phase::Issue) { Some(latest_handoff(&pair)?) } else { fault.handoff };
    if matches!(fault.phase, Phase::Issue) {
        let req = request_for(&pair, plan, fault.sequence, Phase::Intake, handoff)?;
        pair.commit(1, "intake", req)?;
    }
    if !matches!(fault.phase, Phase::Retire) {
        let req = request_for(&pair, plan, fault.sequence, Phase::Retire, handoff)?;
        pair.commit(0, "retire", req)?;
    }
    let heads = pair.chains.each_ref().map(|c| c.head().clone());
    let duplicate = request_for(&pair, plan, fault.sequence, fault.phase, handoff)?;
    if pair.chains[side].prepare_block(duplicate).is_ok() || pair.chains.iter().zip(&heads).any(|(c, h)| c.head() != h) {
        return Err("quota retry allowed a second reward or changed the head".into());
    }
    for chain in &pair.chains {
        loop {
            let pending = store.pending_outbox(chain.config().session_id, 128)?;
            if pending.is_empty() {
                break;
            }
            for delivery in pending {
                store.acknowledge_outbox(chain.config().session_id, delivery.message.id)?;
            }
        }
    }
    save(
        &root.join("quota-reconciled.json"),
        &serde_json::json!({"elapsed_ns":ns(start.elapsed()),"failed_phase":fault.phase,
        "failed_sequence":fault.sequence,"original_command":fault.command,"command_was_durable_before_retry":was_committed,
        "acknowledged_receipts_preserved":fault.acknowledged.len(),"duplicate_rejected":true}),
    )?;
    pair.produce_from(root, plan, fault.sequence + 1, true)?;
    let mut sessions = finish_pair(pair, plan, true)?;
    for index in 1..plan.pairs {
        let mut pair = Pair::open(root, plan, index, program, store.clone(), true)?;
        pair.produce_from(root, plan, 0, true)?;
        sessions.extend(finish_pair(pair, plan, false)?);
    }
    save(&root.join("sessions.json"), &sessions)?;
    Ok(
        serde_json::json!({"stage":"quota-recovered","sessions":sessions,"storage":storage_inventory(&store)?,"reconciled_rewards":1,"vm_cycles":0}),
    )
}
