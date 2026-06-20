use cellscript::{
    ckb_blake2b256, compile, CkbScriptArgsValue, CkbScriptHashTypeValue, CkbScriptValue, CompileOptions, EntryWitnessArg,
};
use ckb_testtool::ckb_types::{
    bytes::Bytes,
    core::{ScriptHashType, TransactionBuilder},
    packed,
    prelude::*,
};
use ckb_testtool::context::Context;

const SCRIPT_REF_READ_PROGRAM: &str = r#"
module v018::script_ref_read

action inspect(
    expected_lock_hash: Hash,
    expected_type_hash: Hash,
    expected_lock_code_hash: Hash,
    expected_type_code_hash: Hash,
    expected_lock_args_hash: Hash,
    expected_type_args_hash: Hash
) -> u64 {
    verification
        let input = source::input(0)
        let lock_hash: Hash = ckb::cell_lock_hash(input)
        let type_hash: Hash = ckb::cell_type_hash(input)
        let lock_code_hash: Hash = ckb::cell_lock_code_hash(input)
        let type_code_hash: Hash = ckb::cell_type_code_hash(input)
        let lock_hash_type = ckb::cell_lock_hash_type(input)
        let type_hash_type = ckb::cell_type_hash_type(input)
        let lock_args_empty = ckb::cell_lock_args_empty(input)
        let type_args_empty = ckb::cell_type_args_empty(input)
        let lock_args_hash: Hash = ckb::cell_lock_args_hash(input)
        let type_args_hash: Hash = ckb::cell_type_args_hash(input)
        require lock_hash == expected_lock_hash
        require type_hash == expected_type_hash
        require lock_code_hash == expected_lock_code_hash
        require type_code_hash == expected_type_code_hash
        require lock_args_hash == expected_lock_args_hash
        require type_args_hash == expected_type_args_hash
        ckb::require_cell_lock_args_prefix_hash(input, expected_lock_args_hash)
        ckb::require_cell_type_args_prefix_hash(input, expected_type_args_hash)
        ckb::require_cell_lock_args_suffix_hash(input, expected_lock_args_hash)
        ckb::require_cell_type_args_suffix_hash(input, expected_type_args_hash)
        let lock_empty_flag = if lock_args_empty { 1 } else { 0 }
        let type_empty_flag = if type_args_empty { 1 } else { 0 }
        return lock_hash_type + type_hash_type + lock_empty_flag + type_empty_flag
}
"#;

const SCRIPT_REF_PROPERTY_PROGRAM: &str = r#"
module v018::script_ref_property

action inspect(
    expected_lock_code_hash: Hash,
    expected_type_code_hash: Hash,
    expected_lock_args_hash: Hash,
    expected_type_args_hash: Hash
) -> u64 {
    verification
        let input = source::input(0)
        let lock = input.lock
        let type_script = input.type
        let lock_code_hash: Hash = lock.code_hash
        let type_code_hash: Hash = type_script.code_hash
        let lock_args_hash: Hash = input.lock.args_hash
        let type_args_hash: Hash = input.type.args_hash
        let lock_args_empty = input.lock.args_empty
        let type_args_empty = input.type.args_empty
        require lock_code_hash == expected_lock_code_hash
        require type_code_hash == expected_type_code_hash
        require lock_args_hash == expected_lock_args_hash
        require type_args_hash == expected_type_args_hash
        let lock_empty_flag = if lock_args_empty { 1 } else { 0 }
        let type_empty_flag = if type_args_empty { 1 } else { 0 }
        return lock.hash_type + type_script.hash_type + lock_empty_flag + type_empty_flag
}
"#;

const SCRIPT_CONSTRUCTION_PROGRAM: &str = r#"
module v018::script_construction

action inspect() -> u64 {
    verification
        let input = source::group_input(0)
        let code_hash: Hash = ckb::cell_lock_code_hash(input)
        let hash_type = ckb::cell_lock_hash_type(input)
        let args = script::args(b"owner")
        let expected = script::new(code_hash, hash_type, args)
        let same_code_hash: Hash = expected.code_hash
        require same_code_hash == code_hash
        require expected.hash_type == hash_type
        require expected.args.len == 5
        script::require_cell_lock_matches(input, expected)
        return 0
}
"#;

const SCRIPT_LITERAL_CONSTRUCTION_PROGRAM: &str = r#"
module v018::script_literal_construction

action inspect() -> u64 {
    verification
        let code_hash: Hash = Hash::from_bytes(b"\x01\x02\x03\x04\x05\x06\x07\x08\x09\x0a\x0b\x0c\x0d\x0e\x0f\x10\x11\x12\x13\x14\x15\x16\x17\x18\x19\x1a\x1b\x1c\x1d\x1e\x1f\x20")
        let expected = script::new(code_hash, script::hash_type_data2(), script::args_empty())
        let same_code_hash: Hash = expected.code_hash
        require same_code_hash == code_hash
        require expected.args.is_empty == true
        return expected.hash_type + expected.args.len
}
"#;

const CELL_DATA_DECODE_PROGRAM: &str = r#"
module v018::cell_data_decode

action inspect() -> u64 {
    verification
        let input = source::group_input(0)
        let quantity = ckb::cell_data_u32_le(input, 0)
        let amount = ckb::cell_data_u64_le(input, 4)
        if quantity != 7 {
            return 90
        }
        if amount != 123456789 {
            return 91
        }
        return 0
}
"#;

const CELL_DATA_U64_BINARY_GUARD_PROGRAM: &str = r#"
module v018::cell_data_u64_binary_guard

struct Header {
    old_nonce: u64,
    new_nonce: u64,
}

action inspect(witness header: Header) -> u64 {
    verification
        let input_nonce = ckb::cell_data_u64_le(source::input(0), 130)
        let output_nonce = ckb::cell_data_u64_le(source::output(0), 130)
        require input_nonce == header.old_nonce
        require output_nonce == header.new_nonce
        require output_nonce > input_nonce
        require output_nonce == input_nonce + 1
        return 0
}
"#;

const MYELIN_PACKAGE_COMMITMENT_TYPE_PROGRAM: &str = r#"
module v018::myelin_package_commitment_type

resource PackageCommitment has store, create
    identity(field(package_id))
{
    package_id: u64,
    word0: u64,
    word1: u64,
    word2: u64,
    word3: u64,
}

action verify_package_commitment(
    expected0: u64,
    expected1: u64,
    expected2: u64,
    expected3: u64
) -> u64 {
    verification
        let output = source::group_output(0)
        if ckb::cell_data_size(output) != 32 {
            return 10
        }

        let word0 = ckb::cell_data_u64_le(output, 0)
        let word1 = ckb::cell_data_u64_le(output, 8)
        let word2 = ckb::cell_data_u64_le(output, 16)
        let word3 = ckb::cell_data_u64_le(output, 24)

        if word0 != expected0 {
            return 11
        }
        if word1 != expected1 {
            return 12
        }
        if word2 != expected2 {
            return 13
        }
        if word3 != expected3 {
            return 14
        }
        return 0
}
"#;

const MYELIN_GENERIC_PACKAGE_COMMITMENT_TYPE_PROGRAM: &str = r#"
module v018::myelin_generic_package_commitment_type

resource PackageCommitment has store, create
    identity(script_args)
{
    word0: u64,
    word1: u64,
    word2: u64,
    word3: u64,
}

action verify_package_commitment() -> u64 {
    verification
        let output = source::group_output(0)
        if ckb::cell_data_size(output) != 32 {
            return 10
        }
        if ckb::cell_type_args_empty(output) {
            return 12
        }
        let data_hash: Hash = ckb::cell_data_hash(output)
        ckb::require_cell_type_args_hash(output, data_hash)
        return 0
}
"#;

const MYELIN_DA_ANCHOR_CARRIER_TYPE_PROGRAM: &str =
    include_str!("../examples/myelin/da-anchor-carrier.cell");
const MYELIN_SETTLEMENT_CARRIER_TYPE_PROGRAM: &str =
    include_str!("../examples/myelin/settlement-carrier.cell");
const MYELIN_DA_ANCHOR_FINAL_TYPE_PROGRAM: &str = include_str!("../examples/myelin/da-anchor-final.cell");
const MYELIN_SETTLEMENT_FINAL_TYPE_PROGRAM: &str = include_str!("../examples/myelin/settlement-final.cell");

const OUT_POINT_API_PROGRAM: &str = r#"
module v018::out_point_api

action inspect() -> u64 {
    verification
        let input = source::group_input(0)
        let tx_hash: Hash = ckb::input_out_point_tx_hash(input)
        let index = ckb::input_out_point_index(input)
        ckb::require_input_out_point_tx_hash(input, tx_hash)
        ckb::require_input_out_point(input, tx_hash, index)
        return 0
}
"#;

const OUT_POINT_OUTPUT_REJECT_PROGRAM: &str = r#"
module v018::out_point_output_reject

