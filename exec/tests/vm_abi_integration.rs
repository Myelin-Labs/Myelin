// SPDX-License-Identifier: MIT
// Copyright (C) 2026 Myelin developers
//
// VM ABI Integration Tests
//
// These tests verify VM-facing types implement VmSerializable correctly.

use myelin_exec::{
    serialization::vm_abi::{
        serialize_cell_input, serialize_cell_output, serialize_outpoint, serialize_script, serialized_cell_output_size,
        serialized_script_size,
    },
    CellInput, CellOutput, OutPoint, ResolvedCell, ResolvedHeader, Script, VmAbiNegotiator, VmSerializable,
};

/// Test ResolvedHeader VmSerializable implementation
#[test]
fn test_resolved_header_vm_serializable() {
    let header = ResolvedHeader {
        hash: [0xAA; 32],
        version: 1,
        parent_hash: [0xBB; 32],
        transactions_root: [0x11; 32],
        proposals_hash: [0x22; 32],
        cell_commitment: [0x33; 32],
        cell_root: [0x44; 32],
        segment_root: [0x55; 32],
        timestamp: 1234567890,
        compact_target: 0x1d00ffff,
        nonce: 42,
        number: 1000,
        dao: [0x66; 32],
        epoch: 500,
        uncles_hash: [0x77; 32],
    };

    // Test ABI version — ResolvedHeader uses Molecule v1 for VM-facing serialization
    assert_eq!(ResolvedHeader::abi_version(), VmAbiNegotiator::ABI_VERSION_MOLECULE_V1);

    // Test serialization roundtrip
    let bytes = header.to_vm_bytes();
    let restored = ResolvedHeader::from_vm_bytes(&bytes).expect("should deserialize");
    assert_eq!(header, restored);
}

/// Test ResolvedCell VmSerializable implementation
#[test]
fn test_resolved_cell_vm_serializable() {
    let cell = ResolvedCell {
        cell_output: CellOutput {
            lock: Script::new([0xAA; 32], 0, vec![0xBB; 20]),
            type_: Some(Script::new([0xCC; 32], 1, vec![0xDD; 10])),
            capacity: 1000,
        },
        data: Some(vec![0xEE; 100]),
    };

    // Test ABI version — ResolvedCell uses Molecule v1 for VM-facing serialization
    assert_eq!(ResolvedCell::abi_version(), VmAbiNegotiator::ABI_VERSION_MOLECULE_V1);

    // Test serialization roundtrip
    let bytes = cell.to_vm_bytes();
    let restored = ResolvedCell::from_vm_bytes(&bytes).expect("should deserialize");
    assert_eq!(cell, restored);
}

/// Test ResolvedCell without data
#[test]
fn test_resolved_cell_without_data() {
    let cell =
        ResolvedCell { cell_output: CellOutput { lock: Script::new([0xAA; 32], 0, vec![]), type_: None, capacity: 500 }, data: None };

    let bytes = cell.to_vm_bytes();
    let restored = ResolvedCell::from_vm_bytes(&bytes).expect("should deserialize");
    assert_eq!(cell, restored);
}

/// Test vm_abi serialization functions
#[test]
fn test_vm_abi_serialize_script() {
    let script = Script::new([0xAA; 32], 1, vec![0xBB; 20]);
    let bytes = serialize_script(&script);

    // Expected format: code_hash (32) || hash_type (1) || args_len (4) || args
    assert_eq!(bytes.len(), 32 + 1 + 4 + 20);
    assert_eq!(&bytes[0..32], &[0xAA; 32]);
    assert_eq!(bytes[32], 1);
    assert_eq!(&bytes[33..37], &[20, 0, 0, 0]); // args_len in LE
    assert_eq!(&bytes[37..], &[0xBB; 20]);

    // Test size helper
    assert_eq!(serialized_script_size(&script), bytes.len());
}

/// Test vm_abi serialize_outpoint
#[test]
fn test_vm_abi_serialize_outpoint() {
    let outpoint = OutPoint::new([0xCC; 32], 0x12345678);
    let bytes = serialize_outpoint(&outpoint);

    // Expected format: tx_hash (32) || index (4)
    assert_eq!(bytes.len(), 36);
    assert_eq!(&bytes[0..32], &[0xCC; 32]);
    assert_eq!(&bytes[32..36], &[0x78, 0x56, 0x34, 0x12]); // index in LE
}

