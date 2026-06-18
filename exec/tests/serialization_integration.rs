// SPDX-License-Identifier: MIT
// Copyright (C) 2026 Myelin developers
//
// Serialization Layer Integration Tests
//
// These tests verify the integration of serialization components
// across the exec crate.

use myelin_exec::SerializationError;
use myelin_exec::{
    CellDep, CellInput, CellOutput, CellTx, DepType, OutPoint, Script, VersionedEnvelope, VersionedSerializable, VmAbiNegotiator,
    CELLTX_SCHEMA_VERSION,
};

/// Test that all CellTx types can be serialized with VersionedEnvelope
#[test]
fn test_all_celltx_types_versioned_serialization() {
    // OutPoint
    let outpoint = OutPoint::new([0xAA; 32], 42);
    let envelope = VersionedEnvelope::new(&outpoint).expect("OutPoint should serialize");
    assert_eq!(envelope.schema_version(), OutPoint::CURRENT_VERSION);
    let restored: OutPoint = envelope.parse().expect("OutPoint should deserialize");
    assert_eq!(outpoint, restored);

    // Script
    let script = Script::new([0xBB; 32], 1, vec![0xCC; 20]);
    let envelope = VersionedEnvelope::new(&script).expect("Script should serialize");
    assert_eq!(envelope.schema_version(), Script::CURRENT_VERSION);
    let restored: Script = envelope.parse().expect("Script should deserialize");
    assert_eq!(script, restored);

    // CellOutput
    let output = CellOutput { lock: script.clone(), type_: Some(Script::new([0xDD; 32], 2, vec![0xEE; 10])), capacity: 1000 };
    let envelope = VersionedEnvelope::new(&output).expect("CellOutput should serialize");
    assert_eq!(envelope.schema_version(), CellOutput::CURRENT_VERSION);
    let restored: CellOutput = envelope.parse().expect("CellOutput should deserialize");
    assert_eq!(output, restored);

    // CellInput
    let input = CellInput::new(outpoint, 0x12345678);
    let envelope = VersionedEnvelope::new(&input).expect("CellInput should serialize");
    assert_eq!(envelope.schema_version(), CellInput::CURRENT_VERSION);
    let restored: CellInput = envelope.parse().expect("CellInput should deserialize");
    assert_eq!(input, restored);

    // CellDep
    let cell_dep = CellDep { out_point: outpoint, dep_type: DepType::Code };
    let envelope = VersionedEnvelope::new(&cell_dep).expect("CellDep should serialize");
    assert_eq!(envelope.schema_version(), CellDep::CURRENT_VERSION);
    let restored: CellDep = envelope.parse().expect("CellDep should deserialize");
    assert_eq!(cell_dep, restored);

    // DepType
    let dep_type = DepType::DepGroup;
    let envelope = VersionedEnvelope::new(&dep_type).expect("DepType should serialize");
    assert_eq!(envelope.schema_version(), DepType::CURRENT_VERSION);
    let restored: DepType = envelope.parse().expect("DepType should deserialize");
    assert_eq!(dep_type, restored);

    // Full CellTx
    let tx = CellTx::new(vec![input], vec![cell_dep], vec![output], vec![vec![0x11; 100]], vec![vec![0x22; 65]])
        .expect("valid transaction");

    let envelope = VersionedEnvelope::new(&tx).expect("CellTx should serialize");
    assert_eq!(envelope.schema_version(), CellTx::CURRENT_VERSION);
    assert_eq!(envelope.schema_version(), CELLTX_SCHEMA_VERSION);
    let restored: CellTx = envelope.parse().expect("CellTx should deserialize");
    assert_eq!(tx.id(), restored.id());
}

/// Test schema version consistency across all CellTx types
#[test]
fn test_celltx_schema_version_consistency() {
    // All CellTx types should use the same schema version
    assert_eq!(OutPoint::CURRENT_VERSION, CELLTX_SCHEMA_VERSION);
    assert_eq!(Script::CURRENT_VERSION, CELLTX_SCHEMA_VERSION);
    assert_eq!(CellOutput::CURRENT_VERSION, CELLTX_SCHEMA_VERSION);
    assert_eq!(CellInput::CURRENT_VERSION, CELLTX_SCHEMA_VERSION);
    assert_eq!(CellDep::CURRENT_VERSION, CELLTX_SCHEMA_VERSION);
    assert_eq!(DepType::CURRENT_VERSION, CELLTX_SCHEMA_VERSION);
    assert_eq!(CellTx::CURRENT_VERSION, CELLTX_SCHEMA_VERSION);
}

/// Test VersionedEnvelope serialization format
#[test]
fn test_versioned_envelope_format() {
    let tx = create_sample_tx();
    let envelope = VersionedEnvelope::new(&tx).expect("should create envelope");

    // Serialize envelope to Molecule-compatible bytes
    let bytes = envelope.to_bytes();

    // Deserialize back
    let restored = VersionedEnvelope::<CellTx>::from_bytes(&bytes).expect("should deserialize envelope");

    // Parse the content
    let restored_tx: CellTx = restored.parse().expect("should parse content");
    assert_eq!(tx.id(), restored_tx.id());
}

