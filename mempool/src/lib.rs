// SPDX-License-Identifier: MIT
// Copyright (C) 2026 Spora developers
//
// Cell transaction memory pool

//! Cell Memory Pool
//!
//! This crate implements the memory pool for Cell transactions:
//! - **CellPool**: Transaction storage and dependency tracking
//! - **Scorer**: Priority scoring (fee_density, unlockability, deps)
//! - **RBF/CPFP**: Replace-by-fee and child-pays-for-parent
//! - **Relay**: Transaction propagation

#![warn(missing_docs)]

/// Cell pool implementation
pub mod cellpool;
/// Transaction scoring module
pub mod scorer;

pub use cellpool::{CellPool, PoolEntry, PoolStats};
pub use scorer::{TransactionScore, TransactionScorer};

/// Mempool errors
#[derive(Debug, thiserror::Error)]
pub enum MempoolError {
    /// Transaction already exists
    #[error("Transaction already exists: {0:?}")]
    TxExists([u8; 32]),

    /// Transaction not found
    #[error("Transaction not found: {0:?}")]
    TxNotFound([u8; 32]),

    /// Invalid transaction
    #[error("Invalid transaction: {0}")]
    InvalidTx(String),

    /// Mempool full
    #[error("Mempool full (max: {0})")]
    MempoolFull(usize),

    /// Dependency not found
    #[error("Dependency not found: {0:?}")]
    DependencyNotFound([u8; 32]),

    /// RBF failed
    #[error("RBF failed: {0}")]
    RBFFailed(String),
}

/// Result type for mempool operations
pub type Result<T> = std::result::Result<T, MempoolError>;
