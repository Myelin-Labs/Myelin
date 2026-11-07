use myelin_consensus::{Authority, CommitteeSigner, ConsensusConfig, FinalityProof, ProofOfAuthorityConfig, SelectedConsensus};
use myelin_session::{
    ApplicationProfile, ApplicationResourceEnvelope, ApplicationVmProfile, BlockRequest, CommitResourceLimits, FinalityVerifier,
    HandoffIntent, Hash32, PreparedBlock, SessionChain, SessionConfig, SessionStore,
};
use myelin_session_runtime::RegisteredFinalityVerifier;
use myelin_session_store_rocksdb::RocksSessionStore;
use myelin_session_transfer::*;
use std::sync::Arc;

pub type Chain = SessionChain<TransferExecutor, RocksSessionStore>;
pub const ASSET: AssetId = AssetId { namespace: [80; 32], object: [81; 32] };
pub fn signer() -> TransferSigner {
    TransferSigner::new([3; 32]).unwrap()
}
pub fn recipient() -> Hash32 {
    TransferSigner::new([4; 32]).unwrap().actor()
}
pub fn policy(kind: TransferKind) -> TransferPolicy {
    TransferPolicy {
        coordination: myelin_session_transfer::TransferCoordination::SharedStore,
        deployment_id: [1; 32],
        uniqueness_domain: [2; 32],
        kind,
        limits: TransferLimits::default(),
        accepted_sources: vec![],
    }
}
pub fn state(policy: TransferPolicy) -> TransferState {
    TransferState::new(
        policy,
        vec![AssetAccount { asset: ASSET, owner: signer().actor(), available: 100, locked: 0, version: 0, components: vec![7, 8] }],
    )
    .unwrap()
}
pub fn consensus() -> (Arc<RegisteredFinalityVerifier>, CommitteeSigner) {
    let signer = CommitteeSigner::new("transfer-test-validator", [7; 32]).unwrap();
    let verifier = RegisteredFinalityVerifier::new(ConsensusConfig::proof_of_authority(ProofOfAuthorityConfig {
        authorities: vec![Authority { id: signer.validator_id().into(), public_key: signer.ckb_public_key() }],
    }))
    .unwrap();
    (Arc::new(verifier), signer)
}
pub fn program_digest() -> Hash32 {
    static DIGEST: std::sync::OnceLock<Hash32> = std::sync::OnceLock::new();
    *DIGEST.get_or_init(|| {
        use std::io::Read;
        let mut file = std::fs::File::open(std::env::current_exe().unwrap()).unwrap();
        let mut hasher = blake3::Hasher::new();
        let mut buffer = [0; 65536];
        loop {
            let count = file.read(&mut buffer).unwrap();
            if count == 0 {
                break;
            }
            hasher.update(&buffer[..count]);
        }
        *hasher.finalize().as_bytes()
    })
}
pub fn config(v: &RegisteredFinalityVerifier, id: u8, state: &TransferState) -> SessionConfig {
    let d = v.descriptor();
    let mut cfg = SessionConfig {
        session_id: [id; 32],
        consensus_kind: d.consensus_kind,
        consensus_config_commitment: d.config_commitment,
        consensus_module_commitment: d.commitment(),
        consensus_wal_schema_hash: d.wal_schema_hash,
        application_profile: ApplicationProfile {
            application_id: "reference-transfer-test".into(),
            command_identity_required: true,
            program_digest: program_digest(),
            input_schema_hash: [12; 32],
            state_codec_hash: [13; 32],
            logical_time_policy_hash: [14; 32],
            entropy_policy_hash: [15; 32],
            vm: ApplicationVmProfile { spawn_ipc_required: false },
            resources: ApplicationResourceEnvelope {
                max_frame_inputs: 1000,
                max_frame_input_bytes: 1_000_000,
                max_frame_cycles: 1_000_000,
                max_logical_time_span: 1000,
                max_inspect_query_bytes: 1024,
                max_inspect_result_bytes: 4096,
            },
            court_profile_hash: [16; 32],
            handoff_policy_hash: [17; 32],
        },
        initial_state_root: state.root().unwrap(),
        initial_input_position: 0,
        initial_logical_time: 0,
        initial_timestamp_ms: 0,
        predecessor: None,
        max_block_transactions: 100,
        max_block_bytes: 1_000_000,
        max_future_drift_ms: 1000,
        commit_limits: CommitResourceLimits::default(),
        protocol_effects: myelin_session::ProtocolEffectsPolicy::Disabled,
    };
    state.policy().bind_config(&mut cfg).unwrap();
    cfg
}
pub fn payload(
    cfg: &SessionConfig,
    policy: &TransferPolicy,
    target: TransferTarget,
    nonce: u8,
    version: u64,
    amount: u64,
) -> TransferPayload {
    TransferPayload {
        deployment_id: policy.deployment_id,
        uniqueness_domain: policy.uniqueness_domain,
        kind: policy.kind,
        intake_schema: policy.kind.intake_schema(),
        source: SourceIdentity::from_config(cfg).unwrap(),
        asset: ASSET,
        owner: signer().actor(),
        recipient: recipient(),
        source_version: version,
        amount,
        components: vec![7, 8],
        nonce: [nonce; 32],
        target,
    }
}
pub fn request(chain: &Chain, commands: Vec<TransferCommand>, intents: Vec<HandoffIntent>, consumed: Vec<Hash32>) -> BlockRequest {
    BlockRequest {
        transactions: vec![],
        frame_input: command_frame(chain.config(), chain.head().input_position, chain.head().logical_time, commands).unwrap(),
        scheduler_commitment: [5; 32],
        data_commitments: vec![],
        timestamp_ms: chain.head().timestamp_ms.max(10),
        local_now_ms: 10,
        successor: None,
        handoffs: intents,
        consume_handoffs: consumed,
        handoff_actions: vec![],
    }
}
pub fn issue_request(chain: &Chain, intents: Vec<HandoffIntent>) -> BlockRequest {
    let commands = intents.iter().map(|i| TransferCommand::Issue { transfer_id: verify_intent(i).unwrap().0 }).collect();
    request(chain, commands, intents, vec![])
}
pub fn intake_request(chain: &Chain, ids: Vec<Hash32>) -> BlockRequest {
    request(chain, ids.iter().map(|id| TransferCommand::Intake { handoff_id: *id }).collect(), vec![], ids)
}
pub fn proof(v: &RegisteredFinalityVerifier, signer: &CommitteeSigner, p: &PreparedBlock<TransferExecutor>) -> FinalityProof {
    let SelectedConsensus::ProofOfAuthority(engine) = v.selected_consensus() else { unreachable!() };
    FinalityProof::ProofOfAuthority(engine.seal_from_signer(p.block().hash(), p.block().number, signer).unwrap())
}
pub fn commit(chain: &mut Chain, v: &RegisteredFinalityVerifier, signer: &CommitteeSigner, request: BlockRequest) -> Vec<Hash32> {
    let p = chain.prepare_block(request).unwrap();
    let ids = p.issued_handoffs().iter().map(|h| h.id()).collect();
    let proof = proof(v, signer, &p);
    chain.commit_finalised(p, proof).unwrap();
    ids
}
pub fn stored_state(store: &RocksSessionStore, id: Hash32) -> TransferState {
    TransferState::decode(&store.load_checkpoint(id).unwrap().state_snapshot).unwrap()
}
pub fn replay(chain: &Chain, store: &Arc<RocksSessionStore>) {
    let cfg = chain.config().clone();
    let store = store.clone();
    chain
        .replay_range(0, chain.head().finalised_height.unwrap(), move |b| {
            restore_executor(&cfg, b, store.as_ref()).map_err(|e| myelin_session::SessionError::Replay(e.to_string()))
        })
        .unwrap();
}

