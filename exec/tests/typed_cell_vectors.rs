use myelin_exec::celltx::{compute_conflict_hash, compute_typed_data_hash, encode_conflict_key_value_composite, Script};

fn hex(bytes: impl AsRef<[u8]>) -> String {
    bytes.as_ref().iter().map(|byte| format!("{byte:02x}")).collect()
}

#[test]
fn typed_cell_hash_fixed_vectors() {
    let script = Script::new([0x42; 32], 1, b"invoice-script-args".to_vec());
    let conflict_key = b"invoice:INV-2026-0001";
    let data = b"invoice-state:issued:amount=1250000";

    assert_eq!(hex(compute_conflict_hash(&script, conflict_key)), "ae22ab496af025a61b4838f4cff61b0ca3c880cc3f0e9671d98bd11efec5ae5a");
    assert_eq!(hex(compute_typed_data_hash(&script, data)), "58aef33001edd12860e0d0509ef341d8a1637ec09e9e460fb4c2500157db260d");
    assert_eq!(
        hex(encode_conflict_key_value_composite(&[b"borrower:acme", b"invoice:INV-2026-0001"])),
        "0d000000626f72726f7765723a61636d6515000000696e766f6963653a494e562d323032362d30303031"
    );
}
