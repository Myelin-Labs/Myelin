// SPDX-License-Identifier: MIT
// Copyright (C) 2026 Myelin developers
//
// Myelin 执行层序列化架构
//
//! # Myelin 执行层序列化架构声明
//!
//! ## 重要保证
//!
//! 1. **Molecule-compatible bytes are the public/default protocol boundary**
//!    - 所有共识关键哈希 (block hash, txid, sighash) 使用 CKB/Molecule-shaped bytes 或显式流式哈希
//!    - Native Myelin execution does not carry a legacy serializer dependency or ABI path
//!
//! 2. **VM-facing ABI 必须经过显式格式边界**
//!    - Molecule (`0x8001`) 是 launch/public VM ABI
//!    - Non-Molecule VM object ABI versions are rejected
//!
//! 3. **VM ABI 是独立抽象层**
//!    - 通过 `VmSerializable` trait 抽象序列化实现
//!    - VM-facing 类型直接使用当前 Molecule 编码
//!
//! ## 分层责任
//!
//! - Layer 1 (共识): CKB/Molecule-shaped bytes 或显式流式哈希
//! - Layer 2 (存储): Molecule-compatible records and envelopes for active state/execution utilities
//! - Layer 3 (VM ABI): Molecule for public script-visible data

/// Molecule canonical VM ABI compatibility layer
pub mod molecule_compat;

/// 序列化工具函数
pub mod utils;

/// 序列化缓存
pub mod cache;

/// 流式序列化
pub mod streaming;

/// 序列化安全
pub mod security;

/// 序列化错误类型
#[derive(Debug, thiserror::Error, Clone, PartialEq, Eq)]
pub enum SerializationError {
    /// 不支持的版本
    #[error("unsupported version: {0}")]
    UnsupportedVersion(u8),
    /// 反序列化失败
    #[error("deserialization failed: {0}")]
    DeserializationFailed(String),
    /// IO 错误
    #[error("io error: {0}")]
    IoError(String),
}

impl From<std::io::Error> for SerializationError {
    fn from(e: std::io::Error) -> Self {
        SerializationError::IoError(e.to_string())
    }
}

/// 版本化序列化接口
///
/// 所有 VM-facing 和 storage-facing 类型必须实现此 trait，
/// 仅接受该类型的当前 schema，拒绝历史或未知版本。
pub trait VersionedSerializable: Sized {
    /// 当前版本号
    const CURRENT_VERSION: u8;

    /// Serialize this value as the payload of a versioned envelope.
    ///
    /// Implementations must use Molecule-compatible or otherwise explicitly
    /// documented bytes. There is intentionally no derive-based default codec.
    fn to_versioned_payload(&self) -> Result<Vec<u8>, SerializationError>;

    /// Decode a payload after the envelope has checked its exact schema version.
    fn from_versioned_payload(bytes: &[u8]) -> Result<Self, SerializationError>;
}

/// 版本化序列化信封
///
/// 所有存储到 RocksDB 的类型必须使用此包装器，
/// 用于显式校验当前序列化格式与 schema。
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct VersionedEnvelope<T> {
    /// 序列化格式版本
    ///
    /// - 0x80: Molecule envelope payloads
    pub format_version: u8,

    /// 数据 schema 版本
    pub schema_version: u8,

    /// 实际序列化数据
    pub payload: Vec<u8>,

    /// 类型标记 (编译时优化)
    _phantom: std::marker::PhantomData<T>,
}

impl<T> VersionedEnvelope<T> {
    /// Molecule-compatible envelope format version.
    pub const FORMAT_VERSION_MOLECULE: u8 = 0x80;

    /// 创建新的版本化信封 (使用当前版本).
    pub fn new(value: &T) -> Result<Self, SerializationError>
    where
        T: VersionedSerializable,
    {
        let payload = value.to_versioned_payload()?;
        Ok(Self {
            format_version: Self::FORMAT_VERSION_MOLECULE,
            schema_version: T::CURRENT_VERSION,
            payload,
            _phantom: std::marker::PhantomData,
        })
    }

    /// Encode this envelope as a Molecule-compatible table:
    ///
    /// ```text
    /// VersionedEnvelope = table {
    ///   format_version: byte,
    ///   schema_version: byte,
    ///   payload: bytes,
    /// }
    /// ```
    pub fn to_bytes(&self) -> Vec<u8> {
        encode_molecule_table(&[vec![self.format_version], vec![self.schema_version], self.payload.clone()])
    }

    /// Decode a versioned envelope from current Molecule-compatible bytes.
    pub fn from_bytes(bytes: &[u8]) -> Result<Self, SerializationError> {
        if let Ok(fields) = decode_molecule_table(bytes, 3) {
            let format_version = decode_single_byte(fields[0], "format_version")?;
            let schema_version = decode_single_byte(fields[1], "schema_version")?;
            return Ok(Self { format_version, schema_version, payload: fields[2].to_vec(), _phantom: std::marker::PhantomData });
        }

        Err(SerializationError::DeserializationFailed("invalid VersionedEnvelope bytes".to_string()))
    }

