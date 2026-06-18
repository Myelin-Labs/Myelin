// SPDX-License-Identifier: MIT
// Copyright (C) 2026 Spora developers
//
// Serialization Compression
//
//! # 序列化压缩
//
//! 本模块提供序列化数据的压缩支持，减少存储和传输开销。
//
//! ## 支持的压缩算法
//
//! - **Zstd**: 高压缩率，快速解压（默认）
//! - **LZ4**: 超快速压缩和解压
//! - **None**: 无压缩（用于小数据）

use crate::serialization::SerializationError;

/// 压缩算法类型
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[repr(u8)]
pub enum CompressionAlgorithm {
    /// 无压缩
    None = 0x00,
    /// LZ4 压缩
    LZ4 = 0x01,
    /// Zstd 压缩
    Zstd = 0x02,
}

impl CompressionAlgorithm {
    /// 从字节解析
    pub fn from_u8(value: u8) -> Option<Self> {
        match value {
            0x00 => Some(Self::None),
            0x01 => Some(Self::LZ4),
            0x02 => Some(Self::Zstd),
            _ => None,
        }
    }

    /// 获取算法名称
    pub fn name(&self) -> &'static str {
        match self {
            Self::None => "none",
            Self::LZ4 => "lz4",
            Self::Zstd => "zstd",
        }
    }
}

impl Default for CompressionAlgorithm {
    fn default() -> Self {
        Self::Zstd
    }
}

/// 压缩配置
#[derive(Clone, Debug)]
pub struct CompressionConfig {
    /// 压缩算法
    pub algorithm: CompressionAlgorithm,
    /// 压缩级别 (1-22 for zstd, 1-12 for lz4)
    pub level: i32,
    /// 最小压缩大小（小于此值不压缩）
    pub min_size: usize,
    /// 启用自动算法选择
    pub auto_select: bool,
}

impl CompressionConfig {
    /// 创建默认配置（Zstd 级别 3）
    pub fn default() -> Self {
        Self {
            algorithm: CompressionAlgorithm::Zstd,
            level: 3,
            min_size: 1024, // 1KB
            auto_select: false,
        }
    }

    /// 创建高速配置（LZ4）
    pub fn fast() -> Self {
        Self { algorithm: CompressionAlgorithm::LZ4, level: 1, min_size: 256, auto_select: false }
    }

    /// 创建高压缩率配置（Zstd 级别 19）
    pub fn best() -> Self {
        Self { algorithm: CompressionAlgorithm::Zstd, level: 19, min_size: 512, auto_select: false }
    }

    /// 创建无压缩配置
    pub fn none() -> Self {
        Self { algorithm: CompressionAlgorithm::None, level: 0, min_size: usize::MAX, auto_select: false }
    }

    /// 创建自动选择配置
    pub fn auto() -> Self {
        Self { algorithm: CompressionAlgorithm::Zstd, level: 3, min_size: 1024, auto_select: true }
    }
}

impl Default for CompressionConfig {
    fn default() -> Self {
        Self::default()
    }
}

/// 压缩结果
#[derive(Clone, Debug)]
pub struct CompressionResult {
    /// 压缩后的数据
    pub data: Vec<u8>,
    /// 使用的算法
    pub algorithm: CompressionAlgorithm,
    /// 原始大小
    pub original_size: usize,
    /// 压缩后大小
    pub compressed_size: usize,
}

impl CompressionResult {
    /// 计算压缩率
    pub fn ratio(&self) -> f64 {
        if self.original_size == 0 {
            return 1.0;
        }
        self.compressed_size as f64 / self.original_size as f64
    }

    /// 计算节省的空间百分比
    pub fn savings_percent(&self) -> f64 {
        (1.0 - self.ratio()) * 100.0
    }

    /// 检查是否实际压缩了（压缩后更小）
    pub fn is_compressed(&self) -> bool {
        self.compressed_size < self.original_size
    }
}

