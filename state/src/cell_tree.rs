// SPDX-License-Identifier: MIT
// Copyright (C) 2024 Spora developers
//
// Cell State Tree - Merkle tree for live cells
// Provides state root for lightweight client verification

use spora_exec::{OutPoint, Script};
use spora_hashes::{Hash, HasherBase, MerkleBranchHash};
use spora_muhash::MuHash;
use std::{
    collections::BTreeMap,
    ops::Bound::{Excluded, Included, Unbounded},
};

/// Cell state entry in the tree
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CellEntry {
    /// Cell capacity
    pub capacity: u64,
    /// Data length in bytes
    pub data_bytes: u64,
    /// Lock script hash
    pub lock_hash: Hash,
    /// Type script hash (if present)
    pub type_hash: Option<Hash>,
    /// Data hash
    pub data_hash: Hash,
    /// Block DAA score where this cell was created
    pub block_daa_score: u64,
    /// Whether this cell was created by a cellbase transaction
    pub is_cellbase: bool,
    /// Full lock script when the live state source can retain it.
    ///
    /// This field is intentionally excluded from [`CellEntry::serialize`] so
    /// state roots remain committed only to the canonical compact metadata.
    pub lock_script: Option<Script>,
    /// Full type script when the live state source can retain it.
    ///
    /// This field is intentionally excluded from [`CellEntry::serialize`].
    pub type_script: Option<Script>,
    /// Full cell data when the live state source can retain it.
    ///
    /// This field is intentionally excluded from [`CellEntry::serialize`].
    pub data: Option<Vec<u8>>,
}

impl CellEntry {
    /// Create a new cell entry
    pub fn new(
        capacity: u64,
        data_bytes: u64,
        lock_hash: Hash,
        type_hash: Option<Hash>,
        data_hash: Hash,
        block_daa_score: u64,
        is_cellbase: bool,
    ) -> Self {
        Self {
            capacity,
            data_bytes,
            lock_hash,
            type_hash,
            data_hash,
            block_daa_score,
            is_cellbase,
            lock_script: None,
            type_script: None,
            data: None,
        }
    }

    /// Attach optional full script/data metadata without changing the state commitment.
    pub fn with_resolved_metadata(mut self, lock_script: Option<Script>, type_script: Option<Script>, data: Option<Vec<u8>>) -> Self {
        self.lock_script = lock_script;
        self.type_script = type_script;
        self.data = data;
        self
    }

    /// Serialize cell entry for hashing
    pub fn serialize(&self) -> Vec<u8> {
        let mut bytes = Vec::new();

        // Capacity (8 bytes)
        bytes.extend_from_slice(&self.capacity.to_le_bytes());

        // Data length (8 bytes)
        bytes.extend_from_slice(&self.data_bytes.to_le_bytes());

        // Lock hash (32 bytes)
        bytes.extend_from_slice(&self.lock_hash.as_bytes());

        // Type hash (1 byte flag + 32 bytes if present)
        if let Some(ref type_hash) = self.type_hash {
            bytes.push(1);
            bytes.extend_from_slice(&type_hash.as_bytes());
        } else {
            bytes.push(0);
        }

        // Data hash (32 bytes)
        bytes.extend_from_slice(&self.data_hash.as_bytes());

        // Creation DAA score (8 bytes)
        bytes.extend_from_slice(&self.block_daa_score.to_le_bytes());

        // Cellbase flag (1 byte)
        bytes.push(u8::from(self.is_cellbase));

        bytes
    }

    /// Hash the cell entry
    pub fn hash(&self) -> Hash {
        let serialized = self.serialize();
        let mut hasher = MerkleBranchHash::new();
        hasher.update(b"spora-cell/entry"); // Domain separation
        hasher.update(&serialized);
        hasher.finalize()
    }
}

/// Cell State Tree - MuHash accumulator for live cells
///
/// This tree maintains the state of all live (unspent) cells.
/// The tree is keyed by OutPoint hash and stores cell metadata.
///
/// Features:
/// - Sparse key space for live-cell membership
/// - Mutable add/remove operations over the live-cell set
/// - O(1) incremental root updates via MuHash accumulator
/// - Deterministic proof / pagination helpers
///
/// # Performance
///
/// Root computation uses MuHash (multiplicative hash accumulator). Insert and
/// remove operations incrementally update the accumulator in O(1), and
/// `root()` calls `muhash.finalize()` which is also O(1) (384-byte modular
/// arithmetic). The root is cached and only recomputed on mutation.
///
/// The leaf-hash cache (`leaf_hashes`) is retained so that `remove` can undo
/// a previously added element without recomputing the leaf hash from scratch.
#[derive(Clone)]
pub struct CellStateTree {
    /// Cells indexed by outpoint hash (public for consensus layer access)
    pub cells: BTreeMap<Hash, CellEntry>,

