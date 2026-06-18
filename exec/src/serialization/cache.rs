// SPDX-License-Identifier: MIT
// Copyright (C) 2026 Spora developers
//
// Serialization Cache
//
//! # 序列化缓存
//!
//! 本模块提供序列化结果的缓存机制，用于优化频繁序列化的场景。
//!
//! ## 使用场景
//!
//! - VM 执行期间多次访问相同数据
//! - 重复计算相同交易的序列化结果
//! - 需要快速获取序列化大小的场景
//!
//! ## 示例
//!
//! ```rust
//! use spora_exec::serialization::cache::SerializationCache;
//!
//! let mut cache = SerializationCache::new(1000); // 最多缓存 1000 项
//!
//! // 缓存可用于任何实现 VersionedSerializable 的类型
//! ```

use crate::serialization::{SerializationError, VersionedSerializable};
use parking_lot::RwLock;
use std::collections::HashMap;
use std::hash::{Hash, Hasher};
use std::sync::Arc;

/// 序列化缓存键
///
/// 使用类型的 TypeId 和数据的 BLAKE3 哈希作为缓存键，
/// 确保极低的碰撞概率。
#[derive(Clone, Debug)]
struct CacheKey {
    type_id: std::any::TypeId,
    /// 数据的 BLAKE3 哈希 (256位)
    hash: [u8; 32],
}

impl PartialEq for CacheKey {
    fn eq(&self, other: &Self) -> bool {
        self.type_id == other.type_id && self.hash == other.hash
    }
}

impl Eq for CacheKey {}

impl Hash for CacheKey {
    fn hash<H: Hasher>(&self, state: &mut H) {
        self.type_id.hash(state);
        // 使用前8字节作为哈希状态输入
        state.write(&self.hash[..8]);
    }
}

/// 序列化缓存
///
/// 缓存序列化结果以避免重复计算。
/// 使用 LRU (Least Recently Used) 策略管理缓存大小。
pub struct SerializationCache {
    cache: HashMap<CacheKey, Arc<Vec<u8>>>,
    max_size: usize,
    access_order: Vec<CacheKey>, // 简单的 LRU 实现
}

impl SerializationCache {
    /// 创建新的序列化缓存
    ///
    /// # Arguments
    /// * `max_size` - 最大缓存项数
    pub fn new(max_size: usize) -> Self {
        Self { cache: HashMap::with_capacity(max_size), max_size, access_order: Vec::with_capacity(max_size) }
    }

    /// 获取或序列化值
    ///
    /// 如果值已在缓存中，返回缓存的副本。
    /// 否则，序列化值并缓存结果。
    pub fn get_or_serialize<T: VersionedSerializable + 'static>(&mut self, value: &T) -> Result<Arc<Vec<u8>>, SerializationError> {
        let key = self.make_key(value);

        // Check cache
        if let Some(cached) = self.cache.get(&key).cloned() {
            self.update_access_order(&key);
            return Ok(cached);
        }

        // Serialize
        let envelope = crate::serialization::VersionedEnvelope::new(value)?;
        let bytes = borsh::to_vec(&envelope).map_err(|e| SerializationError::IoError(e.to_string()))?;
        let bytes = Arc::new(bytes);

        // Store in cache
        self.insert(key, Arc::clone(&bytes));

        Ok(bytes)
    }

    /// 检查值是否在缓存中
    pub fn contains<T: VersionedSerializable + 'static>(&self, value: &T) -> bool {
        let key = self.make_key(value);
        self.cache.contains_key(&key)
    }

    /// 获取缓存命中率统计
    pub fn stats(&self) -> CacheStats {
        CacheStats { size: self.cache.len(), max_size: self.max_size, utilization: self.cache.len() as f64 / self.max_size as f64 }
    }

    /// 清空缓存
    pub fn clear(&mut self) {
        self.cache.clear();
        self.access_order.clear();
    }

    /// 获取当前缓存大小
    pub fn len(&self) -> usize {
        self.cache.len()
    }

    /// 检查缓存是否为空
    pub fn is_empty(&self) -> bool {
        self.cache.is_empty()
    }

    fn make_key<T: VersionedSerializable + 'static>(&self, value: &T) -> CacheKey {
        // 首先序列化数据
        let serialized = match crate::serialization::utils::serialize_to_bytes(value) {
            Ok(bytes) => bytes,
            Err(_) => {
                // 如果序列化失败，使用类型哈希作为回退
                let mut hasher = std::collections::hash_map::DefaultHasher::new();
                std::any::TypeId::of::<T>().hash(&mut hasher);
                let hash_val = hasher.finish();
                let mut bytes = vec![0u8; 32];
                bytes[..8].copy_from_slice(&hash_val.to_le_bytes());
                return CacheKey { type_id: std::any::TypeId::of::<T>(), hash: bytes.try_into().unwrap() };
            }
        };

        // 计算 BLAKE3 哈希
        let hash = crate::serialization::security::compute_hash(&serialized);

        CacheKey { type_id: std::any::TypeId::of::<T>(), hash }
    }

    fn insert(&mut self, key: CacheKey, value: Arc<Vec<u8>>) {
        // Evict if necessary
        if self.cache.len() >= self.max_size && !self.cache.contains_key(&key) {
            if let Some(oldest) = self.access_order.first().cloned() {
                self.cache.remove(&oldest);
                self.access_order.remove(0);
            }
        }

        self.cache.insert(key.clone(), value);
        self.access_order.push(key);
    }

    fn update_access_order(&mut self, key: &CacheKey) {
        if let Some(pos) = self.access_order.iter().position(|k| k == key) {
            let key = self.access_order.remove(pos);
            self.access_order.push(key);
        }
    }
}

