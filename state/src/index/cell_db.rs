// SPDX-License-Identifier: MIT
// Copyright (C) 2026 Spora developers
//
// CellDB: Cell indexing database (OutPoint → CellMeta)

use crate::{Result, StateError};
use borsh::{BorshDeserialize, BorshSerialize};
use parking_lot::RwLock;
use rocksdb::{ColumnFamilyDescriptor, IteratorMode, Options, WriteBatch, DB};
use serde::{Deserialize, Serialize};
use spora_exec::{CellOutput, OutPoint};
use std::path::Path;
use std::sync::Arc;

/// Column families
const CF_CELLS: &str = "cells";
const CF_SPENT: &str = "spent";
const CF_SPEND_JOURNAL: &str = "spend_journal"; // Full metadata for historical queries

/// Cell metadata (stored in CellDB)
///
/// Maps OutPoint → CellMeta for quick Cell lookups
#[derive(Clone, Debug, PartialEq, Eq, BorshSerialize, BorshDeserialize, Serialize, Deserialize)]
pub struct CellMeta {
    /// Cell output structure
    pub cell_output: CellOutput,
    /// Cell data (may be large, consider storing separately in DA layer)
    pub cell_data: Vec<u8>,
    /// DAA score at creation
    pub daa_score: u64,
    /// Block hash containing this Cell
    pub block_hash: [u8; 32],
    /// Is this a cellbase?
    pub is_cellbase: bool,
    /// DA segment info (optional)
    pub segment_info: Option<SegmentInfo>,
}

/// Spend record for historical queries
///
/// Stores both when a Cell was spent and its original metadata.
///
/// `spent_in_block` is the primary branch-aware anchor. `spent_at_daa` is kept
/// only as auxiliary/indexing data and must not be used as a consensus POV.
#[derive(Clone, Debug, PartialEq, Eq, BorshSerialize, BorshDeserialize)]
pub struct SpendRecord {
    /// Block hash where the Cell was spent.
    pub spent_in_block: [u8; 32],
    /// DAA score observed when the Cell was spent.
    pub spent_at_daa: u64,
    /// Original Cell metadata (for historical reconstruction)
    pub cell_meta: CellMeta,
}

/// Segment storage information
#[derive(Clone, Debug, PartialEq, Eq, BorshSerialize, BorshDeserialize, Serialize, Deserialize)]
pub struct SegmentInfo {
    /// Segment ID
    pub segment_id: u32,
    /// Offset within segment
    pub offset: u64,
    /// Length of data
    pub length: u32,
}

/// Cell database
///
/// Responsibilities:
/// - Store live Cells (OutPoint → CellMeta)
/// - Track spent Cells (OutPoint → DAA score)
/// - Support efficient queries
pub struct CellDB {
    /// RocksDB instance
    db: Arc<DB>,
    /// Write lock for atomic updates
    write_lock: Arc<RwLock<()>>,
}

impl CellDB {
    fn normalize_meta_for_storage(meta: &CellMeta) -> CellMeta {
        let mut normalized = meta.clone();
        if normalized.segment_info.is_some() {
            normalized.cell_data.clear();
        }
        normalized
    }

    /// Open or create a CellDB
    pub fn open<P: AsRef<Path>>(path: P) -> Result<Self> {
        let mut opts = Options::default();
        opts.create_if_missing(true);
        opts.create_missing_column_families(true);
        opts.set_compression_type(rocksdb::DBCompressionType::Snappy);
        opts.increase_parallelism(4);

        // Define column families
        let cf_cells = ColumnFamilyDescriptor::new(CF_CELLS, Options::default());
        let cf_spent = ColumnFamilyDescriptor::new(CF_SPENT, Options::default());
        let cf_spend_journal = ColumnFamilyDescriptor::new(CF_SPEND_JOURNAL, Options::default());

        let db = DB::open_cf_descriptors(&opts, path, vec![cf_cells, cf_spent, cf_spend_journal])
            .map_err(|e| StateError::Database(e.to_string()))?;

        Ok(Self { db: Arc::new(db), write_lock: Arc::new(RwLock::new(())) })
    }

    /// Get a Cell by OutPoint
    ///
    /// Returns:
    /// - Some(CellMeta) if Cell is live
    /// - None if Cell is spent or doesn't exist
    pub fn get(&self, out_point: &OutPoint) -> Result<Option<CellMeta>> {
        let cf = self.db.cf_handle(CF_CELLS).ok_or_else(|| StateError::Database("CF_CELLS not found".to_string()))?;

        let key = out_point.to_key();

        match self.db.get_cf(&cf, &key).map_err(|e| StateError::Database(e.to_string()))? {
            Some(data) => {
                let meta = CellMeta::try_from_slice(&data).map_err(|e| StateError::Serialization(e.to_string()))?;
                Ok(Some(meta))
            }
            None => Ok(None),
        }
    }

