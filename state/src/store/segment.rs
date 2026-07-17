// SPDX-License-Identifier: MIT
// Copyright (C) 2026 Myelin developers
//
// Segment storage: 1GB data segments for DA layer

use crate::{
    molecule,
    store::proof::{compute_segment_root, MerkleTreeBuilder},
    Result, SegmentInfo, StateError,
};
use parking_lot::Mutex;
use std::fs::{File, OpenOptions};
use std::io::{Read, Seek, SeekFrom, Write};
use std::path::{Path, PathBuf};
use std::sync::Arc;

/// Segment size: 1GB
const SEGMENT_SIZE: u64 = 1024 * 1024 * 1024;

/// Maximum segments in memory before forcing seal
const MAX_OPEN_SEGMENTS: usize = 8;

/// Sealed segment metadata is consensus material; wall-clock evidence belongs
/// in external reports, not in the committed bytes.
const DETERMINISTIC_SEGMENT_TIMESTAMP: u64 = 0;

#[derive(Clone, Debug, PartialEq, Eq)]
struct AppendRecord {
    offset: u64,
    length: u32,
}

/// Segment metadata
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct SegmentMeta {
    /// Segment ID
    pub segment_id: u32,
    /// Size (bytes written)
    pub size: u64,
    /// Number of Cells
    pub cell_count: u32,
    /// Merkle root (DA commitment)
    pub merkle_root: [u8; 32],
    /// Is sealed?
    pub sealed: bool,
    /// Deterministic creation marker
    pub created_at: u64,
    /// Deterministic sealed marker
    pub sealed_at: Option<u64>,
}

fn encode_append_record(record: &AppendRecord) -> Vec<u8> {
    molecule::encode_table(&[molecule::encode_u64(record.offset), molecule::encode_u32(record.length)])
}

fn decode_append_record(bytes: &[u8]) -> Result<AppendRecord> {
    let fields = molecule::decode_table(bytes, 2, "AppendRecord")?;
    Ok(AppendRecord {
        offset: molecule::decode_u64(fields[0], "AppendRecord.offset")?,
        length: molecule::decode_u32(fields[1], "AppendRecord.length")?,
    })
}

fn encode_append_records(records: &[AppendRecord]) -> Vec<u8> {
    let items = records.iter().map(encode_append_record).collect::<Vec<_>>();
    molecule::encode_dynvec(&items)
}

fn decode_append_records(bytes: &[u8]) -> Result<Vec<AppendRecord>> {
    molecule::decode_dynvec(bytes, "AppendRecordVec")?.into_iter().map(decode_append_record).collect()
}

fn encode_segment_meta(meta: &SegmentMeta) -> Vec<u8> {
    molecule::encode_table(&[
        molecule::encode_u32(meta.segment_id),
        molecule::encode_u64(meta.size),
        molecule::encode_u32(meta.cell_count),
        meta.merkle_root.to_vec(),
        molecule::encode_bool(meta.sealed),
        molecule::encode_u64(meta.created_at),
        meta.sealed_at.map(molecule::encode_u64).unwrap_or_default(),
    ])
}

fn decode_segment_meta(bytes: &[u8]) -> Result<SegmentMeta> {
    let fields = molecule::decode_table(bytes, 7, "SegmentMeta")?;
    Ok(SegmentMeta {
        segment_id: molecule::decode_u32(fields[0], "SegmentMeta.segment_id")?,
        size: molecule::decode_u64(fields[1], "SegmentMeta.size")?,
        cell_count: molecule::decode_u32(fields[2], "SegmentMeta.cell_count")?,
        merkle_root: molecule::decode_array32(fields[3], "SegmentMeta.merkle_root")?,
        sealed: molecule::decode_bool(fields[4], "SegmentMeta.sealed")?,
        created_at: molecule::decode_u64(fields[5], "SegmentMeta.created_at")?,
        sealed_at: if fields[6].is_empty() { None } else { Some(molecule::decode_u64(fields[6], "SegmentMeta.sealed_at")?) },
    })
}

