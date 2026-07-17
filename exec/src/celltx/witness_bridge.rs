// SPDX-License-Identifier: MIT
// Copyright (C) 2026 Myelin developers
//
// Bridge for the cellscript compiler's legacy scheduler-witness wire format.
//
// The cellscript compiler emits a 9-field Molecule table whose access records
// are 38 bytes (`operation | source | index | binding_hash`) and whose envelope
// carries `touches_shared`. Myelin's runtime, by contrast, already ships a
// 7-field schema with 70-byte access records (`... | conflict_hash |
// typed_data_hash`) and no `touches_shared` — a deliberately stronger conflict
// model that binds type-script identity into the conflict hash.
//
// These two formats share the `0xCE11` magic and version `1` but are NOT
// byte-compatible (different field counts and access widths). Cellscript
// cannot emit Myelin's `conflict_hash` directly because the compiler does not
// know the deployed type-script identity at compile time — it only knows the
// source-level `binding` name.
//
// This module decodes cellscript's legacy 9-field bytes and recomputes Myelin's
// `conflict_hash` / `typed_data_hash` from the transaction's concrete cells,
// producing a `CellScriptSchedulerWitness` that the existing DAG and execution
// report paths consume unchanged. Recomputation is deterministic and runs once
// at decode time.

use super::types::{
    compute_conflict_hash, compute_typed_data_hash, scheduler_molecule_decode_table, scheduler_molecule_encode_table,
    scheduler_molecule_pack_number, scheduler_molecule_unpack_number, CellScriptSchedulerAccessWitness,
    CellScriptSchedulerWitness, CellScriptSchedulerWitnessError, CellTx,
    CELLSCRIPT_SCHEDULER_SOURCE_CELL_DEP, CELLSCRIPT_SCHEDULER_SOURCE_INPUT, CELLSCRIPT_SCHEDULER_SOURCE_OUTPUT,
    CELLSCRIPT_SCHEDULER_WITNESS_MAGIC, MAX_CELLSCRIPT_ACCESS_COUNT, TYPED_CELL_SCHEDULER_WITNESS_VERSION,
};

/// Legacy access record (38 bytes): `operation(1) | source(1) | index(4 LE) | binding_hash(32)`.
///
/// `binding_hash` is cellscript's `ckb_blake2b256(binding_string)` — a hash of
/// the source-level variable name the cell is bound to (e.g. `"coin"`). It does
/// NOT carry type-script identity.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CellscriptLegacyAccess {
    /// Operation id (consume=1, transfer=2, destroy=3, read_ref=6, create=7, …).
    pub operation: u8,
    /// Source id (Input=1, CellDep=2, Output=3).
    pub source: u8,
    /// Zero-based index into the source vector.
    pub index: u32,
    /// `ckb_blake2b256(binding_string)` — hash of the source-level binding name.
    pub binding_hash: [u8; 32],
}

/// Legacy 9-field scheduler witness envelope emitted by the cellscript compiler.
///
/// Field order matches `cellscript/src/stdlib/mod.rs::generate_molecule`:
/// `magic | version | effect_class | parallelizable | touches_shared_count |
///  touches_shared | estimated_cycles | access_count | accesses`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CellscriptLegacyWitness {
    /// Magic marker (`0xCE11`).
    pub magic: u16,
    /// Witness format version (`1`).
    pub version: u8,
    /// Effect class id (Pure=0, ReadOnly=1, Mutating=2, Creating=3, Destroying=4).
    pub effect_class: u8,
    /// Compiler hint: whether the action is parallelizable.
    pub parallelizable: bool,
    /// Type-name hashes of shared-state bindings the action touches (dropped on translate).
    pub touches_shared: Vec<[u8; 32]>,
    /// Compiler-estimated cycle count.
    pub estimated_cycles: u64,
    /// Per-access records (`operation | source | index | binding_hash`).
    pub accesses: Vec<CellscriptLegacyAccess>,
}

/// Legacy access record width: `op(1) + source(1) + index(4) + binding_hash(32)`.
const LEGACY_ACCESS_MOLECULE_SIZE: usize = 38;
/// Legacy witness envelope field count (the 9-field cellscript layout).
const LEGACY_WITNESS_MOLECULE_FIELDS: usize = 9;

