// SPDX-License-Identifier: MIT
// Copyright (C) 2026 Myelin developers
//
// Cell transaction core types (CKB-inspired, scheduler-adapted)
//
// Reference: ckb/util/types/src/core/cell.rs

use serde::{Deserialize, Serialize};
use std::{
    collections::{BTreeMap, BTreeSet},
    fmt,
};

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

/// CKB transaction version used by the canonical CellTx wire shape.
pub const CELL_TX_VERSION: u32 = 0;
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
/// `blake3(domain || code_hash || hash_type || len(args) || args || len(conflict_key_value) || conflict_key_value)`
///
/// Does NOT change when cell data is updated.
/// Used by CellDAG conflict detection.
///
/// Length prefixes are inserted between variable-length fields so that
/// `(args="X", conflict_key_value="")` cannot collide with
/// `(args="", conflict_key_value="X")` — the same canonical form used by
/// `Script::hash_v1` and `encode_conflict_key_value_composite`.
pub fn compute_conflict_hash(type_script: &Script, conflict_key_value: &[u8]) -> [u8; 32] {
    let mut hasher = blake3::Hasher::new();
    hasher.update(b"myelin-typed-cell/conflict-hash/v1");
    hasher.update(&type_script.code_hash);
    hasher.update(&[type_script.hash_type]);
    hasher.update(&(type_script.args.len() as u32).to_le_bytes());
    hasher.update(&type_script.args);
    hasher.update(&(conflict_key_value.len() as u32).to_le_bytes());
    hasher.update(conflict_key_value);
    *hasher.finalize().as_bytes()
}

/// Compute typed data hash.
///
/// `blake3(domain || code_hash || hash_type || len(args) || args || len(data) || data)`
///
/// Changes with every data update.
/// Named `typed_data_hash` (not `cell_state_hash`) because it does NOT
/// include lock/capacity — only type script identity + data.
///
/// Length prefixes prevent `(args="X", data="")` from colliding with
/// `(args="", data="X")` — see `compute_conflict_hash` for the same rule.
pub fn compute_typed_data_hash(type_script: &Script, data: &[u8]) -> [u8; 32] {
    let mut hasher = blake3::Hasher::new();
    hasher.update(b"myelin-typed-cell/typed-data-hash/v1");
    hasher.update(&type_script.code_hash);
    hasher.update(&[type_script.hash_type]);
    hasher.update(&(type_script.args.len() as u32).to_le_bytes());
    hasher.update(&type_script.args);
    hasher.update(&(data.len() as u32).to_le_bytes());
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
    match &decl.runtime.conflict_key {
        ConflictKeySpec::Field(field) if field.trim().is_empty() => return Err(TypedCellDeclError::EmptyConflictField),
        ConflictKeySpec::Composite(fields) if fields.is_empty() => return Err(TypedCellDeclError::EmptyCompositeConflictKey),
        ConflictKeySpec::Composite(fields) => {
            let mut unique = BTreeSet::new();
            for field in fields {
                if field.trim().is_empty() {
                    return Err(TypedCellDeclError::EmptyConflictField);
                }
                if !unique.insert(field) {
                    return Err(TypedCellDeclError::DuplicateConflictField(field.clone()));
                }
            }
        }
        _ => {}
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
    /// Field names are schema identifiers and cannot be empty.
    #[error("conflict-key field name must not be empty")]
    EmptyConflictField,
    /// Composite conflict keys must contain at least one field.
    #[error("composite conflict key must contain at least one field")]
    EmptyCompositeConflictKey,
    /// Repeating a field would create an ambiguous key schema.
    #[error("duplicate conflict-key field {0}")]
    DuplicateConflictField(String),
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

pub(super) fn scheduler_molecule_pack_number(value: usize) -> [u8; 4] {
    (value as u32).to_le_bytes()
}

pub(super) fn scheduler_molecule_unpack_number(bytes: &[u8], ty: &'static str) -> Result<usize, String> {
    if bytes.len() < 4 {
        return Err(format!("{ty}: expected at least 4 bytes for number, got {}", bytes.len()));
    }
    Ok(u32::from_le_bytes([bytes[0], bytes[1], bytes[2], bytes[3]]) as usize)
}

pub(super) fn scheduler_molecule_encode_table(fields: &[Vec<u8>]) -> Vec<u8> {
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

pub(super) fn scheduler_molecule_decode_table<'a>(
    bytes: &'a [u8],
    expected_fields: usize,
    ty: &'static str,
) -> Result<Vec<&'a [u8]>, String> {
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

fn scheduler_molecule_decode_u8(bytes: &[u8], ty: &'static str) -> Result<u8, String> {
    if bytes.len() != 1 {
        return Err(format!("{ty}: expected 1 byte, got {}", bytes.len()));
    }
    Ok(bytes[0])
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

/// Parse CKB Molecule `OutPointVec` DepGroup cell data.
pub fn parse_ckb_dep_group_data(data: &[u8]) -> Result<Vec<OutPoint>, String> {
    crate::serialization::molecule_compat::deserialize_ckb_outpoint_vec_molecule(data).map_err(|error| error.to_string())
}

/// Encode CKB Molecule `OutPointVec` DepGroup cell data.
pub fn encode_ckb_dep_group_data(outpoints: &[OutPoint]) -> Result<Vec<u8>, String> {
    crate::serialization::molecule_compat::serialize_ckb_outpoint_vec_molecule(outpoints).map_err(|error| error.to_string())
}

/// Cell transaction (complete structure)
///
/// Reference: CKB Transaction
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct CellTx {
    /// CKB transaction version. New transactions always use version `0`.
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
    fn test_ckb_dep_group_roundtrip() {
        let ops = vec![OutPoint::new([0x11; 32], 0), OutPoint::new([0x22; 32], 7), OutPoint::new([0x33; 32], u32::MAX)];
        let data = encode_ckb_dep_group_data(&ops).unwrap();
        let parsed = parse_ckb_dep_group_data(&data).unwrap();
        assert_eq!(parsed, ops);
    }

    #[test]
    fn test_ckb_dep_group_rejects_empty() {
        let historical_empty = [0, 0, 0, 0];
        assert!(encode_ckb_dep_group_data(&[]).is_err());
        assert!(parse_ckb_dep_group_data(&historical_empty).is_err());
    }

    #[test]
    fn test_ckb_dep_group_rejects_invalid_data() {
        assert!(parse_ckb_dep_group_data(&[]).is_err());
        assert!(parse_ckb_dep_group_data(&[1, 0, 0, 0]).is_err());
        assert!(parse_ckb_dep_group_data(&[1, 0, 0, 0, 0]).is_err());
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
}
