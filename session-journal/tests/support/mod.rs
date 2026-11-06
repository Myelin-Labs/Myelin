#![allow(dead_code)]
use myelin_consensus::{Authority, CommitteeSigner, ConsensusConfig, FinalityProof, ProofOfAuthorityConfig, SelectedConsensus};
use myelin_session::*;
use myelin_session_journal::*;
use myelin_session_runtime::RegisteredFinalityVerifier;
use myelin_session_store_rocksdb::RocksSessionStore;
use secp256k1::{Keypair, Message, Secp256k1, SecretKey, XOnlyPublicKey};
use std::{path::Path, sync::Arc};

pub type Chain = SessionChain<Inventory, RocksSessionStore>;
pub struct Auth;
impl CommandAuthenticator for Auth {
    fn verify(&self, command: &OriginalCommand) -> std::result::Result<(), String> {
        if command.identity.payload_hash != *blake3::hash(&command.canonical_command).as_bytes() {
            return Err("payload digest mismatch".into());
        }
        if command.kind != InputKind::UserCommand || command.canonical_command.len() != 8 {
            return Err("inventory accepts canonical purchase commands only".into());
        }
        let key = XOnlyPublicKey::from_slice(&command.authentication_context).map_err(|e| e.to_string())?;
        if key != owner().x_only_public_key().0 {
            return Err("not the inventory owner".into());
        }
        let sig = secp256k1::schnorr::Signature::from_slice(&command.authorization).map_err(|e| e.to_string())?;
        Secp256k1::verification_only()
            .verify_schnorr(&sig, &Message::from_digest(command.authorization_digest()), &key)
            .map_err(|e| e.to_string())
    }
}
pub fn owner() -> Keypair {
    Keypair::from_secret_key(&Secp256k1::new(), &SecretKey::from_slice(&[3; 32]).unwrap())
}
pub fn command(config: &SessionConfig, id: u8, cost: u64) -> OriginalCommand {
    let body = cost.to_le_bytes().to_vec();
    let mut command = OriginalCommand {
        scope: CommandScope::from_config(config),
        identity: CommandIdentity { id: [id; 32], payload_hash: *blake3::hash(&body).as_bytes() },
        kind: InputKind::UserCommand,
        canonical_command: body,
        authentication_context: owner().x_only_public_key().0.serialize().to_vec(),
        authorization: vec![],
    };
    command.authorization =
        Secp256k1::new().sign_schnorr_no_aux_rand(&Message::from_digest(command.authorization_digest()), &owner()).as_ref().to_vec();
    command
}
#[derive(Clone)]
pub struct Inventory {
    pub scope: CommandScope,
    pub balance: u64,
}
impl TransitionExecutor for Inventory {
    fn state_root(&self) -> Hash32 {
        *blake3::hash(&self.balance.to_le_bytes()).as_bytes()
    }
    fn execute_block(
        &mut self,
        _: u64,
        frame: &FrameInput,
        handoffs: &[SessionHandoff],
        transactions: &[Vec<u8>],
    ) -> std::result::Result<ExecutionOutcome, String> {
        if !handoffs.is_empty() || !transactions.is_empty() {
            return Err("inventory uses original commands, not CellTx or handoffs".into());
        }
        let commands: Vec<OriginalCommand> = serde_json::from_slice(&frame.payload).map_err(|e| e.to_string())?;
        if commands.iter().map(|c| c.identity.clone()).collect::<Vec<_>>() != frame.commands {
            return Err("frame identity mismatch".into());
        }
        let mut outbox = vec![];
        for command in commands {
            if command.scope != self.scope {
                return Err("wrong session/genesis/profile".into());
            }
            Auth.verify(&command)?;
            let cost = u64::from_le_bytes(command.canonical_command.as_slice().try_into().unwrap());
            let result = if cost <= self.balance {
                self.balance -= cost;
                BusinessOutcome::Succeeded(self.balance.to_le_bytes().to_vec())
            } else {
                BusinessOutcome::Rejected(b"insufficient balance".to_vec())
            };
            outbox.push(PendingOutboxMessage {
                topic: "inventory-result".into(),
                payload: serde_json::to_vec(&(command.identity.id, result)).unwrap(),
            });
        }
        Ok(ExecutionOutcome { ordered_cell_tx_commitments: vec![], data_commitments: vec![], outbox, cycles: 0 })
    }
    fn snapshot(&self) -> std::result::Result<Vec<u8>, String> {
        Ok(self.balance.to_le_bytes().to_vec())
    }
}
pub fn consensus() -> (Arc<RegisteredFinalityVerifier>, CommitteeSigner) {
    let signer = CommitteeSigner::new("journal-validator", [7; 32]).unwrap();
    let v = RegisteredFinalityVerifier::new(ConsensusConfig::proof_of_authority(ProofOfAuthorityConfig {
        authorities: vec![Authority { id: signer.validator_id().into(), public_key: signer.ckb_public_key() }],
    }))
    .unwrap();
    (Arc::new(v), signer)
}
pub fn request(chain: &Chain, commands: Vec<OriginalCommand>) -> BlockRequest {
    let h = chain.head();
    BlockRequest {
        frame_input: FrameInput {
            commands: commands.iter().map(|c| c.identity.clone()).collect(),
            start_position: h.input_position,
            end_position: h.input_position + commands.len() as u64,
            logical_time_start: h.logical_time,
            logical_time_end: h.logical_time + 1,
            payload: serde_json::to_vec(&commands).unwrap(),
        },
        transactions: vec![],
        scheduler_commitment: [20; 32],
        data_commitments: vec![],
        timestamp_ms: h.timestamp_ms + 1,
        local_now_ms: h.timestamp_ms + 1,
        successor: None,
        handoffs: vec![],
        consume_handoffs: vec![],
        handoff_actions: vec![],
    }
}
pub fn commit(chain: &mut Chain, commands: Vec<OriginalCommand>) {
    let (v, signer) = consensus();
    let p = chain.prepare_block(request(chain, commands)).unwrap();
    let SelectedConsensus::ProofOfAuthority(engine) = v.selected_consensus() else { unreachable!() };
    let proof = FinalityProof::ProofOfAuthority(engine.seal_from_signer(p.block().hash(), p.block().number, &signer).unwrap());
    chain.commit_finalised(p, proof).unwrap();
}
pub fn result(record: &FinalisedBlockRecord, receipt: &CommandReceipt) -> std::result::Result<Option<BusinessOutcome>, String> {
    for event in &record.outbox {
        let (id, outcome): (Hash32, BusinessOutcome) = serde_json::from_slice(&event.payload).map_err(|e| e.to_string())?;
        if id == receipt.command.id {
            return Ok(Some(outcome));
        }
    }
    Ok(None)
}
pub fn open_chain(path: &Path) -> (Chain, Arc<RocksSessionStore>) {
    let (v, _) = consensus();
    let cfg = config(&v, 1, 100);
    let store = Arc::new(RocksSessionStore::open(path).unwrap());
    let chain = match store.load_head(cfg.session_id) {
        Ok(_) => {
            let (_, snapshot) = Chain::recovery_snapshot(&*store, cfg.session_id).unwrap();
            let executor =
                Inventory { scope: CommandScope::from_config(&cfg), balance: u64::from_le_bytes(snapshot.try_into().unwrap()) };
            Chain::recover(cfg.session_id, &cfg.application_profile, v, executor, store.clone()).unwrap()
        }
        Err(StoreError::NotFound(_)) => {
            let executor = Inventory { scope: CommandScope::from_config(&cfg), balance: 100 };
            Chain::create(cfg, v, executor, store.clone()).unwrap()
        }
        Err(error) => panic!("{error}"),
    };
    (chain, store)
}
pub fn config(v: &RegisteredFinalityVerifier, id: u8, balance: u64) -> SessionConfig {
    let d = v.descriptor();
    let cfg = SessionConfig {
        session_id: [id; 32],
        consensus_kind: d.consensus_kind,
        consensus_config_commitment: d.config_commitment,
        consensus_module_commitment: d.commitment(),
        consensus_wal_schema_hash: d.wal_schema_hash,
        application_profile: ApplicationProfile {
            application_id: "journal-inventory-test".into(),
            command_identity_required: true,
            program_digest: [11; 32],
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
        initial_state_root: *blake3::hash(&balance.to_le_bytes()).as_bytes(),
        initial_input_position: 0,
        initial_logical_time: 0,
        initial_timestamp_ms: 0,
        predecessor: None,
        max_block_transactions: 100,
        max_block_bytes: 1_000_000,
        max_future_drift_ms: 1000,
        commit_limits: CommitResourceLimits::default(),
        protocol_effects: ProtocolEffectsPolicy::Disabled,
    };
    cfg
}
