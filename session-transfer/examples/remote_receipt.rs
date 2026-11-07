//! Two independent development-key services exchanging a copyable test receipt.
//! The proof is an untrusted relay payload; local deployment files supply trust.
//! Run each operation as a separate process. No scarce assets or CKB settlement.
#[path = "support/transfer.rs"]
mod transfer;
use myelin_session::{
    HandoffImportPolicy, HandoffImportRegistry, HandoffInboxRecord, HandoffProofBundle, Hash32, SessionConfig, SessionError,
    SessionStore,
};
use myelin_session_store_rocksdb::{RemoteInboxLimits, RocksSessionStore};
use myelin_session_transfer::*;
use std::{fs, io::Read, path::Path, sync::Arc};
use transfer::*;

const PROOF_LIMIT: usize = 1024 * 1024;
struct Deployment {
    source: SessionConfig,
    source_state: TransferState,
    target: SessionConfig,
    target_state: TransferState,
    program: Hash32,
}
fn program_digest() -> AnyResult<Hash32> {
    let mut file = fs::File::open(std::env::current_exe()?)?;
    let mut hash = blake3::Hasher::new();
    let mut bytes = [0; 65536];
    loop {
        let n = file.read(&mut bytes)?;
        if n == 0 {
            break;
        }
        hash.update(&bytes[..n]);
    }
    Ok(*hash.finalize().as_bytes())
}
fn read_bounded(path: &Path, limit: usize) -> AnyResult<Vec<u8>> {
    let mut bytes = vec![];
    fs::File::open(path)?.take(limit as u64 + 1).read_to_end(&mut bytes)?;
    if bytes.len() > limit {
        return Err("input exceeds byte limit".into());
    }
    Ok(bytes)
}
impl Deployment {
    fn read(root: &Path) -> AnyResult<Self> {
        let source = SessionConfig::decode(&read_bounded(&root.join("source.config"), PROOF_LIMIT)?)?;
        let target = SessionConfig::decode(&read_bounded(&root.join("target.config"), PROOF_LIMIT)?)?;
        let program = program_digest()?;
        if source.application_profile.program_digest != program || target.application_profile.program_digest != program {
            return Err("deployment requires its exact original executable".into());
        }
        Ok(Self {
            source,
            target,
            source_state: TransferState::decode(&read_bounded(&root.join("source.initial"), PROOF_LIMIT)?)?,
            target_state: TransferState::decode(&read_bounded(&root.join("target.initial"), PROOF_LIMIT)?)?,
            program,
        })
    }
    fn target_store(&self, root: &Path, create: bool) -> AnyResult<(Arc<RocksSessionStore>, Hash32)> {
        if !create && !root.join("target").is_dir() {
            return Err("target database is unavailable; restore rather than reinitialize".into());
        }
        let (source_finality, _) = consensus("remote-source", [7; 32])?;
        let policy = HandoffImportPolicy::new(&self.source, &self.target, PROOF_LIMIT)?;
        let application = TransferImportVerifier::new(
            self.target_state.policy().clone(),
            vec![self.source_state.policy().clone()],
            Arc::new(|_, _, _, _| Err(SessionError::Evidence("this receipt fixture has no external evidence adapter".into()))),
        )?;
        let mut registry = HandoffImportRegistry::default();
        let (target_finality, _) = consensus("remote-target", [8; 32])?;
        let id = registry.register(
            policy,
            self.source.clone(),
            self.target.clone(),
            myelin_session::RemoteFinalityVerifiers { source: source_finality, target: target_finality },
            Arc::new(application),
        )?;
        let store = RocksSessionStore::open(root.join("target"))?
            .with_handoff_imports(Arc::new(registry), RemoteInboxLimits { max_entries: 8, max_bytes: 8 * PROOF_LIMIT as u64 })?;
        Ok((Arc::new(store), id))
    }
}
fn initialize(root: &Path) -> AnyResult<serde_json::Value> {
    fs::create_dir(root)?;
    let program = program_digest()?;
    let (source_finality, _) = consensus("remote-source", [7; 32])?;
    let (target_finality, _) = consensus("remote-target", [8; 32])?;
    let owner = TransferSigner::new([3; 32])?;
    let policy = TransferPolicy {
        coordination: myelin_session_transfer::TransferCoordination::SharedStore,
        deployment_id: [1; 32],
        uniqueness_domain: [2; 32],
        kind: TransferKind::CopyableReceipt,
        limits: TransferLimits::default(),
        accepted_sources: vec![],
    };
    let initial = TransferState::new(
        policy.clone(),
        vec![AssetAccount { asset: ASSET, owner: owner.actor(), available: 100, locked: 0, version: 0, components: vec![7, 8] }],
    )?;
    let source = config(&source_finality, 3, &initial, program)?;
    let mut intake = policy;
    intake.accepted_sources.push(SourceIdentity::from_config(&source)?);
    let target_initial = TransferState::new(intake, vec![])?;
    let target = config(&target_finality, 4, &target_initial, program)?;
    fs::write(root.join("source.config"), source.encode()?)?;
    fs::write(root.join("target.config"), target.encode()?)?;
    fs::write(root.join("source.initial"), initial.encode()?)?;
    fs::write(root.join("target.initial"), target_initial.encode()?)?;
    let source_store = Arc::new(RocksSessionStore::open(root.join("source"))?);
    open_chain(source.clone(), initial, source_finality, source_store)?;
    let deployment = Deployment::read(root)?;
    let (store, _) = deployment.target_store(root, true)?;
    open_chain(target.clone(), deployment.target_state, target_finality, store)?;
    Ok(
        serde_json::json!({"mode":"copyable-test-receipt","source_genesis":hex(&source.commitment()),"target_genesis":hex(&target.commitment()),
        "different_authorities":source.consensus_module_commitment!=target.consensus_module_commitment,"program":hex(&program)}),
    )
}
fn run() -> AnyResult<serde_json::Value> {
    let args = std::env::args().skip(1).collect::<Vec<_>>();
    if args.len() < 2 {
        return Err("usage: remote_receipt init|issue|export|import|consume|replay ROOT [PROOF_FILE]".into());
    }
    let root = Path::new(&args[1]);
    if args[0] == "init" {
        return initialize(root);
    }
    let deployment = Deployment::read(root)?;
    if args[0] == "issue" || args[0] == "export" {
        if !root.join("source").is_dir() {
            return Err("source database is unavailable".into());
        }
        let store = Arc::new(RocksSessionStore::open(root.join("source"))?);
        store.load_genesis(deployment.source.session_id)?;
        if args[0] == "issue" {
            let (finality, signer) = consensus("remote-source", [7; 32])?;
            let mut chain = open_chain(deployment.source.clone(), deployment.source_state.clone(), finality.clone(), store.clone())?;
            let payload = TransferPayload {
                deployment_id: deployment.source_state.policy().deployment_id,
                uniqueness_domain: deployment.source_state.policy().uniqueness_domain,
                kind: TransferKind::CopyableReceipt,
                intake_schema: TransferKind::CopyableReceipt.intake_schema(),
                source: SourceIdentity::from_config(&deployment.source)?,
                asset: ASSET,
                owner: TransferSigner::new([3; 32])?.actor(),
                recipient: TransferSigner::new([4; 32])?.actor(),
                source_version: 0,
                amount: 1,
                components: vec![7, 8],
                nonce: [1; 32],
                target: TransferTarget::Session {
                    session_id: deployment.target.session_id,
                    genesis_commitment: deployment.target.commitment(),
                },
            };
            let intent = TransferSigner::new([3; 32])?.sign(payload, 100, None)?;
            let command = TransferCommand::Issue { transfer_id: verify_intent(&intent)?.0 };
            let identity = command.identity(deployment.source.commitment())?;
            if store.load_command_receipt(deployment.source.session_id, identity.id)?.is_none() {
                let mut request = req(&chain, vec![command])?;
                request.handoffs.push(intent);
                commit(&mut chain, &finality, &signer, request)?;
            }
        }
        let block = store.load_block(deployment.source.session_id, 0)?;
        if block.issued_handoffs.len() != 1 {
            return Err("fixture requires one original handoff".into());
        }
        let id = block.issued_handoffs[0].id();
        if args[0] == "export" {
            let path = Path::new(args.get(2).ok_or("export requires a proof file")?);
            let proof = HandoffProofBundle::export(store.as_ref(), id, &deployment.target)?;
            fs::write(path, proof.encode(PROOF_LIMIT)?)?;
        }
        return Ok(
            serde_json::json!({"source_height":block.block.number,"handoff_id":hex(&id),"source_block":hex(&block.block.hash())}),
        );
    }
    let (store, policy) = deployment.target_store(root, false)?;
    store.load_genesis(deployment.target.session_id)?;
    if args[0] == "import" {
        let path = Path::new(args.get(2).ok_or("import requires a proof file")?);
        let proof = HandoffProofBundle::decode(&read_bounded(path, PROOF_LIMIT)?, PROOF_LIMIT)?;
        let imported = store.import_handoff(&HandoffInboxRecord { policy, proof })?;
        return Ok(
            serde_json::json!({"inserted":imported.inserted,"consumed":imported.handoff.is_consumed(),"handoff_id":hex(&imported.handoff.handoff.id()),
            "local_source_genesis":store.load_genesis(deployment.source.session_id).is_ok(),"local_source_block":store.load_block(deployment.source.session_id,0).is_ok()}),
        );
    }
    let (finality, signer) = consensus("remote-target", [8; 32])?;
    let mut chain = open_chain(deployment.target.clone(), deployment.target_state.clone(), finality.clone(), store.clone())?;
    if args[0] == "consume" {
        let bytes = read_bounded(Path::new(args.get(2).ok_or("consume requires the original proof file")?), PROOF_LIMIT)?;
        let proof = HandoffProofBundle::decode(&bytes, PROOF_LIMIT)?;
        let id = proof.handoff_id;
        let command = TransferCommand::Intake { handoff_id: id };
        let identity = command.identity(deployment.target.commitment())?;
        if store.load_command_receipt(deployment.target.session_id, identity.id)?.is_none() {
            let mut request = req(&chain, vec![command])?;
            request.consume_handoffs.push(id);
            commit(&mut chain, &finality, &signer, request)?;
        }
        let receipt =
            store.load_command_receipt(deployment.target.session_id, identity.id)?.ok_or("original consumption receipt absent")?;
        return Ok(
            serde_json::json!({"command_id":hex(&identity.id),"receipt":hex(blake3::hash(&receipt.encode()?).as_bytes()),"head":hex(&chain.head().block_hash),
            "state_root":hex(&chain.head().state_root),"receipts":TransferState::decode(&store.load_checkpoint(deployment.target.session_id)?.state_snapshot)?.receipts().len()}),
        );
    }
    if args[0] == "replay" {
        let cfg = deployment.target.clone();
        let replay_store = store.clone();
        let replay = chain.replay_range(0, chain.head().finalised_height.ok_or("target history is empty")?, move |bytes| {
            restore_executor(&cfg, bytes, replay_store.as_ref()).map_err(|e| SessionError::Replay(e.to_string()))
        })?;
        return Ok(
            serde_json::json!({"verified":true,"state_root":hex(&replay.state_root_after),"head":hex(&chain.head().block_hash),"receipt":hex(&replay.commitment()),"program":hex(&deployment.program)}),
        );
    }
    Err("unknown operation".into())
}
fn main() {
    match run() {
        Ok(result) => println!("{result}"),
        Err(error) => {
            eprintln!("{}", serde_json::json!({"error":error.to_string()}));
            std::process::exit(2)
        }
    }
}