/// Segment writer (append-only)
///
/// Manages sequential writes to 1GB segment files
pub struct SegmentWriter {
    /// Base directory for segments
    base_dir: PathBuf,
    /// Current segment ID
    current_segment_id: Arc<Mutex<u32>>,
    /// Current segment file
    current_file: Arc<Mutex<Option<File>>>,
    /// Current offset
    current_offset: Arc<Mutex<u64>>,
    /// Ordered append records for the active segment.
    current_chunks: Arc<Mutex<Vec<AppendRecord>>>,
    /// Segment metadata
    _segments: Arc<Mutex<Vec<SegmentMeta>>>,
}

impl SegmentWriter {
    /// Create a new segment writer
    pub fn new<P: AsRef<Path>>(base_dir: P) -> Result<Self> {
        let base_dir = base_dir.as_ref().to_path_buf();
        std::fs::create_dir_all(&base_dir)?;

        // Find highest existing segment ID
        let max_id = Self::find_max_segment_id(&base_dir)?;

        let (current_segment_id, current_file, current_offset, current_chunks) =
            if let Some(segment_id) = max_id.filter(|id| !Self::segment_meta_path_for(&base_dir, *id).exists()) {
                let path = Self::segment_path_for(&base_dir, segment_id);
                let size = std::fs::metadata(&path)?.len();
                let chunks = Self::load_chunk_index(&base_dir, segment_id)?;
                let file = OpenOptions::new().create(true).append(true).open(path)?;
                (segment_id, Some(file), size, chunks)
            } else {
                (max_id.map(|id| id + 1).unwrap_or(0), None, 0, Vec::new())
            };

        Ok(Self {
            base_dir,
            current_segment_id: Arc::new(Mutex::new(current_segment_id)),
            current_file: Arc::new(Mutex::new(current_file)),
            current_offset: Arc::new(Mutex::new(current_offset)),
            current_chunks: Arc::new(Mutex::new(current_chunks)),
            _segments: Arc::new(Mutex::new(Vec::new())),
        })
    }

    /// Append Cell data to current segment
    ///
    /// Returns: (segment_id, offset, length)
    pub fn append(&self, data: &[u8]) -> Result<(u32, u64, u32)> {
        let mut file_guard = self.current_file.lock();
        let mut offset_guard = self.current_offset.lock();

        // Open new segment if needed
        if file_guard.is_none() || *offset_guard + data.len() as u64 > SEGMENT_SIZE {
            drop(file_guard);
            drop(offset_guard);
            self.rotate_segment()?;
            file_guard = self.current_file.lock();
            offset_guard = self.current_offset.lock();
        }

        let file = file_guard.as_mut().ok_or_else(|| StateError::Database("No active segment file".to_string()))?;

        let offset = *offset_guard;
        let length = data.len() as u32;

        // Write data
        file.write_all(data)?;
        file.sync_data()?; // Ensure durability

        *offset_guard += data.len() as u64;
        let segment_id = *self.current_segment_id.lock();
        {
            let mut current_chunks = self.current_chunks.lock();
            current_chunks.push(AppendRecord { offset, length });
            self.save_chunk_index(segment_id, &current_chunks)?;
        }

        Ok((segment_id, offset, length))
    }