action inspect() -> u64 {
    verification
        let output = source::output(0)
        let tx_hash: Hash = ckb::input_out_point_tx_hash(output)
        ckb::require_input_out_point_tx_hash(output, tx_hash)
        return 0
}
"#;

const ALWAYS_SUCCESS_LOCK_PROGRAM: &str = r#"
module v018::always_success_lock

action always_success() -> u64 {
    verification
        return 0
}
"#;

fn compile_source_to_elf(source: &str, entry_action: &str) -> Vec<u8> {
    compile_source_to_elf_with_profile(source, entry_action, "ckb")
}

fn compile_source_to_elf_with_profile(source: &str, entry: &str, target_profile: &str) -> Vec<u8> {
    let result = compile(
        source,
        CompileOptions {
            target: Some("riscv64-elf".to_string()),
            target_profile: Some(target_profile.to_string()),
            primitive_compat: Some("0.18".to_string()),
            ..CompileOptions::default()
        },
    )
    .unwrap_or_else(|err| panic!("failed to compile {entry}: {}", err.message));
    let entry_found = result.metadata.actions.iter().any(|action| action.name == entry)
        || result.metadata.locks.iter().any(|lock| lock.name == entry);
    assert!(entry_found, "entry {entry} not found");
    cellscript::strip_vm_abi_trailer(&result.artifact_bytes).to_vec()
}

#[test]
fn v0_18_script_ref_reads_lower_to_fail_closed_ckb_helpers() {
    let result = compile(
        SCRIPT_REF_READ_PROGRAM,
        CompileOptions {
            target: Some("riscv64-asm".to_string()),
            target_profile: Some("ckb".to_string()),
            primitive_compat: Some("0.18".to_string()),
            ..CompileOptions::default()
        },
    )
    .expect("0.18 ScriptRef read program should compile");

    let assembly = std::str::from_utf8(&result.artifact_bytes).expect("assembly utf-8");
    for helper in [
        "__ckb_cell_lock_code_hash",
        "__ckb_cell_type_code_hash",
        "__ckb_cell_lock_hash",
        "__ckb_cell_type_hash",
        "__ckb_cell_lock_hash_type",
        "__ckb_cell_type_hash_type",
        "__ckb_cell_lock_args_empty",
        "__ckb_cell_type_args_empty",
        "__ckb_cell_lock_args_hash",
        "__ckb_cell_type_args_hash",
        "__ckb_require_cell_lock_args_prefix_hash",
        "__ckb_require_cell_type_args_prefix_hash",
        "__ckb_require_cell_lock_args_suffix_hash",
        "__ckb_require_cell_type_args_suffix_hash",
    ] {
        assert!(assembly.contains(&format!(".global {helper}")), "missing helper {helper}:\n{assembly}");
    }
    assert!(
        assembly.contains("read-only ScriptRef Hash field")
            && assembly.contains("read-only ScriptRef scalar field")
            && assembly.contains("load SourceView ScriptRef hash field into addressable Hash"),
        "ScriptRef reads must be explicit runtime extraction helpers:\n{assembly}"
    );
    assert!(
        assembly.contains("first 32 bytes == expected hash") && assembly.contains("last 32 bytes == expected hash"),
        "ScriptArgs prefix/suffix requirements must be visible in generated helpers:\n{assembly}"
    );
    assert!(
        assembly.contains("scalar runtime helper status check (a1 == 0)"),
        "scalar ScriptRef reads must fail closed on helper status:\n{assembly}"
    );

    let features = &result.metadata.runtime.ckb_runtime_features;
    assert!(features.contains(&"ckb-source-view".to_string()), "{features:?}");
    assert!(features.contains(&"ckb-source-cell-fields".to_string()), "{features:?}");
    assert!(features.contains(&"ckb-script-ref-read".to_string()), "{features:?}");
    assert!(features.contains(&"ckb-script-args-read".to_string()), "{features:?}");

    let accesses = result
        .metadata
        .runtime
        .ckb_runtime_accesses
        .iter()
        .map(|access| (access.operation.as_str(), access.syscall.as_str(), access.source.as_str()))
        .collect::<Vec<_>>();
    for operation in [
        "cell-lock-script-code-hash-read",
        "cell-type-script-code-hash-read",
        "cell-lock-hash-read",
        "cell-type-hash-read",
        "cell-lock-script-hash-type-read",
        "cell-type-script-hash-type-read",
        "cell-lock-script-args-empty-read",
        "cell-type-script-args-empty-read",
        "cell-lock-script-args-hash-read",
        "cell-type-script-args-hash-read",
        "cell-lock-script-prefix-hash-args-require",
        "cell-type-script-prefix-hash-args-require",
        "cell-lock-script-suffix-hash-args-require",
        "cell-type-script-suffix-hash-args-require",
    ] {
        assert!(accesses.contains(&(operation, "LOAD_CELL_BY_FIELD", "SourceView")), "{accesses:?}");
    }

    let elf = compile(
        SCRIPT_REF_READ_PROGRAM,
        CompileOptions {
            target: Some("riscv64-elf".to_string()),
            target_profile: Some("ckb".to_string()),
            primitive_compat: Some("0.18".to_string()),
            ..CompileOptions::default()
        },
    )
    .expect("0.18 ScriptRef read program should assemble to ELF");
    assert!(!elf.artifact_bytes.is_empty());
}

#[test]
fn v0_18_cell_data_le_decoders_lower_to_fail_closed_ckb_helpers() {
    let result = compile(
        CELL_DATA_DECODE_PROGRAM,
        CompileOptions {
            target: Some("riscv64-asm".to_string()),
            target_profile: Some("ckb".to_string()),
            primitive_compat: Some("0.18".to_string()),
            ..CompileOptions::default()
        },
    )
    .expect("cell data LE decoder program should compile");

    let assembly = std::str::from_utf8(&result.artifact_bytes).expect("assembly utf-8");
    for helper in ["__ckb_cell_data_u32_le", "__ckb_cell_data_u64_le"] {
        assert!(assembly.contains(&format!(".global {helper}")), "missing helper {helper}:\n{assembly}");
    }
    assert!(
        assembly.contains("little-endian u32 read via LOAD_CELL_DATA")
            && assembly.contains("little-endian u64 read via LOAD_CELL_DATA")
            && assembly.contains("scalar runtime helper status check (a1 == 0)"),
        "cell data decoders must use fail-closed LOAD_CELL_DATA helpers:\n{assembly}"
    );

    let features = &result.metadata.runtime.ckb_runtime_features;
    assert!(features.contains(&"ckb-source-cell-fields".to_string()), "{features:?}");
    assert!(features.contains(&"ckb-cell-data-decode".to_string()), "{features:?}");

    let accesses = result
        .metadata
        .runtime
        .ckb_runtime_accesses
        .iter()
        .map(|access| (access.operation.as_str(), access.syscall.as_str(), access.source.as_str()))
        .collect::<Vec<_>>();
    assert!(accesses.contains(&("cell-data-u32-le", "LOAD_CELL_DATA", "SourceView")), "{accesses:?}");
    assert!(accesses.contains(&("cell-data-u64-le", "LOAD_CELL_DATA", "SourceView")), "{accesses:?}");
}

#[test]
fn v0_18_cell_data_u64_binary_guards_preserve_runtime_operands() {
    let result = compile(
        CELL_DATA_U64_BINARY_GUARD_PROGRAM,
        CompileOptions {
            target: Some("riscv64-asm".to_string()),
            target_profile: Some("ckb".to_string()),
            primitive_compat: Some("0.18".to_string()),
            ..CompileOptions::default()
        },
    )
    .expect("cell data u64 binary guard program should compile");

    let assembly = std::str::from_utf8(&result.artifact_bytes).expect("assembly utf-8");
    assert!(
        assembly.contains("# cellscript abi: expected field Header.old_nonce offset=0 size=8")
            && assembly.contains("# cellscript abi: expected field Header.new_nonce offset=8 size=8"),
        "schema-sourced u64 fields must be loaded for scalar binary guards:\n{assembly}"
    );
    assert!(
        assembly.contains("add t0, t0, t1"),
        "runtime-read u64 addition must be generated from loaded scalar operands:\n{assembly}"
    );
    assert!(
        assembly.contains("sltu t0, t1, t0"),
        "u64 greater-than guards must lower as unsigned comparisons after preserving both operands:\n{assembly}"
    );
}