/// 压缩数据
///
/// 注意：此函数目前返回未压缩数据，因为需要添加 zstd 和 lz4 依赖。
/// 在实际部署时，应该启用相应的压缩算法。
pub fn compress(data: &[u8], config: &CompressionConfig) -> Result<CompressionResult, SerializationError> {
    // Check minimum size
    if data.len() < config.min_size {
        return Ok(CompressionResult {
            data: data.to_vec(),
            algorithm: CompressionAlgorithm::None,
            original_size: data.len(),
            compressed_size: data.len(),
        });
    }

    match config.algorithm {
        CompressionAlgorithm::None => Ok(CompressionResult {
            data: data.to_vec(),
            algorithm: CompressionAlgorithm::None,
            original_size: data.len(),
            compressed_size: data.len(),
        }),

        // LZ4 and Zstd are reserved for future implementation
        // Currently return error to prevent silent failures
        CompressionAlgorithm::LZ4 => Err(SerializationError::DeserializationFailed(
            "LZ4 compression not yet implemented. Add 'lz4' feature to enable.".to_string(),
        )),

        CompressionAlgorithm::Zstd => Err(SerializationError::DeserializationFailed(
            "Zstd compression not yet implemented. Add 'zstd' feature to enable.".to_string(),
        )),
    }
}

/// 解压数据
///
/// 注意：此函数目前仅支持未压缩数据。
pub fn decompress(data: &[u8], algorithm: CompressionAlgorithm) -> Result<Vec<u8>, SerializationError> {
    match algorithm {
        CompressionAlgorithm::None => Ok(data.to_vec()),

        // Placeholder implementations
        CompressionAlgorithm::LZ4 => {
            Err(SerializationError::DeserializationFailed("LZ4 decompression not yet implemented".to_string()))
        }

        CompressionAlgorithm::Zstd => {
            Err(SerializationError::DeserializationFailed("Zstd decompression not yet implemented".to_string()))
        }
    }
}

/// 带压缩的序列化信封
#[derive(Clone, Debug)]
pub struct CompressedEnvelope {
    /// 压缩算法
    pub algorithm: CompressionAlgorithm,
    /// 压缩后的数据
    pub data: Vec<u8>,
    /// 原始大小
    pub original_size: u32,
}

impl CompressedEnvelope {
    /// 压缩数据
    pub fn compress(data: &[u8], config: &CompressionConfig) -> Result<Self, SerializationError> {
        let result = compress(data, config)?;
        Ok(Self { algorithm: result.algorithm, data: result.data, original_size: result.original_size as u32 })
    }

    /// 解压数据
    pub fn decompress(&self) -> Result<Vec<u8>, SerializationError> {
        let data = decompress(&self.data, self.algorithm)?;

        // Verify size
        if data.len() != self.original_size as usize {
            return Err(SerializationError::DeserializationFailed(format!(
                "Decompressed size {} doesn't match expected {}",
                data.len(),
                self.original_size
            )));
        }

        Ok(data)
    }

    /// 计算压缩率
    pub fn compression_ratio(&self) -> f64 {
        if self.original_size == 0 {
            return 1.0;
        }
        self.data.len() as f64 / self.original_size as f64
    }

    /// 序列化为字节
    pub fn to_bytes(&self) -> Vec<u8> {
        let mut result = Vec::with_capacity(5 + self.data.len());
        result.push(self.algorithm as u8);
        result.extend_from_slice(&self.original_size.to_le_bytes());
        result.extend_from_slice(&self.data);
        result
    }

    /// 从字节解析
    pub fn from_bytes(bytes: &[u8]) -> Result<Self, SerializationError> {
        if bytes.len() < 5 {
            return Err(SerializationError::DeserializationFailed("Insufficient bytes for CompressedEnvelope".to_string()));
        }

        let algorithm = CompressionAlgorithm::from_u8(bytes[0])
            .ok_or_else(|| SerializationError::DeserializationFailed(format!("Unknown compression algorithm: {}", bytes[0])))?;

        let original_size = u32::from_le_bytes([bytes[1], bytes[2], bytes[3], bytes[4]]);
        let data = bytes[5..].to_vec();

        Ok(Self { algorithm, data, original_size })
    }
}

/// 压缩统计
#[derive(Clone, Debug, Default)]
pub struct CompressionStats {
    /// 总压缩次数
    pub total_compressions: u64,
    /// 成功压缩次数（实际减小了大小）
    pub successful_compressions: u64,
    /// 跳过的压缩次数（数据太小）
    pub skipped_compressions: u64,
    /// 原始总大小
    pub total_original_size: u64,
    /// 压缩后总大小
    pub total_compressed_size: u64,
}

