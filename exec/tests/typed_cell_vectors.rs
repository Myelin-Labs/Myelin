use myelin_exec::celltx::{
    compute_conflict_hash, compute_typed_data_hash, encode_cellscript_scheduler_witness_molecule, encode_conflict_key_value_composite,
    CellScriptSchedulerAccessWitness, CellScriptSchedulerWitness, Script, CELLSCRIPT_SCHEDULER_EFFECT_MUTATING,
    CELLSCRIPT_SCHEDULER_OP_CONSUME, CELLSCRIPT_SCHEDULER_SOURCE_INPUT, TYPED_CELL_SCHEDULER_WITNESS_VERSION,
};

fn hex(bytes: impl AsRef<[u8]>) -> String {
    bytes.as_ref().iter().map(|byte| format!("{byte:02x}")).collect()
}

#[test]
fn typed_cell_hash_fixed_vectors() {
    let script = Script::new([0x42; 32], 1, b"invoice-script-args".to_vec());
    let conflict_key = b"invoice:INV-2026-0001";
    let data = b"invoice-state:issued:amount=1250000";

    assert_eq!(hex(compute_conflict_hash(&script, conflict_key)), "55c114e95a617a3b3268f8f640f67de241d6d3e9817fd43a364a624aa84e79e9");
    assert_eq!(hex(compute_typed_data_hash(&script, data)), "70932e44d606cd2f0d77c23b626c2359be78a46bc74060199d62a10915e1a607");
    assert_eq!(
        hex(encode_conflict_key_value_composite(&[b"borrower:acme", b"invoice:INV-2026-0001"])),
        "0d000000626f72726f7765723a61636d6515000000696e766f6963653a494e562d323032362d30303031"
    );
}

#[test]
fn typed_cell_scheduler_witness_molecule_fixed_vector() {
    let script = Script::new([0x42; 32], 1, b"invoice-script-args".to_vec());
    let conflict_hash = compute_conflict_hash(&script, b"invoice:INV-2026-0001");
    let typed_data_hash = compute_typed_data_hash(&script, b"invoice-state:issued:amount=1250000");
    let witness = CellScriptSchedulerWitness {
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

    assert_eq!(
        hex(encode_cellscript_scheduler_witness_molecule(&witness)),
        concat!(
            "7b00000020000000220000002300000024000000250000002d00000031000000",
            "11ce010200f40100000000000001000000",
            "01000000010100000000",
            "55c114e95a617a3b3268f8f640f67de241d6d3e9817fd43a364a624aa84e79e9",
            "70932e44d606cd2f0d77c23b626c2359be78a46bc74060199d62a10915e1a607"
        )
    );
}