#[test]
fn v0_18_cell_data_le_decoders_run_in_ckb_vm() {
    let type_elf = compile_source_to_elf(CELL_DATA_DECODE_PROGRAM, "inspect");
    let always_success_elf = compile_source_to_elf(ALWAYS_SUCCESS_LOCK_PROGRAM, "always_success");

    fn run_with_data(type_elf: &[u8], always_success_elf: &[u8], data: Bytes) -> Result<u64, String> {
        let mut context = Context::new_with_deterministic_rng();
        let always_success_out_point = context.deploy_cell(Bytes::copy_from_slice(always_success_elf));
        let type_out_point = context.deploy_cell(Bytes::copy_from_slice(type_elf));
        let lock = context.build_script(&always_success_out_point, Bytes::default()).expect("always success lock");
        let type_script = context.build_script(&type_out_point, Bytes::default()).expect("type script under test");

        let input = context.create_cell(
            packed::CellOutput::new_builder()
                .capacity::<packed::Uint64>(100_000_000_000u64.pack())
                .lock(lock.clone())
                .type_(packed::ScriptOpt::from(type_script))
                .build(),
            data,
        );
        let output = packed::CellOutput::new_builder().capacity::<packed::Uint64>(100_000_000_000u64.pack()).lock(lock).build();
        let tx = TransactionBuilder::default()
            .input(packed::CellInput::new_builder().previous_output(input).build())
            .output(output)
            .output_data(Bytes::default().pack())
            .witness(Bytes::default().pack())
            .build();
        let tx = context.complete_tx(tx);
        context.verify_tx(&tx, 10_000_000).map_err(|err| format!("{err:?}"))
    }

    let mut good = Vec::new();
    good.extend_from_slice(&7u32.to_le_bytes());
    good.extend_from_slice(&123_456_789u64.to_le_bytes());
    let pass_cycles = run_with_data(&type_elf, &always_success_elf, Bytes::from(good)).expect("LE data decoder pass");
    assert!(pass_cycles > 0);

    let reject = run_with_data(&type_elf, &always_success_elf, Bytes::from(vec![7, 0, 0, 0, 1, 2, 3]))
        .expect_err("short u64 data read must fail closed");
    assert!(reject.contains("error") || reject.contains("ValidationFailure"), "{reject}");
}

#[test]
fn v0_18_myelin_package_commitment_has_typed_cell_metadata_and_ckb_vm_rejects_tamper() {
    let type_result = compile(
        MYELIN_PACKAGE_COMMITMENT_TYPE_PROGRAM,
        CompileOptions {
            target: Some("riscv64-elf".to_string()),
            target_profile: Some("typed-cell".to_string()),
            primitive_compat: Some("0.18".to_string()),
            ..CompileOptions::default()
        },
    )
    .unwrap_or_else(|err| panic!("Myelin package commitment script should compile: {}", err.message));
    assert_eq!(type_result.metadata.target_profile.name, "typed-cell");
    let package_type =
        type_result.metadata.types.iter().find(|ty| ty.name == "PackageCommitment").expect("PackageCommitment metadata");
    let typed_cell = package_type.typed_cell.as_ref().expect("typed-cell metadata");
    assert_eq!(typed_cell.conflict_key, "field(package_id)");
    assert_eq!(typed_cell.conflict_key_fields, vec!["package_id"]);

    let ckb_result = compile(
        MYELIN_PACKAGE_COMMITMENT_TYPE_PROGRAM,
        CompileOptions {
            target: Some("riscv64-elf".to_string()),
            target_profile: Some("ckb".to_string()),
            primitive_compat: Some("0.18".to_string()),
            ..CompileOptions::default()
        },
    )
    .unwrap_or_else(|err| panic!("CKB package commitment script should compile: {}", err.message));
    let action = ckb_result
        .metadata
        .actions
        .iter()
        .find(|action| action.name == "verify_package_commitment")
        .expect("verify_package_commitment action metadata");
    let type_elf = cellscript::strip_vm_abi_trailer(&ckb_result.artifact_bytes).to_vec();
    let always_success_elf = compile_source_to_elf(ALWAYS_SUCCESS_LOCK_PROGRAM, "always_success");

    fn run_carrier(type_elf: &[u8], always_success_elf: &[u8], output_data: Bytes, witness: Bytes) -> Result<u64, String> {
        let mut context = Context::new_with_deterministic_rng();
        let always_success_out_point = context.deploy_cell(Bytes::copy_from_slice(always_success_elf));
        let type_out_point = context.deploy_cell(Bytes::copy_from_slice(type_elf));
        let lock = context.build_script(&always_success_out_point, Bytes::default()).expect("always success lock");
        let type_script = context.build_script(&type_out_point, Bytes::default()).expect("package commitment type script");

        let input = context.create_cell(
            packed::CellOutput::new_builder().capacity::<packed::Uint64>(100_000_000_000u64.pack()).lock(lock.clone()).build(),
            Bytes::default(),
        );
        let output = packed::CellOutput::new_builder()
            .capacity::<packed::Uint64>(100_000_000_000u64.pack())
            .lock(lock)
            .type_(packed::ScriptOpt::from(type_script))
            .build();
        let tx = TransactionBuilder::default()
            .input(packed::CellInput::new_builder().previous_output(input).build())
            .output(output)
            .output_data(output_data.pack())
            .witness(witness.pack())
            .build();
        let tx = context.complete_tx(tx);
        context.verify_tx(&tx, 10_000_000).map_err(|err| format!("{err:?}"))
    }

    let package_commitment =
        Bytes::from(hex::decode("c0b0941ff220975d4a43f186df95459368046f3128e8e9d31f393bc371c9d02e").expect("package commitment hex"));
    let expected_words = package_commitment
        .chunks_exact(8)
        .map(|word| u64::from_le_bytes(word.try_into().expect("8-byte commitment word")))
        .collect::<Vec<_>>();
    let witness = action
        .entry_witness_args(&[
            cellscript::EntryWitnessArg::U64(expected_words[0]),
            cellscript::EntryWitnessArg::U64(expected_words[1]),
            cellscript::EntryWitnessArg::U64(expected_words[2]),
            cellscript::EntryWitnessArg::U64(expected_words[3]),
        ])
        .expect("package commitment entry witness");

    let pass_cycles = run_carrier(&type_elf, &always_success_elf, package_commitment.clone(), Bytes::from(witness.clone()))
        .expect("matching package commitment carrier should pass");
    assert!(pass_cycles > 0);

    let mut tampered_commitment = package_commitment.to_vec();
    tampered_commitment[31] ^= 0x01;
    let reject = run_carrier(&type_elf, &always_success_elf, Bytes::from(tampered_commitment), Bytes::from(witness))
        .expect_err("tampered package commitment carrier should fail");
    assert!(reject.contains("error") || reject.contains("ValidationFailure"), "{reject}");
}

#[test]
fn v0_18_generic_package_commitment_binds_data_hash_to_type_args_in_ckb_vm() {
    let type_result = compile(
        MYELIN_GENERIC_PACKAGE_COMMITMENT_TYPE_PROGRAM,
        CompileOptions {
            target: Some("riscv64-elf".to_string()),
            target_profile: Some("ckb".to_string()),
            primitive_compat: Some("0.18".to_string()),
            ..CompileOptions::default()
        },
    )
    .unwrap_or_else(|err| panic!("generic package commitment script should compile: {}", err.message));
    let type_elf = cellscript::strip_vm_abi_trailer(&type_result.artifact_bytes).to_vec();
    let always_success_elf = compile_source_to_elf(ALWAYS_SUCCESS_LOCK_PROGRAM, "always_success");

    fn run_carrier(
        type_elf: &[u8],
        always_success_elf: &[u8],
        type_args: Bytes,
        output_data: Bytes,
        witness: Bytes,
    ) -> Result<u64, String> {
        let mut context = Context::new_with_deterministic_rng();
        let always_success_out_point = context.deploy_cell(Bytes::copy_from_slice(always_success_elf));
        let type_out_point = context.deploy_cell(Bytes::copy_from_slice(type_elf));
        let lock = context.build_script(&always_success_out_point, Bytes::default()).expect("always success lock");
        let type_script = context.build_script(&type_out_point, type_args).expect("generic package commitment type script");

        let input = context.create_cell(
            packed::CellOutput::new_builder().capacity::<packed::Uint64>(300_000_000_000u64.pack()).lock(lock.clone()).build(),
            Bytes::default(),
        );
        let output = packed::CellOutput::new_builder()
            .capacity::<packed::Uint64>(200_000_000_000u64.pack())
            .lock(lock)
            .type_(packed::ScriptOpt::from(type_script))
            .build();
        let tx = TransactionBuilder::default()
            .input(packed::CellInput::new_builder().previous_output(input).build())
            .output(output)
            .output_data(output_data.pack())
            .witness(witness.pack())
            .build();
        let tx = context.complete_tx(tx);
        context.verify_tx(&tx, 10_000_000).map_err(|err| format!("{err:?}"))
    }

    let package_commitment =
        Bytes::from(hex::decode("c0b0941ff220975d4a43f186df95459368046f3128e8e9d31f393bc371c9d02e").expect("package commitment hex"));
    let type_args = Bytes::from(ckb_blake2b256(&package_commitment).to_vec());
    let pass_cycles = run_carrier(
        &type_elf,
        &always_success_elf,
        type_args.clone(),
        package_commitment.clone(),
        Bytes::default(),
    )
    .expect("generic package commitment carrier should pass when data hash matches type args");
    assert!(pass_cycles > 0);

    let mut tampered_commitment = package_commitment.to_vec();
    tampered_commitment[0] ^= 0x01;
    let reject = run_carrier(
        &type_elf,
        &always_success_elf,
        type_args,
        Bytes::from(tampered_commitment),
        Bytes::default(),
    )
    .expect_err("generic package commitment carrier should reject tampered data under unchanged type args");
    assert!(reject.contains("error") || reject.contains("ValidationFailure"), "{reject}");
}

