// SPDX-License-Identifier: MIT
// Copyright (C) 2026 Myelin developers
//
//! Typed helpers for the current versioned envelope.

use crate::serialization::{SerializationError, VersionedEnvelope, VersionedSerializable};

/// Encode a value in its current versioned envelope.
///
/// ```
/// use myelin_exec::{deserialize_from_bytes, serialize_to_bytes, CellOutput, Script};
///
/// let output = CellOutput { lock: Script::new([0xAA; 32], 0, vec![]), type_: None, capacity: 1000 };
/// let bytes = serialize_to_bytes(&output)?;
/// let restored: CellOutput = deserialize_from_bytes(&bytes)?;
/// assert_eq!(output, restored);
/// # Ok::<(), myelin_exec::SerializationError>(())
/// ```
pub fn serialize_to_bytes<T: VersionedSerializable>(value: &T) -> Result<Vec<u8>, SerializationError> {
    Ok(VersionedEnvelope::new(value)?.to_bytes())
}

/// Decode the exact current format/schema and its typed payload.
pub fn deserialize_from_bytes<T: VersionedSerializable>(bytes: &[u8]) -> Result<T, SerializationError> {
    VersionedEnvelope::<T>::from_bytes(bytes)?.parse()
}
