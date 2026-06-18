// SPDX-License-Identifier: MIT
// Copyright (C) 2026 Myelin developers
//
// Small Molecule-compatible table/vector helpers for state persistence.

use crate::{Result, StateError};

const NUMBER_SIZE: usize = 4;

pub(crate) fn encode_u32(value: u32) -> Vec<u8> {
    value.to_le_bytes().to_vec()
}

pub(crate) fn decode_u32(bytes: &[u8], ty: &'static str) -> Result<u32> {
    if bytes.len() != 4 {
        return invalid(ty, format!("expected 4 bytes, got {}", bytes.len()));
    }
    Ok(u32::from_le_bytes([bytes[0], bytes[1], bytes[2], bytes[3]]))
}

pub(crate) fn encode_u64(value: u64) -> Vec<u8> {
    value.to_le_bytes().to_vec()
}

pub(crate) fn decode_u64(bytes: &[u8], ty: &'static str) -> Result<u64> {
    if bytes.len() != 8 {
        return invalid(ty, format!("expected 8 bytes, got {}", bytes.len()));
    }
    Ok(u64::from_le_bytes([bytes[0], bytes[1], bytes[2], bytes[3], bytes[4], bytes[5], bytes[6], bytes[7]]))
}

pub(crate) fn encode_bool(value: bool) -> Vec<u8> {
    vec![u8::from(value)]
}

pub(crate) fn decode_bool(bytes: &[u8], ty: &'static str) -> Result<bool> {
    match bytes {
        [0] => Ok(false),
        [1] => Ok(true),
        _ => invalid(ty, "expected boolean byte 0 or 1"),
    }
}

pub(crate) fn decode_array32(bytes: &[u8], ty: &'static str) -> Result<[u8; 32]> {
    if bytes.len() != 32 {
        return invalid(ty, format!("expected 32 bytes, got {}", bytes.len()));
    }
    let mut out = [0u8; 32];
    out.copy_from_slice(bytes);
    Ok(out)
}

pub(crate) fn encode_table(fields: &[Vec<u8>]) -> Vec<u8> {
    let header_size = NUMBER_SIZE + fields.len() * NUMBER_SIZE;
    let total_size = header_size + fields.iter().map(Vec::len).sum::<usize>();
    let mut out = Vec::with_capacity(total_size);
    out.extend_from_slice(&(total_size as u32).to_le_bytes());

    let mut offset = header_size;
    for field in fields {
        out.extend_from_slice(&(offset as u32).to_le_bytes());
        offset += field.len();
    }
    for field in fields {
        out.extend_from_slice(field);
    }
    out
}

pub(crate) fn decode_table<'a>(bytes: &'a [u8], expected_fields: usize, ty: &'static str) -> Result<Vec<&'a [u8]>> {
    let min_size = NUMBER_SIZE + expected_fields * NUMBER_SIZE;
    if bytes.len() < min_size {
        return invalid(ty, format!("too short for table header: {}", bytes.len()));
    }

    let total_size = unpack_number(bytes, ty)?;
    if total_size != bytes.len() {
        return invalid(ty, format!("declared size {} != actual {}", total_size, bytes.len()));
    }

    let first_offset = unpack_number(&bytes[NUMBER_SIZE..NUMBER_SIZE * 2], ty)?;
    if first_offset != min_size {
        return invalid(ty, format!("expected first offset {}, got {}", min_size, first_offset));
    }

    let mut offsets = Vec::with_capacity(expected_fields + 1);
    for chunk in bytes[NUMBER_SIZE..min_size].chunks_exact(NUMBER_SIZE) {
        offsets.push(unpack_number(chunk, ty)?);
    }
    offsets.push(total_size);

    let mut fields = Vec::with_capacity(expected_fields);
    for pair in offsets.windows(2) {
        let start = pair[0];
        let end = pair[1];
        if start > end || end > bytes.len() {
            return invalid(ty, "invalid table field offsets");
        }
        fields.push(&bytes[start..end]);
    }
    Ok(fields)
}

pub(crate) fn encode_dynvec(items: &[Vec<u8>]) -> Vec<u8> {
    if items.is_empty() {
        return (NUMBER_SIZE as u32).to_le_bytes().to_vec();
    }

    let header_size = NUMBER_SIZE + items.len() * NUMBER_SIZE;
    let total_size = header_size + items.iter().map(Vec::len).sum::<usize>();
    let mut out = Vec::with_capacity(total_size);
    out.extend_from_slice(&(total_size as u32).to_le_bytes());

    let mut offset = header_size;
    for item in items {
        out.extend_from_slice(&(offset as u32).to_le_bytes());
        offset += item.len();
    }
    for item in items {
        out.extend_from_slice(item);
    }
    out
}

pub(crate) fn decode_dynvec<'a>(bytes: &'a [u8], ty: &'static str) -> Result<Vec<&'a [u8]>> {
    if bytes.len() < NUMBER_SIZE {
        return invalid(ty, format!("too short for dynvec header: {}", bytes.len()));
    }

    let total_size = unpack_number(bytes, ty)?;
    if total_size != bytes.len() {
        return invalid(ty, format!("declared size {} != actual {}", total_size, bytes.len()));
    }
    if total_size == NUMBER_SIZE {
        return Ok(Vec::new());
    }

    let first_offset = unpack_number(&bytes[NUMBER_SIZE..NUMBER_SIZE * 2], ty)?;
    if first_offset < NUMBER_SIZE || first_offset > total_size || first_offset % NUMBER_SIZE != 0 {
        return invalid(ty, format!("invalid first offset {}", first_offset));
    }
    let item_count = first_offset / NUMBER_SIZE - 1;

    let mut offsets = Vec::with_capacity(item_count + 1);
    let header_end = NUMBER_SIZE + item_count * NUMBER_SIZE;
    for chunk in bytes[NUMBER_SIZE..header_end].chunks_exact(NUMBER_SIZE) {
        offsets.push(unpack_number(chunk, ty)?);
    }
    offsets.push(total_size);

    let mut items = Vec::with_capacity(item_count);
    for pair in offsets.windows(2) {
        let start = pair[0];
        let end = pair[1];
        if start > end || end > bytes.len() {
            return invalid(ty, "invalid dynvec item offsets");
        }
        items.push(&bytes[start..end]);
    }
    Ok(items)
}

fn unpack_number(bytes: &[u8], ty: &'static str) -> Result<usize> {
    if bytes.len() < NUMBER_SIZE {
        return invalid(ty, "short Molecule number");
    }
    Ok(u32::from_le_bytes([bytes[0], bytes[1], bytes[2], bytes[3]]) as usize)
}

fn invalid<T>(ty: &'static str, reason: impl Into<String>) -> Result<T> {
    Err(StateError::Serialization(format!("invalid Molecule bytes for {ty}: {}", reason.into())))
}
