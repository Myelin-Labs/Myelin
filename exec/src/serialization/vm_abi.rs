// SPDX-License-Identifier: MIT
// Copyright (C) 2026 Myelin developers
//
// VM ABI 序列化实现
//
//! VM ABI 序列化辅助函数
//!
//! 本模块提供 VM-facing 类型的稳定序列化格式。
//! 这些格式是 VM ABI 的一部分，变更需要协调合约升级。

use crate::celltx::{CellInput, CellOutput, OutPoint, Script};

/// Script 的 VM ABI 序列化格式
///
/// 格式: code_hash (32 bytes) || hash_type (1 byte) || args_len (4 bytes LE) || args
pub fn serialize_script(script: &Script) -> Vec<u8> {
    let mut data = Vec::with_capacity(37 + script.args.len());
    data.extend_from_slice(&script.code_hash);
    data.push(script.hash_type);
    data.extend_from_slice(&(script.args.len() as u32).to_le_bytes());
    data.extend_from_slice(&script.args);
    data
}

/// CellInput 的 VM ABI 序列化格式
///
/// 格式: tx_hash (32 bytes) || index (4 bytes LE) || since (8 bytes LE)
pub fn serialize_cell_input(input: &CellInput) -> Vec<u8> {
    let mut data = Vec::with_capacity(44);
    data.extend_from_slice(&input.previous_output.tx_hash);
    data.extend_from_slice(&input.previous_output.index.to_le_bytes());
    data.extend_from_slice(&input.since.to_le_bytes());
    data
}

/// OutPoint 的 VM ABI 序列化格式
///
/// 格式: tx_hash (32 bytes) || index (4 bytes LE)
pub fn serialize_outpoint(outpoint: &OutPoint) -> Vec<u8> {
    let mut data = Vec::with_capacity(36);
    data.extend_from_slice(&outpoint.tx_hash);
    data.extend_from_slice(&outpoint.index.to_le_bytes());
    data
}

/// CellOutput 的 VM ABI 序列化格式（不包含 data）
///
/// 格式: capacity (8 bytes LE) || lock_script || has_type (1 byte) || [type_script]
/// lock_script: code_hash (32) || hash_type (1) || args_len (4) || args
/// type_script: 同上（如果 has_type == 1）
pub fn serialize_cell_output(output: &CellOutput) -> Vec<u8> {
    let mut data = Vec::new();

    // capacity
    data.extend_from_slice(&output.capacity.to_le_bytes());

    // lock script
    let lock = serialize_script(&output.lock);
    data.extend_from_slice(&(lock.len() as u32).to_le_bytes());
    data.extend_from_slice(&lock);

    // type script (optional)
    match output.type_.as_ref() {
        Some(type_script) => {
            data.push(1);
            let type_bytes = serialize_script(type_script);
            data.extend_from_slice(&(type_bytes.len() as u32).to_le_bytes());
            data.extend_from_slice(&type_bytes);
        }
        None => data.push(0),
    }

    data
}

/// 计算序列化 Script 的大小（不含长度前缀）
pub fn serialized_script_size(script: &Script) -> usize {
    32 + 1 + 4 + script.args.len()
}

/// 计算序列化 CellOutput 的大小
pub fn serialized_cell_output_size(output: &CellOutput) -> usize {
    let lock_size = serialized_script_size(&output.lock);
    let type_size = output.type_.as_ref().map(|t| serialized_script_size(t)).unwrap_or(0);
    8 + 4 + lock_size + 1 + if output.type_.is_some() { 4 + type_size } else { 0 }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_serialize_script() {
        let script = Script::new([0xAA; 32], 1, vec![0x10, 0x20, 0x30]);
        let bytes = serialize_script(&script);

        assert_eq!(bytes.len(), 32 + 1 + 4 + 3); // code_hash + hash_type + args_len + args
        assert_eq!(&bytes[0..32], &[0xAA; 32]);
        assert_eq!(bytes[32], 1);
        assert_eq!(&bytes[33..37], &[3, 0, 0, 0]); // args_len in LE
        assert_eq!(&bytes[37..], &[0x10, 0x20, 0x30]);
    }

    #[test]
    fn test_serialize_cell_input() {
        let input = CellInput::new(OutPoint::new([0xBB; 32], 0x12345678), 0xABCDEF00);
        let bytes = serialize_cell_input(&input);

        assert_eq!(bytes.len(), 44);
        assert_eq!(&bytes[0..32], &[0xBB; 32]);
        assert_eq!(&bytes[32..36], &[0x78, 0x56, 0x34, 0x12]); // index in LE
        assert_eq!(&bytes[36..44], &[0x00, 0xEF, 0xCD, 0xAB, 0, 0, 0, 0]); // since in LE
    }

    #[test]
    fn test_serialize_outpoint() {
        let outpoint = OutPoint::new([0xCC; 32], 0xDEADBEEF);
        let bytes = serialize_outpoint(&outpoint);

        assert_eq!(bytes.len(), 36);
        assert_eq!(&bytes[0..32], &[0xCC; 32]);
        assert_eq!(&bytes[32..36], &[0xEF, 0xBE, 0xAD, 0xDE]); // index in LE
    }

    #[test]
    fn test_serialize_cell_output_with_type() {
        let lock = Script::new([0x11; 32], 0, vec![0xAA; 20]);
        let type_script = Script::new([0x22; 32], 1, vec![0xBB; 10]);
        let output = CellOutput { lock, type_: Some(type_script), capacity: 0x0102030405060708 };

        let bytes = serialize_cell_output(&output);

        // Check capacity
        assert_eq!(&bytes[0..8], &[0x08, 0x07, 0x06, 0x05, 0x04, 0x03, 0x02, 0x01]);

        // Check has_type flag
        let lock_len = u32::from_le_bytes([bytes[8], bytes[9], bytes[10], bytes[11]]) as usize;
        assert!(lock_len > 0);
        assert_eq!(bytes[8 + 4 + lock_len], 1); // has_type = 1
    }

    #[test]
    fn test_serialize_cell_output_without_type() {
        let lock = Script::new([0x11; 32], 0, vec![]);
        let output = CellOutput { lock, type_: None, capacity: 1000 };

        let bytes = serialize_cell_output(&output);

        // Check has_type flag
        let lock_len = u32::from_le_bytes([bytes[8], bytes[9], bytes[10], bytes[11]]) as usize;
        assert_eq!(bytes[8 + 4 + lock_len], 0); // has_type = 0
    }

    #[test]
    fn test_serialized_size_helpers() {
        let script = Script::new([0xAA; 32], 1, vec![0x10, 0x20, 0x30]);
        assert_eq!(serialized_script_size(&script), 32 + 1 + 4 + 3);

        let output = CellOutput { lock: script.clone(), type_: Some(script.clone()), capacity: 1000 };
        let expected_size = 8 + 4 + serialized_script_size(&script) + 1 + 4 + serialized_script_size(&script);
        assert_eq!(serialized_cell_output_size(&output), expected_size);
    }
}
