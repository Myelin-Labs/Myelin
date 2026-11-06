//! A checker for native-authenticated application inputs; it is not a journal,
//! signature verifier, authority, network service or CKB settlement verifier.
#[path = "../../support/conformance_contract.rs"]
#[allow(dead_code)]
mod contract;
#[path = "../../support/game_rules.rs"]
#[allow(dead_code)]
mod game_rules;
#[path = "../../../src/application_snapshot.rs"]
mod snapshot;
use contract::{Observation, Outcome, Scenario};
use game_rules::{Game, Kind};

fn restore(case: &Scenario, kind: Kind, bytes: &[u8]) -> Result<(Game, u64, u64), String> {
    let state = snapshot::decode(bytes, case.profile, case.snapshot_limit)?;
    let game = Game::restore(kind, state.state)?;
    if game.snapshot()? != state.state {
        return Err("noncanonical application state".into());
    }
    Ok((game, state.cursor, state.logical_time))
}

#[cfg_attr(target_arch = "wasm32", wasm_bindgen::prelude::wasm_bindgen)]
pub fn run_case(json: &str, width: usize, restart: bool) -> Result<String, String> {
    if json.len() > 4 * 1024 * 1024 || width == 0 || width > 64 {
        return Err("conformance input/width limit".into());
    }
    let case: Scenario = serde_json::from_str(json).map_err(|e| e.to_string())?;
    if case.commands.len() > 1024 {
        return Err("conformance command limit".into());
    }
    let kind = match case.kind.as_str() {
        "inventory" => Kind::Inventory,
        "turns" => Kind::Turns,
        "ticks" => Kind::Ticks,
        _ => return Err("unknown game kind".into()),
    };
    let mut bytes = case.initial.clone();
    let (mut game, mut cursor, mut time) = restore(&case, kind, &bytes)?;
    let mut outcomes = vec![];
    let mut error = None;
    for group in case.commands.chunks(width) {
        let mut candidate = game.clone();
        let mut position = cursor;
        let mut logical = time;
        let mut staged = vec![];
        for command in group {
            let mut speculative = candidate.clone();
            let result = (|| {
                let outcome = match speculative.apply_rule(position, logical, command)? {
                    Ok(()) => {
                        let payload = speculative.snapshot()?;
                        candidate = speculative;
                        Outcome { success: true, payload }
                    }
                    Err(reason) => Outcome { success: false, payload: reason.as_bytes().to_vec() },
                };
                position = position.checked_add(1).ok_or("input cursor overflow")?;
                logical = logical.checked_add(1).ok_or("logical time overflow")?;
                Ok::<_, String>(outcome)
            })();
            match result {
                Ok(outcome) => staged.push(outcome),
                Err(reason) => {
                    error = Some(reason);
                    break;
                }
            }
        }
        if error.is_some() {
            break;
        }
        bytes = snapshot::Snapshot { profile: case.profile, cursor: position, logical_time: logical, state: &candidate.snapshot()? }
            .encode(case.snapshot_limit)?;
        outcomes.extend(staged);
        (game, cursor, time) = if restart { restore(&case, kind, &bytes)? } else { (candidate, position, logical) };
    }
    serde_json::to_string(&Observation { width, restart, root: snapshot::state_root(&bytes), snapshot: bytes, outcomes, error })
        .map_err(|e| e.to_string())
}
