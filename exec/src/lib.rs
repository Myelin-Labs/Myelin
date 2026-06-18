// SPDX-License-Identifier: MIT
// Copyright (C) 2026 Myelin developers
//
// This file is part of Myelin, a finite off-chain Cell ledger runtime.
// Portions adapted from Nervos CKB (MIT License).

//! # Myelin 执行层 (Cell Execution Layer)
//!
//! This crate implements the execution layer for Cell transactions, including:
//! - Cell transaction types (CellTx, CellInput, CellOutput, Script)
//! - Parallel scheduler with explicit read/write Cell dependencies
//! - VM integration (CKB-VM for script verification)
//! - Standard scripts (secp256k1 lock, capacity type)
//!
//! ## 序列化分层治理架构
//!
//! 本 crate 采用分层序列化策略，以平衡性能、兼容性和未来演进需求：
//!
//! ### 三层架构
//!
//! ```text
//! ┌─────────────────────────────────────────────────────────────────┐
//! │  Layer 3: VM/Script ABI 层 (Molecule v1 public)                 │
//! │  - ResolvedHeader, ResolvedCell, Witness Payload                │
//! │  - 脚本可见的所有数据结构                                        │
//! │  - 需要: canonical, partial read, version兼容                  │
//! ├─────────────────────────────────────────────────────────────────┤
//! │  Layer 2: 内部工具/存储层 (Molecule-compatible envelopes)       │
//! │  - Active state records use explicit Molecule-compatible bytes   │
//! │  - VersionedEnvelope is encoded as a Molecule-style table        │
//! │  - Active records use explicit non-derived payload codecs         │
//! ├─────────────────────────────────────────────────────────────────┤
//! │  Layer 1: 共识关键路径 (CKB/Molecule-shaped bytes)              │
//! │  - Block Hash, TxID, SigHash (自定义流式哈希)                   │
//! │  - 完全不受影响，继续用 domain-separated Blake3                 │
//! └─────────────────────────────────────────────────────────────────┘
//! ```
//!
//! ### 重要保证
//!
//! 1. **Molecule-compatible bytes are the public/default protocol boundary**
//!    - 所有共识关键哈希 (block hash, txid, sighash) 使用自定义流式哈希
//!    - Native Myelin execution does not carry a legacy serializer dependency or ABI path
//!
//! 2. **VM-facing ABI 必须经过显式格式边界**
//!    - Molecule v1 (`0x8001`) 是 launch/public VM ABI
//!    - Non-Molecule VM object ABI versions are rejected
//!
//! 3. **VM ABI 是独立抽象层**
//!    - 通过 [`VmSerializable`](serialization::VmSerializable) trait 抽象序列化实现
//!    - VM 层可以独立切到 Molecule，不影响存储层和共识哈希层
//!
//! ### 相关模块
//!
//! - [`serialization`](crate::serialization) - 版本化序列化 trait 和 envelope
//! - [`celltx`](crate::celltx) - Cell 交易类型
//! - [`vm`](crate::vm) - VM 集成和 ABI 层
//!
//! 详细设计文档: `docs/SERIALIZATION_LAYER_GOVERNANCE_MIGRATION_PLAN.md`

#![warn(missing_docs)]

/// Cell transaction types and operations
pub mod celltx;
/// Deterministic execution reports for Cell transactions.
pub mod execution_report;
/// CKB-style projection reports for Cell transactions.
pub mod projection;
/// Parallel transaction scheduler
pub mod scheduler;
/// Standard scripts (secp256k1 lock, capacity type)
pub mod scripts;
/// Serialization framework with version governance
pub mod serialization;
/// VM integration for script execution (CKB-VM based)
#[cfg(feature = "vm")]
pub mod vm;
#[cfg(feature = "vm")]
pub use vm::{ResolvedCell, ResolvedHeader, ScriptVersion, SimpleDataProvider, TransactionScriptVerifier, VmSemantics};

pub use celltx::{
    compute_conflict_hash, compute_typed_data_hash, encode_ckb_dep_group_data, encode_dep_group_data, encode_dep_group_data_for_abi,
    parse_ckb_dep_group_data, parse_dep_group_data, parse_dep_group_data_for_abi, CapacityError, CellAccounting, CellDep,
    CellIdentity, CellInput, CellMutability, CellOutput, CellOwnership, CellTx, ConflictKeySpec, DepGroupDataAbi, DepType,
    InMemoryTypedCellStore, OutPoint, RuntimeCellSemantics, Script, ScriptHashVersion, ScriptId, TypedCellDecl, TypedCellDeclError,
    TypedCellSemanticMetadata, TypedCellStore, CELLTX_SCHEMA_VERSION,
};

pub use execution_report::{build_cell_tx_execution_report, CellTxExecutionReport, ExecutionReportStatus};
pub use projection::{project_cell_tx_to_ckb, CkbProjectionReport, ProjectionBlocker, ProjectionWarning, SemanticProfile};

// Re-export serialization framework
pub use serialization::{
    append_vm_abi_trailer, split_vm_abi_trailer, SerializationError, VersionedEnvelope, VersionedSerializable, VmAbiError,
    VmAbiFormat, VmAbiNegotiator, VmSerializable,
};

// Re-export vm_abi helpers
pub use serialization::vm_abi::{
    serialize_cell_input, serialize_cell_output, serialize_outpoint, serialize_script, serialized_cell_output_size,
    serialized_script_size,
};

