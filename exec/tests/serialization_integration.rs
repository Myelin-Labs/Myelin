// SPDX-License-Identifier: MIT
// Copyright (C) 2026 Myelin developers
//
// Serialization Layer Integration Tests
//
// These tests verify the integration of serialization components
// across the exec crate.

use myelin_exec::{
    CellDep, CellInput, CellOutput, CellTx, DepType, OutPoint, Script, VersionedEnvelope, VersionedSerializable, CELLTX_SCHEMA_VERSION,
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
    let bytes = envelope.to_bytes();
    let restored: CellTx = VersionedEnvelope::<CellTx>::from_bytes(&bytes).unwrap().parse().unwrap();
    assert_eq!(tx, restored);
    assert_eq!(bytes, VersionedEnvelope::new(&restored).unwrap().to_bytes());
}
