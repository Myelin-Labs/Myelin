// SPDX-License-Identifier: MIT
// Copyright (C) 2026 Myelin developers
//
// Cell transaction core types (CKB-inspired, scheduler-adapted)
//
// Reference: ckb/util/types/src/core/cell.rs

use serde::{Deserialize, Serialize};
use std::{collections::BTreeMap, fmt};

/// Serde helpers for serializing `[u8; 32]` as a hex string under the key `transactionId`
/// for human-readable formats (JSON), or raw bytes for binary formats.
mod outpoint_serde {
    use serde::{self, Deserialize, Deserializer, Serializer};

    pub fn serialize<S>(tx_hash: &[u8; 32], serializer: S) -> Result<S::Ok, S::Error>
    where
        S: Serializer,
    {
        if serializer.is_human_readable() {
            let hex: String = tx_hash.iter().map(|b| format!("{:02x}", b)).collect();
            serializer.serialize_str(&hex)
        } else {
            serde::Serialize::serialize(tx_hash, serializer)
        }
    }

    pub fn deserialize<'de, D>(deserializer: D) -> Result<[u8; 32], D::Error>
    where
        D: Deserializer<'de>,
    {
        if deserializer.is_human_readable() {
            let s = String::deserialize(deserializer)?;
            if s.len() != 64 {
                return Err(serde::de::Error::custom(format!("expected 64 hex chars, got {}", s.len())));
            }
            let mut bytes = [0u8; 32];
            for i in 0..32 {
                bytes[i] = u8::from_str_radix(&s[2 * i..2 * i + 2], 16).map_err(serde::de::Error::custom)?;
            }
            Ok(bytes)
        } else {
            <[u8; 32]>::deserialize(deserializer)
        }
    }
}

/// Cell transaction version: 0xC001
pub const CELL_TX_VERSION: u32 = 0xC001;
// Little-endian bytes for the CellScript scheduler witness magic `0xCE11`.
// ─── Typed Cell Classification ──────────────────────────────────────────────
//
// Six dimensions with three enforcement levels:
//
// | Dimension       | Phase 1 status                    | Meaning                                        |
// |-----------------|------------------------------------|------------------------------------------------|
// | Ownership       | partially runtime-enforced         | controls write/read eligibility and shared     |
// |                 |                                    | conflict handling                              |
// | ConflictKeySpec | runtime-enforced                   | directly derives `conflict_hash`               |
// | Mutability      | advisory + validation constraints  | future compiler/runtime semantics              |
// | Accounting      | advisory + validation constraints  | future accounting checks / ProofPlan           |
// | Identity        | advisory + manifest semantics      | future update pairing / settlement              |
// | Settlement      | advisory                           | future checkpoint/exit/bridge layer            |
//
// "Partially runtime-enforced" means Ownership's Immutable/Ephemeral distinction
// is checked by validate_typed_cell_decl, but Shared vs Party is scheduler-equivalent.
// "Advisory" means the field is metadata for future use; cross-axis constraints
// prevent obviously illegal combinations but the field does not affect scheduling.

/// Ownership class — determines parallel execution and access rules
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum CellOwnership {
    /// One owner, easy to parallelise
    Owned,
    /// Public mutable cell (AMM pool, oracle)
    Shared,
    /// Bounded multi-party session (e.g. payment channel)
    /// Advisory in Phase 1: scheduler-equivalent to Shared.
    /// Product-distinct for CellScript but does not affect conflict_hash scheduling.
    Party,
    /// Read-only after creation
    Immutable,
    /// Batch-local intermediate, not admitted to scheduler
    Ephemeral,
}

/// Mutability class — determines state transition pattern
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum CellMutability {
    /// Consume + create
    Linear,
    /// Consume + create with version field
    Versioned,
    /// Successor output, data only appends
    AppendOnly,
    /// Explicit data layout migration
    Migratable,
}

/// Accounting class — domain constraint on data layout
///
/// Multi-label: `Vec<CellAccounting>` in `TypedCellDecl`.
/// E.g. a bridge-claim cell can be both `Receipt` + `StorageClaim`.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum CellAccounting {
    /// Fungible token-like accounting
    Fungible,
    /// Non-fungible unique asset
    NonFungible,
    /// Receipt / proof-of-event
    Receipt,
    /// Claim over occupied-capacity-backed L1 storage space (not a token class)
    StorageClaim,
}

/// Identity class — how identity is preserved across updates
#[derive(Clone, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum CellIdentity {
    /// Natural OutPoint identity
    OutPoint,
    /// TYPE_ID pattern
    TypeId,
    /// One-of-a-kind, identified by type_script alone
    Singleton,
    /// Named field as identity key
    Field(String),
    /// Composite key from multiple fields
    Composite(Vec<String>),
}

/// Settlement class — determines how this cell's state is committed
///
/// Naming is deployment-agnostic: does not presuppose L2, consortium, or standalone.
/// Advisory in Phase 1: not consumed by runtime scheduling.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum CellSettlement {
    /// Settled within this execution environment
    Local,
    /// Participates in root commitment (bridge / rollup / consortium)
    Committed,
    /// Awaiting external settlement/finalisation
    Pending,
}

/// Conflict key specification — determines how conflict_hash is derived
///
/// Rule: mutable cells must not use `ConflictKeySpec::None`.
/// `None` is only valid for Pure / ReadOnly / Ephemeral cells.
#[derive(Clone, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum ConflictKeySpec {
    /// Concrete cell identity — default for owned mutable cells
    CellId,
    /// Single field name (e.g. "pool_id", "owner")
    Field(String),
    /// Composite key from multiple fields (e.g. ["asset_id", "owner", "shard_id"])
    Composite(Vec<String>),
    /// No conflict key — only valid for Pure / ReadOnly / Ephemeral
    None,
}

/// Runtime-scheduling semantics — directly consumed by CellDAG.
///
/// These two axes determine conflict detection and parallel execution safety.
/// They are the **only** typed-cell metadata the runtime scheduler consumes.
///
/// Rule: VM never consumes typed-cell semantic axes; runtime consumes only
/// scheduling-critical metadata (ownership + conflict_key + witness envelope).
///
/// See `docs/TYPED_CELL_CLASSIFICATION_GOVERNANCE.md` §9.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct RuntimeCellSemantics {
    /// Ownership class — determines parallel execution and access rules
    pub ownership: CellOwnership,
    /// Conflict key specification — directly derives `conflict_hash` for conflict detection.
    /// `conflict_hash = blake3(domain || full_script_id || conflict_key_value)`
    pub conflict_key: ConflictKeySpec,
}

/// Semantic metadata — not consumed by the runtime scheduler in Phase 1.
///
/// These axes are validated for contradictions but are primarily consumed
/// by future compiler, ProofPlan, settlement, and audit layers.
///
/// Rule: CellScript/ProofPlan are the semantic source of truth;
/// TypedCellDecl is generated/normalised metadata, not an independent language.
///
/// See `docs/TYPED_CELL_CLASSIFICATION_GOVERNANCE.md` §9.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct TypedCellSemanticMetadata {
    /// Mutability class — future compiler/ProofPlan semantics, current cross-axis checks only
    pub mutability: CellMutability,
    /// Accounting labels — future accounting/ProofPlan checks, current label exclusivity only
    pub accounting: Vec<CellAccounting>,
    /// Identity class — future update pairing / settlement / exit / artifact metadata
    pub identity: CellIdentity,
    /// Settlement class — future checkpoint/exit/bridge layer, current manifest tag
    pub settlement: CellSettlement,
}

/// Normalized typed-cell metadata.
///
/// Split into two enforcement tiers via sub-structs:
///
/// - **`runtime`** (`RuntimeCellSemantics`): scheduling-critical axes directly
///   consumed by CellDAG for conflict detection and parallel execution.
/// - **`semantic`** (`TypedCellSemanticMetadata`): advisory axes validated for
///   contradictions but not consumed by the scheduler in Phase 1.
///
/// Three hard rules prevent TypedCellDecl from becoming a second semantic
/// authority that conflicts with VM, CellScript, or ProofPlan:
///
/// 1. VM never consumes typed-cell semantic axes. VM only executes.
/// 2. Runtime consumes only scheduling-critical metadata:
///    ownership + conflict_key + witness envelope.
/// 3. CellScript/ProofPlan are the semantic source of truth.
///    TypedCellDecl is generated/normalised metadata, not an independent language.
///
/// Anti-override rule:
/// TypedCellDecl must not introduce verifier semantics that are not derivable
/// from CellScript source, ProofPlan obligations, or runtime scheduler requirements.
///
/// See `docs/TYPED_CELL_CLASSIFICATION_GOVERNANCE.md` for full governance.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct TypedCellDecl {
    /// Runtime-scheduling semantics (ownership + conflict_key)
    pub runtime: RuntimeCellSemantics,
    /// Semantic metadata (mutability, accounting, identity, settlement)
    pub semantic: TypedCellSemanticMetadata,
}

/// Canonical script identity for typed cell registry key.
///
/// Keyed by full script identity (not just code_hash), because the same
/// `code_hash` with different `args` represents different type instances.
#[derive(Clone, Debug, PartialEq, Eq, Hash, Ord, PartialOrd, Serialize, Deserialize)]
pub struct ScriptId {
    /// Script code hash
    pub code_hash: [u8; 32],
    /// Script hash type
    pub hash_type: u8,
    /// Hash of script args (blake3)
    pub args_hash: [u8; 32],
}

impl ScriptId {
    /// Derive ScriptId from a Script reference
    pub fn from_script(script: &Script) -> Self {
        let args_hash = *blake3::hash(&script.args).as_bytes();
        Self { code_hash: script.code_hash, hash_type: script.hash_type, args_hash }
    }
}

/// Registry of typed cell declarations keyed by full script identity.
pub trait TypedCellStore {
    /// Look up a typed cell declaration by its type script
    fn get_decl(&self, type_script: &Script) -> Option<&TypedCellDecl>;
    /// Insert a typed cell declaration
    fn insert_decl(&mut self, type_script: Script, decl: TypedCellDecl);
}

/// In-memory typed cell store.
pub struct InMemoryTypedCellStore {
    decls: BTreeMap<ScriptId, TypedCellDecl>,
}

impl InMemoryTypedCellStore {
    /// Create an empty typed cell store
    pub fn new() -> Self {
        Self { decls: BTreeMap::new() }
    }
}

impl Default for InMemoryTypedCellStore {
    fn default() -> Self {
        Self::new()
    }
}

impl TypedCellStore for InMemoryTypedCellStore {
    fn get_decl(&self, type_script: &Script) -> Option<&TypedCellDecl> {
        let id = ScriptId::from_script(type_script);
        self.decls.get(&id)
    }

    fn insert_decl(&mut self, type_script: Script, decl: TypedCellDecl) {
        let id = ScriptId::from_script(&type_script);
        self.decls.insert(id, decl);
    }
}

// ─── Typed Cell Hash Functions ────────────────────────────────────────────────

/// Compute stable conflict hash.
///
/// `blake3(domain || code_hash || hash_type || args || conflict_key_value)`
///
/// Does NOT change when cell data is updated.
/// Used by CellDAG conflict detection.
pub fn compute_conflict_hash(type_script: &Script, conflict_key_value: &[u8]) -> [u8; 32] {
    let mut hasher = blake3::Hasher::new();
    hasher.update(b"myelin-typed-cell/conflict-hash/v1");
    hasher.update(&type_script.code_hash);
    hasher.update(&[type_script.hash_type]);
    hasher.update(&type_script.args);
    hasher.update(conflict_key_value);
    *hasher.finalize().as_bytes()
}

/// Compute typed data hash.
///
/// `blake3(domain || code_hash || hash_type || args || data)`
///
/// Changes with every data update.
/// Named `typed_data_hash` (not `cell_state_hash`) because it does NOT
/// include lock/capacity — only type script identity + data.
pub fn compute_typed_data_hash(type_script: &Script, data: &[u8]) -> [u8; 32] {
    let mut hasher = blake3::Hasher::new();
    hasher.update(b"myelin-typed-cell/typed-data-hash/v1");
    hasher.update(&type_script.code_hash);
    hasher.update(&[type_script.hash_type]);
    hasher.update(&type_script.args);
    hasher.update(data);
    *hasher.finalize().as_bytes()
}

/// Encode composite conflict key values in canonical length-delimited format.
///
/// `conflict_key_value = len(field1_le_u32) || field1 || len(field2_le_u32) || field2 || ...`
///
/// Composite keys must NOT use raw concatenation
/// (avoids `["ab", "c"]` vs `["a", "bc"]` ambiguity).
pub fn encode_conflict_key_value_composite(fields: &[&[u8]]) -> Vec<u8> {
    let mut out = Vec::new();
    for field in fields {
        out.extend_from_slice(&(field.len() as u32).to_le_bytes());
        out.extend_from_slice(field);
    }
    out
}

/// Validate that a `TypedCellDecl` satisfies Phase 1 constraints.
///
/// Internally split into two enforcement tiers:
/// 1. **Runtime-scheduling checks** — ownership + conflict_key rules that
///    directly affect CellDAG scheduling correctness
/// 2. **Semantic consistency checks** — cross-axis constraints that prevent
///    obviously illegal dimension combinations
///
/// See `docs/TYPED_CELL_CLASSIFICATION_GOVERNANCE.md` for full governance.
pub fn validate_typed_cell_decl(decl: &TypedCellDecl) -> Result<(), TypedCellDeclError> {
    check_runtime_scheduling_rules(decl)?;
    check_semantic_consistency_rules(decl)?;
    Ok(())
}

/// Encode typed-cell metadata as a small Molecule-compatible table.
pub fn encode_typed_cell_decl_molecule(decl: &TypedCellDecl) -> Vec<u8> {
    scheduler_molecule_encode_table(&[
        encode_runtime_cell_semantics_molecule(&decl.runtime),
        encode_typed_cell_semantic_metadata_molecule(&decl.semantic),
    ])
}

/// Decode typed-cell metadata from [`encode_typed_cell_decl_molecule`] bytes.
pub fn decode_typed_cell_decl_molecule(bytes: &[u8]) -> Result<TypedCellDecl, String> {
    let fields = scheduler_molecule_decode_table(bytes, 2, "TypedCellDecl")?;
    Ok(TypedCellDecl {
        runtime: decode_runtime_cell_semantics_molecule(fields[0])?,
        semantic: decode_typed_cell_semantic_metadata_molecule(fields[1])?,
    })
}

fn encode_runtime_cell_semantics_molecule(runtime: &RuntimeCellSemantics) -> Vec<u8> {
    scheduler_molecule_encode_table(&[
        vec![encode_cell_ownership_tag(runtime.ownership)],
        encode_conflict_key_spec_molecule(&runtime.conflict_key),
    ])
}

fn decode_runtime_cell_semantics_molecule(bytes: &[u8]) -> Result<RuntimeCellSemantics, String> {
    let fields = scheduler_molecule_decode_table(bytes, 2, "RuntimeCellSemantics")?;
    Ok(RuntimeCellSemantics {
        ownership: decode_cell_ownership_tag(scheduler_molecule_decode_u8(fields[0], "RuntimeCellSemantics.ownership")?)?,
        conflict_key: decode_conflict_key_spec_molecule(fields[1])?,
    })
}

fn encode_typed_cell_semantic_metadata_molecule(semantic: &TypedCellSemanticMetadata) -> Vec<u8> {
    scheduler_molecule_encode_table(&[
        vec![encode_cell_mutability_tag(semantic.mutability)],
        encode_cell_accounting_vec_molecule(&semantic.accounting),
        encode_cell_identity_molecule(&semantic.identity),
        vec![encode_cell_settlement_tag(semantic.settlement)],
    ])
}

fn decode_typed_cell_semantic_metadata_molecule(bytes: &[u8]) -> Result<TypedCellSemanticMetadata, String> {
    let fields = scheduler_molecule_decode_table(bytes, 4, "TypedCellSemanticMetadata")?;
    Ok(TypedCellSemanticMetadata {
        mutability: decode_cell_mutability_tag(scheduler_molecule_decode_u8(fields[0], "TypedCellSemanticMetadata.mutability")?)?,
        accounting: decode_cell_accounting_vec_molecule(fields[1])?,
        identity: decode_cell_identity_molecule(fields[2])?,
        settlement: decode_cell_settlement_tag(scheduler_molecule_decode_u8(fields[3], "TypedCellSemanticMetadata.settlement")?)?,
    })
}

fn encode_conflict_key_spec_molecule(spec: &ConflictKeySpec) -> Vec<u8> {
    match spec {
        ConflictKeySpec::CellId => scheduler_molecule_encode_table(&[vec![0], Vec::new()]),
        ConflictKeySpec::Field(name) => scheduler_molecule_encode_table(&[vec![1], name.as_bytes().to_vec()]),
        ConflictKeySpec::Composite(names) => scheduler_molecule_encode_table(&[vec![2], encode_string_vec_molecule(names)]),
        ConflictKeySpec::None => scheduler_molecule_encode_table(&[vec![3], Vec::new()]),
    }
}

fn decode_conflict_key_spec_molecule(bytes: &[u8]) -> Result<ConflictKeySpec, String> {
    let fields = scheduler_molecule_decode_table(bytes, 2, "ConflictKeySpec")?;
    match scheduler_molecule_decode_u8(fields[0], "ConflictKeySpec.tag")? {
        0 => Ok(ConflictKeySpec::CellId),
        1 => Ok(ConflictKeySpec::Field(decode_utf8_string(fields[1], "ConflictKeySpec.field")?)),
        2 => Ok(ConflictKeySpec::Composite(decode_string_vec_molecule(fields[1], "ConflictKeySpec.composite")?)),
        3 => Ok(ConflictKeySpec::None),
        other => Err(format!("ConflictKeySpec: unknown tag {other}")),
    }
}

fn encode_cell_identity_molecule(identity: &CellIdentity) -> Vec<u8> {
    match identity {
        CellIdentity::OutPoint => scheduler_molecule_encode_table(&[vec![0], Vec::new()]),
        CellIdentity::TypeId => scheduler_molecule_encode_table(&[vec![1], Vec::new()]),
        CellIdentity::Singleton => scheduler_molecule_encode_table(&[vec![2], Vec::new()]),
        CellIdentity::Field(name) => scheduler_molecule_encode_table(&[vec![3], name.as_bytes().to_vec()]),
        CellIdentity::Composite(names) => scheduler_molecule_encode_table(&[vec![4], encode_string_vec_molecule(names)]),
    }
}

fn decode_cell_identity_molecule(bytes: &[u8]) -> Result<CellIdentity, String> {
    let fields = scheduler_molecule_decode_table(bytes, 2, "CellIdentity")?;
    match scheduler_molecule_decode_u8(fields[0], "CellIdentity.tag")? {
        0 => Ok(CellIdentity::OutPoint),
        1 => Ok(CellIdentity::TypeId),
        2 => Ok(CellIdentity::Singleton),
        3 => Ok(CellIdentity::Field(decode_utf8_string(fields[1], "CellIdentity.field")?)),
        4 => Ok(CellIdentity::Composite(decode_string_vec_molecule(fields[1], "CellIdentity.composite")?)),
        other => Err(format!("CellIdentity: unknown tag {other}")),
    }
}

fn encode_cell_accounting_vec_molecule(accounting: &[CellAccounting]) -> Vec<u8> {
    let mut out = Vec::with_capacity(4 + accounting.len());
    out.extend_from_slice(&scheduler_molecule_pack_number(accounting.len()));
    out.extend(accounting.iter().map(|item| encode_cell_accounting_tag(*item)));
    out
}

