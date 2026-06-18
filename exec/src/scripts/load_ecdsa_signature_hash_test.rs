// SPDX-License-Identifier: MIT
// Copyright (C) 2026 Spora developers
//
// Load-ecdsa-signature-hash lock script test

#[cfg(all(test, feature = "vm"))]
mod tests {
    use crate::celltx::sighash::{calc_standard_ecdsa_signature_hash, StandardSigHashReusedValues, StandardSigHashType};
    use crate::celltx::{CellInput, CellOutput, CellTx, OutPoint, Script};
    use crate::scripts::{load_ecdsa_signature_hash_code_hash, LOAD_ECDSA_SIGNATURE_HASH_SCRIPT};
    use crate::vm::syscalls::load_signature_hash::standard_signing_input_from_resolved_cell;
    use crate::vm::{ResolvedCell, ScriptVersion, SimpleDataProvider, TransactionScriptVerifier, VmSemantics};
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

    fn build_resolved_input(code_hash: [u8; 32]) -> ResolvedCell {
        ResolvedCell {
            cell_output: CellOutput { capacity: 4_200, lock: Script { code_hash, hash_type: 0, args: vec![] }, type_: None },
            data: Some(vec![0xAB, 0xCD, 0xEF]),
        }
    }

    fn build_provider(code_hash: [u8; 32], input_out_point: OutPoint, resolved_input: ResolvedCell) -> SimpleDataProvider {
        let mut provider = SimpleDataProvider::new();
        provider.add_script(code_hash, LOAD_ECDSA_SIGNATURE_HASH_SCRIPT.to_vec());
        provider.add_cell(input_out_point.tx_hash, input_out_point.index, resolved_input);
        provider
    }

    #[test]
    fn test_load_ecdsa_signature_hash_verification() {
        let code_hash = load_ecdsa_signature_hash_code_hash();
        let input_out_point = OutPoint::new([0x71; 32], 0);
        let resolved_input = build_resolved_input(code_hash);
        let tx_without_witness = CellTx {
            version: 0xC001,
            inputs: vec![CellInput::new(input_out_point, 0x1122_3344_5566_7788)],
            cell_deps: vec![],
            header_deps: vec![],
            outputs: vec![CellOutput { capacity: 999, lock: Script::new([0x09; 32], 0, vec![0x01]), type_: None }],
            outputs_data: vec![vec![0x42, 0x43]],
            witnesses: vec![vec![]],
        };

        let signing_input = standard_signing_input_from_resolved_cell(&resolved_input);
        let expected_hash =
            calc_standard_ecdsa_signature_hash(&tx_without_witness, 0, TestSigHashType(0x01), &signing_input, &NoCache);

        let tx = CellTx { witnesses: vec![expected_hash.as_bytes().iter().copied().chain([0x01]).collect()], ..tx_without_witness };
        let provider = build_provider(code_hash, input_out_point, resolved_input);
        let verifier =
            TransactionScriptVerifier::new(Arc::new(tx), Arc::new(provider)).with_version(ScriptVersion::V2).with_max_cycles(200_000);

        assert!(verifier.verify().is_ok());
    }

    #[test]
    fn test_load_ecdsa_signature_hash_rejects_wrong_digest() {
        let code_hash = load_ecdsa_signature_hash_code_hash();
        let input_out_point = OutPoint::new([0x72; 32], 0);
        let resolved_input = build_resolved_input(code_hash);
        let tx = CellTx {
            version: 0xC001,
            inputs: vec![CellInput::new(input_out_point, 7)],
            cell_deps: vec![],
            header_deps: vec![],
            outputs: vec![],
            outputs_data: vec![],
            witnesses: vec![vec![0xFF; 33]],
        };
        let provider = build_provider(code_hash, input_out_point, resolved_input);
        let verifier =
            TransactionScriptVerifier::new(Arc::new(tx), Arc::new(provider)).with_version(ScriptVersion::V2).with_max_cycles(200_000);

        assert!(verifier.verify().is_err());
    }

    #[test]
    fn test_load_ecdsa_signature_hash_is_not_available_under_ckb_strict_semantics() {
        let code_hash = load_ecdsa_signature_hash_code_hash();
        let input_out_point = OutPoint::new([0x73; 32], 0);
        let resolved_input = build_resolved_input(code_hash);
        let tx_without_witness = CellTx {
            version: 0xC001,
            inputs: vec![CellInput::new(input_out_point, 0)],
            cell_deps: vec![],
            header_deps: vec![],
            outputs: vec![CellOutput { capacity: 999, lock: Script::new([0x09; 32], 0, vec![0x01]), type_: None }],
            outputs_data: vec![vec![0x42, 0x43]],
            witnesses: vec![vec![]],
        };

        let signing_input = standard_signing_input_from_resolved_cell(&resolved_input);
        let expected_hash =
            calc_standard_ecdsa_signature_hash(&tx_without_witness, 0, TestSigHashType(0x01), &signing_input, &NoCache);
        let tx = CellTx { witnesses: vec![expected_hash.as_bytes().iter().copied().chain([0x01]).collect()], ..tx_without_witness };
        let provider = build_provider(code_hash, input_out_point, resolved_input);
        let verifier = TransactionScriptVerifier::new(Arc::new(tx), Arc::new(provider))
            .with_version(ScriptVersion::V2)
            .with_max_cycles(200_000)
            .with_semantics(VmSemantics::CkbStrict);

        assert!(verifier.verify().is_err(), "CkbStrict must not expose Spora-only signature hash syscall 3004");
    }
}
