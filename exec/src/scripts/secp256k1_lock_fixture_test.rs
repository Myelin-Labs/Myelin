// SPDX-License-Identifier: MIT
// Copyright (C) 2026 Spora developers
//
// Secp256k1 lock fixture test

#[cfg(all(test, feature = "vm"))]
mod tests {
    use crate::celltx::sighash::{calc_standard_ecdsa_signature_hash, StandardSigHashReusedValues, StandardSigHashType};
    use crate::celltx::{CellInput, CellOutput, CellTx, OutPoint, Script};
    use crate::scripts::{secp256k1_lock_fixture_code_hash, SECP256K1_LOCK_FIXTURE_SCRIPT};
    use crate::serialization::VmAbiFormat;
    use crate::vm::syscalls::load_signature_hash::standard_signing_input_from_resolved_cell;
    use crate::vm::{ResolvedCell, ScriptVersion, SimpleDataProvider, TransactionScriptVerifier};
    use secp256k1::{Message, PublicKey, Secp256k1, SecretKey};
    use spora_hashes::Hash;
    use std::sync::Arc;

    #[derive(Clone, Copy)]
    struct TestSigHashType(u8);

    impl StandardSigHashType for TestSigHashType {
        fn is_sighash_none(self) -> bool {
            self.0 & 0b0000_0111 == 0b0000_0010
        }

        fn is_sighash_single(self) -> bool {
            self.0 & 0b0000_0111 == 0b0000_0100
        }

        fn is_sighash_anyone_can_pay(self) -> bool {
            self.0 & 0b1000_0000 == 0b1000_0000
        }

        fn to_u8(self) -> u8 {
            self.0
        }
    }

    #[derive(Default)]
    struct NoCache;

    impl StandardSigHashReusedValues for NoCache {
        fn previous_outputs_hash(&self, set: impl Fn() -> Hash) -> Hash {
            set()
        }

        fn sequences_hash(&self, set: impl Fn() -> Hash) -> Hash {
            set()
        }

        fn sig_op_counts_hash(&self, set: impl Fn() -> Hash) -> Hash {
            set()
        }

        fn outputs_hash(&self, set: impl Fn() -> Hash) -> Hash {
            set()
        }

        fn payload_hash(&self, set: impl Fn() -> Hash) -> Hash {
            set()
        }
    }

    fn pubkey_hash20(pubkey: &PublicKey) -> [u8; 20] {
        let hash = blake3::hash(&pubkey.serialize());
        let mut out = [0u8; 20];
        out.copy_from_slice(&hash.as_bytes()[..20]);
        out
    }

    fn build_provider(code_hash: [u8; 32], input_out_point: OutPoint, resolved_input: ResolvedCell) -> SimpleDataProvider {
        let mut provider = SimpleDataProvider::new();
        provider.add_script(code_hash, SECP256K1_LOCK_FIXTURE_SCRIPT.to_vec());
        provider.add_cell(input_out_point.tx_hash, input_out_point.index, resolved_input);
        provider
    }

    fn base_tx(input_out_point: OutPoint) -> CellTx {
        CellTx {
            version: 0xC001,
            inputs: vec![CellInput::new(input_out_point, 0x0102_0304_0506_0708)],
            cell_deps: vec![],
            header_deps: vec![],
            outputs: vec![CellOutput { capacity: 1_000, lock: Script::new([0x42; 32], 0, vec![0xAA, 0xBB]), type_: None }],
            outputs_data: vec![vec![0x99, 0x88]],
            witnesses: vec![vec![]],
        }
    }

    fn sign_fixture_witness(tx: &CellTx, resolved_input: &ResolvedCell, hash_type: u8, secret_key: &SecretKey) -> Vec<u8> {
        let signing_input = standard_signing_input_from_resolved_cell(resolved_input);
        let sighash = calc_standard_ecdsa_signature_hash(tx, 0, TestSigHashType(hash_type), &signing_input, &NoCache);
        let message = Message::from_digest_slice(&sighash.as_bytes()).expect("message");
        let signature = Secp256k1::new().sign_ecdsa_recoverable(&message, secret_key);
        let (recovery_id, compact) = signature.serialize_compact();

        let mut witness = Vec::with_capacity(66);
        witness.extend_from_slice(&compact);
        witness.push(recovery_id.to_i32() as u8);
        witness.push(hash_type);
        witness
    }

    #[test]
    fn test_secp256k1_lock_fixture_accepts_valid_signature() {
        let code_hash = secp256k1_lock_fixture_code_hash();
        let input_out_point = OutPoint::new([0x81; 32], 0);
        let secret_key = SecretKey::from_slice(&[0x11; 32]).expect("secret key");
        let pubkey = PublicKey::from_secret_key(&Secp256k1::new(), &secret_key);
        let resolved_input = ResolvedCell {
            cell_output: CellOutput { capacity: 4_200, lock: Script::new(code_hash, 0, pubkey_hash20(&pubkey).to_vec()), type_: None },
            data: Some(vec![0xAB, 0xCD, 0xEF]),
        };
        let tx_without_witness = base_tx(input_out_point);
        let witness = sign_fixture_witness(&tx_without_witness, &resolved_input, 0x01, &secret_key);
        let tx = CellTx { witnesses: vec![witness], ..tx_without_witness };
        let provider = build_provider(code_hash, input_out_point, resolved_input);
        let verifier = TransactionScriptVerifier::new(Arc::new(tx), Arc::new(provider))
            .with_abi_format(VmAbiFormat::Legacy)
            .with_version(ScriptVersion::V2)
            .with_max_cycles(400_000);

        assert!(verifier.verify().is_ok());
    }

    #[test]
    fn test_secp256k1_lock_fixture_rejects_tampered_signature() {
        let code_hash = secp256k1_lock_fixture_code_hash();
        let input_out_point = OutPoint::new([0x82; 32], 0);
        let secret_key = SecretKey::from_slice(&[0x22; 32]).expect("secret key");
        let pubkey = PublicKey::from_secret_key(&Secp256k1::new(), &secret_key);
        let resolved_input = ResolvedCell {
            cell_output: CellOutput { capacity: 4_200, lock: Script::new(code_hash, 0, pubkey_hash20(&pubkey).to_vec()), type_: None },
            data: Some(vec![0xFE, 0xED]),
        };
        let tx_without_witness = base_tx(input_out_point);
        let mut witness = sign_fixture_witness(&tx_without_witness, &resolved_input, 0x01, &secret_key);
        witness[0] ^= 0xFF;
        let tx = CellTx { witnesses: vec![witness], ..tx_without_witness };
        let provider = build_provider(code_hash, input_out_point, resolved_input);
        let verifier = TransactionScriptVerifier::new(Arc::new(tx), Arc::new(provider))
            .with_abi_format(VmAbiFormat::Legacy)
            .with_version(ScriptVersion::V2)
            .with_max_cycles(400_000);

        assert!(verifier.verify().is_err());
    }
}