fn decode_cell_accounting_vec_molecule(bytes: &[u8]) -> Result<Vec<CellAccounting>, String> {
    let count = scheduler_molecule_unpack_number(bytes, "CellAccountingVec")?;
    if bytes.len() != 4 + count {
        return Err(format!("CellAccountingVec: expected {} bytes, got {}", 4 + count, bytes.len()));
    }
    bytes[4..].iter().map(|tag| decode_cell_accounting_tag(*tag)).collect()
}

fn encode_string_vec_molecule(values: &[String]) -> Vec<u8> {
    let items = values.iter().map(|value| value.as_bytes().to_vec()).collect::<Vec<_>>();
    typed_molecule_encode_dynvec(&items)
}

fn decode_string_vec_molecule(bytes: &[u8], ty: &'static str) -> Result<Vec<String>, String> {
    typed_molecule_decode_dynvec(bytes, ty)?.into_iter().map(|item| decode_utf8_string(item, ty)).collect()
}

fn decode_utf8_string(bytes: &[u8], ty: &'static str) -> Result<String, String> {
    std::str::from_utf8(bytes).map(|value| value.to_string()).map_err(|error| format!("{ty}: invalid utf-8: {error}"))
}

fn typed_molecule_encode_dynvec(items: &[Vec<u8>]) -> Vec<u8> {
    if items.is_empty() {
        return scheduler_molecule_pack_number(4).to_vec();
    }
    scheduler_molecule_encode_table(items)
}

fn typed_molecule_decode_dynvec<'a>(bytes: &'a [u8], ty: &'static str) -> Result<Vec<&'a [u8]>, String> {
    if bytes.len() == 4 && scheduler_molecule_unpack_number(bytes, ty)? == 4 {
        return Ok(Vec::new());
    }
    if bytes.len() < 8 {
        return Err(format!("{ty}: dynvec header is too short: {}", bytes.len()));
    }
    let first_offset = scheduler_molecule_unpack_number(&bytes[4..], ty)?;
    if first_offset < 8 || first_offset % 4 != 0 {
        return Err(format!("{ty}: invalid dynvec first offset {first_offset}"));
    }
    let count = first_offset / 4 - 1;
    scheduler_molecule_decode_table(bytes, count, ty)
}

fn encode_cell_ownership_tag(value: CellOwnership) -> u8 {
    match value {
        CellOwnership::Owned => 0,
        CellOwnership::Shared => 1,
        CellOwnership::Party => 2,
        CellOwnership::Immutable => 3,
        CellOwnership::Ephemeral => 4,
    }
}

fn decode_cell_ownership_tag(tag: u8) -> Result<CellOwnership, String> {
    match tag {
        0 => Ok(CellOwnership::Owned),
        1 => Ok(CellOwnership::Shared),
        2 => Ok(CellOwnership::Party),
        3 => Ok(CellOwnership::Immutable),
        4 => Ok(CellOwnership::Ephemeral),
        other => Err(format!("CellOwnership: unknown tag {other}")),
    }
}

fn encode_cell_mutability_tag(value: CellMutability) -> u8 {
    match value {
        CellMutability::Linear => 0,
        CellMutability::Versioned => 1,
        CellMutability::AppendOnly => 2,
        CellMutability::Migratable => 3,
    }
}

fn decode_cell_mutability_tag(tag: u8) -> Result<CellMutability, String> {
    match tag {
        0 => Ok(CellMutability::Linear),
        1 => Ok(CellMutability::Versioned),
        2 => Ok(CellMutability::AppendOnly),
        3 => Ok(CellMutability::Migratable),
        other => Err(format!("CellMutability: unknown tag {other}")),
    }
}

fn encode_cell_accounting_tag(value: CellAccounting) -> u8 {
    match value {
        CellAccounting::Fungible => 0,
        CellAccounting::NonFungible => 1,
        CellAccounting::Receipt => 2,
        CellAccounting::StorageClaim => 3,
    }
}

fn decode_cell_accounting_tag(tag: u8) -> Result<CellAccounting, String> {
    match tag {
        0 => Ok(CellAccounting::Fungible),
        1 => Ok(CellAccounting::NonFungible),
        2 => Ok(CellAccounting::Receipt),
        3 => Ok(CellAccounting::StorageClaim),
        other => Err(format!("CellAccounting: unknown tag {other}")),
    }
}

fn encode_cell_settlement_tag(value: CellSettlement) -> u8 {
    match value {
        CellSettlement::Local => 0,
        CellSettlement::Committed => 1,
        CellSettlement::Pending => 2,
    }
}

fn decode_cell_settlement_tag(tag: u8) -> Result<CellSettlement, String> {
    match tag {
        0 => Ok(CellSettlement::Local),
        1 => Ok(CellSettlement::Committed),
        2 => Ok(CellSettlement::Pending),
        other => Err(format!("CellSettlement: unknown tag {other}")),
    }
}

/// Runtime-scheduling critical checks.
///
/// These directly affect CellDAG conflict detection correctness.
/// Violations here can cause missed conflicts or phantom dependencies.
///
/// Only examines `decl.runtime` (ownership + conflict_key).
fn check_runtime_scheduling_rules(decl: &TypedCellDecl) -> Result<(), TypedCellDeclError> {
    // Write-capable cells need a conflict key (conflict_hash must be non-zero)
    let can_write = !matches!(decl.runtime.ownership, CellOwnership::Immutable | CellOwnership::Ephemeral);
    if can_write && matches!(decl.runtime.conflict_key, ConflictKeySpec::None) {
        return Err(TypedCellDeclError::MutableCellWithNoneConflictKey);
    }
    Ok(())
}

/// Semantic consistency checks — cross-axis constraints.
///
/// These prevent obviously illegal dimension combinations but do not
/// affect CellDAG scheduling directly. They are enforced at validation time
/// to catch errors early; the scheduler would produce correct results
/// even without these checks.
///
/// Examines both `decl.runtime` and `decl.semantic` for cross-axis violations.
fn check_semantic_consistency_rules(decl: &TypedCellDecl) -> Result<(), TypedCellDeclError> {
    // Immutable ownership cannot pair with mutable mutability
    if matches!(decl.runtime.ownership, CellOwnership::Immutable) && !matches!(decl.semantic.mutability, CellMutability::Linear) {
        return Err(TypedCellDeclError::ImmutableWithMutableMutability { mutability: decl.semantic.mutability });
    }

    // Fungible and NonFungible are mutually exclusive
    let has_fungible = decl.semantic.accounting.contains(&CellAccounting::Fungible);
    let has_nonfungible = decl.semantic.accounting.contains(&CellAccounting::NonFungible);
    if has_fungible && has_nonfungible {
        return Err(TypedCellDeclError::ConflictingAccountingLabels);
    }

    // Ephemeral cells must not have non-local settlement
    if matches!(decl.runtime.ownership, CellOwnership::Ephemeral) && !matches!(decl.semantic.settlement, CellSettlement::Local) {
        return Err(TypedCellDeclError::EphemeralWithNonLocalSettlement);
    }

    Ok(())
}

/// Typed cell declaration validation errors
#[derive(Clone, Debug, PartialEq, Eq, thiserror::Error)]
pub enum TypedCellDeclError {
    /// Any non-ephemeral access that can produce Write mode must not use ConflictKeySpec::None
    #[error("non-ephemeral write-capable cell must not use ConflictKeySpec::None")]
    MutableCellWithNoneConflictKey,
    /// Immutable ownership cannot pair with a mutable mutability variant
    #[error("Immutable ownership cannot pair with {mutability:?} mutability")]
    ImmutableWithMutableMutability {
        /// The mutability variant that conflicts with Immutable ownership
        mutability: CellMutability,
    },
    /// Fungible and NonFungible are mutually exclusive accounting labels
    #[error("Fungible and NonFungible are mutually exclusive accounting labels")]
    ConflictingAccountingLabels,
    /// Ephemeral cells must not have non-local settlement
    #[error("Ephemeral cells must not have non-local settlement")]
    EphemeralWithNonLocalSettlement,
}

// ─── Scheduler Witness Constants ─────────────────────────────────────────────

/// Little-endian bytes for the CellScript scheduler witness magic `0xCE11`.
pub const CELLSCRIPT_SCHEDULER_WITNESS_MAGIC: [u8; 2] = [0x11, 0xCE];
/// CellScript scheduler witness format version accepted for transaction placement.
/// Typed cell scheduler witness version
pub const TYPED_CELL_SCHEDULER_WITNESS_VERSION: u8 = 1;

/// CellScript scheduler witness version (legacy, not used on this branch)
pub const CELLSCRIPT_SCHEDULER_WITNESS_VERSION: u8 = TYPED_CELL_SCHEDULER_WITNESS_VERSION;
const CELLSCRIPT_SCHEDULER_WITNESS_MOLECULE_FIELDS: usize = 7;
/// Access record size: operation(1) + source(1) + index(4) + conflict_hash(32) + typed_data_hash(32) = 70 bytes
const TYPED_CELL_ACCESS_MOLECULE_SIZE: usize = 70;
/// Maximum number of access records per scheduler witness.
/// Prevents memory-exhaustion attacks from malicious oversized witnesses.
pub const MAX_CELLSCRIPT_ACCESS_COUNT: u32 = 256;
/// Scheduler effect class id for pure actions.
pub const CELLSCRIPT_SCHEDULER_EFFECT_PURE: u8 = 0;
/// Scheduler effect class id for read-only actions.
pub const CELLSCRIPT_SCHEDULER_EFFECT_READ_ONLY: u8 = 1;
/// Scheduler effect class id for mutating actions.
pub const CELLSCRIPT_SCHEDULER_EFFECT_MUTATING: u8 = 2;
/// Scheduler effect class id for creating actions.
pub const CELLSCRIPT_SCHEDULER_EFFECT_CREATING: u8 = 3;
/// Scheduler effect class id for destroying actions.
pub const CELLSCRIPT_SCHEDULER_EFFECT_DESTROYING: u8 = 4;
/// Scheduler operation id for `consume`.
pub const CELLSCRIPT_SCHEDULER_OP_CONSUME: u8 = 1;
/// Scheduler operation id for `transfer`.
pub const CELLSCRIPT_SCHEDULER_OP_TRANSFER: u8 = 2;
/// Scheduler operation id for `destroy`.
pub const CELLSCRIPT_SCHEDULER_OP_DESTROY: u8 = 3;
/// Scheduler operation id for `read_ref`.
pub const CELLSCRIPT_SCHEDULER_OP_READ_REF: u8 = 6;
/// Scheduler operation id for `create`.
pub const CELLSCRIPT_SCHEDULER_OP_CREATE: u8 = 7;
/// Scheduler source id for transaction inputs.
pub const CELLSCRIPT_SCHEDULER_SOURCE_INPUT: u8 = 1;
/// Scheduler source id for cell dependencies.
pub const CELLSCRIPT_SCHEDULER_SOURCE_CELL_DEP: u8 = 2;
/// Scheduler source id for transaction outputs.
pub const CELLSCRIPT_SCHEDULER_SOURCE_OUTPUT: u8 = 3;
/// Domain used by the versioned script hash format.
pub const SCRIPT_HASH_V1_DOMAIN: &[u8] = b"myelin-cell/script-hash";
/// Additional bytes a live-cell state entry needs beyond the raw output body.
const CELL_ENTRY_OVERHEAD_EXCLUDING_OUTPUT_BODY: u64 = 32 + 4 + 8 + 1;
/// Static transient-mass factor used before block-context VM cycles are known.
///
/// This intentionally mirrors the consensus-side transient-byte policy until the
/// Cell-native mass model is fully centralized.
const TRANSIENT_BYTE_TO_MASS_FACTOR: u64 = 2;
/// Mass coefficient for each serialized transaction byte.
///
/// Kept in sync with the consensus-side default params so pre-VM estimates in the
/// exec crate match the non-contextual compute mass policy.
const MASS_PER_TX_BYTE: u64 = 1;
/// Mass coefficient for output lock/type script bytes.
const MASS_PER_SCRIPT_PUB_KEY_BYTE: u64 = 10;
/// Mass coefficient for each implicit input sigop.
const MASS_PER_SIG_OP: u64 = 1000;

/// Estimated serialized size of a `CellTx`.
///
/// This is the canonical estimator shared by exec-side estimate helpers and
/// consensus-side mass calculation. Keep this logic centralized to avoid
/// drift between compatibility estimates and the authoritative mass path.
pub fn cell_tx_estimated_serialized_size(tx: &CellTx) -> u64 {
    let mut size: u64 = 0;
    size += 2; // ver (u16)

    // Inputs: each CellInput = outpoint (32+4) + since (8) = 44 bytes
    size += 8; // number of inputs
    size += tx.inputs.len() as u64 * 44;

    // Deps: each CellDep = outpoint (32+4) + dep_type (1) = 37 bytes
    size += 8; // number of deps
    size += tx.cell_deps.len() as u64 * 37;

    // Header deps: each is a 32-byte hash
    size += 8; // number of header_deps
    size += tx.header_deps.len() as u64 * 32;

    // Outputs: each CellOutput = lock script + optional type script + capacity
    size += 8; // number of outputs
    for output in &tx.outputs {
        size += 32 + 1 + 8; // lock.code_hash + lock.hash_type + len(lock.args)
        size += output.lock.args.len() as u64;
        if let Some(ref type_script) = output.type_ {
            size += 1 + 32 + 1 + 8; // flag + code_hash + hash_type + len(args)
            size += type_script.args.len() as u64;
        } else {
            size += 1; // no-type flag
        }
        size += 8; // capacity
    }

    // Outputs data
    size += 8; // number of outputs_data
    for data in &tx.outputs_data {
        size += 8; // length prefix
        size += data.len() as u64;
    }

    // Witnesses
    size += 8; // number of witnesses
    for witness in &tx.witnesses {
        size += 8; // length prefix
        size += witness.len() as u64;
    }

    size
}

/// Returns true when bytes look like a CellScript scheduler witness.
///
/// This is an admission guard for transaction witness placement only.
/// Only structural and header validity (magic, version, counts) is checked.
/// Operation/source semantic constraints are enforced later by the scheduler
/// consumer when decoding the full payload for conflict or admission decisions.
pub fn is_cellscript_scheduler_witness_bytes(witness: &[u8]) -> bool {
    decode_cellscript_scheduler_witness_molecule_unchecked(witness).ok().is_some_and(|w| {
        w.magic == 0xCE11
            && w.version == CELLSCRIPT_SCHEDULER_WITNESS_VERSION
            && w.access_count <= MAX_CELLSCRIPT_ACCESS_COUNT
            && w.access_count as usize == w.accesses.len()
    })
}

fn is_cellscript_scheduler_witness_candidate_bytes(witness: &[u8]) -> bool {
    decode_cellscript_scheduler_witness_molecule_unchecked(witness)
        .is_ok_and(|decoded| decoded.magic == 0xCE11 && decoded.version == CELLSCRIPT_SCHEDULER_WITNESS_VERSION)
}

/// Typed cell scheduler witness access record.
///
/// Clean break from the old `binding_hash` model.
/// `conflict_hash` is stable across data updates (used for conflict detection).
/// `typed_data_hash` changes with data (used for audit/commitment).
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct CellScriptSchedulerAccessWitness {
    /// Operation id: CONSUME, CREATE, DESTROY, TRANSFER, READ_REF
    pub operation: u8,
    /// Source id: INPUT, CELL_DEP, OUTPUT
    pub source: u8,
    /// Source index
    pub index: u32,
    /// Stable conflict domain hash (from type_script + conflict_key_value, blake3)
    pub conflict_hash: [u8; 32],
    /// Mutable typed-data commitment (from type_script + data, blake3)
    pub typed_data_hash: [u8; 32],
}

/// CellScript scheduler witness payload as emitted by `cellscript`.
///
/// Conflict domain membership is derived from access records: each access carries
/// `conflict_hash`, classified as READ or WRITE by its operation. The former
/// `touches_shared` field has been removed because (1) it was redundant with
/// access-level conflict_hash, (2) its effect_class-based READ/WRITE classification
/// was too coarse for typed-cell actions that mix reads and writes, and (3) no
/// published version constrains this change.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct CellScriptSchedulerWitness {
    /// Magic marker; must be `0xCE11`.
    pub magic: u16,
    /// Witness format version; currently `1`.
    pub version: u8,
    /// Effect class id generated by CellScript.
    pub effect_class: u8,
    /// Whether the action is marked parallelizable by the compiler.
    pub parallelizable: bool,
    /// Compiler-estimated cycles.
    pub estimated_cycles: u64,
    /// Redundant count for admission hardening.
    pub access_count: u32,
    /// Operation/source/index access records with conflict_hash + typed_data_hash.
    pub accesses: Vec<CellScriptSchedulerAccessWitness>,
}

/// Admission error for CellScript scheduler witness bytes.
#[derive(Clone, Debug, PartialEq, Eq, thiserror::Error)]
pub enum CellScriptSchedulerWitnessError {
    /// Decode failed.
    #[error("failed to decode CellScript scheduler witness: {0}")]
    Decode(String),
    /// Wrong magic value.
    #[error("invalid CellScript scheduler witness magic: expected 0xCE11, got 0x{0:04x}")]
    InvalidMagic(u16),
    /// Unsupported version.
    #[error("unsupported CellScript scheduler witness version: {0}")]
    UnsupportedVersion(u8),
    /// Redundant count does not match the vector length.
    #[error("CellScript scheduler witness {field} count mismatch: declared {declared}, actual {actual}")]
    CountMismatch {
        /// Field with a redundant count.
        field: &'static str,
        /// Declared count.
        declared: u32,
        /// Actual vector length.
        actual: usize,
    },
    /// Unknown effect-class id.
    #[error("invalid CellScript scheduler effect class id: {0}")]
    InvalidEffectClass(u8),
    /// Unknown access operation id.
    #[error("invalid CellScript scheduler access operation id: {0}")]
    InvalidOperation(u8),
    /// Unknown access source id.
    #[error("invalid CellScript scheduler access source id: {0}")]
    InvalidSource(u8),
    /// Access operation cannot legally target the declared source class.
    #[error("CellScript scheduler access operation {operation} cannot target source {source_id}")]
    UnexpectedSourceForOperation {
        /// Operation id.
        operation: u8,
        /// Source id.
        source_id: u8,
    },
    /// Access source index is outside the transaction-derived source vector.
    #[error("CellScript scheduler access source {source_id} index {index} is out of bounds; available {available}")]
    SourceIndexOutOfBounds {
        /// Source id.
        source_id: u8,
        /// Requested index.
        index: u32,
        /// Number of available entries in the transaction source vector.
        available: usize,
    },
    /// Decoded witness access set does not match a trusted access summary.
    #[error(
        "CellScript scheduler access set mismatch for operation {operation} source {source_id} index {index} conflict_hash {conflict_hash:?}: expected {expected_count}, actual {actual_count}"
    )]
    AccessSetMismatch {
        /// Operation id.
        operation: u8,
        /// Source id.
        source_id: u8,
        /// Source index.
        index: u32,
        /// Conflict hash.
        conflict_hash: [u8; 32],
        /// Expected multiset count from trusted metadata or builder summary.
        expected_count: usize,
        /// Actual multiset count in the decoded witness.
        actual_count: usize,
    },
    /// Decoded witness metadata does not match a trusted compiler/builder summary.
    #[error("CellScript scheduler trusted summary mismatch in {field}")]
    TrustedSummaryMismatch {
        /// Witness field whose authenticated summary did not match.
        field: &'static str,
    },
    /// More than one CellScript scheduler witness was found on the same transaction.
    #[error("duplicate CellScript scheduler witnesses on one transaction: {count}")]
    DuplicateSchedulerWitness {
        /// Number of scheduler witness slots discovered.
        count: usize,
    },
    /// Access count exceeds the protocol maximum.
    #[error("CellScript scheduler witness access count exceeds max: declared {declared}, max {max}")]
    AccessCountExceedsMax {
        /// Declared access count.
        declared: u32,
        /// Maximum allowed access count.
        max: u32,
    },
    /// All-zero conflict_hash is illegal in typed-cell mode.
    #[error("all-zero conflict_hash is illegal in typed-cell mode")]
    ZeroConflictHash,
}

