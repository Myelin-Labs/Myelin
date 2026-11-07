//! Independent-process scarce test-asset reference. Fixed PUBLIC development
//! keys; one trusted non-rollback terminal authority; no real custody or L1.
#[allow(dead_code)]
#[path = "support/remote_transfer.rs"]
mod fixture;
use fixture::*;
use myelin_session::*;
use myelin_session_transfer::*;
use std::{
    fs,
    io::Read,
    path::{Path, PathBuf},
};
use transfer::{hex, AnyResult};
fn bounded(path: &Path) -> AnyResult<Vec<u8>> {
    let mut bytes = vec![];
    fs::File::open(path)?.take(PROOF_LIMIT as u64 + 1).read_to_end(&mut bytes)?;
    if bytes.len() > PROOF_LIMIT {
        return Err("relay file exceeds proof limit".into());
    }
    Ok(bytes)
}
fn original(root: &Path) -> AnyResult<Deployment> {
    let d = Deployment::new(program_digest()?)?;
    if bounded(&root.join("source.config"))? != d.source.encode()? || bounded(&root.join("target.config"))? != d.target.encode()? {
        return Err("deployment requires its exact original executable/configuration".into());
    }
    Ok(d)
}
fn existing_source(
    d: &Deployment,
    root: &Path,
) -> AnyResult<(transfer::Chain, std::sync::Arc<myelin_session_store_rocksdb::RocksSessionStore>)> {
    if !root.join("source/CURRENT").is_file() {
        return Err("source database unavailable; do not reinitialize".into());
    }
    d.source_chain(&root.join("source"))
}
fn existing_target(d: &Deployment, root: &Path) -> AnyResult<Target> {
    if !root.join("target/canonical/CURRENT").is_file() || !root.join("target/epochs/CURRENT").is_file() {
        return Err("target canonical database/ownership unavailable; restore the original pair".into());
    }
    d.target_authority(&root.join("target"))
}
fn relay(d: &Deployment, path: &Path) -> AnyResult<HandoffInboxRecord> {
    let record = HandoffInboxRecord::decode(&bounded(path)?)?;
    d.registry.verify(&record)?;
    Ok(record)
}
fn cert(path: &Path) -> AnyResult<RemoteTerminalCertificate> {
    Ok(RemoteTerminalCertificate::decode(&bounded(path)?)?)
}
fn source_action(
    d: &Deployment,
    root: &Path,
    h: &SessionHandoff,
    command: TransferCommand,
    action: HandoffAction,
) -> AnyResult<serde_json::Value> {
    let (mut chain, store) = existing_source(d, root)?;
    let identity = command.identity(d.source.commitment())?;
    if store.load_command_receipt(d.source.session_id, identity.id)?.is_none() {
        let mut request = transfer::req(&chain, vec![command])?;
        request.handoff_actions.push(action);
        let (_, signer) = transfer::consensus("remote-source", [7; 32])?;
        transfer::commit(&mut chain, &d.source_finality, &signer, request)?;
    }
    let receipt = store.load_command_receipt(d.source.session_id, identity.id)?.ok_or("canonical command receipt absent")?;
    if receipt.command != identity {
        return Err("command identity conflicts with original payload".into());
    }
    let state = TransferState::decode(&store.load_checkpoint(d.source.session_id)?.state_snapshot)?;
    Ok(
        serde_json::json!({"handoff":hex(&h.id()),"head":hex(&chain.head().block_hash),"state_root":hex(&chain.head().state_root),"available":state.accounts()[0].available,"locked":state.accounts()[0].locked,"pending":state.pending().len(),"receipt":hex(blake3::hash(&receipt.encode()?).as_bytes())}),
    )
}
fn run() -> AnyResult<serde_json::Value> {
    let args = std::env::args().skip(1).collect::<Vec<_>>();
    if args.len() < 2 {
        return Err("usage: remote_transfer OP ROOT [relay files]".into());
    }
    let root = Path::new(&args[1]);
    if args[0] == "init" {
        fs::create_dir(root)?;
        let d = Deployment::new(program_digest()?)?;
        fs::write(root.join("source.config"), d.source.encode()?)?;
        fs::write(root.join("target.config"), d.target.encode()?)?;
        d.source_chain(&root.join("source"))?;
        d.target_authority(&root.join("target"))?;
        d.terminal_create(&root.join("terminal"))?;
        return Ok(
            serde_json::json!({"test_assets":true,"source_genesis":hex(&d.source.commitment()),"target_genesis":hex(&d.target.commitment()),"terminal_policy":hex(&d.terminal_config.terminal.commitment()?),"different_authorities":d.source.consensus_module_commitment!=d.target.consensus_module_commitment}),
        );
    }
    let d = original(root)?;
    let path = |index: usize| -> AnyResult<PathBuf> { Ok(PathBuf::from(args.get(index).ok_or("missing relay file argument")?)) };
    match args[0].as_str() {
        "issue" => {
            let (mut chain, store) = existing_source(&d, root)?;
            let id = if chain.head().finalised_height.is_none() {
                d.issue(&mut chain, 1, 0)?
            } else {
                store.load_block(d.source.session_id, 0)?.issued_handoffs.first().ok_or("original issuance absent")?.id()
            };
            Ok(serde_json::json!({"handoff":hex(&id),"head":hex(&chain.head().block_hash)}))
        }
        "export" => {
            let (_, store) = existing_source(&d, root)?;
            let id = store.load_block(d.source.session_id, 0)?.issued_handoffs[0].id();
            let bytes = d.proof(&store, id)?.encode()?;
            fs::write(path(2)?, &bytes)?;
            Ok(serde_json::json!({"handoff":hex(&id),"proof":hex(blake3::hash(&bytes).as_bytes())}))
        }
        "import" => {
            let proof = relay(&d, &path(2)?)?;
            let target = existing_target(&d, root)?;
            let result = target.ledger.import_handoff(&proof)?;
            Ok(
                serde_json::json!({"inserted":result.inserted,"available":result.handoff.is_available(),"target_height":target.authority.head()?.finalised_height}),
            )
        }
        "prepare" => {
            let proof = relay(&d, &path(2)?)?;
            let target = existing_target(&d, root)?;
            let bytes = if let Some(original) = target.authority.pending_remote_candidate()? {
                original
            } else {
                target.prepare(&d.target, proof.proof.handoff_id, args.get(4).map(|v| v.parse()).transpose()?.unwrap_or(1))?
            };
            let record = FinalisedBlockRecord::decode(&bytes)?;
            fs::write(path(3)?, bytes)?;
            Ok(serde_json::json!({"candidate":hex(&record.block.hash()),"target_height":target.authority.head()?.finalised_height}))
        }
        "authorize" => {
            let proof = relay(&d, &path(2)?)?;
            let candidate = bounded(&path(3)?)?;
            let authority = d.terminal_open(&root.join("terminal"))?;
            let certificate = authority.authorize_consume(&proof, &candidate)?;
            fs::write(path(4)?, certificate.encode()?)?;
            Ok(
                serde_json::json!({"decision":"consume-authorized","handoff":hex(&certificate.handoff_id),"certificate":hex(blake3::hash(&certificate.encode()?).as_bytes())}),
            )
        }
        "decide-cancel" => {
            let proof = relay(&d, &path(2)?)?;
            let verified = d.registry.verify(&proof)?;
            let TransferCommand::Cancel { authorization, .. } = owner().cancel(&d.source, verified.handoff())? else {
                return Err("wrong cancellation command".into());
            };
            let certificate = d.terminal_open(&root.join("terminal"))?.authorize_cancel(&proof, &authorization)?;
            fs::write(path(3)?, certificate.encode()?)?;
            Ok(
                serde_json::json!({"decision":"cancelled","handoff":hex(&certificate.handoff_id),"certificate":hex(blake3::hash(&certificate.encode()?).as_bytes())}),
            )
        }
        "commit" => {
            let certificate = cert(&path(2)?)?;
            let target = existing_target(&d, root)?;
            let receipt = target.authority.commit_remote(&certificate)?;
            receipt.verify(target.authority.policy())?;
            Ok(serde_json::to_value(receipt)?)
        }
        "record" => {
            let target = existing_target(&d, root)?;
            let record = target.authority.block(0)?;
            fs::write(path(2)?, record.encode()?)?;
            let state = TransferState::decode(&record.state_snapshot)?;
            Ok(
                serde_json::json!({"head":hex(&record.block.hash()),"available":state.accounts()[0].available,"state_root":hex(&record.block.state_root_after)}),
            )
        }
        "cancel" | "reclaim" | "complete" => {
            let proof = relay(&d, &path(2)?)?;
            let verified = d.registry.verify(&proof)?;
            let h = verified.handoff();
            let id = h.id();
            let (command, action) = match args[0].as_str() {
                "cancel" => {
                    let TransferCommand::Cancel { authorization, .. } = owner().cancel(&d.source, h)? else {
                        return Err("wrong cancellation command".into());
                    };
                    (
                        TransferCommand::RemoteCancel { handoff_id: id, authorization, certificate: cert(&path(3)?)?.encode()? },
                        HandoffAction::Cancel { handoff_id: id },
                    )
                }
                "reclaim" => (TransferCommand::Reclaim { handoff_id: id }, HandoffAction::Reclaim { handoff_id: id }),
                _ => (
                    TransferCommand::Retire { handoff_id: id },
                    HandoffAction::RemoteRetire {
                        handoff_id: id,
                        proof: RemoteConsumptionProof { import_policy: d.import_policy, target_record: bounded(&path(3)?)? },
                    },
                ),
            };
            source_action(&d, root, h, command, action)
        }
        "release" => {
            let target = existing_target(&d, root)?;
            target.authority.release_cancelled_remote(&cert(&path(2)?)?)?;
            Ok(serde_json::json!({"released":true,"target_height":target.authority.head()?.finalised_height}))
        }
        "source-status" => {
            let (chain, store) = existing_source(&d, root)?;
            let state = TransferState::decode(&store.load_checkpoint(d.source.session_id)?.state_snapshot)?;
            Ok(
                serde_json::json!({"available":state.accounts()[0].available,"locked":state.accounts()[0].locked,"pending":state.pending().len(),"head":hex(&chain.head().block_hash)}),
            )
        }
        "target-status" => {
            let target = existing_target(&d, root)?;
            Ok(
                serde_json::json!({"height":target.authority.head()?.finalised_height,"head":hex(&target.authority.head()?.block_hash),"reserved":target.authority.pending_remote_candidate()?.is_some()}),
            )
        }
        "replay-source" | "replay-target" => {
            let registry = d.registry.clone();
            let cfg = if args[0] == "replay-source" { d.source.clone() } else { d.target.clone() };
            let restore = move |bytes: &[u8]| -> myelin_session::Result<TransferExecutor> {
                let state = TransferState::decode(bytes).map_err(|e| SessionError::Replay(e.to_string()))?;
                TransferExecutor::restore(
                    &cfg,
                    bytes,
                    SourceBindings::from_registry(state.policy(), registry).map_err(|e| SessionError::Replay(e.to_string()))?,
                )
                .map_err(|e| SessionError::Replay(e.to_string()))
            };
            let receipt = if args[0] == "replay-source" {
                let (chain, _) = existing_source(&d, root)?;
                chain.replay_range(0, chain.head().finalised_height.ok_or("empty source")?, restore)?
            } else {
                let target = existing_target(&d, root)?;
                target.authority.replay_range(0, target.authority.head()?.finalised_height.ok_or("empty target")?, restore)?
            };
            Ok(
                serde_json::json!({"state_root":hex(&receipt.state_root_after),"frames":receipt.frame_count,"receipt":hex(&receipt.commitment())}),
            )
        }
        _ => Err("unsupported remote_transfer operation".into()),
    }
}
fn main() {
    match run() {
        Ok(value) => println!("{value}"),
        Err(error) => {
            eprintln!("{error}");
            std::process::exit(1);
        }
    }
}
