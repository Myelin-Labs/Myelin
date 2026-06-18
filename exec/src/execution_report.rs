// SPDX-License-Identifier: MIT
// Copyright (C) 2026 Myelin developers
//
// Deterministic execution reports for Myelin Cell transactions.

//! Deterministic CellTx execution reports.
//!
//! This module records the non-contextual execution artefacts that are known
//! before a full VM/context run: consumed Cells, created Cells, witness hashes,
//! typed scheduler conflict hashes, capacity checks, and a deterministic state
//! transition commitment.

use crate::{
    celltx::{compute_typed_data_hash, CellTx, OutPoint},
    projection::{project_cell_tx_to_ckb, CkbProjectionReport, SemanticProfile},
};
use serde::{Deserialize, Serialize};

const STATE_TRANSITION_DOMAIN: &[u8] = b"myelin:celltx-execution-report:state-transition:v1";

/// Non-contextual execution status.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum ExecutionReportStatus {
    /// The CellTx passed non-contextual report checks.
    Accepted,
    /// The CellTx failed non-contextual report checks.
    Rejected {
        /// Rejection reasons.
        reasons: Vec<String>,
    },
}

/// Deterministic execution report for a CellTx.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct CellTxExecutionReport {
    /// Status from non-contextual checks.
    pub status: ExecutionReportStatus,
    /// Myelin transaction id.
    pub txid: [u8; 32],
    /// Consumed input OutPoints.
    pub consumed_cells: Vec<OutPoint>,
    /// Number of created output Cells.
    pub created_cell_count: usize,
    /// Hashes of output data paired with outputs.
    pub typed_data_hashes: Vec<[u8; 32]>,
    /// Hashes of witnesses.
    pub witness_hashes: Vec<[u8; 32]>,
    /// Conflict hashes declared through admitted CellScript scheduler witnesses.
    pub conflict_hashes: Vec<[u8; 32]>,
    /// Scheduler report commitment for this non-contextual report.
    pub scheduler_report_hash: [u8; 32],
    /// State root before the transition.
    pub state_root_before: [u8; 32],
    /// Deterministic state-transition commitment after this CellTx.
    pub state_root_after: [u8; 32],
    /// Semantic profile assigned by the CKB projection pass.
    pub semantic_profile: SemanticProfile,
    /// CKB-style projection report.
    pub ckb_projection: CkbProjectionReport,
}

/// Build a deterministic non-contextual execution report for a CellTx.
pub fn build_cell_tx_execution_report(tx: &CellTx, state_root_before: [u8; 32]) -> CellTxExecutionReport {
    let txid = tx.id();
    let mut rejection_reasons = Vec::new();

    if tx.outputs.len() != tx.outputs_data.len() {
        rejection_reasons.push(format!(
            "outputs/output-data length mismatch: {} outputs, {} data entries",
            tx.outputs.len(),
            tx.outputs_data.len()
        ));
    }

    for (index, (output, data)) in tx.outputs.iter().zip(tx.outputs_data.iter()).enumerate() {
        if let Err(error) = output.verify_capacity(data.len()) {
            rejection_reasons.push(format!("output {index} capacity error: {error}"));
        }
    }

    let admitted_scheduler_witnesses = tx.admitted_cellscript_scheduler_witnesses().collect::<Result<Vec<_>, _>>();
    let conflict_hashes = match admitted_scheduler_witnesses {
        Ok(witnesses) => witnesses
            .into_iter()
            .flat_map(|witness| witness.accesses.into_iter().map(|access| access.conflict_hash))
            .collect::<Vec<_>>(),
        Err(error) => {
            rejection_reasons.push(format!("scheduler witness error: {error}"));
            Vec::new()
        }
    };

    let consumed_cells = tx.inputs.iter().map(|input| input.previous_output).collect::<Vec<_>>();
    let typed_data_hashes = tx
        .outputs
        .iter()
        .zip(tx.outputs_data.iter())
        .map(|(output, data)| {
            output.type_.as_ref().map_or_else(|| blake3_hash(data), |type_script| compute_typed_data_hash(type_script, data))
        })
        .collect::<Vec<_>>();
    let witness_hashes = tx.witnesses.iter().map(|witness| blake3_hash(witness)).collect::<Vec<_>>();
    let scheduler_report_hash = scheduler_report_hash(txid, &conflict_hashes);
    let state_root_after = state_transition_root(state_root_before, txid, &typed_data_hashes, &witness_hashes, scheduler_report_hash);
    let ckb_projection = project_cell_tx_to_ckb(tx);
    let semantic_profile = ckb_projection.semantic_profile;
    let status = if rejection_reasons.is_empty() {
        ExecutionReportStatus::Accepted
    } else {
        ExecutionReportStatus::Rejected { reasons: rejection_reasons }
    };

    CellTxExecutionReport {
        status,
        txid,
        consumed_cells,
        created_cell_count: tx.outputs.len(),
        typed_data_hashes,
        witness_hashes,
        conflict_hashes,
        scheduler_report_hash,
        state_root_before,
        state_root_after,
        semantic_profile,
        ckb_projection,
    }
}

