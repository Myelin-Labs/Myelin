// SPDX-License-Identifier: MIT
// Copyright (C) 2026 Myelin developers
//
// Serialization Security
//
//! # 序列化安全
//
//! 本模块提供序列化数据的安全性功能，包括完整性校验和防篡改保护。
//
//! ## 功能
//
//! - 序列化数据的哈希校验
//! - 完整性验证
//! - 防重放保护（可选）
//! - 大小限制和深度限制

use crate::serialization::{utils, SerializationError, VersionedSerializable};
use blake3::Hasher;

/// 安全序列化配置
#[derive(Clone, Debug)]
pub struct SecurityConfig {
    /// 启用完整性校验
    pub enable_integrity_check: bool,
    /// 启用大小限制
    pub enable_size_limit: bool,
    /// 最大序列化大小 (字节)
    pub max_size: usize,
    /// 启用深度限制（用于嵌套结构）
    pub enable_depth_limit: bool,
    /// 最大嵌套深度
    pub max_depth: usize,
}

impl Default for SecurityConfig {
    fn default() -> Self {
        Self {
            enable_integrity_check: true,
            enable_size_limit: true,
            max_size: 100 * 1024 * 1024, // 100MB
            enable_depth_limit: true,
            max_depth: 100,
        }
    }
}

impl SecurityConfig {
    /// 创建最小安全配置（仅基本保护）
    pub fn minimal() -> Self {
        Self {
            enable_integrity_check: false,
            enable_size_limit: true,
            max_size: 1024 * 1024 * 1024, // 1GB
            enable_depth_limit: false,
            max_depth: 1000,
        }
    }

    /// 创建严格安全配置
    pub fn strict() -> Self {
        Self {
            enable_integrity_check: true,
            enable_size_limit: true,
            max_size: 10 * 1024 * 1024, // 10MB
            enable_depth_limit: true,
            max_depth: 50,
        }
    }

    /// 禁用所有安全检查（仅用于测试）
    pub fn none() -> Self {
        Self {
            enable_integrity_check: false,
            enable_size_limit: false,
            max_size: usize::MAX,
            enable_depth_limit: false,
            max_depth: usize::MAX,
        }
    }
}

/// 带完整性校验的序列化数据
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct SecureEnvelope {
    /// 原始数据
    pub data: Vec<u8>,
    /// BLAKE3 哈希校验
    pub hash: [u8; 32],
    /// 数据长度（用于快速验证）
    pub length: u32,
}

impl SecureEnvelope {
    /// 创建新的安全信封
    pub fn new(data: Vec<u8>) -> Self {
        let hash = Self::compute_hash(&data);
        let length = data.len() as u32;
        Self { data, hash, length }
    }

    /// 计算数据的 BLAKE3 哈希
    fn compute_hash(data: &[u8]) -> [u8; 32] {
        let mut hasher = Hasher::new();
        hasher.update(data);
        hasher.finalize().into()
    }

    /// 验证数据完整性
    pub fn verify(&self) -> bool {
        if self.data.len() != self.length as usize {
            return false;
        }
        let computed_hash = Self::compute_hash(&self.data);
        computed_hash == self.hash
    }

    /// 序列化为字节
    pub fn to_bytes(&self) -> Vec<u8> {
        encode_table(&[self.length.to_le_bytes().to_vec(), self.hash.to_vec(), self.data.clone()])
    }

    /// 从字节解析
    pub fn from_bytes(bytes: &[u8]) -> Result<Self, SerializationError> {
        let (length, hash, data) = match decode_table(bytes, 3) {
            Ok(fields) => {
                let length = decode_u32(fields[0], "SecureEnvelope.length")? as usize;
                let hash = decode_array_32(fields[1], "SecureEnvelope.hash")?;
                let data = fields[2].to_vec();
                (length, hash, data)
            }
            Err(_) => decode_legacy_bytes(bytes)?,
        };

        if data.len() != length {
            return Err(SerializationError::DeserializationFailed(format!(
                "SecureEnvelope length mismatch: expected {length}, got {}",
                data.len()
            )));
        }

        let envelope = Self { data, hash, length: length as u32 };

        if !envelope.verify() {
            return Err(SerializationError::DeserializationFailed("Integrity check failed".to_string()));
        }

        Ok(envelope)
    }
}

/// 序列化安全守卫
///
/// 用于在序列化/反序列化过程中执行安全检查。
pub struct SecurityGuard {
    config: SecurityConfig,
    current_depth: usize,
}