    /// Seal current segment (finalize and compute commitment)
    pub fn seal(&self) -> Result<SegmentMeta> {
        let file_guard = self.current_file.lock();
        let offset_guard = self.current_offset.lock();

        if file_guard.is_none() {
            return Err(StateError::Database("No active segment to seal".to_string()));
        }

        let segment_id = *self.current_segment_id.lock();
        let size = *offset_guard;

        // F-02 seal binding: snapshot the chunk boundaries under the SAME
        // critical section that holds the file/offset guards, so the root is
        // computed over exactly the chunks that belong to this segment. The
        // previous code dropped file_guard/offset_guard here and then
        // `compute_merkle_root` re-locked `current_chunks` separately — a
        // TOCTOU window in which a concurrent `append` could land a chunk in
        // the soon-to-be-sealed segment's chunk list while the file was
        // already considered sealed. We also drain the snapshot out of the
        // critical section and compute the root from it without re-locking.
        let chunk_snapshot = self.current_chunks.lock().clone();
        let cell_count = chunk_snapshot.len() as u32;
        drop(file_guard);
        drop(offset_guard);

        let merkle_root = self.compute_merkle_root_from_snapshot(segment_id, size, &chunk_snapshot)?;

        let meta = SegmentMeta {
            segment_id,
            size,
            cell_count,
            merkle_root,
            sealed: true,
            created_at: Self::deterministic_timestamp(),
            sealed_at: Some(Self::deterministic_timestamp()),
        };

        // Save metadata and chunk index together so a sealed segment always
        // carries the boundaries its root was computed over.
        self.save_segment_meta(&meta)?;
        self.save_chunk_index(segment_id, &chunk_snapshot)?;

        // Close current segment
        let mut file_guard = self.current_file.lock();
        *file_guard = None;
        drop(file_guard);

        let mut offset_guard = self.current_offset.lock();
        *offset_guard = 0;
        drop(offset_guard);
        self.current_chunks.lock().clear();

        let mut segment_id_guard = self.current_segment_id.lock();
        *segment_id_guard += 1;

        Ok(meta)
    }

    /// Force seal and rotate to new segment
    fn rotate_segment(&self) -> Result<()> {
        // Seal current if exists
        if self.current_file.lock().is_some() {
            self.seal()?;
        }

        // Open new segment
        let new_id = *self.current_segment_id.lock();
        let path = self.segment_path(new_id);

        let file = OpenOptions::new().create(true).append(true).open(&path)?;

        let mut file_guard = self.current_file.lock();
        *file_guard = Some(file);

        let mut offset_guard = self.current_offset.lock();
        *offset_guard = 0;

        Ok(())
    }

    /// Compute a Merkle root over a caller-supplied chunk snapshot, without
    /// touching `current_chunks`. This is the seal-binding path: the snapshot
    /// is taken under the file/offset critical section so it cannot drift.
    fn compute_merkle_root_from_snapshot(
        &self,
        segment_id: u32,
        size: u64,
        chunk_ranges: &[AppendRecord],
    ) -> Result<[u8; 32]> {
        let path = self.segment_path(segment_id);
        let mut file = File::open(path)?;

        if chunk_ranges.is_empty() && size > 0 {
            return Err(StateError::Database("Cannot compute segment root without append chunk boundaries".to_string()));
        }

        let mut chunks = Vec::with_capacity(chunk_ranges.len());
        for AppendRecord { offset, length } in chunk_ranges {
            let mut buffer = vec![0u8; *length as usize];
            file.seek(SeekFrom::Start(*offset))?;
            file.read_exact(&mut buffer)?;
            chunks.push(buffer);
        }

        Ok(compute_segment_root(&chunks))
    }

    /// Save segment metadata
    fn save_segment_meta(&self, meta: &SegmentMeta) -> Result<()> {
        let path = self.segment_meta_path(meta.segment_id);
        std::fs::write(path, encode_segment_meta(meta))?;
        Ok(())
    }

    fn save_chunk_index(&self, segment_id: u32, chunks: &[AppendRecord]) -> Result<()> {
        let path = self.segment_index_path(segment_id);
        std::fs::write(path, encode_append_records(chunks))?;
        Ok(())
    }

    /// Get segment file path
    fn segment_path(&self, segment_id: u32) -> PathBuf {
        Self::segment_path_for(&self.base_dir, segment_id)
    }

    /// Get segment metadata path
    fn segment_meta_path(&self, segment_id: u32) -> PathBuf {
        Self::segment_meta_path_for(&self.base_dir, segment_id)
    }

    fn segment_index_path(&self, segment_id: u32) -> PathBuf {
        Self::segment_index_path_for(&self.base_dir, segment_id)
    }

    fn segment_path_for(base_dir: &Path, segment_id: u32) -> PathBuf {
        base_dir.join(format!("segment_{:08}.dat", segment_id))
    }

    fn segment_meta_path_for(base_dir: &Path, segment_id: u32) -> PathBuf {
        base_dir.join(format!("segment_{:08}.meta", segment_id))
    }

    fn segment_index_path_for(base_dir: &Path, segment_id: u32) -> PathBuf {
        base_dir.join(format!("segment_{:08}.idx", segment_id))
    }