fn blake3_hash(bytes: &[u8]) -> [u8; 32] {
    *blake3::hash(bytes).as_bytes()
}

fn scheduler_report_hash(txid: [u8; 32], conflict_hashes: &[[u8; 32]]) -> [u8; 32] {
    let mut hasher = blake3::Hasher::new();
    hasher.update(b"myelin:celltx-execution-report:scheduler:v1");
    hasher.update(&txid);
    hasher.update(&(conflict_hashes.len() as u32).to_le_bytes());
    for conflict_hash in conflict_hashes {
        hasher.update(conflict_hash);
    }
    *hasher.finalize().as_bytes()
}

fn state_transition_root(
    state_root_before: [u8; 32],
    txid: [u8; 32],
    typed_data_hashes: &[[u8; 32]],
    witness_hashes: &[[u8; 32]],
    scheduler_report_hash: [u8; 32],
) -> [u8; 32] {
    let mut hasher = blake3::Hasher::new();
    hasher.update(STATE_TRANSITION_DOMAIN);
    hasher.update(&state_root_before);
    hasher.update(&txid);
    hasher.update(&(typed_data_hashes.len() as u32).to_le_bytes());
    for typed_data_hash in typed_data_hashes {
        hasher.update(typed_data_hash);
    }
    hasher.update(&(witness_hashes.len() as u32).to_le_bytes());
    for witness_hash in witness_hashes {
        hasher.update(witness_hash);
    }
    hasher.update(&scheduler_report_hash);
    *hasher.finalize().as_bytes()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::celltx::{CellInput, CellOutput, OutPoint, Script};

    fn script(byte: u8) -> Script {
        Script::new([byte; 32], 1, vec![byte])
    }

    #[test]
    fn report_accepts_simple_cell_tx() {
        let tx = CellTx::new(
            vec![CellInput::new(OutPoint::new([1; 32], 0), 0)],
            vec![],
            vec![CellOutput { lock: script(2), type_: None, capacity: 100 }],
            vec![vec![0xAA]],
            vec![vec![0xBB]],
        )
        .unwrap();
        let report = build_cell_tx_execution_report(&tx, [0; 32]);
        assert_eq!(report.status, ExecutionReportStatus::Accepted);
        assert_eq!(report.consumed_cells.len(), 1);
        assert_eq!(report.created_cell_count, 1);
        assert_eq!(report.semantic_profile, SemanticProfile::CkbCompatible);
    }

    #[test]
    fn report_rejects_mismatched_output_data() {
        let mut tx =
            CellTx::new(vec![], vec![], vec![CellOutput { lock: script(2), type_: None, capacity: 100 }], vec![vec![0xAA]], vec![])
                .unwrap();
        tx.outputs_data.clear();

        let report = build_cell_tx_execution_report(&tx, [0; 32]);
        assert!(matches!(report.status, ExecutionReportStatus::Rejected { .. }));
        assert_eq!(report.semantic_profile, SemanticProfile::CkbInspiredOnly);
    }
}