    /// Build an envelope from already-encoded parts.
    pub fn from_parts(format_version: u8, schema_version: u8, payload: Vec<u8>) -> Self {
        Self { format_version, schema_version, payload, _phantom: std::marker::PhantomData }
    }

    /// 解析信封内容
    pub fn parse(&self) -> Result<T, SerializationError>
    where
        T: VersionedSerializable,
    {
        if self.format_version != Self::FORMAT_VERSION_MOLECULE {
            return Err(SerializationError::UnsupportedVersion(self.format_version));
        }
        if self.schema_version != T::CURRENT_VERSION {
            return Err(SerializationError::UnsupportedVersion(self.schema_version));
        }
        T::from_versioned_payload(&self.payload)
    }

    /// 获取格式版本
    pub fn format_version(&self) -> u8 {
        self.format_version
    }

    /// 获取 schema 版本
    pub fn schema_version(&self) -> u8 {
        self.schema_version
    }

    /// 获取 payload 大小
    pub fn payload_size(&self) -> usize {
        self.payload.len()
    }
}

fn encode_molecule_table(fields: &[Vec<u8>]) -> Vec<u8> {
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

fn decode_molecule_table(bytes: &[u8], expected_fields: usize) -> Result<Vec<&[u8]>, SerializationError> {
    if bytes.len() < 4 {
        return Err(SerializationError::DeserializationFailed("Molecule table too short".to_string()));
    }

    let total_size = u32::from_le_bytes([bytes[0], bytes[1], bytes[2], bytes[3]]) as usize;
    if total_size != bytes.len() {
        return Err(SerializationError::DeserializationFailed(format!(
            "Molecule table size mismatch: header {total_size}, actual {}",
            bytes.len()
        )));
    }

    if bytes.len() < 8 {
        return Err(SerializationError::DeserializationFailed("Molecule table missing first offset".to_string()));
    }

    let first_offset = u32::from_le_bytes([bytes[4], bytes[5], bytes[6], bytes[7]]) as usize;
    if first_offset < 4 || !first_offset.is_multiple_of(4) {
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
        let offset = u32::from_le_bytes([bytes[start], bytes[start + 1], bytes[start + 2], bytes[start + 3]]) as usize;
        if offset < first_offset || offset > bytes.len() {
            return Err(SerializationError::DeserializationFailed("Molecule table field offset out of range".to_string()));
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

fn decode_single_byte(bytes: &[u8], field: &'static str) -> Result<u8, SerializationError> {
    if bytes.len() != 1 {
        return Err(SerializationError::DeserializationFailed(format!(
            "VersionedEnvelope {field} must be one byte, got {}",
            bytes.len()
        )));
    }
    Ok(bytes[0])
}

/// VM ABI 错误
#[derive(Debug, thiserror::Error, Clone, PartialEq, Eq)]
pub enum VmAbiError {
    /// 反序列化失败
    #[error("deserialization failed: {0}")]
    DeserializationFailed(String),
    /// 不支持的 ABI 版本
    #[error("unsupported ABI version: 0x{0:04x}")]
    UnsupportedAbiVersion(u16),
}

/// VM 可见数据的序列化抽象
///
/// 此 trait 隔离 VM ABI 与具体序列化实现，
/// 公共脚本可见 ABI 使用 Molecule。
pub trait VmSerializable: Sized {
    /// 序列化为 VM 可见字节
    fn to_vm_bytes(&self) -> Vec<u8>;

    /// 从 VM 可见字节解析
    fn from_vm_bytes(bytes: &[u8]) -> Result<Self, VmAbiError>;
}

/// The only supported VM object ABI: canonical Molecule.
pub const VM_ABI_VERSION: u16 = 0x8001;

/// Fixed trailer used by executable artifacts to declare the VM object ABI.
///
/// The VM loader must strip this trailer before parsing/loading the ELF payload. The code hash still
/// covers the complete artifact bytes, including the trailer.
pub const VM_ABI_TRAILER_MAGIC: &[u8; 8] = b"MYLNABI\0";
/// Length in bytes of the fixed VM ABI trailer.
pub const VM_ABI_TRAILER_LEN: usize = 16;

/// Append the current VM ABI trailer, replacing an existing valid declaration.
pub fn append_vm_abi_trailer(mut artifact: Vec<u8>) -> Result<Vec<u8>, VmAbiError> {
    let payload_len = split_vm_abi_trailer(&artifact)?.len();
    artifact.truncate(payload_len);
    artifact.extend_from_slice(VM_ABI_TRAILER_MAGIC);
    artifact.extend_from_slice(&VM_ABI_VERSION.to_le_bytes());
    artifact.extend_from_slice(&[0; 6]);
    Ok(artifact)
}

/// Split an optional fixed VM ABI trailer from executable artifact bytes.
pub fn split_vm_abi_trailer(bytes: &[u8]) -> Result<&[u8], VmAbiError> {
    if bytes.len() < VM_ABI_TRAILER_LEN {
        return Ok(bytes);
    }

    let trailer_start = bytes.len() - VM_ABI_TRAILER_LEN;
    let trailer = &bytes[trailer_start..];
    if &trailer[..VM_ABI_TRAILER_MAGIC.len()] != VM_ABI_TRAILER_MAGIC {
        return Ok(bytes);
    }

    let version = u16::from_le_bytes([trailer[8], trailer[9]]);
    let flags = u16::from_le_bytes([trailer[10], trailer[11]]);
    let reserved = u32::from_le_bytes([trailer[12], trailer[13], trailer[14], trailer[15]]);
    if flags != 0 || reserved != 0 {
        return Err(VmAbiError::DeserializationFailed("VM ABI trailer flags/reserved bytes must be zero".to_string()));
    }

    if version != VM_ABI_VERSION {
        return Err(VmAbiError::UnsupportedAbiVersion(version));
    }
    Ok(&bytes[..trailer_start])
}

#[cfg(test)]
mod tests {
    use super::*;

    #[derive(Debug, Clone, PartialEq, Eq)]
    struct TestData {
        value: u64,
        name: String,
    }

    impl VersionedSerializable for TestData {
        const CURRENT_VERSION: u8 = 1;

        fn to_versioned_payload(&self) -> Result<Vec<u8>, SerializationError> {
            Ok(encode_test_data(self.value, &self.name))
        }

        fn from_versioned_payload(bytes: &[u8]) -> Result<Self, SerializationError> {
            let (value, name) = decode_test_data(bytes)?;
            Ok(Self { value, name })
        }
    }

    fn encode_test_data(value: u64, name: &str) -> Vec<u8> {
        let mut out = Vec::with_capacity(12 + name.len());
        out.extend_from_slice(&value.to_le_bytes());
        out.extend_from_slice(&(name.len() as u32).to_le_bytes());
        out.extend_from_slice(name.as_bytes());
        out
    }

    fn decode_test_data(bytes: &[u8]) -> Result<(u64, String), SerializationError> {
        if bytes.len() < 12 {
            return Err(SerializationError::DeserializationFailed("test payload too short".to_string()));
        }
        let value = u64::from_le_bytes(bytes[..8].try_into().expect("slice length checked"));
        let name_len = u32::from_le_bytes(bytes[8..12].try_into().expect("slice length checked")) as usize;
        if bytes.len() != 12 + name_len {
            return Err(SerializationError::DeserializationFailed("test payload length mismatch".to_string()));
        }
        let name =
            std::str::from_utf8(&bytes[12..]).map_err(|err| SerializationError::DeserializationFailed(err.to_string()))?.to_string();
        Ok((value, name))
    }

    #[test]
    fn test_versioned_envelope_roundtrip() {
        let data = TestData { value: 42, name: "test".to_string() };

        let envelope = VersionedEnvelope::new(&data).unwrap();
        assert_eq!(envelope.format_version, VersionedEnvelope::<TestData>::FORMAT_VERSION_MOLECULE);
        assert_eq!(envelope.schema_version, TestData::CURRENT_VERSION);

        let encoded = envelope.to_bytes();
        let decoded = VersionedEnvelope::<TestData>::from_bytes(&encoded).unwrap();
        assert_eq!(decoded.parse().unwrap(), data);
        assert_eq!(decoded.to_bytes(), encoded);

        let empty = VersionedEnvelope::<TestData>::from_parts(0x80, 1, vec![]);
        let decoded = VersionedEnvelope::<TestData>::from_bytes(&empty.to_bytes()).unwrap();
        assert!(decoded.payload.is_empty());
        assert!(decoded.parse().is_err());
    }

    #[test]
    fn test_versioned_envelope_unsupported_format() {
        let envelope = VersionedEnvelope::<TestData> {
            format_version: 0x7F,
            schema_version: 1,
            payload: vec![1, 2, 3],
            _phantom: std::marker::PhantomData,
        };

        let result = envelope.parse();
        assert!(matches!(result, Err(SerializationError::UnsupportedVersion(0x7F))));
    }

    #[test]
    fn test_versioned_envelope_rejects_old_schema() {
        let envelope = VersionedEnvelope::<TestData> {
            format_version: VersionedEnvelope::<TestData>::FORMAT_VERSION_MOLECULE,
            schema_version: 0, // Old version
            payload: encode_test_data(42, "test"),
            _phantom: std::marker::PhantomData,
        };

        let result = envelope.parse();
        assert!(matches!(result, Err(SerializationError::UnsupportedVersion(0))));
    }

    #[test]
    fn artifact_trailer_accepts_only_the_current_abi() {
        for artifact in [b"".as_slice(), b"\x7fELFplain", b"\x7fELFplain with longer body"] {
            assert_eq!(split_vm_abi_trailer(artifact).unwrap(), artifact);
            let tagged = append_vm_abi_trailer(artifact.to_vec()).unwrap();
            assert_eq!(split_vm_abi_trailer(&tagged).unwrap(), artifact);
            assert_eq!(append_vm_abi_trailer(tagged.clone()).unwrap(), tagged);
            for index in [8, 10, 12] {
                let mut invalid = tagged.clone();
                invalid[artifact.len() + index] ^= 1;
                assert!(split_vm_abi_trailer(&invalid).is_err());
                assert!(append_vm_abi_trailer(invalid).is_err());
            }
        }
    }
}