impl CompressionStats {
    /// 创建新的统计
    pub fn new() -> Self {
        Self::default()
    }

    /// 记录一次压缩
    pub fn record(&mut self, original_size: usize, compressed_size: usize) {
        self.total_compressions += 1;
        self.total_original_size += original_size as u64;
        self.total_compressed_size += compressed_size as u64;

        if compressed_size < original_size {
            self.successful_compressions += 1;
        }
    }

    /// 记录跳过的压缩
    pub fn record_skipped(&mut self) {
        self.total_compressions += 1;
        self.skipped_compressions += 1;
    }

    /// 计算整体压缩率
    pub fn overall_ratio(&self) -> f64 {
        if self.total_original_size == 0 {
            return 1.0;
        }
        self.total_compressed_size as f64 / self.total_original_size as f64
    }

    /// 计算节省的空间
    pub fn space_saved(&self) -> u64 {
        self.total_original_size.saturating_sub(self.total_compressed_size)
    }

    /// 计算节省的百分比
    pub fn savings_percent(&self) -> f64 {
        (1.0 - self.overall_ratio()) * 100.0
    }
}

/// 估计压缩后大小（启发式）
///
/// 用于决定是否进行压缩。
pub fn estimate_compressed_size(original_size: usize, algorithm: CompressionAlgorithm) -> usize {
    match algorithm {
        CompressionAlgorithm::None => original_size,
        // Conservative estimates
        CompressionAlgorithm::LZ4 => original_size.saturating_sub(original_size / 4), // ~25% reduction
        CompressionAlgorithm::Zstd => original_size.saturating_sub(original_size / 3), // ~33% reduction
    }
}

