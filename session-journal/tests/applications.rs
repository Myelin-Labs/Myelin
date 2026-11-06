#[path = "../examples/support/command_games.rs"]
mod games;
use games::{Action, Auth, Game, Kind};
use myelin_session::*;
use myelin_session_journal::{application::*, *};
use std::{path::Path, sync::Arc};

fn head(config: &SessionConfig) -> SessionHead {
    SessionHead {
        session_id: config.session_id,
        finalised_height: None,
        block_hash: config.commitment(),
        state_root: config.initial_state_root,
        timestamp_ms: 0,
        input_position: 0,
        logical_time: 0,
        sealed_by_successor: None,
    }
}
fn executor(kind: Kind, config: &SessionConfig) -> CommandExecutor<Game> {
    CommandExecutor::new(config.clone(), Game::new(kind), Arc::new(Auth(kind))).unwrap()
}
fn assert_final(kind: Kind, state: &serde_json::Value) {
    match kind {
        Kind::Inventory => {
            assert_eq!(state["Inventory"]["credits"], 75);
            assert_eq!(state["Inventory"]["potions"], 1);
            assert_eq!(state["Inventory"]["checkpoint"], 3);
            assert_eq!(state["Inventory"]["revision"], 2);
        }
        Kind::Turns => {
            assert_eq!(state["Turns"]["board"], serde_json::json!([1, 1, 0, 0, 2, 0, 0, 0, 0]));
            assert_eq!(state["Turns"]["turn"], 3);
            assert_eq!(state["Turns"]["winner"], 0);
            assert_eq!(state["Turns"]["timed_out"], 1);
        }
        Kind::Ticks => {
            assert_eq!(state["Ticks"]["tick"], 12);
            assert_eq!(state["Ticks"]["rng_counter"], 24);
            assert_eq!(state["Ticks"]["players"][0]["position"], 126);
            assert_eq!(state["Ticks"]["players"][0]["control"], "Player");
            assert_eq!(state["Ticks"]["players"][1]["control"], "Player");
        }
    }
}
#[test]
fn identical_commands_ignore_batch_height_and_restart_boundaries() {
    for kind in [Kind::Inventory, Kind::Turns, Kind::Ticks] {
        let config = games::config(kind, [11; 32]);
        let commands = games::demo_commands(&config, kind);
        let mut expected = None;
        for width in 1..=commands.len() {
            let mut executor = executor(kind, &config);
            let mut head = head(&config);
            let mut outcomes = vec![];
            for (batch, group) in commands.chunks(width).enumerate() {
                let frame = command_frame(&config, &head, group.to_vec()).unwrap();
                let result = executor.execute_block(999 - batch as u64, &frame, &[], &[]).unwrap();
                assert_eq!(result.cycles, 0);
                assert!(result.ordered_cell_tx_commitments.is_empty());
                for result in result.outbox {
                    let (identity, outcome): (CommandIdentity, BusinessOutcome) = postcard::from_bytes(&result.payload).unwrap();
                    outcomes.push((identity, outcome));
                }
                head.input_position = frame.end_position;
                head.logical_time = frame.logical_time_end;
                head.timestamp_ms += 1000;
                head.state_root = executor.state_root();
                executor = games::restore(&config, kind, &executor.snapshot().unwrap()).unwrap();
            }
            assert_final(kind, &serde_json::from_slice(&executor.inspect(b"state").unwrap()).unwrap());
            let actual = (executor.state_root(), executor.snapshot().unwrap(), outcomes);
            match &expected {
                None => expected = Some(actual),
                Some(expected) => assert_eq!(&actual, expected),
            }
        }
    }
}
#[test]
fn forged_seat_system_kind_and_retargeted_commands_leave_entire_batch_unchanged() {
    let config = games::config(Kind::Turns, [11; 32]);
    let commands = games::demo_commands(&config, Kind::Turns);
    let mut bad = vec![
        games::signed(&config, [22; 32], Action::Move { seat: 1, square: 4, expected_turn: 1 }, 3),
        games::signed(&config, [23; 32], Action::Timeout { seat: 1, expected_turn: 1, elapsed_ms: 30_000 }, 3),
    ];
    let mut forged = commands[1].clone();
    forged.authorization[0] ^= 1;
    bad.push(forged);
    let mut kind = commands[1].clone();
    kind.kind = InputKind::SystemEvent;
    bad.push(kind);
    let mut payload = commands[1].clone();
    payload.canonical_command.push(b' ');
    bad.push(payload);
    for command in bad {
        let mut e = executor(Kind::Turns, &config);
        let initial = e.snapshot().unwrap();
        let frame = command_frame(&config, &head(&config), vec![commands[0].clone(), command]).unwrap();
        assert!(e.execute_block(0, &frame, &[], &[]).is_err());
        assert_eq!(e.snapshot().unwrap(), initial);
    }
    let mut retargeted = commands[0].clone();
    retargeted.scope.session_id = [99; 32];
    assert!(command_frame(&config, &head(&config), vec![retargeted]).is_err());
}

