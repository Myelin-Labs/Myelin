#[path = "../examples/support/protocol_reference.rs"]
mod reference;

#[test]
fn public_protocol_commitments_match_the_canonical_golden_preimages() {
    let corpus = reference::corpus().unwrap();
    assert_eq!(corpus.vectors.len(), 8);
    for vector in &corpus.vectors {
        reference::verify(vector).unwrap();
        assert!(vector.domain.ends_with(":v1"));
    }
}

#[test]
fn changing_any_committed_field_or_domain_invalidates_the_golden_vector() {
    for vector in reference::corpus().unwrap().vectors {
        let mut changed = vector.clone();
        changed.domain.push('x');
        assert!(reference::verify(&changed).is_err(), "{}", vector.name);
        for index in 0..vector.fields.len() {
            let mut changed = vector.clone();
            let field = &mut changed.fields[index];
            field.value = match field.encoding.as_str() {
                "u8" | "u16" | "u64" => (field.value.parse::<u64>().unwrap() + 1).to_string(),
                "hash32" => "ff".repeat(32),
                "bytes" => format!("{}00", field.value),
                other => panic!("unexpected field encoding {other}"),
            };
            assert!(reference::verify(&changed).is_err(), "{} field {}", vector.name, index);
        }
    }
}

#[test]
fn reference_encoder_rejects_invalid_widths_encodings_and_integer_overflow() {
    let vector = reference::corpus().unwrap().vectors.remove(0);
    for (encoding, value) in
        [("hash32", "00"), ("u8", "256"), ("u16", "65536"), ("u64", "18446744073709551616"), ("bytes", "é"), ("other", "1")]
    {
        let mut changed = vector.clone();
        changed.fields[0].encoding = encoding.to_owned();
        changed.fields[0].value = value.to_owned();
        assert!(reference::preimage(&changed).is_err());
    }
}