impl SecurityGuard {
    /// 创建新的安全守卫
    pub fn new(config: SecurityConfig) -> Self {
        Self { config, current_depth: 0 }
    }

    /// 使用默认配置创建
    pub fn default() -> Self {
        Self::new(SecurityConfig::default())
    }

    /// 检查大小限制
    pub fn check_size(&self, size: usize) -> Result<(), SerializationError> {
        if self.config.enable_size_limit && size > self.config.max_size {
            return Err(SerializationError::DeserializationFailed(format!("Size {} exceeds maximum {}", size, self.config.max_size)));
        }
        Ok(())
    }

    /// 进入嵌套层级
    pub fn enter_nested(&mut self) -> Result<(), SerializationError> {
        let next_depth = self.current_depth.saturating_add(1);
        if self.config.enable_depth_limit && next_depth > self.config.max_depth {
            return Err(SerializationError::DeserializationFailed(format!(
                "Maximum nesting depth {} exceeded",
                self.config.max_depth
            )));
        }
        self.current_depth = next_depth;
        Ok(())
    }

    /// 退出嵌套层级
    pub fn exit_nested(&mut self) {
        if self.current_depth > 0 {
            self.current_depth -= 1;
        }
    }

    /// 获取当前深度
    pub fn current_depth(&self) -> usize {
        self.current_depth
    }

    /// 创建安全信封
    pub fn seal(&self, data: Vec<u8>) -> Result<SecureEnvelope, SerializationError> {
        if self.config.enable_integrity_check {
            self.check_size(data.len())?;
        }
        Ok(SecureEnvelope::new(data))
    }

    /// 验证并打开安全信封
    pub fn unseal(&self, envelope: &SecureEnvelope) -> Result<Vec<u8>, SerializationError> {
        if self.config.enable_integrity_check && !envelope.verify() {
            return Err(SerializationError::DeserializationFailed("Integrity check failed".to_string()));
        }
        self.check_size(envelope.data.len())?;
        Ok(envelope.data.clone())
    }
}

impl Default for SecurityGuard {
    fn default() -> Self {
        Self::default()
    }
}

/// 计算序列化数据的 BLAKE3 哈希
pub fn compute_hash(data: &[u8]) -> [u8; 32] {
    let mut hasher = Hasher::new();
    hasher.update(data);
    hasher.finalize().into()
}

/// 验证数据完整性
pub fn verify_integrity(data: &[u8], expected_hash: &[u8; 32]) -> bool {
    let computed_hash = compute_hash(data);
    computed_hash == *expected_hash
}

/// 带完整性校验的序列化
pub fn serialize_with_integrity<T: VersionedSerializable>(value: &T) -> Result<SecureEnvelope, SerializationError> {
    let data = utils::serialize_to_bytes(value)?;
    Ok(SecureEnvelope::new(data))
}

/// 带完整性校验的反序列化
pub fn deserialize_with_integrity<T: VersionedSerializable>(envelope: &SecureEnvelope) -> Result<T, SerializationError> {
    if !envelope.verify() {
        return Err(SerializationError::DeserializationFailed("Integrity check failed".to_string()));
    }
    utils::deserialize_from_bytes(&envelope.data)
}

fn encode_table(fields: &[Vec<u8>]) -> Vec<u8> {
    let header_size = 4 + fields.len() * 4;
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

fn decode_table<'a>(bytes: &'a [u8], expected_fields: usize) -> Result<Vec<&'a [u8]>, SerializationError> {
    if bytes.len() < 8 {
        return Err(SerializationError::DeserializationFailed("Molecule table too short".to_string()));
    }
    let total_size = u32::from_le_bytes(bytes[..4].try_into().expect("slice length checked")) as usize;
    if total_size != bytes.len() {
        return Err(SerializationError::DeserializationFailed("Molecule table total size mismatch".to_string()));
    }
    let first_offset = u32::from_le_bytes(bytes[4..8].try_into().expect("slice length checked")) as usize;
    if first_offset < 4 || first_offset % 4 != 0 {
        return Err(SerializationError::DeserializationFailed("invalid Molecule table first offset".to_string()));
    }
    let field_count = (first_offset - 4) / 4;
    if field_count != expected_fields {
        return Err(SerializationError::DeserializationFailed(format!(
            "Molecule table field count mismatch: expected {expected_fields}, got {field_count}"
        )));
    }
    let mut offsets = Vec::with_capacity(field_count + 1);
    for index in 0..field_count {
        let start = 4 + index * 4;
        let offset = u32::from_le_bytes(bytes[start..start + 4].try_into().expect("slice length checked")) as usize;
        if offset < first_offset || offset > bytes.len() {
            return Err(SerializationError::DeserializationFailed("Molecule table offset out of range".to_string()));
        }
        if let Some(previous) = offsets.last() {
            if offset < *previous {
                return Err(SerializationError::DeserializationFailed("Molecule table offsets are not ordered".to_string()));
            }
        }
        offsets.push(offset);
    }
    offsets.push(bytes.len());
    Ok(offsets.windows(2).map(|range| &bytes[range[0]..range[1]]).collect())
}

