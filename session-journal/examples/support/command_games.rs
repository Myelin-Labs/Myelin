//! Development-only game rules, outside the reusable journal/adapter library.
//! Public fixed keys must never control production assets or an external service.
#![allow(dead_code)]
use myelin_consensus::{Authority, CommitteeSigner, ConsensusConfig, ProofOfAuthorityConfig};
use myelin_session::*;
use myelin_session_authority::*;
use myelin_session_journal::{application::*, *};
use myelin_session_runtime::RegisteredFinalityVerifier;
use secp256k1::{Keypair, Message, Secp256k1, SecretKey, XOnlyPublicKey};
use std::{io::Read, path::Path, sync::Arc};

pub type AnyResult<T> = std::result::Result<T, Box<dyn std::error::Error>>;
pub mod command_bundle;
pub mod game_rules;
pub use game_rules::{tick_game, Action, Game, Kind};
impl CommandProgram for Game {
    fn clock(&self) -> ApplicationClock {
        if matches!(self, Self::Ticks(_)) {
            ApplicationClock::FixedTicks {
                tick_duration_us: tick_game::TICK_DURATION_US,
                numeric_policy: *blake3::hash(b"integer-millipixels:i32;x-clamp=1000000;v-clamp=12;accel=3-per-tick;1").as_bytes(),
                admission_policy: *blake3::hash(b"two-fixed-seats;ordered-control-events;AI-after-two-disconnected-ticks;1")
                    .as_bytes(),
                replay_policy: *blake3::hash(b"public-state-and-seed;sequencer-signed-whole-tick;1").as_bytes(),
            }
        } else {
            ApplicationClock::CommandSteps
        }
    }
    fn snapshot(&self) -> std::result::Result<Vec<u8>, String> {
        Game::snapshot(self)
    }
    fn apply(&mut self, context: CommandContext, command: &OriginalCommand) -> std::result::Result<BusinessOutcome, String> {
        Ok(match self.apply_rule(context.input_position, context.logical_time, &command.canonical_command)? {
            Ok(()) => BusinessOutcome::Succeeded(self.snapshot()?),
            Err(reason) => BusinessOutcome::Rejected(reason.as_bytes().to_vec()),
        })
    }
    fn inspect(&self, query: &[u8]) -> std::result::Result<Vec<u8>, String> {
        if query != b"state" {
            return Err("unknown query".into());
        }
        self.snapshot()
    }
}
pub fn key(n: u8) -> Keypair {
    Keypair::from_secret_key(&Secp256k1::new(), &SecretKey::from_slice(&[n; 32]).unwrap())
}
pub fn signer(n: u8) -> AuthoritySigner {
    AuthoritySigner::new([n; 32]).unwrap()
}
pub struct Auth(pub Kind);
impl CommandAuthenticator for Auth {
    fn verify(&self, command: &OriginalCommand) -> std::result::Result<(), String> {
        let action: Action = serde_json::from_slice(&command.canonical_command).map_err(|e| e.to_string())?;
        if serde_json::to_vec(&action).map_err(|e| e.to_string())? != command.canonical_command
            || command.identity.payload_hash != *blake3::hash(&command.canonical_command).as_bytes()
        {
            return Err("noncanonical action/hash".into());
        }
        let (expected_key, kind) = match (self.0, &action) {
            (Kind::Inventory, Action::Buy { .. } | Action::Save { .. }) => (3, InputKind::UserCommand),
            (Kind::Turns, Action::Move { seat, .. }) if *seat < 2 => (3 + *seat, InputKind::UserCommand),
            (Kind::Turns, Action::Timeout { seat, .. }) if *seat < 2 => (5, InputKind::SystemEvent),
            (Kind::Ticks, Action::Tick(_)) => (5, InputKind::InputTape),
            _ => return Err("unsupported action/seat".into()),
        };
        let public = XOnlyPublicKey::from_slice(&command.authentication_context).map_err(|e| e.to_string())?;
        if public != key(expected_key).x_only_public_key().0 || command.kind != kind {
            return Err("wrong actor or event authority".into());
        }
        let sig = secp256k1::schnorr::Signature::from_slice(&command.authorization).map_err(|e| e.to_string())?;
        Secp256k1::verification_only()
            .verify_schnorr(&sig, &Message::from_digest(command.authorization_digest()), &public)
            .map_err(|e| e.to_string())
    }
}
pub fn signed(config: &SessionConfig, id: Hash32, action: Action, actor: u8) -> OriginalCommand {
    let body = serde_json::to_vec(&action).unwrap();
    let mut command = OriginalCommand {
        scope: CommandScope::from_config(config),
        identity: CommandIdentity { id, payload_hash: *blake3::hash(&body).as_bytes() },
        kind: match action {
            Action::Timeout { .. } => InputKind::SystemEvent,
            Action::Tick(_) => InputKind::InputTape,
            _ => InputKind::UserCommand,
        },
        canonical_command: body,
        authentication_context: key(actor).x_only_public_key().0.serialize().to_vec(),
        authorization: vec![],
    };
    command.authorization = Secp256k1::new()
        .sign_schnorr_no_aux_rand(&Message::from_digest(command.authorization_digest()), &key(actor))
        .as_ref()
        .to_vec();
    command
}
pub fn config(kind: Kind, program_digest: Hash32) -> SessionConfig {
    let signer = CommitteeSigner::new("command-game-validator", [7; 32]).unwrap();
    let verifier = RegisteredFinalityVerifier::new(ConsensusConfig::proof_of_authority(ProofOfAuthorityConfig {
        authorities: vec![Authority { id: signer.validator_id().into(), public_key: signer.ckb_public_key() }],
    }))
    .unwrap();
    let d = verifier.descriptor();
    let h = |v: &[u8]| *blake3::hash(v).as_bytes();
    let profile = CommandProfile {
        application_id: format!("command-game-example/{}", kind.name()),
        program_digest,
        command_schema: h(b"example-json-actions:integer-inventory,two-seat-board,fixed-tick:1"),
        authorization_policy: h(
            b"development-keys:player0=3;player1=4;system-and-tick-sequencer=5;BIP340-journal-scope;timeout>=30000ms;public-state;1",
        ),
        entropy_policy: h(if kind == Kind::Ticks { tick_game::RNG_POLICY } else { b"no-randomness" }),
        clock: Game::new(kind).clock(),
        resources: ApplicationResourceEnvelope {
            max_frame_inputs: 64,
            max_frame_input_bytes: 1_000_000,
            max_frame_cycles: 1,
            max_logical_time_span: 64,
            max_inspect_query_bytes: 32,
            max_inspect_result_bytes: 4096,
        },
    }
    .build()
    .unwrap();
    let limits = CommitResourceLimits::default();
    let root = CommandExecutor::initial_root(&profile, &Game::new(kind), 0, 0, limits.max_snapshot_bytes).unwrap();
    SessionConfig {
        session_id: [kind.id(); 32],
        consensus_kind: d.consensus_kind,
        consensus_config_commitment: d.config_commitment,
        consensus_module_commitment: d.commitment(),
        consensus_wal_schema_hash: d.wal_schema_hash,
        application_profile: profile,
        initial_state_root: root,
        initial_input_position: 0,
        initial_logical_time: 0,
        initial_timestamp_ms: 0,
        predecessor: None,
        max_block_transactions: 64,
        max_block_bytes: 1_000_000,
        max_future_drift_ms: 1000,
        commit_limits: limits,
        protocol_effects: ProtocolEffectsPolicy::Disabled,
    }
}
pub fn artifact_digest() -> AnyResult<Hash32> {
    file_digest(std::env::current_exe()?)
}
pub fn file_digest(path: impl AsRef<Path>) -> AnyResult<Hash32> {
    let mut file = std::fs::File::open(path)?;
    let mut h = blake3::Hasher::new();
    let mut buf = [0; 65536];
    loop {
        let n = file.read(&mut buf)?;
        if n == 0 {
            break;
        }
        h.update(&buf[..n]);
    }
    Ok(*h.finalize().as_bytes())
}
pub fn restore(config: &SessionConfig, kind: Kind, bytes: &[u8]) -> myelin_session::Result<CommandExecutor<Game>> {
    CommandExecutor::restore(config.clone(), bytes, Arc::new(Auth(kind)), |bytes| Game::restore(kind, bytes))
        .map_err(SessionError::Execution)
}
pub fn open(root: &Path, kind: Kind, program: Hash32) -> AnyResult<(SessionAuthority<CommandExecutor<Game>>, SessionConfig)> {
    let config = config(kind, program);
    let finality = CommitteeSigner::new("command-game-validator", [7; 32])?;
    let consensus = ConsensusConfig::proof_of_authority(ProofOfAuthorityConfig {
        authorities: vec![Authority { id: finality.validator_id().into(), public_key: finality.ckb_public_key() }],
    });
    let executor = CommandExecutor::new(config.clone(), Game::new(kind), Arc::new(Auth(kind)))?;
    let authority = SessionAuthority::open(
        myelin_session_authority::CanonicalLedger::open(root.join("canonical"))?,
        root.join("epochs"),
        AuthorityConfig { session: config.clone(), consensus, administrator: signer(8).public_key(), effect_domain: [kind.id(); 32] },
        AuthorityKeys { finality, certificates: signer(9) },
        executor,
        |bytes| restore(&config, kind, bytes).map_err(|e| e.to_string()),
    )?;
    Ok((authority, config))
}
pub fn demo_commands(config: &SessionConfig, kind: Kind) -> Vec<OriginalCommand> {
    match kind {
        Kind::Inventory => vec![
            signed(config, [1; 32], Action::Buy { quantity: 1, expected_revision: 0 }, 3),
            signed(config, [2; 32], Action::Save { checkpoint: 3, expected_revision: 1 }, 3),
            signed(config, [3; 32], Action::Buy { quantity: 99, expected_revision: 2 }, 3),
        ],
        Kind::Turns => vec![
            signed(config, [1; 32], Action::Move { seat: 0, square: 0, expected_turn: 0 }, 3),
            signed(config, [2; 32], Action::Move { seat: 1, square: 4, expected_turn: 1 }, 4),
            signed(config, [3; 32], Action::Move { seat: 0, square: 1, expected_turn: 0 }, 3),
            signed(config, [4; 32], Action::Move { seat: 0, square: 1, expected_turn: 2 }, 3),
            signed(config, [5; 32], Action::Timeout { seat: 1, expected_turn: 3, elapsed_ms: 29_999 }, 5),
            signed(config, [6; 32], Action::Timeout { seat: 1, expected_turn: 3, elapsed_ms: 30_000 }, 5),
            signed(config, [7; 32], Action::Move { seat: 1, square: 8, expected_turn: 3 }, 4),
        ],
        Kind::Ticks => tick_game::demo().into_iter().map(|t| signed(config, [t.tick as u8 + 1; 32], Action::Tick(t), 5)).collect(),
    }
}
fn crash(selected: Option<&str>, point: &str) {
    if selected == Some(point) {
        std::process::exit(76);
    }
}
pub fn run(root: &Path, kind: Kind, operation: &str, stop: Option<&str>) -> AnyResult<serde_json::Value> {
    if !matches!(operation, "demo" | "show" | "replay") {
        return Err("expected demo, show or replay".into());
    }
    if operation != "demo" && !root.join("canonical/CURRENT").exists() {
        return Err("show/replay requires an existing example store".into());
    }
    if operation == "demo" {
        std::fs::create_dir_all(root)?;
    }
    let program = artifact_digest()?;
    let (authority, config) = open(root, kind, program)?;
    if authority.head()?.finalised_height.is_some() && !root.join("journal/CURRENT").exists() {
        return Err("committed example history is missing its original-command journal".into());
    }
    let journal = CommandJournal::open_fenced(root.join("journal"), &config, JournalLimits::default(), authority.policy().clone())?;
    if operation == "demo" {
        recover_local_commands(&authority, &journal, &signer(8), &signer(10))?;
        for command in demo_commands(&config, kind) {
            journal.enqueue(command, &Auth(kind))?;
        }
        crash(stop, "after-enqueue");
        while let Some(reserved) = journal.reserve_commands(&authority.head()?, 1, 100_000, &Auth(kind))? {
            crash(stop, "after-reserve");
            let head = authority.head()?;
            let grant = authority.current_grant()?.unwrap();
            let request = request_from_reservation(&config, &reserved, head.timestamp_ms + 1, head.timestamp_ms + 1)?;
            let authorization = signer(10).authorize_commit(authority.policy(), &grant, &head, &request)?;
            let binding = journal.begin_fenced_commit(reserved.token, authority.policy().clone(), grant, authorization, &request)?;
            crash(stop, "after-bind");
            authority.commit(binding.authorization.clone(), request, head.timestamp_ms + 1)?;
            crash(stop, "after-commit");
            let observed = authority.reconcile_submission(&binding.authorization, &binding.commands)?;
            journal.reconcile_fenced(reserved.token, &observed, business_result)?;
            crash(stop, "after-result");
        }
        for entry in journal.entries(0, 256)? {
            journal.acknowledge(entry.original.identity.id)?;
        }
        crash(stop, "after-ack");
    }
    let head = authority.head()?;
    let port = command_inspector(config.clone(), Arc::new(Auth(kind)), move |bytes| Game::restore(kind, bytes));
    let inspection = authority.inspect_head(&port, b"state")?;
    let capabilities = CommandExecutor::new(config.clone(), Game::new(kind), Arc::new(Auth(kind)))?.capabilities();
    let replay_config = config.clone();
    let replay = head
        .finalised_height
        .map(|end| authority.replay_range(0, end, move |bytes| restore(&replay_config, kind, bytes)))
        .transpose()?;
    let entries = journal.entries(0, 256)?;
    let outcomes: Vec<_> = entries.iter().map(|e| serde_json::json!({"id":e.original.identity.id,"status":e.status})).collect();
    Ok(serde_json::json!({"kind":kind.name(),"program_digest":program,"genesis":config.commitment(),"height":head.finalised_height,
        "state_root":head.state_root,"state":serde_json::from_slice::<serde_json::Value>(&inspection.result)?,
        "capabilities":capabilities,"commands":outcomes,"replay":replay}))
}
pub fn main_for(kind: Kind) -> AnyResult<()> {
    let args: Vec<_> = std::env::args().skip(1).collect();
    if args.len() == 1 && args[0].len() == 64 && args[0].bytes().all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(&b)) {
        return command_bundle::replay(kind);
    }
    if args.len() == 4 && args[1] == "export" {
        let report = command_bundle::export(kind, Path::new(&args[0]), Path::new(&args[2]), Path::new(&args[3]))?;
        println!("{}", serde_json::to_string_pretty(&report)?);
        return Ok(());
    }
    if !(2..=3).contains(&args.len()) {
        return Err("usage: <example> <store-directory> demo|show|replay [crash-boundary] | <store-directory> export <artifacts.json> <new-bundle-directory>".into());
    }
    let selected = args.get(2).map(String::as_str);
    if selected.is_some_and(|s| {
        !["after-enqueue", "after-reserve", "after-bind", "after-commit", "after-result", "after-ack", "after-response"].contains(&s)
    }) {
        return Err("unknown development crash boundary".into());
    }
    let report = run(Path::new(&args[0]), kind, &args[1], selected)?;
    println!("{}", serde_json::to_string_pretty(&report)?);
    crash(selected, "after-response");
    Ok(())
}
