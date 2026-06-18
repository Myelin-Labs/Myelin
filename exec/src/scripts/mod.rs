// SPDX-License-Identifier: MIT
// Copyright (C) 2026 Myelin developers
//
// Standard scripts for Myelin

//! Standard lock and type scripts
//!
//! This module contains:
//! - secp256k1 lock script (RISC-V binary)
//! - Always-success lock (for testing)
//! - Capacity type script
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

/// Secp256k1 lock fixture (for testing)
///
/// This ELF fixture exercises the VM standard-lock path end-to-end by:
/// - loading the current script via `LOAD_SCRIPT`,
/// - reading the current group-input witness via `LOAD_WITNESS`,
/// - loading the canonical ECDSA sighash via syscall `3004`, and
/// - verifying the recoverable signature against syscall `3002`.
pub const SECP256K1_LOCK_FIXTURE_SCRIPT: &[u8] = include_bytes!("fixtures/secp256k1_lock_fixture.elf");

/// Secp256k1 lock fixture code hash
pub fn secp256k1_lock_fixture_code_hash() -> [u8; 32] {
    blake3::hash(SECP256K1_LOCK_FIXTURE_SCRIPT).into()
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

/// HTLC (Hash Time Locked Contract) script
///
/// This ELF fixture implements a complete HTLC with two spending paths:
/// 1. Recipient path: Provide secret preimage + signature
/// 2. Sender timeout path: Provide signature after timeout
///
/// Signature verification in this fixture is deterministic and test-oriented,
/// not a real secp256k1 implementation.
///
/// Script args format (105 bytes):
/// - [0..32]:   secret_hash (blake3)
/// - [32..64]:  recipient_pubkey (32 bytes)
/// - [64..96]:  sender_pubkey (32 bytes)
/// - [96]:      lock_type (0=abs block number, 1=abs timestamp, 2=rel block number, 3=rel timestamp)
/// - [97..105]: lock_value (u64)
///
/// Witness format:
/// - Recipient: <signature (64)> <secret (32)> <0x01>
/// - Sender:    <signature (64)> <0x00>
pub const HTLC_SCRIPT: &[u8] = include_bytes!("fixtures/htlc.elf");

/// HTLC script code hash
pub fn htlc_code_hash() -> [u8; 32] {
    blake3::hash(HTLC_SCRIPT).into()
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

/// Secp256k1 + Blake3 lock script (Production-Ready ELF)
///
/// This is the production-grade secp256k1 lock script compiled from Rust to RISC-V.
/// It verifies ECDSA signatures using blake3 for hashing (Myelin-specific).
///
/// Features:
/// - Args: pubkey hash (20 bytes, blake3 of pubkey)
/// - Witness: recoverable signature (65 bytes, r + s + v), optional 1-byte sighash flag
/// - Loads canonical per-input ECDSA sighash via syscall 3004
/// - Verifies signatures via syscall 3002 (recover + blake3(pubkey)[0..20] comparison)
/// - Fail-closed semantics
///
/// Build: See `BUILD_INSTRUCTIONS`
pub const SECP256K1_BLAKE3_LOCK_SCRIPT: &[u8] = include_bytes!("fixtures/secp256k1_blake3_lock.elf");

/// Secp256k1 + Blake3 lock script code hash
pub fn secp256k1_blake3_lock_code_hash() -> [u8; 32] {
    blake3::hash(SECP256K1_BLAKE3_LOCK_SCRIPT).into()
}

/// Secp256k1 + Blake3 lock script source (C version for reference).
///
/// Note: this is the original C implementation. The production ELF is compiled
/// from the Rust version in `fixtures/secp256k1_blake3_lock.rs`.
pub const SECP256K1_BLAKE3_LOCK_SOURCE: &str = include_str!("secp256k1_blake3_lock.c");

/// Build instructions for secp256k1 lock
pub const BUILD_INSTRUCTIONS: &str = r#"
# Build secp256k1_blake3_lock.c to RISC-V binary

## Prerequisites
- RISC-V GNU toolchain (riscv64-unknown-elf-gcc)
- Install: https://github.com/riscv-collab/riscv-gnu-toolchain

## Build Command
riscv64-unknown-elf-gcc -O3 -nostdlib -nostartfiles \
    -fno-builtin-printf -fno-builtin-memcmp \
    -Wl,-Ttext=0x0 \
    -o secp256k1_blake3_lock.elf \
    secp256k1_blake3_lock.c

riscv64-unknown-elf-objcopy -O binary \
    secp256k1_blake3_lock.elf \
    secp256k1_blake3_lock.bin

## Verify
hexdump -C secp256k1_blake3_lock.bin

## Get Code Hash
blake3sum secp256k1_blake3_lock.bin

# If blake3sum/b3sum is unavailable:
cargo run -p myelin-exec --example fixture_hashes -- secp256k1_blake3_lock.bin

# Note: exec/src/scripts/fixtures/build_fixtures.sh only builds the Rust-based
# ELF fixtures under fixtures/*.rs. This C lock requires a separate RISC-V C
# toolchain.
"#;

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_always_success_code_hash() {
        let hash = always_success_code_hash();
        assert_eq!(hash.len(), 32);

        // Verify it's deterministic
        let hash2 = always_success_code_hash();
        assert_eq!(hash, hash2);
    }

    #[test]
    fn test_always_success_script_size() {
        assert!(ALWAYS_SUCCESS_SCRIPT.len() > 64);
        assert_eq!(&ALWAYS_SUCCESS_SCRIPT[..4], b"\x7fELF");
    }

    #[test]
    fn test_load_input_since_script_size() {
        assert!(LOAD_INPUT_SINCE_SCRIPT.len() > 64);
        assert_eq!(&LOAD_INPUT_SINCE_SCRIPT[..4], b"\x7fELF");
    }

    #[test]
    fn test_load_header_timestamp_script_size() {
        assert!(LOAD_HEADER_TIMESTAMP_SCRIPT.len() > 64);
        assert_eq!(&LOAD_HEADER_TIMESTAMP_SCRIPT[..4], b"\x7fELF");
    }

    #[test]
    fn test_load_dep_cell_data_script_size() {
        assert!(LOAD_DEP_CELL_DATA_SCRIPT.len() > 64);
        assert_eq!(&LOAD_DEP_CELL_DATA_SCRIPT[..4], b"\x7fELF");
    }

    #[test]
    fn test_load_ecdsa_signature_hash_script_size() {
        assert!(LOAD_ECDSA_SIGNATURE_HASH_SCRIPT.len() > 64);
        assert_eq!(&LOAD_ECDSA_SIGNATURE_HASH_SCRIPT[..4], b"\x7fELF");
    }

    #[test]
    fn test_secp256k1_lock_fixture_script_size() {
        assert!(SECP256K1_LOCK_FIXTURE_SCRIPT.len() > 64);
        assert_eq!(&SECP256K1_LOCK_FIXTURE_SCRIPT[..4], b"\x7fELF");
    }

    #[test]
    fn test_timelock_absolute_script_size() {
        assert!(TIMELOCK_ABSOLUTE_SCRIPT.len() > 64);
        assert_eq!(&TIMELOCK_ABSOLUTE_SCRIPT[..4], b"\x7fELF");
    }

    #[test]
    fn test_timelock_relative_script_size() {
        assert!(TIMELOCK_RELATIVE_SCRIPT.len() > 64);
        assert_eq!(&TIMELOCK_RELATIVE_SCRIPT[..4], b"\x7fELF");
    }

    #[test]
    fn test_htlc_script_size() {
        assert!(HTLC_SCRIPT.len() > 64);
        assert_eq!(&HTLC_SCRIPT[..4], b"\x7fELF");
    }

    #[test]
    fn test_htlc_minimal_script_size() {
        assert!(HTLC_MINIMAL_SCRIPT.len() > 64);
        assert_eq!(&HTLC_MINIMAL_SCRIPT[..4], b"\x7fELF");
    }

    #[test]
    fn test_secp256k1_blake3_lock_script_size() {
        assert!(SECP256K1_BLAKE3_LOCK_SCRIPT.len() > 64);
        assert_eq!(&SECP256K1_BLAKE3_LOCK_SCRIPT[..4], b"\x7fELF");
    }

    #[test]
    fn test_secp256k1_blake3_lock_code_hash() {
        let hash = secp256k1_blake3_lock_code_hash();
        assert_eq!(hash.len(), 32);

        // Verify it's deterministic
        let hash2 = secp256k1_blake3_lock_code_hash();
        assert_eq!(hash, hash2);
    }
}

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
mod secp256k1_lock_fixture_test;

#[cfg(all(test, feature = "vm"))]
mod timelock_absolute_test;

#[cfg(all(test, feature = "vm"))]
mod timelock_relative_test;

#[cfg(all(test, feature = "vm"))]
mod htlc_test;

#[cfg(all(test, feature = "vm"))]
mod htlc_minimal_test;

#[cfg(all(test, feature = "vm"))]
mod syscall_edge_cases_test;
