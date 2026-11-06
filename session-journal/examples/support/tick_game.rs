//! Pure, integer simulation rules. No storage, network, wall clock or VM APIs.
use serde::{Deserialize, Serialize};

pub const TICK_DURATION_US: u64 = 20_000;
pub const RNG_POLICY: &[u8] = b"tick-example:blake3(domain,seed,counter-le64);two-draws-per-tick;public-seed=17x32;1";
const SEED: [u8; 32] = [17; 32];
const MAX_POSITION: i32 = 1_000_000;

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub enum Event {
    Disconnect { seat: u8 },
    Takeover { seat: u8 },
    Reconnect { seat: u8 },
}
impl Event {
    fn seat(&self) -> usize {
        match self {
            Self::Disconnect { seat } | Self::Takeover { seat } | Self::Reconnect { seat } => usize::from(*seat),
        }
    }
}
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Tick {
    pub tick: u64,
    pub events: Vec<Event>,
    pub inputs: [i8; 2],
}
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub enum Control {
    Player,
    Disconnected { since: u64 },
    Ai { since: u64 },
}
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Player {
    pub position: i32,
    pub velocity: i32,
    pub control: Control,
}
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct World {
    pub tick: u64,
    pub seed: [u8; 32],
    pub rng_counter: u64,
    pub players: [Player; 2],
}
impl World {
    pub fn new() -> Self {
        Self {
            tick: 0,
            seed: SEED,
            rng_counter: 0,
            players: std::array::from_fn(|_| Player { position: 0, velocity: 0, control: Control::Player }),
        }
    }
    pub fn valid(&self) -> bool {
        self.seed == SEED
            && self.tick.checked_mul(2) == Some(self.rng_counter)
            && self.players.iter().all(|p| {
                (-MAX_POSITION..=MAX_POSITION).contains(&p.position)
                    && (-12..=12).contains(&p.velocity)
                    && match p.control {
                        Control::Player => true,
                        Control::Disconnected { since } => since < self.tick,
                        Control::Ai { since } => since.checked_add(2).is_some_and(|earliest| earliest < self.tick),
                    }
            })
    }
    pub fn step(&mut self, input: &Tick) -> Result<(), &'static str> {
        if !self.valid() || input.tick != self.tick || input.events.len() > 2 || input.inputs.iter().any(|v| !(-1..=1).contains(v)) {
            return Err("invalid, stale or skipped tick");
        }
        // Validate the entire packet on an isolated copy before advancing time.
        let mut next = self.clone();
        let mut previous = None;
        for event in &input.events {
            let seat = event.seat();
            if seat >= 2 || previous.is_some_and(|last| last >= seat) {
                return Err("events must name distinct admitted seats in ascending order");
            }
            previous = Some(seat);
            let p = &mut next.players[seat];
            p.control = match (event, &p.control) {
                (Event::Disconnect { .. }, Control::Player) => Control::Disconnected { since: self.tick },
                (Event::Takeover { .. }, Control::Disconnected { since })
                    if since.checked_add(2).is_some_and(|earliest| self.tick >= earliest) =>
                {
                    Control::Ai { since: *since }
                }
                (Event::Reconnect { .. }, Control::Disconnected { .. } | Control::Ai { .. }) => Control::Player,
                _ => return Err("invalid control transition or early takeover"),
            };
        }
        for (seat, p) in next.players.iter_mut().enumerate() {
            let mut rng = blake3::Hasher::new();
            rng.update(b"myelin:tick-example-rng:v1");
            rng.update(&self.seed);
            rng.update(&next.rng_counter.to_le_bytes());
            let draw = u32::from_le_bytes(rng.finalize().as_bytes()[..4].try_into().unwrap());
            next.rng_counter = next.rng_counter.checked_add(1).ok_or("RNG counter exhausted")?;
            if !matches!(p.control, Control::Player) && input.inputs[seat] != 0 {
                return Err("non-player seat must have neutral external input");
            }
            let direction = match p.control {
                Control::Player => i32::from(input.inputs[seat]),
                Control::Disconnected { .. } => 0,
                Control::Ai { .. } => (draw % 3) as i32 - 1,
            };
            p.velocity = (p.velocity + 3 * direction).clamp(-12, 12);
            p.position = (p.position + p.velocity).clamp(-MAX_POSITION, MAX_POSITION);
        }
        next.tick = self.tick.checked_add(1).ok_or("tick counter exhausted")?;
        *self = next;
        Ok(())
    }
}

pub fn demo() -> Vec<Tick> {
    (0..12)
        .map(|tick| Tick {
            tick,
            events: match tick {
                2 => vec![Event::Disconnect { seat: 1 }],
                4 => vec![Event::Takeover { seat: 1 }],
                8 => vec![Event::Reconnect { seat: 1 }],
                _ => vec![],
            },
            inputs: [1, if (2..8).contains(&tick) { 0 } else { -1 }],
        })
        .collect()
}
