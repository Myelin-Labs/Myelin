// SPDX-License-Identifier: MIT
// Copyright (C) 2026 Myelin developers
//
// VM integration for Cell script execution

#![allow(ambiguous_glob_reexports, missing_docs)]

#[cfg(feature = "vm")]
pub mod cost_model;
#[cfg(feature = "vm")]
pub mod error;
#[cfg(feature = "vm")]
pub mod machine;
#[cfg(feature = "vm")]
pub mod scheduler;
#[cfg(feature = "vm")]
pub mod syscalls;
#[cfg(feature = "vm")]
pub mod verifier;

#[cfg(feature = "vm")]
pub use cost_model::*;
#[cfg(feature = "vm")]
pub use error::*;
#[cfg(feature = "vm")]
pub use machine::*;
#[cfg(feature = "vm")]
pub use scheduler::*;
#[cfg(feature = "vm")]
pub use syscalls::*;
#[cfg(feature = "vm")]
pub use verifier::*;

/// VM integration status
pub const VM_ENABLED: bool = cfg!(feature = "vm");

/// VM version for MYELIN
pub const MYELIN_VM_VERSION: u32 = 0x0001_0000; // 1.0.0

/// VM ISA support
pub const MYELIN_VM_ISA: u8 = 0x07; // IMC + B + MOP

//
// Default VM Limits (CKB-compatible)
//

/// Maximum cycles per block (CKB default)
pub const MAX_BLOCK_CYCLES: u64 = 70_000_000; // 70M cycles

/// Maximum cycles per transaction  
pub const MAX_TX_CYCLES: u64 = 10_000_000; // 10M cycles

/// Maximum script code size
pub const MAX_SCRIPT_SIZE: usize = 1024 * 1024; // 1 MB

/// Maximum VM memory
pub const MAX_VM_MEMORY: usize = 4 * 1024 * 1024; // 4 MB (ckb-vm 0.24 maximum)

/// Cycles per byte for effective size calculation
pub const DEFAULT_CYCLES_PER_BYTE: u64 = 100;

/// Configurable VM limits
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct VmLimits {
    /// Maximum cycles per transaction
    pub max_tx_cycles: u64,
    /// Maximum cycles per block
    pub max_block_cycles: u64,
    /// Maximum script code size in bytes
    pub max_script_size: usize,
    /// Maximum VM memory in bytes
    pub max_memory: usize,
    /// Cycles per byte for fee density calculation
    pub cycles_per_byte: u64,
}

impl Default for VmLimits {
    fn default() -> Self {
        Self {
            max_tx_cycles: MAX_TX_CYCLES,
            max_block_cycles: MAX_BLOCK_CYCLES,
            max_script_size: MAX_SCRIPT_SIZE,
            max_memory: MAX_VM_MEMORY,
            cycles_per_byte: DEFAULT_CYCLES_PER_BYTE,
        }
    }
}

/// VM syscall semantics profile.
///
/// `MyelinExtended` preserves current Myelin-only syscall extensions such as
/// resolving `HeaderDep` through `LOAD_CELL` / `LOAD_CELL_DATA`.
/// `CkbStrict` disables those extensions so syscall behavior more closely
/// matches upstream CKB.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum VmSemantics {
    /// Preserve Myelin-specific syscall extensions.
    #[default]
    MyelinExtended,
    /// Prefer upstream CKB syscall semantics.
    CkbStrict,
}

impl VmSemantics {
    /// Whether `LOAD_CELL` / `LOAD_CELL_DATA` may map `HeaderDep` to a cell.
    pub const fn allow_header_dep_cell_lookup(self) -> bool {
        matches!(self, Self::MyelinExtended)
    }

    /// Whether Myelin-only helper syscalls in the `3001..3004` range are exposed.
    pub const fn allow_myelin_extension_syscalls(self) -> bool {
        matches!(self, Self::MyelinExtended)
    }

    /// Whether Myelin's session header object and field ABI are exposed through
    /// `LOAD_HEADER` and `LOAD_HEADER_BY_FIELD`.
    pub const fn allow_myelin_header_abi(self) -> bool {
        matches!(self, Self::MyelinExtended)
    }

    /// Whether legacy Myelin group source encodings such as `0x0100` are
    /// accepted in addition to canonical CKB high-bit group source values.
    pub const fn allow_legacy_group_source_encoding(self) -> bool {
        matches!(self, Self::MyelinExtended)
    }
}

impl VmLimits {
    /// Create VM limits with custom values
    pub const fn new(
        max_tx_cycles: u64,
        max_block_cycles: u64,
        max_script_size: usize,
        max_memory: usize,
        cycles_per_byte: u64,
    ) -> Self {
        Self { max_tx_cycles, max_block_cycles, max_script_size, max_memory, cycles_per_byte }
    }

    /// CKB-compatible defaults
    pub const fn ckb_defaults() -> Self {
        Self::new(MAX_TX_CYCLES, MAX_BLOCK_CYCLES, MAX_SCRIPT_SIZE, MAX_VM_MEMORY, DEFAULT_CYCLES_PER_BYTE)
    }

    /// Calculate effective transaction size (for fee density)
    ///
    /// effective_size = max(serialized_size, cycles / cycles_per_byte)
    pub fn effective_size(&self, serialized_size: usize, cycles: u64) -> usize {
        let cycles_size = (cycles / self.cycles_per_byte) as usize;
        serialized_size.max(cycles_size)
    }
}