#[test]
fn v0_18_myelin_da_and_settlement_carriers_bind_compact_payloads_to_type_args_in_ckb_vm() {
    fn compile_myelin_payload_script(
        source: &str,
        entry: &str,
        type_name: &str,
        conflict_key: &str,
        conflict_key_field: &str,
    ) -> Vec<u8> {
        let typed_result = compile(
            source,
            CompileOptions {
                target: Some("riscv64-elf".to_string()),
                target_profile: Some("typed-cell".to_string()),
                primitive_compat: Some("0.18".to_string()),
                ..CompileOptions::default()
            },
        )
        .unwrap_or_else(|err| panic!("{type_name} typed-cell script should compile: {}", err.message));
        assert_eq!(typed_result.metadata.target_profile.name, "typed-cell");
        let cell_type = typed_result.metadata.types.iter().find(|ty| ty.name == type_name).expect("Myelin type metadata");
        let typed_cell = cell_type.typed_cell.as_ref().expect("Myelin typed-cell metadata");
        assert_eq!(typed_cell.conflict_key, conflict_key);
        assert_eq!(typed_cell.conflict_key_fields, vec![conflict_key_field]);

        let ckb_result = compile(
            source,
            CompileOptions {
                target: Some("riscv64-elf".to_string()),
                target_profile: Some("ckb".to_string()),
                primitive_compat: Some("0.18".to_string()),
                ..CompileOptions::default()
            },
        )
        .unwrap_or_else(|err| panic!("{type_name} CKB script should compile: {}", err.message));
        assert!(
            ckb_result.metadata.actions.iter().any(|action| action.name == entry),
            "{entry} action metadata missing"
        );
        cellscript::strip_vm_abi_trailer(&ckb_result.artifact_bytes).to_vec()
    }

    fn myelin_payload_witness(source: &str, entry: &str, args: &[EntryWitnessArg]) -> Bytes {
        for arg in args {
            if let EntryWitnessArg::Bytes(bytes) = arg {
                assert!(
                    bytes.len() == 64,
                    "{entry} fixed byte witness arg has unexpected length {}",
                    bytes.len()
                );
            }
        }
        let ckb_result = compile(
            source,
            CompileOptions {
                target: Some("riscv64-elf".to_string()),
                target_profile: Some("ckb".to_string()),
                primitive_compat: Some("0.18".to_string()),
                ..CompileOptions::default()
            },
        )
        .unwrap_or_else(|err| panic!("{entry} CKB script should compile for witness metadata: {}", err.message));
        let action = ckb_result
            .metadata
            .actions
            .iter()
            .find(|action| action.name == entry)
            .unwrap_or_else(|| panic!("{entry} action metadata missing"));
        Bytes::from(
            action
                .entry_witness_args(args)
                .unwrap_or_else(|err| panic!("{entry} witness should encode: {}", err.message)),
        )
    }

    #[derive(Clone)]
    struct EvidenceInput<'a> {
        type_elf: &'a [u8],
        type_args: Vec<u8>,
        data: Bytes,
        include_cell_dep: bool,
        distinct_lock: bool,
        expected_code_hash: Option<[u8; 32]>,
        expected_type_args: Option<Vec<u8>>,
    }

    #[derive(Clone)]
    struct AuthorityInput {
        data: Bytes,
        distinct_lock: bool,
    }

    fn final_da_evidence<'a>(da_final_elf: &'a [u8], settlement_payload: &Bytes) -> EvidenceInput<'a> {
        let mut da_payload = (0..160).map(|offset| 0x31u8.wrapping_add(offset as u8)).collect::<Vec<_>>();
        da_payload[..32].copy_from_slice(&settlement_payload[64..96]);
        let da_payload = Bytes::from(da_payload);
        let mut da_type_args = ckb_blake2b256(&da_payload).to_vec();
        da_type_args.extend_from_slice(&da_payload[..32]);
        EvidenceInput {
            type_elf: da_final_elf,
            type_args: da_type_args,
            data: da_payload,
            include_cell_dep: true,
            distinct_lock: false,
            expected_code_hash: None,
            expected_type_args: None,
        }
    }

    fn settlement_authority_for_payload(payload: &Bytes) -> AuthorityInput {
        let mut data = Vec::with_capacity(32 * 6);
        data.extend_from_slice(&payload[..32]);
        data.extend_from_slice(&[0xb0; 32]);
        data.extend_from_slice(&[0xc1; 32]);
        data.extend_from_slice(&[0xd2; 32]);
        data.extend_from_slice(&[0xe3; 32]);
        data.extend_from_slice(&[0xf4; 32]);
        AuthorityInput {
            data: Bytes::from(data),
            distinct_lock: false,
        }
    }

    fn settlement_final_type_args(payload: &Bytes, authority: &AuthorityInput) -> Vec<u8> {
        let mut type_args = authority.data[32..64].to_vec();
        type_args.extend_from_slice(&ckb_blake2b256(payload));
        type_args
    }

    fn run_myelin_payload_script(
        source: &str,
        entry: &str,
        type_elf: &[u8],
        always_success_elf: &[u8],
        type_args: Vec<u8>,
        witness_type_args: Vec<u8>,
        output_data: Bytes,
        evidence_input: Option<EvidenceInput<'_>>,
        authority_input: Option<AuthorityInput>,
        duplicate_group_output: bool,
        competing_final_output_type_args: Option<Vec<u8>>,
        same_type_group_input: bool,
    ) -> Result<u64, String> {
        let mut context = Context::new_with_deterministic_rng();
        let always_success_out_point = context.deploy_cell(Bytes::copy_from_slice(always_success_elf));
        let type_out_point = context.deploy_cell(Bytes::copy_from_slice(type_elf));
        let lock = context.build_script(&always_success_out_point, Bytes::default()).expect("always success lock");
        let type_script = context
            .build_script_with_hash_type(&type_out_point, ScriptHashType::Data2, Bytes::from(type_args))
            .expect("Myelin payload type script");
        let mut witness_args = vec![EntryWitnessArg::Bytes(witness_type_args)];

        let funding_input = context.create_cell(
            packed::CellOutput::new_builder().capacity::<packed::Uint64>(500_000_000_000u64.pack()).lock(lock.clone()).build(),
            Bytes::default(),
        );
        let output = packed::CellOutput::new_builder()
            .capacity::<packed::Uint64>(400_000_000_000u64.pack())
            .lock(lock.clone())
            .type_(packed::ScriptOpt::from(type_script.clone()))
            .build();
        let mut tx_builder = TransactionBuilder::default()
            .input(packed::CellInput::new_builder().previous_output(funding_input).build())
            .output(output)
            .output_data(output_data.clone().pack());
        if let Some(authority) = authority_input {
            let authority_lock = if authority.distinct_lock {
                context
                    .build_script(&always_success_out_point, Bytes::from_static(b"wrong-settlement-authority"))
                    .expect("distinct settlement authority lock")
            } else {
                lock.clone()
            };
            let authority_output = packed::CellOutput::new_builder()
                .capacity::<packed::Uint64>(400_000_000_000u64.pack())
                .lock(authority_lock)
                .build();
            let authority_cell = context.create_cell(authority_output, authority.data);
            tx_builder = tx_builder.input(packed::CellInput::new_builder().previous_output(authority_cell).build());
        }
        if let Some(evidence) = evidence_input {
            let evidence_type_out_point = context.deploy_cell(Bytes::copy_from_slice(evidence.type_elf));
            let evidence_type_script = context
                .build_script_with_hash_type(&evidence_type_out_point, ScriptHashType::Data2, Bytes::from(evidence.type_args.clone()))
                .expect("Myelin DA evidence type script");
            let evidence_type_code_hash: [u8; 32] = evidence_type_script.code_hash().unpack();
            witness_args.push(EntryWitnessArg::Hash(evidence.expected_code_hash.unwrap_or(evidence_type_code_hash)));
            witness_args.push(EntryWitnessArg::Bytes(evidence.expected_type_args.clone().unwrap_or_else(|| evidence.type_args.clone())));
            if evidence.include_cell_dep {
                let evidence_lock = if evidence.distinct_lock {
                    context
                        .build_script(&always_success_out_point, Bytes::from_static(b"wrong-da-owner"))
                        .expect("distinct DA evidence lock")
                } else {
                    lock.clone()
                };
                let evidence_output = packed::CellOutput::new_builder()
                    .capacity::<packed::Uint64>(400_000_000_000u64.pack())
                    .lock(evidence_lock)
                    .type_(packed::ScriptOpt::from(evidence_type_script))
                    .build();
                let evidence_cell_dep = context.create_cell(evidence_output, evidence.data);
                tx_builder = tx_builder.cell_dep(packed::CellDep::new_builder().out_point(evidence_cell_dep).build());
            }
        }
        for arg in &witness_args {
            if let EntryWitnessArg::Bytes(bytes) = arg {
                assert_eq!(bytes.len(), 64, "{entry} runner witness arg must be raw 64 bytes");
            }
        }
        let witness = myelin_payload_witness(source, entry, &witness_args);
        tx_builder = tx_builder.witness(witness.pack());
        if same_type_group_input {
            let typed_input_output = packed::CellOutput::new_builder()
                .capacity::<packed::Uint64>(400_000_000_000u64.pack())
                .lock(lock.clone())
                .type_(packed::ScriptOpt::from(type_script.clone()))
                .build();
            let typed_input = context.create_cell(typed_input_output, output_data.clone());
            tx_builder = tx_builder.input(packed::CellInput::new_builder().previous_output(typed_input).build());
        }
        if duplicate_group_output {
            let duplicate_output = packed::CellOutput::new_builder()
                .capacity::<packed::Uint64>(400_000_000_000u64.pack())
                .lock(lock.clone())
                .type_(packed::ScriptOpt::from(type_script.clone()))
                .build();
            tx_builder = tx_builder.output(duplicate_output).output_data(output_data.clone().pack());
        }
        if let Some(competing_type_args) = competing_final_output_type_args {
            let competing_type_script = context
                .build_script_with_hash_type(&type_out_point, ScriptHashType::Data2, Bytes::from(competing_type_args))
                .expect("competing final settlement type script");
            let competing_output = packed::CellOutput::new_builder()
                .capacity::<packed::Uint64>(400_000_000_000u64.pack())
                .lock(lock)
                .type_(packed::ScriptOpt::from(competing_type_script))
                .build();
            tx_builder = tx_builder.output(competing_output).output_data(output_data.pack());
        }
        let tx = tx_builder.build();
        let tx = context.complete_tx(tx);
        context.verify_tx(&tx, 10_000_000).map_err(|err| format!("{err:?}"))
    }

    let always_success_elf = compile_source_to_elf(ALWAYS_SUCCESS_LOCK_PROGRAM, "always_success");
    let da_carrier_elf = compile_myelin_payload_script(
        MYELIN_DA_ANCHOR_CARRIER_TYPE_PROGRAM,
        "verify_da_anchor_carrier",
        "DaAnchorCarrier",
        "field(da_manifest_hash)",
        "da_manifest_hash",
    );
    let settlement_carrier_elf = compile_myelin_payload_script(
        MYELIN_SETTLEMENT_CARRIER_TYPE_PROGRAM,
        "verify_settlement_carrier",
        "SettlementCarrier",
        "field(intent_hash)",
        "intent_hash",
    );
    let da_final_elf = compile_myelin_payload_script(
        MYELIN_DA_ANCHOR_FINAL_TYPE_PROGRAM,
        "verify_final_da_publication",
        "DaAnchorFinal",
        "field(da_manifest_hash)",
        "da_manifest_hash",
    );
    let settlement_final_elf = compile_myelin_payload_script(
        MYELIN_SETTLEMENT_FINAL_TYPE_PROGRAM,
        "verify_final_settlement",
        "SettlementFinal",
        "field(intent_hash)",
        "intent_hash",
    );

    for (label, source, entry, type_elf, seed, final_script_creation_only) in [
        (
            "DA anchor carrier",
            MYELIN_DA_ANCHOR_CARRIER_TYPE_PROGRAM,
            "verify_da_anchor_carrier",
            da_carrier_elf.as_slice(),
            0x11u8,
            false,
        ),
        (
            "settlement carrier",
            MYELIN_SETTLEMENT_CARRIER_TYPE_PROGRAM,
            "verify_settlement_carrier",
            settlement_carrier_elf.as_slice(),
            0x71u8,
            false,
        ),
        (
            "final DA publication",
            MYELIN_DA_ANCHOR_FINAL_TYPE_PROGRAM,
            "verify_final_da_publication",
            da_final_elf.as_slice(),
            0x31u8,
            true,
        ),
        (
            "final settlement",
            MYELIN_SETTLEMENT_FINAL_TYPE_PROGRAM,
            "verify_final_settlement",
            settlement_final_elf.as_slice(),
            0x91u8,
            true,
        ),
    ] {
        let payload = Bytes::from((0..160).map(|offset| seed.wrapping_add(offset as u8)).collect::<Vec<_>>());
        let authority_input = if label == "final settlement" {
            Some(settlement_authority_for_payload(&payload))
        } else {
            None
        };
        let mut type_args = ckb_blake2b256(&payload).to_vec();
        if let Some(authority) = authority_input.as_ref() {
            type_args = settlement_final_type_args(&payload, authority);
        } else {
            type_args.extend_from_slice(&payload[..32]);
        }
        assert_eq!(type_args.len(), 64, "{label} test type args must be raw 64 bytes");
        let type_args = type_args;
        let evidence_input = if label == "final settlement" {
            Some(final_da_evidence(da_final_elf.as_slice(), &payload))
        } else {
            None
        };
        let pass_cycles = run_myelin_payload_script(
            source,
            entry,
            type_elf,
            &always_success_elf,
            type_args.clone(),
            type_args.clone(),
            payload.clone(),
            evidence_input.clone(),
            authority_input.clone(),
            false,
            None,
            false,
        )
        .unwrap_or_else(|err| {
            panic!("{label} compact payload script should pass when payload data hash and identity match type args: {err}")
        });
        assert!(pass_cycles > 0);

        if label == "final settlement" {
            let canonical_evidence = evidence_input.clone().expect("final settlement evidence");
            let canonical_authority = authority_input.clone().expect("final settlement authority");

            let reject_missing_authority = run_myelin_payload_script(
                source,
                entry,
                type_elf,
                &always_success_elf,
                type_args.clone(),
                type_args.clone(),
                payload.clone(),
                Some(canonical_evidence.clone()),
                None,
                false,
                None,
                false,
            )
            .expect_err("final settlement must reject when the settlement authority input is absent");
            assert!(
                reject_missing_authority.contains("error") || reject_missing_authority.contains("ValidationFailure"),
                "{reject_missing_authority}"
            );

            let mut wrong_authority_data = canonical_authority.clone();
            let mut wrong_authority_bytes = wrong_authority_data.data.to_vec();
            wrong_authority_bytes[0] ^= 0x01;
            wrong_authority_data.data = Bytes::from(wrong_authority_bytes);
            let reject_wrong_authority_data = run_myelin_payload_script(
                source,
                entry,
                type_elf,
                &always_success_elf,
                type_args.clone(),
                type_args.clone(),
                payload.clone(),
                Some(canonical_evidence.clone()),
                Some(wrong_authority_data),
                false,
                None,
                false,
            )
            .expect_err("final settlement must reject authority input data that does not equal the settlement intent hash");
            assert!(
                reject_wrong_authority_data.contains("error") || reject_wrong_authority_data.contains("ValidationFailure"),
                "{reject_wrong_authority_data}"
            );

            let mut wrong_authority_session = canonical_authority.clone();
            let mut wrong_authority_session_bytes = wrong_authority_session.data.to_vec();
            wrong_authority_session_bytes[32] ^= 0x01;
            wrong_authority_session.data = Bytes::from(wrong_authority_session_bytes);
            let reject_wrong_session_identity = run_myelin_payload_script(
                source,
                entry,
                type_elf,
                &always_success_elf,
                type_args.clone(),
                type_args.clone(),
                payload.clone(),
                Some(canonical_evidence.clone()),
                Some(wrong_authority_session),
                false,
                None,
                false,
            )
            .expect_err("final settlement must reject authority data with a mismatched session identity");
            assert!(
                reject_wrong_session_identity.contains("error")
                    || reject_wrong_session_identity.contains("ValidationFailure"),
                "{reject_wrong_session_identity}"
            );

            let mut wrong_authority_lock = canonical_authority.clone();
            wrong_authority_lock.distinct_lock = true;
            let reject_wrong_authority_lock = run_myelin_payload_script(
                source,
                entry,
                type_elf,
                &always_success_elf,
                type_args.clone(),
                type_args.clone(),
                payload.clone(),
                Some(canonical_evidence.clone()),
                Some(wrong_authority_lock),
                false,
                None,
                false,
            )
            .expect_err("final settlement must reject an authority input controlled by a different lock");
            assert!(
                reject_wrong_authority_lock.contains("error") || reject_wrong_authority_lock.contains("ValidationFailure"),
                "{reject_wrong_authority_lock}"
            );

            let mut missing_da_dep = canonical_evidence.clone();
            missing_da_dep.include_cell_dep = false;
            let reject_missing_da_dep = run_myelin_payload_script(
                source,
                entry,
                type_elf,
                &always_success_elf,
                type_args.clone(),
                type_args.clone(),
                payload.clone(),
                Some(missing_da_dep),
                authority_input.clone(),
                false,
                None,
                false,
            )
            .expect_err("final settlement must reject when the final DA publication CellDep is absent");
            assert!(
                reject_missing_da_dep.contains("error") || reject_missing_da_dep.contains("ValidationFailure"),
                "{reject_missing_da_dep}"
            );

            let mut wrong_da_payload = canonical_evidence.data.to_vec();
            wrong_da_payload[0] ^= 0x01;
            let wrong_da_payload = Bytes::from(wrong_da_payload);
            let mut wrong_da_type_args = ckb_blake2b256(&wrong_da_payload).to_vec();
            wrong_da_type_args.extend_from_slice(&wrong_da_payload[..32]);
            let reject_wrong_da_manifest = run_myelin_payload_script(
                source,
                entry,
                type_elf,
                &always_success_elf,
                type_args.clone(),
                type_args.clone(),
                payload.clone(),
                Some(EvidenceInput {
                    type_elf: da_final_elf.as_slice(),
                    type_args: wrong_da_type_args,
                    data: wrong_da_payload,
                    include_cell_dep: true,
                    distinct_lock: false,
                    expected_code_hash: None,
                    expected_type_args: None,
                }),
                authority_input.clone(),
                false,
                None,
                false,
            )
            .expect_err("final settlement must reject DA evidence bound to a different manifest hash");
            assert!(
                reject_wrong_da_manifest.contains("error") || reject_wrong_da_manifest.contains("ValidationFailure"),
                "{reject_wrong_da_manifest}"
            );

            let mut wrong_expected_da_args = canonical_evidence.clone();
            let mut expected_type_args = wrong_expected_da_args.type_args.clone();
            expected_type_args[63] ^= 0x01;
            wrong_expected_da_args.expected_type_args = Some(expected_type_args);
            let reject_wrong_da_type_args = run_myelin_payload_script(
                source,
                entry,
                type_elf,
                &always_success_elf,
                type_args.clone(),
                type_args.clone(),
                payload.clone(),
                Some(wrong_expected_da_args),
                authority_input.clone(),
                false,
                None,
                false,
            )
            .expect_err("final settlement must reject a witness that names the wrong final DA type args");
            assert!(
                reject_wrong_da_type_args.contains("error") || reject_wrong_da_type_args.contains("ValidationFailure"),
                "{reject_wrong_da_type_args}"
            );

            let mut wrong_expected_code_hash = canonical_evidence;
            wrong_expected_code_hash.expected_code_hash = Some([0x42; 32]);
            let reject_wrong_da_code_hash = run_myelin_payload_script(
                source,
                entry,
                type_elf,
                &always_success_elf,
                type_args.clone(),
                type_args.clone(),
                payload.clone(),
                Some(wrong_expected_code_hash),
                authority_input.clone(),
                false,
                None,
                false,
            )
            .expect_err("final settlement must reject a witness that names the wrong final DA verifier code hash");
            assert!(
                reject_wrong_da_code_hash.contains("error") || reject_wrong_da_code_hash.contains("ValidationFailure"),
                "{reject_wrong_da_code_hash}"
            );

            let mut wrong_da_lock = evidence_input.clone().expect("final settlement evidence");
            wrong_da_lock.distinct_lock = true;
            let reject_wrong_da_lock = run_myelin_payload_script(
                source,
                entry,
                type_elf,
                &always_success_elf,
                type_args.clone(),
                type_args.clone(),
                payload.clone(),
                Some(wrong_da_lock),
                authority_input.clone(),
                false,
                None,
                false,
            )
            .expect_err("final settlement must reject DA evidence controlled by a different lock");
            assert!(
                reject_wrong_da_lock.contains("error") || reject_wrong_da_lock.contains("ValidationFailure"),
                "{reject_wrong_da_lock}"
            );
        }

        let reject_duplicate = run_myelin_payload_script(
            source,
            entry,
            type_elf,
            &always_success_elf,
            type_args.clone(),
            type_args.clone(),
            payload.clone(),
            evidence_input.clone(),
            authority_input.clone(),
            true,
            None,
            false,
        )
        .expect_err(&format!("{label} compact payload script must reject a duplicate group output"));
        assert!(reject_duplicate.contains("error") || reject_duplicate.contains("ValidationFailure"), "{reject_duplicate}");
        if label == "final settlement" {
            let mut competing_payload = payload.clone().to_vec();
            competing_payload[128] ^= 0x01;
            let competing_payload = Bytes::from(competing_payload);
            let competing_type_args = settlement_final_type_args(
                &competing_payload,
                authority_input.as_ref().expect("final settlement authority"),
            );
            let reject_competing_settlement = run_myelin_payload_script(
                source,
                entry,
                type_elf,
                &always_success_elf,
                type_args.clone(),
                type_args.clone(),
                payload.clone(),
                evidence_input.clone(),
                authority_input.clone(),
                false,
                Some(competing_type_args),
                false,
            )
            .expect_err("final settlement must reject a competing final settlement output under the same verifier");
            assert!(
                reject_competing_settlement.contains("error")
                    || reject_competing_settlement.contains("ValidationFailure"),
                "{reject_competing_settlement}"
            );
        }
        if final_script_creation_only {
            let reject_same_type_input = run_myelin_payload_script(
                source,
                entry,
                type_elf,
                &always_success_elf,
                type_args.clone(),
                type_args.clone(),
                payload.clone(),
                evidence_input.clone(),
                authority_input.clone(),
                false,
                None,
                true,
            )
            .expect_err(&format!("{label} final script must reject a same-type group input"));
            assert!(
                reject_same_type_input.contains("error") || reject_same_type_input.contains("ValidationFailure"),
                "{reject_same_type_input}"
            );
        }

        let mut tampered_payload = payload.to_vec();
        tampered_payload[159] ^= 0x01;
        let reject = run_myelin_payload_script(
            source,
            entry,
            type_elf,
            &always_success_elf,
            type_args.clone(),
            type_args.clone(),
            Bytes::from(tampered_payload),
            evidence_input.clone(),
                authority_input.clone(),
                false,
                None,
                false,
            )
        .expect_err(&format!("{label} tampered compact payload must be rejected"));
        assert!(reject.contains("error") || reject.contains("ValidationFailure"), "{reject}");

        let mut wrong_identity_args = type_args.clone();
        wrong_identity_args[63] ^= 0x01;
        let reject_identity = run_myelin_payload_script(
            source,
            entry,
            type_elf,
            &always_success_elf,
            wrong_identity_args,
            type_args.clone(),
            payload.clone(),
            evidence_input.clone(),
            authority_input.clone(),
            false,
            None,
            false,
        )
        .expect_err(&format!("{label} compact payload script must reject mismatched identity args"));
        assert!(reject_identity.contains("error") || reject_identity.contains("ValidationFailure"), "{reject_identity}");

        let mut extended_type_args = type_args.clone();
        extended_type_args.extend_from_slice(&[0x42, 0x43]);
        let reject_extended = run_myelin_payload_script(
            source,
            entry,
            type_elf,
            &always_success_elf,
            extended_type_args,
            type_args.clone(),
            payload.clone(),
            evidence_input.clone(),
            authority_input.clone(),
            false,
            None,
            false,
        )
        .expect_err(&format!("{label} compact payload script must reject non-canonical extended type args"));
        assert!(reject_extended.contains("error") || reject_extended.contains("ValidationFailure"), "{reject_extended}");

        let mut zero_field_payload = payload.to_vec();
        zero_field_payload[64..96].fill(0);
        let zero_field_payload = Bytes::from(zero_field_payload);
        let zero_field_type_args = if let Some(authority) = authority_input.as_ref() {
            settlement_final_type_args(&zero_field_payload, authority)
        } else {
            let mut args = ckb_blake2b256(&zero_field_payload).to_vec();
            args.extend_from_slice(&zero_field_payload[..32]);
            args
        };
        let reject_zero_field = run_myelin_payload_script(
            source,
            entry,
            type_elf,
            &always_success_elf,
            zero_field_type_args.clone(),
            zero_field_type_args,
            zero_field_payload,
            evidence_input.clone(),
            authority_input.clone(),
            false,
            None,
            false,
        )
        .expect_err(&format!("{label} compact payload script must reject self-consistent zero hash fields"));
        assert!(reject_zero_field.contains("error") || reject_zero_field.contains("ValidationFailure"), "{reject_zero_field}");

        let short_payload = Bytes::from(payload[..32].to_vec());
        let mut short_args = ckb_blake2b256(&short_payload).to_vec();
        short_args.extend_from_slice(&short_payload[..32]);
        let reject_short = run_myelin_payload_script(
            source,
            entry,
            type_elf,
            &always_success_elf,
            short_args.clone(),
            short_args,
            short_payload,
            evidence_input,
            authority_input,
            false,
            None,
            false,
        )
            .expect_err(&format!("{label} compact payload script must reject a short payload"));
        assert!(reject_short.contains("error") || reject_short.contains("ValidationFailure"), "{reject_short}");
    }
}

