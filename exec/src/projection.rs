// SPDX-License-Identifier: MIT
// Copyright (C) 2026 Myelin developers
//
// CKB-style projection reports for Myelin Cell transactions.

//! CKB-style projection reports.
//!
//! A projection report answers a narrow question: can this Myelin CellTx be
//! represented as a CKB Molecule transaction/context shape? It does not execute
//! a CKB node and does not claim L1 acceptance by itself.

use crate::{
    celltx::{compute_txid, CellTx},
    serialization::molecule_compat::{
        ckb_raw_transaction_hash_molecule, ckb_transaction_witness_hash_molecule, serialize_transaction_molecule,
    },
    CELL_TX_VERSION,
};
use serde::{Deserialize, Serialize};

/// Semantic compatibility profile for a Myelin transition.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum SemanticProfile {
    /// Uses Myelin-only helper semantics or metadata.
    MyelinNative,
    /// Can be represented as a CKB-style transaction/context.
    CkbCompatible,
    /// Follows Cell-style ideas but is not projectable to CKB transaction shape.
    CkbInspiredOnly,
}

/// A condition that prevents CKB-style projection.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum ProjectionBlocker {
    /// The number of outputs and output data items differs.
    OutputsDataLengthMismatch {
        /// Number of Cell outputs.
        outputs: usize,
        /// Number of output data entries.
        outputs_data: usize,
    },
    /// The CellTx could not be serialised with the CKB Molecule transaction layout.
    MoleculeEncodingFailed {
        /// Error returned by the Molecule compatibility layer.
        error: String,
    },
    /// The CKB-style raw transaction hash could not be derived.
    RawTransactionHashFailed {
        /// Error returned by the Molecule compatibility layer.
        error: String,
    },
    /// The CKB-style full transaction witness hash could not be derived.
    WitnessTransactionHashFailed {
        /// Error returned by the Molecule compatibility layer.
        error: String,
    },
}

/// A non-fatal projection warning.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum ProjectionWarning {
    /// Myelin CellTx version is not the usual CKB transaction version.
    NonCkbTransactionVersion {
        /// Version carried by the Myelin CellTx.
        actual: u32,
        /// Version normally used for CKB transaction projection fixtures.
        ckb_fixture_version: u32,
    },
    /// The transaction carries no witness bytes.
    EmptyWitnessSet,
    /// The transaction carries no inputs and is therefore a cellbase-style context.
    CellbaseStyleContext,
}

/// CKB-style projection report for a Myelin CellTx.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct CkbProjectionReport {
    /// Myelin transaction id.
    pub source_txid: [u8; 32],
    /// Semantic profile assigned by the projection pass.
    pub semantic_profile: SemanticProfile,
    /// Whether CKB-style projection is possible.
    pub ckb_projection_possible: bool,
    /// Blockers that prevented projection.
    pub blockers: Vec<ProjectionBlocker>,
    /// Non-fatal warnings.
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
    /// Size of the CKB Molecule transaction bytes when encoding succeeds.
    pub molecule_transaction_bytes: Option<usize>,
    /// CKB-style raw transaction hash when derivation succeeds.
    pub ckb_raw_tx_hash: Option<[u8; 32]>,
    /// CKB-style transaction-with-witness hash when derivation succeeds.
    pub ckb_wtx_hash: Option<[u8; 32]>,
}

/// Build a CKB-style projection report for a CellTx.
pub fn project_cell_tx_to_ckb(tx: &CellTx) -> CkbProjectionReport {
    let mut blockers = Vec::new();
    let mut warnings = Vec::new();

    if tx.outputs.len() != tx.outputs_data.len() {
        blockers.push(ProjectionBlocker::OutputsDataLengthMismatch { outputs: tx.outputs.len(), outputs_data: tx.outputs_data.len() });
    }

    if tx.version != 0 && tx.version != CELL_TX_VERSION {
        warnings.push(ProjectionWarning::NonCkbTransactionVersion { actual: tx.version, ckb_fixture_version: 0 });
    } else if tx.version == CELL_TX_VERSION {
        warnings.push(ProjectionWarning::NonCkbTransactionVersion { actual: CELL_TX_VERSION, ckb_fixture_version: 0 });
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

    let ckb_projection_possible = blockers.is_empty();
    let semantic_profile = if ckb_projection_possible { SemanticProfile::CkbCompatible } else { SemanticProfile::CkbInspiredOnly };

    CkbProjectionReport {
        source_txid: compute_txid(tx),
        semantic_profile,
        ckb_projection_possible,
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

    fn script(byte: u8) -> Script {
        Script::new([byte; 32], 1, vec![byte, byte + 1])
    }

    #[test]
    fn simple_cell_tx_projects_to_ckb_shape() {
        let tx = CellTx::new(
            vec![CellInput::new(OutPoint::new([1; 32], 0), 0)],
            vec![],
            vec![CellOutput { lock: script(2), type_: None, capacity: 100 }],
            vec![vec![0xAA, 0xBB]],
            vec![vec![0xCC]],
        )
        .unwrap();

        let report = project_cell_tx_to_ckb(&tx);
        assert!(report.ckb_projection_possible);
        assert_eq!(report.semantic_profile, SemanticProfile::CkbCompatible);
        assert!(report.blockers.is_empty());
        assert!(report.molecule_transaction_bytes.unwrap() > 0);
        assert!(report.ckb_raw_tx_hash.is_some());
        assert!(report.ckb_wtx_hash.is_some());
    }

    #[test]
    fn malformed_output_data_is_a_projection_blocker() {
        let mut tx = CellTx::new(
            vec![CellInput::new(OutPoint::new([1; 32], 0), 0)],
            vec![],
            vec![CellOutput { lock: script(2), type_: None, capacity: 100 }],
            vec![vec![0xAA]],
            vec![vec![0xCC]],
        )
        .unwrap();
        tx.outputs_data.clear();

        let report = project_cell_tx_to_ckb(&tx);
        assert!(!report.ckb_projection_possible);
        assert_eq!(report.semantic_profile, SemanticProfile::CkbInspiredOnly);
        assert!(matches!(report.blockers.first(), Some(ProjectionBlocker::OutputsDataLengthMismatch { outputs: 1, outputs_data: 0 })));
    }
}