    /// Put a new Cell
    ///
    /// Adds a Cell to the live set
    pub fn put(&self, out_point: &OutPoint, meta: &CellMeta) -> Result<()> {
        let _lock = self.write_lock.write();

        let cf = self.db.cf_handle(CF_CELLS).ok_or_else(|| StateError::Database("CF_CELLS not found".to_string()))?;
        let cf_spent = self.db.cf_handle(CF_SPENT).ok_or_else(|| StateError::Database("CF_SPENT not found".to_string()))?;
        let cf_journal =
            self.db.cf_handle(CF_SPEND_JOURNAL).ok_or_else(|| StateError::Database("CF_SPEND_JOURNAL not found".to_string()))?;

        let key = out_point.to_key();
        let normalized = Self::normalize_meta_for_storage(meta);
        let value = borsh::to_vec(&normalized).map_err(|e| StateError::Serialization(e.to_string()))?;

        let mut batch = WriteBatch::default();
        // Re-adding a live cell after a reorg must clear the previous canonical spend marker.
        batch.delete_cf(&cf_spent, &key);
        batch.delete_cf(&cf_journal, &key);
        batch.put_cf(&cf, &key, &value);

        self.db.write(batch).map_err(|e| StateError::Database(e.to_string()))?;

        Ok(())
    }

    /// Remove a live Cell from the live set without writing spent history.
    ///
    /// This exists for secondary-index maintenance paths that only receive net
    /// virtual diffs and therefore do not know the canonical spending block.
    /// Consensus and canonical historical journaling must use
    /// [`CellDB::spend_in_block`].
    pub fn remove_live_cell(&self, out_point: &OutPoint) -> Result<Option<CellMeta>> {
        let _lock = self.write_lock.write();

        let cf = self.db.cf_handle(CF_CELLS).ok_or_else(|| StateError::Database("CF_CELLS not found".to_string()))?;
        let key = out_point.to_key();

        let existing = self.db.get_cf(&cf, &key).map_err(|e| StateError::Database(e.to_string()))?;
        let Some(existing) = existing else {
            return Ok(None);
        };

        let meta = CellMeta::try_from_slice(&existing).map_err(|e| StateError::Serialization(e.to_string()))?;
        self.db.delete_cf(&cf, &key).map_err(|e| StateError::Database(e.to_string()))?;
        Ok(Some(meta))
    }

    /// Spend a Cell using a block-aware journal record.
    ///
    /// Moves a Cell from live set to spent set, preserving metadata for debugging
    /// and index-only historical inspection.
    pub fn spend_in_block(&self, out_point: &OutPoint, spent_at_daa: u64, spent_in_block: [u8; 32]) -> Result<()> {
        let _lock = self.write_lock.write();

        let cf_cells = self.db.cf_handle(CF_CELLS).ok_or_else(|| StateError::Database("CF_CELLS not found".to_string()))?;
        let cf_spent = self.db.cf_handle(CF_SPENT).ok_or_else(|| StateError::Database("CF_SPENT not found".to_string()))?;
        let cf_journal =
            self.db.cf_handle(CF_SPEND_JOURNAL).ok_or_else(|| StateError::Database("CF_SPEND_JOURNAL not found".to_string()))?;

        let key = out_point.to_key();

        // Get Cell metadata before deleting
        let cell_data = self
            .db
            .get_cf(&cf_cells, &key)
            .map_err(|e| StateError::Database(e.to_string()))?
            .ok_or_else(|| StateError::CellNotFound([0; 32]))?;

        let cell_meta = CellMeta::try_from_slice(&cell_data).map_err(|e| StateError::Serialization(e.to_string()))?;

        // Create spend record
        let spend_record = SpendRecord { spent_in_block, spent_at_daa, cell_meta };

        // Atomic update: delete from cells, add to spent + journal
        let mut batch = WriteBatch::default();
        batch.delete_cf(&cf_cells, &key);
        batch.put_cf(&cf_spent, &key, &spent_at_daa.to_le_bytes());
        batch.put_cf(&cf_journal, &key, &borsh::to_vec(&spend_record).map_err(|e| StateError::Serialization(e.to_string()))?);

        self.db.write(batch).map_err(|e| StateError::Database(e.to_string()))?;

        Ok(())
    }

    /// Check if a Cell is spent
    pub fn is_spent(&self, out_point: &OutPoint) -> Result<Option<u64>> {
        let cf = self.db.cf_handle(CF_SPENT).ok_or_else(|| StateError::Database("CF_SPENT not found".to_string()))?;

        let key = out_point.to_key();

        match self.db.get_cf(&cf, &key).map_err(|e| StateError::Database(e.to_string()))? {
            Some(data) => {
                if data.len() != 8 {
                    return Err(StateError::Serialization("Invalid DAA score".to_string()));
                }
                let daa = u64::from_le_bytes([data[0], data[1], data[2], data[3], data[4], data[5], data[6], data[7]]);
                Ok(Some(daa))
            }
            None => Ok(None),
        }
    }

