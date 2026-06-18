// SPDX-License-Identifier: MIT
// Copyright (C) 2026 Myelin developers
//
// Time lock scripts using CKB-VM with `since` syscall
//
// This module provides Script construction helpers for time lock scripts
// that use the Cell model's `since` field instead of txscript CLTV/CSV opcodes.

//! Time lock script helpers for Cell model
//!
//! This module provides utilities for creating time lock scripts using CKB-VM
//! with the `since` syscall. In the Cell model, time locks are enforced per-input
//! using the `since` field, not tx-level lock_time or sequence-based semantics.
//!
//! ## Migration from Txscript Locks
//!
//! | Txscript | Cell Model (CKB-VM) |
//! |-------------------|---------------------|
//! | `OP_CHECKLOCKTIMEVERIFY` | `load_input_since` syscall + comparison |
//! | `OP_CHECKSEQUENCEVERIFY` | `since` with relative lock flags |
//! | `lock_time` field | `since` per input |
//! | `sequence` field | `since` with bit63=1 (relative) |
//!
//! ## Since Encoding
//!
//! The `since` field is a 64-bit value with the following structure:
//! - Bit 63: Relative lock flag (1 = relative, 0 = absolute)
//! - Bit 62: block number vs timestamp flag (1 = timestamp, 0 = block number)
//! - Bits 0-55: Lock value (block number or timestamp)
//!
//! See: <https://github.com/nervosnetwork/rfcs/blob/master/rfcs/0017-tx-valid-since/0017-tx-valid-since.md>

use crate::celltx::Script;

/// Code hash for the absolute time lock script (timestamp-based)
///
/// This script verifies that the input's `since` field is >= a target timestamp.
/// Script args: [target_timestamp: u64 (8 bytes, little-endian)]
pub const ABSOLUTE_TIME_LOCK_CODE_HASH: [u8; 32] = [
    0x01, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00,
    0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00,
];

/// Code hash for the relative time lock script (block number-based)
///
/// This script verifies that the input's `since` field is >= a target block number delta.
/// Script args: [target_delta: u64 (8 bytes, little-endian)]
pub const RELATIVE_TIME_LOCK_CODE_HASH: [u8; 32] = [
    0x02, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00,
    0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00,
];

/// Code hash for the absolute block number lock script
///
/// This script verifies that the input's `since` field is >= a target block number.
/// Script args: [target_block_number: u64 (8 bytes, little-endian)]
pub const ABSOLUTE_BLOCK_NUMBER_LOCK_CODE_HASH: [u8; 32] = [
    0x03, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00,
    0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00,
];

/// Code hash for the relative timestamp lock script
///
/// This script verifies that the input's `since` field is >= a target timestamp delta.
/// Script args: [target_delta_seconds: u64 (8 bytes, little-endian)]
pub const RELATIVE_TIMESTAMP_LOCK_CODE_HASH: [u8; 32] = [
    0x04, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00,
    0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00,
];

/// Reserved code hash for the absolute timestamp + secp256k1 combined lock script.
pub const COMBINED_ABSOLUTE_TIME_LOCK_CODE_HASH: [u8; 32] = [
    0x11, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00,
    0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00,
];

/// Reserved code hash for the relative block number + secp256k1 combined lock script.
pub const COMBINED_RELATIVE_TIME_LOCK_CODE_HASH: [u8; 32] = [
    0x12, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00,
    0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00,
];

/// Reserved code hash for the absolute block number + secp256k1 combined lock script.
pub const COMBINED_ABSOLUTE_BLOCK_NUMBER_LOCK_CODE_HASH: [u8; 32] = [
    0x13, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00,
    0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00,
];

/// Reserved code hash for the relative timestamp + secp256k1 combined lock script.
pub const COMBINED_RELATIVE_TIMESTAMP_LOCK_CODE_HASH: [u8; 32] = [
    0x14, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00,
    0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00,
];

/// Hash type for all time lock scripts
pub const TIME_LOCK_HASH_TYPE: u8 = 0;

/// Since flags
pub mod since_flags {
    /// Relative lock flag (bit 63)
    pub const RELATIVE: u64 = 1 << 63;
    /// Timestamp flag (bit 62) - if set, interpret as Unix timestamp; otherwise block number
    pub const TIMESTAMP: u64 = 1 << 62;
    /// Mask for the value portion (bits 0-55)
    pub const VALUE_MASK: u64 = 0x00FF_FFFF_FFFF_FFFF;
}

/// Create an absolute timestamp lock Script
///
/// This creates a script that requires the input's `since` field to be >= the
/// specified Unix timestamp (seconds since epoch).
///
/// # Arguments
/// * `target_timestamp` - The minimum Unix timestamp required (seconds since epoch)
///
/// # Example
/// ```rust
/// use myelin_exec::scripts::timelock;
///
/// // Lock until January 1, 2025
/// let target = 1735689600u64; // 2025-01-01 00:00:00 UTC
/// let script = timelock::absolute_timestamp_lock(target);
/// ```
pub fn absolute_timestamp_lock(target_timestamp: u64) -> Script {
    let args = target_timestamp.to_le_bytes().to_vec();
    Script::new(ABSOLUTE_TIME_LOCK_CODE_HASH, TIME_LOCK_HASH_TYPE, args)
}

