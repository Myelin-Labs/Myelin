//! Generate native observations for an independently executed browser checker.
#[path = "support/conformance_contract.rs"]
mod contract;
#[path = "support/command_games.rs"]
mod games;
use contract::{Observation, Outcome, Report, Scenario, Vector};
use games::{Action, Auth, Game, Kind};
use myelin_session::{CommandIdentity, SessionConfig, SessionHead, TransitionExecutor};
use myelin_session_journal::{application::*, BusinessOutcome, CommandAuthenticator, OriginalCommand};
use std::sync::Arc;

fn observe(
    config: &SessionConfig,
    kind: Kind,
    case: &Scenario,
    commands: &[OriginalCommand],
    width: usize,
    restart: bool,
) -> games::AnyResult<Observation> {
    let mut executor = games::restore(config, kind, &case.initial)?;
    let initial = snapshot::decode(&case.initial, case.profile, case.snapshot_limit)?;
    let mut head = SessionHead {
        session_id: config.session_id,
        finalised_height: None,
        block_hash: config.commitment(),
        state_root: executor.state_root(),
        timestamp_ms: 0,
        input_position: initial.cursor,
        logical_time: initial.logical_time,
        sealed_by_successor: None,
    };
    let mut outcomes = vec![];
    let mut error = None;
    for (index, group) in commands.chunks(width).enumerate() {
        let frame = command_frame(config, &head, group.to_vec())?;
        match executor.execute_block(999 - index as u64, &frame, &[], &[]) {
            Ok(result) => {
                for message in result.outbox {
                    let (_, result): (CommandIdentity, BusinessOutcome) = postcard::from_bytes(&message.payload)?;
                    outcomes.push(match result {
                        BusinessOutcome::Succeeded(payload) => Outcome { success: true, payload },
                        BusinessOutcome::Rejected(payload) => Outcome { success: false, payload },
                    });
                }
                head.input_position = frame.end_position;
                head.logical_time = frame.logical_time_end;
                head.timestamp_ms += 1703;
                head.state_root = executor.state_root();
                if restart {
                    executor = games::restore(config, kind, &executor.snapshot()?)?;
                }
            }
            Err(reason) => {
                error = Some(reason);
                break;
            }
        }
    }
    Ok(Observation { width, restart, snapshot: executor.snapshot()?, root: executor.state_root(), outcomes, error })
}