/// Decode and admit self-contained Molecule CellScript scheduler witness bytes.
///
/// This verifies the witness envelope, effect class, and operation/source
/// combinations. Runtime policy must still compare the decoded accesses with
/// transaction-derived source bounds before using the witness for scheduling.
pub fn decode_cellscript_scheduler_witness(bytes: &[u8]) -> Result<CellScriptSchedulerWitness, CellScriptSchedulerWitnessError> {
    decode_cellscript_scheduler_witness_molecule(bytes)
}

/// Encode a CellScript scheduler witness as the launch Molecule witness schema.
pub fn encode_cellscript_scheduler_witness_molecule(witness: &CellScriptSchedulerWitness) -> Vec<u8> {
    scheduler_molecule_encode_table(&[
        witness.magic.to_le_bytes().to_vec(),
        vec![witness.version],
        vec![witness.effect_class],
        vec![u8::from(witness.parallelizable)],
        witness.estimated_cycles.to_le_bytes().to_vec(),
        witness.access_count.to_le_bytes().to_vec(),
        scheduler_molecule_encode_accesses(&witness.accesses),
    ])
}

/// Decode and admit the launch Molecule CellScript scheduler witness schema.
pub fn decode_cellscript_scheduler_witness_molecule(
    bytes: &[u8],
) -> Result<CellScriptSchedulerWitness, CellScriptSchedulerWitnessError> {
    let witness = decode_cellscript_scheduler_witness_molecule_unchecked(bytes)?;
    validate_cellscript_scheduler_witness_header_and_body(witness)
}

fn decode_cellscript_scheduler_witness_molecule_unchecked(
    bytes: &[u8],
) -> Result<CellScriptSchedulerWitness, CellScriptSchedulerWitnessError> {
    let fields = scheduler_molecule_decode_table(bytes, CELLSCRIPT_SCHEDULER_WITNESS_MOLECULE_FIELDS, "CellScriptSchedulerWitness")
        .map_err(CellScriptSchedulerWitnessError::Decode)?;
    Ok(CellScriptSchedulerWitness {
        magic: scheduler_molecule_decode_u16(fields[0], "CellScriptSchedulerWitness.magic")
            .map_err(CellScriptSchedulerWitnessError::Decode)?,
        version: scheduler_molecule_decode_u8(fields[1], "CellScriptSchedulerWitness.version")
            .map_err(CellScriptSchedulerWitnessError::Decode)?,
        effect_class: scheduler_molecule_decode_u8(fields[2], "CellScriptSchedulerWitness.effect_class")
            .map_err(CellScriptSchedulerWitnessError::Decode)?,
        parallelizable: scheduler_molecule_decode_bool(fields[3], "CellScriptSchedulerWitness.parallelizable")
            .map_err(CellScriptSchedulerWitnessError::Decode)?,
        estimated_cycles: scheduler_molecule_decode_u64(fields[4], "CellScriptSchedulerWitness.estimated_cycles")
            .map_err(CellScriptSchedulerWitnessError::Decode)?,
        access_count: scheduler_molecule_decode_u32(fields[5], "CellScriptSchedulerWitness.access_count")
            .map_err(CellScriptSchedulerWitnessError::Decode)?,
        accesses: scheduler_molecule_decode_accesses(fields[6]).map_err(CellScriptSchedulerWitnessError::Decode)?,
    })
}

fn validate_cellscript_scheduler_witness_header_and_body(
    witness: CellScriptSchedulerWitness,
) -> Result<CellScriptSchedulerWitness, CellScriptSchedulerWitnessError> {
    if witness.magic != 0xCE11 {
        return Err(CellScriptSchedulerWitnessError::InvalidMagic(witness.magic));
    }
    if witness.version != CELLSCRIPT_SCHEDULER_WITNESS_VERSION {
        return Err(CellScriptSchedulerWitnessError::UnsupportedVersion(witness.version));
    }
    validate_cellscript_scheduler_witness_counts(&witness)?;
    validate_cellscript_scheduler_witness_envelope(&witness)?;
    Ok(witness)
}

fn scheduler_molecule_pack_number(value: usize) -> [u8; 4] {
    (value as u32).to_le_bytes()
}

fn scheduler_molecule_unpack_number(bytes: &[u8], ty: &'static str) -> Result<usize, String> {
    if bytes.len() < 4 {
        return Err(format!("{ty}: expected at least 4 bytes for number, got {}", bytes.len()));
    }
    Ok(u32::from_le_bytes([bytes[0], bytes[1], bytes[2], bytes[3]]) as usize)
}

fn scheduler_molecule_encode_table(fields: &[Vec<u8>]) -> Vec<u8> {
    let header_size = 4 * (fields.len() + 1);
    let total_size = header_size + fields.iter().map(Vec::len).sum::<usize>();
    let mut out = Vec::with_capacity(total_size);
    out.extend_from_slice(&scheduler_molecule_pack_number(total_size));

    let mut offset = header_size;
    for field in fields {
        out.extend_from_slice(&scheduler_molecule_pack_number(offset));
        offset += field.len();
    }
    for field in fields {
        out.extend_from_slice(field);
    }
    out
}

fn scheduler_molecule_decode_table<'a>(bytes: &'a [u8], expected_fields: usize, ty: &'static str) -> Result<Vec<&'a [u8]>, String> {
    if bytes.len() < 8 {
        return Err(format!("{ty}: table header is too short: {}", bytes.len()));
    }
    let total_size = scheduler_molecule_unpack_number(bytes, ty)?;
    if total_size != bytes.len() {
        return Err(format!("{ty}: total size mismatch: header {total_size}, actual {}", bytes.len()));
    }

    let first_offset = scheduler_molecule_unpack_number(&bytes[4..], ty)?;
    if first_offset % 4 != 0 || first_offset < 8 || first_offset > bytes.len() {
        return Err(format!("{ty}: invalid first field offset {first_offset}"));
    }

    let field_count = first_offset / 4 - 1;
    if field_count != expected_fields {
        return Err(format!("{ty}: expected {expected_fields} fields, got {field_count}"));
    }

    let mut offsets = Vec::with_capacity(field_count + 1);
    for chunk in bytes[4..first_offset].chunks_exact(4) {
        offsets.push(scheduler_molecule_unpack_number(chunk, ty)?);
    }
    offsets.push(total_size);

    if offsets.windows(2).any(|pair| pair[0] > pair[1]) {
        return Err(format!("{ty}: field offsets are not monotonic"));
    }
    if offsets.iter().any(|offset| *offset < first_offset || *offset > total_size) {
        return Err(format!("{ty}: field offset is outside table payload"));
    }

    Ok(offsets.windows(2).map(|pair| &bytes[pair[0]..pair[1]]).collect())
}

fn scheduler_molecule_encode_accesses(accesses: &[CellScriptSchedulerAccessWitness]) -> Vec<u8> {
    let mut out = Vec::with_capacity(4 + accesses.len() * TYPED_CELL_ACCESS_MOLECULE_SIZE);
    out.extend_from_slice(&scheduler_molecule_pack_number(accesses.len()));
    for access in accesses {
        out.push(access.operation);
        out.push(access.source);
        out.extend_from_slice(&access.index.to_le_bytes());
        out.extend_from_slice(&access.conflict_hash);
        out.extend_from_slice(&access.typed_data_hash);
    }
    out
}

fn scheduler_molecule_decode_accesses(bytes: &[u8]) -> Result<Vec<CellScriptSchedulerAccessWitness>, String> {
    let count = scheduler_molecule_unpack_number(bytes, "TypedCellAccessVec")?;
    let expected = 4 + count * TYPED_CELL_ACCESS_MOLECULE_SIZE;
    if bytes.len() != expected {
        return Err(format!("TypedCellAccessVec: expected {expected} bytes for {count} accesses, got {}", bytes.len()));
    }
    bytes[4..]
        .chunks_exact(TYPED_CELL_ACCESS_MOLECULE_SIZE)
        .map(|chunk| {
            let mut conflict_hash = [0u8; 32];
            conflict_hash.copy_from_slice(&chunk[6..38]);
            let mut typed_data_hash = [0u8; 32];
            typed_data_hash.copy_from_slice(&chunk[38..70]);
            Ok(CellScriptSchedulerAccessWitness {
                operation: chunk[0],
                source: chunk[1],
                index: u32::from_le_bytes([chunk[2], chunk[3], chunk[4], chunk[5]]),
                conflict_hash,
                typed_data_hash,
            })
        })
        .collect()
}

fn scheduler_molecule_decode_u8(bytes: &[u8], ty: &'static str) -> Result<u8, String> {
    if bytes.len() != 1 {
        return Err(format!("{ty}: expected 1 byte, got {}", bytes.len()));
    }
    Ok(bytes[0])
}

fn scheduler_molecule_decode_bool(bytes: &[u8], ty: &'static str) -> Result<bool, String> {
    match scheduler_molecule_decode_u8(bytes, ty)? {
        0 => Ok(false),
        1 => Ok(true),
        other => Err(format!("{ty}: expected boolean byte 0 or 1, got {other}")),
    }
}

fn scheduler_molecule_decode_u16(bytes: &[u8], ty: &'static str) -> Result<u16, String> {
    if bytes.len() != 2 {
        return Err(format!("{ty}: expected 2 bytes, got {}", bytes.len()));
    }
    Ok(u16::from_le_bytes([bytes[0], bytes[1]]))
}

fn scheduler_molecule_decode_u32(bytes: &[u8], ty: &'static str) -> Result<u32, String> {
    if bytes.len() != 4 {
        return Err(format!("{ty}: expected 4 bytes, got {}", bytes.len()));
    }
    Ok(u32::from_le_bytes([bytes[0], bytes[1], bytes[2], bytes[3]]))
}

fn scheduler_molecule_decode_u64(bytes: &[u8], ty: &'static str) -> Result<u64, String> {
    if bytes.len() != 8 {
        return Err(format!("{ty}: expected 8 bytes, got {}", bytes.len()));
    }
    Ok(u64::from_le_bytes([bytes[0], bytes[1], bytes[2], bytes[3], bytes[4], bytes[5], bytes[6], bytes[7]]))
}

/// Decode a CellScript scheduler witness and validate it against a concrete transaction.
pub fn decode_cellscript_scheduler_witness_for_tx(
    tx: &CellTx,
    bytes: &[u8],
) -> Result<CellScriptSchedulerWitness, CellScriptSchedulerWitnessError> {
    let witness = decode_cellscript_scheduler_witness(bytes)?;
    validate_cellscript_scheduler_witness_against_transaction(tx, &witness)?;
    Ok(witness)
}

/// Produce a trusted scheduler access summary from authenticated compiled metadata.
///
/// The `compiled_scheduler_witness` bytes must come from a trusted CellScript
/// compile artifact or transaction-builder input, not from an untrusted
/// transaction witness. The helper admits the bytes against the concrete
/// transaction shape and returns the operation/source/index/conflict_hash
/// multiset that consensus policy can compare against the transaction witness.
pub fn cellscript_compiled_scheduler_accesses_for_tx(
    tx: &CellTx,
    compiled_scheduler_witness: &[u8],
) -> Result<Vec<CellScriptSchedulerAccessWitness>, CellScriptSchedulerWitnessError> {
    cellscript_compiled_scheduler_summary_for_tx(tx, compiled_scheduler_witness).map(|witness| witness.accesses)
}

/// Produce a trusted scheduler summary from authenticated compiled metadata.
///
/// The returned summary is the decoded scheduler witness itself, admitted
/// against the concrete transaction. Consumers can require an untrusted
/// transaction-carried witness to match this full summary before using effect,
/// shared-touch, cycle, or access metadata for scheduling.
pub fn cellscript_compiled_scheduler_summary_for_tx(
    tx: &CellTx,
    compiled_scheduler_witness: &[u8],
) -> Result<CellScriptSchedulerWitness, CellScriptSchedulerWitnessError> {
    decode_cellscript_scheduler_witness_for_tx(tx, compiled_scheduler_witness)
}

/// Validate decoded CellScript scheduler metadata against a concrete transaction.
pub fn validate_cellscript_scheduler_witness_against_transaction(
    tx: &CellTx,
    witness: &CellScriptSchedulerWitness,
) -> Result<(), CellScriptSchedulerWitnessError> {
    validate_cellscript_scheduler_witness_envelope(witness)?;
    for access in &witness.accesses {
        let available = match access.source {
            CELLSCRIPT_SCHEDULER_SOURCE_INPUT => tx.inputs.len(),
            CELLSCRIPT_SCHEDULER_SOURCE_CELL_DEP => tx.cell_deps.len(),
            CELLSCRIPT_SCHEDULER_SOURCE_OUTPUT => tx.outputs.len(),
            source => return Err(CellScriptSchedulerWitnessError::InvalidSource(source)),
        };
        let index = usize::try_from(access.index).unwrap_or(usize::MAX);
        if index >= available {
            return Err(CellScriptSchedulerWitnessError::SourceIndexOutOfBounds {
                source_id: access.source,
                index: access.index,
                available,
            });
        }
    }
    Ok(())
}

/// Validate decoded CellScript scheduler accesses against a trusted access set.
///
/// This is the Phase-3 bridge from advisory metadata to policy-consumable
/// metadata: a transaction builder, compiler artifact, or other trusted source
/// can provide the expected operation/source/index/binding-hash multiset, and
/// the decoded witness must match it exactly before scheduler use.
pub fn validate_cellscript_scheduler_witness_access_set(
    witness: &CellScriptSchedulerWitness,
    expected_accesses: &[CellScriptSchedulerAccessWitness],
) -> Result<(), CellScriptSchedulerWitnessError> {
    validate_cellscript_scheduler_witness_counts(witness)?;
    validate_cellscript_scheduler_witness_envelope(witness)?;
    let mut expected = BTreeMap::<SchedulerAccessKey, usize>::new();
    for access in expected_accesses {
        validate_cellscript_scheduler_access_envelope(access)?;
        *expected.entry(SchedulerAccessKey::from(access)).or_default() += 1;
    }
    let mut actual = BTreeMap::<SchedulerAccessKey, usize>::new();
    for access in &witness.accesses {
        *actual.entry(SchedulerAccessKey::from(access)).or_default() += 1;
    }

    for key in expected.keys().chain(actual.keys()) {
        let expected_count = expected.get(key).copied().unwrap_or(0);
        let actual_count = actual.get(key).copied().unwrap_or(0);
        if expected_count != actual_count {
            return Err(CellScriptSchedulerWitnessError::AccessSetMismatch {
                operation: key.operation,
                source_id: key.source,
                index: key.index,
                conflict_hash: key.conflict_hash,
                expected_count,
                actual_count,
            });
        }
    }
    Ok(())
}

/// Validate decoded CellScript scheduler metadata against a trusted full summary.
///
/// Access records remain order-insensitive and multiplicity-sensitive, while
/// effect class, parallelizability, shared touch multiset, and cycle estimate
/// must match the authenticated compiler/builder summary exactly.
pub fn validate_cellscript_scheduler_witness_summary(
    witness: &CellScriptSchedulerWitness,
    expected: &CellScriptSchedulerWitness,
) -> Result<(), CellScriptSchedulerWitnessError> {
    validate_cellscript_scheduler_witness_counts(witness)?;
    validate_cellscript_scheduler_witness_counts(expected)?;
    validate_cellscript_scheduler_witness_envelope(witness)?;
    validate_cellscript_scheduler_witness_envelope(expected)?;

    if witness.magic != expected.magic {
        return Err(CellScriptSchedulerWitnessError::TrustedSummaryMismatch { field: "magic" });
    }
    if witness.version != expected.version {
        return Err(CellScriptSchedulerWitnessError::TrustedSummaryMismatch { field: "version" });
    }
    if witness.effect_class != expected.effect_class {
        return Err(CellScriptSchedulerWitnessError::TrustedSummaryMismatch { field: "effect_class" });
    }
    if witness.parallelizable != expected.parallelizable {
        return Err(CellScriptSchedulerWitnessError::TrustedSummaryMismatch { field: "parallelizable" });
    }
    if witness.estimated_cycles != expected.estimated_cycles {
        return Err(CellScriptSchedulerWitnessError::TrustedSummaryMismatch { field: "estimated_cycles" });
    }
    validate_cellscript_scheduler_witness_access_set(witness, &expected.accesses)
}

impl CellScriptSchedulerWitness {
    /// Validate this decoded witness against a concrete transaction.
    pub fn validate_against_transaction(&self, tx: &CellTx) -> Result<(), CellScriptSchedulerWitnessError> {
        validate_cellscript_scheduler_witness_against_transaction(tx, self)
    }

    /// Validate this decoded witness against a trusted access multiset.
    pub fn validate_access_set(
        &self,
        expected_accesses: &[CellScriptSchedulerAccessWitness],
    ) -> Result<(), CellScriptSchedulerWitnessError> {
        validate_cellscript_scheduler_witness_access_set(self, expected_accesses)
    }