/// 选择最佳压缩算法
///
/// 根据数据特征选择最佳算法。
pub fn select_algorithm(data: &[u8], speed_priority: bool) -> CompressionAlgorithm {
    if data.len() < 1024 {
        return CompressionAlgorithm::None;
    }

    if speed_priority {
        CompressionAlgorithm::LZ4
    } else {
        CompressionAlgorithm::Zstd
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_compression_algorithm_from_u8() {
        assert_eq!(CompressionAlgorithm::from_u8(0x00), Some(CompressionAlgorithm::None));
        assert_eq!(CompressionAlgorithm::from_u8(0x01), Some(CompressionAlgorithm::LZ4));
        assert_eq!(CompressionAlgorithm::from_u8(0x02), Some(CompressionAlgorithm::Zstd));
        assert_eq!(CompressionAlgorithm::from_u8(0xFF), None);
    }

    #[test]
    fn test_compression_algorithm_name() {
        assert_eq!(CompressionAlgorithm::None.name(), "none");
        assert_eq!(CompressionAlgorithm::LZ4.name(), "lz4");
        assert_eq!(CompressionAlgorithm::Zstd.name(), "zstd");
    }

    #[test]
    fn test_compression_config_variants() {
        let default = CompressionConfig::default();
        assert_eq!(default.algorithm, CompressionAlgorithm::Zstd);
        assert_eq!(default.level, 3);

        let fast = CompressionConfig::fast();
        assert_eq!(fast.algorithm, CompressionAlgorithm::LZ4);

        let best = CompressionConfig::best();
        assert_eq!(best.algorithm, CompressionAlgorithm::Zstd);
        assert_eq!(best.level, 19);

        let none = CompressionConfig::none();
        assert_eq!(none.algorithm, CompressionAlgorithm::None);
    }

    #[test]
    fn test_compress_none() {
        let config = CompressionConfig::none();
        let data = vec![0x01, 0x02, 0x03, 0x04];

        let result = compress(&data, &config).unwrap();
        assert_eq!(result.algorithm, CompressionAlgorithm::None);
        assert_eq!(result.data, data);
        assert!(!result.is_compressed());
        assert_eq!(result.savings_percent(), 0.0);
    }

    #[test]
    fn test_compress_min_size() {
        let config = CompressionConfig { min_size: 100, ..CompressionConfig::default() };
        let data = vec![0x01; 50]; // Less than min_size

        let result = compress(&data, &config).unwrap();
        assert_eq!(result.algorithm, CompressionAlgorithm::None);
    }

    #[test]
    fn test_compressed_envelope() {
        let data = vec![0x01, 0x02, 0x03, 0x04];
        let config = CompressionConfig::none();

        let envelope = CompressedEnvelope::compress(&data, &config).unwrap();
        assert_eq!(envelope.algorithm, CompressionAlgorithm::None);
        assert_eq!(envelope.original_size, 4);

        let bytes = envelope.to_bytes();
        let restored = CompressedEnvelope::from_bytes(&bytes).unwrap();
        assert_eq!(envelope.algorithm, restored.algorithm);
        assert_eq!(envelope.original_size, restored.original_size);

        let decompressed = restored.decompress().unwrap();
        assert_eq!(decompressed, data);
    }

    #[test]
    fn test_compressed_envelope_invalid_algorithm() {
        let bytes = vec![0xFF, 0x04, 0x00, 0x00, 0x00]; // Invalid algorithm
        let result = CompressedEnvelope::from_bytes(&bytes);
        assert!(result.is_err());
    }

    #[test]
    fn test_compression_stats() {
        let mut stats = CompressionStats::new();

        stats.record(1000, 800);
        assert_eq!(stats.total_compressions, 1);
        assert_eq!(stats.successful_compressions, 1);
        assert_eq!(stats.total_original_size, 1000);
        assert_eq!(stats.total_compressed_size, 800);

        stats.record_skipped();
        assert_eq!(stats.total_compressions, 2);
        assert_eq!(stats.skipped_compressions, 1);

        assert_eq!(stats.space_saved(), 200);
        assert!((stats.savings_percent() - 20.0).abs() < 1e-9);
    }

    #[test]
    fn test_estimate_compressed_size() {
        let size = 1000;

        assert_eq!(estimate_compressed_size(size, CompressionAlgorithm::None), 1000);
        assert!(estimate_compressed_size(size, CompressionAlgorithm::LZ4) < 1000);
        assert!(estimate_compressed_size(size, CompressionAlgorithm::Zstd) < 1000);
    }

    #[test]
    fn test_select_algorithm() {
        let small_data = vec![0x01; 100];
        assert_eq!(select_algorithm(&small_data, false), CompressionAlgorithm::None);

        let large_data = vec![0x01; 10000];
        assert_eq!(select_algorithm(&large_data, true), CompressionAlgorithm::LZ4);
        assert_eq!(select_algorithm(&large_data, false), CompressionAlgorithm::Zstd);
    }

    #[test]
    fn test_compression_result_ratio() {
        let result = CompressionResult {
            data: vec![0x01; 800],
            algorithm: CompressionAlgorithm::Zstd,
            original_size: 1000,
            compressed_size: 800,
        };

        assert_eq!(result.ratio(), 0.8);
        assert!((result.savings_percent() - 20.0).abs() < 1e-9);
        assert!(result.is_compressed());
    }

    #[test]
    fn test_compression_result_not_compressed() {
        let result = CompressionResult {
            data: vec![0x01; 1000],
            algorithm: CompressionAlgorithm::None,
            original_size: 1000,
            compressed_size: 1000,
        };

        assert!(!result.is_compressed());
        assert_eq!(result.savings_percent(), 0.0);
    }

    #[test]
    fn test_compressed_envelope_empty() {
        let data: Vec<u8> = vec![];
        let config = CompressionConfig::none();

        let envelope = CompressedEnvelope::compress(&data, &config).unwrap();
        let bytes = envelope.to_bytes();
        let restored = CompressedEnvelope::from_bytes(&bytes).unwrap();
        let decompressed = restored.decompress().unwrap();

        assert!(decompressed.is_empty());
    }

    #[test]
    fn test_compressed_envelope_large() {
        let data: Vec<u8> = (0..10000).map(|i| (i % 256) as u8).collect();
        let config = CompressionConfig::none();

        let envelope = CompressedEnvelope::compress(&data, &config).unwrap();
        let bytes = envelope.to_bytes();
        assert_eq!(bytes.len(), 5 + data.len());

        let restored = CompressedEnvelope::from_bytes(&bytes).unwrap();
        let decompressed = restored.decompress().unwrap();
        assert_eq!(decompressed, data);
    }
}