    /// Original outpoints indexed by outpoint hash.
    ///
    /// The Merkle tree is still keyed by `outpoint_hash`, but consensus query paths need the
    /// original outpoint for chunked enumeration.
    outpoints_by_hash: BTreeMap<Hash, OutPoint>,

    /// Reverse index used for stable pagination by original outpoint ordering.
    outpoint_hashes: BTreeMap<OutPoint, Hash>,

    /// Per-leaf hash cache: avoids re-serializing and re-hashing unchanged cells.
    leaf_hashes: BTreeMap<Hash, Hash>,

    /// MuHash accumulator for O(1) incremental root updates.
    muhash: MuHash,

    /// Cached root (invalidated on updates)
    cached_root: Option<Hash>,
}

impl CellStateTree {
    /// Create a new empty cell state tree
    pub fn new() -> Self {
        Self {
            cells: BTreeMap::new(),
            outpoints_by_hash: BTreeMap::new(),
            outpoint_hashes: BTreeMap::new(),
            leaf_hashes: BTreeMap::new(),
            muhash: MuHash::new(),
            cached_root: None,
        }
    }

    /// Insert a cell into the tree
    pub fn insert(&mut self, outpoint_hash: Hash, entry: CellEntry) {
        // Pre-compute and cache the leaf hash
        let leaf_hash = Self::compute_leaf_hash(&outpoint_hash, &entry);
        // Update MuHash accumulator: remove old leaf if replacing, then add new
        if let Some(old_leaf_hash) = self.leaf_hashes.insert(outpoint_hash, leaf_hash) {
            self.muhash.remove_element(&old_leaf_hash.as_bytes());
        }
        self.muhash.add_element(&leaf_hash.as_bytes());
        self.cells.insert(outpoint_hash, entry);
        self.cached_root = None; // Invalidate cache
    }

    /// Insert a cell into the tree while preserving the original outpoint.
    pub fn insert_with_outpoint(&mut self, outpoint_hash: Hash, outpoint: OutPoint, entry: CellEntry) {
        if let Some(previous_outpoint) = self.outpoints_by_hash.insert(outpoint_hash, outpoint.clone()) {
            self.outpoint_hashes.remove(&previous_outpoint);
        }
        if let Some(previous_hash) = self.outpoint_hashes.insert(outpoint.clone(), outpoint_hash) {
            if previous_hash != outpoint_hash {
                self.cells.remove(&previous_hash);
                self.outpoints_by_hash.remove(&previous_hash);
                // Remove evicted leaf from MuHash
                if let Some(old_leaf_hash) = self.leaf_hashes.remove(&previous_hash) {
                    self.muhash.remove_element(&old_leaf_hash.as_bytes());
                }
            }
        }
        // Pre-compute and cache the leaf hash
        let leaf_hash = Self::compute_leaf_hash(&outpoint_hash, &entry);
        // Update MuHash accumulator: remove old leaf if replacing, then add new
        if let Some(old_leaf_hash) = self.leaf_hashes.insert(outpoint_hash, leaf_hash) {
            self.muhash.remove_element(&old_leaf_hash.as_bytes());
        }
        self.muhash.add_element(&leaf_hash.as_bytes());
        self.cells.insert(outpoint_hash, entry);
        self.cached_root = None; // Invalidate cache
    }

    /// Remove a cell from the tree
    pub fn remove(&mut self, outpoint_hash: &Hash) -> Option<CellEntry> {
        let result = self.cells.remove(outpoint_hash);
        if result.is_some() {
            // Remove leaf from MuHash accumulator
            if let Some(old_leaf_hash) = self.leaf_hashes.remove(outpoint_hash) {
                self.muhash.remove_element(&old_leaf_hash.as_bytes());
            }
            if let Some(outpoint) = self.outpoints_by_hash.remove(outpoint_hash) {
                self.outpoint_hashes.remove(&outpoint);
            }
            self.cached_root = None; // Invalidate cache
        }
        result
    }

    /// Get a cell from the tree
    pub fn get(&self, outpoint_hash: &Hash) -> Option<&CellEntry> {
        self.cells.get(outpoint_hash)
    }

    /// Get the original outpoint corresponding to a hashed tree key.
    pub fn get_outpoint(&self, outpoint_hash: &Hash) -> Option<&OutPoint> {
        self.outpoints_by_hash.get(outpoint_hash)
    }

    /// Get the number of cells in the tree
    pub fn len(&self) -> usize {
        self.cells.len()
    }