    /// Validate this decoded witness against a trusted full scheduler summary.
    pub fn validate_summary(&self, expected: &CellScriptSchedulerWitness) -> Result<(), CellScriptSchedulerWitnessError> {
        validate_cellscript_scheduler_witness_summary(self, expected)
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
struct SchedulerAccessKey {
    operation: u8,
    source: u8,
    index: u32,
    conflict_hash: [u8; 32],
}

impl From<&CellScriptSchedulerAccessWitness> for SchedulerAccessKey {
    fn from(access: &CellScriptSchedulerAccessWitness) -> Self {
        Self { operation: access.operation, source: access.source, index: access.index, conflict_hash: access.conflict_hash }
    }
}

fn validate_cellscript_scheduler_witness_envelope(
    witness: &CellScriptSchedulerWitness,
) -> Result<(), CellScriptSchedulerWitnessError> {
    if !matches!(
        witness.effect_class,
        CELLSCRIPT_SCHEDULER_EFFECT_PURE
            | CELLSCRIPT_SCHEDULER_EFFECT_READ_ONLY
            | CELLSCRIPT_SCHEDULER_EFFECT_MUTATING
            | CELLSCRIPT_SCHEDULER_EFFECT_CREATING
            | CELLSCRIPT_SCHEDULER_EFFECT_DESTROYING
    ) {
        return Err(CellScriptSchedulerWitnessError::InvalidEffectClass(witness.effect_class));
    }
    for access in &witness.accesses {
        validate_cellscript_scheduler_access_envelope(access)?;
    }
    Ok(())
}

fn validate_cellscript_scheduler_witness_counts(witness: &CellScriptSchedulerWitness) -> Result<(), CellScriptSchedulerWitnessError> {
    if witness.access_count > MAX_CELLSCRIPT_ACCESS_COUNT {
        return Err(CellScriptSchedulerWitnessError::AccessCountExceedsMax {
            declared: witness.access_count,
            max: MAX_CELLSCRIPT_ACCESS_COUNT,
        });
    }
    if witness.access_count as usize != witness.accesses.len() {
        return Err(CellScriptSchedulerWitnessError::CountMismatch {
            field: "accesses",
            declared: witness.access_count,
            actual: witness.accesses.len(),
        });
    }
    Ok(())
}

fn validate_cellscript_scheduler_access_envelope(
    access: &CellScriptSchedulerAccessWitness,
) -> Result<(), CellScriptSchedulerWitnessError> {
    if !matches!(
        access.operation,
        CELLSCRIPT_SCHEDULER_OP_CONSUME
            | CELLSCRIPT_SCHEDULER_OP_TRANSFER
            | CELLSCRIPT_SCHEDULER_OP_DESTROY
            | CELLSCRIPT_SCHEDULER_OP_READ_REF
            | CELLSCRIPT_SCHEDULER_OP_CREATE
    ) {
        return Err(CellScriptSchedulerWitnessError::InvalidOperation(access.operation));
    }
    if !matches!(
        access.source,
        CELLSCRIPT_SCHEDULER_SOURCE_INPUT | CELLSCRIPT_SCHEDULER_SOURCE_CELL_DEP | CELLSCRIPT_SCHEDULER_SOURCE_OUTPUT
    ) {
        return Err(CellScriptSchedulerWitnessError::InvalidSource(access.source));
    }
    if !cellscript_scheduler_operation_accepts_source(access.operation, access.source) {
        return Err(CellScriptSchedulerWitnessError::UnexpectedSourceForOperation {
            operation: access.operation,
            source_id: access.source,
        });
    }
    // Typed-cell mode: all-zero conflict_hash is illegal.
    // The typed-cell branch only handles typed cells — there is no reason
    // to allow a zero conflict_hash, and doing so would create a collision
    // domain that silently merges unrelated accesses.
    if access.conflict_hash == [0u8; 32] {
        return Err(CellScriptSchedulerWitnessError::ZeroConflictHash);
    }
    Ok(())
}

fn cellscript_scheduler_operation_accepts_source(operation: u8, source: u8) -> bool {
    match operation {
        CELLSCRIPT_SCHEDULER_OP_CONSUME | CELLSCRIPT_SCHEDULER_OP_DESTROY => source == CELLSCRIPT_SCHEDULER_SOURCE_INPUT,
        CELLSCRIPT_SCHEDULER_OP_READ_REF => {
            source == CELLSCRIPT_SCHEDULER_SOURCE_CELL_DEP || source == CELLSCRIPT_SCHEDULER_SOURCE_INPUT
        }
        CELLSCRIPT_SCHEDULER_OP_CREATE => source == CELLSCRIPT_SCHEDULER_SOURCE_OUTPUT,
        CELLSCRIPT_SCHEDULER_OP_TRANSFER => {
            source == CELLSCRIPT_SCHEDULER_SOURCE_INPUT || source == CELLSCRIPT_SCHEDULER_SOURCE_OUTPUT
        }
        _ => false,
    }
}

/// Structured capacity validation error shared by Cell outputs and Cell metadata.
#[derive(Clone, Copy, Debug, PartialEq, Eq, thiserror::Error)]
pub enum CapacityError {
    /// The declared capacity is below the minimum occupied capacity.
    #[error("insufficient capacity: required {required}, available {available}")]
    InsufficientCapacity {
        /// Minimum occupied capacity required by the cell shape.
        required: u64,
        /// Capacity declared by the offending value.
        available: u64,
    },
}

/// Script hash format selector.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ScriptHashVersion {
    /// Domain-separated format with an explicit version byte.
    V1,
}

/// OutPoint: uniquely identifies a Cell (tx_hash || output_index)
///
/// Reference: CKB OutPoint
#[derive(Clone, Copy, Default, Debug, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize)]
pub struct OutPoint {
    /// Transaction hash (32 bytes), serialized as hex string `transactionId` in JSON
    #[serde(rename = "transactionId", with = "outpoint_serde")]
    pub tx_hash: [u8; 32],
    /// Output index (u32)
    pub index: u32,
}

impl OutPoint {
    /// Create a new OutPoint
    pub fn new(tx_hash: [u8; 32], index: u32) -> Self {
        Self { tx_hash, index }
    }

    /// Encode to 36-byte key for indexing
    pub fn to_key(&self) -> [u8; 36] {
        let mut key = [0u8; 36];
        key[..32].copy_from_slice(&self.tx_hash);
        key[32..].copy_from_slice(&self.index.to_le_bytes());
        key
    }

    /// Decode from 36-byte key
    pub fn from_key(key: &[u8; 36]) -> Self {
        let mut tx_hash = [0u8; 32];
        tx_hash.copy_from_slice(&key[..32]);
        let index = u32::from_le_bytes([key[32], key[33], key[34], key[35]]);
        Self { tx_hash, index }
    }
}

impl fmt::Display for OutPoint {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        for byte in &self.tx_hash {
            write!(f, "{:02x}", byte)?;
        }
        write!(f, ":{}", self.index)
    }
}

/// Script reference (Lock or Type script)
///
/// Reference: CKB Script
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Script {
    /// Script code hash (points to a Cell's data)
    pub code_hash: [u8; 32],
    /// Hash type: 0=Data, 1=Type, 2=Data1, 4=Data2
    ///
    /// NOTE: Aligned with CKB ScriptHashType encoding:
    /// - Data = 0
    /// - Type = 1
    /// - Data1 = 2
    /// - Data2 = 4 (NOT 3, to maintain CKB compatibility)
    pub hash_type: u8,
    /// Script arguments (passed to VM)
    pub args: Vec<u8>,
}

impl Script {
    /// Create a new script reference
    pub fn new(code_hash: [u8; 32], hash_type: u8, args: Vec<u8>) -> Self {
        Self { code_hash, hash_type, args }
    }

    /// Calculate the canonical script hash currently used by the protocol.
    pub fn hash(&self) -> [u8; 32] {
        self.hash_v1()
    }

    /// Calculate the V1 script hash with explicit domain separation and versioning.
    pub fn hash_v1(&self) -> [u8; 32] {
        let mut hasher = blake3::Hasher::new();
        hasher.update(SCRIPT_HASH_V1_DOMAIN);
        hasher.update(&[1u8]);
        hasher.update(&self.code_hash);
        hasher.update(&[self.hash_type]);
        hasher.update(&(self.args.len() as u32).to_le_bytes());
        hasher.update(&self.args);
        *hasher.finalize().as_bytes()
    }

    /// Calculate the script hash using an explicit format version.
    pub fn hash_with_version(&self, version: ScriptHashVersion) -> [u8; 32] {
        match version {
            ScriptHashVersion::V1 => self.hash_v1(),
        }
    }

    /// Serialize the script reference to bytes.
    ///
    /// Format: code_hash (32) || hash_type (1) || args (variable)
    /// This is used by txscript opcodes that inspect output script data.
    pub fn to_bytes(&self) -> Vec<u8> {
        let mut bytes = Vec::with_capacity(33 + self.args.len());
        bytes.extend_from_slice(&self.code_hash);
        bytes.push(self.hash_type);
        bytes.extend_from_slice(&self.args);
        bytes
    }
}

/// Cell output structure
///
/// Note: data field is separated to CellTx.outputs_data (CKB optimization)
///
/// Reference: CKB CellOutput
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct CellOutput {
    /// Lock script: defines who can spend this Cell
    pub lock: Script,
    /// Type script (optional): defines state transition constraints
    pub type_: Option<Script>,
    /// Capacity (saus): amount + storage cost
    pub capacity: u64,
    // ⚠️ NO data field here! Data is in CellTx.outputs_data
}

impl CellOutput {
    /// Calculate occupied capacity (minimum required)
    pub fn occupied_capacity(&self, data_len: usize) -> u64 {
        let mut size = 8; // capacity field
        size += 32 + 1 + self.lock.args.len(); // lock script
        if let Some(ref type_script) = self.type_ {
            size += 32 + 1 + type_script.args.len(); // type script
        }
        size += data_len; // data
        size as u64
    }

    /// Verify capacity is sufficient
    pub fn verify_capacity(&self, data_len: usize) -> Result<(), CapacityError> {
        let occupied = self.occupied_capacity(data_len);
        if self.capacity < occupied {
            return Err(CapacityError::InsufficientCapacity { required: occupied, available: self.capacity });
        }
        Ok(())
    }
}

/// Cell input reference
///
/// Reference: CKB CellInput
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct CellInput {
    /// Previous output: which Cell to spend (CKB calls this previous_output)
    pub previous_output: OutPoint,
    /// Since: time lock (relative/absolute, timestamp/block number)
    /// Bit 63: 0=absolute, 1=relative
    /// Bit 62: 0=timestamp, 1=block number
    /// Bit 61-0: lock value
    pub since: u64,
}

impl CellInput {
    /// Create a new cell reference
    pub fn new(previous_output: OutPoint, since: u64) -> Self {
        Self { previous_output, since }
    }

    /// Check if this is a relative time lock
    pub fn is_relative_lock(&self) -> bool {
        (self.since & 0x8000_0000_0000_0000) != 0
    }

    /// Check if this uses block number (vs timestamp)
    pub fn is_block_number_lock(&self) -> bool {
        (self.since & 0x4000_0000_0000_0000) != 0
    }

    /// Get the lock value
    pub fn lock_value(&self) -> u64 {
        self.since & 0x3FFF_FFFF_FFFF_FFFF
    }
}

/// Cell dependency
///
/// Reference: CKB CellDep
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct CellDep {
    /// OutPoint: which Cell to depend on
    pub out_point: OutPoint,
    /// Dependency type
    pub dep_type: DepType,
}

/// Dependency type
///
/// Reference: CKB DepType
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[repr(u8)]
pub enum DepType {
    /// Code: single Cell as script code
    Code = 0,
    /// DepGroup: a Cell containing multiple OutPoints (batch dependency)
    DepGroup = 1,
}

/// DepGroup cell-data ABI.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum DepGroupDataAbi {
    /// Myelin's existing count-prefixed OutPoint list. Empty lists are accepted
    /// for compatibility with existing Myelin tests/builders.
    Myelin,
    /// CKB Molecule `OutPointVec`. The byte layout matches the non-empty Myelin
    /// list, but CKB rejects empty DepGroup data.
    CkbMolecule,
}

/// Parse DepGroup cell data as a list of OutPoints.
///
/// Format: 4-byte LE count, then count × 36-byte entries
/// (32-byte tx_hash + 4-byte LE index per OutPoint).
pub fn parse_dep_group_data(data: &[u8]) -> Result<Vec<OutPoint>, String> {
    if data.len() < 4 {
        return Err("DepGroup data too short for count header".into());
    }
    let count = u32::from_le_bytes([data[0], data[1], data[2], data[3]]) as usize;
    let expected = 4 + count * 36;
    if data.len() != expected {
        return Err(format!("DepGroup data length mismatch: expected {} bytes for {} outpoints, got {}", expected, count, data.len()));
    }
    let mut outpoints = Vec::with_capacity(count);
    for i in 0..count {
        let offset = 4 + i * 36;
        let key: &[u8; 36] = data[offset..offset + 36].try_into().map_err(|_| "slice conversion failed")?;
        outpoints.push(OutPoint::from_key(key));
    }
    Ok(outpoints)
}

/// Encode a list of OutPoints into DepGroup cell data format.
///
/// This is the inverse of [`parse_dep_group_data`].
pub fn encode_dep_group_data(outpoints: &[OutPoint]) -> Vec<u8> {
    let mut data = Vec::with_capacity(4 + outpoints.len() * 36);
    data.extend_from_slice(&(outpoints.len() as u32).to_le_bytes());
    for op in outpoints {
        data.extend_from_slice(&op.to_key());
    }
    data
}

/// Parse DepGroup cell data for an explicit target ABI.
pub fn parse_dep_group_data_for_abi(data: &[u8], abi: DepGroupDataAbi) -> Result<Vec<OutPoint>, String> {
    match abi {
        DepGroupDataAbi::Myelin => parse_dep_group_data(data),
        DepGroupDataAbi::CkbMolecule => {
            crate::serialization::molecule_compat::deserialize_ckb_outpoint_vec_molecule(data).map_err(|error| error.to_string())
        }
    }
}

/// Encode DepGroup cell data for an explicit target ABI.
pub fn encode_dep_group_data_for_abi(outpoints: &[OutPoint], abi: DepGroupDataAbi) -> Result<Vec<u8>, String> {
    match abi {
        DepGroupDataAbi::Myelin => Ok(encode_dep_group_data(outpoints)),
        DepGroupDataAbi::CkbMolecule => {
            crate::serialization::molecule_compat::serialize_ckb_outpoint_vec_molecule(outpoints).map_err(|error| error.to_string())
        }
    }
}

/// Parse CKB Molecule `OutPointVec` DepGroup cell data.
pub fn parse_ckb_dep_group_data(data: &[u8]) -> Result<Vec<OutPoint>, String> {
    parse_dep_group_data_for_abi(data, DepGroupDataAbi::CkbMolecule)
}

/// Encode CKB Molecule `OutPointVec` DepGroup cell data.
pub fn encode_ckb_dep_group_data(outpoints: &[OutPoint]) -> Result<Vec<u8>, String> {
    encode_dep_group_data_for_abi(outpoints, DepGroupDataAbi::CkbMolecule)
}

/// Cell transaction (complete structure)
///
/// Reference: CKB Transaction
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct CellTx {
    /// Transaction version: 0xC001 (Cell v1)
    pub version: u32,
    /// Inputs: Cells to spend
    pub inputs: Vec<CellInput>,
    /// Cell dependencies: read-only Cells (e.g., script code)
    pub cell_deps: Vec<CellDep>,
    /// Header dependencies available to VM scripts.
    pub header_deps: Vec<[u8; 32]>,
    /// Outputs: new Cells to create
    pub outputs: Vec<CellOutput>,
    /// Output data (1:1 with outputs)
    /// Note: CKB separates outputs and data for verification optimization
    pub outputs_data: Vec<Vec<u8>>,
    /// Witnesses: signatures, multi-sig scripts, etc.
    pub witnesses: Vec<Vec<u8>>,
}

impl CellTx {
    /// Create a new Cell transaction
    pub fn new(
        inputs: Vec<CellInput>,
        cell_deps: Vec<CellDep>,
        outputs: Vec<CellOutput>,
        outputs_data: Vec<Vec<u8>>,
        witnesses: Vec<Vec<u8>>,
    ) -> Result<Self, &'static str> {
        Self::new_with_header_deps(inputs, cell_deps, vec![], outputs, outputs_data, witnesses)
    }

    /// Create a new Cell transaction with explicit header dependencies.
    pub fn new_with_header_deps(
        inputs: Vec<CellInput>,
        cell_deps: Vec<CellDep>,
        header_deps: Vec<[u8; 32]>,
        outputs: Vec<CellOutput>,
        outputs_data: Vec<Vec<u8>>,
        witnesses: Vec<Vec<u8>>,
    ) -> Result<Self, &'static str> {
        if outputs.len() != outputs_data.len() {
            return Err("outputs and outputs_data length mismatch");
        }
        Ok(Self { version: CELL_TX_VERSION, inputs, cell_deps, header_deps, outputs, outputs_data, witnesses })
    }

    /// Append a CellScript scheduler witness to the transaction witness vector.
    ///
    /// The witness is kept as an ordinary transaction witness so existing txid /
    /// wtxid and VM loading rules remain unchanged. Phase-3 scheduler policy can
    /// scan for this marker without trusting source-level sidecars.
    pub fn push_cellscript_scheduler_witness(&mut self, scheduler_witness: Vec<u8>) -> Result<(), &'static str> {
        if !is_cellscript_scheduler_witness_bytes(&scheduler_witness) {
            return Err("invalid CellScript scheduler witness header");
        }
        if self.cellscript_scheduler_witnesses().next().is_some() {
            return Err("duplicate CellScript scheduler witness");
        }
        self.witnesses.push(scheduler_witness);
        Ok(())
    }

    /// Return a transaction with an appended CellScript scheduler witness.
    pub fn with_cellscript_scheduler_witness(mut self, scheduler_witness: Vec<u8>) -> Result<Self, &'static str> {
        self.push_cellscript_scheduler_witness(scheduler_witness)?;
        Ok(self)
    }

    /// Append a trusted compiled CellScript scheduler witness and return its access summary.
    ///
    /// This is the transaction-builder side of the Phase-3 trust boundary:
    /// callers pass bytes from authenticated compiled metadata, this method
    /// admits them against the concrete transaction's Input/CellDep/Output
    /// vectors, appends the witness bytes, and returns the access multiset that
    /// consensus policy can later require before scheduler merge.
    pub fn push_cellscript_compiled_scheduler_witness(
        &mut self,
        compiled_scheduler_witness: Vec<u8>,
    ) -> Result<CellScriptSchedulerWitness, CellScriptSchedulerWitnessError> {
        let existing_count = self.cellscript_scheduler_witnesses().count();
        if existing_count > 0 {
            return Err(CellScriptSchedulerWitnessError::DuplicateSchedulerWitness { count: existing_count + 1 });
        }
        let trusted_summary = cellscript_compiled_scheduler_summary_for_tx(self, &compiled_scheduler_witness)?;
        self.witnesses.push(compiled_scheduler_witness);
        Ok(trusted_summary)
    }

    /// Return a transaction with an appended compiled CellScript scheduler witness.
    pub fn with_cellscript_compiled_scheduler_witness(
        mut self,
        compiled_scheduler_witness: Vec<u8>,
    ) -> Result<(Self, CellScriptSchedulerWitness), CellScriptSchedulerWitnessError> {
        let trusted_summary = self.push_cellscript_compiled_scheduler_witness(compiled_scheduler_witness)?;
        Ok((self, trusted_summary))
    }

    /// Iterate over witness slots that appear to carry CellScript scheduler metadata.
    ///
    /// The iterator includes malformed scheduler candidates so policy code can
    /// reject them explicitly instead of ignoring attacker-controlled bytes.
    pub fn cellscript_scheduler_witnesses(&self) -> impl Iterator<Item = &[u8]> {
        self.witnesses.iter().map(Vec::as_slice).filter(|witness| is_cellscript_scheduler_witness_candidate_bytes(witness))
    }

    /// Decode all CellScript scheduler witnesses carried by this transaction.
    pub fn decoded_cellscript_scheduler_witnesses(
        &self,
    ) -> impl Iterator<Item = Result<CellScriptSchedulerWitness, CellScriptSchedulerWitnessError>> + '_ {
        self.cellscript_scheduler_witnesses().map(decode_cellscript_scheduler_witness)
    }

    /// Decode and validate all CellScript scheduler witnesses against this transaction.
    pub fn admitted_cellscript_scheduler_witnesses(
        &self,
    ) -> impl Iterator<Item = Result<CellScriptSchedulerWitness, CellScriptSchedulerWitnessError>> + '_ {
        self.cellscript_scheduler_witnesses().map(move |witness| decode_cellscript_scheduler_witness_for_tx(self, witness))
    }

    /// Get transaction ID (same as compute_txid)
    pub fn id(&self) -> [u8; 32] {
        crate::celltx::compute_txid(self)
    }

    /// Get transaction version
    pub fn version(&self) -> u32 {
        self.version
    }

    /// Check if this is a cellbase-style transaction.
    ///
    /// Cellbase-style transactions have no inputs and are reserved for explicit
    /// session genesis or issuance contexts.
    pub fn is_coinbase(&self) -> bool {
        self.inputs.is_empty()
    }

    /// Get the compute-side mass hint of the transaction.
    ///
    /// This is a deterministic pre-VM compute hint aligned with the
    /// consensus-side non-contextual mass policy:
    ///
    /// - serialized bytes
    /// - output lock/type script bytes
    /// - one implicit sigop per input
    ///
    /// It does not include actual VM-verified cycles, so consensus and mempool
    /// callers must continue using the unified mass pipeline when they need the
    /// authoritative `effective_compute_mass`.
    pub fn estimated_compute_mass(&self) -> u64 {
        let serialized_size = self.serialized_size() as u64;
        let size_mass = serialized_size.saturating_mul(MASS_PER_TX_BYTE);
        let script_mass = self.total_output_script_bytes().saturating_mul(MASS_PER_SCRIPT_PUB_KEY_BYTE);
        let sigops_mass = (self.inputs.len() as u64).saturating_mul(MASS_PER_SIG_OP);
        size_mass.saturating_add(script_mass).saturating_add(sigops_mass)
    }

    /// Get the transient-storage mass of the transaction.
    ///
    /// This tracks temporary mempool/relay footprint using a deterministic
    /// serialized-size based factor before contextual execution data exists.
    pub fn estimated_transient_mass(&self) -> u64 {
        (self.serialized_size() as u64).saturating_mul(TRANSIENT_BYTE_TO_MASS_FACTOR)
    }

    fn total_output_script_bytes(&self) -> u64 {
        self.outputs
            .iter()
            .map(|output| {
                let mut script_size = 32 + 1 + output.lock.args.len() as u64;
                if let Some(ref type_script) = output.type_ {
                    script_size = script_size.saturating_add(32 + 1 + type_script.args.len() as u64);
                }
                script_size
            })
            .sum()
    }

    /// Get the storage-side mass of the transaction.
    ///
    /// This tracks the persistent live-cell footprint created by outputs,
    /// including per-entry overhead in the state commitment layer.
    ///
    /// It remains an output-footprint estimate and is not the contextual
    /// KIP-0009 storage truth used after input resolution.
    pub fn estimated_storage_mass(&self) -> u64 {
        self.outputs
            .iter()
            .zip(self.outputs_data.iter())
            .map(|(output, data)| CELL_ENTRY_OVERHEAD_EXCLUDING_OUTPUT_BODY + output.occupied_capacity(data.len()))
            .sum()
    }

    /// Get cellbase-style payload from the first output data or fallback witness.
    pub fn payload(&self) -> Option<&[u8]> {
        if !self.is_coinbase() {
            return None;
        }

        if let Some(first_output_data) = self.outputs_data.first() {
            return Some(first_output_data);
        }

        self.witnesses.first().map(Vec::as_slice)
    }

    /// Estimate serialized size using the canonical shared estimator.
    pub fn serialized_size(&self) -> usize {
        cell_tx_estimated_serialized_size(self) as usize
    }

    /// Calculate total input capacity (requires resolved inputs)
    pub fn input_capacity(&self, resolved_inputs: &[ResolvedCellMeta]) -> u64 {
        resolved_inputs.iter().map(|m| m.cell_output.capacity).sum()
    }

    /// Calculate total output capacity
    pub fn output_capacity(&self) -> u64 {
        self.outputs.iter().map(|o| o.capacity).sum()
    }

    /// Calculate fee (input_capacity - output_capacity)
    pub fn fee(&self, resolved_inputs: &[ResolvedCellMeta]) -> u64 {
        self.input_capacity(resolved_inputs).saturating_sub(self.output_capacity())
    }
}

