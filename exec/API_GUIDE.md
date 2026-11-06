# Myelin Serialization API Guide

Myelin uses Molecule-compatible serialization and accepts each type's exact
current schema. Historical formats, schema upgrades and compatibility switches
are not supported. Typed decoding validates the envelope and payload together;
there is no separate permissive version-range validator.

## Basic Storage Bytes

```rust
use myelin_exec::{deserialize_from_bytes, serialize_to_bytes, CellOutput, Script};

let output = CellOutput {
    lock: Script::new([0xAA; 32], 0, vec![]),
    type_: None,
    capacity: 1000,
};

let bytes = serialize_to_bytes(&output)?;
let restored: CellOutput = deserialize_from_bytes(&bytes)?;
```

## VersionedSerializable

`VersionedSerializable` requires both payload codec methods at compile time.
There is no default codec or macro that generates a version-only implementation.

```rust
use myelin_exec::{SerializationError, VersionedEnvelope, VersionedSerializable};

#[derive(Clone, Debug, PartialEq, Eq)]
struct MyData {
    value: u64,
}

impl VersionedSerializable for MyData {
    const CURRENT_VERSION: u8 = 1;

    fn to_versioned_payload(&self) -> Result<Vec<u8>, SerializationError> {
        Ok(self.value.to_le_bytes().to_vec())
    }

    fn from_versioned_payload(bytes: &[u8]) -> Result<Self, SerializationError> {
        if bytes.len() != 8 {
            return Err(SerializationError::DeserializationFailed("MyData payload must be 8 bytes".into()));
        }
        Ok(Self { value: u64::from_le_bytes(bytes.try_into().expect("length checked")) })
    }
}

let data = MyData { value: 42 };
let envelope = VersionedEnvelope::new(&data)?;
let bytes = envelope.to_bytes();
let restored = VersionedEnvelope::<MyData>::from_bytes(&bytes)?.parse()?;
assert_eq!(data, restored);
```

## VM ABI Bytes

`VmSerializable` encodes and decodes script-visible Molecule bytes. The sole
supported version is `VM_ABI_VERSION` (`0x8001`); there is no version negotiation.

```rust
use myelin_exec::{ResolvedHeader, VmSerializable};

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
    compact_target: 0,
    nonce: 0,
    number: 1,
    dao: [0; 32],
    epoch: 0,
    uncles_hash: [0; 32],
};

let bytes = header.to_vm_bytes();
let restored = ResolvedHeader::from_vm_bytes(&bytes)?;
assert_eq!(header.hash, restored.hash);
```

## Cache, Streaming, and Integrity

These helpers all sit above the same explicit serialization model:

- `SerializationCache` stores `serialize_to_bytes` results.
- `StreamingSerializer` writes `VersionedEnvelope::to_bytes()` frames.
  Sequences use `serialize_streaming` / `deserialize_streaming`: an item count
  followed directly by those frames. Metadata comes from `VersionedEnvelope`;
  the encoded buffer length gives the exact size.
- `SecureEnvelope::to_bytes()` uses its own Molecule-compatible envelope, and
  `SecureEnvelope::from_bytes()` accepts only that Molecule-compatible table.

## Format Policy

- `0x80`: the current Molecule-compatible envelope format.
- Every other envelope format and every non-current schema is rejected by typed decoding.

New protocol evidence should use Molecule-compatible bytes and should be
reported with its semantic profile and CKB projection status.
