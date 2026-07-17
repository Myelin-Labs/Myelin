// SPDX-License-Identifier: MIT
// Copyright (C) 2026 Myelin developers
//
// Cell transaction types (CKB-inspired)

//! Cell transaction types module

/// Signature hashing functions
pub mod sighash;
/// Cell transaction core types
pub mod types;
/// Bridge for the cellscript compiler's legacy scheduler-witness wire format.
pub mod witness_bridge;
// pub mod codec;  // Phase 1.5 - Molecule serialization

pub use sighash::{compute_rw_bound_sighash, compute_txid, compute_wtxid, pubkey_hash};
pub use types::{
    cell_tx_estimated_serialized_size, cellscript_compiled_scheduler_accesses_for_tx, cellscript_compiled_scheduler_summary_for_tx,
    compute_conflict_hash, compute_typed_data_hash, decode_cellscript_scheduler_witness, decode_cellscript_scheduler_witness_for_tx,
    encode_cellscript_scheduler_witness_molecule, encode_ckb_dep_group_data, encode_conflict_key_value_composite,
    encode_dep_group_data, encode_dep_group_data_for_abi, is_cellscript_scheduler_witness_bytes, parse_ckb_dep_group_data,
    parse_dep_group_data, parse_dep_group_data_for_abi, validate_cellscript_scheduler_witness_access_set,
    validate_cellscript_scheduler_witness_against_transaction, validate_cellscript_scheduler_witness_summary,
    validate_typed_cell_decl, CapacityError, CellAccounting, CellDep, CellIdentity, CellInput, CellMutability, CellOutput,
    CellOwnership, CellScriptSchedulerAccessWitness, CellScriptSchedulerWitness, CellScriptSchedulerWitnessError, CellSettlement,
    CellTx, ConflictKeySpec, DepGroupDataAbi, DepType, InMemoryTypedCellStore, OutPoint, ResolvedCellMeta, ResolvedCellTx,
    RuntimeCellSemantics, Script, ScriptHashVersion, ScriptId, TransactionInfo, TypedCellDecl, TypedCellDeclError,
    TypedCellSemanticMetadata, TypedCellStore, CELLSCRIPT_SCHEDULER_EFFECT_CREATING, CELLSCRIPT_SCHEDULER_EFFECT_DESTROYING,
    CELLSCRIPT_SCHEDULER_EFFECT_MUTATING, CELLSCRIPT_SCHEDULER_EFFECT_PURE, CELLSCRIPT_SCHEDULER_EFFECT_READ_ONLY,
    CELLSCRIPT_SCHEDULER_OP_CONSUME, CELLSCRIPT_SCHEDULER_OP_CREATE, CELLSCRIPT_SCHEDULER_OP_DESTROY,
    CELLSCRIPT_SCHEDULER_OP_READ_REF, CELLSCRIPT_SCHEDULER_OP_TRANSFER, CELLSCRIPT_SCHEDULER_SOURCE_CELL_DEP,
    CELLSCRIPT_SCHEDULER_SOURCE_INPUT, CELLSCRIPT_SCHEDULER_SOURCE_OUTPUT, CELLSCRIPT_SCHEDULER_WITNESS_MAGIC,
    CELLSCRIPT_SCHEDULER_WITNESS_VERSION, CELLTX_SCHEMA_VERSION, MAX_CELLSCRIPT_ACCESS_COUNT, TYPED_CELL_SCHEDULER_WITNESS_VERSION,
};
pub use witness_bridge::{translate_legacy_witness_for_tx, CellscriptLegacyAccess, CellscriptLegacyWitness};

// Re-export VersionedSerializable implementations for storage layer types
pub use types::{
    ResolvedCellMeta as ResolvedCellMetaVersioned, ResolvedCellTx as ResolvedCellTxVersioned,
    TransactionInfo as TransactionInfoVersioned,
};