#[test]
fn v0_18_out_point_tx_hash_read_lowers_to_addressable_hash() {
    let result = compile(
        OUT_POINT_API_PROGRAM,
        CompileOptions {
            target: Some("riscv64-asm".to_string()),
            target_profile: Some("ckb".to_string()),
            primitive_compat: Some("0.18".to_string()),
            ..CompileOptions::default()
        },
    )
    .expect("OutPoint API program should compile");

    let assembly = std::str::from_utf8(&result.artifact_bytes).expect("assembly utf-8");
    for helper in [
        "__ckb_input_out_point_tx_hash",
        "__ckb_input_out_point_index",
        "__ckb_require_input_out_point_tx_hash",
        "__ckb_require_input_out_point",
    ] {
        assert!(assembly.contains(&format!(".global {helper}")), "missing helper {helper}:\n{assembly}");
    }
    assert!(
        assembly.contains("OutPoint full tx-hash read")
            && assembly.contains("load SourceView input OutPoint tx hash into addressable Hash")
            && assembly.contains("OutPoint full tx-hash + index requirement"),
        "OutPoint API must expose full tx-hash reads and binding checks:\n{assembly}"
    );

    let features = &result.metadata.runtime.ckb_runtime_features;
    assert!(features.contains(&"ckb-source-input-out-point".to_string()), "{features:?}");
    assert!(features.contains(&"ckb-source-cell-fields".to_string()), "{features:?}");

    let accesses = result
        .metadata
        .runtime
        .ckb_runtime_accesses
        .iter()
        .map(|access| (access.operation.as_str(), access.syscall.as_str(), access.source.as_str()))
        .collect::<Vec<_>>();
    assert!(accesses.contains(&("input-out-point-tx-hash-read", "LOAD_INPUT_BY_FIELD", "SourceView")), "{accesses:?}");
    assert!(accesses.contains(&("input-out-point-require", "LOAD_INPUT_BY_FIELD", "SourceView")), "{accesses:?}");
}

