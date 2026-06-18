// SPDX-License-Identifier: MIT
// Copyright (C) 2026 Spora developers
//
// Schema Migration Example
//
// This example demonstrates how to implement schema migration
// when data structures evolve.

use borsh::{BorshDeserialize, BorshSerialize};
use spora_exec::{SerializationError, VersionedEnvelope, VersionedSerializable};

// ============================================================================
// Old Schema (v1)
// ============================================================================

#[derive(Debug, Clone, PartialEq, Eq, BorshSerialize, BorshDeserialize)]
struct UserDataV1 {
    name: String,
    age: u32,
}

// ============================================================================
// New Schema (v2) - Added email field, changed age to birth_year
// ============================================================================

#[derive(Debug, Clone, PartialEq, Eq, BorshSerialize, BorshDeserialize)]
struct UserDataV2 {
    name: String,
    birth_year: u32,       // Changed from age
    email: Option<String>, // New field
}

impl UserDataV2 {
    fn new(name: &str, birth_year: u32, email: Option<&str>) -> Self {
        Self { name: name.to_string(), birth_year, email: email.map(|s| s.to_string()) }
    }
}

// ============================================================================
// VersionedSerializable Implementation with Migration
// ============================================================================

impl VersionedSerializable for UserDataV2 {
    const CURRENT_VERSION: u8 = 2;

    fn upgrade_from(version: u8, bytes: &[u8]) -> Result<Self, SerializationError> {
        match version {
            1 => {
                // Parse old version
                let v1: UserDataV1 =
                    BorshDeserialize::try_from_slice(bytes).map_err(|e| SerializationError::DeserializationFailed(e.to_string()))?;

                // Migrate to new version
                // Assume current year is 2026 for age calculation
                let current_year: u32 = 2026;
                let birth_year = current_year.saturating_sub(v1.age);

                Ok(Self {
                    name: v1.name,
                    birth_year,
                    email: None, // New field, default to None
                })
            }
            2 => {
                // Current version
                BorshDeserialize::try_from_slice(bytes).map_err(|e| SerializationError::DeserializationFailed(e.to_string()))
            }
            _ => Err(SerializationError::UpgradePathNotAvailable { from: version, to: Self::CURRENT_VERSION }),
        }
    }
}

fn main() -> Result<(), Box<dyn std::error::Error>> {
    println!("=== Schema Migration Example ===\n");

    // ============================================================================
    // Scenario 1: Create and store v2 data
    // ============================================================================
    println!("--- Scenario 1: Storing new data (v2) ---");

    let user_v2 = UserDataV2::new("Alice", 1990, Some("alice@example.com"));
    let envelope = VersionedEnvelope::new(&user_v2)?;

    println!("Created user: {:?}", user_v2);
    println!("Schema version: {}", envelope.schema_version());

    // Simulate storing to database
    let stored_bytes = borsh::to_vec(&envelope)?;
    println!("Stored {} bytes to database", stored_bytes.len());

    // ============================================================================
    // Scenario 2: Read v2 data
    // ============================================================================
    println!("\n--- Scenario 2: Reading new data (v2) ---");

    let restored_envelope: VersionedEnvelope<UserDataV2> = borsh::from_slice(&stored_bytes)?;
    let restored_user = restored_envelope.parse()?;

    println!("Restored user: {:?}", restored_user);
    assert_eq!(user_v2, restored_user);
    println!("✓ Data integrity verified");

    // ============================================================================
    // Scenario 3: Simulate old v1 data in database
    // ============================================================================
    println!("\n--- Scenario 3: Migrating old data (v1 → v2) ---");

    // Create old v1 data
    let user_v1 = UserDataV1 {
        name: "Bob".to_string(),
        age: 30, // Will be converted to birth_year
    };

    // Store as v1 (simulating old database entry)
    let mut old_envelope = VersionedEnvelope::<UserDataV2>::default();
    old_envelope.format_version = 0x00; // Borsh
    old_envelope.schema_version = 1; // Old version
    old_envelope.payload = borsh::to_vec(&user_v1)?;
    let old_stored_bytes = borsh::to_vec(&old_envelope)?;

    println!("Old v1 data: {:?}", user_v1);
    println!("Stored with schema version: 1");

    // ============================================================================
    // Scenario 4: Read and auto-migrate v1 data
    // ============================================================================
    println!("\n--- Scenario 4: Auto-migration on read ---");

    let migrated_envelope: VersionedEnvelope<UserDataV2> = borsh::from_slice(&old_stored_bytes)?;
    println!("Detected schema version: {}", migrated_envelope.schema_version());

    let migrated_user = migrated_envelope.parse()?;
    println!("Migrated user: {:?}", migrated_user);

    // Verify migration
    assert_eq!(migrated_user.name, "Bob");
    assert_eq!(migrated_user.birth_year, 1996); // 2026 - 30
    assert_eq!(migrated_user.email, None); // Default value
    println!("✓ Migration successful: age 30 → birth_year 1996");

    // ============================================================================
    // Summary
    // ============================================================================
    println!("\n=== Summary ===");
    println!("✓ New data stored with current schema version (v2)");
    println!("✓ Old data (v1) automatically migrated to v2 on read");
    println!("✓ No data loss during migration");
    println!("✓ Application code only works with v2 structure");

    println!("\nKey benefits:");
    println!("  - Backward compatibility: Old data continues to work");
    println!("  - Forward compatibility: New data can't be read by old code");
    println!("  - Zero-downtime migration: No need to migrate entire database at once");
    println!("  - Type safety: Application always works with current schema");

    Ok(())
}