impl Default for SerializationCache {
    fn default() -> Self {
        Self::new(1000)
    }
}

/// 缓存统计信息
#[derive(Clone, Debug, Copy)]
pub struct CacheStats {
    /// 当前缓存项数
    pub size: usize,
    /// 最大缓存项数
    pub max_size: usize,
    /// 缓存利用率 (0.0 - 1.0)
    pub utilization: f64,
}

impl CacheStats {
    /// 获取缓存利用率百分比
    pub fn utilization_percent(&self) -> f64 {
        self.utilization * 100.0
    }
}

/// 线程安全的序列化缓存
///
/// 使用 parking_lot::RwLock 实现线程安全，适合多线程环境。
/// parking_lot 提供更优的性能和更小的内存占用。
pub struct ThreadSafeSerializationCache {
    inner: RwLock<SerializationCache>,
}

impl ThreadSafeSerializationCache {
    /// 创建新的线程安全缓存
    pub fn new(max_size: usize) -> Self {
        Self { inner: RwLock::new(SerializationCache::new(max_size)) }
    }

    /// 获取或序列化值
    pub fn get_or_serialize<T: VersionedSerializable + 'static>(&self, value: &T) -> Result<Arc<Vec<u8>>, SerializationError> {
        let mut cache = self.inner.write();
        cache.get_or_serialize(value)
    }

    /// 获取缓存统计
    pub fn stats(&self) -> CacheStats {
        let cache = self.inner.read();
        cache.stats()
    }

    /// 清空缓存
    pub fn clear(&self) {
        let mut cache = self.inner.write();
        cache.clear()
    }
}

impl Default for ThreadSafeSerializationCache {
    fn default() -> Self {
        Self::new(1000)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::celltx::{CellOutput, Script};

    fn create_test_output() -> CellOutput {
        CellOutput { lock: Script::new([0xAA; 32], 0, vec![0xBB; 20]), type_: None, capacity: 1000 }
    }

    #[test]
    fn test_cache_basic() {
        let mut cache = SerializationCache::new(10);
        let output = create_test_output();

        // First access - should serialize
        let bytes1 = cache.get_or_serialize(&output).unwrap();
        assert_eq!(cache.len(), 1);

        // Second access - should return cached
        let bytes2 = cache.get_or_serialize(&output).unwrap();
        assert_eq!(cache.len(), 1); // No new entry
        assert_eq!(bytes1.as_ptr(), bytes2.as_ptr()); // Same memory
    }

    #[test]
    fn test_cache_contains() {
        let mut cache = SerializationCache::new(10);
        let output = create_test_output();

        assert!(!cache.contains(&output));
        cache.get_or_serialize(&output).unwrap();
        assert!(cache.contains(&output));
    }

    #[test]
    fn test_cache_eviction() {
        let mut cache = SerializationCache::new(2);

        let output1 = CellOutput { lock: Script::new([0x01; 32], 0, vec![0x01; 20]), type_: None, capacity: 1000 };
        let output2 = CellOutput { lock: Script::new([0x02; 32], 0, vec![0x02; 20]), type_: None, capacity: 2000 };
        let output3 = CellOutput { lock: Script::new([0x03; 32], 0, vec![0x03; 20]), type_: None, capacity: 3000 };

        cache.get_or_serialize(&output1).unwrap();
        cache.get_or_serialize(&output2).unwrap();
        assert_eq!(cache.len(), 2);
        assert!(cache.contains(&output1));
        assert!(cache.contains(&output2));

        // Add third item, should evict first
        cache.get_or_serialize(&output3).unwrap();
        assert_eq!(cache.len(), 2);
        assert!(!cache.contains(&output1)); // Evicted
        assert!(cache.contains(&output2));
        assert!(cache.contains(&output3));
    }

    #[test]
    fn test_cache_stats() {
        let mut cache = SerializationCache::new(100);
        let output = create_test_output();

        let stats = cache.stats();
        assert_eq!(stats.size, 0);
        assert_eq!(stats.max_size, 100);
        assert_eq!(stats.utilization, 0.0);

        cache.get_or_serialize(&output).unwrap();
        let stats = cache.stats();
        assert_eq!(stats.size, 1);
        assert_eq!(stats.utilization, 0.01);
        assert_eq!(stats.utilization_percent(), 1.0);
    }

    #[test]
    fn test_cache_clear() {
        let mut cache = SerializationCache::new(10);
        let output = create_test_output();

        cache.get_or_serialize(&output).unwrap();
        assert_eq!(cache.len(), 1);

        cache.clear();
        assert_eq!(cache.len(), 0);
        assert!(!cache.contains(&output));
    }

    #[test]
    fn test_thread_safe_cache() {
        let cache = ThreadSafeSerializationCache::new(10);
        let output = create_test_output();

        let bytes1 = cache.get_or_serialize(&output).unwrap();
        let bytes2 = cache.get_or_serialize(&output).unwrap();

        assert_eq!(bytes1.as_ptr(), bytes2.as_ptr());
    }

    #[test]
    fn test_default_cache() {
        let cache: SerializationCache = Default::default();
        assert_eq!(cache.max_size, 1000);
        assert!(cache.is_empty());
    }
}
