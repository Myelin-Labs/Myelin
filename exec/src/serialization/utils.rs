// SPDX-License-Identifier: MIT
// Copyright (C) 2026 Myelin developers
//
// Serialization Utilities
//
//! # 序列化工具函数
//!
//! 本模块提供实用的序列化辅助函数和宏。

use crate::serialization::{SerializationError, VersionedEnvelope, VersionedSerializable};

/// 将类型序列化为字节向量，包装在 VersionedEnvelope 中
///
/// # Example
/// ```
/// use myelin_exec::serialization::utils::serialize_to_bytes;
/// use myelin_exec::serialization::VersionedSerializable;
///
/// // Use any type that implements VersionedSerializable
/// let value = 42u8;
/// ```
pub fn serialize_to_bytes<T: VersionedSerializable>(value: &T) -> Result<Vec<u8>, SerializationError> {
    let envelope = VersionedEnvelope::new(value)?;
    Ok(envelope.to_bytes())
}

/// 从字节向量解析类型，自动处理 VersionedEnvelope
///
/// # Example
/// ```
/// use myelin_exec::serialization::utils::{serialize_to_bytes, deserialize_from_bytes};
/// use myelin_exec::serialization::VersionedSerializable;
///
/// // Roundtrip serialize/deserialize works for any VersionedSerializable type
/// let value = 42u8;
/// ```
pub fn deserialize_from_bytes<T: VersionedSerializable>(bytes: &[u8]) -> Result<T, SerializationError> {
    let envelope = VersionedEnvelope::<T>::from_bytes(bytes)?;
    envelope.parse()
}

/// 检查字节数据是否为有效的 VersionedEnvelope
///
/// 这个函数可以快速检查数据格式，而不需要完全解析。
pub fn is_valid_versioned_envelope(bytes: &[u8]) -> bool {
    VersionedEnvelope::<()>::from_bytes(bytes).is_ok()
}

/// 从 VersionedEnvelope 字节中提取 schema 版本
///
/// # Safety
/// 这个函数不会验证数据的完整性，仅用于快速检查版本。
/// 如果数据无效，可能返回错误的结果。
pub fn peek_schema_version(bytes: &[u8]) -> Option<u8> {
    VersionedEnvelope::<()>::from_bytes(bytes).ok().map(|envelope| envelope.schema_version)
}

/// 从 VersionedEnvelope 字节中提取格式版本
///
/// # Safety
/// 这个函数不会验证数据的完整性，仅用于快速检查版本。
pub fn peek_format_version(bytes: &[u8]) -> Option<u8> {
    VersionedEnvelope::<()>::from_bytes(bytes).ok().map(|envelope| envelope.format_version)
}

/// 序列化多个值到一个字节向量
///
/// 使用长度前缀编码，支持反序列化时逐个读取。
pub fn serialize_many<T: VersionedSerializable>(values: &[T]) -> Result<Vec<u8>, SerializationError> {
    // 检查 count 是否超过 u32::MAX
    let count = values.len();
    if count > u32::MAX as usize {
        return Err(SerializationError::DeserializationFailed(format!("Too many values: {} exceeds maximum {}", count, u32::MAX)));
    }

    let mut result = Vec::new();

    // Write count
    result.extend_from_slice(&(count as u32).to_le_bytes());

    // Write each value
    for value in values {
        let bytes = serialize_to_bytes(value)?;
        let len = bytes.len();

        // 检查单个值大小是否超过 u32::MAX
        if len > u32::MAX as usize {
            return Err(SerializationError::DeserializationFailed(format!(
                "Value too large: {} bytes exceeds maximum {}",
                len,
                u32::MAX
            )));
        }

        result.extend_from_slice(&(len as u32).to_le_bytes());
        result.extend_from_slice(&bytes);
    }

    Ok(result)
}

/// 从字节向量反序列化多个值
///
/// 与 `serialize_many` 配对使用。
pub fn deserialize_many<T: VersionedSerializable>(bytes: &[u8]) -> Result<Vec<T>, SerializationError> {
    if bytes.len() < 4 {
        return Err(SerializationError::DeserializationFailed("insufficient bytes for count".to_string()));
    }

    let count = u32::from_le_bytes([bytes[0], bytes[1], bytes[2], bytes[3]]) as usize;
    let mut result = Vec::with_capacity(count);
    let mut offset = 4;

    for _ in 0..count {
        if offset + 4 > bytes.len() {
            return Err(SerializationError::DeserializationFailed("insufficient bytes for length prefix".to_string()));
        }

        let len = u32::from_le_bytes([bytes[offset], bytes[offset + 1], bytes[offset + 2], bytes[offset + 3]]) as usize;
        offset += 4;

        if offset + len > bytes.len() {
            return Err(SerializationError::DeserializationFailed("insufficient bytes for value".to_string()));
        }

        let value = deserialize_from_bytes(&bytes[offset..offset + len])?;
        result.push(value);
        offset += len;
    }

    Ok(result)
}

