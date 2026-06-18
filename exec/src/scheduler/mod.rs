// SPDX-License-Identifier: MIT
// Copyright (C) 2026 Spora developers
//
// Parallel transaction scheduler with RW-Set DAG

//! Parallel Transaction Scheduler
//!
//! This module implements parallel execution of Cell transactions:
//! - **DAG Construction**: RW-Set → CellDAG (dependency and conflict edges)
//! - **Conflict Resolution**: Deterministic ordering (fee_density, wtxid)
//! - **Parallel Execution**: Topological layering with Rayon

/// Conflict resolution module
pub mod conflict;
/// DAG construction module
pub mod dag;
/// Parallel executor module
pub mod executor;

pub use conflict::{ConflictKey, ConflictResolution, ConflictResolver};
pub use dag::{AccessMode, CellDAG, ConflictEntry, DagEdge, DagNode};
pub use executor::{ExecutionReceipt, ExecutionResult, ParallelExecutor};