impl CellscriptLegacyWitness {
    /// Decode a cellscript-emitted 9-field scheduler witness from Molecule bytes.
    pub fn decode(bytes: &[u8]) -> Result<Self, CellScriptSchedulerWitnessError> {
        const TY: &str = "CellscriptLegacyWitness";
        let fields =
            scheduler_molecule_decode_table(bytes, LEGACY_WITNESS_MOLECULE_FIELDS, TY).map_err(CellScriptSchedulerWitnessError::Decode)?;

        // fields[0] = magic (u16 LE)
        let magic = u16::from_le_bytes([first_byte(fields[0]), fields[0].get(1).copied().unwrap_or(0)]);
        // fields[1] = version (u8)
        let version = first_byte(fields[1]);
        // fields[2] = effect_class (u8)
        let effect_class = first_byte(fields[2]);
        // fields[3] = parallelizable (bool/u8)
        let parallelizable = first_byte(fields[3]) != 0;
        // fields[4] = touches_shared_count (u32 LE)
        let touches_shared_count = scheduler_molecule_unpack_number(fields[4], TY).map_err(CellScriptSchedulerWitnessError::Decode)?;
        // fields[5] = touches_shared fixvec of [u8;32]
        let touches_shared = decode_fixvec_byte32(fields[5], touches_shared_count, TY)?;
        // fields[6] = estimated_cycles (u64 LE)
        let estimated_cycles = decode_u64_le(fields[6]);
        // fields[7] = access_count (u32 LE)
        let access_count = scheduler_molecule_unpack_number(fields[7], TY).map_err(CellScriptSchedulerWitnessError::Decode)?;
        if access_count > MAX_CELLSCRIPT_ACCESS_COUNT as usize {
            return Err(CellScriptSchedulerWitnessError::CountMismatch {
                field: "access_count",
                declared: access_count as u32,
                actual: MAX_CELLSCRIPT_ACCESS_COUNT as usize,
            });
        }
        // fields[8] = accesses fixvec of 38-byte records
        let accesses = decode_legacy_accesses(fields[8], access_count, TY)?;

        Ok(Self { magic, version, effect_class, parallelizable, touches_shared, estimated_cycles, accesses })
    }

    /// Encode back to the cellscript 9-field Molecule layout (inverse of `decode`).
    pub fn encode(&self) -> Vec<u8> {
        let touches_shared_field = encode_fixvec_byte32(&self.touches_shared);
        let accesses_field = encode_legacy_accesses(&self.accesses);
        scheduler_molecule_encode_table(&[
            self.magic.to_le_bytes().to_vec(),
            vec![self.version],
            vec![self.effect_class],
            vec![u8::from(self.parallelizable)],
            scheduler_molecule_pack_number(self.touches_shared.len()).to_vec(),
            touches_shared_field,
            self.estimated_cycles.to_le_bytes().to_vec(),
            scheduler_molecule_pack_number(self.accesses.len()).to_vec(),
            accesses_field,
        ])
    }
}

/// Translate a decoded legacy witness into Myelin's native `CellScriptSchedulerWitness`,
/// recomputing `conflict_hash` and `typed_data_hash` from the transaction's
/// concrete cells.
///
/// ## Conflict-hash recomputation
///
/// Myelin's native `conflict_hash` binds **type-script identity + conflict-key**.
/// Cellscript's `binding_hash` is only the binding-name hash (no type-script).
/// The bridge reconstructs the strongest possible conflict domain from what is
/// available at decode time:
///
/// - **Output accesses** (`source = OUTPUT`): the type script is in the tx
///   itself (`tx.outputs[index].type_`), so we use the full
///   `compute_conflict_hash(type_script, binding_hash)`. Two outputs conflict
///   iff they share the same type script AND the same binding — Myelin's
///   intended semantics.
/// - **Input / CellDep accesses**: the consumed cell is referenced by OutPoint
///   and is NOT present in the tx, so the type script is unknown. We fall back
///   to a witness-only domain `blake3("myelin:legacy-bridge-conflict:v1" ||
///   binding_hash)`. Two such accesses collide iff they share the same
///   binding hash — weaker than the Output path but deterministic and the best
///   the available information supports.
///
/// `touches_shared` is dropped: once `conflict_hash` binds type-script identity
/// (on the Output path), the "does this action touch shared state?" question is
/// derivable from the conflict domain, matching Myelin's rationale for removing
/// the field in the native schema.
///
/// `typed_data_hash` is computed on the Output path (data is in the tx); on
/// Input/CellDep it is left zeroed (it is not read by the DAG scheduler; the
/// execution report computes its own).
pub fn translate_legacy_witness_for_tx(tx: &CellTx, legacy: &CellscriptLegacyWitness) -> Result<CellScriptSchedulerWitness, CellScriptSchedulerWitnessError> {
    let accesses = legacy
        .accesses
        .iter()
        .map(|access| translate_one_access(tx, access))
        .collect::<Result<Vec<_>, _>>()?;

    Ok(CellScriptSchedulerWitness {
        magic: u16::from_le_bytes(CELLSCRIPT_SCHEDULER_WITNESS_MAGIC),
        version: TYPED_CELL_SCHEDULER_WITNESS_VERSION,
        effect_class: legacy.effect_class,
        parallelizable: legacy.parallelizable,
        estimated_cycles: legacy.estimated_cycles,
        access_count: accesses.len() as u32,
        accesses,
    })
}