// Re-export utils
pub use serialization::utils::{
    deserialize_from_bytes, deserialize_many, estimate_serialized_size, is_valid_versioned_envelope, peek_format_version,
    peek_schema_version, serialize_many, serialize_to_bytes, SerializeResult,
};

// Re-export cache
pub use serialization::cache::{CacheStats, SerializationCache, ThreadSafeSerializationCache};

// Re-export validation
pub use serialization::validation::{is_valid_envelope, validate_envelope, SerializerValidator, ValidationConfig, ValidationResult};

// Re-export streaming
pub use serialization::streaming::{deserialize_streaming, serialize_streaming, StreamingDeserializer, StreamingSerializer};

// Re-export security
pub use serialization::security::{
    compute_hash, deserialize_with_integrity, serialize_with_integrity, verify_integrity, SecureEnvelope, SecurityConfig,
    SecurityGuard,
};

// Re-export compression
pub use serialization::compression::{
    compress, decompress, estimate_compressed_size, select_algorithm, CompressedEnvelope, CompressionAlgorithm, CompressionConfig,
    CompressionResult, CompressionStats,
};

// Re-export molecule compatibility layer
pub use serialization::molecule_compat::{
    ckb_apply_type_id_args_to_output_molecule, ckb_apply_type_id_script_to_output_molecule, ckb_blake160, ckb_blake2b_256,
    ckb_cell_data_hash, ckb_dep_group_cell_dep, ckb_epoch_number_with_fraction_from_full_value,
    ckb_epoch_number_with_fraction_full_value, ckb_header_epoch_index, ckb_header_epoch_length, ckb_header_epoch_number,
    ckb_header_epoch_start_block_number, ckb_header_hash_molecule, ckb_raw_transaction_hash_molecule, ckb_script_hash_molecule,
    ckb_secp256k1_blake160_pubkey_hash, ckb_secp256k1_blake160_sighash_all_lock_script, ckb_secp256k1_blake160_sighash_all_type_hash,
    ckb_sighash_all_message_from_witness_args_molecule, ckb_sighash_all_message_molecule,
    ckb_sighash_all_message_with_zeroed_witness_lock_molecule, ckb_sign_secp256k1_blake160_sighash_all_input_molecule,
    ckb_sign_secp256k1_blake160_sighash_all_lock_group_molecule, ckb_sign_secp256k1_blake160_sighash_all_molecule,
    ckb_transaction_witness_hash_molecule, ckb_type_id_args, ckb_type_id_script, ckb_verify_secp256k1_blake160_recoverable_signature,
    ckb_verify_secp256k1_blake160_sighash_all_lock_group_molecule, ckb_verify_secp256k1_blake160_sighash_all_molecule,
    ckb_verify_type_id_script_group_molecule, ckb_verify_type_id_script_molecule, deserialize_cell_dep_molecule,
    deserialize_cell_input_molecule, deserialize_cell_output_molecule, deserialize_ckb_header_molecule,
    deserialize_ckb_outpoint_vec_molecule, deserialize_ckb_raw_header_molecule, deserialize_ckb_witness_args_molecule,
    deserialize_outpoint_molecule, deserialize_raw_transaction_molecule, deserialize_resolved_cell_molecule,
    deserialize_resolved_header_molecule, deserialize_script_molecule, deserialize_transaction_molecule, serialize_cell_dep_molecule,
    serialize_cell_input_molecule, serialize_cell_output_molecule, serialize_ckb_header_molecule, serialize_ckb_outpoint_vec_molecule,
    serialize_ckb_raw_header_molecule, serialize_ckb_witness_args_molecule, serialize_outpoint_molecule,
    serialize_raw_transaction_molecule, serialize_resolved_cell_molecule, serialize_resolved_header_molecule,
    serialize_script_molecule, serialize_transaction_molecule, CkbEpochNumberWithFraction, CkbHeader, CkbRawHeader,
    CkbSecp256k1Blake160SighashAllLockConfig, CkbWitnessArgs, MoleculeError, MoleculeSerializer, CKB_SCRIPT_HASH_TYPE_TYPE,
    CKB_SECP256K1_BLAKE160_LOCK_ARG_SIZE, CKB_SECP256K1_SIGHASH_ALL_SIGNATURE_SIZE, CKB_TYPE_ID_CODE_HASH,
};

/// Cell transaction version
pub const CELL_TX_VERSION: u32 = 0xC001;

/// Network ID (u32, little-endian)
#[repr(u32)]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum NetworkId {
    /// Reserved (invalid)
    Reserved = 0x00000000,
    /// Mainnet
    Mainnet = 0x00000001,
    /// Testnet
    Testnet = 0x00000002,
    /// Devnet
    Devnet = 0x00000003,
    /// Regtest
    Regtest = 0xFFFFFFFF,
}

impl NetworkId {
    /// Convert to u32
    pub fn to_u32(self) -> u32 {
        self as u32
    }

    /// Parse from u32
    pub fn from_u32(val: u32) -> Option<Self> {
        match val {
            0x00000000 => Some(Self::Reserved),
            0x00000001 => Some(Self::Mainnet),
            0x00000002 => Some(Self::Testnet),
            0x00000003 => Some(Self::Devnet),
            0xFFFFFFFF => Some(Self::Regtest),
            _ => None,
        }
    }
}
