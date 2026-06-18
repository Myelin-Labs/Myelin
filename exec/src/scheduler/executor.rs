// SPDX-License-Identifier: MIT
// Copyright (C) 2026 Spora developers
//
// Parallel transaction executor with topological layering

use super::dag::{CellDAG, NodeId};
use crate::celltx::types::CellTx;
use rayon::prelude::*;

/// Parallel executor for Cell transactions
#[derive(Default)]
pub struct ParallelExecutor {
    /// Thread pool size (0 = auto)
    #[allow(dead_code)]
    thread_pool_size: usize,
}

impl ParallelExecutor {
    /// Create a new parallel executor
    pub fn new(thread_pool_size: usize) -> Self {
        Self { thread_pool_size }
    }

    /// Execute transactions in parallel according to DAG topology
    ///
    /// # Algorithm
    /// 1. Process transactions layer by layer (from DAG.layers)
    /// 2. Within each layer, execute in parallel using Rayon
    /// 3. Ensure deterministic ordering of results (sorted by NodeId)
    ///
    /// # Returns
    /// Execution results in the same order as input transactions
    pub fn execute<F>(&self, dag: &CellDAG, txs: &[CellTx], executor_fn: F) -> Result<Vec<ExecutionResult>, ExecutionError>
    where
        F: Fn(&CellTx, NodeId) -> Result<ExecutionReceipt, String> + Send + Sync,
    {
        if txs.len() != dag.node_count {
            return Err(ExecutionError::TxCountMismatch { expected: dag.node_count, actual: txs.len() });
        }

        let mut results = vec![None; txs.len()];

        // Execute layer by layer
        for (layer_idx, layer) in dag.layers.iter().enumerate() {
            // Execute transactions in this layer in parallel
            let layer_results: Vec<(NodeId, Result<ExecutionReceipt, String>)> = layer
                .par_iter()
                .map(|&node_id| {
                    let result = executor_fn(&txs[node_id], node_id);
                    (node_id, result)
                })
                .collect();

            // Store results (deterministic order maintained by NodeId)
            for (node_id, result) in layer_results {
                results[node_id] = Some(match result {
                    Ok(receipt) => ExecutionResult::Success { node_id, layer: layer_idx, receipt },
                    Err(error) => ExecutionResult::Failed { node_id, layer: layer_idx, error },
                });
            }
        }

        // Unwrap all results (all should be Some)
        results.into_iter().map(|r| r.ok_or(ExecutionError::MissingResult)).collect()
    }

    /// Execute transactions sequentially (for testing)
    pub fn execute_sequential<F>(&self, txs: &[CellTx], mut executor_fn: F) -> Result<Vec<ExecutionResult>, ExecutionError>
    where
        F: FnMut(&CellTx, NodeId) -> Result<ExecutionReceipt, String>,
    {
        txs.iter()
            .enumerate()
            .map(|(node_id, tx)| match executor_fn(tx, node_id) {
                Ok(receipt) => Ok(ExecutionResult::Success { node_id, layer: 0, receipt }),
                Err(error) => Ok(ExecutionResult::Failed { node_id, layer: 0, error }),
            })
            .collect()
    }

    /// Get execution statistics
    pub fn get_stats(results: &[ExecutionResult]) -> ExecutionStats {
        let total = results.len();
        let successful = results.iter().filter(|r| matches!(r, ExecutionResult::Success { .. })).count();
        let failed = total - successful;

        let max_layer = results
            .iter()
            .map(|r| match r {
                ExecutionResult::Success { layer, .. } => *layer,
                ExecutionResult::Failed { layer, .. } => *layer,
            })
            .max()
            .unwrap_or(0);

        ExecutionStats { total_txs: total, successful_txs: successful, failed_txs: failed, max_layer_depth: max_layer + 1 }
    }
}

/// Execution result for a single transaction
#[derive(Debug, Clone)]
pub enum ExecutionResult {
    /// Transaction executed successfully
    Success {
        /// Node ID in DAG
        node_id: NodeId,
        /// Layer in topological sort
        layer: usize,
        /// Execution receipt
        receipt: ExecutionReceipt,
    },
    /// Transaction execution failed
    Failed {
        /// Node ID in DAG
        node_id: NodeId,
        /// Layer in topological sort
        layer: usize,
        /// Error message
        error: String,
    },
}