/// Create a relative block number lock Script
///
/// This creates a script that requires the input's `since` field to indicate
/// a relative lock of at least `delta_blocks` blocks from the input's confirmation.
///
/// # Arguments
/// * `delta_blocks` - The minimum number of blocks to wait
///
/// # Example
/// ```rust
/// use myelin_exec::scripts::timelock;
///
/// // Lock for 100 blocks relative to confirmation
/// let script = timelock::relative_block_number_lock(100);
/// ```
pub fn relative_block_number_lock(delta_blocks: u64) -> Script {
    let args = delta_blocks.to_le_bytes().to_vec();
    Script::new(RELATIVE_TIME_LOCK_CODE_HASH, TIME_LOCK_HASH_TYPE, args)
}

/// Create an absolute block number lock Script
///
/// This creates a script that requires the input's `since` field to be >= the
/// specified absolute block number.
///
/// # Arguments
/// * `target_block_number` - The minimum block number required
///
/// # Example
/// ```rust
/// use myelin_exec::scripts::timelock;
///
/// // Lock until block number 1000000
/// let script = timelock::absolute_block_number_lock(1_000_000);
/// ```
pub fn absolute_block_number_lock(target_block_number: u64) -> Script {
    let args = target_block_number.to_le_bytes().to_vec();
    Script::new(ABSOLUTE_BLOCK_NUMBER_LOCK_CODE_HASH, TIME_LOCK_HASH_TYPE, args)
}

/// Create a relative timestamp lock Script
///
/// This creates a script that requires the input's `since` field to indicate
/// a relative lock of at least `delta_seconds` from the input's confirmation.
///
/// # Arguments
/// * `delta_seconds` - The minimum number of seconds to wait
///
/// # Example
/// ```rust
/// use myelin_exec::scripts::timelock;
///
/// // Lock for 24 hours relative to confirmation
/// let script = timelock::relative_timestamp_lock(24 * 60 * 60);
/// ```
pub fn relative_timestamp_lock(delta_seconds: u64) -> Script {
    let args = delta_seconds.to_le_bytes().to_vec();
    Script::new(RELATIVE_TIMESTAMP_LOCK_CODE_HASH, TIME_LOCK_HASH_TYPE, args)
}

/// Encode a `since` value for absolute timestamp lock
///
/// # Arguments
/// * `timestamp` - Unix timestamp (seconds since epoch)
///
/// # Returns
/// The encoded `since` value to use in `CellInput::since`
pub fn encode_absolute_timestamp_since(timestamp: u64) -> u64 {
    since_flags::TIMESTAMP | (timestamp & since_flags::VALUE_MASK)
}

/// Encode a `since` value for relative block number lock
///
/// # Arguments
/// * `delta` - Number of blocks to wait
///
/// # Returns
/// The encoded `since` value to use in `CellInput::since`
pub fn encode_relative_block_number_since(delta: u64) -> u64 {
    since_flags::RELATIVE | (delta & since_flags::VALUE_MASK)
}

/// Encode a `since` value for absolute block number lock
///
/// # Arguments
/// * `block_number` - Target block number
///
/// # Returns
/// The encoded `since` value to use in `CellInput::since`
pub fn encode_absolute_block_number_since(block_number: u64) -> u64 {
    block_number & since_flags::VALUE_MASK
}

/// Encode a `since` value for relative timestamp lock
///
/// # Arguments
/// * `delta_seconds` - Number of seconds to wait
///
/// # Returns
/// The encoded `since` value to use in `CellInput::since`
pub fn encode_relative_timestamp_since(delta_seconds: u64) -> u64 {
    since_flags::RELATIVE | since_flags::TIMESTAMP | (delta_seconds & since_flags::VALUE_MASK)
}

/// Decode a `since` value
///
/// Returns (is_relative, is_timestamp, value)
pub fn decode_since(since: u64) -> (bool, bool, u64) {
    let is_relative = since & since_flags::RELATIVE != 0;
    let is_timestamp = since & since_flags::TIMESTAMP != 0;
    let value = since & since_flags::VALUE_MASK;
    (is_relative, is_timestamp, value)
}

