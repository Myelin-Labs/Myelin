// SPDX-License-Identifier: MIT
// Copyright (C) 2026 Spora developers
//
// VM system calls
// Adapted from CKB script/src/syscalls/

pub mod blake3;
pub mod close;
pub mod current_cycles;
pub mod debugger;
pub mod exec;
pub mod inherited_fd;
pub mod load_cell;
pub mod load_cell_data;
pub mod load_header;
pub mod load_input;
pub mod load_script;
pub mod load_signature_hash;
pub mod load_tx;
pub mod load_witness;
pub mod pipe;
pub mod process_id;
pub mod read;
pub mod secp256k1_verify;
pub mod spawn;
pub mod utils; // Spora-specific: blake3 hash syscall
pub mod vm_version;
pub mod wait;
pub mod write;

pub use blake3::Blake3Hash;
pub use close::Close;
pub use current_cycles::CurrentCycles;
pub use debugger::Debugger;
pub use exec::Exec;
pub use inherited_fd::InheritedFd;
pub use load_cell::LoadCell;
pub use load_cell_data::LoadCellData;
pub use load_header::LoadHeader;
pub use load_input::LoadInput;
pub use load_script::LoadScript;
pub use load_signature_hash::{LoadSignatureHash, LOAD_SIGNATURE_HASH_BASE_CYCLES};
pub use load_tx::LoadTx;
pub use load_witness::LoadWitness;
pub use pipe::Pipe;
pub use process_id::ProcessId;
pub use read::Read;
pub use secp256k1_verify::{Secp256k1Verify, SECP256K1_VERIFY_BASE_CYCLES};
pub use spawn::Spawn;
pub use utils::*;
pub use vm_version::VMVersion;
pub use wait::Wait;
pub use write::Write;

/// System call numbers (aligned with CKB)
pub const VM_VERSION_SYSCALL_NUMBER: u64 = 2041;
pub const LOAD_TRANSACTION_SYSCALL_NUMBER: u64 = 2051;
pub const LOAD_TX_HASH_SYSCALL_NUMBER: u64 = 2061;
pub const LOAD_SCRIPT_HASH_SYSCALL_NUMBER: u64 = 2062;
pub const LOAD_CELL_SYSCALL_NUMBER: u64 = 2071;
pub const LOAD_HEADER_SYSCALL_NUMBER: u64 = 2072;
pub const LOAD_INPUT_SYSCALL_NUMBER: u64 = 2073;
pub const LOAD_WITNESS_SYSCALL_NUMBER: u64 = 2074;
/// Spora `LOAD_SCRIPT` syscall number.
///
/// Upstream CKB uses `2052` for `LOAD_SCRIPT`; Spora historically used `2075`.
/// Keep this constant as the Spora value for existing scripts and fixtures.
pub const LOAD_SCRIPT_SYSCALL_NUMBER: u64 = 2075;
/// Upstream CKB `LOAD_SCRIPT` syscall number.
pub const CKB_LOAD_SCRIPT_SYSCALL_NUMBER: u64 = 2052;
pub const LOAD_CELL_BY_FIELD_SYSCALL_NUMBER: u64 = 2081;
pub const LOAD_HEADER_BY_FIELD_SYSCALL_NUMBER: u64 = 2082;
pub const LOAD_INPUT_BY_FIELD_SYSCALL_NUMBER: u64 = 2083;
pub const LOAD_CELL_DATA_AS_CODE_SYSCALL_NUMBER: u64 = 2091;
pub const LOAD_CELL_DATA_SYSCALL_NUMBER: u64 = 2092;
pub const CURRENT_CYCLES_SYSCALL_NUMBER: u64 = 2042;
pub const DEBUG_PRINT_SYSCALL_NUMBER: u64 = 2177;
pub const SPAWN_SYSCALL_NUMBER: u64 = 2601;
pub const WAIT_SYSCALL_NUMBER: u64 = 2602;
pub const PROCESS_ID_SYSCALL_NUMBER: u64 = 2603;
pub const PIPE_SYSCALL_NUMBER: u64 = 2604;
pub const WRITE_SYSCALL_NUMBER: u64 = 2605;
pub const READ_SYSCALL_NUMBER: u64 = 2606;
pub const INHERITED_FD_SYSCALL_NUMBER: u64 = 2607;
pub const CLOSE_SYSCALL_NUMBER: u64 = 2608;

/// Spora-specific syscall numbers (3000+ range to avoid conflicts)
pub const BLAKE3_HASH_SYSCALL_NUMBER: u64 = 3001;
pub const SECP256K1_VERIFY_SYSCALL_NUMBER: u64 = 3002;
pub const LOAD_SCHNORR_SIGNATURE_HASH_SYSCALL_NUMBER: u64 = 3003;
pub const LOAD_ECDSA_SIGNATURE_HASH_SYSCALL_NUMBER: u64 = 3004;
pub const EXEC_SYSCALL_NUMBER: u64 = 2043;