    /// Batch put Cells (for block processing)
    pub fn batch_put(&self, cells: &[(OutPoint, CellMeta)]) -> Result<()> {
        let _lock = self.write_lock.write();

        let cf = self.db.cf_handle(CF_CELLS).ok_or_else(|| StateError::Database("CF_CELLS not found".to_string()))?;
        let cf_spent = self.db.cf_handle(CF_SPENT).ok_or_else(|| StateError::Database("CF_SPENT not found".to_string()))?;
        let cf_journal =
            self.db.cf_handle(CF_SPEND_JOURNAL).ok_or_else(|| StateError::Database("CF_SPEND_JOURNAL not found".to_string()))?;

        let mut batch = WriteBatch::default();

        for (out_point, meta) in cells {
            let key = out_point.to_key();
            let normalized = Self::normalize_meta_for_storage(meta);
            let value = borsh::to_vec(&normalized).map_err(|e| StateError::Serialization(e.to_string()))?;
            batch.delete_cf(&cf_spent, &key);
            batch.delete_cf(&cf_journal, &key);
            batch.put_cf(&cf, &key, &value);
        }

        self.db.write(batch).map_err(|e| StateError::Database(e.to_string()))?;

        Ok(())
    }

    /// Batch spend Cells using block-aware journal records.
    pub fn batch_spend_in_block(&self, spends: &[(OutPoint, u64, [u8; 32])]) -> Result<()> {
        let _lock = self.write_lock.write();

        let cf_cells = self.db.cf_handle(CF_CELLS).ok_or_else(|| StateError::Database("CF_CELLS not found".to_string()))?;
        let cf_spent = self.db.cf_handle(CF_SPENT).ok_or_else(|| StateError::Database("CF_SPENT not found".to_string()))?;
        let cf_journal =
            self.db.cf_handle(CF_SPEND_JOURNAL).ok_or_else(|| StateError::Database("CF_SPEND_JOURNAL not found".to_string()))?;

        let mut batch = WriteBatch::default();

        for (out_point, spent_at_daa, spent_in_block) in spends {
            let key = out_point.to_key();

            // Get Cell metadata before deleting
            if let Some(cell_data) = self.db.get_cf(&cf_cells, &key).map_err(|e| StateError::Database(e.to_string()))? {
                let cell_meta = CellMeta::try_from_slice(&cell_data).map_err(|e| StateError::Serialization(e.to_string()))?;

                // Create spend record
                let spend_record = SpendRecord { spent_in_block: *spent_in_block, spent_at_daa: *spent_at_daa, cell_meta };

                batch.delete_cf(&cf_cells, &key);
                batch.put_cf(&cf_spent, &key, &spent_at_daa.to_le_bytes());
                batch.put_cf(&cf_journal, &key, &borsh::to_vec(&spend_record).map_err(|e| StateError::Serialization(e.to_string()))?);
            }
        }

        self.db.write(batch).map_err(|e| StateError::Database(e.to_string()))?;

        Ok(())
    }