#[test]
fn v0_18_out_point_tx_hash_read_runs_in_ckb_vm() {
    let type_elf = compile_source_to_elf(OUT_POINT_API_PROGRAM, "inspect");
    let reject_type_elf = compile_source_to_elf(OUT_POINT_OUTPUT_REJECT_PROGRAM, "inspect");
    let always_success_elf = compile_source_to_elf(ALWAYS_SUCCESS_LOCK_PROGRAM, "always_success");

    fn run(type_elf: &[u8], always_success_elf: &[u8]) -> Result<u64, String> {
        let mut context = Context::new_with_deterministic_rng();
        let always_success_out_point = context.deploy_cell(Bytes::copy_from_slice(always_success_elf));
        let type_out_point = context.deploy_cell(Bytes::copy_from_slice(type_elf));
        let lock = context.build_script(&always_success_out_point, Bytes::default()).expect("always success lock");
        let type_script = context.build_script(&type_out_point, Bytes::default()).expect("type script under test");

        let input = context.create_cell(
            packed::CellOutput::new_builder()
                .capacity::<packed::Uint64>(100_000_000_000u64.pack())
                .lock(lock.clone())
                .type_(packed::ScriptOpt::from(type_script.clone()))
                .build(),
            Bytes::default(),
        );
        let output = packed::CellOutput::new_builder()
            .capacity::<packed::Uint64>(100_000_000_000u64.pack())
            .lock(lock)
            .type_(packed::ScriptOpt::from(type_script))
            .build();
        let tx = TransactionBuilder::default()
            .input(packed::CellInput::new_builder().previous_output(input).build())
            .output(output)
            .output_data(Bytes::default().pack())
            .witness(Bytes::default().pack())
            .build();
        let tx = context.complete_tx(tx);
        context.verify_tx(&tx, 10_000_000).map_err(|err| format!("{err:?}"))
    }

    let pass_cycles = run(&type_elf, &always_success_elf).expect("OutPoint tx-hash read should pass for input SourceView");
    assert!(pass_cycles > 0);

    let reject =
        run(&reject_type_elf, &always_success_elf).expect_err("OutPoint tx-hash read must reject non-input SourceView at runtime");
    assert!(reject.contains("error") || reject.contains("ValidationFailure"), "{reject}");
}

