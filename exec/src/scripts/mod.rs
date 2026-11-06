// SPDX-License-Identifier: MIT
// Copyright (C) 2026 Myelin developers
//
// Standard scripts for Myelin

//! VM syscall and time-lock fixtures
//!
//! This module contains:
//! - Always-success lock (for testing)
//! - Time lock scripts (CKB-VM based, replaces txscript CLTV/CSV)

/// Always-success lock script (for testing)
///
/// This is a real RISC-V ELF fixture that exits with code 0.
pub const ALWAYS_SUCCESS_SCRIPT: &[u8] = include_bytes!("fixtures/always_success.elf");

/// Always-success lock script code hash
pub fn always_success_code_hash() -> [u8; 32] {
    blake3::hash(ALWAYS_SUCCESS_SCRIPT).into()
}

/// Load-input-since lock script (for testing)
///
/// This ELF fixture exercises `LOAD_INPUT_BY_FIELD` and exits with code 0 only
/// when the first input's `since` matches the baked-in constant.
pub const LOAD_INPUT_SINCE_SCRIPT: &[u8] = include_bytes!("fixtures/load_input_since.elf");

/// Load-input-since lock script code hash
pub fn load_input_since_code_hash() -> [u8; 32] {
    blake3::hash(LOAD_INPUT_SINCE_SCRIPT).into()
}

/// Load-header-timestamp lock script (for testing)
///
/// This ELF fixture exercises `LOAD_HEADER_BY_FIELD` over the first header dep
/// and exits with code 0 only when the timestamp matches the baked-in constant.
pub const LOAD_HEADER_TIMESTAMP_SCRIPT: &[u8] = include_bytes!("fixtures/load_header_timestamp.elf");

/// Load-header-timestamp lock script code hash
pub fn load_header_timestamp_code_hash() -> [u8; 32] {
    blake3::hash(LOAD_HEADER_TIMESTAMP_SCRIPT).into()
}

/// Load-dep-cell-data lock script (for testing)
///
/// This ELF fixture exercises `LOAD_CELL_DATA` against the first cell dep and
/// exits with code 0 only when the returned bytes match the baked-in constant.
pub const LOAD_DEP_CELL_DATA_SCRIPT: &[u8] = include_bytes!("fixtures/load_dep_cell_data.elf");

/// Load-dep-cell-data lock script code hash
pub fn load_dep_cell_data_code_hash() -> [u8; 32] {
    blake3::hash(LOAD_DEP_CELL_DATA_SCRIPT).into()
}

/// Load-ecdsa-signature-hash lock script (for testing)
///
/// This ELF fixture exercises syscall `3004` by loading the canonical ECDSA
/// signature hash for the first group input and comparing it with an expected
/// digest embedded in the witness prefix.
pub const LOAD_ECDSA_SIGNATURE_HASH_SCRIPT: &[u8] = include_bytes!("fixtures/load_ecdsa_signature_hash.elf");

/// Load-ecdsa-signature-hash lock script code hash
pub fn load_ecdsa_signature_hash_code_hash() -> [u8; 32] {
    blake3::hash(LOAD_ECDSA_SIGNATURE_HASH_SCRIPT).into()
}

/// Absolute timestamp lock script (for testing)
///
/// This ELF fixture verifies that the input's `since` field (as absolute timestamp)
/// is >= 1735689600 (2025-01-01 00:00:00 UTC).
/// Expected since format: bit63=0 (absolute), bit62=1 (timestamp), bits0-55=unix_timestamp
pub const TIMELOCK_ABSOLUTE_SCRIPT: &[u8] = include_bytes!("fixtures/timelock_absolute.elf");

/// Absolute timestamp lock script code hash
pub fn timelock_absolute_code_hash() -> [u8; 32] {
    blake3::hash(TIMELOCK_ABSOLUTE_SCRIPT).into()
}

/// Relative block number lock script (for testing)
///
/// This ELF fixture verifies that the input's `since` field (as relative block number)
/// is >= 100 blocks.
/// Expected since format: bit63=1 (relative), bit62=0 (block number), bits0-55=delta
pub const TIMELOCK_RELATIVE_SCRIPT: &[u8] = include_bytes!("fixtures/timelock_relative.elf");

/// Relative block number lock script code hash
pub fn timelock_relative_code_hash() -> [u8; 32] {
    blake3::hash(TIMELOCK_RELATIVE_SCRIPT).into()
}

/// Minimal HTLC witness-loading script (for debugging)
///
/// This fixture only verifies that `LOAD_WITNESS` over the current input group works.
pub const HTLC_MINIMAL_SCRIPT: &[u8] = include_bytes!("fixtures/htlc_minimal.elf");

/// Minimal HTLC witness-loading script code hash
pub fn htlc_minimal_code_hash() -> [u8; 32] {
    blake3::hash(HTLC_MINIMAL_SCRIPT).into()
}

/// Time lock script helpers (CKB-VM based)
///
/// Replaces txscript OP_CHECKLOCKTIMEVERIFY and OP_CHECKSEQUENCEVERIFY
/// with CKB-VM scripts that use the `since` syscall.
pub mod timelock;

#[cfg(all(test, feature = "vm"))]
mod always_success_test;

#[cfg(all(test, feature = "vm"))]
mod load_input_since_test;

#[cfg(all(test, feature = "vm"))]
mod load_header_timestamp_test;

#[cfg(all(test, feature = "vm"))]
mod load_dep_cell_data_test;

#[cfg(all(test, feature = "vm"))]
mod load_ecdsa_signature_hash_test;

#[cfg(all(test, feature = "vm"))]
mod timelock_absolute_test;

#[cfg(all(test, feature = "vm"))]
mod timelock_relative_test;

#[cfg(all(test, feature = "vm"))]
mod htlc_minimal_test;

#[cfg(all(test, feature = "vm"))]
mod syscall_edge_cases_test;
