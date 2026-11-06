//! Development-key example. It operates only on the explicitly supplied local
//! RocksDB path. `resume` reconciles committed heads before issuing any retry.
use myelin_consensus::{Authority, CommitteeSigner, ConsensusConfig, FinalityProof, ProofOfAuthorityConfig, SelectedConsensus};
use myelin_session::{
    ApplicationProfile, ApplicationResourceEnvelope, ApplicationVmProfile, BlockRequest, CommitResourceLimits, FinalityVerifier,
    Hash32, SessionChain, SessionConfig, SessionError, SessionStore,
};
use myelin_session_runtime::RegisteredFinalityVerifier;
use myelin_session_store_rocksdb::RocksSessionStore;
use myelin_session_transfer::*;
use std::{path::Path, sync::Arc};

type AnyResult<T> = std::result::Result<T, Box<dyn std::error::Error>>;
type Chain = SessionChain<TransferExecutor, RocksSessionStore>;
const ASSET: AssetId = AssetId { namespace: [80; 32], object: [81; 32] };
fn hex(value: &[u8]) -> String {
    value.iter().map(|b| format!("{b:02x}")).collect()
}
fn consensus() -> AnyResult<(Arc<RegisteredFinalityVerifier>, CommitteeSigner)> {
    let signer = CommitteeSigner::new("development-validator", [7; 32])?;
    let v = RegisteredFinalityVerifier::new(ConsensusConfig::proof_of_authority(ProofOfAuthorityConfig {
        authorities: vec![Authority { id: signer.validator_id().into(), public_key: signer.ckb_public_key() }],
    }))?;
    Ok((Arc::new(v), signer))
}
fn config(v: &RegisteredFinalityVerifier, id: u8, state: &TransferState, program: Hash32) -> AnyResult<SessionConfig> {
    let d = v.descriptor();
    let mut cfg = SessionConfig {
        session_id: [id; 32],
        consensus_kind: d.consensus_kind,
        consensus_config_commitment: d.config_commitment,
        consensus_module_commitment: d.commitment(),
        consensus_wal_schema_hash: d.wal_schema_hash,
        application_profile: ApplicationProfile {
            application_id: "reference-transfer-example/v1".into(),
            command_identity_required: true,
            program_digest: program,
            input_schema_hash: input_schema(),
            state_codec_hash: state_codec(),
            logical_time_policy_hash: *blake3::hash(b"one-logical-step-per-transfer-command:v1").as_bytes(),
            entropy_policy_hash: *blake3::hash(b"no-randomness:v1").as_bytes(),
            vm: ApplicationVmProfile { spawn_ipc_required: false },
            resources: ApplicationResourceEnvelope {
                max_frame_inputs: 64,
                max_frame_input_bytes: 1_000_000,
                max_frame_cycles: 1_000_000,
                max_logical_time_span: 64,
                max_inspect_query_bytes: 1024,
                max_inspect_result_bytes: 4096,
            },
            court_profile_hash: *blake3::hash(b"host-replay-only:no-court:v1").as_bytes(),
            handoff_policy_hash: state.policy().intake_policy()?,
        },
        initial_state_root: state.root()?,
        initial_input_position: 0,
        initial_logical_time: 0,
        initial_timestamp_ms: 0,
        predecessor: None,
        max_block_transactions: 64,
        max_block_bytes: 1_000_000,
        max_future_drift_ms: 1000,
        commit_limits: CommitResourceLimits::default(),
        protocol_effects: myelin_session::ProtocolEffectsPolicy::Disabled,
    };
    state.policy().bind_config(&mut cfg)?;
    Ok(cfg)
}
fn open_chain(
    cfg: SessionConfig,
    initial: TransferState,
    v: Arc<RegisteredFinalityVerifier>,
    store: Arc<RocksSessionStore>,
) -> AnyResult<Chain> {
    match store.load_genesis(cfg.session_id) {
        Ok(existing) => {
            if existing.config != cfg {
                return Err("existing genesis differs; use its original binary, policy and mode".into());
            }
            let snapshot = store.load_checkpoint(cfg.session_id)?.state_snapshot;
            let executor = restore_executor(&cfg, &snapshot, store.as_ref())?;
            Ok(SessionChain::recover(cfg.session_id, &cfg.application_profile, v, executor, store)?)
        }
        Err(myelin_session::StoreError::NotFound(_)) => Ok(create_chain(cfg, initial, v, store)?),
        Err(e) => Err(e.into()),
    }
}
fn commit(chain: &mut Chain, v: &RegisteredFinalityVerifier, signer: &CommitteeSigner, req: BlockRequest) -> AnyResult<()> {
    let prepared = chain.prepare_block(req)?;
    let SelectedConsensus::ProofOfAuthority(engine) = v.selected_consensus() else { return Err("wrong example finality".into()) };
    let proof = FinalityProof::ProofOfAuthority(engine.seal_from_signer(prepared.block().hash(), prepared.block().number, signer)?);
    chain.commit_finalised(prepared, proof)?;
    Ok(())
}
fn req(chain: &Chain, commands: Vec<TransferCommand>) -> AnyResult<BlockRequest> {
    Ok(BlockRequest {
        transactions: vec![],
        frame_input: command_frame(chain.config(), chain.head().input_position, chain.head().logical_time, commands)?,
        scheduler_commitment: *blake3::hash(b"reference-transfer-sequential:v1").as_bytes(),
        data_commitments: vec![],
        timestamp_ms: 10,
        local_now_ms: 10,
        successor: None,
        handoffs: vec![],
        consume_handoffs: vec![],
        handoff_actions: vec![],
    })
}
fn resume(path: &Path, kind: TransferKind, program: Hash32, policy_target: bool, operation: &str) -> AnyResult<()> {
    if operation == "retire" && kind != TransferKind::Scarce {
        return Err("retire requires scarce mode".into());
    }
    let needs_issued = matches!(operation, "cancel" | "refund" | "retire" | "continue" | "unsupported-upgrade");
    if needs_issued && !path.exists() {
        return Err("lifecycle operation requires an existing issued transfer".into());
    }
    let store = Arc::new(RocksSessionStore::open(path)?);
    if needs_issued {
        store.load_genesis([3; 32])?;
        store.load_genesis([4; 32])?;
        if store.load_head([3; 32])?.finalised_height.is_none() {
            return Err("lifecycle operation cannot issue a new transfer".into());
        }
    }
    let (v, validator) = consensus()?;
    let owner = TransferSigner::new([3; 32])?;
    let recipient = TransferSigner::new([4; 32])?.actor();
    let policy = TransferPolicy {
        deployment_id: [1; 32],
        uniqueness_domain: [2; 32],
        kind,
        limits: TransferLimits::default(),
        accepted_sources: vec![],
    };
    let initial = TransferState::new(
        policy.clone(),
        vec![AssetAccount { asset: ASSET, owner: owner.actor(), available: 100, locked: 0, version: 0, components: vec![7, 8] }],
    )?;
    let source_cfg = config(&v, 3, &initial, program)?;
    let mut source = open_chain(source_cfg.clone(), initial, v.clone(), store.clone())?;
    let mut intake = policy.clone();
    intake.accepted_sources.push(SourceIdentity::from_config(&source_cfg)?);
    let target_initial = TransferState::new(intake, vec![])?;
    let target_cfg = config(&v, 4, &target_initial, program)?;
    let mut target = open_chain(target_cfg.clone(), target_initial, v.clone(), store.clone())?;
    let destination = if policy_target {
        TransferTarget::SharedStorePolicy { policy_commitment: target_cfg.application_profile.handoff_policy_hash }
    } else {
        TransferTarget::Session { session_id: target_cfg.session_id, genesis_commitment: target_cfg.commitment() }
    };
    if source.head().finalised_height.is_none() {
        let payload = TransferPayload {
            deployment_id: policy.deployment_id,
            uniqueness_domain: policy.uniqueness_domain,
            kind,
            intake_schema: kind.intake_schema(),
            source: SourceIdentity::from_config(&source_cfg)?,
            asset: ASSET,
            owner: owner.actor(),
            recipient,
            source_version: 0,
            amount: if kind == TransferKind::Scarce { 10 } else { 1 },
            components: vec![7, 8],
            nonce: [1; 32],
            target: destination.clone(),
        };
        let intent = owner.sign(payload, 100, None)?;
        let transfer_id = verify_intent(&intent)?.0;
        let mut request = req(&source, vec![TransferCommand::Issue { transfer_id }])?;
        request.handoffs.push(intent);
        commit(&mut source, &v, &validator, request)?;
    }
    if target.head().finalised_height.is_some_and(|h| h != 0) {
        return Err("unexpected history for this single-transfer example".into());
    }
    let source_record = store.load_block(source_cfg.session_id, 0)?;
    if source_record.issued_handoffs.len() != 1 {
        return Err("example source must issue exactly one handoff".into());
    }
    let issued_payload = TransferPayload::decode(&source_record.issued_handoffs[0].payload)?;
    if issued_payload.target != destination {
        return Err("existing transfer uses another target mode; reconcile the original intent".into());
    }
    let id = source_record.issued_handoffs[0].id();
    if operation == "issue" {
        return Ok(());
    }
    if let Some(next) = source.head().sealed_by_successor {
        let cfg = store.load_genesis(next)?.config;
        let snapshot = store.load_checkpoint(next)?.state_snapshot;
        let executor = restore_executor(&cfg, &snapshot, store.as_ref())?;
        source = SessionChain::recover(next, &cfg.application_profile, v.clone(), executor, store.clone())?;
        if operation == "continue" {
            return Ok(());
        }
    }
    if matches!(operation, "continue" | "unsupported-upgrade") {
        let state = TransferState::decode(&store.load_checkpoint(source.config().session_id)?.state_snapshot)?;
        let mut next = source.config().clone();
        next.session_id = [if operation == "continue" { 5 } else { 6 }; 32];
        next.initial_state_root = source.head().state_root;
        next.initial_input_position = source.head().input_position.checked_add(1).ok_or("input cursor overflow")?;
        next.initial_logical_time = source.head().logical_time.checked_add(1).ok_or("logical time overflow")?;
        next.initial_timestamp_ms = 10;
        next.predecessor = Some(myelin_session::PredecessorReference {
            session_id: source.config().session_id,
            final_height: source.head().finalised_height.map_or(Some(0), |h| h.checked_add(1)).ok_or("height overflow")?,
            final_state_root: source.head().state_root,
            application_profile_commitment: source.config().application_profile.commitment(),
        });
        if operation == "unsupported-upgrade" {
            next.application_profile.program_digest = *blake3::hash(b"unsupported reference rule change").as_bytes();
        }
        let approval = owner.continue_session(source.config(), &state, &next)?;
        let mut request =
            req(&source, vec![TransferCommand::Continue { target_genesis: next.commitment(), approvals: vec![approval] }])?;
        request.successor = Some(next);
        commit(&mut source, &v, &validator, request)?;
        return Ok(());
    }
    if matches!(operation, "cancel" | "refund" | "retire") {
        let status = store.load_handoff(id)?.ok_or("handoff index missing; reconcile")?;
        let terminal = &status.lifecycle.terminal;
        use myelin_session::{HandoffAction, HandoffTerminal};
        let operation = match (operation, terminal) {
            ("cancel", HandoffTerminal::Issued) => {
                Some((owner.cancel(source.config(), &status.handoff)?, HandoffAction::Cancel { handoff_id: id }))
            }
            ("refund", HandoffTerminal::Cancelled { .. }) => {
                Some((TransferCommand::Reclaim { handoff_id: id }, HandoffAction::Reclaim { handoff_id: id }))
            }
            ("retire", HandoffTerminal::Consumed { at }) => {
                Some((TransferCommand::Retire { handoff_id: id }, HandoffAction::Retire { handoff_id: id, consumed_at: at.clone() }))
            }
            ("retire", HandoffTerminal::Retired { .. }) => None,
            ("cancel", HandoffTerminal::Cancelled { .. } | HandoffTerminal::Reclaimed { .. })
            | ("refund", HandoffTerminal::Reclaimed { .. }) => None,
            _ => return Err("lifecycle conflict: refund requires cancellation; retirement requires consumption".into()),
        };
        if let Some((command, action)) = operation {
            let mut request = req(&source, vec![command])?;
            request.handoff_actions.push(action);
            commit(&mut source, &v, &validator, request)?;
        }
        return Ok(());
    }
    if target.head().finalised_height.is_none() {
        let status = store.load_handoff(id)?.ok_or("source handoff index missing; reconcile rather than reissue")?;
        if !status.is_available() {
            return Err("handoff already consumed without expected target head; reconcile".into());
        }
        let mut request = req(&target, vec![TransferCommand::Intake { handoff_id: id }])?;
        request.consume_handoffs.push(id);
        commit(&mut target, &v, &validator, request)?;
    }
    Ok(())
}
fn replay(path: &Path, program: Hash32) -> AnyResult<serde_json::Value> {
    let store = Arc::new(RocksSessionStore::open(path)?);
    let (v, _) = consensus()?;
    let mut reports = vec![];
    let mut ids = vec![[3; 32], [4; 32]];
    let active_source = if let Some(next) = store.load_head([3; 32])?.sealed_by_successor {
        ids.push(next);
        next
    } else {
        [3; 32]
    };
    for id in ids {
        let cfg = store.load_genesis(id)?.config;
        if cfg.application_profile.program_digest != program {
            return Err("historical replay requires the exact original example binary".into());
        }
        let snapshot = store.load_checkpoint(cfg.session_id)?.state_snapshot;
        let executor = restore_executor(&cfg, &snapshot, store.as_ref())?;
        let chain = SessionChain::recover(cfg.session_id, &cfg.application_profile, v.clone(), executor, store.clone())?;
        let receipt = chain
            .head()
            .finalised_height
            .map(|end| {
                let replay_config = cfg.clone();
                let replay_store = store.clone();
                chain.replay_range(0, end, move |b| {
                    restore_executor(&replay_config, b, replay_store.as_ref()).map_err(|e| SessionError::Replay(e.to_string()))
                })
            })
            .transpose()?;
        let state = TransferState::decode(&snapshot)?;
        reports.push(serde_json::json!({"session_id":hex(&cfg.session_id),"genesis":hex(&cfg.commitment()),"state_root":hex(&chain.head().state_root),
            "effect_policy":cfg.protocol_effects,"policy":state.policy(),"available":state.accounts().iter().map(|a| a.available).sum::<u64>(),
            "locked":state.accounts().iter().map(|a| a.locked).sum::<u64>(),"pending_locks":state.pending().len(),"received_receipts":state.receipts().len(),
            "height":chain.head().finalised_height,"sealed_by_successor":chain.head().sealed_by_successor.map(|id|hex(&id)),
            "replay_receipt":receipt.map(|r|hex(&r.commitment())),"pending_outbox":store.pending_outbox(cfg.session_id,100)?.len()}));
    }
    let source_record = store.load_block([3; 32], 0)?;
    let handoff = source_record.issued_handoffs[0].id();
    let issued_payload = TransferPayload::decode(&source_record.issued_handoffs[0].payload)?;
    let status = store.load_handoff(handoff)?.ok_or("missing handoff")?;
    Ok(serde_json::json!({"format":"myelin-reference-transfer-example-v1","program_digest":hex(&program),"sessions":reports,
        "active_source_session":hex(&active_source),
        "target_mode":match issued_payload.target {TransferTarget::Session{..}=>"exact",TransferTarget::SharedStorePolicy{..}=>"policy"},
        "issued_by_session":hex(&source_record.block.session_id),"issued_at_height":source_record.block.number,"issuing_block":hex(&source_record.block.hash()),
        "handoff_id":hex(&handoff),"consumed_by":status.consumption().map(|at| at.session_id).map(|id|hex(&id)),"consumed_at_height":status.consumption().map(|at| at.height),
        "consuming_block":status.consumption().map(|at| at.block_hash).map(|hash|hex(&hash)),"lifecycle":status.lifecycle,
        "capabilities":{"shared_store_lock_and_intake":true,"owner_signature_verification":true,"copyable_receipts":true,"cancellation":issued_payload.kind==TransferKind::Scarce,"refund":issued_payload.kind==TransferKind::Scarce,
        "consumed_lock_retirement":issued_payload.kind==TransferKind::Scarce && store.load_genesis([3;32])?.config.application_profile.input_schema_hash==input_schema(),
        "owner_authorized_continuation":true,"rule_upgrade":false,"cross_store_atomicity":false,"ckb_court":false}}))
}
fn main() -> AnyResult<()> {
    let args = std::env::args().skip(1).collect::<Vec<_>>();
    if args.len() < 2
        || args.len() > 4
        || !matches!(
            args[0].as_str(),
            "resume" | "issue" | "cancel" | "refund" | "retire" | "continue" | "unsupported-upgrade" | "replay"
        )
    {
        return Err(
            "usage: shared_store_transfer <resume|issue|cancel|refund|retire|continue|unsupported-upgrade|replay> <explicit-store-path> [scarce|copyable] [exact|policy]"
                .into(),
        );
    }
    #[cfg(target_os = "linux")]
    let executable = std::path::PathBuf::from("/proc/self/exe");
    #[cfg(not(target_os = "linux"))]
    let executable = std::env::current_exe()?;
    let program = *blake3::hash(&std::fs::read(executable)?).as_bytes();
    let path = Path::new(&args[1]);
    if args[0] != "replay" {
        let kind = match args.get(2).map(String::as_str).unwrap_or("scarce") {
            "scarce" => TransferKind::Scarce,
            "copyable" => TransferKind::CopyableReceipt,
            _ => return Err("mode must be scarce or copyable".into()),
        };
        let policy_target = match args.get(3).map(String::as_str).unwrap_or("exact") {
            "exact" => false,
            "policy" => true,
            _ => return Err("target mode must be exact or policy".into()),
        };
        resume(path, kind, program, policy_target, &args[0])?;
    }
    println!("{}", serde_json::to_string_pretty(&replay(path, program)?)?);
    Ok(())
}