/// Create a combined secp256k1 + time lock Script
///
/// This creates a script that requires both signature verification AND
/// time lock verification. The script args contain:
/// - [0..20]: pubkey hash (20 bytes)
/// - [20..28]: target timestamp or delta (8 bytes, little-endian)
///
/// # Arguments
/// * `pubkey_hash` - 20-byte hash of the public key
/// * `target` - Target timestamp or delta (depending on lock type)
/// * `is_relative` - Whether this is a relative lock
/// * `is_timestamp` - Whether to use timestamp (vs block number)
///
/// The returned script uses a reserved combined-lock code hash. The actual
/// RISC-V binary still needs to be deployed under that hash before this helper
/// is usable in production.
pub fn secp256k1_with_timelock(pubkey_hash: [u8; 20], target: u64, is_relative: bool, is_timestamp: bool) -> Script {
    // Combined args: pubkey_hash (20 bytes) + target (8 bytes)
    let mut args = Vec::with_capacity(28);
    args.extend_from_slice(&pubkey_hash);
    args.extend_from_slice(&target.to_le_bytes());

    // Choose the reserved combined-lock code hash based on lock type.
    let code_hash = match (is_relative, is_timestamp) {
        (false, true) => COMBINED_ABSOLUTE_TIME_LOCK_CODE_HASH, // Absolute timestamp
        (true, false) => COMBINED_RELATIVE_TIME_LOCK_CODE_HASH, // Relative block number
        (false, false) => COMBINED_ABSOLUTE_BLOCK_NUMBER_LOCK_CODE_HASH, // Absolute block number
        (true, true) => COMBINED_RELATIVE_TIMESTAMP_LOCK_CODE_HASH, // Relative timestamp
    };

    Script::new(code_hash, TIME_LOCK_HASH_TYPE, args)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_absolute_timestamp_lock() {
        let target = 1735689600u64; // 2025-01-01 00:00:00 UTC
        let script = absolute_timestamp_lock(target);

        assert_eq!(script.code_hash, ABSOLUTE_TIME_LOCK_CODE_HASH);
        assert_eq!(script.hash_type, TIME_LOCK_HASH_TYPE);
        assert_eq!(script.args, target.to_le_bytes().to_vec());
    }

    #[test]
    fn test_relative_block_number_lock() {
        let delta = 100u64;
        let script = relative_block_number_lock(delta);

        assert_eq!(script.code_hash, RELATIVE_TIME_LOCK_CODE_HASH);
        assert_eq!(script.hash_type, TIME_LOCK_HASH_TYPE);
        assert_eq!(script.args, delta.to_le_bytes().to_vec());
    }

    #[test]
    fn test_absolute_block_number_lock() {
        let target = 1_000_000u64;
        let script = absolute_block_number_lock(target);

        assert_eq!(script.code_hash, ABSOLUTE_BLOCK_NUMBER_LOCK_CODE_HASH);
        assert_eq!(script.hash_type, TIME_LOCK_HASH_TYPE);
        assert_eq!(script.args, target.to_le_bytes().to_vec());
    }

    #[test]
    fn test_relative_timestamp_lock() {
        let delta = 24 * 60 * 60u64; // 24 hours
        let script = relative_timestamp_lock(delta);

        assert_eq!(script.code_hash, RELATIVE_TIMESTAMP_LOCK_CODE_HASH);
        assert_eq!(script.hash_type, TIME_LOCK_HASH_TYPE);
        assert_eq!(script.args, delta.to_le_bytes().to_vec());
    }

    #[test]
    fn test_encode_absolute_timestamp_since() {
        let timestamp = 1735689600u64;
        let since = encode_absolute_timestamp_since(timestamp);

        assert_eq!(since & since_flags::TIMESTAMP, since_flags::TIMESTAMP);
        assert_eq!(since & since_flags::VALUE_MASK, timestamp);
        assert_eq!(since & since_flags::RELATIVE, 0);
    }

    #[test]
    fn test_encode_relative_block_number_since() {
        let delta = 100u64;
        let since = encode_relative_block_number_since(delta);

        assert_eq!(since & since_flags::RELATIVE, since_flags::RELATIVE);
        assert_eq!(since & since_flags::VALUE_MASK, delta);
        assert_eq!(since & since_flags::TIMESTAMP, 0);
    }

    #[test]
    fn test_encode_relative_timestamp_since() {
        let delta = 3600u64; // 1 hour
        let since = encode_relative_timestamp_since(delta);

        assert_eq!(since & since_flags::RELATIVE, since_flags::RELATIVE);
        assert_eq!(since & since_flags::TIMESTAMP, since_flags::TIMESTAMP);
        assert_eq!(since & since_flags::VALUE_MASK, delta);
    }

    #[test]
    fn test_decode_since() {
        let since = since_flags::RELATIVE | since_flags::TIMESTAMP | 3600;
        let (is_relative, is_timestamp, value) = decode_since(since);

        assert!(is_relative);
        assert!(is_timestamp);
        assert_eq!(value, 3600);
    }

    #[test]
    fn test_secp256k1_with_timelock() {
        let pubkey_hash = [0xABu8; 20];
        let target = 1735689600u64;

        let script = secp256k1_with_timelock(pubkey_hash, target, false, true);

        assert_eq!(script.code_hash, COMBINED_ABSOLUTE_TIME_LOCK_CODE_HASH);
        assert_eq!(script.args.len(), 28);
        assert_eq!(&script.args[0..20], &pubkey_hash);
        assert_eq!(&script.args[20..28], &target.to_le_bytes());
    }

    #[test]
    fn test_secp256k1_with_timelock_uses_distinct_reserved_hashes() {
        let script = secp256k1_with_timelock([0xCD; 20], 100, true, false);

        assert_eq!(script.code_hash, COMBINED_RELATIVE_TIME_LOCK_CODE_HASH);
        assert_ne!(script.code_hash, RELATIVE_TIME_LOCK_CODE_HASH);
    }
}