    /// Get Cell state at a specific DAA score.
    ///
    /// # WARNING: Non-consensus index helper
    ///
    /// DAA score does not uniquely identify a DAG history point-of-view.
    /// This method MUST NOT be used for consensus validation, reorg logic,
    /// or double-spend decisions.
    ///
    /// For consensus-safe queries, use [`get_cell_snapshot_at_pov`] or
    /// [`batch_get_cell_snapshots_at_pov`] which are anchored by block hash.
    ///
    /// Logic:
    /// - Cell must have been created at or before `at_daa`
    /// - Cell must be either:
    ///   a) Still live (in CF_CELLS), OR
    ///   b) Spent after `at_daa` (in CF_SPEND_JOURNAL with spent_at_daa > at_daa)
    ///
    /// Correct consensus queries must be anchored by block hash / POV, for
    /// example `get_cell_at_pov(outpoint, block_hash)`.
    #[deprecated(
        since = "0.2.0",
        note = "Use get_cell_snapshot_at_pov() for consensus queries. This DAA-based method is retained only for index/debug purposes."
    )]
    pub fn get_cell_snapshot_at_daa(&self, out_point: &OutPoint, at_daa: u64) -> Result<Option<CellMeta>> {
        let cf_cells = self.db.cf_handle(CF_CELLS).ok_or_else(|| StateError::Database("CF_CELLS not found".to_string()))?;
        let cf_journal =
            self.db.cf_handle(CF_SPEND_JOURNAL).ok_or_else(|| StateError::Database("CF_SPEND_JOURNAL not found".to_string()))?;

        let key = out_point.to_key();

        // CASE 1: Cell is currently live
        if let Some(data) = self.db.get_cf(&cf_cells, &key).map_err(|e| StateError::Database(e.to_string()))? {
            let meta = CellMeta::try_from_slice(&data).map_err(|e| StateError::Serialization(e.to_string()))?;

            // Cell exists and is live. Was it created by at_daa?
            if meta.daa_score <= at_daa {
                return Ok(Some(meta));
            } else {
                // Cell was created after at_daa, so it didn't exist yet
                return Ok(None);
            }
        }

        // CASE 2: Cell has been spent - check spend journal
        if let Some(journal_data) = self.db.get_cf(&cf_journal, &key).map_err(|e| StateError::Database(e.to_string()))? {
            let spend_record = SpendRecord::try_from_slice(&journal_data).map_err(|e| StateError::Serialization(e.to_string()))?;

            // Check creation and spend DAA scores
            let created_at = spend_record.cell_meta.daa_score;
            let spent_at = spend_record.spent_at_daa;

            // Cell was live if: created_at <= at_daa < spent_at
            if created_at <= at_daa && spent_at > at_daa {
                return Ok(Some(spend_record.cell_meta));
            } else {
                // Either not yet created or already spent
                return Ok(None);
            }
        }

        // CASE 3: Cell doesn't exist in any index
        Ok(None)
    }

    /// Get Cell state from the canonical journal using an explicit POV block.
    ///
    /// The caller supplies the DAG-specific ancestry predicate that decides
    /// whether `block_hash` is included in the history visible from `pov`.
    /// This keeps `CellDB` free of consensus dependencies while still enabling
    /// branch-aware historical queries.
    pub fn get_cell_snapshot_at_pov<F>(
        &self,
        out_point: &OutPoint,
        pov: [u8; 32],
        mut block_in_pov_history: F,
    ) -> Result<Option<CellMeta>>
    where
        F: FnMut([u8; 32], [u8; 32]) -> Result<bool>,
    {
        let cf_cells = self.db.cf_handle(CF_CELLS).ok_or_else(|| StateError::Database("CF_CELLS not found".to_string()))?;
        let cf_journal =
            self.db.cf_handle(CF_SPEND_JOURNAL).ok_or_else(|| StateError::Database("CF_SPEND_JOURNAL not found".to_string()))?;

        let key = out_point.to_key();

        if let Some(data) = self.db.get_cf(&cf_cells, &key).map_err(|e| StateError::Database(e.to_string()))? {
            let meta = CellMeta::try_from_slice(&data).map_err(|e| StateError::Serialization(e.to_string()))?;
            return if block_in_pov_history(meta.block_hash, pov)? { Ok(Some(meta)) } else { Ok(None) };
        }

        if let Some(journal_data) = self.db.get_cf(&cf_journal, &key).map_err(|e| StateError::Database(e.to_string()))? {
            let spend_record = SpendRecord::try_from_slice(&journal_data).map_err(|e| StateError::Serialization(e.to_string()))?;

            let created_visible = block_in_pov_history(spend_record.cell_meta.block_hash, pov)?;
            if !created_visible {
                return Ok(None);
            }

            let spend_visible = block_in_pov_history(spend_record.spent_in_block, pov)?;
            return if spend_visible { Ok(None) } else { Ok(Some(spend_record.cell_meta)) };
        }

        Ok(None)
    }

    /// Batch query Cells at a specific DAA score for index/debug use.
    ///
    /// # WARNING: Non-consensus index helper
    ///
    /// DAA score does not uniquely identify a DAG history point-of-view.
    /// This method MUST NOT be used for consensus validation, reorg logic,
    /// or double-spend decisions.
    ///
    /// For consensus-safe queries, use [`get_cell_snapshot_at_pov`] or
    /// [`batch_get_cell_snapshots_at_pov`] which are anchored by block hash.
    #[deprecated(
        since = "0.2.0",
        note = "Use batch_get_cell_snapshots_at_pov() for consensus queries. This DAA-based method is retained only for index/debug purposes."
    )]
    #[allow(deprecated)]
    pub fn batch_get_cell_snapshots_at_daa(&self, out_points: &[OutPoint], at_daa: u64) -> Result<Vec<Option<CellMeta>>> {
        let mut results = Vec::with_capacity(out_points.len());

        for out_point in out_points {
            results.push(self.get_cell_snapshot_at_daa(out_point, at_daa)?);
        }

        Ok(results)
    }

    /// Batch query Cells from the canonical journal using an explicit POV block.
    pub fn batch_get_cell_snapshots_at_pov<F>(
        &self,
        out_points: &[OutPoint],
        pov: [u8; 32],
        mut block_in_pov_history: F,
    ) -> Result<Vec<Option<CellMeta>>>
    where
        F: FnMut([u8; 32], [u8; 32]) -> Result<bool>,
    {
        let mut results = Vec::with_capacity(out_points.len());

        for out_point in out_points {
            results.push(self.get_cell_snapshot_at_pov(out_point, pov, &mut block_in_pov_history)?);
        }

        Ok(results)
    }

    /// Get database statistics
    pub fn stats(&self) -> Result<CellDBStats> {
        // Note: RocksDB's estimate_num_keys is approximate
        let cf_cells = self.db.cf_handle(CF_CELLS).ok_or_else(|| StateError::Database("CF_CELLS not found".to_string()))?;
        let cf_spent = self.db.cf_handle(CF_SPENT).ok_or_else(|| StateError::Database("CF_SPENT not found".to_string()))?;

        // Use property queries (RocksDB internal stats)
        let live_cells = self
            .db
            .property_int_value_cf(&cf_cells, "rocksdb.estimate-num-keys")
            .map_err(|e| StateError::Database(e.to_string()))?
            .unwrap_or(0);

        let spent_cells = self
            .db
            .property_int_value_cf(&cf_spent, "rocksdb.estimate-num-keys")
            .map_err(|e| StateError::Database(e.to_string()))?
            .unwrap_or(0);

        Ok(CellDBStats { live_cells, spent_cells })
    }

    /// Return the total capacity currently held by live cells.
    pub fn total_live_capacity(&self) -> Result<u64> {
        let cf_cells = self.db.cf_handle(CF_CELLS).ok_or_else(|| StateError::Database("CF_CELLS not found".to_string()))?;
        let mut total = 0u64;

        for item in self.db.iterator_cf(&cf_cells, IteratorMode::Start) {
            let (_key, value) = item.map_err(|e| StateError::Database(e.to_string()))?;
            let meta = CellMeta::try_from_slice(&value).map_err(|e| StateError::Serialization(e.to_string()))?;
            total = total.saturating_add(meta.cell_output.capacity);
        }

        Ok(total)
    }

    /// Returns true if the live cell set currently contains at least one cell.
    pub fn has_live_cells(&self) -> Result<bool> {
        let cf_cells = self.db.cf_handle(CF_CELLS).ok_or_else(|| StateError::Database("CF_CELLS not found".to_string()))?;
        let mut iter = self.db.iterator_cf(&cf_cells, IteratorMode::Start);
        Ok(match iter.next() {
            Some(item) => {
                item.map_err(|e| StateError::Database(e.to_string()))?;
                true
            }
            None => false,
        })
    }
}

