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
//!    - Molecule v1 (`0x8001`) 是 launch/public VM ABI
//!    - Non-Molecule VM object ABI versions are rejected
//!
//! 3. **VM ABI 是独立抽象层**
//!    - 通过 `VmSerializable` trait 抽象序列化实现
//!    - VM 层可以独立切到 Molecule，不影响其他层
//!
//! ## 分层责任
//!
//! - Layer 1 (共识): CKB/Molecule-shaped bytes 或显式流式哈希
//! - Layer 2 (存储): Molecule-compatible records and envelopes for active state/execution utilities
//! - Layer 3 (VM ABI): Molecule v1 for public script-visible data

/// VM ABI 序列化辅助函数
pub mod vm_abi;

/// Molecule canonical VM ABI compatibility layer
pub mod molecule_compat;

/// 序列化工具函数
pub mod utils;

/// 序列化缓存
pub mod cache;

/// 序列化宏
pub mod macros;

/// 序列化验证
pub mod validation;

/// 流式序列化
pub mod streaming;

/// 序列化安全
pub mod security;

/// 序列化压缩
pub mod compression;

/// 序列化错误类型
#[derive(Debug, thiserror::Error, Clone, PartialEq, Eq)]
pub enum SerializationError {
    /// 不支持的版本
    #[error("unsupported version: {0}")]
    UnsupportedVersion(u8),
    /// 反序列化失败
    #[error("deserialization failed: {0}")]
    DeserializationFailed(String),
    /// 升级路径不可用
    #[error("upgrade path not available: from {from} to {to}")]
    UpgradePathNotAvailable {
        /// Stored schema version.
        from: u8,
        /// Current schema version requested by the type.
        to: u8,
    },
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
/// 以确保 schema 演进时的向后兼容性。
pub trait VersionedSerializable: Sized {
    /// 当前版本号
    const CURRENT_VERSION: u8;

    /// 获取实例的版本号
    fn version(&self) -> u8 {
        Self::CURRENT_VERSION
    }

    /// Serialize this value as the payload of a versioned envelope.
    ///
    /// Implementations must use Molecule-compatible or otherwise explicitly
    /// documented bytes. There is intentionally no derive-based default codec.
    fn to_versioned_payload(&self) -> Result<Vec<u8>, SerializationError> {
        Err(SerializationError::DeserializationFailed("VersionedSerializable requires an explicit payload codec".to_string()))
    }

