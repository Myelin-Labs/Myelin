// SPDX-License-Identifier: MIT
// Copyright (C) 2026 Myelin developers
//
// Serialization Validation
//
//! # 序列化验证
//
//! 本模块提供序列化数据的验证和校验功能。
//
//! ## 功能
//
//! - 格式版本验证
//! - Schema 版本验证
//! - 数据完整性校验
//! - 大小限制检查

use crate::serialization::{SerializationError, VersionedEnvelope};

/// 验证配置
#[derive(Clone, Debug)]
pub struct ValidationConfig {
    /// 最小允许的 schema 版本
    pub min_schema_version: u8,
    /// 最大允许的 schema 版本
    pub max_schema_version: u8,
    /// Allow Molecule-compatible envelope bytes.
    pub allow_molecule: bool,
    /// 最大 payload 大小 (字节)
    pub max_payload_size: usize,
    /// 是否启用严格模式
    pub strict_mode: bool,
}

impl Default for ValidationConfig {
    fn default() -> Self {
        Self {
            min_schema_version: 1,
            max_schema_version: 255,
            allow_molecule: true,
            max_payload_size: 10 * 1024 * 1024, // 10MB default
            strict_mode: false,
        }
    }
}

impl ValidationConfig {
    /// 创建宽松验证配置
    pub fn permissive() -> Self {
        Self {
            min_schema_version: 0,
            max_schema_version: 255,
            allow_molecule: true,
            max_payload_size: 100 * 1024 * 1024, // 100MB
            strict_mode: false,
        }
    }

    /// 创建严格验证配置
    pub fn strict() -> Self {
        Self {
            min_schema_version: 1,
            max_schema_version: 1,
            allow_molecule: true,
            max_payload_size: 1024 * 1024, // 1MB
            strict_mode: true,
        }
    }

    /// 设置最小 schema 版本
    pub fn with_min_schema_version(mut self, version: u8) -> Self {
        self.min_schema_version = version;
        self
    }

    /// 设置最大 schema 版本
    pub fn with_max_schema_version(mut self, version: u8) -> Self {
        self.max_schema_version = version;
        self
    }

    /// 设置最大 payload 大小
    pub fn with_max_payload_size(mut self, size: usize) -> Self {
        self.max_payload_size = size;
        self
    }
}

/// 验证结果
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum ValidationResult {
    /// 验证通过
    Valid,
    /// 警告（非严格模式下通过）
    Warning(String),
    /// 验证失败
    Invalid(String),
}

impl ValidationResult {
    /// 检查是否有效
    pub fn is_valid(&self) -> bool {
        matches!(self, ValidationResult::Valid | ValidationResult::Warning(_))
    }

    /// 检查是否是警告
    pub fn is_warning(&self) -> bool {
        matches!(self, ValidationResult::Warning(_))
    }

    /// 检查是否无效
    pub fn is_invalid(&self) -> bool {
        matches!(self, ValidationResult::Invalid(_))
    }

    /// 获取错误信息（如果有）
    pub fn error_message(&self) -> Option<&str> {
        match self {
            ValidationResult::Invalid(msg) => Some(msg),
            _ => None,
        }
    }

    /// 转换为 Result
    pub fn into_result(self, strict: bool) -> Result<(), SerializationError> {
        match self {
            ValidationResult::Valid => Ok(()),
            ValidationResult::Warning(_) if !strict => Ok(()),
            ValidationResult::Warning(msg) => Err(SerializationError::DeserializationFailed(format!("Validation warning: {}", msg))),
            ValidationResult::Invalid(msg) => Err(SerializationError::DeserializationFailed(format!("Validation failed: {}", msg))),
        }
    }
}

/// 序列化验证器
pub struct SerializerValidator {
    config: ValidationConfig,
}

impl SerializerValidator {
    /// 创建新的验证器
    pub fn new(config: ValidationConfig) -> Self {
        Self { config }
    }

    /// 使用默认配置创建验证器
    pub fn default() -> Self {
        Self::new(ValidationConfig::default())
    }