/// CellDB statistics
#[derive(Debug, Clone, Copy)]
pub struct CellDBStats {
    /// Number of live Cells
    pub live_cells: u64,
    /// Number of spent Cells
    pub spent_cells: u64,
}

#[cfg(test)]
mod tests {
    use super::*;
    use spora_exec::{CellOutput, Script};
    use tempfile::TempDir;

    fn create_test_cell_meta(capacity: u64, daa: u64) -> CellMeta {
        let lock = Script::new([0x00; 32], 0, vec![0; 20]);
        CellMeta {
            cell_output: CellOutput { lock, type_: None, capacity },
            cell_data: vec![0xAA; 100],
            daa_score: daa,
            block_hash: [0x11; 32],
            is_cellbase: false,
            segment_info: None,
        }
    }

    fn create_segment_backed_cell_meta(capacity: u64, daa: u64) -> CellMeta {
        let mut meta = create_test_cell_meta(capacity, daa);
        meta.segment_info = Some(SegmentInfo { segment_id: 3, offset: 128, length: meta.cell_data.len() as u32 });
        meta
    }

    #[test]
    fn test_cell_db_open() {
        let tmp = TempDir::new().unwrap();
        let db = CellDB::open(tmp.path()).unwrap();
        let stats = db.stats().unwrap();
        assert_eq!(stats.live_cells, 0);
        assert_eq!(stats.spent_cells, 0);
    }

    #[test]
    fn test_put_get_cell() {
        let tmp = TempDir::new().unwrap();
        let db = CellDB::open(tmp.path()).unwrap();

        let out_point = OutPoint::new([0x42; 32], 0);
        let meta = create_test_cell_meta(1000, 100);

        db.put(&out_point, &meta).unwrap();

        let retrieved = db.get(&out_point).unwrap().unwrap();
        assert_eq!(retrieved, meta);
    }

    #[test]
    fn test_put_clears_stale_spent_state_when_cell_becomes_live_again() {
        let tmp = TempDir::new().unwrap();
        let db = CellDB::open(tmp.path()).unwrap();

        let out_point = OutPoint::new([0x42; 32], 7);
        let meta = create_test_cell_meta(1000, 100);

        db.put(&out_point, &meta).unwrap();
        db.spend_in_block(&out_point, 150, [0x99; 32]).unwrap();
        assert_eq!(db.is_spent(&out_point).unwrap(), Some(150));

        db.put(&out_point, &meta).unwrap();

        assert_eq!(db.is_spent(&out_point).unwrap(), None);
        assert_eq!(db.get(&out_point).unwrap(), Some(meta));
    }