/// System call return codes
pub const SUCCESS: u8 = 0;
pub const INDEX_OUT_OF_BOUND: u8 = 1;
pub const ITEM_MISSING: u8 = 2;
pub const SLICE_OUT_OF_BOUND: u8 = 3;
pub const WRONG_FORMAT: u8 = 4;
pub const WAIT_FAILURE: u8 = 5;
pub const INVALID_FD: u8 = 6;
pub const OTHER_END_CLOSED: u8 = 7;
pub const MAX_VMS_SPAWNED: u8 = 8;
pub const MAX_FDS_CREATED: u8 = 9;

/// Spawn/IPC syscall cost baseline from CKB.
pub const SPAWN_EXTRA_CYCLES_BASE: u64 = 100_000;
/// Spawn/IPC yield syscall cost baseline from CKB.
pub const SPAWN_YIELD_CYCLES_BASE: u64 = 800;

/// Canonical CKB group-source high-bit flag.
pub const SOURCE_GROUP_FLAG: u64 = 0x0100_0000_0000_0000;
const SOURCE_GROUP_MASK: u64 = 0xFF00_0000_0000_0000;
pub const SOURCE_ENTRY_MASK: u64 = 0x00FF_FFFF_FFFF_FFFF;

/// Source type for loading data
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[repr(u64)]
pub enum Source {
    /// Load from transaction inputs
    Input = 0x01,
    /// Load from transaction outputs
    Output = 0x02,
    /// Load from cell dependencies
    CellDep = 0x03,
    /// Load from header dependencies
    HeaderDep = 0x04,
    /// Load from current script group
    GroupInput = 0x0100,
    /// Load from current script group outputs
    GroupOutput = 0x0200,
    /// Load from current script group's cell deps
    GroupCellDep = 0x0300,
    /// Load from current script group's header deps
    GroupHeaderDep = 0x0400,
}

impl Source {
    /// Parse source from u64
    pub fn parse(source: u64) -> Option<Self> {
        if let Some(legacy) = Self::parse_legacy(source) {
            return Some(legacy);
        }

        Self::parse_canonical(source)
    }

    pub fn parse_from_u64(source: u64) -> Result<Self, ckb_vm::Error> {
        if let Some(parsed) = Self::parse(source) {
            return Ok(parsed);
        }

        Err(ckb_vm::Error::External(format!("SourceEntry parse_from_u64 {}", source & SOURCE_ENTRY_MASK)))
    }

    pub fn parse_for_semantics(source: u64, semantics: crate::vm::VmSemantics) -> Option<Self> {
        if semantics.allow_legacy_group_source_encoding() {
            Self::parse(source)
        } else {
            Self::parse_canonical(source)
        }
    }

    pub fn parse_from_u64_for_semantics(source: u64, semantics: crate::vm::VmSemantics) -> Result<Self, ckb_vm::Error> {
        if let Some(parsed) = Self::parse_for_semantics(source, semantics) {
            return Ok(parsed);
        }

        Err(ckb_vm::Error::External(format!("SourceEntry parse_from_u64 {}", source & SOURCE_ENTRY_MASK)))
    }

    fn parse_canonical(source: u64) -> Option<Self> {
        let entry = source & SOURCE_ENTRY_MASK;
        match source & SOURCE_GROUP_MASK {
            0 => match entry {
                0x01 => Some(Self::Input),
                0x02 => Some(Self::Output),
                0x03 => Some(Self::CellDep),
                0x04 => Some(Self::HeaderDep),
                _ => None,
            },
            SOURCE_GROUP_FLAG => match entry {
                0x01 => Some(Self::GroupInput),
                0x02 => Some(Self::GroupOutput),
                0x03 => Some(Self::GroupCellDep),
                0x04 => Some(Self::GroupHeaderDep),
                _ => None,
            },
            _ => None,
        }
    }

    fn parse_legacy(source: u64) -> Option<Self> {
        match source {
            0x01 => Some(Self::Input),
            0x02 => Some(Self::Output),
            0x03 => Some(Self::CellDep),
            0x04 => Some(Self::HeaderDep),
            0x0100 => Some(Self::GroupInput),
            0x0200 => Some(Self::GroupOutput),
            0x0300 => Some(Self::GroupCellDep),
            0x0400 => Some(Self::GroupHeaderDep),
            _ => None,
        }
    }
}

/// Cell field selector
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CellField {
    /// Capacity field
    Capacity = 0,
    /// Data hash field
    DataHash = 1,
    /// Lock field
    Lock = 2,
    /// Lock hash field
    LockHash = 3,
    /// Type field
    Type = 4,
    /// Type hash field
    TypeHash = 5,
    /// Occupied capacity field
    OccupiedCapacity = 6,
}