    /// Find maximum existing segment ID
    fn find_max_segment_id(base_dir: &Path) -> Result<Option<u32>> {
        let entries = std::fs::read_dir(base_dir)?;
        let mut max_id = None;

        for entry in entries {
            let entry = entry?;
            let filename = entry.file_name();
            let filename_str = filename.to_string_lossy();

            if filename_str.starts_with("segment_") && filename_str.ends_with(".dat") {
                if let Some(id_str) = filename_str.strip_prefix("segment_").and_then(|s| s.strip_suffix(".dat")) {
                    if let Ok(id) = id_str.parse::<u32>() {
                        max_id = Some(max_id.unwrap_or(0).max(id));
                    }
                }
            }
        }

        Ok(max_id)
    }

    fn load_chunk_index(base_dir: &Path, segment_id: u32) -> Result<Vec<AppendRecord>> {
        let path = Self::segment_index_path_for(base_dir, segment_id);
        match std::fs::read(path) {
            Ok(data) => decode_append_records(&data),
            Err(err) if err.kind() == std::io::ErrorKind::NotFound => Ok(Vec::new()),
            Err(err) => Err(StateError::Database(err.to_string())),
        }
    }

    fn deterministic_timestamp() -> u64 {
        DETERMINISTIC_SEGMENT_TIMESTAMP
    }
}

/// Segment reader (random access)
pub struct SegmentReader {
    /// Base directory
    base_dir: PathBuf,
    /// Open files cache
    files: Arc<Mutex<lru::LruCache<u32, File>>>,
}

impl SegmentReader {
    /// Create a new segment reader
    pub fn new<P: AsRef<Path>>(base_dir: P) -> Result<Self> {
        Ok(Self {
            base_dir: base_dir.as_ref().to_path_buf(),
            files: Arc::new(Mutex::new(lru::LruCache::new(std::num::NonZeroUsize::new(MAX_OPEN_SEGMENTS).unwrap()))),
        })
    }

    /// Read data from a segment
    pub fn read(&self, segment_id: u32, offset: u64, length: u32) -> Result<Vec<u8>> {
        // F-16 cache contention fix: take the LRU lock only for the cache
        // lookup / file-open, clone the handle, and DROP the lock before doing
        // any disk I/O. Previously the lock was held across seek + read_exact,
        // which serialized all readers behind a single slow disk read.
        let mut file_clone = {
            let mut files = self.files.lock();
            // Get or open file
            let file = if let Some(f) = files.get(&segment_id) {
                f
            } else {
                let path = self.segment_path(segment_id);
                let file = File::open(path).map_err(|_| StateError::SegmentNotFound(segment_id))?;
                files.put(segment_id, file);
                files.get(&segment_id).unwrap()
            };
            file.try_clone()?
        }; // files lock released here

        // Read data outside the cache lock.
        let mut buffer = vec![0u8; length as usize];
        file_clone.seek(SeekFrom::Start(offset))?;
        file_clone.read_exact(&mut buffer)?;

        Ok(buffer)
    }

    /// Load segment metadata
    pub fn load_meta(&self, segment_id: u32) -> Result<SegmentMeta> {
        let path = self.segment_meta_path(segment_id);
        let data = std::fs::read(path).map_err(|_| StateError::SegmentNotFound(segment_id))?;
        decode_segment_meta(&data)
    }

