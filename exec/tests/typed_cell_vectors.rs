use myelin_exec::celltx::{compute_conflict_hash, compute_typed_data_hash, encode_conflict_key_value_composite, Script};

fn hex(bytes: impl AsRef<[u8]>) -> String {
    bytes.as_ref().iter().map(|byte| format!("{byte:02x}")).collect()
}

#[test]
fn typed_cell_hash_fixed_vectors() {
    let script = Script::new([0x42; 32], 1, b"invoice-script-args".to_vec());
    let conflict_key = b"invoice:INV-2026-0001";
    let data = b"invoice-state:issued:amount=1250000";

    assert_eq!(hex(compute_conflict_hash(&script, conflict_key)), "7041cd328a8317c1a0ffecda4fbcc0a46c68cc5867d72b1d6dcc2f35030af66f");
    assert_eq!(hex(compute_typed_data_hash(&script, data)), "7d03c13d9d04f0077d5c72181abe04d68f2170dd2bbb82f731c2d969b0ce6c71");
    assert_eq!(
        hex(encode_conflict_key_value_composite(&[b"borrower:acme", b"invoice:INV-2026-0001"])),
        "0d000000626f72726f7765723a61636d6515000000696e766f6963653a494e562d323032362d30303031"
    );
}
