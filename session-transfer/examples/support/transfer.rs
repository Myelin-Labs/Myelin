use myelin_consensus::{Authority, CommitteeSigner, ConsensusConfig, FinalityProof, ProofOfAuthorityConfig, SelectedConsensus};
use myelin_session::{
    ApplicationProfile, ApplicationResourceEnvelope, ApplicationVmProfile, BlockRequest, CommitResourceLimits, FinalityVerifier,
    Hash32, SessionChain, SessionConfig, SessionStore,
};
use myelin_session_runtime::RegisteredFinalityVerifier;
use myelin_session_store_rocksdb::RocksSessionStore;
use myelin_session_transfer::*;
use std::sync::Arc;

pub type AnyResult<T> = std::result::Result<T, Box<dyn std::error::Error>>;
pub type Chain = SessionChain<TransferExecutor, RocksSessionStore>;
pub const ASSET: AssetId = AssetId { namespace: [80; 32], object: [81; 32] };
pub fn hex(value: &[u8]) -> String {
    value.iter().map(|b| format!("{b:02x}")).collect()
}
pub fn consensus(name: &str, secret: [u8; 32]) -> AnyResult<(Arc<RegisteredFinalityVerifier>, CommitteeSigner)> {
    let signer = CommitteeSigner::new(name, secret)?;
    let v = RegisteredFinalityVerifier::new(ConsensusConfig::proof_of_authority(ProofOfAuthorityConfig {
        authorities: vec![Authority { id: signer.validator_id().into(), public_key: signer.ckb_public_key() }],
    }))?;
    Ok((Arc::new(v), signer))
}
pub fn config(v: &RegisteredFinalityVerifier, id: u8, state: &TransferState, program: Hash32) -> AnyResult<SessionConfig> {
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
pub fn open_chain(
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
pub fn commit(chain: &mut Chain, v: &RegisteredFinalityVerifier, signer: &CommitteeSigner, req: BlockRequest) -> AnyResult<()> {
    let prepared = chain.prepare_block(req)?;
    let SelectedConsensus::ProofOfAuthority(engine) = v.selected_consensus() else { return Err("wrong example finality".into()) };
    let proof = FinalityProof::ProofOfAuthority(engine.seal_from_signer(prepared.block().hash(), prepared.block().number, signer)?);
    chain.commit_finalised(prepared, proof)?;
    Ok(())
}
pub fn req(chain: &Chain, commands: Vec<TransferCommand>) -> AnyResult<BlockRequest> {
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
