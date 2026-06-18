// SPDX-License-Identifier: MIT
// Copyright (C) 2026 Spora developers
//
// Storage Layer Versioning Example
//
// This example demonstrates how to use VersionedEnvelope for RocksDB storage.

use spora_exec::{CellTx, VersionedEnvelope, VersionedSerializable, CELLTX_SCHEMA_VERSION};
use std::collections::HashMap;

/// Simulated RocksDB storage
struct MockStorage {
    data: HashMap<Vec<u8>, Vec<u8>>,
}

impl MockStorage {
    fn new() -> Self {
        Self { data: HashMap::new() }
    }

    /// Store a value with version envelope
    fn put<T: VersionedSerializable>(&mut self, key: &[u8], value: &T) -> Result<(), StorageError> {
        let envelope = VersionedEnvelope::new(value)?;
        let bytes = borsh::to_vec(&envelope)?;
        self.data.insert(key.to_vec(), bytes);
        Ok(())
    }

    /// Get a value from version envelope
    fn get<T: VersionedSerializable>(&self, key: &[u8]) -> Result<Option<T>, StorageError> {
        match self.data.get(key) {
            Some(bytes) => {
                let envelope: VersionedEnvelope<T> = borsh::from_slice(bytes)?;
                Ok(Some(envelope.parse()?))
            }
            None => Ok(None),
        }
    }

    /// Get raw bytes (for debugging)
    fn get_raw(&self, key: &[u8]) -> Option<&Vec<u8>> {
        self.data.get(key)
    }
}

#[derive(Debug, thiserror::Error)]
enum StorageError {
    #[error("serialization error: {0}")]
    Serialization(#[from] spora_exec::SerializationError),
    #[error("borsh error: {0}")]
    Borsh(String),
}

impl From<std::io::Error> for StorageError {
    fn from(e: std::io::Error) -> Self {
        StorageError::Borsh(e.to_string())
    }
}

fn main() -> Result<(), StorageError> {
    println!("=== Storage Layer Versioning Example ===\n");

    let mut storage = MockStorage::new();

    // Create a sample transaction
    let tx = create_sample_tx();
    let tx_id = tx.id();
    println!("Created transaction with ID: {:02x?}", &tx_id[..8]);
    println!("Current schema version: {}", CELLTX_SCHEMA_VERSION);

    // Store transaction with version envelope
    let key = format!("tx:{}", hex::encode(&tx_id));
    storage.put(key.as_bytes(), &tx)?;
    println!("\nStored transaction with VersionedEnvelope");

    // Show raw storage format
    if let Some(raw) = storage.get_raw(key.as_bytes()) {
        println!("Raw storage size: {} bytes", raw.len());
        println!("Format version: 0x{:02X}", raw[0]);
        println!("Schema version: {}", raw[1]);
    }

    // Retrieve transaction
    let retrieved: CellTx = storage.get(key.as_bytes())?.expect("transaction should exist");
    assert_eq!(tx.id(), retrieved.id());
    println!("\nRetrieved transaction matches original: ✓");

    // Demonstrate schema evolution scenario
    println!("\n=== Schema Evolution Scenario ===");
    demonstrate_schema_evolution();

    // Demonstrate format migration scenario
    println!("\n=== Format Migration Scenario ===");
    demonstrate_format_migration();

    println!("\n=== All storage examples completed successfully! ===");
    Ok(())
}

fn demonstrate_schema_evolution() {
    println!("Current schema version: {}", CellTx::CURRENT_VERSION);
    println!("When schema changes:");
    println!("  1. Increment CURRENT_VERSION constant");
    println!("  2. Implement upgrade_from() for backward compatibility");
    println!("  3. Old data automatically upgrades on read");
    println!();
    println!("Example upgrade path implementation:");
    println!("  impl VersionedSerializable for CellTx {{");
    println!("      const CURRENT_VERSION: u8 = 2; // New version");
    println!("      ");
    println!("      fn upgrade_from(version: u8, bytes: &[u8]) -> Result<Self, SerializationError> {{");
    println!("          match version {{");
    println!("              1 => {{");
    println!("                  // Parse v1 format");
    println!("                  let v1: CellTxV1 = BorshDeserialize::try_from_slice(bytes)?;");
    println!("                  // Migrate to v2");
    println!("                  Ok(v1.into())");
    println!("              }}");
    println!("              2 => BorshDeserialize::try_from_slice(bytes)");
    println!("                  .map_err(|e| SerializationError::DeserializationFailed(e.to_string())),");
    println!("              _ => Err(SerializationError::UnsupportedVersion(version)),");
    println!("          }}");
    println!("      }}");
    println!("  }}");
}

fn demonstrate_format_migration() {
    println!("Current format: Borsh (format_version = 0x00)");
    println!("Future format: Molecule (format_version = 0x80)");
    println!();
    println!("VersionedEnvelope supports multiple formats:");
    println!("  - 0x00-0x7F: Reserved for Borsh variants");
    println!("  - 0x80-0xFF: Reserved for Molecule variants");
    println!();
    println!("When migrating to Molecule:");
    println!("  1. Implement MoleculeSerializer in serialization::molecule_compat");
    println!("  2. Add new format_version variant (e.g., 0x80)");
    println!("  3. Update VersionedEnvelope::parse() to handle new format");
    println!("  4. Existing Borsh data continues to work");
}

fn create_sample_tx() -> CellTx {
    use spora_exec::{CellInput, CellOutput, OutPoint, Script};

    let lock_script = Script::new([0x00; 32], 0, vec![0xAB; 20]);
    let output = CellOutput { lock: lock_script, type_: None, capacity: 1000 };

    CellTx::new(vec![CellInput::new(OutPoint::new([0x11; 32], 0), 0)], vec![], vec![output], vec![vec![]], vec![vec![0xCC; 65]])
        .expect("valid transaction")
}

// Helper for hex encoding
mod hex {
    pub fn encode(bytes: &[u8]) -> String {
        bytes.iter().map(|b| format!("{:02x}", b)).collect()
    }
}
