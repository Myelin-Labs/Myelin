// SPDX-License-Identifier: MIT
// Copyright (C) 2026 Spora developers
//
// Serialization Utils Usage Example
//
// This example demonstrates the utility functions for serialization.

use spora_exec::{
    serialization::utils::{
        deserialize_from_bytes, deserialize_many, estimate_serialized_size, is_valid_versioned_envelope, peek_format_version,
        peek_schema_version, serialize_many, serialize_to_bytes,
    },
    CellOutput, Script, VersionedSerializable,
};

fn main() -> Result<(), Box<dyn std::error::Error>> {
    println!("=== Serialization Utils Usage Example ===\n");

    // ============================================================================
    // Basic Serialization
    // ============================================================================
    println!("--- Basic Serialization ---");

    let output = CellOutput {
        lock: Script::new([0xAA; 32], 0, vec![0xBB; 20]),
        type_: Some(Script::new([0xCC; 32], 1, vec![0xDD; 10])),
        capacity: 1000,
    };

    // Serialize to bytes
    let bytes = serialize_to_bytes(&output)?;
    println!("Serialized {} bytes", bytes.len());

    // Estimate size
    let estimated = estimate_serialized_size(&output)?;
    println!("Estimated size: {} bytes", estimated);

    // Deserialize
    let restored: CellOutput = deserialize_from_bytes(&bytes)?;
    assert_eq!(output, restored);
    println!("✓ Roundtrip successful");

    // ============================================================================
    // Peeking at Envelope Metadata
    // ============================================================================
    println!("\n--- Peeking at Envelope Metadata ---");

    let format_version = peek_format_version(&bytes).unwrap();
    println!("Format version: 0x{:02X}", format_version);

    let schema_version = peek_schema_version(&bytes).unwrap();
    println!("Schema version: {}", schema_version);
    println!("Expected schema version: {}", CellOutput::CURRENT_VERSION);

    // Validate envelope
    if is_valid_versioned_envelope(&bytes) {
        println!("✓ Valid VersionedEnvelope");
    }

    // ============================================================================
    // Batch Serialization
    // ============================================================================
    println!("\n--- Batch Serialization ---");

    let outputs: Vec<CellOutput> = (0..5)
        .map(|i| CellOutput {
            lock: Script::new([i as u8; 32], 0, vec![i as u8; 20]),
            type_: if i % 2 == 0 { Some(Script::new([0x11; 32], 1, vec![0x22; 10])) } else { None },
            capacity: 1000 + i as u64,
        })
        .collect();

    println!("Serializing {} outputs", outputs.len());
    let batch_bytes = serialize_many(&outputs)?;
    println!("Batch size: {} bytes", batch_bytes.len());

    let restored_batch = deserialize_many::<CellOutput>(&batch_bytes)?;
    assert_eq!(outputs.len(), restored_batch.len());
    println!("✓ Batch roundtrip successful");

    // Verify each item
    for (i, (orig, rest)) in outputs.iter().zip(restored_batch.iter()).enumerate() {
        assert_eq!(orig, rest, "Item {} mismatch", i);
    }
    println!("✓ All {} items verified", outputs.len());

    // ============================================================================
    // Error Handling
    // ============================================================================
    println!("\n--- Error Handling ---");

    // Invalid data
    let invalid_bytes = vec![0xFF, 0xFF, 0xFF];
    match deserialize_from_bytes::<CellOutput>(&invalid_bytes) {
        Ok(_) => println!("✗ Should have failed"),
        Err(e) => println!("✓ Correctly rejected invalid data: {}", e),
    }

    // Empty data
    match deserialize_from_bytes::<CellOutput>(&[]) {
        Ok(_) => println!("✗ Should have failed"),
        Err(e) => println!("✓ Correctly rejected empty data: {}", e),
    }

    // Invalid envelope check
    if !is_valid_versioned_envelope(&invalid_bytes) {
        println!("✓ Correctly identified invalid envelope");
    }

    // ============================================================================
    // Performance Comparison
    // ============================================================================
    println!("\n--- Performance Comparison ---");

    let large_outputs: Vec<CellOutput> = (0..1000)
        .map(|i| CellOutput { lock: Script::new([i as u8; 32], 0, vec![0xBB; 20]), type_: None, capacity: i as u64 })
        .collect();

    // Individual serialization
    let start = std::time::Instant::now();
    let mut individual_bytes = Vec::new();
    for output in &large_outputs {
        let bytes = serialize_to_bytes(output).unwrap();
        individual_bytes.extend_from_slice(&(bytes.len() as u32).to_le_bytes());
        individual_bytes.extend_from_slice(&bytes);
    }
    let individual_time = start.elapsed();
    println!("Individual serialization: {:?} for {} items", individual_time, large_outputs.len());

    // Batch serialization
    let start = std::time::Instant::now();
    let batch_bytes = serialize_many(&large_outputs).unwrap();
    let batch_time = start.elapsed();
    println!("Batch serialization: {:?} for {} items", batch_time, large_outputs.len());

    // Size comparison
    println!("Individual size: {} bytes", individual_bytes.len());
    println!("Batch size: {} bytes", batch_bytes.len());

    // ============================================================================
    // Summary
    // ============================================================================
    println!("\n=== Summary ===");
    println!("✓ serialize_to_bytes / deserialize_from_bytes - Simple one-liner serialization");
    println!("✓ peek_format_version / peek_schema_version - Quick metadata inspection");
    println!("✓ is_valid_versioned_envelope - Fast validation");
    println!("✓ serialize_many / deserialize_many - Efficient batch operations");
    println!("✓ estimate_serialized_size - Size prediction");

    Ok(())
}