/// Test ABI version negotiation scenarios
#[test]
fn test_abi_version_negotiation_scenarios() {
    // Scenario 1: Molecule exact match
    let caps = vec![VmAbiNegotiator::ABI_VERSION_MOLECULE_V1];
    let result = VmAbiNegotiator::negotiate(VmAbiNegotiator::ABI_VERSION_MOLECULE_V1, &caps);
    assert_eq!(result.unwrap(), VmAbiNegotiator::ABI_VERSION_MOLECULE_V1);

    // Scenario 2: Molecule request when VM only advertises an obsolete ABI
    let caps = vec![0x0001];
    let result = VmAbiNegotiator::negotiate(VmAbiNegotiator::ABI_VERSION_MOLECULE_V1, &caps);
    assert!(result.is_err());

    // Scenario 3: Multiple capabilities with Molecule present
    let caps = vec![0x0001, 0x0002, VmAbiNegotiator::ABI_VERSION_MOLECULE_V1];
    let result = VmAbiNegotiator::negotiate(VmAbiNegotiator::ABI_VERSION_MOLECULE_V1, &caps);
    assert_eq!(result.unwrap(), VmAbiNegotiator::ABI_VERSION_MOLECULE_V1);

    // Scenario 4: Non-Molecule script request is unsupported
    let caps = vec![0x0002, 0x0003];
    let result = VmAbiNegotiator::negotiate(0x0001, &caps);
    assert!(result.is_err());
}

/// Test default capabilities
#[test]
fn test_default_vm_capabilities() {
    let caps = VmAbiNegotiator::default_capabilities();
    assert_eq!(caps, vec![VmAbiNegotiator::ABI_VERSION_MOLECULE_V1]);
    assert!(!caps.contains(&0x0001));
}

/// Test that VersionedEnvelope can handle empty payloads gracefully
#[test]
fn test_versioned_envelope_empty_payload() {
    #[derive(Debug, Clone, PartialEq, Eq)]
    struct EmptyData;

    impl VersionedSerializable for EmptyData {
        const CURRENT_VERSION: u8 = 1;

        fn to_versioned_payload(&self) -> Result<Vec<u8>, SerializationError> {
            Ok(Vec::new())
        }

        fn upgrade_from(version: u8, bytes: &[u8]) -> Result<Self, SerializationError> {
            if version != Self::CURRENT_VERSION {
                return Err(SerializationError::UpgradePathNotAvailable { from: version, to: Self::CURRENT_VERSION });
            }
            if bytes.is_empty() {
                Ok(Self)
            } else {
                Err(SerializationError::DeserializationFailed("EmptyData payload must be empty".to_string()))
            }
        }
    }

    let data = EmptyData;
    let envelope = VersionedEnvelope::new(&data).expect("should serialize empty data");
    let restored: EmptyData = envelope.parse().expect("should deserialize empty data");
    assert_eq!(data, restored);
}

/// Test large payload handling
#[test]
fn test_versioned_envelope_large_payload() {
    #[derive(Debug, Clone, PartialEq, Eq)]
    struct LargeData {
        data: Vec<u8>,
    }

    impl VersionedSerializable for LargeData {
        const CURRENT_VERSION: u8 = 1;

        fn to_versioned_payload(&self) -> Result<Vec<u8>, SerializationError> {
            Ok(self.data.clone())
        }

        fn upgrade_from(version: u8, bytes: &[u8]) -> Result<Self, SerializationError> {
            if version == Self::CURRENT_VERSION {
                Ok(Self { data: bytes.to_vec() })
            } else {
                Err(SerializationError::UpgradePathNotAvailable { from: version, to: Self::CURRENT_VERSION })
            }
        }
    }

    let large_data = LargeData {
        data: vec![0xAB; 1024 * 1024], // 1MB of data
    };

    let envelope = VersionedEnvelope::new(&large_data).expect("should serialize large data");
    let restored: LargeData = envelope.parse().expect("should deserialize large data");
    assert_eq!(large_data.data.len(), restored.data.len());
    assert_eq!(large_data.data[..100], restored.data[..100]);
}

/// Test multiple serialization roundtrips
#[test]
fn test_multiple_roundtrips() {
    let tx = create_sample_tx();

    // First roundtrip
    let envelope1 = VersionedEnvelope::new(&tx).unwrap();
    let bytes1 = envelope1.to_bytes();
    let restored1 = VersionedEnvelope::<CellTx>::from_bytes(&bytes1).unwrap();
    let tx1 = restored1.parse().unwrap();

    // Second roundtrip
    let envelope2 = VersionedEnvelope::new(&tx1).unwrap();
    let bytes2 = envelope2.to_bytes();
    let restored2 = VersionedEnvelope::<CellTx>::from_bytes(&bytes2).unwrap();
    let tx2 = restored2.parse().unwrap();

    // All should be equal
    assert_eq!(tx.id(), tx1.id());
    assert_eq!(tx1.id(), tx2.id());
    assert_eq!(bytes1, bytes2);
}

/// Helper function to create a sample transaction
fn create_sample_tx() -> CellTx {
    let lock_script = Script::new([0x00; 32], 0, vec![0xAB; 20]);
    let output = CellOutput { lock: lock_script, type_: None, capacity: 1000 };

    CellTx::new(vec![CellInput::new(OutPoint::new([0x11; 32], 0), 0)], vec![], vec![output], vec![vec![]], vec![vec![0xCC; 65]])
        .expect("valid transaction")
}