fn decode_u32(bytes: &[u8], field: &'static str) -> Result<u32, SerializationError> {
    if bytes.len() != 4 {
        return Err(SerializationError::DeserializationFailed(format!("{field} must be u32le")));
    }
    Ok(u32::from_le_bytes(bytes.try_into().expect("slice length checked")))
}

fn decode_array_32(bytes: &[u8], field: &'static str) -> Result<[u8; 32], SerializationError> {
    bytes.try_into().map_err(|_| SerializationError::DeserializationFailed(format!("{field} must be 32 bytes")))
}

fn decode_legacy_bytes(bytes: &[u8]) -> Result<(usize, [u8; 32], Vec<u8>), SerializationError> {
    if bytes.len() < 36 {
        return Err(SerializationError::DeserializationFailed("Insufficient bytes for legacy SecureEnvelope".to_string()));
    }
    let length = u32::from_le_bytes(bytes[..4].try_into().expect("slice length checked")) as usize;
    if bytes.len() != 36 + length {
        return Err(SerializationError::DeserializationFailed(format!("Expected {} bytes, got {}", 36 + length, bytes.len())));
    }
    let hash = decode_array_32(&bytes[4..36], "legacy SecureEnvelope.hash")?;
    Ok((length, hash, bytes[36..].to_vec()))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[derive(Clone, Debug, PartialEq, Eq)]
    struct TestData {
        value: u64,
        data: Vec<u8>,
    }

    impl VersionedSerializable for TestData {
        const CURRENT_VERSION: u8 = 1;

        fn to_versioned_payload(&self) -> Result<Vec<u8>, SerializationError> {
            let mut out = Vec::with_capacity(12 + self.data.len());
            out.extend_from_slice(&self.value.to_le_bytes());
            out.extend_from_slice(&(self.data.len() as u32).to_le_bytes());
            out.extend_from_slice(&self.data);
            Ok(out)
        }

        fn upgrade_from(version: u8, bytes: &[u8]) -> Result<Self, SerializationError> {
            if version != Self::CURRENT_VERSION {
                return Err(SerializationError::UpgradePathNotAvailable { from: version, to: Self::CURRENT_VERSION });
            }
            if bytes.len() < 12 {
                return Err(SerializationError::DeserializationFailed("TestData payload too short".to_string()));
            }
            let value = u64::from_le_bytes(bytes[..8].try_into().expect("slice length checked"));
            let data_len = u32::from_le_bytes(bytes[8..12].try_into().expect("slice length checked")) as usize;
            if bytes.len() != 12 + data_len {
                return Err(SerializationError::DeserializationFailed("TestData payload length mismatch".to_string()));
            }
            Ok(Self { value, data: bytes[12..].to_vec() })
        }
    }

    #[test]
    fn test_secure_envelope_basic() {
        let data = vec![0x01, 0x02, 0x03, 0x04];
        let envelope = SecureEnvelope::new(data.clone());

        assert_eq!(envelope.data, data);
        assert_eq!(envelope.length, 4);
        assert!(envelope.verify());
    }

    #[test]
    fn test_secure_envelope_serialization() {
        let data = vec![0x01, 0x02, 0x03, 0x04];
        let envelope = SecureEnvelope::new(data);

        let bytes = envelope.to_bytes();
        let restored = SecureEnvelope::from_bytes(&bytes).unwrap();

        assert_eq!(envelope, restored);
    }

    #[test]
    fn test_secure_envelope_tamper_detection() {
        let data = vec![0x01, 0x02, 0x03, 0x04];
        let mut envelope = SecureEnvelope::new(data);

        // Tamper with data
        envelope.data[0] = 0xFF;

        assert!(!envelope.verify());
    }

    #[test]
    fn test_secure_envelope_invalid_hash() {
        let data = vec![0x01, 0x02, 0x03, 0x04];
        let mut envelope = SecureEnvelope::new(data);

        // Corrupt hash
        envelope.hash[0] = 0xFF;

        assert!(!envelope.verify());
    }

    #[test]
    fn test_secure_envelope_from_bytes_corrupted() {
        let data = vec![0x01, 0x02, 0x03, 0x04];
        let envelope = SecureEnvelope::new(data);
        let mut bytes = envelope.to_bytes();

        // Corrupt the data portion
        bytes[39] = 0xFF;

        let result = SecureEnvelope::from_bytes(&bytes);
        assert!(result.is_err());
    }

    #[test]
    fn test_security_guard_size_check() {
        let config = SecurityConfig::strict();
        let guard = SecurityGuard::new(config);

        // Should pass for small size
        assert!(guard.check_size(100).is_ok());

        // Should fail for large size
        assert!(guard.check_size(20 * 1024 * 1024).is_err());
    }

    #[test]
    fn test_security_guard_depth_check() {
        let config = SecurityConfig::strict();
        let mut guard = SecurityGuard::new(config);

        // Enter nested levels
        for _ in 0..50 {
            assert!(guard.enter_nested().is_ok());
        }

        // Should fail at depth 51
        assert!(guard.enter_nested().is_err());

        // Exit and re-enter should work
        guard.exit_nested();
        assert!(guard.enter_nested().is_ok());
    }

    #[test]
    fn test_security_guard_seal_unseal() {
        let config = SecurityConfig::default();
        let guard = SecurityGuard::new(config);

        let data = vec![0x01, 0x02, 0x03];
        let envelope = guard.seal(data.clone()).unwrap();
        let unsealed = guard.unseal(&envelope).unwrap();

        assert_eq!(data, unsealed);
    }

    #[test]
    fn test_compute_hash() {
        let data1 = vec![0x01, 0x02, 0x03];
        let data2 = vec![0x01, 0x02, 0x03];
        let data3 = vec![0x01, 0x02, 0x04];

        let hash1 = compute_hash(&data1);
        let hash2 = compute_hash(&data2);
        let hash3 = compute_hash(&data3);

        assert_eq!(hash1, hash2);
        assert_ne!(hash1, hash3);
    }

    #[test]
    fn test_verify_integrity() {
        let data = vec![0x01, 0x02, 0x03];
        let hash = compute_hash(&data);

        assert!(verify_integrity(&data, &hash));

        let wrong_hash = [0u8; 32];
        assert!(!verify_integrity(&data, &wrong_hash));
    }

    #[test]
    fn test_serialize_with_integrity() {
        let data = TestData { value: 42, data: vec![0x01, 0x02, 0x03] };

        let envelope = serialize_with_integrity(&data).unwrap();
        assert!(envelope.verify());

        let restored: TestData = deserialize_with_integrity(&envelope).unwrap();
        assert_eq!(data, restored);
    }

    #[test]
    fn test_deserialize_with_integrity_corrupted() {
        let data = TestData { value: 42, data: vec![0x01, 0x02, 0x03] };

        let mut envelope = serialize_with_integrity(&data).unwrap();
        envelope.data[0] = 0xFF; // Corrupt data

        let result: Result<TestData, _> = deserialize_with_integrity(&envelope);
        assert!(result.is_err());
    }

    #[test]
    fn test_security_config_variants() {
        let minimal = SecurityConfig::minimal();
        assert!(!minimal.enable_integrity_check);
        assert!(minimal.enable_size_limit);

        let strict = SecurityConfig::strict();
        assert!(strict.enable_integrity_check);
        assert!(strict.enable_size_limit);
        assert!(strict.enable_depth_limit);

        let none = SecurityConfig::none();
        assert!(!none.enable_integrity_check);
        assert!(!none.enable_size_limit);
        assert!(!none.enable_depth_limit);
    }

    #[test]
    fn test_secure_envelope_empty_data() {
        let data: Vec<u8> = vec![];
        let envelope = SecureEnvelope::new(data);

        assert!(envelope.verify());
        let bytes = envelope.to_bytes();
        let restored = SecureEnvelope::from_bytes(&bytes).unwrap();
        assert_eq!(envelope, restored);
    }

    #[test]
    fn test_secure_envelope_large_data() {
        let data: Vec<u8> = (0..10000).map(|i| (i % 256) as u8).collect();
        let envelope = SecureEnvelope::new(data);

        assert!(envelope.verify());
        let bytes = envelope.to_bytes();
        assert_eq!(bytes.len(), 16 + 4 + 32 + 10000);

        let restored = SecureEnvelope::from_bytes(&bytes).unwrap();
        assert_eq!(envelope, restored);
    }
}