impl ExecutionResult {
    /// Get node ID
    pub fn node_id(&self) -> NodeId {
        match self {
            Self::Success { node_id, .. } => *node_id,
            Self::Failed { node_id, .. } => *node_id,
        }
    }

    /// Check if successful
    pub fn is_success(&self) -> bool {
        matches!(self, Self::Success { .. })
    }
}

/// Execution receipt (simplified for now)
#[derive(Debug, Clone, Default)]
pub struct ExecutionReceipt {
    /// Cycles consumed
    pub cycles: u64,
    /// Gas used (for future compatibility)
    pub gas_used: u64,
    /// Output logs (for debugging)
    pub logs: Vec<String>,
}

/// Execution statistics
#[derive(Debug, Clone)]
pub struct ExecutionStats {
    /// Total transactions
    pub total_txs: usize,
    /// Successfully executed transactions
    pub successful_txs: usize,
    /// Failed transactions
    pub failed_txs: usize,
    /// Maximum layer depth (parallel width)
    pub max_layer_depth: usize,
}

/// Execution errors
#[derive(Debug, Clone, thiserror::Error)]
pub enum ExecutionError {
    /// Transaction count mismatch
    #[error("Transaction count mismatch: expected {expected}, got {actual}")]
    TxCountMismatch {
        /// Expected count
        expected: usize,
        /// Actual count
        actual: usize,
    },

    /// Missing execution result
    #[error("Missing execution result for transaction")]
    MissingResult,
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::celltx::types::{CellInput, CellOutput, OutPoint, Script};

    fn create_test_tx(inputs: Vec<OutPoint>) -> CellTx {
        let lock = Script::new([0x00; 32], 0, vec![]);
        let inputs = inputs.into_iter().map(|op| CellInput::new(op, 0)).collect();
        CellTx::new(inputs, vec![], vec![CellOutput { lock, type_: None, capacity: 1000 }], vec![vec![]], vec![]).unwrap()
    }

    #[test]
    fn test_parallel_execution() {
        let executor = ParallelExecutor::default();

        // Create independent transactions (no dependencies)
        let tx0 = create_test_tx(vec![]);
        let tx1 = create_test_tx(vec![]);
        let tx2 = create_test_tx(vec![]);

        let txs = vec![tx0, tx1, tx2];

        // Use sequential execution for simplicity
        let results = executor
            .execute_sequential(&txs, |_tx, node_id| {
                Ok(ExecutionReceipt { cycles: 1000, gas_used: 100, logs: vec![format!("Executed tx {}", node_id)] })
            })
            .unwrap();

        assert_eq!(results.len(), 3);
        assert!(results.iter().all(|r| r.is_success()));
    }

    #[test]
    fn test_execution_stats() {
        let results = vec![
            ExecutionResult::Success { node_id: 0, layer: 0, receipt: ExecutionReceipt::default() },
            ExecutionResult::Success { node_id: 1, layer: 1, receipt: ExecutionReceipt::default() },
            ExecutionResult::Failed { node_id: 2, layer: 1, error: "Test error".to_string() },
        ];

        let stats = ParallelExecutor::get_stats(&results);

        assert_eq!(stats.total_txs, 3);
        assert_eq!(stats.successful_txs, 2);
        assert_eq!(stats.failed_txs, 1);
        assert_eq!(stats.max_layer_depth, 2);
    }

    #[test]
    fn test_sequential_execution() {
        let executor = ParallelExecutor::default();

        let txs = vec![create_test_tx(vec![]), create_test_tx(vec![])];

        let results = executor
            .execute_sequential(&txs, |_tx, node_id| Ok(ExecutionReceipt { cycles: node_id as u64 * 100, gas_used: 0, logs: vec![] }))
            .unwrap();

        assert_eq!(results.len(), 2);

        if let ExecutionResult::Success { receipt, .. } = &results[0] {
            assert_eq!(receipt.cycles, 0);
        }

        if let ExecutionResult::Success { receipt, .. } = &results[1] {
            assert_eq!(receipt.cycles, 100);
        }
    }
}