    /// 验证 VersionedEnvelope 字节
    pub fn validate_envelope(&self, bytes: &[u8]) -> ValidationResult {
        let envelope = match VersionedEnvelope::<()>::from_bytes(bytes) {
            Ok(envelope) => envelope,
            Err(error) => return ValidationResult::Invalid(error.to_string()),
        };

        let format_version = envelope.format_version();
        let schema_version = envelope.schema_version();

        // Validate format version
        let format_result = self.validate_format_version(format_version);
        if format_result.is_invalid() {
            return format_result;
        }

        // Validate schema version
        let schema_result = self.validate_schema_version(schema_version);
        if schema_result.is_invalid() {
            return schema_result;
        }

        if envelope.payload_size() > self.config.max_payload_size {
            return ValidationResult::Invalid(format!(
                "Payload too large: {} bytes, max {}",
                envelope.payload_size(),
                self.config.max_payload_size
            ));
        }

        // Combine results
        if format_result.is_warning() || schema_result.is_warning() {
            let msg = match (&format_result, &schema_result) {
                (ValidationResult::Warning(f), ValidationResult::Warning(s)) => {
                    format!("{}; {}", f, s)
                }
                (ValidationResult::Warning(f), _) => f.clone(),
                (_, ValidationResult::Warning(s)) => s.clone(),
                _ => String::new(),
            };
            ValidationResult::Warning(msg)
        } else {
            ValidationResult::Valid
        }
    }

    /// 验证格式版本
    fn validate_format_version(&self, version: u8) -> ValidationResult {
        match version {
            0x00 => ValidationResult::Invalid("Legacy format not allowed".to_string()),
            0x80..=0x8F if self.config.allow_molecule => ValidationResult::Valid,
            0x80..=0x8F => ValidationResult::Invalid("Molecule format not allowed".to_string()),
            _ => ValidationResult::Invalid(format!("Unknown format version: 0x{:02X}", version)),
        }
    }

    /// 验证 schema 版本
    fn validate_schema_version(&self, version: u8) -> ValidationResult {
        if version < self.config.min_schema_version {
            ValidationResult::Invalid(format!("Schema version {} below minimum {}", version, self.config.min_schema_version))
        } else if version > self.config.max_schema_version {
            if self.config.strict_mode {
                ValidationResult::Invalid(format!("Schema version {} above maximum {}", version, self.config.max_schema_version))
            } else {
                ValidationResult::Warning(format!(
                    "Schema version {} above maximum {}, may need upgrade",
                    version, self.config.max_schema_version
                ))
            }
        } else {
            ValidationResult::Valid
        }
    }

    /// 快速检查是否为有效的 envelope 格式
    pub fn is_valid_envelope(&self, bytes: &[u8]) -> bool {
        self.validate_envelope(bytes).is_valid()
    }

    /// 获取验证配置
    pub fn config(&self) -> &ValidationConfig {
        &self.config
    }
}

impl Default for SerializerValidator {
    fn default() -> Self {
        Self::default()
    }
}

/// 验证 VersionedEnvelope 字节（使用默认配置）
pub fn validate_envelope(bytes: &[u8]) -> ValidationResult {
    let validator = SerializerValidator::default();
    validator.validate_envelope(bytes)
}