    #[test]
    fn test_put_strips_cell_data_when_segment_info_present() {
        let tmp = TempDir::new().unwrap();
        let db = CellDB::open(tmp.path()).unwrap();

        let out_point = OutPoint::new([0x52; 32], 0);
        let meta = create_segment_backed_cell_meta(1000, 100);

        db.put(&out_point, &meta).unwrap();

        let retrieved = db.get(&out_point).unwrap().unwrap();
        assert!(retrieved.cell_data.is_empty());
        assert_eq!(retrieved.segment_info, meta.segment_info);
    }

    #[test]
    fn test_spend_cell() {
        let tmp = TempDir::new().unwrap();
        let db = CellDB::open(tmp.path()).unwrap();

        let out_point = OutPoint::new([0x42; 32], 0);
        let meta = create_test_cell_meta(1000, 100);

        db.put(&out_point, &meta).unwrap();
        db.spend_in_block(&out_point, 200, [0; 32]).unwrap();

        // Cell should no longer be in live set
        assert!(db.get(&out_point).unwrap().is_none());

        // Cell should be marked as spent
        assert_eq!(db.is_spent(&out_point).unwrap(), Some(200));
    }

    #[test]
    fn test_remove_live_cell_does_not_create_spent_marker() {
        let tmp = TempDir::new().unwrap();
        let db = CellDB::open(tmp.path()).unwrap();

        let out_point = OutPoint::new([0x55; 32], 0);
        let meta = create_test_cell_meta(1234, 99);
        db.put(&out_point, &meta).unwrap();

        let removed = db.remove_live_cell(&out_point).unwrap();
        assert_eq!(removed, Some(meta));
        assert!(db.get(&out_point).unwrap().is_none());
        assert_eq!(db.is_spent(&out_point).unwrap(), None);
    }

    #[test]
    fn test_spend_journal_records_spending_block_hash() {
        let tmp = TempDir::new().unwrap();
        let db = CellDB::open(tmp.path()).unwrap();

        let out_point = OutPoint::new([0x42; 32], 1);
        let meta = create_test_cell_meta(1000, 100);
        let spending_block = [0x77; 32];

        db.put(&out_point, &meta).unwrap();
        db.spend_in_block(&out_point, 200, spending_block).unwrap();

        let cf_journal = db.db.cf_handle(CF_SPEND_JOURNAL).unwrap();
        let journal_data = db.db.get_cf(&cf_journal, out_point.to_key()).unwrap().unwrap();
        let spend_record = SpendRecord::try_from_slice(&journal_data).unwrap();

        assert_eq!(spend_record.spent_in_block, spending_block);
        assert_eq!(spend_record.spent_at_daa, 200);
        assert_eq!(spend_record.cell_meta, meta);
    }

    #[test]
    fn test_batch_operations() {
        let tmp = TempDir::new().unwrap();
        let db = CellDB::open(tmp.path()).unwrap();

        let cells = vec![
            (OutPoint::new([0x01; 32], 0), create_test_cell_meta(1000, 100)),
            (OutPoint::new([0x02; 32], 0), create_test_cell_meta(2000, 101)),
            (OutPoint::new([0x03; 32], 0), create_test_cell_meta(3000, 102)),
        ];

        db.batch_put(&cells).unwrap();

        // Verify all Cells are stored
        for (out_point, meta) in &cells {
            let retrieved = db.get(out_point).unwrap().unwrap();
            assert_eq!(&retrieved, meta);
        }

        // Spend first two Cells
        let spends = vec![(OutPoint::new([0x01; 32], 0), 200), (OutPoint::new([0x02; 32], 0), 201)];

        let spends_with_block = spends.iter().map(|(out_point, daa)| (out_point.clone(), *daa, [0; 32])).collect::<Vec<_>>();
        db.batch_spend_in_block(&spends_with_block).unwrap();

        // Verify spends
        assert!(db.get(&OutPoint::new([0x01; 32], 0)).unwrap().is_none());
        assert!(db.get(&OutPoint::new([0x02; 32], 0)).unwrap().is_none());
        assert!(db.get(&OutPoint::new([0x03; 32], 0)).unwrap().is_some());
    }

    #[test]
    fn test_batch_put_strips_cell_data_when_segment_info_present() {
        let tmp = TempDir::new().unwrap();
        let db = CellDB::open(tmp.path()).unwrap();

        let cells = vec![
            (OutPoint::new([0x21; 32], 0), create_segment_backed_cell_meta(1000, 100)),
            (OutPoint::new([0x22; 32], 0), create_test_cell_meta(2000, 101)),
        ];

        db.batch_put(&cells).unwrap();

        let segment_backed = db.get(&cells[0].0).unwrap().unwrap();
        assert!(segment_backed.cell_data.is_empty());
        assert_eq!(segment_backed.segment_info, cells[0].1.segment_info);

        let inline = db.get(&cells[1].0).unwrap().unwrap();
        assert_eq!(inline.cell_data, cells[1].1.cell_data);
        assert_eq!(inline.segment_info, None);
    }