pub fn continuation_request(chain: &Chain, store: &RocksSessionStore, id: u8, owners: &[TransferSigner]) -> BlockRequest {
    let state = stored_state(store, chain.config().session_id);
    let mut target = chain.config().clone();
    target.session_id = [id; 32];
    target.initial_state_root = chain.head().state_root;
    target.initial_input_position = chain.head().input_position + 1;
    target.initial_logical_time = chain.head().logical_time + 1;
    target.initial_timestamp_ms = chain.head().timestamp_ms.max(10);
    target.predecessor = Some(myelin_session::PredecessorReference {
        session_id: chain.config().session_id,
        final_height: chain.head().finalised_height.map_or(0, |h| h + 1),
        final_state_root: chain.head().state_root,
        application_profile_commitment: chain.config().application_profile.commitment(),
    });
    let mut approvals = owners.iter().map(|o| o.continue_session(chain.config(), &state, &target).unwrap()).collect::<Vec<_>>();
    approvals.sort_by_key(|a| a.owner);
    let mut req = request(chain, vec![TransferCommand::Continue { target_genesis: target.commitment(), approvals }], vec![], vec![]);
    req.successor = Some(target);
    req
}

pub fn recover_chain(store: &Arc<RocksSessionStore>, verifier: &Arc<RegisteredFinalityVerifier>, id: u8) -> Chain {
    let cfg = store.load_genesis([id; 32]).unwrap().config;
    let bytes = store.load_checkpoint(cfg.session_id).unwrap().state_snapshot;
    let executor = restore_executor(&cfg, &bytes, store.as_ref()).unwrap();
    SessionChain::recover(cfg.session_id, &cfg.application_profile, verifier.clone(), executor, store.clone()).unwrap()
}
pub struct Fixture {
    pub directory: tempfile::TempDir,
    pub store: Arc<RocksSessionStore>,
    pub verifier: Arc<RegisteredFinalityVerifier>,
    pub validator: CommitteeSigner,
    pub source: Chain,
    pub target: Chain,
}
impl Fixture {
    pub fn new(kind: TransferKind) -> Self {
        Self::with_program(kind, program_digest())
    }
    pub fn with_program(kind: TransferKind, program: Hash32) -> Self {
        let directory = tempfile::tempdir().unwrap();
        let store = Arc::new(RocksSessionStore::open(directory.path()).unwrap());
        let (verifier, validator) = consensus();
        let state = state(policy(kind));
        let mut cfg = config(&verifier, 3, &state);
        cfg.application_profile.program_digest = program;
        let source = create_chain(cfg.clone(), state, verifier.clone(), store.clone()).unwrap();
        let mut p = policy(kind);
        p.accepted_sources.push(SourceIdentity::from_config(&cfg).unwrap());
        let state = TransferState::new(p, vec![]).unwrap();
        let mut target_cfg = config(&verifier, 4, &state);
        target_cfg.application_profile.program_digest = program;
        let target = create_chain(target_cfg, state, verifier.clone(), store.clone()).unwrap();
        Self { directory, store, verifier, validator, source, target }
    }
    pub fn intent(&self, nonce: u8, version: u64, amount: u64) -> HandoffIntent {
        let p = stored_state(&self.store, self.source.config().session_id).policy().clone();
        signer()
            .sign(
                payload(
                    self.source.config(),
                    &p,
                    TransferTarget::Session {
                        session_id: self.target.config().session_id,
                        genesis_commitment: self.target.config().commitment(),
                    },
                    nonce,
                    version,
                    amount,
                ),
                100,
                None,
            )
            .unwrap()
    }
}
