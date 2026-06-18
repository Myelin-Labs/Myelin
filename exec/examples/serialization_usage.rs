// SPDX-License-Identifier: MIT
// Copyright (C) 2026 Myelin developers
//
// Serialization Layer Usage Examples

use myelin_exec::{
    CellInput, CellOutput, CellTx, OutPoint, ResolvedHeader, Script, VersionedEnvelope, VersionedSerializable, VmAbiNegotiator,
    VmSerializable,
};

fn store_cell_tx_example() {
    let tx = create_sample_tx();
    let envelope = VersionedEnvelope::new(&tx).expect("serialization should succeed");

    println!("Format version: 0x{:02X}", envelope.format_version());
    println!("Schema version: {}", envelope.schema_version());
    println!("Payload size: {} bytes", envelope.payload_size());

    let storage_bytes = envelope.to_bytes();
    let restored_envelope = VersionedEnvelope::<CellTx>::from_bytes(&storage_bytes).expect("envelope deserialization should succeed");
    let restored_tx = restored_envelope.parse().expect("parsing should succeed");

    assert_eq!(tx.id(), restored_tx.id());
    println!("Transaction roundtrip successful");
}

fn vm_abi_example() {
    let header = ResolvedHeader {
        hash: [0xAA; 32],
        version: 1,
        parent_hash: [0xBB; 32],
        transactions_root: [0xCC; 32],
        proposals_hash: [0xDD; 32],
        cell_commitment: [0xEE; 32],
        cell_root: [0xFF; 32],
        segment_root: [0x11; 32],
        timestamp: 1234567890,
        compact_target: 0x1d00ffff,
        nonce: 42,
        number: 1000,
        dao: [0x22; 32],
        epoch: 500,
        uncles_hash: [0x33; 32],
    };

    let vm_bytes = header.to_vm_bytes();
    println!("VM bytes size: {} bytes", vm_bytes.len());
    println!("ABI version: 0x{:04X}", ResolvedHeader::abi_version());

    let restored_header = ResolvedHeader::from_vm_bytes(&vm_bytes).expect("VM deserialization should succeed");

    assert_eq!(header.hash, restored_header.hash);
    println!("VM ABI roundtrip successful");
}

fn abi_negotiation_example() {
    let vm_capabilities = VmAbiNegotiator::default_capabilities();
    println!("VM supports ABI versions: {:?}", vm_capabilities);

    let script_version = VmAbiNegotiator::ABI_VERSION_MOLECULE_V1;
    match VmAbiNegotiator::negotiate(script_version, &vm_capabilities) {
        Ok(agreed_version) => {
            println!("ABI negotiation successful: 0x{:04X}", agreed_version);
        }
        Err(e) => {
            println!("ABI negotiation failed: {}", e);
        }
    }
}

fn schema_evolution_example() {
    println!("CellTx schema version: {}", CellTx::CURRENT_VERSION);
    println!("Schema upgrades must implement explicit payload codecs.");
}

fn create_sample_tx() -> CellTx {
    let lock_script = Script::new([0x00; 32], 0, vec![0xAB; 20]);
    let output = CellOutput { lock: lock_script, type_: None, capacity: 1000 };

    CellTx::new(vec![CellInput::new(OutPoint::new([0x11; 32], 0), 0)], vec![], vec![output], vec![vec![]], vec![vec![0xCC; 65]])
        .expect("valid transaction")
}

fn main() {
    println!("=== Myelin Serialization Layer Examples ===\n");

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

    println!("All examples completed successfully");
}