    /// Check if the tree is empty
    pub fn is_empty(&self) -> bool {
        self.cells.is_empty()
    }

    /// Compute leaf hash for a single cell (used for caching)
    fn compute_leaf_hash(outpoint_hash: &Hash, entry: &CellEntry) -> Hash {
        let cell_hash = entry.hash();
        let mut hasher = MerkleBranchHash::new();
        hasher.update(b"spora-cell/leaf");
        hasher.update(&outpoint_hash.as_bytes());
        hasher.update(&cell_hash.as_bytes());
        hasher.finalize()
    }

    /// Calculate the MuHash root of the cell tree
    ///
    /// Returns the finalized MuHash accumulator value. This is O(1) since
    /// the accumulator is maintained incrementally on insert/remove.
    pub fn root(&mut self) -> Hash {
        // Return cached root if available
        if let Some(ref root) = self.cached_root {
            return *root;
        }

        let root = self.muhash.clone().finalize();
        self.cached_root = Some(root);
        root
    }

    /// Clear the tree
    pub fn clear(&mut self) {
        self.cells.clear();
        self.outpoints_by_hash.clear();
        self.outpoint_hashes.clear();
        self.leaf_hashes.clear();
        self.muhash = MuHash::new();
        self.cached_root = None;
    }

    /// Get all cell entries (for iteration)
    pub fn iter(&self) -> impl Iterator<Item = (&Hash, &CellEntry)> {
        self.cells.iter()
    }

    /// Iterate cells ordered by their original outpoint.
    pub fn iter_by_outpoint(&self) -> impl Iterator<Item = (&OutPoint, &Hash, &CellEntry)> {
        self.outpoint_hashes
            .iter()
            .filter_map(|(outpoint, outpoint_hash)| self.cells.get(outpoint_hash).map(|entry| (outpoint, outpoint_hash, entry)))
    }

