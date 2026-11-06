//! Pure application rules shared by native execution and browser verification.
use serde::{Deserialize, Serialize};
#[path = "tick_game.rs"]
pub mod tick_game;

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum Kind {
    Inventory,
    Turns,
    Ticks,
}
impl Kind {
    pub fn name(self) -> &'static str {
        match self {
            Self::Inventory => "inventory",
            Self::Turns => "turns",
            Self::Ticks => "ticks",
        }
    }
    pub fn id(self) -> u8 {
        match self {
            Self::Inventory => 51,
            Self::Turns => 52,
            Self::Ticks => 53,
        }
    }
}
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub enum Action {
    Buy { quantity: u64, expected_revision: u64 },
    Save { checkpoint: u32, expected_revision: u64 },
    Move { seat: u8, square: u8, expected_turn: u64 },
    Timeout { seat: u8, expected_turn: u64, elapsed_ms: u64 },
    Tick(tick_game::Tick),
}
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Inventory {
    pub credits: u64,
    pub potions: u64,
    pub checkpoint: u32,
    pub revision: u64,
}
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Turns {
    pub board: [u8; 9],
    pub next: u8,
    pub turn: u64,
    pub winner: Option<u8>,
    pub timed_out: Option<u8>,
}
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub enum Game {
    Inventory(Inventory),
    Turns(Turns),
    Ticks(tick_game::World),
}
impl Game {
    pub fn new(kind: Kind) -> Self {
        match kind {
            Kind::Inventory => Self::Inventory(Inventory { credits: 100, potions: 0, checkpoint: 0, revision: 0 }),
            Kind::Turns => Self::Turns(Turns { board: [0; 9], next: 0, turn: 0, winner: None, timed_out: None }),
            Kind::Ticks => Self::Ticks(tick_game::World::new()),
        }
    }
    pub fn restore(kind: Kind, bytes: &[u8]) -> std::result::Result<Self, String> {
        let value: Self = serde_json::from_slice(bytes).map_err(|e| e.to_string())?;
        match (&value, kind) {
            (Self::Inventory(s), Kind::Inventory)
                if s.potions <= 4
                    && s.credits <= 100
                    && s.credits + s.potions * 25 == 100
                    && s.checkpoint <= 100
                    && s.revision <= 4096 => {}
            (Self::Turns(s), Kind::Turns) if s.valid() => (),
            (Self::Ticks(s), Kind::Ticks) if s.valid() => (),
            _ => return Err("invalid game snapshot/kind".into()),
        }
        Ok(value)
    }
    fn transition(&mut self, action: Action) -> std::result::Result<(), &'static str> {
        match (self, action) {
            (Self::Inventory(s), Action::Buy { quantity, expected_revision }) => {
                if expected_revision != s.revision {
                    return Err("stale inventory revision");
                }
                if s.revision >= 4096 {
                    return Err("inventory revision limit");
                }
                if quantity == 0 || quantity > 4 || quantity * 25 > s.credits {
                    return Err("insufficient balance or invalid quantity");
                }
                s.credits -= quantity * 25;
                s.potions += quantity;
                s.revision += 1;
            }
            (Self::Inventory(s), Action::Save { checkpoint, expected_revision }) => {
                if expected_revision != s.revision {
                    return Err("stale inventory revision");
                }
                if s.revision >= 4096 {
                    return Err("inventory revision limit");
                }
                if checkpoint > 100 {
                    return Err("invalid checkpoint");
                }
                s.checkpoint = checkpoint;
                s.revision += 1;
            }
            (Self::Turns(s), Action::Move { seat, square, expected_turn }) => {
                if s.winner.is_some() {
                    return Err("game finished");
                }
                if expected_turn != s.turn || seat != s.next {
                    return Err("stale or wrong turn");
                }
                if square >= 9 || s.board[square as usize] != 0 {
                    return Err("occupied or invalid square");
                }
                s.board[square as usize] = seat + 1;
                s.turn += 1;
                s.next ^= 1;
                s.winner = s.board_winner().or_else(|| (s.turn == 9).then_some(2));
            }
            (Self::Turns(s), Action::Timeout { seat, expected_turn, elapsed_ms }) => {
                if s.winner.is_some() || seat != s.next || expected_turn != s.turn {
                    return Err("stale timeout");
                }
                if elapsed_ms < 30_000 {
                    return Err("deadline not reached");
                }
                s.timed_out = Some(seat);
                s.winner = Some(seat ^ 1);
            }
            _ => return Err("unsupported game action"),
        }
        Ok(())
    }
}
impl Turns {
    fn board_winner(&self) -> Option<u8> {
        for [a, b, c] in [[0, 1, 2], [3, 4, 5], [6, 7, 8], [0, 3, 6], [1, 4, 7], [2, 5, 8], [0, 4, 8], [2, 4, 6]] {
            if self.board[a] != 0 && self.board[a] == self.board[b] && self.board[a] == self.board[c] {
                return Some(self.board[a] - 1);
            }
        }
        None
    }
    fn valid(&self) -> bool {
        let a = self.board.iter().filter(|v| **v == 1).count() as u64;
        let b = self.board.iter().filter(|v| **v == 2).count() as u64;
        if self.board.iter().any(|v| *v > 2) || !(a == b || a == b + 1) || self.turn != a + b || self.next != (self.turn % 2) as u8 {
            return false;
        }
        if let Some(seat) = self.timed_out {
            seat == self.next && self.board_winner().is_none() && self.turn < 9 && self.winner == Some(seat ^ 1)
        } else {
            self.winner == self.board_winner().or_else(|| (self.turn == 9).then_some(2))
        }
    }
}
impl Game {
    pub fn snapshot(&self) -> Result<Vec<u8>, String> {
        serde_json::to_vec(self).map_err(|e| e.to_string())
    }
    pub fn apply_rule(&mut self, position: u64, time: u64, bytes: &[u8]) -> Result<Result<(), &'static str>, String> {
        let action: Action = serde_json::from_slice(bytes).map_err(|e| e.to_string())?;
        if serde_json::to_vec(&action).map_err(|e| e.to_string())? != bytes {
            return Err("noncanonical action".into());
        }
        if let Self::Ticks(world) = self {
            let Action::Tick(tick) = action else { return Err("fixed ticks require a tick packet".into()) };
            if position != world.tick || time != world.tick {
                return Err("simulation tick differs from adapter clock".into());
            }
            world.step(&tick)?;
            return Ok(Ok(()));
        }
        Ok(self.transition(action))
    }
}
