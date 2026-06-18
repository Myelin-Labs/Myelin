// SPDX-License-Identifier: MIT
// Copyright (C) 2026 Spora developers
//
// Data availability proofs: Merkle-based sampling with an upgrade path to NMT/KZG

use crate::Result;
use borsh::{BorshDeserialize, BorshSerialize};

/// Segment proof (Merkle proof for DA sampling)
///
/// Current implementation uses a conventional Merkle tree over ordered chunk
/// payloads. This keeps proofs sound today while preserving an upgrade path to
/// NMT/KZG for namespaced sampling later.
#[derive(Clone, Debug, PartialEq, Eq, BorshSerialize, BorshDeserialize)]
pub struct SegmentProof {
    /// Segment ID
    pub segment_id: u32,
    /// Leaf index in the ordered append sequence committed by the segment root.
    pub leaf_index: u32,
    /// Data chunk (sampled)
    pub chunk_data: Vec<u8>,
    /// Chunk offset
    pub chunk_offset: u64,
    /// Chunk length
    pub chunk_length: u32,
    /// Merkle path from leaf to root (sibling hashes).
    pub merkle_path: Vec<[u8; 32]>,
    /// Segment root (commitment)
    pub segment_root: [u8; 32],
}

impl SegmentProof {
    /// Create a new segment proof
    pub fn new(
        segment_id: u32,
        leaf_index: u32,
        chunk_data: Vec<u8>,
        chunk_offset: u64,
        chunk_length: u32,
        segment_root: [u8; 32],
    ) -> Self {
        Self { segment_id, leaf_index, chunk_data, chunk_offset, chunk_length, merkle_path: vec![], segment_root }
    }

    /// Verify proof against the committed segment root.
    pub fn verify(&self) -> Result<bool> {
        if self.chunk_data.len() != self.chunk_length as usize {
            return Ok(false);
        }

        let leaf = hash_leaf(&self.chunk_data);
        Ok(verify_merkle_proof(&leaf, &self.merkle_path, &self.segment_root, self.leaf_index as usize))
    }
}

/// Proof verifier
pub struct ProofVerifier {
    /// Verification parameters (for future KZG/NMT)
    _params: (),
}

impl ProofVerifier {
    /// Create a new proof verifier
    pub fn new() -> Self {
        Self { _params: () }
    }

    /// Verify a segment proof
    pub fn verify_proof(&self, proof: &SegmentProof) -> Result<bool> {
        proof.verify()
    }

    /// Batch verify multiple proofs
    pub fn batch_verify(&self, proofs: &[SegmentProof]) -> Result<Vec<bool>> {
        proofs.iter().map(|p| p.verify()).collect()
    }
}

impl Default for ProofVerifier {
    fn default() -> Self {
        Self::new()
    }
}

/// Merkle tree builder
///
/// Future: Replace with proper NMT implementation while keeping the same
/// high-level proof API.
pub struct MerkleTreeBuilder {
    leaves: Vec<[u8; 32]>,
}

impl MerkleTreeBuilder {
    /// Create a new Merkle tree builder
    pub fn new() -> Self {
        Self { leaves: Vec::new() }
    }

    /// Add a leaf
    pub fn add_leaf(&mut self, data: &[u8]) {
        self.leaves.push(hash_leaf(data));
    }

    /// Add a pre-hashed leaf.
    pub fn add_hashed_leaf(&mut self, hash: [u8; 32]) {
        self.leaves.push(hash);
    }

    /// Build the tree and return root
    pub fn build(&self) -> [u8; 32] {
        compute_merkle_root_from_leaves(&self.leaves)
    }

    /// Get Merkle proof for a leaf index
    pub fn get_proof(&self, index: usize) -> Vec<[u8; 32]> {
        if index >= self.leaves.len() {
            return vec![];
        }

        let mut proof = Vec::new();
        let mut current_index = index;
        let mut level = self.leaves.clone();

        while level.len() > 1 {
            let sibling_index = if current_index % 2 == 0 { current_index + 1 } else { current_index - 1 };
            if sibling_index < level.len() {
                proof.push(level[sibling_index]);
            }

            level = build_next_level(&level);
            current_index /= 2;
        }

        proof
    }
}

impl Default for MerkleTreeBuilder {
    fn default() -> Self {
        Self::new()
    }
}

fn hash_leaf(data: &[u8]) -> [u8; 32] {
    let mut hasher = blake3::Hasher::new();
    hasher.update(b"spora-segment/leaf");
    hasher.update(data);
    *hasher.finalize().as_bytes()
}

fn hash_internal(left: &[u8; 32], right: &[u8; 32]) -> [u8; 32] {
    let mut hasher = blake3::Hasher::new();
    hasher.update(b"spora-segment/node");
    hasher.update(left);
    hasher.update(right);
    *hasher.finalize().as_bytes()
}