    /// Iterate cells ordered by original outpoint with an optional pagination anchor.
    pub fn iter_by_outpoint_from<'a>(
        &'a self,
        from_outpoint: Option<&'a OutPoint>,
        skip_first: bool,
    ) -> Box<dyn Iterator<Item = (&'a OutPoint, &'a Hash, &'a CellEntry)> + 'a> {
        let iter: Box<dyn Iterator<Item = (&'a OutPoint, &'a Hash, &'a CellEntry)> + 'a> =
            match from_outpoint {
                Some(from_outpoint) if skip_first => {
                    Box::new(self.outpoint_hashes.range((Excluded(from_outpoint.clone()), Unbounded)).filter_map(
                        |(outpoint, outpoint_hash)| self.cells.get(outpoint_hash).map(|entry| (outpoint, outpoint_hash, entry)),
                    ))
                }
                Some(from_outpoint) => Box::new(self.outpoint_hashes.range((Included(from_outpoint.clone()), Unbounded)).filter_map(
                    |(outpoint, outpoint_hash)| self.cells.get(outpoint_hash).map(|entry| (outpoint, outpoint_hash, entry)),
                )),
                None => Box::new(self.iter_by_outpoint()),
            };

        iter
    }
}

impl Default for CellStateTree {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn create_test_entry(capacity: u64) -> CellEntry {
        CellEntry::new(capacity, 0, Hash::from_bytes([1u8; 32]), None, Hash::from_bytes([2u8; 32]), 100, false)
    }

    fn create_test_outpoint(byte: u8, index: u32) -> OutPoint {
        OutPoint::new([byte; 32], index)
    }

    #[test]
    fn test_cell_entry_serialization() {
        let entry = create_test_entry(100000);
        let serialized = entry.serialize();

        // 8 (capacity) + 8 (data bytes) + 32 (lock) + 1 (type flag) + 32 (data hash) + 8 (daa) + 1 (cellbase) = 90 bytes
        assert_eq!(serialized.len(), 90);

        // Verify capacity
        assert_eq!(&serialized[0..8], &100000u64.to_le_bytes());

        // Verify data length
        assert_eq!(&serialized[8..16], &0u64.to_le_bytes());

        // Verify no type script
        assert_eq!(serialized[48], 0);
    }

    #[test]
    fn test_cell_entry_with_type_script() {
        let entry = CellEntry::new(
            100000,
            0,
            Hash::from_bytes([1u8; 32]),
            Some(Hash::from_bytes([3u8; 32])),
            Hash::from_bytes([2u8; 32]),
            100,
            true,
        );

        let serialized = entry.serialize();

        // 8 + 8 + 32 + 1 + 32 (type) + 32 + 8 + 1 = 122 bytes
        assert_eq!(serialized.len(), 122);

        // Verify type script present
        assert_eq!(serialized[48], 1);
        assert_eq!(&serialized[49..81], &[3u8; 32]);
    }

    #[test]
    fn test_cell_entry_hash() {
        let entry1 = create_test_entry(100000);
        let entry2 = create_test_entry(100000);
        let entry3 = create_test_entry(200000);

        // Same entries should have same hash
        assert_eq!(entry1.hash(), entry2.hash());

        // Different entries should have different hash
        assert_ne!(entry1.hash(), entry3.hash());
    }

    #[test]
    fn test_empty_tree() {
        let mut tree = CellStateTree::new();
        assert_eq!(tree.len(), 0);
        assert!(tree.is_empty());

        // Empty tree root is EMPTY_MUHASH
        assert_eq!(tree.root(), spora_muhash::EMPTY_MUHASH);
    }

    #[test]
    fn test_insert_and_get() {
        let mut tree = CellStateTree::new();
        let outpoint = Hash::from_bytes([10u8; 32]);
        let entry = create_test_entry(100000);

        tree.insert(outpoint, entry.clone());

        assert_eq!(tree.len(), 1);
        assert_eq!(tree.get(&outpoint), Some(&entry));
    }

    #[test]
    fn test_remove() {
        let mut tree = CellStateTree::new();
        let outpoint = Hash::from_bytes([10u8; 32]);
        let entry = create_test_entry(100000);

        tree.insert(outpoint, entry.clone());
        assert_eq!(tree.len(), 1);

        let removed = tree.remove(&outpoint);
        assert_eq!(removed, Some(entry));
        assert_eq!(tree.len(), 0);
        assert!(tree.is_empty());
    }

    #[test]
    fn test_single_cell_root() {
        let mut tree = CellStateTree::new();
        let outpoint = Hash::from_bytes([10u8; 32]);
        let entry = create_test_entry(100000);

        tree.insert(outpoint, entry);

        let root = tree.root();
        assert_ne!(root, Hash::from_bytes([0u8; 32]));

        // Root should be cached
        let root2 = tree.root();
        assert_eq!(root, root2);
    }

    #[test]
    fn test_multiple_cells_root() {
        let mut tree = CellStateTree::new();

        // Insert 3 cells
        for i in 0..3 {
            let outpoint = Hash::from_bytes([i as u8; 32]);
            let entry = create_test_entry(100000 + i as u64);
            tree.insert(outpoint, entry);
        }

        assert_eq!(tree.len(), 3);

        let root = tree.root();
        assert_ne!(root, Hash::from_bytes([0u8; 32]));
    }

    #[test]
    fn test_root_changes_on_modification() {
        let mut tree = CellStateTree::new();
        let outpoint1 = Hash::from_bytes([1u8; 32]);
        let outpoint2 = Hash::from_bytes([2u8; 32]);

        tree.insert(outpoint1, create_test_entry(100000));
        let root1 = tree.root();

        tree.insert(outpoint2, create_test_entry(200000));
        let root2 = tree.root();

        // Root should change after insertion
        assert_ne!(root1, root2);

        tree.remove(&outpoint2);
        let root3 = tree.root();

        // Root should return to original after removal
        assert_eq!(root1, root3);
    }

    #[test]
    fn test_deterministic_root() {
        // Same cells in different order should produce same root
        let mut tree1 = CellStateTree::new();
        let mut tree2 = CellStateTree::new();

        let cells = vec![
            (Hash::from_bytes([1u8; 32]), create_test_entry(100)),
            (Hash::from_bytes([2u8; 32]), create_test_entry(200)),
            (Hash::from_bytes([3u8; 32]), create_test_entry(300)),
        ];

        // Insert in forward order
        for (outpoint, entry) in &cells {
            tree1.insert(*outpoint, entry.clone());
        }

        // Insert in reverse order
        for (outpoint, entry) in cells.iter().rev() {
            tree2.insert(*outpoint, entry.clone());
        }

        assert_eq!(tree1.root(), tree2.root());
    }

    #[test]
    fn test_insert_with_outpoint_preserves_original_outpoint_order() {
        let mut tree = CellStateTree::new();

        tree.insert_with_outpoint(Hash::from_bytes([2u8; 32]), create_test_outpoint(2, 0), create_test_entry(200));
        tree.insert_with_outpoint(Hash::from_bytes([1u8; 32]), create_test_outpoint(1, 0), create_test_entry(100));

        let ordered =
            tree.iter_by_outpoint().map(|(outpoint, _, entry)| (outpoint.tx_hash, outpoint.index, entry.capacity)).collect::<Vec<_>>();

        assert_eq!(ordered, vec![([1u8; 32], 0, 100), ([2u8; 32], 0, 200)]);
    }
}