/// 计算序列化后的大小的估计值
///
/// 这个函数返回一个估计值，实际大小可能略有不同。
pub fn estimate_serialized_size<T: VersionedSerializable>(value: &T) -> Result<usize, SerializationError> {
    let envelope = VersionedEnvelope::new(value)?;
    Ok(envelope.to_bytes().len())
}

/// 序列化结果类型别名
pub type SerializeResult<T> = Result<T, SerializationError>;

#[cfg(test)]
mod tests {
    use super::*;
    use crate::celltx::{CellOutput, Script};

    fn create_test_output() -> CellOutput {
        CellOutput { lock: Script::new([0xAA; 32], 0, vec![0xBB; 20]), type_: None, capacity: 1000 }
    }

    #[test]
    fn test_serialize_to_bytes_roundtrip() {
        let output = create_test_output();
        let bytes = serialize_to_bytes(&output).unwrap();
        let restored: CellOutput = deserialize_from_bytes(&bytes).unwrap();
        assert_eq!(output, restored);
    }

    #[test]
    fn test_is_valid_versioned_envelope() {
        let output = create_test_output();
        let bytes = serialize_to_bytes(&output).unwrap();
        assert!(is_valid_versioned_envelope(&bytes));

        // Invalid: too short
        assert!(!is_valid_versioned_envelope(&[]));
        assert!(!is_valid_versioned_envelope(&[0x00]));
        assert!(!is_valid_versioned_envelope(&[0x00, 0x01]));

        // Invalid: malformed Molecule table
        assert!(!is_valid_versioned_envelope(&[0x81, 0x01, 0x00]));
    }

    #[test]
    fn test_peek_schema_version() {
        let output = create_test_output();
        let bytes = serialize_to_bytes(&output).unwrap();

        let version = peek_schema_version(&bytes).unwrap();
        assert_eq!(version, CellOutput::CURRENT_VERSION);
    }

    #[test]
    fn test_peek_format_version() {
        let output = create_test_output();
        let bytes = serialize_to_bytes(&output).unwrap();

        let version = peek_format_version(&bytes).unwrap();
        assert_eq!(version, VersionedEnvelope::<CellOutput>::FORMAT_VERSION_MOLECULE);
    }

    #[test]
    fn test_serialize_many_roundtrip() {
        let outputs: Vec<CellOutput> = (0..5)
            .map(|i| CellOutput { lock: Script::new([i as u8; 32], 0, vec![i as u8; 20]), type_: None, capacity: 1000 + i as u64 })
            .collect();

        let bytes = serialize_many(&outputs).unwrap();
        let restored = deserialize_many::<CellOutput>(&bytes).unwrap();

        assert_eq!(outputs.len(), restored.len());
        for (orig, rest) in outputs.iter().zip(restored.iter()) {
            assert_eq!(orig, rest);
        }
    }

    #[test]
    fn test_serialize_many_empty() {
        let outputs: Vec<CellOutput> = vec![];
        let bytes = serialize_many(&outputs).unwrap();
        assert_eq!(bytes.len(), 4); // Just the count (0)

        let restored = deserialize_many::<CellOutput>(&bytes).unwrap();
        assert!(restored.is_empty());
    }

    #[test]
    fn test_estimate_serialized_size() {
        let output = create_test_output();
        let estimated = estimate_serialized_size(&output).unwrap();
        let actual = serialize_to_bytes(&output).unwrap().len();

        // Estimate should be close to actual
        let diff = estimated.abs_diff(actual);
        assert!(diff <= 10, "estimate {} should be close to actual {}", estimated, actual);
    }

    #[test]
    fn test_deserialize_many_invalid_data() {
        // Too short for count
        let result = deserialize_many::<CellOutput>(&[0x00]);
        assert!(result.is_err());

        // Invalid count (claims more items than present)
        let invalid = vec![
            0x02, 0x00, 0x00, 0x00, // count = 2
            0x05, 0x00, 0x00, 0x00, // len = 5
            0x01, 0x02, 0x03, 0x04,
        ]; // only 4 bytes
        let result = deserialize_many::<CellOutput>(&invalid);
        assert!(result.is_err());
    }

    #[test]
    fn test_peek_functions_on_empty() {
        assert_eq!(peek_schema_version(&[]), None);
        assert_eq!(peek_format_version(&[]), None);
        assert_eq!(peek_schema_version(&[0x00]), None); // Need at least 2 bytes
    }
}
