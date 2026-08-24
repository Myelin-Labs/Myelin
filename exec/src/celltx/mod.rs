// SPDX-License-Identifier: MIT
// Copyright (C) 2026 Myelin developers
//
// Cell transaction types (CKB-inspired)

//! Cell transaction types module

/// Signature hashing functions
pub mod sighash;
/// Cell transaction core types
pub mod types;
// pub mod codec;  // Phase 1.5 - Molecule serialization

pub use sighash::{compute_rw_bound_sighash, compute_txid, compute_wtxid, pubkey_hash};
pub use types::{
    cell_tx_estimated_serialized_size, compute_conflict_hash, compute_typed_data_hash, encode_ckb_dep_group_data,
    encode_conflict_key_value_composite, parse_ckb_dep_group_data, validate_typed_cell_decl, CapacityError, CellAccounting, CellDep,
    CellIdentity, CellInput, CellMutability, CellOutput, CellOwnership, CellSettlement, CellTx, ConflictKeySpec, DepType,
    InMemoryTypedCellStore, OutPoint, ResolvedCellMeta, ResolvedCellTx, RuntimeCellSemantics, Script, ScriptId, TransactionInfo,
    TypedCellDecl, TypedCellDeclError, TypedCellSemanticMetadata, TypedCellStore, CELLTX_SCHEMA_VERSION,
};
// Re-export VersionedSerializable implementations for storage layer types
pub use types::{
    ResolvedCellMeta as ResolvedCellMetaVersioned, ResolvedCellTx as ResolvedCellTxVersioned,
    TransactionInfo as TransactionInfoVersioned,
};