    /// Verify that a sealed segment's persisted `merkle_root` is
    /// cryptographically bound to the actual chunk bytes on disk.
    ///
    /// F-02 seal binding: `SegmentMeta.sealed` is just a boolean written by
    /// the writer, and `merkle_root` is a field that could in principle be
    /// constructed arbitrarily. This method re-reads the chunk index and the
    /// segment file, recomputes the root, and checks it equals the persisted
    /// `merkle_root`. Returns `Ok(true)` when the binding holds, `Ok(false)`
    /// when the segment is unsealed (nothing to bind), and `Err` when the
    /// segment claims to be sealed but the recomputed root diverges — i.e.
    /// the on-disk bytes were mutated after sealing.
    pub fn verify_seal_binding(&self, segment_id: u32) -> Result<bool> {
        let meta = self.load_meta(segment_id)?;
        if !meta.sealed {
            return Ok(false);
        }
        let chunk_index = SegmentWriter::load_chunk_index(&self.base_dir, segment_id)?;
        let builder = self.build_merkle_builder(segment_id, &chunk_index)?;
        let computed_root = builder.build();
        if computed_root != meta.merkle_root {
            return Err(StateError::InvalidProof(format!(
                "segment {segment_id} seal binding broken: persisted merkle_root {:?} != recomputed {:?} (segment file mutated after seal)",
                meta.merkle_root, computed_root
            )));
        }
        Ok(true)
    }

    fn build_merkle_builder(&self, segment_id: u32, chunk_index: &[AppendRecord]) -> Result<MerkleTreeBuilder> {
        let mut builder = MerkleTreeBuilder::new();
        for AppendRecord { offset, length } in chunk_index {
            let chunk = self.read(segment_id, *offset, *length)?;
            builder.add_leaf(&chunk);
        }
        Ok(builder)
    }

    fn resolve_segment_root(&self, segment_id: u32, chunk_index: &[AppendRecord]) -> Result<[u8; 32]> {
        match self.load_meta(segment_id) {
            Ok(meta) => Ok(meta.merkle_root),
            Err(StateError::SegmentNotFound(_)) => {
                let builder = self.build_merkle_builder(segment_id, chunk_index)?;
                Ok(builder.build())
            }
            Err(err) => Err(err),
        }
    }

    /// Build a Merkle proof for the requested append chunk in a sealed segment.
    pub fn build_proof(&self, segment_id: u32, leaf_index: u32) -> Result<crate::store::proof::SegmentProof> {
        let chunk_index = SegmentWriter::load_chunk_index(&self.base_dir, segment_id)?;
        let leaf_index = leaf_index as usize;
        let record = chunk_index
            .get(leaf_index)
            .ok_or_else(|| StateError::InvalidProof(format!("leaf index {} out of bounds for segment {}", leaf_index, segment_id)))?;

        let builder = self.build_merkle_builder(segment_id, &chunk_index)?;
        let segment_root = self.resolve_segment_root(segment_id, &chunk_index)?;

        let computed_root = builder.build();
        if computed_root != segment_root {
            return Err(StateError::InvalidProof(format!(
                "segment {} proof index root mismatch: expected={:?}, computed={:?}",
                segment_id, segment_root, computed_root
            )));
        }

        let chunk_data = self.read(segment_id, record.offset, record.length)?;
        let mut proof = crate::store::proof::SegmentProof::new(
            segment_id,
            leaf_index as u32,
            chunk_data,
            record.offset,
            record.length,
            segment_root,
        );
        proof.merkle_path = builder.get_proof(leaf_index);
        Ok(proof)
    }

    /// Locate the append-order leaf index for an existing segment pointer.
    pub fn find_leaf_index(&self, segment_info: &SegmentInfo) -> Result<u32> {
        let chunk_index = SegmentWriter::load_chunk_index(&self.base_dir, segment_info.segment_id)?;
        chunk_index
            .iter()
            .position(|record| record.offset == segment_info.offset && record.length == segment_info.length)
            .map(|index| index as u32)
            .ok_or_else(|| {
                StateError::InvalidProof(format!(
                    "segment pointer ({}, {}, {}) not found in chunk index",
                    segment_info.segment_id, segment_info.offset, segment_info.length
                ))
            })
    }

    /// Build a Merkle proof directly from a persisted segment pointer.
    pub fn build_proof_for_segment_info(&self, segment_info: &SegmentInfo) -> Result<crate::store::proof::SegmentProof> {
        let leaf_index = self.find_leaf_index(segment_info)?;
        self.build_proof(segment_info.segment_id, leaf_index)
    }

    fn segment_path(&self, segment_id: u32) -> PathBuf {
        self.base_dir.join(format!("segment_{:08}.dat", segment_id))
    }

