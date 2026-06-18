// SPDX-License-Identifier: MIT
// Copyright (C) 2026 Spora developers
//
// Data availability storage: segments and proofs

pub mod proof;
pub mod segment;

pub use proof::{compute_segment_root, MerkleTreeBuilder, ProofVerifier, SegmentProof};
pub use segment::{SegmentMeta, SegmentReader, SegmentWriter};