impl CellField {
    /// Parse field from u64
    pub fn parse(field: u64) -> Option<Self> {
        match field {
            0 => Some(Self::Capacity),
            1 => Some(Self::DataHash),
            2 => Some(Self::Lock),
            3 => Some(Self::LockHash),
            4 => Some(Self::Type),
            5 => Some(Self::TypeHash),
            6 => Some(Self::OccupiedCapacity),
            _ => None,
        }
    }

    pub fn parse_from_u64(field: u64) -> Result<Self, ckb_vm::Error> {
        Self::parse(field).ok_or_else(|| ckb_vm::Error::External(format!("CellField parse_from_u64 {field}")))
    }
}

/// Input field selector
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum InputField {
    /// OutPoint (tx_hash + index)
    OutPoint = 0,
    /// Since (time-lock value)
    Since = 1,
}

impl InputField {
    /// Parse field from u64
    pub fn parse(field: u64) -> Option<Self> {
        match field {
            0 => Some(Self::OutPoint),
            1 => Some(Self::Since),
            _ => None,
        }
    }

    pub fn parse_from_u64(field: u64) -> Result<Self, ckb_vm::Error> {
        Self::parse(field).ok_or_else(|| ckb_vm::Error::External(format!("InputField parse_from_u64 {field}")))
    }
}

/// Header field selector
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum HeaderField {
    /// DAA score
    DaaScore = 0,
    /// Timestamp
    Timestamp = 1,
    /// Block hash
    Hash = 2,
    /// Direct parent hashes
    Parents = 3,
    /// Header version
    Version = 4,
    /// Compact difficulty bits
    Bits = 5,
    /// Mining nonce
    Nonce = 6,
    /// Transaction hash merkle root
    HashMerkleRoot = 7,
    /// Accepted transaction ID merkle root
    AcceptedIdMerkleRoot = 8,
    /// Execution state commitment
    CellCommitment = 9,
    /// Cell state root
    CellRoot = 10,
    /// Data-availability segment root
    SegmentRoot = 11,
    /// Blue score
    BlueScore = 12,
    /// Accumulated blue work
    BlueWork = 13,
    /// Pruning-point hash
    PruningPoint = 14,
}

impl HeaderField {
    /// Parse field from u64
    pub fn parse(field: u64) -> Option<Self> {
        match field {
            0 => Some(Self::DaaScore),
            1 => Some(Self::Timestamp),
            2 => Some(Self::Hash),
            3 => Some(Self::Parents),
            4 => Some(Self::Version),
            5 => Some(Self::Bits),
            6 => Some(Self::Nonce),
            7 => Some(Self::HashMerkleRoot),
            8 => Some(Self::AcceptedIdMerkleRoot),
            9 => Some(Self::CellCommitment),
            10 => Some(Self::CellRoot),
            11 => Some(Self::SegmentRoot),
            12 => Some(Self::BlueScore),
            13 => Some(Self::BlueWork),
            14 => Some(Self::PruningPoint),
            _ => None,
        }
    }

    pub fn parse_from_u64(field: u64) -> Result<Self, ckb_vm::Error> {
        Self::parse(field).ok_or_else(|| ckb_vm::Error::External(format!("HeaderField parse_from_u64 {field}")))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_source_parse() {
        assert_eq!(Source::parse(0x01), Some(Source::Input));
        assert_eq!(Source::parse(0x02), Some(Source::Output));
        assert_eq!(Source::parse(0x03), Some(Source::CellDep));
        assert_eq!(Source::parse(0x0100), Some(Source::GroupInput));
        assert_eq!(Source::parse(0x0200), Some(Source::GroupOutput));
        assert_eq!(Source::parse(0x0100_0000_0000_0001), Some(Source::GroupInput));
        assert_eq!(Source::parse(0x0100_0000_0000_0002), Some(Source::GroupOutput));
        assert_eq!(Source::parse(0x0100_0000_0000_0003), Some(Source::GroupCellDep));
        assert_eq!(Source::parse(0x0100_0000_0000_0004), Some(Source::GroupHeaderDep));
        assert_eq!(Source::parse(0x99), None);
    }

    #[test]
    fn test_source_parse_for_semantics_rejects_legacy_group_values_under_ckb_strict() {
        assert_eq!(Source::parse_for_semantics(0x0100, crate::vm::VmSemantics::SporaExtended), Some(Source::GroupInput));
        assert_eq!(Source::parse_for_semantics(0x0100, crate::vm::VmSemantics::CkbStrict), None);
        assert_eq!(Source::parse_for_semantics(0x0100_0000_0000_0001, crate::vm::VmSemantics::CkbStrict), Some(Source::GroupInput));
        assert_eq!(Source::parse_for_semantics(0x0200_0000_0000_0001, crate::vm::VmSemantics::CkbStrict), None);
    }

    #[test]
    fn test_cell_field_parse() {
        assert_eq!(CellField::parse(0), Some(CellField::Capacity));
        assert_eq!(CellField::parse(1), Some(CellField::DataHash));
        assert_eq!(CellField::parse(99), None);
    }
}
