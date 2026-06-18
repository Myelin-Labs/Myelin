// SPDX-License-Identifier: MIT
// Copyright (C) 2026 Spora developers
//
// Serialization Layer Usage Examples
//
// This example demonstrates how to use the versioned serialization framework.

use spora_exec::{
    CellInput, CellOutput, CellTx, OutPoint, ResolvedHeader, Script, VersionedEnvelope, VersionedSerializable, VmAbiNegotiator,
    VmSerializable,
};

/// Example: Storing a CellTx with version envelope
fn store_cell_tx_example() {
    // Create a transaction
    let tx = create_sample_tx();

    // Wrap in versioned envelope for storage
    let envelope = VersionedEnvelope::new(&tx).expect("serialization should succeed");

    // The envelope contains:
    // - format_version: 0x00 (Borsh)
    // - schema_version: 1 (current schema)
    // - payload: serialized bytes

    println!("Format version: 0x{:02X}", envelope.format_version());
    println!("Schema version: {}", envelope.schema_version());
    println!("Payload size: {} bytes", envelope.payload_size());

    // Serialize envelope for storage (e.g., RocksDB)
    let storage_bytes = borsh::to_vec(&envelope).expect("envelope serialization should succeed");

    // Later: deserialize envelope and parse content
    let restored_envelope: VersionedEnvelope<CellTx> = borsh::from_slice(&storage_bytes).expect("deserialization should succeed");
    let restored_tx = restored_envelope.parse().expect("parsing should succeed");

    assert_eq!(tx.id(), restored_tx.id());
    println!("Transaction roundtrip successful!");
}

/// Example: VM ABI serialization for ResolvedHeader
fn vm_abi_example() {
    // Create a resolved header
    let header = ResolvedHeader {
        hash: [0xAA; 32],
        version: 1,
        parents_by_level: vec![vec![[0xBB; 32]]],
        hash_merkle_root: [0xCC; 32],
        accepted_id_merkle_root: [0xDD; 32],
        cell_commitment: [0xEE; 32],
        cell_root: [0xFF; 32],
        segment_root: [0x11; 32],
        timestamp: 1234567890,
        bits: 0x1d00ffff,
        nonce: 42,
        daa_score: 1000,
        blue_work: [0x22; 24],
        blue_score: 500,
        pruning_point: [0x33; 32],
    };

    // Serialize for VM (uses VmSerializable trait)
    let vm_bytes = header.to_vm_bytes();
    println!("VM bytes size: {} bytes", vm_bytes.len());
    println!("ABI version: 0x{:04X}", ResolvedHeader::abi_version());

    // VM deserializes using the same trait
    let restored_header = ResolvedHeader::from_vm_bytes(&vm_bytes).expect("VM deserialization should succeed");

    assert_eq!(header.hash, restored_header.hash);
    println!("VM ABI roundtrip successful!");
}

/// Example: ABI version negotiation
fn abi_negotiation_example() {
    // VM capabilities (what the VM supports)
    let vm_capabilities = VmAbiNegotiator::default_capabilities();
    println!("VM supports ABI versions: {:?}", vm_capabilities);

    // Script requests a specific ABI version
    let script_version = VmAbiNegotiator::ABI_VERSION_BORSH_V1;

    // Negotiate
    match VmAbiNegotiator::negotiate(script_version, &vm_capabilities) {
        Ok(agreed_version) => {
            println!("ABI negotiation successful: 0x{:04X}", agreed_version);
        }
        Err(e) => {
            println!("ABI negotiation failed: {}", e);
        }
    }

    // Example: Future Molecule support with fallback
    let future_script_version = VmAbiNegotiator::ABI_VERSION_MOLECULE_V1;
    match VmAbiNegotiator::negotiate(future_script_version, &vm_capabilities) {
        Ok(agreed_version) => {
            // Falls back to Borsh if Molecule not available
            println!("Fallback ABI version: 0x{:04X}", agreed_version);
        }
        Err(e) => {
            println!("Negotiation failed: {}", e);
        }
    }
}

/// Example: Schema evolution
fn schema_evolution_example() {
    // Current schema version
    println!("CellTx schema version: {}", CellTx::CURRENT_VERSION);

    // When schema changes in the future:
    // 1. Increment CURRENT_VERSION
    // 2. Implement upgrade_from() to handle old versions
    // 3. Old data automatically upgrades on read
}

// Helper function to create a sample transaction
fn create_sample_tx() -> CellTx {
    let lock_script = Script::new([0x00; 32], 0, vec![0xAB; 20]);
    let output = CellOutput { lock: lock_script, type_: None, capacity: 1000 };

    CellTx::new(vec![CellInput::new(OutPoint::new([0x11; 32], 0), 0)], vec![], vec![output], vec![vec![]], vec![vec![0xCC; 65]])
        .expect("valid transaction")
}

fn main() {
    println!("=== Spora Serialization Layer Examples ===\n");

    println!("--- Storage Layer (VersionedEnvelope) ---");
    store_cell_tx_example();
    println!();

    println!("--- VM ABI Layer (VmSerializable) ---");
    vm_abi_example();
    println!();

    println!("--- ABI Version Negotiation ---");
    abi_negotiation_example();
    println!();

    println!("--- Schema Evolution ---");
    schema_evolution_example();
    println!();

    println!("All examples completed successfully!");
}
