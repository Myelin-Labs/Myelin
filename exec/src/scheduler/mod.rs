// SPDX-License-Identifier: MIT
// Copyright (C) 2026 Myelin developers
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
/// Bridge between the CellDAG scheduler and the CKB-VM verifier.
pub mod vm_bridge;

pub use conflict::{ConflictKey, ConflictResolution, ConflictResolver};
pub use dag::{AccessMode, CellDAG, ConflictEntry, DagEdge, DagNode};
pub use executor::{ExecutionError, ExecutionReceipt, ExecutionResult, ExecutionStats, ParallelExecutor};
pub use vm_bridge::{verify_celltx_via_dag, verify_with_existing_dag};