    /// 从指定版本的二进制数据升级解析
    ///
    /// # Arguments
    /// * `version` - 数据存储时的版本号
    /// * `bytes` - 原始二进制数据
    ///
    /// # Returns
    /// * `Ok(Self)` - 成功解析并升级
    /// * `Err(Error)` - 解析失败或不支持的版本
    fn upgrade_from(version: u8, bytes: &[u8]) -> Result<Self, SerializationError> {
        let _ = bytes;
        Err(SerializationError::UpgradePathNotAvailable { from: version, to: Self::CURRENT_VERSION })
    }
}

/// 版本化序列化信封
///
/// 所有存储到 RocksDB 的类型必须使用此包装器，
/// 以确保未来可以平滑迁移序列化格式。
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct VersionedEnvelope<T> {
    /// 序列化格式版本
    ///
    /// - 0x80-0xFF: Molecule-compatible envelope payloads
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
        match self.format_version {
            Self::FORMAT_VERSION_MOLECULE..=0xFF => T::upgrade_from(self.schema_version, &self.payload),
            _ => Err(SerializationError::UnsupportedVersion(self.format_version)),
        }
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

impl<T> Default for VersionedEnvelope<T> {
    fn default() -> Self {
        Self {
            format_version: VersionedEnvelope::<T>::FORMAT_VERSION_MOLECULE,
            schema_version: 0,
            payload: Vec::new(),
            _phantom: std::marker::PhantomData,
        }
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
    /// 序列化失败
    #[error("serialization failed: {0}")]
    SerializationFailed(String),
    /// 反序列化失败
    #[error("deserialization failed: {0}")]
    DeserializationFailed(String),
    /// ABI 版本不匹配
    #[error("ABI version mismatch: expected {expected}, got {actual}")]
    VersionMismatch {
        /// ABI version requested by the script.
        expected: u16,
        /// First VM-supported ABI version reported during negotiation.
        actual: u16,
    },
    /// 不支持的 ABI 版本
    #[error("unsupported ABI version: 0x{0:04x}")]
    UnsupportedAbiVersion(u16),
}

/// VM 可见数据的序列化抽象
///
/// 此 trait 隔离 VM ABI 与具体序列化实现，
/// 公共脚本可见 ABI 现在以 Molecule 为准；legacy 实现仅用于显式兼容路径。
pub trait VmSerializable: Sized {
    /// 序列化为 VM 可见字节
    fn to_vm_bytes(&self) -> Vec<u8>;

    /// 从 VM 可见字节解析
    fn from_vm_bytes(bytes: &[u8]) -> Result<Self, VmAbiError>;

    /// 获取 ABI 版本
    fn abi_version() -> u16;

    /// 检查 ABI 版本是否兼容
    fn is_abi_compatible(version: u16) -> bool {
        version == Self::abi_version()
    }
}

/// VM-visible ABI wire format.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum VmAbiFormat {
    /// Canonical Molecule VM ABI for launch/public script-visible data.
    #[default]
    Molecule,
}

impl VmAbiFormat {
    /// Return the negotiated ABI version for this wire format.
    pub const fn abi_version(self) -> u16 {
        match self {
            Self::Molecule => VmAbiNegotiator::ABI_VERSION_MOLECULE_V1,
        }
    }

    /// Resolve a VM ABI version into a wire format.
    pub fn from_abi_version(version: u16) -> Result<Self, VmAbiError> {
        match version {
            VmAbiNegotiator::ABI_VERSION_MOLECULE_V1 => Ok(Self::Molecule),
            other => Err(VmAbiError::UnsupportedAbiVersion(other)),
        }
    }
}

/// Fixed trailer used by executable artifacts to declare the VM object ABI.
///
/// The VM loader must strip this trailer before parsing/loading the ELF payload. The code hash still
/// covers the complete artifact bytes, including the trailer.
pub const VM_ABI_TRAILER_MAGIC: &[u8; 8] = b"MYLNABI\0";
/// Length in bytes of the fixed VM ABI trailer.
pub const VM_ABI_TRAILER_LEN: usize = 16;

/// Append or replace a fixed VM ABI trailer.
pub fn append_vm_abi_trailer(mut artifact: Vec<u8>, abi_format: VmAbiFormat) -> Vec<u8> {
    if split_vm_abi_trailer(&artifact).ok().and_then(|(_, format)| format).is_some() {
        artifact.truncate(artifact.len() - VM_ABI_TRAILER_LEN);
    }
    artifact.extend_from_slice(VM_ABI_TRAILER_MAGIC);
    artifact.extend_from_slice(&abi_format.abi_version().to_le_bytes());
    artifact.extend_from_slice(&0u16.to_le_bytes());
    artifact.extend_from_slice(&0u32.to_le_bytes());
    artifact
}

/// Split an optional fixed VM ABI trailer from executable artifact bytes.
pub fn split_vm_abi_trailer(bytes: &[u8]) -> Result<(&[u8], Option<VmAbiFormat>), VmAbiError> {
    if bytes.len() < VM_ABI_TRAILER_LEN {
        return Ok((bytes, None));
    }

    let trailer_start = bytes.len() - VM_ABI_TRAILER_LEN;
    let trailer = &bytes[trailer_start..];
    if &trailer[..VM_ABI_TRAILER_MAGIC.len()] != VM_ABI_TRAILER_MAGIC {
        return Ok((bytes, None));
    }

    let version = u16::from_le_bytes([trailer[8], trailer[9]]);
    let flags = u16::from_le_bytes([trailer[10], trailer[11]]);
    let reserved = u32::from_le_bytes([trailer[12], trailer[13], trailer[14], trailer[15]]);
    if flags != 0 || reserved != 0 {
        return Err(VmAbiError::DeserializationFailed("VM ABI trailer flags/reserved bytes must be zero".to_string()));
    }

    Ok((&bytes[..trailer_start], Some(VmAbiFormat::from_abi_version(version)?)))
}

/// VM ABI 版本协商器
pub struct VmAbiNegotiator;

impl VmAbiNegotiator {
    /// Molecule-based ABI v1 版本号，launch/public VM ABI。
    pub const ABI_VERSION_MOLECULE_V1: u16 = 0x8001;

    /// 协商脚本和 VM 之间的 ABI 版本
    ///
    /// # Arguments
    /// * `script_version` - 脚本要求的 ABI 版本
    /// * `vm_capabilities` - VM 支持的 ABI 版本列表
    ///
    /// # Returns
    /// * `Ok(u16)` - 协商成功的 ABI 版本
    /// * `Err(VmAbiError)` - 协商失败
    pub fn negotiate(script_version: u16, vm_capabilities: &[u16]) -> Result<u16, VmAbiError> {
        if script_version != Self::ABI_VERSION_MOLECULE_V1 {
            return Err(VmAbiError::UnsupportedAbiVersion(script_version));
        }

        // 优先使用脚本要求的版本
        for cap in vm_capabilities {
            if *cap == script_version {
                return Ok(*cap);
            }
        }

        Err(VmAbiError::VersionMismatch { expected: script_version, actual: vm_capabilities.first().copied().unwrap_or(0) })
    }

    /// 获取 VM 默认支持的 ABI 版本列表.
    ///
    /// Public/default negotiation advertises only Molecule.
    pub fn default_capabilities() -> Vec<u16> {
        vec![Self::ABI_VERSION_MOLECULE_V1]
    }
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

        fn upgrade_from(version: u8, bytes: &[u8]) -> Result<Self, SerializationError> {
            if version != Self::CURRENT_VERSION {
                return Err(SerializationError::UpgradePathNotAvailable { from: version, to: Self::CURRENT_VERSION });
            }
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

    impl VmSerializable for TestData {
        fn to_vm_bytes(&self) -> Vec<u8> {
            encode_test_data(self.value, &self.name)
        }

        fn from_vm_bytes(bytes: &[u8]) -> Result<Self, VmAbiError> {
            let (value, name) = decode_test_data(bytes).map_err(|err| VmAbiError::DeserializationFailed(err.to_string()))?;
            Ok(Self { value, name })
        }

        fn abi_version() -> u16 {
            Self::CURRENT_VERSION as u16
        }
    }

    #[test]
    fn test_versioned_envelope_roundtrip() {
        let data = TestData { value: 42, name: "test".to_string() };

        let envelope = VersionedEnvelope::new(&data).unwrap();
        assert_eq!(envelope.format_version, VersionedEnvelope::<TestData>::FORMAT_VERSION_MOLECULE);
        assert_eq!(envelope.schema_version, TestData::CURRENT_VERSION);

        let parsed: TestData = envelope.parse().unwrap();
        assert_eq!(parsed, data);
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
    fn test_version_upgrade_path_not_available() {
        let envelope = VersionedEnvelope::<TestData> {
            format_version: VersionedEnvelope::<TestData>::FORMAT_VERSION_MOLECULE,
            schema_version: 0, // Old version
            payload: encode_test_data(42, "test"),
            _phantom: std::marker::PhantomData,
        };

        let result = envelope.parse();
        assert!(matches!(result, Err(SerializationError::UpgradePathNotAvailable { from: 0, to: 1 })));
    }

    #[test]
    fn test_vm_abi_trailer_roundtrip() {
        let artifact = b"\x7fELFdemo".to_vec();
        let with_trailer = append_vm_abi_trailer(artifact.clone(), VmAbiFormat::Molecule);

        let (stripped, format) = split_vm_abi_trailer(&with_trailer).unwrap();

        assert_eq!(stripped, artifact.as_slice());
        assert_eq!(format, Some(VmAbiFormat::Molecule));
        assert_eq!(with_trailer.len(), artifact.len() + VM_ABI_TRAILER_LEN);
    }

    #[test]
    fn test_vm_abi_trailer_ignores_plain_artifacts() {
        let artifact = b"\x7fELFplain";

        let (stripped, format) = split_vm_abi_trailer(artifact).unwrap();

        assert_eq!(stripped, artifact);
        assert_eq!(format, None);
    }

    #[test]
    fn test_vm_abi_negotiation_success() {
        let caps = vec![VmAbiNegotiator::ABI_VERSION_MOLECULE_V1];
        let result = VmAbiNegotiator::negotiate(VmAbiNegotiator::ABI_VERSION_MOLECULE_V1, &caps);
        assert_eq!(result.unwrap(), VmAbiNegotiator::ABI_VERSION_MOLECULE_V1);
    }

    #[test]
    fn test_vm_abi_negotiation_rejects_non_molecule_capabilities() {
        let caps = vec![0x0001];
        let result = VmAbiNegotiator::negotiate(VmAbiNegotiator::ABI_VERSION_MOLECULE_V1, &caps);
        assert!(matches!(
            result,
            Err(VmAbiError::VersionMismatch { expected: VmAbiNegotiator::ABI_VERSION_MOLECULE_V1, actual: 0x0001 })
        ));
    }

    #[test]
    fn test_vm_abi_negotiation_failure() {
        let caps = vec![0x0002]; // 只支持 v2
        let result = VmAbiNegotiator::negotiate(0x0001, &caps);
        assert!(matches!(result, Err(VmAbiError::UnsupportedAbiVersion(0x0001))));
    }

    #[test]
    fn test_default_capabilities() {
        let caps = VmAbiNegotiator::default_capabilities();
        assert_eq!(caps, vec![VmAbiNegotiator::ABI_VERSION_MOLECULE_V1]);
        assert!(!caps.contains(&0x0001));
    }

    #[test]
    fn test_versioned_envelope_default() {
        let envelope: VersionedEnvelope<TestData> = VersionedEnvelope::default();
        assert_eq!(envelope.format_version, VersionedEnvelope::<TestData>::FORMAT_VERSION_MOLECULE);
        assert_eq!(envelope.schema_version, 0);
        assert!(envelope.payload.is_empty());
    }

    #[test]
    fn test_versioned_envelope_size_methods() {
        let data = TestData { value: 42, name: "test".to_string() };
        let envelope = VersionedEnvelope::new(&data).unwrap();

        assert_eq!(envelope.format_version(), VersionedEnvelope::<TestData>::FORMAT_VERSION_MOLECULE);
        assert_eq!(envelope.schema_version(), TestData::CURRENT_VERSION);
        assert!(envelope.payload_size() > 0);
    }

    #[test]
    fn test_serialization_error_conversions() {
        let io_err = std::io::Error::new(std::io::ErrorKind::Other, "test error");
        let ser_err: SerializationError = io_err.into();
        assert!(matches!(ser_err, SerializationError::IoError(_)));
    }

    #[test]
    fn test_vm_serializable_abi_compatibility() {
        // Test that abi_version matches is_abi_compatible
        let version = TestData::CURRENT_VERSION as u16;
        assert!(TestData::is_abi_compatible(version));
        assert!(!TestData::is_abi_compatible(version + 1));
    }

    // Test struct for an explicit VM wire format.
    #[derive(Debug, Clone, PartialEq, Eq)]
    struct TestVmData {
        id: u32,
        data: Vec<u8>,
    }

    impl VmSerializable for TestVmData {
        fn to_vm_bytes(&self) -> Vec<u8> {
            let mut out = Vec::with_capacity(8 + self.data.len());
            out.extend_from_slice(&self.id.to_le_bytes());
            out.extend_from_slice(&(self.data.len() as u32).to_le_bytes());
            out.extend_from_slice(&self.data);
            out
        }

        fn from_vm_bytes(bytes: &[u8]) -> Result<Self, VmAbiError> {
            if bytes.len() < 8 {
                return Err(VmAbiError::DeserializationFailed("test VM payload too short".to_string()));
            }
            let id = u32::from_le_bytes(bytes[..4].try_into().expect("slice length checked"));
            let data_len = u32::from_le_bytes(bytes[4..8].try_into().expect("slice length checked")) as usize;
            if bytes.len() != 8 + data_len {
                return Err(VmAbiError::DeserializationFailed("test VM payload length mismatch".to_string()));
            }
            Ok(Self { id, data: bytes[8..].to_vec() })
        }

        fn abi_version() -> u16 {
            VmAbiNegotiator::ABI_VERSION_MOLECULE_V1
        }
    }

    #[test]
    fn test_vm_serializable_roundtrip() {
        let data = TestVmData { id: 123, data: vec![1, 2, 3, 4, 5] };

        let bytes = data.to_vm_bytes();
        let restored = TestVmData::from_vm_bytes(&bytes).unwrap();

        assert_eq!(data, restored);
        assert_eq!(TestVmData::abi_version(), VmAbiNegotiator::ABI_VERSION_MOLECULE_V1);
    }

    #[test]
    fn test_vm_serializable_deserialization_error() {
        let invalid_bytes = vec![0xFF, 0xFF, 0xFF]; // Invalid test payload
        let result = TestVmData::from_vm_bytes(&invalid_bytes);
        assert!(matches!(result, Err(VmAbiError::DeserializationFailed(_))));
    }

    #[test]
    fn test_vm_abi_error_display() {
        let err = VmAbiError::SerializationFailed("test".to_string());
        assert!(err.to_string().contains("test"));

        let err = VmAbiError::DeserializationFailed("test".to_string());
        assert!(err.to_string().contains("test"));

        let err = VmAbiError::VersionMismatch { expected: 1, actual: 2 };
        assert!(err.to_string().contains("1") && err.to_string().contains("2"));

        let err = VmAbiError::UnsupportedAbiVersion(0x8001);
        assert!(err.to_string().contains("8001"));
    }

    #[test]
    fn test_versioned_envelope_with_empty_data() {
        #[derive(Debug, Clone, PartialEq, Eq)]
        struct MinimalData;

        impl VersionedSerializable for MinimalData {
            const CURRENT_VERSION: u8 = 1;

            fn to_versioned_payload(&self) -> Result<Vec<u8>, SerializationError> {
                Ok(Vec::new())
            }

            fn upgrade_from(version: u8, bytes: &[u8]) -> Result<Self, SerializationError> {
                if version != Self::CURRENT_VERSION {
                    return Err(SerializationError::UpgradePathNotAvailable { from: version, to: Self::CURRENT_VERSION });
                }
                if bytes.is_empty() {
                    Ok(Self)
                } else {
                    Err(SerializationError::DeserializationFailed("MinimalData payload must be empty".to_string()))
                }
            }
        }

        let data = MinimalData;
        let envelope = VersionedEnvelope::new(&data).unwrap();
        let restored: MinimalData = envelope.parse().unwrap();
        assert_eq!(data, restored);
    }

    #[test]
    fn test_versioned_envelope_with_very_large_version() {
        #[derive(Debug, Clone, PartialEq, Eq)]
        struct TestData;

        impl VersionedSerializable for TestData {
            const CURRENT_VERSION: u8 = 255; // Max u8

            fn to_versioned_payload(&self) -> Result<Vec<u8>, SerializationError> {
                Ok(Vec::new())
            }

            fn upgrade_from(version: u8, bytes: &[u8]) -> Result<Self, SerializationError> {
                if version != Self::CURRENT_VERSION {
                    return Err(SerializationError::UpgradePathNotAvailable { from: version, to: Self::CURRENT_VERSION });
                }
                if bytes.is_empty() {
                    Ok(Self)
                } else {
                    Err(SerializationError::DeserializationFailed("unit TestData payload must be empty".to_string()))
                }
            }
        }

        let data = TestData;
        let envelope = VersionedEnvelope::new(&data).unwrap();
        assert_eq!(envelope.schema_version(), 255);
        let restored: TestData = envelope.parse().unwrap();
        assert_eq!(data, restored);
    }

    #[test]
    fn test_version_upgrade_with_multiple_versions() {
        #[derive(Debug, Clone, PartialEq, Eq)]
        struct MultiVersionData {
            value: u32,
        }

        impl VersionedSerializable for MultiVersionData {
            const CURRENT_VERSION: u8 = 3;

            fn to_versioned_payload(&self) -> Result<Vec<u8>, SerializationError> {
                Ok(self.value.to_le_bytes().to_vec())
            }

            fn upgrade_from(version: u8, bytes: &[u8]) -> Result<Self, SerializationError> {
                match version {
                    1 => {
                        // v1 had a single byte value
                        if bytes.is_empty() {
                            return Err(SerializationError::DeserializationFailed("empty bytes".to_string()));
                        }
                        Ok(Self { value: bytes[0] as u32 })
                    }
                    2 => {
                        // v2 had a u16 value
                        if bytes.len() < 2 {
                            return Err(SerializationError::DeserializationFailed("insufficient bytes".to_string()));
                        }
                        let value = u16::from_le_bytes([bytes[0], bytes[1]]) as u32;
                        Ok(Self { value })
                    }
                    3 => {
                        if bytes.len() != 4 {
                            return Err(SerializationError::DeserializationFailed("v3 value must be u32le".to_string()));
                        }
                        Ok(Self { value: u32::from_le_bytes(bytes.try_into().expect("slice length checked")) })
                    }
                    _ => Err(SerializationError::UpgradePathNotAvailable { from: version, to: 3 }),
                }
            }
        }

        // Test v1 → v3 migration
        let v1_envelope = VersionedEnvelope {
            format_version: VersionedEnvelope::<MultiVersionData>::FORMAT_VERSION_MOLECULE,
            schema_version: 1,
            payload: vec![42],
            _phantom: std::marker::PhantomData::<MultiVersionData>,
        };
        let result = v1_envelope.parse().unwrap();
        assert_eq!(result.value, 42);

        // Test v2 → v3 migration
        let v2_envelope = VersionedEnvelope {
            format_version: VersionedEnvelope::<MultiVersionData>::FORMAT_VERSION_MOLECULE,
            schema_version: 2,
            payload: vec![0x39, 0x05], // 1337 in little-endian
            _phantom: std::marker::PhantomData::<MultiVersionData>,
        };
        let result = v2_envelope.parse().unwrap();
        assert_eq!(result.value, 1337);
    }

    #[test]
    fn test_vm_abi_negotiation_with_empty_capabilities() {
        let result = VmAbiNegotiator::negotiate(VmAbiNegotiator::ABI_VERSION_MOLECULE_V1, &[]);
        assert!(result.is_err());
        assert!(matches!(result, Err(VmAbiError::VersionMismatch { expected: VmAbiNegotiator::ABI_VERSION_MOLECULE_V1, actual: 0 })));
    }

    #[test]
    fn test_vm_abi_negotiation_with_multiple_capabilities() {
        let caps = vec![0x0001, 0x0002, 0x8001];

        // Should find exact match
        let result = VmAbiNegotiator::negotiate(VmAbiNegotiator::ABI_VERSION_MOLECULE_V1, &caps).unwrap();
        assert_eq!(result, VmAbiNegotiator::ABI_VERSION_MOLECULE_V1);

        let result = VmAbiNegotiator::negotiate(0x8002, &caps);
        assert!(matches!(result, Err(VmAbiError::UnsupportedAbiVersion(0x8002))));
    }
}