    #[test]
    #[allow(deprecated)]
    fn test_get_cell_at_daa_live_cell() {
        let temp_dir = TempDir::new().unwrap();
        let db = CellDB::open(temp_dir.path()).unwrap();

        let out_point = OutPoint::new([1; 32], 0);
        let meta = create_test_cell_meta(1000, 50); // Created at DAA 50

        db.put(&out_point, &meta).unwrap();

        // Query before creation: should be None
        assert_eq!(db.get_cell_snapshot_at_daa(&out_point, 40).unwrap(), None);

        // Query at creation: should be Some
        assert_eq!(db.get_cell_snapshot_at_daa(&out_point, 50).unwrap(), Some(meta.clone()));

        // Query after creation: should be Some (still live)
        assert_eq!(db.get_cell_snapshot_at_daa(&out_point, 100).unwrap(), Some(meta));
    }

    #[test]
    #[allow(deprecated)]
    fn test_get_cell_at_daa_spent_cell() {
        let temp_dir = TempDir::new().unwrap();
        let db = CellDB::open(temp_dir.path()).unwrap();

        let out_point = OutPoint::new([2; 32], 0);
        let meta = create_test_cell_meta(1000, 50); // Created at DAA 50

        db.put(&out_point, &meta).unwrap();
        db.spend_in_block(&out_point, 150, [0; 32]).unwrap(); // Spent at DAA 150

        // Query before creation: None
        assert_eq!(db.get_cell_snapshot_at_daa(&out_point, 40).unwrap(), None);

        // Query when live (50 <= 100 < 150): Some
        let result = db.get_cell_snapshot_at_daa(&out_point, 100).unwrap();
        assert!(result.is_some());
        assert_eq!(result.unwrap().cell_output.capacity, 1000);

        // Query at spend point (DAA 150): None (just spent)
        assert_eq!(db.get_cell_snapshot_at_daa(&out_point, 150).unwrap(), None);

        // Query after spend: None
        assert_eq!(db.get_cell_snapshot_at_daa(&out_point, 200).unwrap(), None);
    }

    #[test]
    #[allow(deprecated)]
    fn test_get_cell_at_daa_reorg_scenario() {
        let temp_dir = TempDir::new().unwrap();
        let db = CellDB::open(temp_dir.path()).unwrap();

        // Simulate reorg scenario:
        // Branch A: Cell created at DAA 50, spent at DAA 150
        // Branch B: Need to validate tx at DAA 100 (Cell should be live)

        let out_point = OutPoint::new([3; 32], 0);
        let meta = create_test_cell_meta(1000, 50);

        db.put(&out_point, &meta).unwrap();
        db.spend_in_block(&out_point, 150, [0; 32]).unwrap();

        // Reorg validation: Check if Cell was live at DAA 100
        let cell_at_100 = db.get_cell_snapshot_at_daa(&out_point, 100).unwrap();
        let cell_at_100 = cell_at_100.expect("cell should be live at daa 100");
        assert_eq!(cell_at_100.cell_output.capacity, 1000);
        assert_eq!(cell_at_100.daa_score, 50);
    }

    #[test]
    #[allow(deprecated)]
    fn test_get_cell_at_daa_fork_scenario() {
        let temp_dir = TempDir::new().unwrap();
        let db = CellDB::open(temp_dir.path()).unwrap();

        // Fork scenario:
        // Same Cell spent in different branches at different DAA scores
        // Cell created at DAA 50
        // Branch A: spent at DAA 120
        // Branch B: need to check at DAA 100 (should be live)

        let out_point = OutPoint::new([4; 32], 0);
        let meta = create_test_cell_meta(2000, 50);

        db.put(&out_point, &meta).unwrap();
        db.spend_in_block(&out_point, 120, [0; 32]).unwrap();

        // Query at DAA 100: Cell should be live (50 <= 100 < 120)
        let result = db.get_cell_snapshot_at_daa(&out_point, 100).unwrap();
        assert!(result.is_some());

        // Query at DAA 130: Cell should be spent (130 >= 120)
        let result = db.get_cell_snapshot_at_daa(&out_point, 130).unwrap();
        assert!(result.is_none());
    }

