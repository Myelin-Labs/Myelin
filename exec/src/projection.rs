// SPDX-License-Identifier: MIT
// Copyright (C) 2026 Myelin developers

//! Evidence-staged CKB projection reports.

use crate::{
    celltx::{compute_txid, CellTx},
    serialization::molecule_compat::{
        ckb_raw_transaction_hash_molecule, ckb_transaction_witness_hash_molecule, serialize_transaction_molecule,
    },
    CELL_TX_VERSION,
};
use serde::{Deserialize, Serialize};

/// Highest CKB-alignment stage supported by concrete evidence.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum ProjectionStage {
    /// Wire or invariant validation failed.
    Rejected,
    /// Canonical CKB Molecule bytes and hashes were produced.
    WireEncoded,
    /// Every input, cell dep and header dep was resolved.
    ContextResolved,
    /// Contextual CKB consensus checks passed.
    ConsensusValidated,
    /// All CKB script groups passed under a shared cycle budget.
    ScriptsVerified,
    /// A CKB node accepted the exact transaction through RPC.
    NodeAccepted,
    /// The exact transaction has a node-verified inclusion proof in a canonical block.
    Committed,
    /// The committed block remained canonical through the configured confirmation depth.
    Finalized,
}

/// A condition that invalidates the claimed stage.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum ProjectionBlocker {
    /// Transaction version is not CKB version zero.
    NonCkbTransactionVersion {
        /// Actual transaction version.
        actual: u32,
    },
    /// Output and output-data lengths differ.
    OutputsDataLengthMismatch {
        /// Number of outputs.
        outputs: usize,
        /// Number of output data entries.
        outputs_data: usize,
    },
    /// Molecule transaction encoding failed.
    MoleculeEncodingFailed {
        /// Encoding error.
        error: String,
    },
    /// Raw transaction hashing failed.
    RawTransactionHashFailed {
        /// Hashing error.
        error: String,
    },
    /// Witness-inclusive hashing failed.
    WitnessTransactionHashFailed {
        /// Hashing error.
        error: String,
    },
}

/// Non-fatal context note.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum ProjectionWarning {
    /// No witnesses are present; scripts may still be valid depending on locks.
    EmptyWitnessSet,
    /// No inputs are present, so a cellbase/genesis context is required.
    CellbaseStyleContext,
}

/// Evidence-staged projection report for one exact transaction.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct CkbProjectionReport {
    /// Raw Myelin/CKB transaction id.
    pub source_txid: [u8; 32],
    /// Highest stage actually supported by evidence.
    pub stage: ProjectionStage,
    /// Conditions invalidating the claimed stage.
    pub blockers: Vec<ProjectionBlocker>,
    /// Non-fatal notes.
    pub warnings: Vec<ProjectionWarning>,
    /// Number of inputs.
    pub input_count: usize,
    /// Number of cell dependencies.
    pub cell_dep_count: usize,
    /// Number of header dependencies.
    pub header_dep_count: usize,
    /// Number of outputs.
    pub output_count: usize,
    /// Number of witnesses.
    pub witness_count: usize,
    /// Size of canonical Molecule transaction bytes.
    pub molecule_transaction_bytes: Option<usize>,
    /// CKB raw transaction hash.
    pub ckb_raw_tx_hash: Option<[u8; 32]>,
    /// Witness-inclusive CKB transaction hash.
    pub ckb_wtx_hash: Option<[u8; 32]>,
}

impl CkbProjectionReport {
    /// True only after strict script verification, not merely wire encoding.
    pub fn scripts_verified(&self) -> bool {
        self.stage >= ProjectionStage::ScriptsVerified
    }

    /// True only when a node accepted the exact transaction.
    pub fn node_accepted(&self) -> bool {
        self.stage >= ProjectionStage::NodeAccepted
    }
}