/// Cell metadata for resolved execution inputs.
///
/// Reference: CKB CellMeta, specialized for resolved execution inputs.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ResolvedCellMeta {
    /// Cell output structure
    pub cell_output: CellOutput,
    /// OutPoint
    pub out_point: OutPoint,
    /// CKB-style transaction inclusion info.
    pub transaction_info: Option<TransactionInfo>,
    /// Data size (bytes)
    pub data_bytes: u64,
    /// In-memory cell data cache
    pub mem_cell_data: Option<Vec<u8>>,
    /// In-memory cell data hash cache
    pub mem_cell_data_hash: Option<[u8; 32]>,
}

impl ResolvedCellMeta {
    /// Check if this is a cellbase-style resolved cell.
    pub fn is_cellbase(&self) -> bool {
        self.transaction_info.as_ref().map(|info| info.is_cellbase).unwrap_or(false)
    }

    /// Get capacity
    pub fn capacity(&self) -> u64 {
        self.cell_output.capacity
    }
}

/// CKB-style transaction inclusion information.
///
/// Compatibility transaction information carried by resolved cells.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct TransactionInfo {
    /// Transaction hash
    pub tx_hash: [u8; 32],
    /// Linear block number containing the transaction.
    pub block_number: u64,
    /// Block hash containing the transaction.
    pub block_hash: [u8; 32],
    /// Is this a cellbase transaction?
    pub is_cellbase: bool,
}

/// Cell status (for queries)
///
/// Reference: CKB CellStatus
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum CellStatus {
    /// Cell exists and is unspent
    Live(Box<ResolvedCellMeta>),
    /// Cell has been spent at the given block number.
    Dead(u64),
    /// Cell not found in index
    Unknown,
}

/// Resolved Cell transaction (all inputs/deps loaded)
///
/// Reference: CKB ResolvedTransaction
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ResolvedCellTx {
    /// The transaction
    pub transaction: CellTx,
    /// Resolved inputs
    pub resolved_inputs: Vec<ResolvedCellMeta>,
    /// Resolved dependencies
    pub resolved_deps: Vec<ResolvedCellMeta>,
}

impl AsRef<CellTx> for CellTx {
    fn as_ref(&self) -> &CellTx {
        self
    }
}

impl ResolvedCellTx {
    /// Calculate fee
    pub fn fee(&self) -> u64 {
        self.transaction.fee(&self.resolved_inputs)
    }

    /// Calculate effective fee rate (considering size and cycles)
    pub fn effective_fee_rate(&self, cycles: u64) -> f64 {
        const CYCLES_PER_BYTE: f64 = 100.0;
        let size = self.transaction.serialized_size() as f64;
        let cycles_size = cycles as f64 / CYCLES_PER_BYTE;
        let effective_size = size.max(cycles_size);
        self.fee() as f64 / effective_size
    }
}

// ============================================================================
// VersionedSerializable Implementations
// ============================================================================
//
// These implementations enable schema versioning for storage layer types.
// All Cell transaction types use version 1 as the initial schema version.

use crate::serialization::VersionedSerializable;

/// Current schema version for Cell transaction types
pub const CELLTX_SCHEMA_VERSION: u8 = 1;

impl VersionedSerializable for OutPoint {
    const CURRENT_VERSION: u8 = CELLTX_SCHEMA_VERSION;

    fn to_versioned_payload(&self) -> Result<Vec<u8>, crate::serialization::SerializationError> {
        crate::serialization::molecule_compat::serialize_outpoint_molecule(self).map_err(Into::into)
    }

    fn upgrade_from(version: u8, bytes: &[u8]) -> Result<Self, crate::serialization::SerializationError> {
        ensure_celltx_schema_version(version)?;
        crate::serialization::molecule_compat::deserialize_outpoint_molecule(bytes).map_err(Into::into)
    }
}

impl VersionedSerializable for Script {
    const CURRENT_VERSION: u8 = CELLTX_SCHEMA_VERSION;

    fn to_versioned_payload(&self) -> Result<Vec<u8>, crate::serialization::SerializationError> {
        crate::serialization::molecule_compat::serialize_script_molecule(self).map_err(Into::into)
    }

    fn upgrade_from(version: u8, bytes: &[u8]) -> Result<Self, crate::serialization::SerializationError> {
        ensure_celltx_schema_version(version)?;
        crate::serialization::molecule_compat::deserialize_script_molecule(bytes).map_err(Into::into)
    }
}

impl VersionedSerializable for CellOutput {
    const CURRENT_VERSION: u8 = CELLTX_SCHEMA_VERSION;

    fn to_versioned_payload(&self) -> Result<Vec<u8>, crate::serialization::SerializationError> {
        crate::serialization::molecule_compat::serialize_cell_output_molecule(self).map_err(Into::into)
    }

    fn upgrade_from(version: u8, bytes: &[u8]) -> Result<Self, crate::serialization::SerializationError> {
        ensure_celltx_schema_version(version)?;
        crate::serialization::molecule_compat::deserialize_cell_output_molecule(bytes).map_err(Into::into)
    }
}

impl VersionedSerializable for CellInput {
    const CURRENT_VERSION: u8 = CELLTX_SCHEMA_VERSION;

    fn to_versioned_payload(&self) -> Result<Vec<u8>, crate::serialization::SerializationError> {
        crate::serialization::molecule_compat::serialize_cell_input_molecule(self).map_err(Into::into)
    }

    fn upgrade_from(version: u8, bytes: &[u8]) -> Result<Self, crate::serialization::SerializationError> {
        ensure_celltx_schema_version(version)?;
        crate::serialization::molecule_compat::deserialize_cell_input_molecule(bytes).map_err(Into::into)
    }
}

impl VersionedSerializable for CellDep {
    const CURRENT_VERSION: u8 = CELLTX_SCHEMA_VERSION;

    fn to_versioned_payload(&self) -> Result<Vec<u8>, crate::serialization::SerializationError> {
        crate::serialization::molecule_compat::serialize_cell_dep_molecule(self).map_err(Into::into)
    }

    fn upgrade_from(version: u8, bytes: &[u8]) -> Result<Self, crate::serialization::SerializationError> {
        ensure_celltx_schema_version(version)?;
        crate::serialization::molecule_compat::deserialize_cell_dep_molecule(bytes).map_err(Into::into)
    }
}

impl VersionedSerializable for DepType {
    const CURRENT_VERSION: u8 = CELLTX_SCHEMA_VERSION;

    fn to_versioned_payload(&self) -> Result<Vec<u8>, crate::serialization::SerializationError> {
        Ok(vec![match self {
            DepType::Code => 0,
            DepType::DepGroup => 1,
        }])
    }

    fn upgrade_from(version: u8, bytes: &[u8]) -> Result<Self, crate::serialization::SerializationError> {
        ensure_celltx_schema_version(version)?;
        match bytes {
            [0] => Ok(DepType::Code),
            [1] => Ok(DepType::DepGroup),
            _ => Err(crate::serialization::SerializationError::DeserializationFailed(format!(
                "invalid DepType Molecule payload length/value: {bytes:?}"
            ))),
        }
    }
}

impl VersionedSerializable for CellTx {
    const CURRENT_VERSION: u8 = CELLTX_SCHEMA_VERSION;

    fn to_versioned_payload(&self) -> Result<Vec<u8>, crate::serialization::SerializationError> {
        crate::serialization::molecule_compat::serialize_transaction_molecule(self).map_err(Into::into)
    }

    fn upgrade_from(version: u8, bytes: &[u8]) -> Result<Self, crate::serialization::SerializationError> {
        ensure_celltx_schema_version(version)?;
        crate::serialization::molecule_compat::deserialize_transaction_molecule(bytes).map_err(Into::into)
    }
}

impl VersionedSerializable for TransactionInfo {
    const CURRENT_VERSION: u8 = CELLTX_SCHEMA_VERSION;
}

impl VersionedSerializable for ResolvedCellMeta {
    const CURRENT_VERSION: u8 = CELLTX_SCHEMA_VERSION;
}

impl VersionedSerializable for ResolvedCellTx {
    const CURRENT_VERSION: u8 = CELLTX_SCHEMA_VERSION;
}