fn vector(
    kind: Kind,
    name: &str,
    actions: Option<Vec<Action>>,
    state: Option<(Game, u64)>,
    program: [u8; 32],
) -> games::AnyResult<Vector> {
    let config = games::config(kind, program);
    let id_offset = state.as_ref().map(|(_, tick)| *tick).filter(|tick| *tick < 128).unwrap_or(0) as u8;
    let commands = match actions {
        None => games::demo_commands(&config, kind),
        Some(actions) => actions
            .into_iter()
            .enumerate()
            .map(|(i, action)| games::signed(&config, [id_offset + i as u8 + 1; 32], action, 5))
            .collect(),
    };
    for command in &commands {
        Auth(kind).verify(command)?;
    }
    let profile = config.application_profile.commitment();
    let limit = config.commit_limits.max_snapshot_bytes;
    let initial = match state {
        None => CommandExecutor::new(config.clone(), Game::new(kind), Arc::new(Auth(kind)))?.snapshot()?,
        Some((game, tick)) => {
            snapshot::Snapshot { profile, cursor: tick, logical_time: tick, state: &game.snapshot()? }.encode(limit)?
        }
    };
    let scenario = Scenario {
        name: name.into(),
        kind: kind.name().into(),
        profile,
        snapshot_limit: limit,
        initial,
        commands: commands.iter().map(|c| c.canonical_command.clone()).collect(),
    };
    let mut expected = vec![];
    let mut widths: Vec<_> = [1, 2, 7, 16, 64].into_iter().map(|n| n.min(commands.len())).collect();
    widths.sort_unstable();
    widths.dedup();
    for width in widths {
        for restart in [false, true] {
            expected.push(observe(&config, kind, &scenario, &commands, width, restart)?);
        }
    }
    Ok(Vector { scenario, expected })
}
fn main() -> games::AnyResult<()> {
    use games::tick_game::{Event, Tick, World};
    let program = games::artifact_digest()?;
    let mut vectors = vec![];
    for kind in [Kind::Inventory, Kind::Turns, Kind::Ticks] {
        vectors.push(vector(kind, kind.name(), None, None, program)?);
    }
    let config = games::config(Kind::Ticks, program);
    let commands = games::demo_commands(&config, Kind::Ticks);
    let prefix = observe(&config, Kind::Ticks, &vectors[2].scenario, &commands[..3], 3, true)?;
    let saved = snapshot::decode(&prefix.snapshot, config.application_profile.commitment(), config.commit_limits.max_snapshot_bytes)?;
    let recovered = Game::restore(Kind::Ticks, saved.state)?;
    let remaining = games::tick_game::demo().into_iter().skip(3).map(Action::Tick).collect();
    vectors.push(vector(
        Kind::Ticks,
        "resume-native-checkpoint-after-disconnect",
        Some(remaining),
        Some((recovered, saved.cursor)),
        program,
    )?);
    let long = (0..128)
        .map(|tick| {
            Action::Tick(Tick {
                tick,
                events: match tick % 32 {
                    2 => vec![Event::Disconnect { seat: 1 }],
                    4 => vec![Event::Takeover { seat: 1 }],
                    8 => vec![Event::Reconnect { seat: 1 }],
                    _ => vec![],
                },
                inputs: [1, if (2..8).contains(&(tick % 32)) { 0 } else { -1 }],
            })
        })
        .collect();
    vectors.push(vector(Kind::Ticks, "repeated-disconnect-takeover-reconnect", Some(long), None, program)?);
    let mut early: Vec<_> = games::tick_game::demo().into_iter().take(3).map(Action::Tick).collect();
    early.push(Action::Tick(Tick { tick: 3, events: vec![Event::Takeover { seat: 1 }], inputs: [1, 0] }));
    vectors.push(vector(Kind::Ticks, "early-takeover-rollback", Some(early), None, program)?);
    for (name, tick) in [("stale-tick-rollback", 0), ("skipped-tick-rollback", 2)] {
        let commands = vec![
            Action::Tick(Tick { tick: 0, events: vec![], inputs: [1, -1] }),
            Action::Tick(Tick { tick, events: vec![], inputs: [1, -1] }),
        ];
        vectors.push(vector(Kind::Ticks, name, Some(commands), None, program)?);
    }
    let mut bounds = World::new();
    bounds.players[0].position = 999_999;
    bounds.players[0].velocity = 12;
    bounds.players[1].position = -999_999;
    bounds.players[1].velocity = -12;
    let commands = (0..3).map(|tick| Action::Tick(Tick { tick, events: vec![], inputs: [1, -1] })).collect();
    vectors.push(vector(Kind::Ticks, "synthetic-position-boundary", Some(commands), Some((Game::Ticks(bounds), 0)), program)?);
    let mut rng = World::new();
    rng.tick = u64::MAX / 2 - 1;
    rng.rng_counter = rng.tick * 2;
    let tick = rng.tick;
    let commands = (tick..tick + 3).map(|tick| Action::Tick(Tick { tick, events: vec![], inputs: [1, -1] })).collect();
    vectors.push(vector(Kind::Ticks, "synthetic-rng-exhaustion", Some(commands), Some((Game::Ticks(rng), tick)), program)?);
    println!(
        "{}",
        serde_json::to_string(&Report { format: "myelin-game-conformance-v1".into(), native_program_digest: program, vectors })?
    );
    Ok(())
}
