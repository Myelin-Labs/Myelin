use myelin_session::HandoffIntent;
use myelin_session_transfer::{verify_intent, TransferPayload, TransferSigner};

fn unhex(s: &str) -> Vec<u8> {
    assert!(s.is_ascii() && s.len().is_multiple_of(2));
    (0..s.len()).step_by(2).map(|i| u8::from_str_radix(&s[i..i + 2], 16).unwrap()).collect()
}
#[test]
fn fixed_payload_authorization_and_signature_independent_ids_remain_byte_exact() {
    let corpus: serde_json::Value = serde_json::from_str(include_str!("../../fixtures/protocol/reference-transfer-v1.json")).unwrap();
    assert_eq!(corpus["format"], "myelin-reference-transfer-v1");
    let vectors = corpus["vectors"].as_array().unwrap();
    assert_eq!(vectors.len(), 2);
    for vector in vectors {
        let expected_payload: TransferPayload = serde_json::from_value(vector["payload"].clone()).unwrap();
        let bytes = unhex(vector["payload_hex"].as_str().unwrap());
        assert_eq!(TransferPayload::decode(&bytes).unwrap(), expected_payload);
        assert_eq!(expected_payload.encode().unwrap(), bytes);
        let intent = HandoffIntent {
            target: expected_payload.target.outer(),
            expires_at_ms: 100,
            payload: bytes,
            authorization: unhex(vector["authorization_hex"].as_str().unwrap()),
            evidence_requirement: None,
        };
        let (id, payload) = verify_intent(&intent).unwrap();
        assert_eq!(id.to_vec(), unhex(vector["transfer_id"].as_str().unwrap()));
        assert_eq!(TransferSigner::new([3; 32]).unwrap().sign(payload.clone(), 100, None).unwrap(), intent);
        let mut preimage = b"myelin:session-effect-handoff-authorization:v1".to_vec();
        preimage.extend(
            postcard::to_allocvec(&(
                payload.source.genesis_commitment,
                payload.scope(),
                &intent.target,
                intent.expires_at_ms,
                &intent.payload,
                &intent.evidence_requirement,
            ))
            .unwrap(),
        );
        assert_eq!(preimage, unhex(vector["source_authorization_preimage_hex"].as_str().unwrap()));
        assert_eq!(*blake3::hash(&preimage).as_bytes(), id);
        let mut changed = intent.clone();
        changed.authorization.push(0);
        assert!(verify_intent(&changed).is_err());
        let mut changed = intent;
        changed.payload[4] = 2;
        assert!(verify_intent(&changed).is_err());
    }
}

#[test]
fn retirement_command_identity_and_schema_match_independent_vectors() {
    use myelin_session_transfer::{input_schema, TransferCommand};
    let v: serde_json::Value = serde_json::from_str(include_str!("../../fixtures/protocol/retirement.json")).unwrap();
    let id = serde_json::from_value(v["resolved_action"]["action"]["Retire"]["handoff_id"].clone()).unwrap();
    let genesis = serde_json::from_value(v["command_genesis"].clone()).unwrap();
    let command = TransferCommand::Retire { handoff_id: id };
    let actual = command.identity(genesis).unwrap();
    assert_eq!(actual.id.to_vec(), unhex(v["command_id"]["commitment"].as_str().unwrap()));
    assert_eq!(actual.payload_hash.to_vec(), unhex(v["command_payload"]["commitment"].as_str().unwrap()));
    assert_eq!(input_schema().to_vec(), unhex(v["input_schema"]["commitment"].as_str().unwrap()));
    for key in ["command_id", "command_payload", "input_schema"] {
        assert_eq!(blake3::hash(&unhex(v[key]["preimage_hex"].as_str().unwrap())).to_hex().as_str(), v[key]["commitment"]);
    }
}