/// Test vm_abi serialize_cell_input
#[test]
fn test_vm_abi_serialize_cell_input() {
    let input = CellInput::new(OutPoint::new([0xDD; 32], 1), 0xABCDEF00);
    let bytes = serialize_cell_input(&input);

    // Expected format: tx_hash (32) || index (4) || since (8)
    assert_eq!(bytes.len(), 44);
    assert_eq!(&bytes[0..32], &[0xDD; 32]);
    assert_eq!(&bytes[32..36], &[1, 0, 0, 0]); // index in LE
    assert_eq!(&bytes[36..44], &[0x00, 0xEF, 0xCD, 0xAB, 0, 0, 0, 0]); // since in LE
}

/// Test vm_abi serialize_cell_output with type script
#[test]
fn test_vm_abi_serialize_cell_output_with_type() {
    let output = CellOutput {
        lock: Script::new([0x11; 32], 0, vec![0xAA; 20]),
        type_: Some(Script::new([0x22; 32], 1, vec![0xBB; 10])),
        capacity: 0x0102030405060708,
    };

    let bytes = serialize_cell_output(&output);

    // Check capacity (first 8 bytes, LE)
    assert_eq!(&bytes[0..8], &[0x08, 0x07, 0x06, 0x05, 0x04, 0x03, 0x02, 0x01]);

    // Check has_type flag (after lock script)
    let lock_len = u32::from_le_bytes([bytes[8], bytes[9], bytes[10], bytes[11]]) as usize;
    let has_type_pos = 8 + 4 + lock_len;
    assert_eq!(bytes[has_type_pos], 1); // has_type = 1

    // Test size helper
    assert_eq!(serialized_cell_output_size(&output), bytes.len());
}

/// Test vm_abi serialize_cell_output without type script
#[test]
fn test_vm_abi_serialize_cell_output_without_type() {
    let output = CellOutput { lock: Script::new([0x11; 32], 0, vec![]), type_: None, capacity: 1000 };

    let bytes = serialize_cell_output(&output);

    // Check has_type flag
    let lock_len = u32::from_le_bytes([bytes[8], bytes[9], bytes[10], bytes[11]]) as usize;
    let has_type_pos = 8 + 4 + lock_len;
    assert_eq!(bytes[has_type_pos], 0); // has_type = 0
}

/// Test VmSerializable error handling
#[test]
fn test_vm_serializable_error_handling() {
    // Test deserialization with invalid bytes
    let invalid_bytes = vec![0xFF; 100];
    let result = ResolvedHeader::from_vm_bytes(&invalid_bytes);
    assert!(result.is_err());

    // Test with empty bytes
    let result = ResolvedHeader::from_vm_bytes(&[]);
    assert!(result.is_err());
}

/// Test ABI version compatibility check
#[test]
fn test_abi_version_compatibility() {
    // ResolvedHeader and ResolvedCell use Molecule v1
    assert!(ResolvedHeader::is_abi_compatible(VmAbiNegotiator::ABI_VERSION_MOLECULE_V1));
    assert!(ResolvedCell::is_abi_compatible(VmAbiNegotiator::ABI_VERSION_MOLECULE_V1));

    // Old pre-Molecule ABI numbers are not compatible.
    assert!(!ResolvedHeader::is_abi_compatible(0x0001));
    assert!(!ResolvedCell::is_abi_compatible(0x0001));

    // Unknown versions should not be compatible
    assert!(!ResolvedHeader::is_abi_compatible(0x9999));
    assert!(!ResolvedCell::is_abi_compatible(0x9999));
}

/// Test that VmSerializable implementations are consistent
#[test]
fn test_vm_serializable_consistency() {
    // Create test data
    let header = ResolvedHeader {
        hash: [0xAA; 32],
        version: 1,
        parent_hash: [0xBB; 32],
        transactions_root: [0xCC; 32],
        proposals_hash: [0xDD; 32],
        cell_commitment: [0xEE; 32],
        cell_root: [0xFF; 32],
        segment_root: [0x11; 32],
        timestamp: 1000,
        compact_target: 0x1d00ffff,
        nonce: 42,
        number: 100,
        dao: [0x22; 32],
        epoch: 50,
        uncles_hash: [0x33; 32],
    };

    // Multiple serializations should produce same bytes
    let bytes1 = header.to_vm_bytes();
    let bytes2 = header.to_vm_bytes();
    assert_eq!(bytes1, bytes2);

    // Multiple deserializations should produce same result
    let restored1 = ResolvedHeader::from_vm_bytes(&bytes1).unwrap();
    let restored2 = ResolvedHeader::from_vm_bytes(&bytes1).unwrap();
    assert_eq!(restored1, restored2);
}
