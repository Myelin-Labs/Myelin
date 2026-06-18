# Myelin Serialization API Guide

The public/default Myelin serialization path is Molecule-compatible. Legacy
custom formats exist only behind explicit compatibility switches and must not be
used for new VM ABI, state, typed metadata, scheduler-witness, or default
storage bytes.

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

`VersionedSerializable` requires an explicit payload codec. There is no default
derive-based codec.

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

    fn upgrade_from(version: u8, bytes: &[u8]) -> Result<Self, SerializationError> {
        match version {
            1 if bytes.len() == 8 => {
                let value = u64::from_le_bytes(bytes.try_into().expect("length checked"));
                Ok(Self { value })
            }
            1 => Err(SerializationError::DeserializationFailed(
                "MyData payload must be 8 bytes".to_string(),
            )),
            _ => Err(SerializationError::UpgradePathNotAvailable {
                from: version,
                to: Self::CURRENT_VERSION,
            }),
        }
    }
}

let data = MyData { value: 42 };
let envelope = VersionedEnvelope::new(&data)?;
let bytes = envelope.to_bytes();
let restored = VersionedEnvelope::<MyData>::from_bytes(&bytes)?.parse()?;
assert_eq!(data, restored);
```

## VM ABI Bytes

`VmSerializable` describes script-visible bytes. Current public/default objects
advertise the Molecule ABI version.

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

## Cache, Streaming, Integrity, and Compression

These helpers all sit above the same explicit serialization model:

- `SerializationCache` stores `serialize_to_bytes` results.
- `StreamingSerializer` writes `VersionedEnvelope::to_bytes()` frames.
- `SecureEnvelope::to_bytes()` uses its own Molecule-compatible envelope.
- `CompressedEnvelope` compresses caller-provided bytes and does not define a
  consensus codec.

## Format Policy

- `0x80..=0xFF`: Molecule-compatible public/default envelope formats.
- `0x00..=0x7F`: explicit legacy compatibility only.

New protocol evidence should use Molecule-compatible bytes and should be
reported with its semantic profile and CKB projection status.