#[test]
fn tick_packets_enforce_clock_controls_and_authority_atomically() {
    use games::tick_game::{Event, Tick};
    let config = games::config(Kind::Ticks, [11; 32]);
    let commands = games::demo_commands(&config, Kind::Ticks);
    let cases = [
        (Tick { tick: 0, events: vec![], inputs: [0, 0] }, 5), // stale after first packet
        (Tick { tick: 2, events: vec![], inputs: [0, 0] }, 5), // skipped tick
        (Tick { tick: 1, events: vec![Event::Takeover { seat: 1 }], inputs: [0, 0] }, 5),
        (Tick { tick: 1, events: vec![Event::Disconnect { seat: 2 }], inputs: [0, 0] }, 5),
        (Tick { tick: 1, events: vec![Event::Disconnect { seat: 1 }], inputs: [0, 1] }, 5),
        (Tick { tick: 1, events: vec![Event::Reconnect { seat: 1 }], inputs: [0, 0] }, 5),
        (Tick { tick: 1, events: vec![Event::Disconnect { seat: 1 }, Event::Disconnect { seat: 0 }], inputs: [0, 0] }, 5),
        (Tick { tick: 1, events: vec![Event::Disconnect { seat: 1 }, Event::Reconnect { seat: 1 }], inputs: [0, 0] }, 5),
        (Tick { tick: 1, events: vec![], inputs: [2, 0] }, 5),
        (Tick { tick: 1, events: vec![], inputs: [0, 0] }, 3), // player cannot sign sequencer packets
    ];
    for (packet, signer) in cases {
        let mut e = executor(Kind::Ticks, &config);
        let initial = e.snapshot().unwrap();
        let frame = command_frame(
            &config,
            &head(&config),
            vec![commands[0].clone(), games::signed(&config, [22; 32], Action::Tick(packet), signer)],
        )
        .unwrap();
        assert!(e.execute_block(0, &frame, &[], &[]).is_err());
        assert_eq!(e.snapshot().unwrap(), initial);
    }
    let mut e = executor(Kind::Ticks, &config);
    let initial = e.snapshot().unwrap();
    let early =
        games::signed(&config, [23; 32], Action::Tick(Tick { tick: 3, events: vec![Event::Takeover { seat: 1 }], inputs: [0, 0] }), 5);
    let frame = command_frame(&config, &head(&config), [commands[..3].to_vec(), vec![early]].concat()).unwrap();
    assert!(e.execute_block(0, &frame, &[], &[]).is_err());
    assert_eq!(e.snapshot().unwrap(), initial);

    let c = e.capabilities();
    assert!(c.tick_tapes);
    assert!(!c.authenticated_system_events);
    let mut wrong_kind = commands[0].clone();
    wrong_kind.kind = InputKind::UserCommand;
    assert!(command_frame(&config, &head(&config), vec![wrong_kind]).is_err());
    let mut foreign = config.clone();
    foreign.application_profile.logical_time_policy_hash = [13; 32];
    assert!(CommandExecutor::new(foreign, Game::new(Kind::Ticks), Arc::new(Auth(Kind::Ticks))).is_err());
}
#[test]
fn snapshot_profile_and_canonical_encoding_are_required_and_capabilities_are_enforced() {
    let config = games::config(Kind::Inventory, [11; 32]);
    let mut e = executor(Kind::Inventory, &config);
    let bytes = e.snapshot().unwrap();
    let mut foreign = config.clone();
    foreign.application_profile.program_digest = [12; 32];
    assert!(CommandExecutor::<Game>::restore(foreign, &bytes, Arc::new(Auth(Kind::Inventory)), |_| panic!(
        "foreign code must not load"
    ))
    .is_err());
    for data in [vec![0; 5], [bytes.as_slice(), &[0]].concat()] {
        assert!(games::restore(&config, Kind::Inventory, &data).is_err());
    }
    assert!(snapshot::decode(&bytes, config.application_profile.commitment(), bytes.len() as u64 - 1).is_err());
    let mut forged_length = b"MCAS\x01".to_vec();
    forged_length.extend(postcard::to_allocvec(&(config.application_profile.commitment(), 0u64, 0u64, u64::MAX)).unwrap());
    assert!(games::restore(&config, Kind::Inventory, &forged_length).is_err());
    let frame = command_frame(&config, &head(&config), vec![]).unwrap();
    assert!(e.execute_block(0, &frame, &[], &[vec![1]]).is_err());
    let mut tape = games::demo_commands(&config, Kind::Inventory)[0].clone();
    tape.kind = InputKind::InputTape;
    assert!(command_frame(&config, &head(&config), vec![tape]).is_err());
    let c = e.capabilities();
    assert!(!c.tick_tapes && !c.handoffs && !c.successors && !c.settlement && !c.ckb_vm_execution);
    assert!(e.inspect(&[0; 33]).is_err());
    assert!(e.inspect(b"private-key").is_err());
    let mut invalid = config;
    invalid.protocol_effects = ProtocolEffectsPolicy::AuthorityMetadata { policy_hash: [1; 32] };
    assert!(CommandExecutor::new(invalid, Game::new(Kind::Inventory), Arc::new(Auth(Kind::Inventory))).is_err());
}
#[test]
fn too_small_batch_keeps_originals_queued_and_wrong_payload_cannot_reuse_id() {
    let root = tempfile::tempdir().unwrap();
    let config = games::config(Kind::Inventory, [11; 32]);
    let journal = CommandJournal::open(root.path(), &config, JournalLimits::default()).unwrap();
    let commands = games::demo_commands(&config, Kind::Inventory);
    for command in &commands {
        journal.enqueue(command.clone(), &Auth(Kind::Inventory)).unwrap();
    }
    assert!(journal.reserve_commands(&head(&config), 3, 1, &Auth(Kind::Inventory)).unwrap().is_none());
    assert_eq!(journal.entry(commands[0].identity.id).unwrap().unwrap().status, EntryStatus::Queued);
    let changed = games::signed(&config, [1; 32], Action::Buy { quantity: 2, expected_revision: 0 }, 3);
    assert!(matches!(journal.enqueue(changed, &Auth(Kind::Inventory)), Err(JournalError::IdentityConflict)));
    let r = journal.reserve_commands(&head(&config), 2, 100_000, &Auth(Kind::Inventory)).unwrap().unwrap();
    let request = request_from_reservation(&config, &r, 1, 1).unwrap();
    assert_eq!(request.frame_input.commands, commands[..2].iter().map(|c| c.identity.clone()).collect::<Vec<_>>());
    assert!(request.transactions.is_empty());
}

