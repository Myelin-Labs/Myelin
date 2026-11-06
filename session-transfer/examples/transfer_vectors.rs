//! Reproduce synthetic codec/signature vectors. These contain development keys
//! and are not source finality, balance or deployment evidence.
use myelin_session_transfer::*;

fn hex(bytes: &[u8]) -> String {
    bytes.iter().map(|b| format!("{b:02x}")).collect()
}
fn main() -> Result<()> {
    let signer = TransferSigner::new([3; 32])?;
    let recipient = TransferSigner::new([4; 32])?.actor();
    let mut vectors = vec![];
    for kind in [TransferKind::Scarce, TransferKind::CopyableReceipt] {
        let payload = TransferPayload {
            deployment_id: [1; 32],
            uniqueness_domain: [2; 32],
            kind,
            intake_schema: kind.intake_schema(),
            source: SourceIdentity {
                session_id: [3; 32],
                genesis_commitment: [31; 32],
                application_profile_commitment: [32; 32],
                consensus_module_commitment: [33; 32],
                effects_policy_hash: [34; 32],
            },
            asset: AssetId { namespace: [80; 32], object: [81; 32] },
            owner: signer.actor(),
            recipient,
            source_version: 0,
            amount: if kind == TransferKind::Scarce { 10 } else { 1 },
            components: vec![7, 8],
            nonce: [1; 32],
            target: TransferTarget::Session { session_id: [4; 32], genesis_commitment: [5; 32] },
        };
        let intent = signer.sign(payload.clone(), 100, None)?;
        let (id, verified) = verify_intent(&intent)?;
        assert_eq!(verified, payload);
        let scope = payload.scope();
        let mut preimage = b"myelin:session-effect-handoff-authorization:v1".to_vec();
        preimage.extend(
            postcard::to_allocvec(&(
                payload.source.genesis_commitment,
                &scope,
                &intent.target,
                intent.expires_at_ms,
                &intent.payload,
                &intent.evidence_requirement,
            ))
            .map_err(|e| Error::Codec(e.to_string()))?,
        );
        vectors.push(serde_json::json!({"kind":kind,"payload":payload,"payload_hex":hex(&intent.payload),"authorization_hex":hex(&intent.authorization),
            "source_authorization_preimage_hex":hex(&preimage),"transfer_id":hex(&id)}));
    }
    println!("{}",serde_json::to_string_pretty(&serde_json::json!({"format":"myelin-reference-transfer-v1","synthetic":true,"signer_secret_hex":hex(&[3;32]),"vectors":vectors})).map_err(|e| Error::Codec(e.to_string()))?);
    Ok(())
}
