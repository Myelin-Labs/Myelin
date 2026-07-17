// SPDX-License-Identifier: MIT
// Copyright (C) 2026 Myelin developers
//
// Bridge between the CellDAG parallel scheduler and the CKB-VM verifier.
//
// This is the integration point that turns the DAG from a metadata structure
// into a real inter-transaction parallel verifier. It builds a CellDAG over a
// batch of CellTx, then runs `TransactionScriptVerifier::verify_with_cycles`
// per transaction through `ParallelExecutor::execute` (rayon across
// topological layers). No VM change is involved — the verifier is already
// `Send + Sync` when `D: CellDataProvider`, so it is capturable in the
// `Fn + Send + Sync` closure that `ParallelExecutor::execute` expects.

use crate::celltx::types::CellTx;
use crate::vm::{CellDataProvider, TransactionScriptVerifier};
use std::sync::Arc;

use super::dag::CellDAG;
use super::executor::{ExecutionError, ExecutionReceipt, ExecutionResult, ParallelExecutor};

/// Verify a batch of Cell transactions through the real CKB-VM verifier,
/// scheduled in parallel by the CellDAG.
///
/// The DAG is built with `CellDAG::build_from_typed` so that scheduler
/// witnesses (when present) add typed conflict-hash edges on top of the
/// OutPoint-level dependencies. Each transaction is then verified via
/// `TransactionScriptVerifier::verify_with_cycles`, which itself already
/// parallelizes across the script groups of a single transaction. This
/// function adds inter-transaction parallelism on top of that.
///
/// `skip_lock_groups` controls whether lock-script groups are executed. The
/// CellDAG is the authority on inter-tx ordering, so callers that only need
/// to prove ordering (and whose lock scripts are placeholders) pass `true`;
/// callers that want a full VM run (e.g. the multi-tx demo with a real
/// `always_success` lock) pass `false` to get positive cycle counts.
///
/// Returns one `ExecutionResult` per input transaction, in input order.
pub fn verify_celltx_via_dag<D: CellDataProvider>(
    txs: &[CellTx],
    data_provider: Arc<D>,
    max_cycles: u64,
    skip_lock_groups: bool,
) -> Result<Vec<ExecutionResult>, ExecutionError> {
    let dag = CellDAG::build_from_typed(txs).map_err(|err| ExecutionError::DagBuild(err.to_string()))?;
    verify_with_existing_dag(&dag, txs, data_provider, max_cycles, skip_lock_groups)
}

/// As `verify_celltx_via_dag`, but accepts a pre-built `CellDAG` (useful when
/// the caller wants to inspect the DAG topology before executing).
pub fn verify_with_existing_dag<D: CellDataProvider>(
    dag: &CellDAG,
    txs: &[CellTx],
    data_provider: Arc<D>,
    max_cycles: u64,
    skip_lock_groups: bool,
) -> Result<Vec<ExecutionResult>, ExecutionError> {
    let executor = ParallelExecutor::default();
    executor.execute(dag, txs, |cell_tx, _node_id| {
        let verifier = TransactionScriptVerifier::new(Arc::new(cell_tx.clone()), data_provider.clone())
            .with_skip_lock_groups(skip_lock_groups)
            .with_max_cycles(max_cycles);
        let cycles = verifier.verify_with_cycles().map_err(|err| err.to_string())?;
        Ok(ExecutionReceipt { cycles, exit_code: 0, gas_used: 0, logs: Vec::new() })
    })
}