fn build_next_level(level: &[[u8; 32]]) -> Vec<[u8; 32]> {
    let mut next_level = Vec::with_capacity(level.len().div_ceil(2));
    for chunk in level.chunks(2) {
        let hash = if chunk.len() == 2 { hash_internal(&chunk[0], &chunk[1]) } else { chunk[0] };
        next_level.push(hash);
    }
    next_level
}

pub fn compute_merkle_root_from_leaves(leaves: &[[u8; 32]]) -> [u8; 32] {
    if leaves.is_empty() {
        return [0u8; 32];
    }

    let mut level = leaves.to_vec();
    while level.len() > 1 {
        level = build_next_level(&level);
    }
    level[0]
}

/// Compute a segment root from ordered data chunks.
pub fn compute_segment_root(chunks: &[Vec<u8>]) -> [u8; 32] {
    let mut builder = MerkleTreeBuilder::new();
    for chunk in chunks {
        builder.add_leaf(chunk);
    }
    builder.build()
}

/// Verify Merkle proof
pub fn verify_merkle_proof(leaf: &[u8; 32], proof: &[[u8; 32]], root: &[u8; 32], index: usize) -> bool {
    let mut current = *leaf;
    let mut current_index = index;

    for sibling in proof {
        current = if current_index % 2 == 0 { hash_internal(&current, sibling) } else { hash_internal(sibling, &current) };
        current_index /= 2;
    }

    &current == root
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_merkle_tree_builder() {
        let mut builder = MerkleTreeBuilder::new();

        builder.add_leaf(b"data1");
        builder.add_leaf(b"data2");
        builder.add_leaf(b"data3");
        builder.add_leaf(b"data4");

        let root = builder.build();

        // Root should be non-zero
        assert_ne!(root, [0u8; 32]);
    }

    #[test]
    fn test_merkle_tree_single_leaf() {
        let mut builder = MerkleTreeBuilder::new();
        builder.add_leaf(b"single");

        let root = builder.build();
        let expected = hash_leaf(b"single");

        assert_eq!(root, expected);
    }

    #[test]
    fn test_segment_proof_creation() {
        let proof = SegmentProof::new(0, 0, vec![0xAA; 1024], 0, 1024, [0x42; 32]);

        assert_eq!(proof.segment_id, 0);
        assert_eq!(proof.leaf_index, 0);
        assert_eq!(proof.chunk_length, 1024);
    }

    #[test]
    fn test_proof_verification() {
        let chunk = vec![0xBB; 512];
        let mut builder = MerkleTreeBuilder::new();
        builder.add_leaf(&chunk);
        let proof = SegmentProof::new(0, 0, chunk, 0, 512, builder.build());

        assert!(proof.verify().unwrap());
    }

    #[test]
    fn test_proof_verifier() {
        let verifier = ProofVerifier::new();
        let chunk = vec![0xCC; 256];
        let mut builder = MerkleTreeBuilder::new();
        builder.add_leaf(&chunk);
        let proof = SegmentProof::new(0, 0, chunk, 0, 256, builder.build());

        assert!(verifier.verify_proof(&proof).unwrap());
    }

    #[test]
    fn test_batch_verify() {
        let verifier = ProofVerifier::new();

        let proofs = [vec![0xAA; 128], vec![0xBB; 256], vec![0xCC; 512]]
            .into_iter()
            .enumerate()
            .map(|(segment_id, chunk)| {
                let mut builder = MerkleTreeBuilder::new();
                builder.add_leaf(&chunk);
                SegmentProof::new(segment_id as u32, 0, chunk.clone(), 0, chunk.len() as u32, builder.build())
            })
            .collect::<Vec<_>>();

        let results = verifier.batch_verify(&proofs).unwrap();
        assert_eq!(results.len(), 3);
        assert!(results.iter().all(|&r| r));
    }

    #[test]
    fn test_merkle_proof_roundtrip() {
        let chunks = [b"a".to_vec(), b"b".to_vec(), b"c".to_vec(), b"d".to_vec()];
        let mut builder = MerkleTreeBuilder::new();
        for chunk in &chunks {
            builder.add_leaf(chunk);
        }

        let proof = builder.get_proof(2);
        let leaf = hash_leaf(&chunks[2]);
        let root = builder.build();
        assert!(verify_merkle_proof(&leaf, &proof, &root, 2));
    }

    #[test]
    fn test_segment_proof_verification_with_variable_sized_chunks_uses_leaf_index() {
        let chunks = [vec![0x11; 128], vec![0x22; 512], vec![0x33; 96]];
        let mut builder = MerkleTreeBuilder::new();
        for chunk in &chunks {
            builder.add_leaf(chunk);
        }

        let mut proof = SegmentProof::new(7, 1, chunks[1].clone(), 128, chunks[1].len() as u32, builder.build());
        proof.merkle_path = builder.get_proof(1);

        assert!(proof.verify().unwrap());
    }
}