    fn segment_meta_path(&self, segment_id: u32) -> PathBuf {
        self.base_dir.join(format!("segment_{:08}.meta", segment_id))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::store::proof::compute_segment_root;
    use tempfile::TempDir;

    #[test]
    fn test_segment_writer_append() {
        let tmp = TempDir::new().unwrap();
        let writer = SegmentWriter::new(tmp.path()).unwrap();

        let data = vec![0xAA; 1024];
        let (seg_id, offset, length) = writer.append(&data).unwrap();

        assert_eq!(seg_id, 0);
        assert_eq!(offset, 0);
        assert_eq!(length, 1024);
    }

    #[test]
    fn test_segment_seal() {
        let tmp = TempDir::new().unwrap();
        let writer = SegmentWriter::new(tmp.path()).unwrap();

        let data = vec![0xBB; 2048];
        writer.append(&data).unwrap();

        let meta = writer.seal().unwrap();
        assert_eq!(meta.segment_id, 0);
        assert_eq!(meta.size, 2048);
        assert!(meta.sealed);
    }

    #[test]
    fn test_segment_seal_uses_deterministic_metadata_time() {
        let tmp = TempDir::new().unwrap();
        let writer = SegmentWriter::new(tmp.path()).unwrap();

        writer.append(&[0x42; 32]).unwrap();

        let meta = writer.seal().unwrap();
        let encoded = encode_segment_meta(&meta);
        let decoded = decode_segment_meta(&encoded).unwrap();

        assert_eq!(meta.created_at, DETERMINISTIC_SEGMENT_TIMESTAMP);
        assert_eq!(meta.sealed_at, Some(DETERMINISTIC_SEGMENT_TIMESTAMP));
        assert_eq!(decoded.created_at, DETERMINISTIC_SEGMENT_TIMESTAMP);
        assert_eq!(decoded.sealed_at, Some(DETERMINISTIC_SEGMENT_TIMESTAMP));
    }

    #[test]
    fn test_segment_seal_persists_merkle_root_for_ordered_chunks() {
        let tmp = TempDir::new().unwrap();
        let writer = SegmentWriter::new(tmp.path()).unwrap();

        let chunk_a = vec![0xAB; 1024];
        let chunk_b = vec![0xCD; 1536];
        writer.append(&chunk_a).unwrap();
        writer.append(&chunk_b).unwrap();

        let meta = writer.seal().unwrap();
        let expected_root = compute_segment_root(&[chunk_a, chunk_b]);

        assert_eq!(meta.merkle_root, expected_root);

        let reader = SegmentReader::new(tmp.path()).unwrap();
        let loaded_meta = reader.load_meta(meta.segment_id).unwrap();
        assert_eq!(loaded_meta.merkle_root, expected_root);
    }

    #[test]
    fn test_segment_reader() {
        let tmp = TempDir::new().unwrap();
        let writer = SegmentWriter::new(tmp.path()).unwrap();

        let data = vec![0xCC; 512];
        let (seg_id, offset, length) = writer.append(&data).unwrap();
        writer.seal().unwrap();

        let reader = SegmentReader::new(tmp.path()).unwrap();
        let read_data = reader.read(seg_id, offset, length).unwrap();

        assert_eq!(read_data, data);
    }

    #[test]
    fn test_segment_rotation() {
        let tmp = TempDir::new().unwrap();
        let writer = SegmentWriter::new(tmp.path()).unwrap();

        // Write small data to first segment
        let data1 = vec![0x11; 1024];
        let (seg1, _, _) = writer.append(&data1).unwrap();

        // Seal and rotate
        writer.seal().unwrap();

        // Write to second segment
        let data2 = vec![0x22; 1024];
        let (seg2, _, _) = writer.append(&data2).unwrap();

        assert_ne!(seg1, seg2);
        assert_eq!(seg2, seg1 + 1);
    }

    #[test]
    fn test_segment_writer_recovers_unsealed_segment_after_restart() {
        let tmp = TempDir::new().unwrap();
        let chunk_a = vec![0x31; 128];
        let chunk_b = vec![0x42; 96];

        {
            let writer = SegmentWriter::new(tmp.path()).unwrap();
            writer.append(&chunk_a).unwrap();
            writer.append(&chunk_b).unwrap();
        }

        let recovered_writer = SegmentWriter::new(tmp.path()).unwrap();
        let meta = recovered_writer.seal().unwrap();
        let expected_root = compute_segment_root(&[chunk_a, chunk_b]);

        assert_eq!(meta.segment_id, 0);
        assert_eq!(meta.merkle_root, expected_root);
        assert!(tmp.path().join("segment_00000000.idx").exists());
    }

    #[test]
    fn test_segment_reader_builds_proof_for_sealed_segment() {
        let tmp = TempDir::new().unwrap();
        let writer = SegmentWriter::new(tmp.path()).unwrap();

        let chunk_a = vec![0xAA; 64];
        let chunk_b = vec![0xBB; 96];
        writer.append(&chunk_a).unwrap();
        writer.append(&chunk_b).unwrap();
        let meta = writer.seal().unwrap();

        let reader = SegmentReader::new(tmp.path()).unwrap();
        let proof = reader.build_proof(meta.segment_id, 1).unwrap();

        assert_eq!(proof.segment_id, meta.segment_id);
        assert_eq!(proof.leaf_index, 1);
        assert_eq!(proof.chunk_data, chunk_b);
        assert_eq!(proof.segment_root, meta.merkle_root);
        assert!(proof.verify().unwrap());
    }

    #[test]
    fn test_segment_reader_builds_proof_from_segment_info_pointer() {
        let tmp = TempDir::new().unwrap();
        let writer = SegmentWriter::new(tmp.path()).unwrap();

        let chunk_a = vec![0x10; 48];
        let chunk_b = vec![0x20; 80];
        writer.append(&chunk_a).unwrap();
        let (segment_id, offset, length) = writer.append(&chunk_b).unwrap();
        writer.seal().unwrap();

        let segment_info = SegmentInfo { segment_id, offset, length };
        let reader = SegmentReader::new(tmp.path()).unwrap();
        let proof = reader.build_proof_for_segment_info(&segment_info).unwrap();

        assert_eq!(proof.leaf_index, 1);
        assert_eq!(proof.chunk_offset, offset);
        assert_eq!(proof.chunk_length, length);
        assert_eq!(proof.chunk_data, chunk_b);
        assert!(proof.verify().unwrap());
    }

    #[test]
    fn test_verify_seal_binding_accepts_intact_segment() {
        let tmp = TempDir::new().unwrap();
        let writer = SegmentWriter::new(tmp.path()).unwrap();
        writer.append(&[0xAA; 64]).unwrap();
        writer.append(&[0xBB; 96]).unwrap();
        let meta = writer.seal().unwrap();

        let reader = SegmentReader::new(tmp.path()).unwrap();
        assert!(reader.verify_seal_binding(meta.segment_id).expect("intact segment should bind"));
    }

    #[test]
    fn test_verify_seal_binding_rejects_mutated_segment_file() {
        // F-02 regression: `SegmentMeta.sealed` is a boolean the writer sets,
        // and `merkle_root` is a persisted field. Without binding the root to
        // the bytes on disk, mutating the segment file after seal would be
        // undetectable. `verify_seal_binding` recomputes the root from disk
        // and must reject a mutated file.
        let tmp = TempDir::new().unwrap();
        let writer = SegmentWriter::new(tmp.path()).unwrap();
        writer.append(&[0xAA; 64]).unwrap();
        writer.append(&[0xBB; 96]).unwrap();
        let meta = writer.seal().unwrap();

        // Tamper with the segment file bytes after sealing.
        let segment_path = tmp.path().join(format!("segment_{:08}.dat", meta.segment_id));
        let mut bytes = std::fs::read(&segment_path).unwrap();
        // Flip a byte inside the first chunk (offset 0) — not metadata.
        bytes[0] ^= 0xFF;
        std::fs::write(&segment_path, bytes).unwrap();

        let reader = SegmentReader::new(tmp.path()).unwrap();
        let result = reader.verify_seal_binding(meta.segment_id);
        assert!(matches!(result, Err(StateError::InvalidProof(_))), "mutated segment must fail seal binding: {result:?}");
    }
}
