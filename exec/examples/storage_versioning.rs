// SPDX-License-Identifier: MIT
// Copyright (C) 2026 Myelin developers
//
// Storage Layer Versioning Example

use myelin_exec::{CellTx, VersionedEnvelope, VersionedSerializable, CELLTX_SCHEMA_VERSION};
use std::collections::HashMap;

struct MockStorage {
    data: HashMap<Vec<u8>, Vec<u8>>,
}

impl MockStorage {
    fn new() -> Self {
        Self { data: HashMap::new() }
    }

    fn put<T: VersionedSerializable>(&mut self, key: &[u8], value: &T) -> Result<(), StorageError> {
        let envelope = VersionedEnvelope::new(value)?;
        self.data.insert(key.to_vec(), envelope.to_bytes());
        Ok(())
    }

    fn get<T: VersionedSerializable>(&self, key: &[u8]) -> Result<Option<T>, StorageError> {
        match self.data.get(key) {
            Some(bytes) => {
                let envelope = VersionedEnvelope::<T>::from_bytes(bytes)?;
                Ok(Some(envelope.parse()?))
            }
            None => Ok(None),
        }
    }

    fn get_raw(&self, key: &[u8]) -> Option<&Vec<u8>> {
        self.data.get(key)
    }
}

#[derive(Debug, thiserror::Error)]
enum StorageError {
    #[error("serialization error: {0}")]
    Serialization(#[from] myelin_exec::SerializationError),
}

fn main() -> Result<(), StorageError> {
    println!("=== Storage Layer Versioning Example ===\n");

    let mut storage = MockStorage::new();
    let tx = create_sample_tx();
    let tx_id = tx.id();
    println!("Created transaction with ID: {:02x?}", &tx_id[..8]);
    println!("Current schema version: {}", CELLTX_SCHEMA_VERSION);

    let key = format!("tx:{}", hex::encode(&tx_id));
    storage.put(key.as_bytes(), &tx)?;
    println!("\nStored transaction with VersionedEnvelope");

    if let Some(raw) = storage.get_raw(key.as_bytes()) {
        let envelope = VersionedEnvelope::<CellTx>::from_bytes(raw)?;
        println!("Raw storage size: {} bytes", raw.len());
        println!("Format version: 0x{:02X}", envelope.format_version());
        println!("Schema version: {}", envelope.schema_version());
    }

    let retrieved: CellTx = storage.get(key.as_bytes())?.expect("transaction should exist");
    assert_eq!(tx.id(), retrieved.id());
    println!("\nRetrieved transaction matches original");

    demonstrate_schema_evolution();
    demonstrate_format_policy();

    println!("\n=== All storage examples completed successfully ===");
    Ok(())
}

fn demonstrate_schema_evolution() {
    println!("\n=== Schema Evolution Scenario ===");
    println!("Current schema version: {}", CellTx::CURRENT_VERSION);
    println!("When schema changes:");
    println!("  1. Increment CURRENT_VERSION constant");
    println!("  2. Implement upgrade_from() for backward compatibility");
    println!("  3. Keep each version's payload codec explicit");
}

fn demonstrate_format_policy() {
    println!("\n=== Format Policy ===");
    println!("Current public/default format: Molecule-compatible envelope (format_version = 0x80)");
    println!("Legacy custom formats are not default storage or public VM ABI bytes.");
}

fn create_sample_tx() -> CellTx {
    use myelin_exec::{CellInput, CellOutput, OutPoint, Script};

    let lock_script = Script::new([0x00; 32], 0, vec![0xAB; 20]);
    let output = CellOutput { lock: lock_script, type_: None, capacity: 1000 };

    CellTx::new(vec![CellInput::new(OutPoint::new([0x11; 32], 0), 0)], vec![], vec![output], vec![vec![]], vec![vec![0xCC; 65]])
        .expect("valid transaction")
}

mod hex {
    pub fn encode(bytes: &[u8]) -> String {
        bytes.iter().map(|b| format!("{:02x}", b)).collect()
    }
}