#[test]
fn v0_18_script_ref_property_surface_lowers_to_same_helpers() {
    let result = compile(
        SCRIPT_REF_PROPERTY_PROGRAM,
        CompileOptions {
            target: Some("riscv64-asm".to_string()),
            target_profile: Some("ckb".to_string()),
            primitive_compat: Some("0.18".to_string()),
            ..CompileOptions::default()
        },
    )
    .expect("0.18 ScriptRef property program should compile");

    let assembly = std::str::from_utf8(&result.artifact_bytes).expect("assembly utf-8");
    for helper in [
        "__ckb_cell_lock_code_hash",
        "__ckb_cell_type_code_hash",
        "__ckb_cell_lock_hash_type",
        "__ckb_cell_type_hash_type",
        "__ckb_cell_lock_args_empty",
        "__ckb_cell_type_args_empty",
        "__ckb_cell_lock_args_hash",
        "__ckb_cell_type_args_hash",
    ] {
        assert!(assembly.contains(&format!(".global {helper}")), "missing property helper {helper}:\n{assembly}");
    }
    assert!(
        assembly.contains("read-only ScriptRef Hash field") && assembly.contains("read-only ScriptRef scalar field"),
        "property ScriptRef reads must reuse the runtime extraction helpers:\n{assembly}"
    );

    let features = &result.metadata.runtime.ckb_runtime_features;
    assert!(features.contains(&"ckb-script-ref-read".to_string()), "{features:?}");
    assert!(features.contains(&"ckb-script-args-read".to_string()), "{features:?}");

    let accesses = result
        .metadata
        .runtime
        .ckb_runtime_accesses
        .iter()
        .map(|access| (access.operation.as_str(), access.syscall.as_str(), access.source.as_str()))
        .collect::<Vec<_>>();
    for operation in [
        "cell-lock-script-code-hash-read",
        "cell-type-script-code-hash-read",
        "cell-lock-script-hash-type-read",
        "cell-type-script-hash-type-read",
        "cell-lock-script-args-empty-read",
        "cell-type-script-args-empty-read",
        "cell-lock-script-args-hash-read",
        "cell-type-script-args-hash-read",
    ] {
        assert!(accesses.contains(&(operation, "LOAD_CELL_BY_FIELD", "SourceView")), "{accesses:?}");
    }

    let elf = compile(
        SCRIPT_REF_PROPERTY_PROGRAM,
        CompileOptions {
            target: Some("riscv64-elf".to_string()),
            target_profile: Some("ckb".to_string()),
            primitive_compat: Some("0.18".to_string()),
            ..CompileOptions::default()
        },
    )
    .expect("0.18 ScriptRef property program should assemble to ELF");
    assert!(!elf.artifact_bytes.is_empty());
}

#[test]
fn v0_18_script_construction_lowers_to_exact_script_requirements() {
    let result = compile(
        SCRIPT_CONSTRUCTION_PROGRAM,
        CompileOptions {
            target: Some("riscv64-asm".to_string()),
            target_profile: Some("ckb".to_string()),
            primitive_compat: Some("0.18".to_string()),
            ..CompileOptions::default()
        },
    )
    .expect("0.18 Script construction program should compile");

    let assembly = std::str::from_utf8(&result.artifact_bytes).expect("assembly utf-8");
    for helper in [
        "__ckb_cell_lock_code_hash",
        "__ckb_cell_lock_hash_type",
        "__ckb_require_cell_lock_script_hash_type",
        "__ckb_require_cell_lock_args_exact",
    ] {
        assert!(assembly.contains(&format!(".global {helper}")), "missing constructed Script helper {helper}:\n{assembly}");
    }
    assert!(
        assembly.contains("Script arbitrary exact args requirement")
            && assembly.contains("validates Molecule packed::Script args Bytes exactly"),
        "constructed Script args must lower to arbitrary exact args verification:\n{assembly}"
    );

    let features = &result.metadata.runtime.ckb_runtime_features;
    assert!(features.contains(&"ckb-script-construction".to_string()), "{features:?}");
    assert!(features.contains(&"ckb-script-identity-requirements".to_string()), "{features:?}");
    assert!(features.contains(&"ckb-script-args-requirements".to_string()), "{features:?}");

    let accesses = result
        .metadata
        .runtime
        .ckb_runtime_accesses
        .iter()
        .map(|access| (access.operation.as_str(), access.syscall.as_str(), access.source.as_str(), access.binding.as_str()))
        .collect::<Vec<_>>();
    assert!(
        accesses.contains(&(
            "cell-lock-script-exact-args-require",
            "LOAD_CELL_BY_FIELD",
            "SourceView",
            "script::require_cell_lock_matches",
        )),
        "{accesses:?}"
    );

    let elf = compile(
        SCRIPT_CONSTRUCTION_PROGRAM,
        CompileOptions {
            target: Some("riscv64-elf".to_string()),
            target_profile: Some("ckb".to_string()),
            primitive_compat: Some("0.18".to_string()),
            ..CompileOptions::default()
        },
    )
    .expect("0.18 Script construction program should assemble to ELF");
    assert!(!elf.artifact_bytes.is_empty());
}