fn translate_one_access(tx: &CellTx, access: &CellscriptLegacyAccess) -> Result<CellScriptSchedulerAccessWitness, CellScriptSchedulerWitnessError> {
    let index = usize::try_from(access.index).unwrap_or(usize::MAX);
    let (conflict_hash, typed_data_hash) = match access.source {
        CELLSCRIPT_SCHEDULER_SOURCE_OUTPUT => {
            let output = tx.outputs.get(index).ok_or(CellScriptSchedulerWitnessError::SourceIndexOutOfBounds {
                source_id: access.source,
                index: access.index,
                available: tx.outputs.len(),
            })?;
            let data = tx.outputs_data.get(index).cloned().unwrap_or_default();
            match output.type_.as_ref() {
                Some(type_script) => (
                    compute_conflict_hash(type_script, &access.binding_hash),
                    compute_typed_data_hash(type_script, &data),
                ),
                None => (legacy_binding_only_conflict(&access.binding_hash), [0u8; 32]),
            }
        }
        CELLSCRIPT_SCHEDULER_SOURCE_INPUT | CELLSCRIPT_SCHEDULER_SOURCE_CELL_DEP => {
            // Consumed cell not present in tx; fall back to binding-only domain.
            (legacy_binding_only_conflict(&access.binding_hash), [0u8; 32])
        }
        other => return Err(CellScriptSchedulerWitnessError::InvalidSource(other)),
    };

    Ok(CellScriptSchedulerAccessWitness {
        operation: access.operation,
        source: access.source,
        index: access.index,
        conflict_hash,
        typed_data_hash,
    })
}

/// Witness-only conflict domain for Input/CellDep accesses where the type
/// script cannot be resolved from the tx alone. Distinct domain tag from the
/// full `compute_conflict_hash` so the two never collide.
fn legacy_binding_only_conflict(binding_hash: &[u8; 32]) -> [u8; 32] {
    let mut hasher = blake3::Hasher::new();
    hasher.update(b"myelin:legacy-bridge-conflict:v1");
    hasher.update(binding_hash);
    *hasher.finalize().as_bytes()
}

// ---- Molecule helpers for the legacy layout (fixvec of [u8;32] and 38-byte records) ----

fn decode_fixvec_byte32(bytes: &[u8], expected_count: usize, ty: &'static str) -> Result<Vec<[u8; 32]>, CellScriptSchedulerWitnessError> {
    let count = scheduler_molecule_unpack_number(bytes, ty).map_err(CellScriptSchedulerWitnessError::Decode)?;
    if count != expected_count {
        return Err(CellScriptSchedulerWitnessError::Decode(format!(
            "{ty}: touches_shared count mismatch: header {expected_count}, fixvec {count}"
        )));
    }
    let body = &bytes[4..];
    if body.len() != count * 32 {
        return Err(CellScriptSchedulerWitnessError::Decode(format!(
            "{ty}: touches_shared body expected {} bytes, got {}",
            count * 32,
            body.len()
        )));
    }
    let mut out = Vec::with_capacity(count);
    for chunk in body.chunks_exact(32) {
        let mut v = [0u8; 32];
        v.copy_from_slice(chunk);
        out.push(v);
    }
    Ok(out)
}

fn decode_legacy_accesses(bytes: &[u8], expected_count: usize, ty: &'static str) -> Result<Vec<CellscriptLegacyAccess>, CellScriptSchedulerWitnessError> {
    let count = scheduler_molecule_unpack_number(bytes, ty).map_err(CellScriptSchedulerWitnessError::Decode)?;
    if count != expected_count {
        return Err(CellScriptSchedulerWitnessError::Decode(format!(
            "{ty}: access count mismatch: header {expected_count}, fixvec {count}"
        )));
    }
    let body = &bytes[4..];
    if body.len() != count * LEGACY_ACCESS_MOLECULE_SIZE {
        return Err(CellScriptSchedulerWitnessError::Decode(format!(
            "{ty}: accesses body expected {} bytes, got {}",
            count * LEGACY_ACCESS_MOLECULE_SIZE,
            body.len()
        )));
    }
    let mut out = Vec::with_capacity(count);
    for chunk in body.chunks_exact(LEGACY_ACCESS_MOLECULE_SIZE) {
        let operation = chunk[0];
        let source = chunk[1];
        let index = u32::from_le_bytes([chunk[2], chunk[3], chunk[4], chunk[5]]);
        let mut binding_hash = [0u8; 32];
        binding_hash.copy_from_slice(&chunk[6..38]);
        out.push(CellscriptLegacyAccess { operation, source, index, binding_hash });
    }
    Ok(out)
}

