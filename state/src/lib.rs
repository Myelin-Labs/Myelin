// SPDX-License-Identifier: MIT
// Copyright (C) 2026 Spora developers
//
// Cell state management and data availability storage

//! Cell State Layer
//!
//! This crate implements the state layer for Cell transactions, including:
//! - **Cell indexing**: CellDB (OutPoint → CellMeta)
//! - **Script indexing**: ScriptIndex (lock_hash/type_hash → Cells)
//! - **DA storage**: Segment files with NMT/KZG commitments
//! - **Sampling proofs**: Data availability verification

#![allow(missing_docs)]

pub mod cell_tree;
pub mod index;
pub mod store;

pub use cell_tree::{CellEntry, CellStateTree};
pub use index::{CellDB, CellMeta as IndexedCellMeta, ScriptIndex, SegmentInfo};
pub use store::{compute_segment_root, MerkleTreeBuilder, SegmentMeta, SegmentProof, SegmentReader, SegmentWriter};

/// Cell state errors
#[derive(Debug, thiserror::Error)]
pub enum StateError {
    /// Database error
    #[error("Database error: {0}")]
    Database(String),

    /// Cell not found
    #[error("Cell not found: {0:?}")]
    CellNotFound([u8; 32]),

    /// Segment not found
    #[error("Segment not found: {0}")]
    SegmentNotFound(u32),

    /// Invalid proof
    #[error("Invalid proof: {0}")]
    InvalidProof(String),

    /// I/O error
    #[error("I/O error: {0}")]
    Io(#[from] std::io::Error),

    /// Serialization error
    #[error("Serialization error: {0}")]
    Serialization(String),
}

/// Result type for state operations
pub type Result<T> = std::result::Result<T, StateError>;
