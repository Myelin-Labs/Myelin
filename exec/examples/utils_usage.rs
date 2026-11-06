// SPDX-License-Identifier: MIT
// Copyright (C) 2026 Myelin developers

use myelin_exec::{
    deserialize_from_bytes,
    serialization::streaming::{deserialize_streaming, serialize_streaming},
    serialize_to_bytes, CellOutput, Script, VersionedEnvelope,
};

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let output = CellOutput { lock: Script::new([0xAA; 32], 0, vec![0xBB; 20]), type_: None, capacity: 1000 };
    let bytes = serialize_to_bytes(&output)?;
    assert_eq!(deserialize_from_bytes::<CellOutput>(&bytes)?, output);
    let envelope = VersionedEnvelope::<CellOutput>::from_bytes(&bytes)?;
    println!("{} bytes; format {:#04x}, schema {}", bytes.len(), envelope.format_version(), envelope.schema_version());

    // Sequences use the streaming format: item count, then current envelopes.
    let outputs = vec![output; 5];
    let mut batch = Vec::new();
    serialize_streaming(&mut batch, &outputs)?;
    assert_eq!(deserialize_streaming::<_, CellOutput>(&mut batch.as_slice())?, outputs);
    println!("{} outputs in {} bytes", outputs.len(), batch.len());
    assert!(deserialize_from_bytes::<CellOutput>(&[0xFF; 3]).is_err());
    Ok(())
}
