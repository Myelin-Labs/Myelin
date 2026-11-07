#![no_std]
#![no_main]
use ckb_std::{ckb_constants::Source, error::SysError, high_level::*, syscalls};
ckb_std::entry!(program_entry);
ckb_std::default_alloc!();

// Fixed bytes, current-only schema. No dynamic nested proof decoding on chain.
const N: usize = 365;
const MAGIC: &[u8] = b"MGC1";
const SCHEMA: &[u8] =
    b"myelin:microgame:state365:program-schema-vm-game-players-stake-timeout-seq-phase-counter-turn-delta-claimed-root-post-da:v1";
const VM: &[u8] = b"ckb:Data2:VM2:riscv64imc:strict-no-spawn:no-oracle:10000000-cycles:v1";
const ROOT: &[u8] = b"myelin:microgame:logical-state:v1";
const MOVE: &[u8] = b"myelin:microgame:move-da:v1";
const GOAL: u8 = 5;
const RELATIVE_BLOCK: u64 = 1 << 63;
// Offsets: program4, schema36, VM68, game100, players132/164, stake196,
// timeout204, seq212, phase220, counter221, turn222, delta223, claimed224,
// logical root225, proposed post257, independently retrievable input root289.
// Final 44 bytes are reserved zero and checked, so no hidden state is accepted.
#[derive(Clone)]
struct State([u8; N]);
fn hash(parts: &[&[u8]]) -> [u8; 32] {
    let mut h = ckb_hash::new_blake2b();
    for part in parts {
        h.update(part);
    }
    let mut out = [0; 32];
    h.finalize(&mut out);
    out
}
impl State {
    fn u64(&self, offset: usize) -> u64 {
        u64::from_le_bytes(self.0[offset..offset + 8].try_into().unwrap())
    }
    fn phase(&self) -> u8 {
        self.0[220]
    }
    fn counter(&self) -> u8 {
        self.0[221]
    }
    fn turn(&self) -> u8 {
        self.0[222]
    }
    fn root(&self) -> [u8; 32] {
        hash(&[ROOT, &self.0[4..212], &self.0[212..220], &[u8::from(self.phase() == 2), self.counter(), self.turn()]])
    }
    fn da(&self) -> [u8; 32] {
        hash(&[MOVE, &self.0[4..212], &self.0[212..220], &self.0[225..257], &self.0[222..225], &self.0[257..289]])
    }
    fn validate(&self) -> Result<(), i8> {
        let script = load_script().map_err(|_| 2)?;
        if &self.0[..4] != MAGIC
            || self.0[4..36] != script.code_hash().raw_data()[..]
            || self.0[36..68] != hash(&[SCHEMA])
            || self.0[68..100] != hash(&[VM])
            || self.0[100..132] != script.args().raw_data()[..]
            || self.0[132..164] == self.0[164..196]
            || self.0[132..164] == [0; 32]
            || self.0[164..196] == [0; 32]
            || self.u64(196) < 100_000_000_000
            || self.u64(196) > u64::MAX / 2
            || self.u64(204) == 0
            || self.u64(204) > 128
            || self.phase() > 2
            || self.counter() > GOAL
            || self.turn() > 1
            || self.0[225..257] != self.root()
            || self.0[321..] != [0; 44]
        {
            return Err(10);
        }
        if self.phase() != 1 && self.0[223..225] != [0; 2] {
            return Err(11);
        }
        if self.phase() != 1 && self.0[257..321] != [0; 64] {
            return Err(11);
        }
        if self.phase() == 1 && (!(1..=2).contains(&self.0[223]) || self.0[289..321] != self.da()) {
            return Err(12);
        }
        if self.phase() == 0 && self.counter() == GOAL {
            return Err(13);
        }
        Ok(())
    }
    fn moved(&self) -> Result<State, i8> {
        let count = self.counter().checked_add(self.0[223]).filter(|n| *n <= GOAL).ok_or(20)?;
        let mut next = self.clone();
        next.0[212..220].copy_from_slice(&self.u64(212).checked_add(1).ok_or(20)?.to_le_bytes());
        next.0[220] = if count == GOAL { 2 } else { 0 };
        next.0[221] = count;
        next.0[222] = if count == GOAL { self.turn() } else { self.turn() ^ 1 };
        next.0[223..225].fill(0);
        next.0[257..321].fill(0);
        let root = next.root();
        next.0[225..257].copy_from_slice(&root);
        Ok(next)
    }
    fn valid_proposal(&self) -> bool {
        self.moved().is_ok_and(|next| self.0[224] == next.counter() && self.0[257..289] == next.0[225..257])
    }
    fn fraud(&self) -> Result<State, i8> {
        let mut next = self.clone();
        next.0[220] = 2;
        next.0[222] ^= 1;
        next.0[212..220].copy_from_slice(&self.u64(212).checked_add(1).ok_or(20)?.to_le_bytes());
        next.0[223..225].fill(0);
        next.0[257..321].fill(0);
        let root = next.root();
        next.0[225..257].copy_from_slice(&root);
        Ok(next)
    }
}
fn state(source: Source) -> Result<Option<State>, i8> {
    let mut bytes = [0; N];
    match syscalls::load_cell_data(&mut bytes, 0, 0, source) {
        Ok(n) if n == N => {
            let state = State(bytes);
            state.validate()?;
            Ok(Some(state))
        }
        Err(SysError::IndexOutOfBound) => Ok(None),
        _ => Err(3),
    }
}
fn authorized(state: &State, player: u8) -> bool {
    let expected = &state.0[132 + player as usize * 32..164 + player as usize * 32];
    QueryIter::new(load_cell_lock_hash, Source::Input).any(|hash| hash == expected)
}
fn capacity(source: Source) -> Result<u64, i8> {
    load_cell_capacity(0, source).map_err(|_| 4)
}
fn public_lock() -> [u8; 32] {
    let code = [
        0x28, 0xe8, 0x3a, 0x12, 0x77, 0xd4, 0x8a, 0xdd, 0x8e, 0x72, 0xfa, 0xda, 0xa9, 0x24, 0x85, 0x59, 0xe1, 0xb6, 0x32, 0xba, 0xb2,
        0xbd, 0x60, 0xb2, 0x79, 0x55, 0xeb, 0xc4, 0xc0, 0x38, 0x00, 0xa5,
    ];
    // Molecule Script table: total53, field offsets16/48/49, byte hash_type0,
    // empty Bytes has a four-byte item count. This hash never depends on a relay.
    hash(&[&53u32.to_le_bytes(), &16u32.to_le_bytes(), &48u32.to_le_bytes(), &49u32.to_le_bytes(), &code, &[0], &0u32.to_le_bytes()])
}
fn payout(state: &State, index: usize, player: u8, amount: u64) -> Result<(), i8> {
    if load_cell_capacity(index, Source::Output).map_err(|_| 30)? != amount
        || load_cell_lock_hash(index, Source::Output).map_err(|_| 30)?
            != state.0[132 + player as usize * 32..164 + player as usize * 32]
        || load_cell_type_hash(index, Source::Output).map_err(|_| 30)?.is_some()
        || !load_cell_data(index, Source::Output).map_err(|_| 30)?.is_empty()
    {
        return Err(30);
    }
    Ok(())
}
fn verify() -> Result<(), i8> {
    let script = load_script().map_err(|_| 2)?;
    if script.args().raw_data().len() != 32 || script.hash_type().as_slice() != [4] {
        return Err(2);
    }
    ckb_std::type_id::check_type_id(0, 32).map_err(|_| 5)?;
    let before = state(Source::GroupInput)?;
    let after = state(Source::GroupOutput)?;
    if let Some(output) = &after {
        if load_cell_lock_hash(0, Source::GroupOutput).map_err(|_| 4)? != public_lock()
            || capacity(Source::GroupOutput)? != output.u64(196) * 2
        {
            return Err(14);
        }
    }
    let Some(old) = before else {
        let output = after.ok_or(6)?;
        if output.phase() != 0
            || output.counter() != 0
            || output.turn() != 0
            || output.u64(212) != 0
            || !authorized(&output, 0)
            || !authorized(&output, 1)
        {
            return Err(15);
        }
        return Ok(());
    };
    if capacity(Source::GroupInput)? != old.u64(196) * 2 || load_cell_lock_hash(0, Source::GroupInput).map_err(|_| 4)? != public_lock()
    {
        return Err(14);
    }
    let witness = load_witness_args(0, Source::GroupInput).map_err(|_| 7)?.input_type().to_opt().ok_or(7)?.raw_data();
    if witness.len() != 1 {
        return Err(7);
    }
    let action = witness[0];
    if let Some(new) = after {
        if old.0[4..212] != new.0[4..212] {
            return Err(16);
        }
        match action {
            1 => {
                if old.phase() != 0
                    || new.phase() != 1
                    || old.0[212..220] != new.0[212..220]
                    || old.0[221..223] != new.0[221..223]
                    || old.0[225..257] != new.0[225..257]
                    || !authorized(&old, old.turn())
                {
                    return Err(17);
                }
            }
            2 => {
                if old.phase() != 1 || !old.valid_proposal() || new.0 != old.moved()?.0 {
                    return Err(20);
                }
            }
            3 => {
                if old.phase() != 1 || old.valid_proposal() || !authorized(&old, old.turn() ^ 1) || new.0 != old.fraud()?.0 {
                    return Err(21);
                }
            }
            _ => return Err(18),
        }
        return Ok(());
    }
    match action {
        4 => {
            if old.phase() != 2 {
                return Err(22);
            }
            payout(&old, 0, old.turn(), old.u64(196) * 2)
        }
        5 => {
            let since = load_input_since(0, Source::GroupInput).map_err(|_| 23)?;
            if since & 0xff00_0000_0000_0000 != RELATIVE_BLOCK || since & 0x00ff_ffff_ffff_ffff < old.u64(204) {
                return Err(23);
            }
            let winner = if old.phase() == 2 {
                Some(old.turn())
            } else if old.phase() == 1 && !old.valid_proposal() {
                Some(old.turn() ^ 1)
            } else if old.phase() == 1 && old.moved()?.phase() == 2 {
                Some(old.turn())
            } else {
                None
            };
            if let Some(winner) = winner {
                payout(&old, 0, winner, old.u64(196) * 2)
            } else {
                payout(&old, 0, 0, old.u64(196))?;
                payout(&old, 1, 1, old.u64(196))
            }
        }
        6 => {
            if old.phase() != 0 || old.counter() != 0 || old.u64(212) != 0 || !authorized(&old, 0) || !authorized(&old, 1) {
                return Err(24);
            }
            payout(&old, 0, 0, old.u64(196))?;
            payout(&old, 1, 1, old.u64(196))
        }
        _ => Err(18),
    }
}
fn program_entry() -> i8 {
    match verify() {
        Ok(()) => 0,
        Err(code) => code,
    }
}