#[derive(Clone)]
struct MutatingRejection(Game);
impl CommandProgram for MutatingRejection {
    fn clock(&self) -> ApplicationClock {
        self.0.clock()
    }
    fn snapshot(&self) -> std::result::Result<Vec<u8>, String> {
        self.0.snapshot()
    }
    fn inspect(&self, query: &[u8]) -> std::result::Result<Vec<u8>, String> {
        self.0.inspect(query)
    }
    fn apply(&mut self, _: CommandContext, _: &OriginalCommand) -> std::result::Result<BusinessOutcome, String> {
        self.0 = Game::Inventory(games::game_rules::Inventory { credits: 0, potions: 4, checkpoint: 0, revision: 1 });
        Ok(BusinessOutcome::Rejected(b"business refused after speculative work".to_vec()))
    }
}
#[test]
fn rejected_commands_record_outcomes_but_rejected_ticks_cannot_advance_time() {
    for kind in [Kind::Inventory, Kind::Ticks] {
        let config = games::config(kind, [11; 32]);
        let mut e = CommandExecutor::new(config.clone(), MutatingRejection(Game::new(kind)), Arc::new(Auth(kind))).unwrap();
        let initial = e.snapshot().unwrap();
        let initial_game = e.inspect(b"state").unwrap();
        let command = games::demo_commands(&config, kind).remove(0);
        let frame = command_frame(&config, &head(&config), vec![command]).unwrap();
        let result = e.execute_block(0, &frame, &[], &[]);
        assert_eq!(e.inspect(b"state").unwrap(), initial_game);
        if kind == Kind::Ticks {
            assert!(result.is_err());
            assert_eq!(e.snapshot().unwrap(), initial);
        } else {
            let result = result.unwrap();
            assert_ne!(e.state_root(), config.initial_state_root); // Cursor/time advanced.
            let (_, outcome): (CommandIdentity, BusinessOutcome) = postcard::from_bytes(&result.outbox[0].payload).unwrap();
            assert!(matches!(outcome, BusinessOutcome::Rejected(_)));
        }
    }
}
#[test]
fn inspection_checks_snapshot_root_profile_and_current_pruned_checkpoint() {
    let root = tempfile::tempdir().unwrap();
    games::run(root.path(), Kind::Inventory, "demo", None).unwrap();
    let (authority, config) = games::open(root.path(), Kind::Inventory, games::artifact_digest().unwrap()).unwrap();
    assert!(authority.block(2).unwrap().state_snapshot.is_empty());
    let inspector = command_inspector(config.clone(), Arc::new(Auth(Kind::Inventory)), |bytes| Game::restore(Kind::Inventory, bytes));
    let inspected = authority.inspect_head(&inspector, b"state").unwrap();
    assert_eq!(inspected.state_root, authority.head().unwrap().state_root);
    assert_final(Kind::Inventory, &serde_json::from_slice(&inspected.result).unwrap());
    let initial = executor(Kind::Inventory, &config).snapshot().unwrap();
    let context = InspectContext {
        session_id: config.session_id,
        finalised_height: Some(2),
        state_root: inspected.state_root,
        application_profile: &config.application_profile,
    };
    assert!(inspector.inspect(&context, &initial, b"state").is_err());
}
#[test]
fn business_results_require_exact_receipt_and_one_matching_outbox_record() {
    let root = tempfile::tempdir().unwrap();
    games::run(root.path(), Kind::Inventory, "demo", None).unwrap();
    let (authority, _) = games::open(root.path(), Kind::Inventory, games::artifact_digest().unwrap()).unwrap();
    let record = authority.block(0).unwrap();
    let receipt = CommandReceipt::for_record(&record, 0).unwrap();
    assert!(matches!(business_result(&record, &receipt).unwrap(), Some(BusinessOutcome::Succeeded(_))));
    let mut wrong = receipt.clone();
    wrong.command.payload_hash = [9; 32];
    assert!(business_result(&record, &wrong).is_err());
    let mut duplicate = record.clone();
    duplicate.outbox.push(record.outbox[0].clone());
    assert!(business_result(&duplicate, &receipt).is_err());
    let mut absent = record;
    absent.outbox.clear();
    assert_eq!(business_result(&absent, &receipt).unwrap(), None);
}
#[test]
fn examples_restart_without_reissuing_and_reject_changed_program_or_missing_originals() {
    for kind in [Kind::Inventory, Kind::Turns, Kind::Ticks] {
        let root = tempfile::tempdir().unwrap();
        let before = games::run(root.path(), kind, "demo", None).unwrap();
        assert_final(kind, &before["state"]);
        for _ in 0..2 {
            let after = games::run(root.path(), kind, "demo", None).unwrap();
            assert_eq!(after, before);
            assert_eq!(games::run(root.path(), kind, "replay", None).unwrap(), before);
        }
        assert!(games::open(root.path(), kind, [99; 32]).is_err());
        std::fs::rename(root.path().join("journal"), root.path().join("preserved-journal")).unwrap();
        assert!(games::run(root.path(), kind, "demo", None).is_err());
        assert!(!root.path().join("journal").exists());
    }
    let root = tempfile::tempdir().unwrap();
    let missing = root.path().join("missing");
    assert!(games::run(&missing, Kind::Inventory, "replay", None).is_err());
    assert!(!missing.exists());
}
#[test]
fn command_game_crash_child() {
    let Ok(root) = std::env::var("MYELIN_COMMAND_GAME_CRASH_ROOT") else { return };
    let kind = match std::env::var("MYELIN_COMMAND_GAME_KIND").unwrap().as_str() {
        "inventory" => Kind::Inventory,
        "turns" => Kind::Turns,
        "ticks" => Kind::Ticks,
        _ => panic!("unknown example"),
    };
    let point = std::env::var("MYELIN_COMMAND_GAME_CRASH_POINT").unwrap();
    let report = games::run(Path::new(&root), kind, "demo", Some(&point)).unwrap();
    if point == "after-response" {
        println!("{}", serde_json::to_string(&report).unwrap());
        std::process::exit(76);
    }
    panic!("crash boundary was not reached");
}
#[test]
fn examples_recover_each_process_boundary_twice() {
    use std::{
        process::{Command, Stdio},
        time::{Duration, Instant},
    };
    for kind in [Kind::Inventory, Kind::Turns, Kind::Ticks] {
        for point in ["after-enqueue", "after-reserve", "after-bind", "after-commit", "after-result", "after-ack", "after-response"] {
            let root = tempfile::tempdir().unwrap();
            let response_path = root.path().join("response.log");
            let mut child = Command::new(std::env::current_exe().unwrap())
                .args(["--exact", "command_game_crash_child", "--nocapture"])
                .env("MYELIN_COMMAND_GAME_CRASH_ROOT", root.path())
                .env("MYELIN_COMMAND_GAME_KIND", kind.name())
                .env("MYELIN_COMMAND_GAME_CRASH_POINT", point)
                .stdout(Stdio::from(std::fs::File::create(&response_path).unwrap()))
                .stderr(Stdio::inherit())
                .spawn()
                .unwrap();
            let deadline = Instant::now() + Duration::from_secs(15);
            let status = loop {
                if let Some(status) = child.try_wait().unwrap() {
                    break status;
                }
                if Instant::now() > deadline {
                    let _ = child.kill();
                    let _ = child.wait();
                    panic!("crash child timed out: {point}");
                }
                std::thread::sleep(Duration::from_millis(10));
            };
            assert_eq!(status.code(), Some(76), "{kind:?}/{point}");
            let mut previous = None;
            for _ in 0..2 {
                let report = games::run(root.path(), kind, "demo", None).unwrap();
                assert_final(kind, &report["state"]);
                assert_eq!(report["replay"]["state_root_after"], report["state_root"]);
                assert_eq!(report["replay"]["total_cycles"], 0);
                for command in report["commands"].as_array().unwrap() {
                    assert_eq!(command["status"]["Committed"]["acknowledged"], true);
                }
                match &previous {
                    None => previous = Some(report),
                    Some(previous) => assert_eq!(&report, previous),
                }
            }
            if point == "after-response" {
                assert!(std::fs::read_to_string(response_path).unwrap().contains("\"state_root\""));
            }
        }
    }
}