#[test]
fn v0_18_script_value_encoding_matches_ckb_types_packed_script() {
    let code_hash = [7u8; 32];
    let args = vec![1u8, 3, 5, 7, 9, 11, 13];
    let value = CkbScriptValue::new(code_hash, CkbScriptHashTypeValue::Data1, CkbScriptArgsValue::exact(args.clone()));
    let ckb_script = packed::Script::new_builder()
        .code_hash(code_hash.pack())
        .hash_type(ScriptHashType::Data1)
        .args(Bytes::from(args).pack())
        .build();

    assert_eq!(value.packed_bytes(), ckb_script.as_slice());
    let expected_hash: [u8; 32] = ckb_script.calc_script_hash().unpack();
    assert_eq!(value.hash(), expected_hash);
}

#[test]
fn v0_18_constructed_script_exact_args_runs_in_ckb_vm() {
    let type_elf = compile_source_to_elf(SCRIPT_CONSTRUCTION_PROGRAM, "inspect");
    let always_success_elf = compile_source_to_elf(ALWAYS_SUCCESS_LOCK_PROGRAM, "always_success");

    fn run_with_lock_args(type_elf: &[u8], always_success_elf: &[u8], lock_args: Vec<u8>) -> Result<u64, String> {
        let mut context = Context::new_with_deterministic_rng();
        let always_success_out_point = context.deploy_cell(Bytes::copy_from_slice(always_success_elf));
        let type_out_point = context.deploy_cell(Bytes::copy_from_slice(type_elf));
        let input_lock = context.build_script(&always_success_out_point, Bytes::from(lock_args)).expect("input lock script");
        let output_lock = context.build_script(&always_success_out_point, Bytes::default()).expect("output lock script");
        let type_script = context.build_script(&type_out_point, Bytes::default()).expect("type script under test");

        let input = context.create_cell(
            packed::CellOutput::new_builder()
                .capacity::<packed::Uint64>(100_000_000_000u64.pack())
                .lock(input_lock)
                .type_(packed::ScriptOpt::from(type_script.clone()))
                .build(),
            Bytes::default(),
        );
        let output = packed::CellOutput::new_builder()
            .capacity::<packed::Uint64>(100_000_000_000u64.pack())
            .lock(output_lock)
            .type_(packed::ScriptOpt::from(type_script))
            .build();
        let tx = TransactionBuilder::default()
            .input(packed::CellInput::new_builder().previous_output(input).build())
            .output(output)
            .output_data(Bytes::default().pack())
            .witness(Bytes::default().pack())
            .build();
        let tx = context.complete_tx(tx);
        context.verify_tx(&tx, 10_000_000).map_err(|err| format!("{err:?}"))
    }

    let pass_cycles =
        run_with_lock_args(&type_elf, &always_success_elf, b"owner".to_vec()).expect("matching constructed Script args pass");
    assert!(pass_cycles > 0);

    let reject = run_with_lock_args(&type_elf, &always_success_elf, b"wrong".to_vec())
        .expect_err("mismatched constructed Script args must reject");
    assert!(reject.contains("error") || reject.contains("Script"), "{reject}");
}

#[test]
fn v0_18_script_ref_reads_reject_non_source_view_arguments() {
    let err = compile(
        r#"
module v018::bad_script_ref_read

action inspect(flag: bool) -> Hash {
    verification
        return ckb::cell_lock_code_hash(flag)
}
"#,
        CompileOptions {
            target: Some("riscv64-asm".to_string()),
            target_profile: Some("ckb".to_string()),
            primitive_compat: Some("0.18".to_string()),
            ..CompileOptions::default()
        },
    )
    .expect_err("ScriptRef reads must reject non-SourceView arguments");

    assert!(
        err.message.contains("cell_lock_code_hash expects a source view returned by source::*"),
        "unexpected error: {}",
        err.message
    );
}

#[test]
fn v0_18_script_ref_property_rejects_unknown_script_field() {
    let err = compile(
        r#"
module v018::bad_script_ref_property

action inspect() -> Hash {
    verification
        let input = source::group_input(0)
        return input.lock.owner_hash
}
"#,
        CompileOptions {
            target: Some("riscv64-asm".to_string()),
            target_profile: Some("ckb".to_string()),
            primitive_compat: Some("0.18".to_string()),
            ..CompileOptions::default()
        },
    )
    .expect_err("ScriptRef property surface must reject unknown fields");

    assert!(
        err.message.contains("unknown ScriptRef field 'owner_hash'; expected code_hash, hash_type, args_empty, or args_hash"),
        "unexpected error: {}",
        err.message
    );
}

#[test]
fn v0_18_script_args_prefix_suffix_require_hash_operands() {
    let err = compile(
        r#"
module v018::bad_script_args_hash

action inspect() -> u64 {
    verification
        let input = source::group_input(0)
        ckb::require_cell_lock_args_prefix_hash(input, 1)
        return 0
}
"#,
        CompileOptions {
            target: Some("riscv64-asm".to_string()),
            target_profile: Some("ckb".to_string()),
            primitive_compat: Some("0.18".to_string()),
            ..CompileOptions::default()
        },
    )
    .expect_err("ScriptArgs prefix/suffix requirements must reject non-Hash expected operands");

    assert!(
        err.message.contains("require_cell_lock_args_prefix_hash expects (source_view: u64, expected_args_hash: Hash)"),
        "unexpected error: {}",
        err.message
    );
}

#[test]
fn v0_18_script_construction_accepts_literal_code_hashes() {
    let result = compile(
        SCRIPT_LITERAL_CONSTRUCTION_PROGRAM,
        CompileOptions {
            target: Some("riscv64-asm".to_string()),
            target_profile: Some("ckb".to_string()),
            primitive_compat: Some("0.18".to_string()),
            ..CompileOptions::default()
        },
    )
    .expect("literal Hash-backed Script construction should compile");

    let assembly = std::str::from_utf8(&result.artifact_bytes).expect("assembly utf-8");
    assert!(
        assembly.contains(".byte 1\n    .byte 2\n    .byte 3\n    .byte 4"),
        "literal code_hash should be emitted as fixed bytes:\n{assembly}"
    );
}

#[test]
fn v0_18_script_construction_rejects_bad_operands() {
    let bad_hash_type = compile(
        r#"
module v018::bad_script_hash_type

action inspect(code_hash: Hash) -> u64 {
    verification
        let expected = script::new(code_hash, 3, script::args_empty())
        return expected.hash_type
}
"#,
        CompileOptions {
            target: Some("riscv64-asm".to_string()),
            target_profile: Some("ckb".to_string()),
            primitive_compat: Some("0.18".to_string()),
            ..CompileOptions::default()
        },
    )
    .expect_err("script::new must reject unsupported literal hash_type values");
    assert!(bad_hash_type.message.contains("script hash_type must be one of"), "unexpected error: {}", bad_hash_type.message);

    let bad_args = compile(
        r#"
module v018::bad_script_args

action inspect(code_hash: Hash) -> u64 {
    verification
        let expected = script::new(code_hash, script::hash_type_data1(), script::args(1))
        return expected.hash_type
}
"#,
        CompileOptions {
            target: Some("riscv64-asm".to_string()),
            target_profile: Some("ckb".to_string()),
            primitive_compat: Some("0.18".to_string()),
            ..CompileOptions::default()
        },
    )
    .expect_err("script::args must reject scalar non-byte payloads");
    assert!(bad_args.message.contains("script::args expects fixed bytes"), "unexpected error: {}", bad_args.message);

    let bad_hash_bytes = compile(
        r#"
module v018::bad_hash_bytes

action inspect() -> u64 {
    verification
        let code_hash: Hash = Hash::from_bytes(b"short")
        let expected = script::new(code_hash, script::hash_type_data1(), script::args_empty())
        return expected.hash_type
}
"#,
        CompileOptions {
            target: Some("riscv64-asm".to_string()),
            target_profile: Some("ckb".to_string()),
            primitive_compat: Some("0.18".to_string()),
            ..CompileOptions::default()
        },
    )
    .expect_err("Hash::from_bytes must reject non-32-byte payloads");
    assert!(
        bad_hash_bytes.message.contains("Hash::from_bytes expects exactly 32 bytes"),
        "unexpected error: {}",
        bad_hash_bytes.message
    );
}