fn ensure_celltx_schema_version(version: u8) -> Result<(), crate::serialization::SerializationError> {
    if version == CELLTX_SCHEMA_VERSION {
        Ok(())
    } else {
        Err(crate::serialization::SerializationError::UpgradePathNotAvailable { from: version, to: CELLTX_SCHEMA_VERSION })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use proptest::prelude::*;

    #[test]
    fn test_outpoint_key_encoding() {
        let op = OutPoint::new([0x42; 32], 0x12345678);
        let key = op.to_key();
        let decoded = OutPoint::from_key(&key);
        assert_eq!(op, decoded);
    }

    #[test]
    fn test_script_hash() {
        let script = Script::new([0x11; 32], 1, vec![0xAA, 0xBB]);
        let hash = script.hash();
        assert_eq!(hash.len(), 32);
    }

    #[test]
    fn test_script_hash_v1_is_versioned_and_distinct() {
        let script = Script::new([0x11; 32], 1, vec![0xAA, 0xBB]);
        let canonical = script.hash();
        let versioned = script.hash_v1();

        assert_eq!(canonical, versioned);
        assert_eq!(versioned, script.hash_with_version(ScriptHashVersion::V1));
    }

    #[test]
    fn test_cell_out_capacity() {
        let lock = Script::new([0x00; 32], 0, vec![0; 20]);
        let cell = CellOutput { lock, type_: None, capacity: 1000 };
        let occupied = cell.occupied_capacity(100);
        assert!(occupied > 0);
        assert!(cell.verify_capacity(100).is_ok());
        assert_eq!(
            CellOutput { lock: Script::new([0x00; 32], 0, vec![0; 20]), type_: None, capacity: 10 }.verify_capacity(100),
            Err(CapacityError::InsufficientCapacity { required: occupied, available: 10 })
        );
    }

    #[test]
    fn test_time_lock_flags() {
        let relative_block_number_lock = CellInput::new(
            OutPoint::new([0; 32], 0),
            0xC000_0000_0000_0064, // relative + block number + value=100
        );
        assert!(relative_block_number_lock.is_relative_lock());
        assert!(relative_block_number_lock.is_block_number_lock());
        assert_eq!(relative_block_number_lock.lock_value(), 100);
    }

    #[test]
    fn test_celltx_creation() {
        let inputs = vec![CellInput::new(OutPoint::new([0; 32], 0), 0)];
        let deps = vec![];
        let lock = Script::new([0x00; 32], 0, vec![]);
        let outputs = vec![CellOutput { lock, type_: None, capacity: 1000 }];
        let outputs_data = vec![vec![]];
        let witnesses = vec![vec![0; 65]];

        let tx = CellTx::new(inputs, deps, outputs, outputs_data, witnesses);
        assert!(tx.is_ok());
        let tx = tx.unwrap();
        assert_eq!(tx.version, CELL_TX_VERSION);
    }

    #[test]
    fn test_cellscript_scheduler_witness_placement() {
        let lock = Script::new([0x00; 32], 0, vec![]);
        let outputs = vec![CellOutput { lock, type_: None, capacity: 1000 }];
        let outputs_data = vec![vec![]];
        let mut tx = CellTx::new(vec![], vec![], outputs, outputs_data, vec![vec![0xAA]]).unwrap();
        let expected_access = CellScriptSchedulerAccessWitness {
            operation: CELLSCRIPT_SCHEDULER_OP_CREATE,
            source: CELLSCRIPT_SCHEDULER_SOURCE_OUTPUT,
            index: 0,
            conflict_hash: [0x24; 32],
            typed_data_hash: [0x00; 32],
        };
        let scheduler_witness = encode_cellscript_scheduler_witness_molecule(&CellScriptSchedulerWitness {
            magic: 0xCE11,
            version: CELLSCRIPT_SCHEDULER_WITNESS_VERSION,
            effect_class: CELLSCRIPT_SCHEDULER_EFFECT_CREATING,
            parallelizable: false,

            estimated_cycles: 64,
            access_count: 1,
            accesses: vec![expected_access.clone()],
        });

        assert!(is_cellscript_scheduler_witness_bytes(&scheduler_witness));
        tx.push_cellscript_scheduler_witness(scheduler_witness.clone()).unwrap();

        assert_eq!(tx.witnesses.last().map(Vec::as_slice), Some(scheduler_witness.as_slice()));
        let scheduler_witnesses = tx.cellscript_scheduler_witnesses().collect::<Vec<_>>();
        assert_eq!(scheduler_witnesses, vec![scheduler_witness.as_slice()]);
        let decoded = tx.decoded_cellscript_scheduler_witnesses().collect::<Result<Vec<_>, _>>().unwrap();
        assert_eq!(decoded.len(), 1);
        assert_eq!(decoded[0].magic, 0xCE11);
        assert_eq!(decoded[0].accesses[0].operation, CELLSCRIPT_SCHEDULER_OP_CREATE);
        decoded[0].validate_access_set(&[expected_access]).unwrap();
        let admitted = tx.admitted_cellscript_scheduler_witnesses().collect::<Result<Vec<_>, _>>().unwrap();
        assert_eq!(admitted.len(), 1);
        assert!(tx.with_cellscript_scheduler_witness(vec![0x11, 0xCE, 2]).is_err());
        assert!(!is_cellscript_scheduler_witness_bytes(&[0x11, 0xCE]));
        assert!(!is_cellscript_scheduler_witness_bytes(&[0xCE, 0x11, 1]));
    }

    #[test]
    fn test_cellscript_scheduler_witness_molecule_placement() {
        let lock = Script::new([0x00; 32], 0, vec![]);
        let outputs = vec![CellOutput { lock, type_: None, capacity: 1000 }];
        let outputs_data = vec![vec![]];
        let mut tx = CellTx::new(vec![], vec![], outputs, outputs_data, vec![vec![0xAA]]).unwrap();
        let expected_access = CellScriptSchedulerAccessWitness {
            operation: CELLSCRIPT_SCHEDULER_OP_CREATE,
            source: CELLSCRIPT_SCHEDULER_SOURCE_OUTPUT,
            index: 0,
            conflict_hash: [0x24; 32],
            typed_data_hash: [0x00; 32],
        };
        let witness = CellScriptSchedulerWitness {
            magic: 0xCE11,
            version: CELLSCRIPT_SCHEDULER_WITNESS_VERSION,
            effect_class: CELLSCRIPT_SCHEDULER_EFFECT_CREATING,
            parallelizable: false,

            estimated_cycles: 64,
            access_count: 1,
            accesses: vec![expected_access.clone()],
        };
        let scheduler_witness = encode_cellscript_scheduler_witness_molecule(&witness);

        assert!(!scheduler_witness.starts_with(&CELLSCRIPT_SCHEDULER_WITNESS_MAGIC));
        assert!(is_cellscript_scheduler_witness_bytes(&scheduler_witness));
        assert_eq!(decode_cellscript_scheduler_witness_molecule(&scheduler_witness).unwrap(), witness);
        assert_eq!(decode_cellscript_scheduler_witness(&scheduler_witness).unwrap(), witness);

        tx.push_cellscript_scheduler_witness(scheduler_witness.clone()).unwrap();
        assert_eq!(tx.witnesses.last().map(Vec::as_slice), Some(scheduler_witness.as_slice()));
        let decoded = tx.decoded_cellscript_scheduler_witnesses().collect::<Result<Vec<_>, _>>().unwrap();
        assert_eq!(decoded, vec![witness.clone()]);
        decoded[0].validate_access_set(&[expected_access]).unwrap();
        let admitted = tx.admitted_cellscript_scheduler_witnesses().collect::<Result<Vec<_>, _>>().unwrap();
        assert_eq!(admitted, vec![witness]);
    }

    #[test]
    fn test_cellscript_scheduler_witness_molecule_decode_rejects_malformed_counts() {
        let bytes = encode_cellscript_scheduler_witness_molecule(&CellScriptSchedulerWitness {
            magic: 0xCE11,
            version: CELLSCRIPT_SCHEDULER_WITNESS_VERSION,
            effect_class: CELLSCRIPT_SCHEDULER_EFFECT_READ_ONLY,
            parallelizable: true,
            estimated_cycles: 32,
            access_count: 1, // mismatch: claims 1 but has 0
            accesses: vec![],
        });

        assert!(matches!(decode_cellscript_scheduler_witness(&bytes), Err(CellScriptSchedulerWitnessError::CountMismatch { .. })));
        assert!(!is_cellscript_scheduler_witness_bytes(&bytes));
    }

    #[test]
    fn test_cellscript_scheduler_witness_placement_rejects_duplicate_marker() {
        let lock = Script::new([0x00; 32], 0, vec![]);
        let outputs = vec![CellOutput { lock, type_: None, capacity: 1000 }];
        let outputs_data = vec![vec![]];
        let mut tx = CellTx::new(vec![], vec![], outputs, outputs_data, vec![]).unwrap();
        let scheduler_witness = encode_cellscript_scheduler_witness_molecule(&CellScriptSchedulerWitness {
            magic: 0xCE11,
            version: CELLSCRIPT_SCHEDULER_WITNESS_VERSION,
            effect_class: CELLSCRIPT_SCHEDULER_EFFECT_CREATING,
            parallelizable: false,

            estimated_cycles: 64,
            access_count: 1,
            accesses: vec![CellScriptSchedulerAccessWitness {
                operation: CELLSCRIPT_SCHEDULER_OP_CREATE,
                source: CELLSCRIPT_SCHEDULER_SOURCE_OUTPUT,
                index: 0,
                conflict_hash: [0x24; 32],
                typed_data_hash: [0x00; 32],
            }],
        });

        tx.push_cellscript_scheduler_witness(scheduler_witness.clone()).unwrap();
        assert_eq!(tx.push_cellscript_scheduler_witness(scheduler_witness), Err("duplicate CellScript scheduler witness"));
        assert_eq!(tx.cellscript_scheduler_witnesses().count(), 1);
    }

    #[test]
    fn test_cellscript_compiled_scheduler_witness_produces_trusted_accesses() {
        let lock = Script::new([0x00; 32], 0, vec![]);
        let outputs = vec![CellOutput { lock, type_: None, capacity: 1000 }];
        let outputs_data = vec![vec![]];
        let mut tx = CellTx::new(vec![], vec![], outputs, outputs_data, vec![]).unwrap();
        let expected_access = CellScriptSchedulerAccessWitness {
            operation: CELLSCRIPT_SCHEDULER_OP_CREATE,
            source: CELLSCRIPT_SCHEDULER_SOURCE_OUTPUT,
            index: 0,
            conflict_hash: [0x24; 32],
            typed_data_hash: [0x00; 32],
        };
        let compiled_scheduler_witness = encode_cellscript_scheduler_witness_molecule(&CellScriptSchedulerWitness {
            magic: 0xCE11,
            version: CELLSCRIPT_SCHEDULER_WITNESS_VERSION,
            effect_class: CELLSCRIPT_SCHEDULER_EFFECT_CREATING,
            parallelizable: false,

            estimated_cycles: 64,
            access_count: 1,
            accesses: vec![expected_access.clone()],
        });

        let trusted_summary = tx.push_cellscript_compiled_scheduler_witness(compiled_scheduler_witness.clone()).unwrap();

        assert_eq!(trusted_summary.accesses, vec![expected_access]);
        assert_eq!(tx.witnesses.last().map(Vec::as_slice), Some(compiled_scheduler_witness.as_slice()));
        let admitted = tx.admitted_cellscript_scheduler_witnesses().collect::<Result<Vec<_>, _>>().unwrap();
        assert_eq!(admitted.len(), 1);
        assert_eq!(admitted[0], trusted_summary);
    }

    #[test]
    fn test_cellscript_compiled_scheduler_witness_rejects_unmatched_transaction_shape() {
        let mut tx = CellTx::new(vec![], vec![], vec![], vec![], vec![]).unwrap();
        let compiled_scheduler_witness = encode_cellscript_scheduler_witness_molecule(&CellScriptSchedulerWitness {
            magic: 0xCE11,
            version: CELLSCRIPT_SCHEDULER_WITNESS_VERSION,
            effect_class: CELLSCRIPT_SCHEDULER_EFFECT_CREATING,
            parallelizable: false,

            estimated_cycles: 64,
            access_count: 1,
            accesses: vec![CellScriptSchedulerAccessWitness {
                operation: CELLSCRIPT_SCHEDULER_OP_CREATE,
                source: CELLSCRIPT_SCHEDULER_SOURCE_OUTPUT,
                index: 0,
                conflict_hash: [0x24; 32],
                typed_data_hash: [0x00; 32],
            }],
        });

        let error = tx.push_cellscript_compiled_scheduler_witness(compiled_scheduler_witness).unwrap_err();

        assert_eq!(
            error,
            CellScriptSchedulerWitnessError::SourceIndexOutOfBounds {
                source_id: CELLSCRIPT_SCHEDULER_SOURCE_OUTPUT,
                index: 0,
                available: 0,
            }
        );
        assert!(tx.witnesses.is_empty());
    }

    #[test]
    fn test_cellscript_compiled_scheduler_witness_rejects_duplicate_without_append() {
        let lock = Script::new([0x00; 32], 0, vec![]);
        let outputs = vec![CellOutput { lock, type_: None, capacity: 1000 }];
        let outputs_data = vec![vec![]];
        let mut tx = CellTx::new(vec![], vec![], outputs, outputs_data, vec![]).unwrap();
        let compiled_scheduler_witness = encode_cellscript_scheduler_witness_molecule(&CellScriptSchedulerWitness {
            magic: 0xCE11,
            version: CELLSCRIPT_SCHEDULER_WITNESS_VERSION,
            effect_class: CELLSCRIPT_SCHEDULER_EFFECT_CREATING,
            parallelizable: false,

            estimated_cycles: 64,
            access_count: 1,
            accesses: vec![CellScriptSchedulerAccessWitness {
                operation: CELLSCRIPT_SCHEDULER_OP_CREATE,
                source: CELLSCRIPT_SCHEDULER_SOURCE_OUTPUT,
                index: 0,
                conflict_hash: [0x24; 32],
                typed_data_hash: [0x00; 32],
            }],
        });

        tx.push_cellscript_compiled_scheduler_witness(compiled_scheduler_witness.clone()).unwrap();
        let error = tx.push_cellscript_compiled_scheduler_witness(compiled_scheduler_witness).unwrap_err();

        assert_eq!(error, CellScriptSchedulerWitnessError::DuplicateSchedulerWitness { count: 2 });
        assert_eq!(tx.cellscript_scheduler_witnesses().count(), 1);
    }

    #[test]
    fn test_cellscript_scheduler_witness_decode_rejects_malformed_counts() {
        let bytes = encode_cellscript_scheduler_witness_molecule(&CellScriptSchedulerWitness {
            magic: 0xCE11,
            version: CELLSCRIPT_SCHEDULER_WITNESS_VERSION,
            effect_class: CELLSCRIPT_SCHEDULER_EFFECT_READ_ONLY,
            parallelizable: true,
            estimated_cycles: 32,
            access_count: 2, // mismatch: claims 2 but has 0
            accesses: vec![],
        });

        assert!(matches!(decode_cellscript_scheduler_witness(&bytes), Err(CellScriptSchedulerWitnessError::CountMismatch { .. })));
        assert!(decode_cellscript_scheduler_witness(&[0x11, 0xCE]).is_err());
    }

    #[test]
    fn test_cellscript_scheduler_witness_decode_rejects_invalid_access_envelope() {
        let bytes = encode_cellscript_scheduler_witness_molecule(&CellScriptSchedulerWitness {
            magic: 0xCE11,
            version: CELLSCRIPT_SCHEDULER_WITNESS_VERSION,
            effect_class: CELLSCRIPT_SCHEDULER_EFFECT_READ_ONLY,
            parallelizable: true,

            estimated_cycles: 32,
            access_count: 1,
            accesses: vec![CellScriptSchedulerAccessWitness {
                operation: CELLSCRIPT_SCHEDULER_OP_READ_REF,
                source: CELLSCRIPT_SCHEDULER_SOURCE_OUTPUT,
                index: 0,
                conflict_hash: [0x24; 32],
                typed_data_hash: [0x00; 32],
            }],
        });

        assert_eq!(
            decode_cellscript_scheduler_witness(&bytes),
            Err(CellScriptSchedulerWitnessError::UnexpectedSourceForOperation {
                operation: CELLSCRIPT_SCHEDULER_OP_READ_REF,
                source_id: CELLSCRIPT_SCHEDULER_SOURCE_OUTPUT
            })
        );
    }

    #[test]
    fn test_cellscript_scheduler_witness_admission_rejects_out_of_bounds_access() {
        let tx = CellTx::new(vec![], vec![], vec![], vec![], vec![]).unwrap();
        let witness = CellScriptSchedulerWitness {
            magic: 0xCE11,
            version: CELLSCRIPT_SCHEDULER_WITNESS_VERSION,
            effect_class: CELLSCRIPT_SCHEDULER_EFFECT_CREATING,
            parallelizable: false,

            estimated_cycles: 64,
            access_count: 1,
            accesses: vec![CellScriptSchedulerAccessWitness {
                operation: CELLSCRIPT_SCHEDULER_OP_CREATE,
                source: CELLSCRIPT_SCHEDULER_SOURCE_OUTPUT,
                index: 0,
                conflict_hash: [0x24; 32],
                typed_data_hash: [0x00; 32],
            }],
        };
        let bytes = encode_cellscript_scheduler_witness_molecule(&witness);

        assert_eq!(
            witness.validate_against_transaction(&tx),
            Err(CellScriptSchedulerWitnessError::SourceIndexOutOfBounds {
                source_id: CELLSCRIPT_SCHEDULER_SOURCE_OUTPUT,
                index: 0,
                available: 0
            })
        );
        assert_eq!(
            decode_cellscript_scheduler_witness_for_tx(&tx, &bytes),
            Err(CellScriptSchedulerWitnessError::SourceIndexOutOfBounds {
                source_id: CELLSCRIPT_SCHEDULER_SOURCE_OUTPUT,
                index: 0,
                available: 0
            })
        );
    }

    #[test]
    fn test_cellscript_scheduler_witness_access_set_rejects_binding_mismatch() {
        let witness = CellScriptSchedulerWitness {
            magic: 0xCE11,
            version: CELLSCRIPT_SCHEDULER_WITNESS_VERSION,
            effect_class: CELLSCRIPT_SCHEDULER_EFFECT_CREATING,
            parallelizable: false,

            estimated_cycles: 64,
            access_count: 1,
            accesses: vec![CellScriptSchedulerAccessWitness {
                operation: CELLSCRIPT_SCHEDULER_OP_CREATE,
                source: CELLSCRIPT_SCHEDULER_SOURCE_OUTPUT,
                index: 0,
                conflict_hash: [0x24; 32],
                typed_data_hash: [0x00; 32],
            }],
        };
        let expected = [CellScriptSchedulerAccessWitness {
            operation: CELLSCRIPT_SCHEDULER_OP_CREATE,
            source: CELLSCRIPT_SCHEDULER_SOURCE_OUTPUT,
            index: 0,
            conflict_hash: [0x25; 32],
            typed_data_hash: [0x00; 32],
        }];

        assert_eq!(
            witness.validate_access_set(&expected),
            Err(CellScriptSchedulerWitnessError::AccessSetMismatch {
                operation: CELLSCRIPT_SCHEDULER_OP_CREATE,
                source_id: CELLSCRIPT_SCHEDULER_SOURCE_OUTPUT,
                index: 0,
                conflict_hash: [0x25; 32],
                expected_count: 1,
                actual_count: 0
            })
        );
    }

    #[test]
    fn test_cellscript_scheduler_witness_access_set_is_order_insensitive() {
        let create_access = CellScriptSchedulerAccessWitness {
            operation: CELLSCRIPT_SCHEDULER_OP_CREATE,
            source: CELLSCRIPT_SCHEDULER_SOURCE_OUTPUT,
            index: 0,
            conflict_hash: [0x24; 32],
            typed_data_hash: [0x00; 32],
        };
        let read_access = CellScriptSchedulerAccessWitness {
            operation: CELLSCRIPT_SCHEDULER_OP_READ_REF,
            source: CELLSCRIPT_SCHEDULER_SOURCE_CELL_DEP,
            index: 1,
            conflict_hash: [0x42; 32],
            typed_data_hash: [0x00; 32],
        };
        let witness = CellScriptSchedulerWitness {
            magic: 0xCE11,
            version: CELLSCRIPT_SCHEDULER_WITNESS_VERSION,
            effect_class: CELLSCRIPT_SCHEDULER_EFFECT_CREATING,
            parallelizable: false,

            estimated_cycles: 64,
            access_count: 2,
            accesses: vec![create_access.clone(), read_access.clone()],
        };

        witness.validate_access_set(&[read_access, create_access]).unwrap();
    }

    #[test]
    fn test_cellscript_scheduler_witness_access_set_rejects_unexpected_duplicate() {
        let access = CellScriptSchedulerAccessWitness {
            operation: CELLSCRIPT_SCHEDULER_OP_CREATE,
            source: CELLSCRIPT_SCHEDULER_SOURCE_OUTPUT,
            index: 0,
            conflict_hash: [0x24; 32],
            typed_data_hash: [0x00; 32],
        };
        let witness = CellScriptSchedulerWitness {
            magic: 0xCE11,
            version: CELLSCRIPT_SCHEDULER_WITNESS_VERSION,
            effect_class: CELLSCRIPT_SCHEDULER_EFFECT_CREATING,
            parallelizable: false,

            estimated_cycles: 64,
            access_count: 2,
            accesses: vec![access.clone(), access.clone()],
        };

        assert_eq!(
            witness.validate_access_set(&[access]),
            Err(CellScriptSchedulerWitnessError::AccessSetMismatch {
                operation: CELLSCRIPT_SCHEDULER_OP_CREATE,
                source_id: CELLSCRIPT_SCHEDULER_SOURCE_OUTPUT,
                index: 0,
                conflict_hash: [0x24; 32],
                expected_count: 1,
                actual_count: 2
            })
        );
    }

    #[test]
    fn test_cellscript_scheduler_witness_access_set_rejects_missing_duplicate() {
        let access = CellScriptSchedulerAccessWitness {
            operation: CELLSCRIPT_SCHEDULER_OP_CREATE,
            source: CELLSCRIPT_SCHEDULER_SOURCE_OUTPUT,
            index: 0,
            conflict_hash: [0x24; 32],
            typed_data_hash: [0x00; 32],
        };
        let witness = CellScriptSchedulerWitness {
            magic: 0xCE11,
            version: CELLSCRIPT_SCHEDULER_WITNESS_VERSION,
            effect_class: CELLSCRIPT_SCHEDULER_EFFECT_CREATING,
            parallelizable: false,

            estimated_cycles: 64,
            access_count: 1,
            accesses: vec![access.clone()],
        };

        assert_eq!(
            witness.validate_access_set(&[access.clone(), access]),
            Err(CellScriptSchedulerWitnessError::AccessSetMismatch {
                operation: CELLSCRIPT_SCHEDULER_OP_CREATE,
                source_id: CELLSCRIPT_SCHEDULER_SOURCE_OUTPUT,
                index: 0,
                conflict_hash: [0x24; 32],
                expected_count: 2,
                actual_count: 1
            })
        );
    }

    #[test]
    fn test_cellscript_scheduler_witness_summary_rejects_conflict_hash_tampering() {
        let access = CellScriptSchedulerAccessWitness {
            operation: CELLSCRIPT_SCHEDULER_OP_CREATE,
            source: CELLSCRIPT_SCHEDULER_SOURCE_OUTPUT,
            index: 0,
            conflict_hash: [0x24; 32],
            typed_data_hash: [0x00; 32],
        };
        let expected = CellScriptSchedulerWitness {
            magic: 0xCE11,
            version: CELLSCRIPT_SCHEDULER_WITNESS_VERSION,
            effect_class: CELLSCRIPT_SCHEDULER_EFFECT_CREATING,
            parallelizable: false,
            estimated_cycles: 64,
            access_count: 1,
            accesses: vec![access.clone()],
        };
        // Tamper the conflict_hash in the actual witness
        let tampered_access = CellScriptSchedulerAccessWitness { conflict_hash: [0xFF; 32], ..access };
        let actual = CellScriptSchedulerWitness { accesses: vec![tampered_access], ..expected.clone() };

        assert_eq!(
            actual.validate_summary(&expected),
            Err(CellScriptSchedulerWitnessError::AccessSetMismatch {
                operation: CELLSCRIPT_SCHEDULER_OP_CREATE,
                source_id: CELLSCRIPT_SCHEDULER_SOURCE_OUTPUT,
                index: 0,
                conflict_hash: [0x24; 32],
                expected_count: 1,
                actual_count: 0,
            })
        );
    }

    #[test]
    fn test_cellscript_scheduler_witness_summary_rejects_effect_tampering() {
        let expected = CellScriptSchedulerWitness {
            magic: 0xCE11,
            version: CELLSCRIPT_SCHEDULER_WITNESS_VERSION,
            effect_class: CELLSCRIPT_SCHEDULER_EFFECT_CREATING,
            parallelizable: false,

            estimated_cycles: 64,
            access_count: 0,
            accesses: vec![],
        };
        let actual = CellScriptSchedulerWitness { effect_class: CELLSCRIPT_SCHEDULER_EFFECT_READ_ONLY, ..expected.clone() };

        assert_eq!(
            actual.validate_summary(&expected),
            Err(CellScriptSchedulerWitnessError::TrustedSummaryMismatch { field: "effect_class" })
        );
    }

    fn scheduler_witness_for_accesses(accesses: Vec<CellScriptSchedulerAccessWitness>) -> CellScriptSchedulerWitness {
        scheduler_witness_for_summary(CELLSCRIPT_SCHEDULER_EFFECT_CREATING, false, 64, accesses)
    }

    fn scheduler_witness_for_summary(
        effect_class: u8,
        parallelizable: bool,
        estimated_cycles: u64,
        accesses: Vec<CellScriptSchedulerAccessWitness>,
    ) -> CellScriptSchedulerWitness {
        CellScriptSchedulerWitness {
            magic: 0xCE11,
            version: CELLSCRIPT_SCHEDULER_WITNESS_VERSION,
            effect_class,
            parallelizable,
            estimated_cycles,
            access_count: accesses.len() as u32,
            accesses,
        }
    }

    fn effect_class_strategy() -> impl Strategy<Value = u8> {
        prop_oneof![
            Just(CELLSCRIPT_SCHEDULER_EFFECT_PURE),
            Just(CELLSCRIPT_SCHEDULER_EFFECT_READ_ONLY),
            Just(CELLSCRIPT_SCHEDULER_EFFECT_MUTATING),
            Just(CELLSCRIPT_SCHEDULER_EFFECT_CREATING),
            Just(CELLSCRIPT_SCHEDULER_EFFECT_DESTROYING),
        ]
    }

    fn hash32_strategy() -> impl Strategy<Value = [u8; 32]> {
        proptest::array::uniform32(any::<u8>())
    }

    fn scheduler_access_strategy() -> impl Strategy<Value = CellScriptSchedulerAccessWitness> {
        prop_oneof![
            (0u32..16, hash32_strategy(), hash32_strategy()).prop_map(|(index, conflict_hash, typed_data_hash)| {
                CellScriptSchedulerAccessWitness {
                    operation: CELLSCRIPT_SCHEDULER_OP_CONSUME,
                    source: CELLSCRIPT_SCHEDULER_SOURCE_INPUT,
                    index,
                    conflict_hash,
                    typed_data_hash,
                }
            }),
            (0u32..16, hash32_strategy(), hash32_strategy()).prop_map(|(index, conflict_hash, typed_data_hash)| {
                CellScriptSchedulerAccessWitness {
                    operation: CELLSCRIPT_SCHEDULER_OP_DESTROY,
                    source: CELLSCRIPT_SCHEDULER_SOURCE_INPUT,
                    index,
                    conflict_hash,
                    typed_data_hash,
                }
            }),
            (0u32..16, hash32_strategy(), hash32_strategy()).prop_map(|(index, conflict_hash, typed_data_hash)| {
                CellScriptSchedulerAccessWitness {
                    operation: CELLSCRIPT_SCHEDULER_OP_READ_REF,
                    source: CELLSCRIPT_SCHEDULER_SOURCE_CELL_DEP,
                    index,
                    conflict_hash,
                    typed_data_hash,
                }
            }),
            (0u32..16, hash32_strategy(), hash32_strategy()).prop_map(|(index, conflict_hash, typed_data_hash)| {
                CellScriptSchedulerAccessWitness {
                    operation: CELLSCRIPT_SCHEDULER_OP_CREATE,
                    source: CELLSCRIPT_SCHEDULER_SOURCE_OUTPUT,
                    index,
                    conflict_hash,
                    typed_data_hash,
                }
            }),
            (0u32..16, hash32_strategy(), hash32_strategy()).prop_map(|(index, conflict_hash, typed_data_hash)| {
                CellScriptSchedulerAccessWitness {
                    operation: CELLSCRIPT_SCHEDULER_OP_TRANSFER,
                    source: CELLSCRIPT_SCHEDULER_SOURCE_INPUT,
                    index,
                    conflict_hash,
                    typed_data_hash,
                }
            }),
            (0u32..16, hash32_strategy(), hash32_strategy()).prop_map(|(index, conflict_hash, typed_data_hash)| {
                CellScriptSchedulerAccessWitness {
                    operation: CELLSCRIPT_SCHEDULER_OP_TRANSFER,
                    source: CELLSCRIPT_SCHEDULER_SOURCE_OUTPUT,
                    index,
                    conflict_hash,
                    typed_data_hash,
                }
            }),
        ]
    }

    fn operation_tamper_strategy() -> impl Strategy<Value = (CellScriptSchedulerAccessWitness, u8)> {
        prop_oneof![
            (0u32..16, hash32_strategy(), hash32_strategy()).prop_map(|(index, conflict_hash, typed_data_hash)| {
                (
                    CellScriptSchedulerAccessWitness {
                        operation: CELLSCRIPT_SCHEDULER_OP_CONSUME,
                        source: CELLSCRIPT_SCHEDULER_SOURCE_INPUT,
                        index,
                        conflict_hash,
                        typed_data_hash,
                    },
                    CELLSCRIPT_SCHEDULER_OP_DESTROY,
                )
            }),
            (0u32..16, hash32_strategy(), hash32_strategy()).prop_map(|(index, conflict_hash, typed_data_hash)| {
                (
                    CellScriptSchedulerAccessWitness {
                        operation: CELLSCRIPT_SCHEDULER_OP_CREATE,
                        source: CELLSCRIPT_SCHEDULER_SOURCE_OUTPUT,
                        index,
                        conflict_hash,
                        typed_data_hash,
                    },
                    CELLSCRIPT_SCHEDULER_OP_TRANSFER,
                )
            }),
        ]
    }

    fn source_tamper_strategy() -> impl Strategy<Value = (CellScriptSchedulerAccessWitness, u8)> {
        prop_oneof![(0u32..16, hash32_strategy(), hash32_strategy()).prop_map(|(index, conflict_hash, typed_data_hash)| {
            (
                CellScriptSchedulerAccessWitness {
                    operation: CELLSCRIPT_SCHEDULER_OP_TRANSFER,
                    source: CELLSCRIPT_SCHEDULER_SOURCE_INPUT,
                    index,
                    conflict_hash,
                    typed_data_hash,
                },
                CELLSCRIPT_SCHEDULER_SOURCE_OUTPUT,
            )
        }),]
    }

    fn assert_access_set_mismatch(error: CellScriptSchedulerWitnessError) {
        assert!(matches!(error, CellScriptSchedulerWitnessError::AccessSetMismatch { .. }), "{error:?}");
    }

    proptest! {
        #[test]
        fn prop_cellscript_scheduler_access_set_accepts_reordered_multiset(
            accesses in prop::collection::vec(scheduler_access_strategy(), 0..24)
        ) {
            let witness = scheduler_witness_for_accesses(accesses.clone());
            let mut expected = accesses;
            expected.reverse();

            prop_assert!(witness.validate_access_set(&expected).is_ok());
        }

        #[test]
        fn prop_cellscript_scheduler_access_set_rejects_missing_access(
            accesses in prop::collection::vec(scheduler_access_strategy(), 1..24)
        ) {
            let mut actual = accesses.clone();
            actual.pop();
            let witness = scheduler_witness_for_accesses(actual);

            assert_access_set_mismatch(witness.validate_access_set(&accesses).unwrap_err());
        }

        #[test]
        fn prop_cellscript_scheduler_access_set_rejects_unexpected_duplicate(
            accesses in prop::collection::vec(scheduler_access_strategy(), 1..24)
        ) {
            let mut actual = accesses.clone();
            actual.push(accesses[0].clone());
            let witness = scheduler_witness_for_accesses(actual);

            assert_access_set_mismatch(witness.validate_access_set(&accesses).unwrap_err());
        }

        #[test]
        fn prop_cellscript_scheduler_access_set_rejects_conflict_hash_tamper(
            accesses in prop::collection::vec(scheduler_access_strategy(), 1..24)
        ) {
            let mut actual = accesses.clone();
            actual[0].conflict_hash[0] ^= 0x80;
            let witness = scheduler_witness_for_accesses(actual);

            assert_access_set_mismatch(witness.validate_access_set(&accesses).unwrap_err());
        }

        #[test]
        fn prop_cellscript_scheduler_access_set_rejects_index_tamper(
            accesses in prop::collection::vec(scheduler_access_strategy(), 1..24)
        ) {
            let mut actual = accesses.clone();
            actual[0].index += 1;
            let witness = scheduler_witness_for_accesses(actual);

            assert_access_set_mismatch(witness.validate_access_set(&accesses).unwrap_err());
        }

        #[test]
        fn prop_cellscript_scheduler_access_set_rejects_operation_tamper(
            (access, replacement_operation) in operation_tamper_strategy()
        ) {
            let mut actual = access.clone();
            actual.operation = replacement_operation;
            let witness = scheduler_witness_for_accesses(vec![actual]);

            assert_access_set_mismatch(witness.validate_access_set(&[access]).unwrap_err());
        }

        #[test]
        fn prop_cellscript_scheduler_access_set_rejects_source_tamper(
            (access, replacement_source) in source_tamper_strategy()
        ) {
            let mut actual = access.clone();
            actual.source = replacement_source;
            let witness = scheduler_witness_for_accesses(vec![actual]);

            assert_access_set_mismatch(witness.validate_access_set(&[access]).unwrap_err());
        }

        #[test]
        fn prop_cellscript_scheduler_summary_accepts_reordered_access_and_touch_multisets(
            effect_class in effect_class_strategy(),
            parallelizable in any::<bool>(),
            estimated_cycles in any::<u64>(),
            accesses in prop::collection::vec(scheduler_access_strategy(), 0..24)
        ) {
            let expected = scheduler_witness_for_summary(
                effect_class,
                parallelizable,
                estimated_cycles,
                accesses.clone(),
            );
            let actual = scheduler_witness_for_summary(
                effect_class,
                parallelizable,
                estimated_cycles,
                accesses.into_iter().rev().collect(),
            );

            prop_assert!(actual.validate_summary(&expected).is_ok());
        }

        #[test]
        fn prop_cellscript_scheduler_summary_rejects_access_multiplicity_tamper(
            access in scheduler_access_strategy(),
            accesses in prop::collection::vec(scheduler_access_strategy(), 0..24)
        ) {
            let expected = scheduler_witness_for_summary(
                CELLSCRIPT_SCHEDULER_EFFECT_CREATING,
                false,
                64,
                std::iter::once(access.clone()).chain(accesses.clone()).collect(),
            );
            let actual = scheduler_witness_for_summary(
                CELLSCRIPT_SCHEDULER_EFFECT_CREATING,
                false,
                64,
                std::iter::once(access.clone()).chain(std::iter::once(access)).chain(accesses).collect(),
            );

            prop_assert!(actual.validate_summary(&expected).is_err());
        }

        #[test]
        fn prop_cellscript_scheduler_summary_rejects_parallelizable_and_cycle_tamper(
            parallelizable in any::<bool>(),
            estimated_cycles in any::<u64>(),
            accesses in prop::collection::vec(scheduler_access_strategy(), 0..24)
        ) {
            let expected = scheduler_witness_for_summary(
                CELLSCRIPT_SCHEDULER_EFFECT_CREATING,
                parallelizable,
                estimated_cycles,
                accesses.clone(),
            );
            let parallelizable_tamper = scheduler_witness_for_summary(
                CELLSCRIPT_SCHEDULER_EFFECT_CREATING,
                !parallelizable,
                estimated_cycles,
                accesses.clone(),
            );
            let cycle_tamper = scheduler_witness_for_summary(
                CELLSCRIPT_SCHEDULER_EFFECT_CREATING,
                parallelizable,
                estimated_cycles.wrapping_add(1),
                accesses,
            );

            prop_assert_eq!(
                parallelizable_tamper.validate_summary(&expected),
                Err(CellScriptSchedulerWitnessError::TrustedSummaryMismatch { field: "parallelizable" })
            );
            prop_assert_eq!(
                cycle_tamper.validate_summary(&expected),
                Err(CellScriptSchedulerWitnessError::TrustedSummaryMismatch { field: "estimated_cycles" })
            );
        }
    }

    #[test]
    fn test_celltx_compute_and_storage_mass_are_distinct() {
        let inputs = vec![CellInput::new(OutPoint::new([0; 32], 0), 0)];
        let deps = vec![];
        let lock = Script::new([0x10; 32], 1, vec![1; 20]);
        let outputs = vec![CellOutput { lock, type_: None, capacity: 10_000 }];
        let outputs_data = vec![vec![7; 128]];
        let witnesses = vec![vec![0; 65]];

        let tx = CellTx::new(inputs, deps, outputs, outputs_data, witnesses).unwrap();
        assert!(tx.estimated_compute_mass() > tx.serialized_size() as u64);
        assert!(tx.estimated_compute_mass() > 0);
        assert!(tx.estimated_transient_mass() > 0);
        assert!(tx.estimated_storage_mass() > 0);
        assert_eq!(tx.estimated_transient_mass(), (tx.serialized_size() as u64) * TRANSIENT_BYTE_TO_MASS_FACTOR);
        assert_ne!(tx.estimated_compute_mass(), tx.estimated_storage_mass());
    }

    #[test]
    fn test_celltx_compute_mass_matches_non_contextual_formula() {
        let inputs = vec![CellInput::new(OutPoint::new([0x01; 32], 0), 0), CellInput::new(OutPoint::new([0x02; 32], 1), 0)];
        let deps = vec![];
        let outputs = vec![
            CellOutput { lock: Script::new([0x10; 32], 1, vec![1; 20]), type_: None, capacity: 10_000 },
            CellOutput {
                lock: Script::new([0x20; 32], 1, vec![2; 32]),
                type_: Some(Script::new([0x30; 32], 1, vec![3; 12])),
                capacity: 20_000,
            },
        ];
        let outputs_data = vec![vec![0xAA; 16], vec![0xBB; 8]];
        let witnesses = vec![vec![0xCC; 65], vec![0xDD; 32]];

        let tx = CellTx::new(inputs, deps, outputs, outputs_data, witnesses).unwrap();
        let serialized_size = tx.serialized_size() as u64;
        let total_output_script_bytes = (32 + 1 + 20) + (32 + 1 + 32) + (32 + 1 + 12);
        let expected =
            serialized_size * MASS_PER_TX_BYTE + total_output_script_bytes as u64 * MASS_PER_SCRIPT_PUB_KEY_BYTE + 2 * MASS_PER_SIG_OP;

        assert_eq!(tx.estimated_compute_mass(), expected);
    }

    #[test]
    fn test_dep_group_roundtrip() {
        let ops = vec![OutPoint::new([0x11; 32], 0), OutPoint::new([0x22; 32], 7), OutPoint::new([0x33; 32], u32::MAX)];
        let data = encode_dep_group_data(&ops);
        let parsed = parse_dep_group_data(&data).unwrap();
        assert_eq!(parsed, ops);
    }

    #[test]
    fn test_dep_group_empty() {
        let data = encode_dep_group_data(&[]);
        assert_eq!(data, [0, 0, 0, 0]);
        let parsed = parse_dep_group_data(&data).unwrap();
        assert!(parsed.is_empty());
        assert!(encode_ckb_dep_group_data(&[]).is_err());
        assert!(parse_ckb_dep_group_data(&data).is_err());
    }

    #[test]
    fn test_dep_group_invalid_data() {
        assert!(parse_dep_group_data(&[]).is_err());
        assert!(parse_dep_group_data(&[1, 0, 0, 0]).is_err()); // count=1 but no data
        assert!(parse_dep_group_data(&[1, 0, 0, 0, 0]).is_err()); // count=1 but only 1 byte
    }

    #[test]
    fn test_ckb_dep_group_abi_matches_nonempty_myelin_bytes() {
        let ops = vec![OutPoint::new([0x44; 32], 1), OutPoint::new([0x55; 32], 2)];
        let myelin_data = encode_dep_group_data_for_abi(&ops, DepGroupDataAbi::Myelin).unwrap();
        let ckb_data = encode_dep_group_data_for_abi(&ops, DepGroupDataAbi::CkbMolecule).unwrap();

        assert_eq!(ckb_data, myelin_data);
        assert_eq!(parse_dep_group_data_for_abi(&ckb_data, DepGroupDataAbi::CkbMolecule).unwrap(), ops);
        assert_eq!(parse_ckb_dep_group_data(&ckb_data).unwrap(), ops);
    }

    // ─── Typed Cell Tests ──────────────────────────────────────────────────────

    fn test_script(code: u8, hash_type: u8, args_len: usize) -> Script {
        Script::new([code; 32], hash_type, vec![code; args_len])
    }

    #[test]
    fn test_compute_conflict_hash_determinism() {
        let script = test_script(0xAA, 1, 4);
        let conflict_key = b"pool_id=A";
        let h1 = compute_conflict_hash(&script, conflict_key);
        let h2 = compute_conflict_hash(&script, conflict_key);
        assert_eq!(h1, h2, "conflict_hash must be deterministic for same inputs");
    }

    #[test]
    fn test_compute_typed_data_hash_determinism() {
        let script = test_script(0xBB, 1, 4);
        let data = b"reserve_a=100;reserve_b=200";
        let h1 = compute_typed_data_hash(&script, data);
        let h2 = compute_typed_data_hash(&script, data);
        assert_eq!(h1, h2, "typed_data_hash must be deterministic for same inputs");
    }

    #[test]
    fn test_conflict_hash_stable_across_data_updates() {
        // conflict_hash does NOT change when data changes
        let script = test_script(0xAA, 1, 4);
        let conflict_key = b"pool_id=A";
        let data_v1 = b"reserve_a=100";
        let data_v2 = b"reserve_a=200";

        let ch = compute_conflict_hash(&script, conflict_key);
        let tdh1 = compute_typed_data_hash(&script, data_v1);
        let tdh2 = compute_typed_data_hash(&script, data_v2);

        assert_eq!(ch, compute_conflict_hash(&script, conflict_key), "conflict_hash is stable");
        assert_ne!(tdh1, tdh2, "typed_data_hash changes with data");
        assert_ne!(ch, tdh1, "conflict_hash and typed_data_hash are different concepts");
    }

    #[test]
    fn test_conflict_hash_differs_for_different_conflict_key_values() {
        let script = test_script(0xAA, 1, 4);
        let key_a = b"pool_id=A";
        let key_b = b"pool_id=B";

        let ch_a = compute_conflict_hash(&script, key_a);
        let ch_b = compute_conflict_hash(&script, key_b);

        assert_ne!(ch_a, ch_b, "different conflict_key_values must produce different conflict_hashes");
    }

    #[test]
    fn test_typed_data_hash_differs_for_different_data() {
        let script = test_script(0xAA, 1, 4);
        let data1 = b"state=1";
        let data2 = b"state=2";

        let tdh1 = compute_typed_data_hash(&script, data1);
        let tdh2 = compute_typed_data_hash(&script, data2);

        assert_ne!(tdh1, tdh2, "different data must produce different typed_data_hashes");
    }

    #[test]
    fn test_conflict_hash_differs_for_different_scripts() {
        let script_a = test_script(0xAA, 1, 4);
        let script_b = test_script(0xBB, 1, 4);
        let conflict_key = b"pool_id=A";

        let ch_a = compute_conflict_hash(&script_a, conflict_key);
        let ch_b = compute_conflict_hash(&script_b, conflict_key);

        assert_ne!(ch_a, ch_b, "different scripts must produce different conflict_hashes");
    }

    #[test]
    fn test_encode_conflict_key_value_composite_canonical() {
        // Canonical length-delimited encoding: len(field1_le_u32) || field1 || len(field2_le_u32) || field2
        let fields: Vec<&[u8]> = vec![b"ab", b"c"];
        let encoded = encode_conflict_key_value_composite(&fields);

        // field1 "ab" -> len=2 (LE u32) + "ab"
        // field2 "c"  -> len=1 (LE u32) + "c"
        let mut expected = Vec::new();
        expected.extend_from_slice(&2u32.to_le_bytes());
        expected.extend_from_slice(b"ab");
        expected.extend_from_slice(&1u32.to_le_bytes());
        expected.extend_from_slice(b"c");

        assert_eq!(encoded, expected);
    }

    #[test]
    fn test_encode_conflict_key_value_composite_disambiguates() {
        // ["ab", "c"] must produce a different encoding than ["a", "bc"]
        let enc1 = encode_conflict_key_value_composite(&[b"ab", b"c"]);
        let enc2 = encode_conflict_key_value_composite(&[b"a", b"bc"]);

        assert_ne!(enc1, enc2, "canonical encoding must disambiguate raw-concat collisions");
    }

    #[test]
    fn test_encode_conflict_key_value_composite_empty() {
        let encoded = encode_conflict_key_value_composite(&[]);
        assert!(encoded.is_empty(), "empty fields produce empty encoding");
    }

    #[test]
    fn test_validate_typed_cell_decl_rejects_mutable_with_none() {
        let decl = TypedCellDecl {
            runtime: RuntimeCellSemantics { ownership: CellOwnership::Owned, conflict_key: ConflictKeySpec::None },
            semantic: TypedCellSemanticMetadata {
                mutability: CellMutability::Linear,
                accounting: vec![CellAccounting::Fungible],
                identity: CellIdentity::OutPoint,
                settlement: CellSettlement::Local,
            },
        };
        assert_eq!(
            validate_typed_cell_decl(&decl),
            Err(TypedCellDeclError::MutableCellWithNoneConflictKey),
            "mutable Owned cell with ConflictKeySpec::None must be rejected"
        );
    }

    #[test]
    fn test_validate_typed_cell_decl_rejects_shared_mutable_with_none() {
        let decl = TypedCellDecl {
            runtime: RuntimeCellSemantics { ownership: CellOwnership::Shared, conflict_key: ConflictKeySpec::None },
            semantic: TypedCellSemanticMetadata {
                mutability: CellMutability::Versioned,
                accounting: vec![CellAccounting::NonFungible],
                identity: CellIdentity::Singleton,
                settlement: CellSettlement::Pending,
            },
        };
        assert_eq!(
            validate_typed_cell_decl(&decl),
            Err(TypedCellDeclError::MutableCellWithNoneConflictKey),
            "mutable Shared cell with ConflictKeySpec::None must be rejected"
        );
    }

    #[test]
    fn test_validate_typed_cell_decl_accepts_immutable_with_none() {
        let decl = TypedCellDecl {
            runtime: RuntimeCellSemantics { ownership: CellOwnership::Immutable, conflict_key: ConflictKeySpec::None },
            semantic: TypedCellSemanticMetadata {
                mutability: CellMutability::Linear,
                accounting: vec![CellAccounting::Receipt],
                identity: CellIdentity::TypeId,
                settlement: CellSettlement::Local,
            },
        };
        assert!(validate_typed_cell_decl(&decl).is_ok(), "Immutable cell with ConflictKeySpec::None is valid");
    }

    #[test]
    fn test_validate_typed_cell_decl_accepts_ephemeral_with_none() {
        let decl = TypedCellDecl {
            runtime: RuntimeCellSemantics { ownership: CellOwnership::Ephemeral, conflict_key: ConflictKeySpec::None },
            semantic: TypedCellSemanticMetadata {
                mutability: CellMutability::Linear,
                accounting: vec![],
                identity: CellIdentity::OutPoint,
                settlement: CellSettlement::Local,
            },
        };
        assert!(validate_typed_cell_decl(&decl).is_ok(), "Ephemeral cell with ConflictKeySpec::None and Local settlement is valid");
    }

    #[test]
    fn test_validate_typed_cell_decl_accepts_owned_with_cell_id() {
        let decl = TypedCellDecl {
            runtime: RuntimeCellSemantics { ownership: CellOwnership::Owned, conflict_key: ConflictKeySpec::CellId },
            semantic: TypedCellSemanticMetadata {
                mutability: CellMutability::Linear,
                accounting: vec![CellAccounting::Fungible],
                identity: CellIdentity::OutPoint,
                settlement: CellSettlement::Local,
            },
        };
        assert!(validate_typed_cell_decl(&decl).is_ok());
    }

    #[test]
    fn test_validate_rejects_immutable_with_versioned() {
        let decl = TypedCellDecl {
            runtime: RuntimeCellSemantics { ownership: CellOwnership::Immutable, conflict_key: ConflictKeySpec::None },
            semantic: TypedCellSemanticMetadata {
                mutability: CellMutability::Versioned,
                accounting: vec![],
                identity: CellIdentity::Singleton,
                settlement: CellSettlement::Local,
            },
        };
        assert_eq!(
            validate_typed_cell_decl(&decl),
            Err(TypedCellDeclError::ImmutableWithMutableMutability { mutability: CellMutability::Versioned })
        );
    }

    #[test]
    fn test_validate_rejects_immutable_with_append_only() {
        let decl = TypedCellDecl {
            runtime: RuntimeCellSemantics { ownership: CellOwnership::Immutable, conflict_key: ConflictKeySpec::None },
            semantic: TypedCellSemanticMetadata {
                mutability: CellMutability::AppendOnly,
                accounting: vec![],
                identity: CellIdentity::Singleton,
                settlement: CellSettlement::Local,
            },
        };
        assert_eq!(
            validate_typed_cell_decl(&decl),
            Err(TypedCellDeclError::ImmutableWithMutableMutability { mutability: CellMutability::AppendOnly })
        );
    }

    #[test]
    fn test_validate_rejects_fungible_plus_nonfungible() {
        let decl = TypedCellDecl {
            runtime: RuntimeCellSemantics { ownership: CellOwnership::Owned, conflict_key: ConflictKeySpec::CellId },
            semantic: TypedCellSemanticMetadata {
                mutability: CellMutability::Linear,
                accounting: vec![CellAccounting::Fungible, CellAccounting::NonFungible],
                identity: CellIdentity::OutPoint,
                settlement: CellSettlement::Local,
            },
        };
        assert_eq!(validate_typed_cell_decl(&decl), Err(TypedCellDeclError::ConflictingAccountingLabels));
    }

    #[test]
    fn test_validate_rejects_ephemeral_with_committed_settlement() {
        let decl = TypedCellDecl {
            runtime: RuntimeCellSemantics { ownership: CellOwnership::Ephemeral, conflict_key: ConflictKeySpec::None },
            semantic: TypedCellSemanticMetadata {
                mutability: CellMutability::Linear,
                accounting: vec![],
                identity: CellIdentity::OutPoint,
                settlement: CellSettlement::Committed,
            },
        };
        assert_eq!(validate_typed_cell_decl(&decl), Err(TypedCellDeclError::EphemeralWithNonLocalSettlement));
    }

    #[test]
    fn test_validate_rejects_ephemeral_with_pending_settlement() {
        let decl = TypedCellDecl {
            runtime: RuntimeCellSemantics { ownership: CellOwnership::Ephemeral, conflict_key: ConflictKeySpec::None },
            semantic: TypedCellSemanticMetadata {
                mutability: CellMutability::Linear,
                accounting: vec![],
                identity: CellIdentity::OutPoint,
                settlement: CellSettlement::Pending,
            },
        };
        assert_eq!(validate_typed_cell_decl(&decl), Err(TypedCellDeclError::EphemeralWithNonLocalSettlement));
    }

    #[test]
    fn test_typed_cell_decl_molecule_roundtrip() {
        let decl = TypedCellDecl {
            runtime: RuntimeCellSemantics {
                ownership: CellOwnership::Shared,
                conflict_key: ConflictKeySpec::Composite(vec!["asset_id".to_string(), "owner".to_string()]),
            },
            semantic: TypedCellSemanticMetadata {
                mutability: CellMutability::Versioned,
                accounting: vec![CellAccounting::NonFungible, CellAccounting::Receipt],
                identity: CellIdentity::Field("pool_id".to_string()),
                settlement: CellSettlement::Committed,
            },
        };

        let bytes = encode_typed_cell_decl_molecule(&decl);
        let restored = decode_typed_cell_decl_molecule(&bytes).expect("decode Molecule typed-cell metadata");
        assert_eq!(decl, restored, "TypedCellDecl must round-trip through Molecule metadata bytes");
    }

    #[test]
    fn test_script_id_from_script() {
        let script = test_script(0xAA, 1, 4);
        let id1 = ScriptId::from_script(&script);
        let id2 = ScriptId::from_script(&script);
        assert_eq!(id1, id2, "same script produces same ScriptId");

        let different_script = test_script(0xBB, 1, 4);
        let id3 = ScriptId::from_script(&different_script);
        assert_ne!(id1, id3, "different scripts produce different ScriptIds");
    }

    #[test]
    fn test_script_id_differs_for_different_args() {
        let script_a = Script::new([0xAA; 32], 1, vec![0x01]);
        let script_b = Script::new([0xAA; 32], 1, vec![0x02]);
        let id_a = ScriptId::from_script(&script_a);
        let id_b = ScriptId::from_script(&script_b);
        assert_ne!(id_a, id_b, "same code_hash+hash_type but different args must differ");
    }

    #[test]
    fn test_in_memory_typed_cell_store_roundtrip() {
        let mut store = InMemoryTypedCellStore::new();
        let script = test_script(0xAA, 1, 4);
        let decl = TypedCellDecl {
            runtime: RuntimeCellSemantics {
                ownership: CellOwnership::Shared,
                conflict_key: ConflictKeySpec::Field("pool_id".to_string()),
            },
            semantic: TypedCellSemanticMetadata {
                mutability: CellMutability::Versioned,
                accounting: vec![CellAccounting::NonFungible],
                identity: CellIdentity::Field("pool_id".to_string()),
                settlement: CellSettlement::Pending,
            },
        };

        assert!(store.get_decl(&script).is_none());
        store.insert_decl(script.clone(), decl.clone());
        let retrieved = store.get_decl(&script).expect("should find inserted decl");
        assert_eq!(*retrieved, decl);
    }

    #[test]
    fn test_typed_cell_scheduler_witness_encode_decode_roundtrip() {
        let script = test_script(0xAA, 1, 4);
        let conflict_key = b"pool_id=A";
        let data = b"reserve_a=100;reserve_b=200";

        let conflict_hash = compute_conflict_hash(&script, conflict_key);
        let typed_data_hash = compute_typed_data_hash(&script, data);

        let access = CellScriptSchedulerAccessWitness {
            operation: CELLSCRIPT_SCHEDULER_OP_CONSUME,
            source: CELLSCRIPT_SCHEDULER_SOURCE_INPUT,
            index: 0,
            conflict_hash,
            typed_data_hash,
        };

        let witness = CellScriptSchedulerWitness {
            magic: 0xCE11,
            version: TYPED_CELL_SCHEDULER_WITNESS_VERSION,
            effect_class: CELLSCRIPT_SCHEDULER_EFFECT_MUTATING,
            parallelizable: false,
            estimated_cycles: 500,
            access_count: 1,
            accesses: vec![access],
        };

        let encoded = encode_cellscript_scheduler_witness_molecule(&witness);
        let decoded = decode_cellscript_scheduler_witness_molecule(&encoded).expect("decode should succeed");

        assert_eq!(decoded.magic, 0xCE11);
        assert_eq!(decoded.version, TYPED_CELL_SCHEDULER_WITNESS_VERSION);
        assert_eq!(decoded.accesses.len(), 1);
        assert_eq!(decoded.accesses[0].conflict_hash, conflict_hash);
        assert_eq!(decoded.accesses[0].typed_data_hash, typed_data_hash);
    }

    #[test]
    fn test_typed_cell_witness_conflict_hash_stability_across_data_update() {
        // Simulate a shared pool cell whose data changes between two blocks.
        // conflict_hash remains stable; typed_data_hash changes.
        let script = test_script(0xAA, 1, 4);
        let conflict_key = b"pool_id=A";

        let ch = compute_conflict_hash(&script, conflict_key);
        let tdh_v1 = compute_typed_data_hash(&script, b"reserve_a=100");
        let tdh_v2 = compute_typed_data_hash(&script, b"reserve_a=200");

        // Build two witnesses for the same cell at different data versions
        let access_v1 = CellScriptSchedulerAccessWitness {
            operation: CELLSCRIPT_SCHEDULER_OP_CONSUME,
            source: CELLSCRIPT_SCHEDULER_SOURCE_INPUT,
            index: 0,
            conflict_hash: ch,
            typed_data_hash: tdh_v1,
        };
        let access_v2 = CellScriptSchedulerAccessWitness {
            operation: CELLSCRIPT_SCHEDULER_OP_CONSUME,
            source: CELLSCRIPT_SCHEDULER_SOURCE_INPUT,
            index: 0,
            conflict_hash: ch,
            typed_data_hash: tdh_v2,
        };

        // Same conflict_hash, different typed_data_hash
        assert_eq!(access_v1.conflict_hash, access_v2.conflict_hash);
        assert_ne!(access_v1.typed_data_hash, access_v2.typed_data_hash);
    }

    #[test]
    fn test_validate_summary_rejects_forged_conflict_hash() {
        let script = test_script(0xAA, 1, 4);
        let conflict_key = b"pool_id=A";
        let conflict_hash = compute_conflict_hash(&script, conflict_key);
        let typed_data_hash = compute_typed_data_hash(&script, b"data");

        let trusted = CellScriptSchedulerWitness {
            magic: 0xCE11,
            version: TYPED_CELL_SCHEDULER_WITNESS_VERSION,
            effect_class: CELLSCRIPT_SCHEDULER_EFFECT_MUTATING,
            parallelizable: false,
            estimated_cycles: 500,
            access_count: 1,
            accesses: vec![CellScriptSchedulerAccessWitness {
                operation: CELLSCRIPT_SCHEDULER_OP_CONSUME,
                source: CELLSCRIPT_SCHEDULER_SOURCE_INPUT,
                index: 0,
                conflict_hash,
                typed_data_hash,
            }],
        };

        let mut forged = trusted.clone();
        forged.accesses[0].conflict_hash = [0xFF; 32]; // tamper

        let result = validate_cellscript_scheduler_witness_access_set(&forged, &trusted.accesses);
        assert!(result.is_err(), "forged conflict_hash must be rejected");
    }

    #[test]
    fn test_access_count_exceeds_max_is_rejected() {
        // Construct a witness with access_count exceeding MAX_CELLSCRIPT_ACCESS_COUNT
        let too_many: Vec<CellScriptSchedulerAccessWitness> = (0..=MAX_CELLSCRIPT_ACCESS_COUNT)
            .map(|i| CellScriptSchedulerAccessWitness {
                operation: CELLSCRIPT_SCHEDULER_OP_READ_REF,
                source: CELLSCRIPT_SCHEDULER_SOURCE_CELL_DEP,
                index: i,
                conflict_hash: [0xAA; 32],
                typed_data_hash: [0xBB; 32],
            })
            .collect();
        let witness = CellScriptSchedulerWitness {
            magic: 0xCE11,
            version: CELLSCRIPT_SCHEDULER_WITNESS_VERSION,
            effect_class: CELLSCRIPT_SCHEDULER_EFFECT_READ_ONLY,
            parallelizable: true,
            estimated_cycles: 1000,
            access_count: too_many.len() as u32,
            accesses: too_many,
        };
        let bytes = encode_cellscript_scheduler_witness_molecule(&witness);
        // Decode should reject
        assert!(matches!(
            decode_cellscript_scheduler_witness(&bytes),
            Err(CellScriptSchedulerWitnessError::AccessCountExceedsMax { .. })
        ));
        // Admission guard should also reject
        assert!(!is_cellscript_scheduler_witness_bytes(&bytes));
    }

    #[test]
    fn test_zero_conflict_hash_in_merge_is_rejected() {
        // Zero conflict_hash should be rejected, not silently skipped
        let access = CellScriptSchedulerAccessWitness {
            operation: CELLSCRIPT_SCHEDULER_OP_READ_REF,
            source: CELLSCRIPT_SCHEDULER_SOURCE_CELL_DEP,
            index: 0,
            conflict_hash: [0u8; 32], // zero — illegal in typed-cell mode
            typed_data_hash: [0xBB; 32],
        };
        let result = validate_cellscript_scheduler_access_envelope(&access);
        assert!(result.is_err(), "zero conflict_hash must be rejected in typed-cell mode");
    }

    #[test]
    fn test_shared_cell_must_declare_explicit_conflict_key() {
        // Shared mutable cell without explicit conflict_key (using CellId) is valid
        // but Shared + None is not
        let shared_cellid = TypedCellDecl {
            runtime: RuntimeCellSemantics {
                ownership: CellOwnership::Shared,
                conflict_key: ConflictKeySpec::Field("pool_id".to_string()),
            },
            semantic: TypedCellSemanticMetadata {
                mutability: CellMutability::Versioned,
                accounting: vec![CellAccounting::NonFungible],
                identity: CellIdentity::Singleton,
                settlement: CellSettlement::Pending,
            },
        };
        assert!(validate_typed_cell_decl(&shared_cellid).is_ok());

        let shared_none = TypedCellDecl {
            runtime: RuntimeCellSemantics { ownership: CellOwnership::Shared, conflict_key: ConflictKeySpec::None },
            semantic: TypedCellSemanticMetadata {
                mutability: CellMutability::Versioned,
                accounting: vec![CellAccounting::NonFungible],
                identity: CellIdentity::Singleton,
                settlement: CellSettlement::Pending,
            },
        };
        assert_eq!(
            validate_typed_cell_decl(&shared_none),
            Err(TypedCellDeclError::MutableCellWithNoneConflictKey),
            "Shared mutable cell with ConflictKeySpec::None must be rejected"
        );
    }

    #[test]
    fn test_composite_conflict_key_produces_different_hash_than_field_key() {
        let script = test_script(0xAA, 1, 4);
        let field_key = b"pool_id=A";
        let composite_key = encode_conflict_key_value_composite(&[b"pool_id", b"A"]);

        let ch_field = compute_conflict_hash(&script, field_key);
        let ch_composite = compute_conflict_hash(&script, &composite_key);

        assert_ne!(ch_field, ch_composite, "field key and composite key must produce different hashes");
    }

    #[test]
    fn test_operation_source_validation() {
        // Valid: CONSUME + INPUT
        let access = CellScriptSchedulerAccessWitness {
            operation: CELLSCRIPT_SCHEDULER_OP_CONSUME,
            source: CELLSCRIPT_SCHEDULER_SOURCE_INPUT,
            index: 0,
            conflict_hash: [0x24; 32],
            typed_data_hash: [0x00; 32],
        };
        assert!(validate_cellscript_scheduler_access_envelope(&access).is_ok());

        // Invalid: CONSUME + OUTPUT
        let access_bad = CellScriptSchedulerAccessWitness {
            operation: CELLSCRIPT_SCHEDULER_OP_CONSUME,
            source: CELLSCRIPT_SCHEDULER_SOURCE_OUTPUT,
            index: 0,
            conflict_hash: [0x24; 32],
            typed_data_hash: [0x00; 32],
        };
        assert!(validate_cellscript_scheduler_access_envelope(&access_bad).is_err());

        // Valid: READ_REF + INPUT
        let access_read_input = CellScriptSchedulerAccessWitness {
            operation: CELLSCRIPT_SCHEDULER_OP_READ_REF,
            source: CELLSCRIPT_SCHEDULER_SOURCE_INPUT,
            index: 0,
            conflict_hash: [0x24; 32],
            typed_data_hash: [0x00; 32],
        };
        assert!(validate_cellscript_scheduler_access_envelope(&access_read_input).is_ok());

        // Valid: READ_REF + CELL_DEP
        let access_read_dep = CellScriptSchedulerAccessWitness {
            operation: CELLSCRIPT_SCHEDULER_OP_READ_REF,
            source: CELLSCRIPT_SCHEDULER_SOURCE_CELL_DEP,
            index: 0,
            conflict_hash: [0x24; 32],
            typed_data_hash: [0x00; 32],
        };
        assert!(validate_cellscript_scheduler_access_envelope(&access_read_dep).is_ok());

        // Invalid: READ_REF + OUTPUT
        let access_read_output = CellScriptSchedulerAccessWitness {
            operation: CELLSCRIPT_SCHEDULER_OP_READ_REF,
            source: CELLSCRIPT_SCHEDULER_SOURCE_OUTPUT,
            index: 0,
            conflict_hash: [0x24; 32],
            typed_data_hash: [0x00; 32],
        };
        assert!(validate_cellscript_scheduler_access_envelope(&access_read_output).is_err());

        // Valid: CREATE + OUTPUT
        let access_create = CellScriptSchedulerAccessWitness {
            operation: CELLSCRIPT_SCHEDULER_OP_CREATE,
            source: CELLSCRIPT_SCHEDULER_SOURCE_OUTPUT,
            index: 0,
            conflict_hash: [0x24; 32],
            typed_data_hash: [0x00; 32],
        };
        assert!(validate_cellscript_scheduler_access_envelope(&access_create).is_ok());
    }
}