/// Read the first byte of a slice, defaulting to 0 if empty.
fn first_byte(bytes: &[u8]) -> u8 {
    *bytes.first().unwrap_or(&0)
}

fn decode_u64_le(bytes: &[u8]) -> u64 {
    let mut arr = [0u8; 8];
    let n = bytes.len().min(8);
    arr[..n].copy_from_slice(&bytes[..n]);
    u64::from_le_bytes(arr)
}

fn encode_fixvec_byte32(values: &[[u8; 32]]) -> Vec<u8> {
    let mut out = Vec::with_capacity(4 + values.len() * 32);
    out.extend_from_slice(&scheduler_molecule_pack_number(values.len()));
    for value in values {
        out.extend_from_slice(value);
    }
    out
}

fn encode_legacy_accesses(accesses: &[CellscriptLegacyAccess]) -> Vec<u8> {
    let mut out = Vec::with_capacity(4 + accesses.len() * LEGACY_ACCESS_MOLECULE_SIZE);
    out.extend_from_slice(&scheduler_molecule_pack_number(accesses.len()));
    for access in accesses {
        out.push(access.operation);
        out.push(access.source);
        out.extend_from_slice(&access.index.to_le_bytes());
        out.extend_from_slice(&access.binding_hash);
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use super::super::types::{
        CellOutput, Script, CELLSCRIPT_SCHEDULER_EFFECT_CREATING, CELLSCRIPT_SCHEDULER_EFFECT_MUTATING,
        CELLSCRIPT_SCHEDULER_OP_CONSUME, CELLSCRIPT_SCHEDULER_OP_CREATE, CELLSCRIPT_SCHEDULER_WITNESS_VERSION,
    };
    // u16 form of the magic constant ([u8;2] in types.rs).
    const MAGIC: u16 = u16::from_le_bytes(CELLSCRIPT_SCHEDULER_WITNESS_MAGIC);

    fn sample_legacy() -> CellscriptLegacyWitness {
        CellscriptLegacyWitness {
            magic: MAGIC,
            version: CELLSCRIPT_SCHEDULER_WITNESS_VERSION,
            effect_class: CELLSCRIPT_SCHEDULER_EFFECT_MUTATING,
            parallelizable: false,
            touches_shared: vec![[0x24; 32]],
            estimated_cycles: 500,
            accesses: vec![CellscriptLegacyAccess {
                operation: CELLSCRIPT_SCHEDULER_OP_CONSUME,
                source: CELLSCRIPT_SCHEDULER_SOURCE_INPUT,
                index: 0,
                binding_hash: [0xAB; 32],
            }],
        }
    }

    #[test]
    fn legacy_witness_round_trips_through_molecule() {
        let original = sample_legacy();
        let bytes = original.encode();
        let decoded = CellscriptLegacyWitness::decode(&bytes).expect("decode");
        assert_eq!(decoded, original);
    }

    #[test]
    fn legacy_decode_rejects_wrong_field_count() {
        // Truncate to force a field-count mismatch: feed a 7-field (Myelin
        // native) witness to the legacy 9-field decoder.
        let native = CellScriptSchedulerWitness {
            magic: MAGIC,
            version: TYPED_CELL_SCHEDULER_WITNESS_VERSION,
            effect_class: CELLSCRIPT_SCHEDULER_EFFECT_MUTATING,
            parallelizable: false,
            estimated_cycles: 500,
            access_count: 0,
            accesses: vec![],
        };
        let native_bytes = super::super::types::encode_cellscript_scheduler_witness_molecule(&native);
        assert!(CellscriptLegacyWitness::decode(&native_bytes).is_err());
    }

    #[test]
    fn translate_output_access_uses_full_type_script_conflict_hash() {
        // Two txs creating an output with the same type script + binding must
        // produce colliding conflict_hashes (so the DAG schedules them serially).
        let type_script = Script::new([0x99; 32], 1, vec![0x01, 0x02]);
        let binding = [0xAB; 32];

        let make_tx = |data: &[u8]| {
            let lock = Script::new([0x00; 32], 0, vec![]);
            let output = CellOutput { lock, type_: Some(type_script.clone()), capacity: 1000 };
            CellTx::new(vec![], vec![], vec![output], vec![data.to_vec()], vec![]).unwrap()
        };
        let tx_a = make_tx(&[1, 2, 3]);
        let tx_b = make_tx(&[4, 5, 6]); // different data, same type script + binding

        let legacy = CellscriptLegacyWitness {
            magic: MAGIC,
            version: CELLSCRIPT_SCHEDULER_WITNESS_VERSION,
            effect_class: CELLSCRIPT_SCHEDULER_EFFECT_CREATING,
            parallelizable: false,
            touches_shared: vec![],
            estimated_cycles: 100,
            accesses: vec![CellscriptLegacyAccess {
                operation: CELLSCRIPT_SCHEDULER_OP_CREATE,
                source: CELLSCRIPT_SCHEDULER_SOURCE_OUTPUT,
                index: 0,
                binding_hash: binding,
            }],
        };

        let translated_a = translate_legacy_witness_for_tx(&tx_a, &legacy).expect("translate a");
        let translated_b = translate_legacy_witness_for_tx(&tx_b, &legacy).expect("translate b");

        // Same type script + binding => same conflict_hash (they conflict).
        assert_eq!(
            translated_a.accesses[0].conflict_hash, translated_b.accesses[0].conflict_hash,
            "same type script + binding must collide"
        );
        // Different data => different typed_data_hash.
        assert_ne!(
            translated_a.accesses[0].typed_data_hash, translated_b.accesses[0].typed_data_hash,
            "different data must produce different typed_data_hash"
        );
    }

    #[test]
    fn translate_output_access_different_type_scripts_do_not_collide() {
        let script_a = Script::new([0x11; 32], 1, vec![]);
        let script_b = Script::new([0x22; 32], 1, vec![]);
        let binding = [0xAB; 32];

        let make_tx = |script: Script| {
            let lock = Script::new([0x00; 32], 0, vec![]);
            let output = CellOutput { lock, type_: Some(script), capacity: 1000 };
            CellTx::new(vec![], vec![], vec![output], vec![vec![]], vec![]).unwrap()
        };

        let legacy = CellscriptLegacyWitness {
            magic: MAGIC,
            version: CELLSCRIPT_SCHEDULER_WITNESS_VERSION,
            effect_class: CELLSCRIPT_SCHEDULER_EFFECT_CREATING,
            parallelizable: false,
            touches_shared: vec![],
            estimated_cycles: 100,
            accesses: vec![CellscriptLegacyAccess {
                operation: CELLSCRIPT_SCHEDULER_OP_CREATE,
                source: CELLSCRIPT_SCHEDULER_SOURCE_OUTPUT,
                index: 0,
                binding_hash: binding,
            }],
        };

        let translated_a = translate_legacy_witness_for_tx(&make_tx(script_a), &legacy).expect("translate a");
        let translated_b = translate_legacy_witness_for_tx(&make_tx(script_b), &legacy).expect("translate b");

        assert_ne!(
            translated_a.accesses[0].conflict_hash, translated_b.accesses[0].conflict_hash,
            "different type scripts must NOT collide even with the same binding"
        );
    }

    #[test]
    fn translate_input_access_uses_binding_only_domain() {
        // Input accesses cannot resolve the consumed cell's type script from
        // the tx alone; the bridge falls back to a binding-only domain.
        let tx = CellTx::new(
            vec![],
            vec![],
            vec![CellOutput { lock: Script::new([0; 32], 0, vec![]), type_: None, capacity: 0 }],
            vec![vec![]],
            vec![],
        )
        .unwrap();

        let legacy = CellscriptLegacyWitness {
            magic: MAGIC,
            version: CELLSCRIPT_SCHEDULER_WITNESS_VERSION,
            effect_class: CELLSCRIPT_SCHEDULER_EFFECT_MUTATING,
            parallelizable: false,
            touches_shared: vec![],
            estimated_cycles: 100,
            accesses: vec![CellscriptLegacyAccess {
                operation: CELLSCRIPT_SCHEDULER_OP_CONSUME,
                source: CELLSCRIPT_SCHEDULER_SOURCE_INPUT,
                index: 0,
                binding_hash: [0xAB; 32],
            }],
        };

        let translated = translate_legacy_witness_for_tx(&tx, &legacy).expect("translate");
        // Binding-only domain is non-zero and stable.
        assert_ne!(translated.accesses[0].conflict_hash, [0u8; 32]);
        // typed_data_hash is zeroed on the Input path (not resolvable).
        assert_eq!(translated.accesses[0].typed_data_hash, [0u8; 32]);
    }
}