/// 检查是否为有效的 envelope 格式（使用默认配置）
pub fn is_valid_envelope(bytes: &[u8]) -> bool {
    validate_envelope(bytes).is_valid()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn test_envelope_bytes(format_version: u8, schema_version: u8, payload_len: usize) -> Vec<u8> {
        VersionedEnvelope::<()>::from_parts(format_version, schema_version, vec![0u8; payload_len]).to_bytes()
    }

    #[test]
    fn test_validation_config_default() {
        let config = ValidationConfig::default();
        assert_eq!(config.min_schema_version, 1);
        assert_eq!(config.max_schema_version, 255);
        assert!(config.allow_molecule);
        assert_eq!(config.max_payload_size, 10 * 1024 * 1024);
        assert!(!config.strict_mode);
    }

    #[test]
    fn test_validation_config_permissive() {
        let config = ValidationConfig::permissive();
        assert_eq!(config.min_schema_version, 0);
        assert!(config.allow_molecule);
        assert_eq!(config.max_payload_size, 100 * 1024 * 1024);
    }

    #[test]
    fn test_validation_config_strict() {
        let config = ValidationConfig::strict();
        assert_eq!(config.min_schema_version, 1);
        assert_eq!(config.max_schema_version, 1);
        assert!(config.allow_molecule);
        assert!(config.strict_mode);
        assert_eq!(config.max_payload_size, 1024 * 1024);
    }

    #[test]
    fn test_validation_config_builder() {
        let config = ValidationConfig::default().with_min_schema_version(2).with_max_schema_version(10).with_max_payload_size(1024);

        assert_eq!(config.min_schema_version, 2);
        assert_eq!(config.max_schema_version, 10);
        assert_eq!(config.max_payload_size, 1024);
    }

    #[test]
    fn test_validate_envelope_rejects_legacy_format_by_default() {
        let validator = SerializerValidator::default();
        // Explicit legacy/custom format (0x00), schema version 1, minimal payload
        let data = test_envelope_bytes(0x00, 0x01, 0);
        let result = validator.validate_envelope(&data);
        assert!(result.is_invalid());
        assert!(result.error_message().unwrap().contains("Legacy format not allowed"));
    }

    #[test]
    fn test_validate_envelope_too_short() {
        let validator = SerializerValidator::default();
        let result = validator.validate_envelope(&[0x00, 0x01]);
        assert!(result.is_invalid());
        assert!(result.error_message().unwrap().contains("invalid VersionedEnvelope"));
    }

    #[test]
    fn test_validate_envelope_invalid_format() {
        let validator = SerializerValidator::default();
        let data = test_envelope_bytes(0xFF, 0x01, 0); // Invalid format version
        let result = validator.validate_envelope(&data);
        assert!(result.is_invalid());
        assert!(result.error_message().unwrap().contains("Unknown format"));
    }

    #[test]
    fn test_validate_envelope_molecule_valid() {
        let validator = SerializerValidator::default();
        let data = test_envelope_bytes(0x80, 0x01, 0); // Molecule format
        let result = validator.validate_envelope(&data);
        assert!(result.is_valid());
    }

    #[test]
    fn test_validate_envelope_molecule_allowed() {
        let config = ValidationConfig::default().with_min_schema_version(0);
        let validator = SerializerValidator::new(config);
        let data = test_envelope_bytes(0x80, 0x01, 0);
        let result = validator.validate_envelope(&data);
        assert!(result.is_valid());
    }

    #[test]
    fn test_validate_envelope_schema_too_low() {
        let validator = SerializerValidator::default();
        let data = test_envelope_bytes(0x80, 0x00, 0); // Schema version 0
        let result = validator.validate_envelope(&data);
        assert!(result.is_invalid());
        assert!(result.error_message().unwrap().contains("below minimum"));
    }

    #[test]
    fn test_validate_envelope_schema_too_high_strict() {
        let config = ValidationConfig::strict();
        let validator = SerializerValidator::new(config);
        let data = test_envelope_bytes(0x80, 0x02, 0); // Schema version 2
        let result = validator.validate_envelope(&data);
        assert!(result.is_invalid());
        assert!(result.error_message().unwrap().contains("above maximum"));
    }

    #[test]
    fn test_validate_envelope_schema_too_high_non_strict() {
        let config = ValidationConfig::default().with_max_schema_version(1);
        let validator = SerializerValidator::new(config);
        let data = test_envelope_bytes(0x80, 0x02, 0);
        let result = validator.validate_envelope(&data);
        assert!(result.is_warning());
        assert!(result.error_message().is_none()); // Warning doesn't have error_message
    }

    #[test]
    fn test_validation_result_helpers() {
        let valid = ValidationResult::Valid;
        assert!(valid.is_valid());
        assert!(!valid.is_warning());
        assert!(!valid.is_invalid());

        let warning = ValidationResult::Warning("test".to_string());
        assert!(warning.is_valid());
        assert!(warning.is_warning());
        assert!(!warning.is_invalid());

        let invalid = ValidationResult::Invalid("test".to_string());
        assert!(!invalid.is_valid());
        assert!(!invalid.is_warning());
        assert!(invalid.is_invalid());
        assert_eq!(invalid.error_message(), Some("test"));
    }

    #[test]
    fn test_validation_result_into_result() {
        let valid = ValidationResult::Valid;
        assert!(valid.clone().into_result(false).is_ok());
        assert!(valid.into_result(true).is_ok());

        let warning = ValidationResult::Warning("warn".to_string());
        assert!(warning.clone().into_result(false).is_ok());
        assert!(warning.into_result(true).is_err());

        let invalid = ValidationResult::Invalid("error".to_string());
        assert!(invalid.clone().into_result(false).is_err());
        assert!(invalid.into_result(true).is_err());
    }

    #[test]
    fn test_is_valid_envelope() {
        let validator = SerializerValidator::default();
        let valid_data = test_envelope_bytes(0x80, 0x01, 0);
        assert!(validator.is_valid_envelope(&valid_data));

        let invalid_data = test_envelope_bytes(0xFF, 0x01, 0);
        assert!(!validator.is_valid_envelope(&invalid_data));
    }

    #[test]
    fn test_global_validate_functions() {
        let valid_data = test_envelope_bytes(0x80, 0x01, 0);
        assert!(is_valid_envelope(&valid_data));
        assert!(validate_envelope(&valid_data).is_valid());
    }
}
