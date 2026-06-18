// SPDX-License-Identifier: MIT
// Copyright (C) 2026 Spora developers
//
// Transaction scorer: priority computation

use spora_exec::CellTx;

/// Transaction score components
#[derive(Clone, Debug, PartialEq)]
pub struct TransactionScore {
    /// Fee density (fee / effective_size), where effective_size uses verified
    /// or projected VM cycles.
    pub fee_density: f64,

    /// Unlockability score (time lock penalty)
    pub unlockability: f64,

    /// Dependency width (number of deps)
    pub deps_width: f64,

    /// Total score (weighted combination)
    pub total: f64,
}

/// Transaction scorer
///
/// Computes priority score:
/// ```text
/// Score = α·fee_density + β·unlockability - γ·deps_width
/// ```
/// where:
/// - α = 0.6 (fee weight)
/// - β = 0.3 (unlockability weight)
/// - γ = 0.1 (dependency penalty)
pub struct TransactionScorer {
    /// Fee density weight
    alpha: f64,

    /// Unlockability weight
    beta: f64,

    /// Dependency penalty weight
    gamma: f64,

    /// Cycles per byte (for effective size)
    cycles_per_byte: f64,
}

impl Default for TransactionScorer {
    fn default() -> Self {
        Self { alpha: 0.6, beta: 0.3, gamma: 0.1, cycles_per_byte: 100.0 }
    }
}

impl TransactionScorer {
    /// Create a new scorer
    pub fn new(alpha: f64, beta: f64, gamma: f64, cycles_per_byte: f64) -> Self {
        Self { alpha, beta, gamma, cycles_per_byte }
    }

    /// Compute transaction score
    pub fn compute_score(&self, tx: &CellTx, fee: u64, cycles: u64) -> TransactionScore {
        let fee_density = self.compute_fee_density(tx, fee, cycles);
        let unlockability = self.compute_unlockability(tx);
        let deps_width = tx.cell_deps.len() as f64;

        let total = self.alpha * fee_density + self.beta * unlockability - self.gamma * deps_width;

        TransactionScore { fee_density, unlockability, deps_width, total }
    }

    /// Compute fee density (fee / effective_size)
    fn compute_fee_density(&self, tx: &CellTx, fee: u64, cycles: u64) -> f64 {
        let size = tx.serialized_size() as f64;
        let cycles_size = cycles as f64 / self.cycles_per_byte;
        let effective_size = size.max(cycles_size);

        if effective_size > 0.0 {
            fee as f64 / effective_size
        } else {
            0.0
        }
    }

    /// Compute unlockability score
    ///
    /// Penalizes transactions with time locks:
    /// - No time locks: 1.0
    /// - Relative locks: 0.5
    /// - Absolute locks: depends on lock distance
    fn compute_unlockability(&self, tx: &CellTx) -> f64 {
        if tx.inputs.is_empty() {
            return 1.0;
        }

        let mut total_score = 0.0;

        for input in &tx.inputs {
            let score = if input.since == 0 {
                1.0 // No lock
            } else if input.is_relative_lock() {
                0.5 // Relative lock (moderate penalty)
            } else {
                // Absolute lock (time-based penalty)
                let lock_value = input.lock_value();
                if lock_value > 1_000_000 {
                    // Far future lock
                    0.1
                } else {
                    // Near future lock
                    0.7
                }
            };
            total_score += score;
        }

        total_score / tx.inputs.len() as f64
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use spora_exec::{CellInput, CellOutput, OutPoint, Script};

    fn create_test_tx(num_inputs: usize, num_deps: usize) -> CellTx {
        let lock = Script::new([0x00; 32], 0, vec![0; 20]);
        let inputs = (0..num_inputs).map(|i| CellInput::new(OutPoint::new([i as u8; 32], 0), 0)).collect();
        let deps = (0..num_deps)
            .map(|i| spora_exec::CellDep { out_point: OutPoint::new([100 + i as u8; 32], 0), dep_type: spora_exec::DepType::Code })
            .collect();

        CellTx::new(inputs, deps, vec![CellOutput { lock, type_: None, capacity: 1000 }], vec![vec![]], vec![vec![0; 65]]).unwrap()
    }

    #[test]
    fn test_fee_density_computation() {
        let scorer = TransactionScorer::default();
        let tx = create_test_tx(1, 0);

        let score = scorer.compute_score(&tx, 1000, 1000);

        // Fee density should be positive
        assert!(score.fee_density > 0.0);
    }

    #[test]
    fn test_unlockability_no_lock() {
        let scorer = TransactionScorer::default();
        let tx = create_test_tx(2, 0);

        let score = scorer.compute_score(&tx, 1000, 1000);

        // No time locks: unlockability should be 1.0
        assert_eq!(score.unlockability, 1.0);
    }

    #[test]
    fn test_deps_penalty() {
        let scorer = TransactionScorer::default();

        let tx_no_deps = create_test_tx(1, 0);
        let tx_with_deps = create_test_tx(1, 5);

        let score_no_deps = scorer.compute_score(&tx_no_deps, 1000, 1000);
        let score_with_deps = scorer.compute_score(&tx_with_deps, 1000, 1000);

        // Transaction with deps should have lower score
        assert!(score_no_deps.total > score_with_deps.total);
    }

    #[test]
    fn test_higher_fee_wins() {
        let scorer = TransactionScorer::default();
        let tx = create_test_tx(1, 0);

        let score_low = scorer.compute_score(&tx, 100, 1000);
        let score_high = scorer.compute_score(&tx, 1000, 1000);

        // Higher fee should have higher score
        assert!(score_high.total > score_low.total);
    }

    #[test]
    fn test_cycles_affect_fee_density() {
        let scorer = TransactionScorer::default();
        let tx = create_test_tx(1, 0);

        let score_low_cycles = scorer.compute_score(&tx, 1000, 100);
        let score_high_cycles = scorer.compute_score(&tx, 1000, 100_000);

        // Higher cycles → lower fee density (larger effective size)
        assert!(score_low_cycles.fee_density > score_high_cycles.fee_density);
    }

}
