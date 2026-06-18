// SPDX-License-Identifier: MIT
// Copyright (C) 2026 Spora developers
//
// ScriptIndex: lock_hash/type_hash → Cells (for wallet queries)

use crate::{Result, StateError};
use parking_lot::RwLock;
use rocksdb::{ColumnFamilyDescriptor, Options, DB};
use spora_exec::OutPoint;
use std::collections::BTreeSet;
use std::path::Path;
use std::sync::Arc;

/// Column families for script indexing
const CF_LOCK_INDEX: &str = "lock_index";
const CF_TYPE_INDEX: &str = "type_index";

/// Script index database
///
/// Maps script hashes to OutPoints for wallet queries:
/// - lock_hash → [OutPoint, ...]
/// - type_hash → [OutPoint, ...]
pub struct ScriptIndex {
    /// RocksDB instance
    db: Arc<DB>,
    /// Write lock
    write_lock: Arc<RwLock<()>>,
}

impl ScriptIndex {
    /// Open or create a ScriptIndex
    pub fn open<P: AsRef<Path>>(path: P) -> Result<Self> {
        let mut opts = Options::default();
        opts.create_if_missing(true);
        opts.create_missing_column_families(true);
        opts.set_compression_type(rocksdb::DBCompressionType::Snappy);

        let cf_lock = ColumnFamilyDescriptor::new(CF_LOCK_INDEX, Options::default());
        let cf_type = ColumnFamilyDescriptor::new(CF_TYPE_INDEX, Options::default());

        let db = DB::open_cf_descriptors(&opts, path, vec![cf_lock, cf_type]).map_err(|e| StateError::Database(e.to_string()))?;

        Ok(Self { db: Arc::new(db), write_lock: Arc::new(RwLock::new(())) })
    }

    /// Add a Cell to lock script index
    pub fn add_lock(&self, lock_hash: &[u8; 32], out_point: &OutPoint) -> Result<()> {
        self.add_to_index(CF_LOCK_INDEX, lock_hash, out_point)
    }

    /// Add a Cell to type script index
    pub fn add_type(&self, type_hash: &[u8; 32], out_point: &OutPoint) -> Result<()> {
        self.add_to_index(CF_TYPE_INDEX, type_hash, out_point)
    }

    /// Remove a Cell from lock script index
    pub fn remove_lock(&self, lock_hash: &[u8; 32], out_point: &OutPoint) -> Result<()> {
        self.remove_from_index(CF_LOCK_INDEX, lock_hash, out_point)
    }

    /// Remove a Cell from type script index
    pub fn remove_type(&self, type_hash: &[u8; 32], out_point: &OutPoint) -> Result<()> {
        self.remove_from_index(CF_TYPE_INDEX, type_hash, out_point)
    }

    /// Get all OutPoints for a lock script
    pub fn get_by_lock(&self, lock_hash: &[u8; 32]) -> Result<Vec<OutPoint>> {
        self.get_from_index(CF_LOCK_INDEX, lock_hash)
    }

    /// Get all OutPoints for a type script
    pub fn get_by_type(&self, type_hash: &[u8; 32]) -> Result<Vec<OutPoint>> {
        self.get_from_index(CF_TYPE_INDEX, type_hash)
    }

    // Internal implementation

    fn add_to_index(&self, cf_name: &str, script_hash: &[u8; 32], out_point: &OutPoint) -> Result<()> {
        let _lock = self.write_lock.write();

        let cf = self.db.cf_handle(cf_name).ok_or_else(|| StateError::Database(format!("{} not found", cf_name)))?;

        // Get existing OutPoints
        let mut out_points = match self.db.get_cf(&cf, script_hash).map_err(|e| StateError::Database(e.to_string()))? {
            Some(data) => self.deserialize_outpoints(&data)?,
            None => BTreeSet::new(),
        };

        // Add new OutPoint
        out_points.insert(out_point.clone());

        // Serialize and store
        let value = self.serialize_outpoints(&out_points)?;
        self.db.put_cf(&cf, script_hash, &value).map_err(|e| StateError::Database(e.to_string()))?;

        Ok(())
    }