    #[test]
    #[allow(deprecated)]
    fn test_batch_get_at_daa() {
        let temp_dir = TempDir::new().unwrap();
        let db = CellDB::open(temp_dir.path()).unwrap();

        let out1 = OutPoint::new([6; 32], 0);
        let out2 = OutPoint::new([7; 32], 0);
        let out3 = OutPoint::new([8; 32], 0);

        let meta1 = create_test_cell_meta(1000, 50); // Created at 50
        let meta2 = create_test_cell_meta(2000, 60); // Created at 60
        let meta3 = create_test_cell_meta(3000, 70); // Created at 70

        db.put(&out1, &meta1).unwrap();
        db.put(&out2, &meta2).unwrap();
        db.put(&out3, &meta3).unwrap();

        // Spend meta2 at DAA 100
        db.spend_in_block(&out2, 100, [0; 32]).unwrap();

        // Batch query at DAA 80
        let results = db.batch_get_cell_snapshots_at_daa(&[out1.clone(), out2.clone(), out3.clone()], 80).unwrap();

        assert!(results[0].is_some()); // meta1: created at 50, still live
        assert!(results[1].is_some()); // meta2: created at 60, spent at 100 (live at 80)
        assert!(results[2].is_some()); // meta3: created at 70, still live
    }

    #[test]
    fn test_get_cell_at_pov_live_cell_requires_creation_in_visible_history() {
        let temp_dir = TempDir::new().unwrap();
        let db = CellDB::open(temp_dir.path()).unwrap();

        let out_point = OutPoint::new([9; 32], 0);
        let creation_block = [0x11; 32];
        let visible_pov = [0xAA; 32];
        let hidden_pov = [0xBB; 32];
        let mut meta = create_test_cell_meta(1000, 50);
        meta.block_hash = creation_block;

        db.put(&out_point, &meta).unwrap();

        let visible =
            db.get_cell_snapshot_at_pov(&out_point, visible_pov, |block_hash, pov| {
                Ok(pov == visible_pov && block_hash == creation_block)
            })
            .unwrap();
        assert_eq!(visible, Some(meta.clone()));

        let hidden = db
            .get_cell_snapshot_at_pov(&out_point, hidden_pov, |block_hash, pov| Ok(pov == visible_pov && block_hash == creation_block))
            .unwrap();
        assert_eq!(hidden, None);
    }

    #[test]
    fn test_get_cell_at_pov_spent_cell_uses_branch_visibility() {
        let temp_dir = TempDir::new().unwrap();
        let db = CellDB::open(temp_dir.path()).unwrap();

        let out_point = OutPoint::new([0x31; 32], 0);
        let creation_block = [0x41; 32];
        let spend_block = [0x51; 32];
        let pov_before_spend = [0x61; 32];
        let pov_after_spend = [0x71; 32];
        let unrelated_pov = [0x81; 32];
        let mut meta = create_test_cell_meta(1000, 50);
        meta.block_hash = creation_block;

        db.put(&out_point, &meta).unwrap();
        db.spend_in_block(&out_point, 150, spend_block).unwrap();

        let before_spend = db
            .get_cell_snapshot_at_pov(&out_point, pov_before_spend, |block_hash, pov| {
                Ok(match pov {
                    p if p == pov_before_spend => block_hash == creation_block,
                    p if p == pov_after_spend => block_hash == creation_block || block_hash == spend_block,
                    _ => false,
                })
            })
            .unwrap();
        assert_eq!(before_spend, Some(meta.clone()));

        let after_spend = db
            .get_cell_snapshot_at_pov(&out_point, pov_after_spend, |block_hash, pov| {
                Ok(match pov {
                    p if p == pov_before_spend => block_hash == creation_block,
                    p if p == pov_after_spend => block_hash == creation_block || block_hash == spend_block,
                    _ => false,
                })
            })
            .unwrap();
        assert_eq!(after_spend, None);

        let unrelated = db
            .get_cell_snapshot_at_pov(&out_point, unrelated_pov, |block_hash, pov| {
                Ok(match pov {
                    p if p == pov_before_spend => block_hash == creation_block,
                    p if p == pov_after_spend => block_hash == creation_block || block_hash == spend_block,
                    _ => false,
                })
            })
            .unwrap();
        assert_eq!(unrelated, None);
    }

    #[test]
    fn test_batch_get_at_pov() {
        let temp_dir = TempDir::new().unwrap();
        let db = CellDB::open(temp_dir.path()).unwrap();

        let pov = [0xD1; 32];
        let live_block = [0xD2; 32];
        let hidden_block = [0xD3; 32];

        let out1 = OutPoint::new([0x91; 32], 0);
        let out2 = OutPoint::new([0x92; 32], 0);

        let mut meta1 = create_test_cell_meta(1000, 50);
        meta1.block_hash = live_block;
        let mut meta2 = create_test_cell_meta(2000, 60);
        meta2.block_hash = hidden_block;

        db.put(&out1, &meta1).unwrap();
        db.put(&out2, &meta2).unwrap();

        let results = db
            .batch_get_cell_snapshots_at_pov(&[out1.clone(), out2.clone()], pov, |block_hash, query_pov| {
                Ok(query_pov == pov && block_hash == live_block)
            })
            .unwrap();

        assert_eq!(results, vec![Some(meta1), None]);
    }
}
