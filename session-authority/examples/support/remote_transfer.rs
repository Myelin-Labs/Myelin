//! Public-key, test-asset deployment helpers shared by the runnable fault drill.
#[path = "../../../session-transfer/examples/support/transfer.rs"]
pub mod transfer;
use myelin_consensus::{Authority, CommitteeSigner, ConsensusConfig, ProofOfAuthorityConfig};
use myelin_session::*;
use myelin_session_authority::*;
use myelin_session_runtime::RegisteredFinalityVerifier;
use myelin_session_store_rocksdb::{RemoteInboxLimits, RocksSessionStore};
use myelin_session_transfer::*;
use std::{path::Path, sync::Arc};
use transfer::{AnyResult, ASSET};
pub const PROOF_LIMIT: usize = 1_000_000;
pub fn authority_key(n: u8) -> AuthoritySigner {
    AuthoritySigner::new([n; 32]).unwrap()
}
pub fn owner() -> TransferSigner {
    TransferSigner::new([3; 32]).unwrap()
}
pub fn recipient() -> Hash32 {
    TransferSigner::new([4; 32]).unwrap().actor()
}
fn target_consensus() -> ConsensusConfig {
    let signer = CommitteeSigner::new("remote-target", [8; 32]).unwrap();
    ConsensusConfig::proof_of_authority(ProofOfAuthorityConfig {
        authorities: vec![Authority { id: signer.validator_id().into(), public_key: signer.ckb_public_key() }],
    })
}
#[derive(Clone)]
pub struct Deployment {
    pub source: SessionConfig,
    pub source_state: TransferState,
    pub target: SessionConfig,
    pub target_state: TransferState,
    pub registry: Arc<HandoffImportRegistry>,
    pub import_policy: Hash32,
    pub source_finality: Arc<RegisteredFinalityVerifier>,
    pub target_finality: Arc<RegisteredFinalityVerifier>,
    pub terminal_config: TerminalAuthorityConfig,
}
impl Deployment {
    pub fn new(program: Hash32) -> AnyResult<Self> {
        let (source_finality, _) = transfer::consensus("remote-source", [7; 32])?;
        let target_finality = Arc::new(RegisteredFinalityVerifier::new(target_consensus())?);
        let terminal = RemoteTerminalPolicy {
            version: REMOTE_TERMINAL_VERSION,
            domain: [29; 32],
            coordinator_key: authority_key(17).public_key(),
            source_session: [3; 32],
            target_session: [4; 32],
        };
        let policy = TransferPolicy {
            deployment_id: [1; 32],
            uniqueness_domain: terminal.commitment()?,
            coordination: TransferCoordination::Remote { terminal: terminal.clone() },
            kind: TransferKind::Scarce,
            limits: TransferLimits::default(),
            accepted_sources: vec![],
        };
        let source_state = TransferState::new(
            policy.clone(),
            vec![AssetAccount { asset: ASSET, owner: owner().actor(), available: 100, locked: 0, version: 0, components: vec![7, 8] }],
        )?;
        let source = transfer::config(&source_finality, 3, &source_state, program)?;
        let mut intake = policy.clone();
        intake.accepted_sources.push(SourceIdentity::from_config(&source)?);
        let target_state = TransferState::new(intake, vec![])?;
        let target = transfer::config(&target_finality, 4, &target_state, program)?;
        let import = HandoffImportPolicy::new(&source, &target, PROOF_LIMIT)?;
        let application = TransferImportVerifier::new(
            target_state.policy().clone(),
            vec![policy],
            Arc::new(|_, _, _, _| Err(SessionError::Evidence("no external stages in this test-asset deployment".into()))),
        )?;
        let mut registry = HandoffImportRegistry::default();
        let import_policy = registry.register(
            import,
            source.clone(),
            target.clone(),
            RemoteFinalityVerifiers { source: source_finality.clone(), target: target_finality.clone() },
            Arc::new(application),
        )?;
        let terminal_config = TerminalAuthorityConfig { import_policy, terminal, max_claims: 32, max_bytes: 32_000_000 };
        Ok(Self {
            source,
            source_state,
            target,
            target_state,
            registry: Arc::new(registry),
            import_policy,
            source_finality,
            target_finality,
            terminal_config,
        })
    }
    pub fn store(&self, path: &Path) -> AnyResult<Arc<RocksSessionStore>> {
        Ok(Arc::new(
            RocksSessionStore::open(path)?
                .with_handoff_imports(self.registry.clone(), RemoteInboxLimits { max_entries: 32, max_bytes: 32_000_000 })?,
        ))
    }
    pub fn source_chain(&self, path: &Path) -> AnyResult<(transfer::Chain, Arc<RocksSessionStore>)> {
        let store = self.store(path)?;
        let chain = transfer::open_chain(self.source.clone(), self.source_state.clone(), self.source_finality.clone(), store.clone())?;
        Ok((chain, store))
    }
    pub fn target_authority(&self, root: &Path) -> AnyResult<Target> {
        std::fs::create_dir_all(root)?;
        let ledger = CanonicalLedger::open_with_handoff_imports(
            root.join("canonical"),
            self.registry.clone(),
            RemoteInboxLimits { max_entries: 32, max_bytes: 32_000_000 },
        )?;
        let initial = TransferExecutor::new(
            &self.target,
            self.target_state.clone(),
            SourceBindings::from_registry(self.target_state.policy(), self.registry.clone())?,
        )?;
        let cfg = self.target.clone();
        let registry = self.registry.clone();
        let authority = SessionAuthority::open(
            ledger.clone(),
            root.join("epochs"),
            AuthorityConfig {
                session: cfg.clone(),
                consensus: target_consensus(),
                administrator: authority_key(11).public_key(),
                effect_domain: [99; 32],
            },
            AuthorityKeys { finality: CommitteeSigner::new("remote-target", [8; 32])?, certificates: authority_key(12) },
            initial,
            move |bytes| {
                let state = TransferState::decode(bytes).map_err(|e| e.to_string())?;
                let bindings = SourceBindings::from_registry(state.policy(), registry).map_err(|e| e.to_string())?;
                TransferExecutor::restore(&cfg, bytes, bindings).map_err(|e| e.to_string())
            },
        )?;
        if authority.current_grant()?.is_none() {
            let request =
                authority_key(11).authorize_grant(authority.policy(), 0, authority_key(13).public_key(), &authority.head()?)?;
            authority.grant(request)?;
        }
        Ok(Target { ledger, authority })
    }
    pub fn terminal_create(&self, path: &Path) -> AnyResult<TerminalAuthority> {
        Ok(TerminalAuthority::create(path, self.terminal_config.clone(), self.registry.clone(), authority_key(17))?)
    }
    pub fn terminal_open(&self, path: &Path) -> AnyResult<TerminalAuthority> {
        Ok(TerminalAuthority::open(path, self.terminal_config.clone(), self.registry.clone(), authority_key(17))?)
    }
    pub fn issue(&self, chain: &mut transfer::Chain, nonce: u8, version: u64) -> AnyResult<Hash32> {
        let payload = TransferPayload {
            deployment_id: [1; 32],
            uniqueness_domain: self.source_state.policy().uniqueness_domain,
            kind: TransferKind::Scarce,
            intake_schema: TransferKind::Scarce.intake_schema(),
            source: SourceIdentity::from_config(&self.source)?,
            asset: ASSET,
            owner: owner().actor(),
            recipient: recipient(),
            source_version: version,
            amount: 25,
            components: vec![7, 8],
            nonce: [nonce; 32],
            target: TransferTarget::Session { session_id: self.target.session_id, genesis_commitment: self.target.commitment() },
        };
        let intent = owner().sign(payload, 100, None)?;
        let command = TransferCommand::Issue { transfer_id: verify_intent(&intent)?.0 };
        let mut request = transfer::req(chain, vec![command])?;
        request.handoffs.push(intent);
        let (_, signer) = transfer::consensus("remote-source", [7; 32])?;
        let prepared = chain.prepare_block(request)?;
        let id = prepared.issued_handoffs()[0].id();
        use myelin_consensus::{FinalityProof, SelectedConsensus};
        let SelectedConsensus::ProofOfAuthority(engine) = self.source_finality.selected_consensus() else {
            return Err("not PoA".into());
        };
        let proof =
            FinalityProof::ProofOfAuthority(engine.seal_from_signer(prepared.block().hash(), prepared.block().number, &signer)?);
        chain.commit_finalised(prepared, proof)?;
        Ok(id)
    }
    pub fn proof(&self, store: &RocksSessionStore, id: Hash32) -> AnyResult<HandoffInboxRecord> {
        Ok(HandoffInboxRecord { policy: self.import_policy, proof: HandoffProofBundle::export(store, id, &self.target)? })
    }
}
pub struct Target {
    pub ledger: Arc<CanonicalLedger>,
    pub authority: SessionAuthority<TransferExecutor>,
}
pub fn request(config: &SessionConfig, head: &SessionHead, commands: Vec<TransferCommand>) -> AnyResult<BlockRequest> {
    Ok(BlockRequest {
        transactions: vec![],
        frame_input: command_frame(config, head.input_position, head.logical_time, commands)?,
        scheduler_commitment: [0; 32],
        data_commitments: vec![],
        timestamp_ms: head.timestamp_ms + 1,
        local_now_ms: head.timestamp_ms + 1,
        successor: None,
        handoffs: vec![],
        consume_handoffs: vec![],
        handoff_actions: vec![],
    })
}
impl Target {
    pub fn prepare(&self, config: &SessionConfig, id: Hash32, time: u64) -> AnyResult<Vec<u8>> {
        let head = self.authority.head()?;
        let mut request = request(config, &head, vec![TransferCommand::Intake { handoff_id: id }])?;
        request.consume_handoffs.push(id);
        request.timestamp_ms = time;
        let grant = self.authority.current_grant()?.ok_or("writer grant absent")?;
        let auth = authority_key(13).authorize_commit(self.authority.policy(), &grant, &head, &request)?;
        Ok(self.authority.prepare_remote(auth, request, time)?)
    }
}

pub fn program_digest() -> AnyResult<Hash32> {
    use std::io::Read;
    let mut input = std::fs::File::open(std::env::current_exe()?)?;
    let mut digest = blake3::Hasher::new();
    let mut buffer = [0; 65536];
    loop {
        let n = input.read(&mut buffer)?;
        if n == 0 {
            break;
        }
        digest.update(&buffer[..n]);
    }
    Ok(*digest.finalize().as_bytes())
}