    fn remove_from_index(&self, cf_name: &str, script_hash: &[u8; 32], out_point: &OutPoint) -> Result<()> {
        let _lock = self.write_lock.write();

        let cf = self.db.cf_handle(cf_name).ok_or_else(|| StateError::Database(format!("{} not found", cf_name)))?;

        // Get existing OutPoints
        let mut out_points = match self.db.get_cf(&cf, script_hash).map_err(|e| StateError::Database(e.to_string()))? {
            Some(data) => self.deserialize_outpoints(&data)?,
            None => return Ok(()), // Already removed or never existed
        };

        // Remove OutPoint
        out_points.remove(out_point);

        if out_points.is_empty() {
            // Delete key if no more OutPoints
            self.db.delete_cf(&cf, script_hash).map_err(|e| StateError::Database(e.to_string()))?;
        } else {
            // Update with remaining OutPoints
            let value = self.serialize_outpoints(&out_points)?;
            self.db.put_cf(&cf, script_hash, &value).map_err(|e| StateError::Database(e.to_string()))?;
        }

        Ok(())
    }

    fn get_from_index(&self, cf_name: &str, script_hash: &[u8; 32]) -> Result<Vec<OutPoint>> {
        let cf = self.db.cf_handle(cf_name).ok_or_else(|| StateError::Database(format!("{} not found", cf_name)))?;

        match self.db.get_cf(&cf, script_hash).map_err(|e| StateError::Database(e.to_string()))? {
            Some(data) => {
                let set = self.deserialize_outpoints(&data)?;
                Ok(set.into_iter().collect())
            }
            None => Ok(Vec::new()),
        }
    }

    // Serialization helpers

    fn serialize_outpoints(&self, outpoints: &BTreeSet<OutPoint>) -> Result<Vec<u8>> {
        let mut buf = Vec::new();
        buf.extend_from_slice(&(outpoints.len() as u32).to_le_bytes());

        for op in outpoints {
            buf.extend_from_slice(&op.to_key());
        }

        Ok(buf)
    }

    fn deserialize_outpoints(&self, data: &[u8]) -> Result<BTreeSet<OutPoint>> {
        if data.len() < 4 {
            return Err(StateError::Serialization("Invalid data length".to_string()));
        }

        let count = u32::from_le_bytes([data[0], data[1], data[2], data[3]]) as usize;
        let expected_len = 4 + count * 36;

        if data.len() != expected_len {
            return Err(StateError::Serialization(format!("Expected {} bytes, got {}", expected_len, data.len())));
        }

        let mut set = BTreeSet::new();
        for i in 0..count {
            let offset = 4 + i * 36;
            let mut key = [0u8; 36];
            key.copy_from_slice(&data[offset..offset + 36]);
            set.insert(OutPoint::from_key(&key));
        }

        Ok(set)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::TempDir;

    #[test]
    fn test_script_index_open() {
        let tmp = TempDir::new().unwrap();
        let _index = ScriptIndex::open(tmp.path()).unwrap();
    }

    #[test]
    fn test_add_get_lock() {
        let tmp = TempDir::new().unwrap();
        let index = ScriptIndex::open(tmp.path()).unwrap();

        let lock_hash = [0x42; 32];
        let op1 = OutPoint::new([0x01; 32], 0);
        let op2 = OutPoint::new([0x02; 32], 0);

        index.add_lock(&lock_hash, &op1).unwrap();
        index.add_lock(&lock_hash, &op2).unwrap();

        let result = index.get_by_lock(&lock_hash).unwrap();
        assert_eq!(result.len(), 2);
        assert!(result.contains(&op1));
        assert!(result.contains(&op2));
    }

    #[test]
    fn test_remove_lock() {
        let tmp = TempDir::new().unwrap();
        let index = ScriptIndex::open(tmp.path()).unwrap();

        let lock_hash = [0x42; 32];
        let op1 = OutPoint::new([0x01; 32], 0);
        let op2 = OutPoint::new([0x02; 32], 0);

        index.add_lock(&lock_hash, &op1).unwrap();
        index.add_lock(&lock_hash, &op2).unwrap();

        index.remove_lock(&lock_hash, &op1).unwrap();

        let result = index.get_by_lock(&lock_hash).unwrap();
        assert_eq!(result.len(), 1);
        assert!(result.contains(&op2));
    }

    #[test]
    fn test_type_script_index() {
        let tmp = TempDir::new().unwrap();
        let index = ScriptIndex::open(tmp.path()).unwrap();

        let type_hash = [0x99; 32];
        let op = OutPoint::new([0x01; 32], 0);

        index.add_type(&type_hash, &op).unwrap();

        let result = index.get_by_type(&type_hash).unwrap();
        assert_eq!(result.len(), 1);
        assert_eq!(result[0], op);

        index.remove_type(&type_hash, &op).unwrap();

        let result = index.get_by_type(&type_hash).unwrap();
        assert_eq!(result.len(), 0);
    }
}