/// Produce a wire-stage report. This function never claims contextual or
/// executable CKB compatibility.
pub fn project_cell_tx_to_ckb(tx: &CellTx) -> CkbProjectionReport {
    let mut blockers = Vec::new();
    let mut warnings = Vec::new();
    if tx.version != CELL_TX_VERSION {
        blockers.push(ProjectionBlocker::NonCkbTransactionVersion { actual: tx.version });
    }
    if tx.outputs.len() != tx.outputs_data.len() {
        blockers.push(ProjectionBlocker::OutputsDataLengthMismatch { outputs: tx.outputs.len(), outputs_data: tx.outputs_data.len() });
    }
    if tx.witnesses.is_empty() {
        warnings.push(ProjectionWarning::EmptyWitnessSet);
    }
    if tx.inputs.is_empty() {
        warnings.push(ProjectionWarning::CellbaseStyleContext);
    }
    let molecule_transaction_bytes = match serialize_transaction_molecule(tx) {
        Ok(bytes) => Some(bytes.len()),
        Err(error) => {
            blockers.push(ProjectionBlocker::MoleculeEncodingFailed { error: error.to_string() });
            None
        }
    };
    let ckb_raw_tx_hash = match ckb_raw_transaction_hash_molecule(tx) {
        Ok(hash) => Some(hash),
        Err(error) => {
            blockers.push(ProjectionBlocker::RawTransactionHashFailed { error: error.to_string() });
            None
        }
    };
    let ckb_wtx_hash = match ckb_transaction_witness_hash_molecule(tx) {
        Ok(hash) => Some(hash),
        Err(error) => {
            blockers.push(ProjectionBlocker::WitnessTransactionHashFailed { error: error.to_string() });
            None
        }
    };

    // This module currently owns only canonical wire encoding. Higher stages
    // deliberately have no boolean escape hatch: they will require opaque
    // receipts from contextual validation, strict VM verification, and a node.
    let stage = if blockers.is_empty() { ProjectionStage::WireEncoded } else { ProjectionStage::Rejected };

    CkbProjectionReport {
        source_txid: compute_txid(tx),
        stage,
        blockers,
        warnings,
        input_count: tx.inputs.len(),
        cell_dep_count: tx.cell_deps.len(),
        header_dep_count: tx.header_deps.len(),
        output_count: tx.outputs.len(),
        witness_count: tx.witnesses.len(),
        molecule_transaction_bytes,
        ckb_raw_tx_hash,
        ckb_wtx_hash,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::celltx::{CellInput, CellOutput, OutPoint, Script};

    fn tx() -> CellTx {
        CellTx::new(
            vec![CellInput::new(OutPoint::new([1; 32], 0), 0)],
            vec![],
            vec![CellOutput { lock: Script::new([2; 32], 1, vec![]), type_: None, capacity: 100 }],
            vec![vec![]],
            vec![vec![0xCC]],
        )
        .unwrap()
    }

    #[test]
    fn encoding_alone_claims_only_wire_stage() {
        let report = project_cell_tx_to_ckb(&tx());
        assert_eq!(report.stage, ProjectionStage::WireEncoded);
        assert!(!report.scripts_verified());
        assert!(report.blockers.is_empty());
    }

    #[test]
    fn nonzero_version_is_a_blocker() {
        let mut transaction = tx();
        transaction.version = 1;
        let report = project_cell_tx_to_ckb(&transaction);
        assert_eq!(report.stage, ProjectionStage::Rejected);
        assert!(matches!(report.blockers.first(), Some(ProjectionBlocker::NonCkbTransactionVersion { actual: 1 })));
    }

    #[test]
    fn witness_changes_only_witness_hash() {
        let tx_a = tx();
        let mut tx_b = tx_a.clone();
        tx_b.witnesses[0] = vec![0xDD];
        let a = project_cell_tx_to_ckb(&tx_a);
        let b = project_cell_tx_to_ckb(&tx_b);
        assert_eq!(a.ckb_raw_tx_hash, b.ckb_raw_tx_hash);
        assert_ne!(a.ckb_wtx_hash, b.ckb_wtx_hash);
    }
}
