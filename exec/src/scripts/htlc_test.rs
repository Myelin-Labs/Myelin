// SPDX-License-Identifier: MIT
// Copyright (C) 2026 Myelin developers
//
// HTLC (Hash Time Locked Contract) script test

#[cfg(all(test, feature = "vm"))]
mod tests {
    use crate::celltx::{CellInput, CellOutput, CellTx, OutPoint, Script};
    use crate::scripts::timelock::encode_absolute_timestamp_since;
    use crate::scripts::{htlc_code_hash, HTLC_SCRIPT};
    use crate::serialization::VmAbiFormat;
    use crate::vm::{ResolvedCell, ScriptVersion, SimpleDataProvider, TransactionScriptVerifier};
    use std::sync::Arc;

    /// Target timestamp for HTLC timeout: 2025-01-01 00:00:00 UTC
    const TARGET_TIMESTAMP: u64 = 1735689600;

    fn build_provider(code_hash: [u8; 32], input_out_point: OutPoint, args: Vec<u8>) -> SimpleDataProvider {
        let mut provider = SimpleDataProvider::new();
        provider.add_script(code_hash, HTLC_SCRIPT.to_vec());
        provider.add_cell(
            input_out_point.tx_hash,
            input_out_point.index,
            ResolvedCell {
                cell_output: CellOutput { capacity: 1000, lock: Script { code_hash, hash_type: 0, args }, type_: None },
                data: Some(vec![]),
            },
        );
        provider
    }

    /// Build HTLC script args
    /// Format: secret_hash(32) + recipient_pubkey(32) + sender_pubkey(32) + lock_type(1) + lock_value(8)
    fn build_htlc_args(
        secret_hash: [u8; 32],
        recipient_pubkey: [u8; 32],
        sender_pubkey: [u8; 32],
        lock_type: u8,
        lock_value: u64,
    ) -> Vec<u8> {
        let mut args = Vec::with_capacity(105);
        args.extend_from_slice(&secret_hash);
        args.extend_from_slice(&recipient_pubkey);
        args.extend_from_slice(&sender_pubkey);
        args.push(lock_type);
        args.extend_from_slice(&lock_value.to_le_bytes());
        args
    }

    /// Build recipient witness: signature(64) + secret(32) + path_selector(1)
    fn build_recipient_witness(signature: [u8; 64], secret: [u8; 32]) -> Vec<u8> {
        let mut witness = Vec::with_capacity(97);
        witness.extend_from_slice(&signature);
        witness.extend_from_slice(&secret);
        witness.push(0x01); // Recipient path selector
        witness
    }

    /// Build sender witness: signature(64) + path_selector(1)
    fn build_sender_witness(signature: [u8; 64]) -> Vec<u8> {
        let mut witness = Vec::with_capacity(65);
        witness.extend_from_slice(&signature);
        witness.push(0x00); // Sender path selector
        witness
    }

    fn build_fixture_signature(pubkey: [u8; 32]) -> [u8; 64] {
        let mut signature = [0u8; 64];
        let first = blake3::hash(&[b"myelin-htlc-fixture-sig-a".as_slice(), pubkey.as_slice()].concat());
        let second = blake3::hash(&[b"myelin-htlc-fixture-sig-b".as_slice(), pubkey.as_slice()].concat());
        signature[..32].copy_from_slice(first.as_bytes());
        signature[32..].copy_from_slice(second.as_bytes());
        signature
    }

    #[test]
    fn test_htlc_recipient_path_rejects_non_molecule_fixture() {
        let code_hash = htlc_code_hash();
        let input_out_point = OutPoint::new([0x31; 32], 0);

        // Create secret and its hash
        let secret = [0xABu8; 32];
        let secret_hash = blake3::hash(&secret).into();

        let recipient_pubkey = [0x11u8; 32];
        let sender_pubkey = [0x22u8; 32];
        let signature = build_fixture_signature(recipient_pubkey);

        let args = build_htlc_args(
            secret_hash,
            recipient_pubkey,
            sender_pubkey,
            1, // absolute timestamp
            TARGET_TIMESTAMP,
        );

        let provider = build_provider(code_hash, input_out_point.clone(), args);

        // Use any since value (recipient path doesn't check timelock)
        let since = 0u64;
        let witness = build_recipient_witness(signature, secret);

        let tx = CellTx {
            version: 0xC001,
            inputs: vec![CellInput::new(input_out_point, since)],
            cell_deps: vec![],
            header_deps: vec![],
            outputs: vec![],
            outputs_data: vec![],
            witnesses: vec![witness],
        };

        let verifier = TransactionScriptVerifier::new(Arc::new(tx), Arc::new(provider))
            .with_abi_format(VmAbiFormat::Molecule)
            .with_version(ScriptVersion::V2)
            .with_max_cycles(1_000_000);

        let result = verifier.verify();
        assert!(result.is_err(), "legacy HTLC fixture should not validate under Molecule-only VM ABI");
    }

    #[test]
    fn test_htlc_recipient_path_wrong_secret() {
        let code_hash = htlc_code_hash();
        let input_out_point = OutPoint::new([0x32; 32], 0);

        // Create secret and its hash
        let secret = [0xABu8; 32];
        let secret_hash = blake3::hash(&secret).into();

        let recipient_pubkey = [0x11u8; 32];
        let sender_pubkey = [0x22u8; 32];
        let signature = build_fixture_signature(recipient_pubkey);

        let args = build_htlc_args(
            secret_hash,
            recipient_pubkey,
            sender_pubkey,
            1, // absolute timestamp
            TARGET_TIMESTAMP,
        );

        let provider = build_provider(code_hash, input_out_point.clone(), args);

        // Use wrong secret
        let wrong_secret = [0xCDu8; 32];
        let since = 0u64;
        let witness = build_recipient_witness(signature, wrong_secret);

        let tx = CellTx {
            version: 0xC001,
            inputs: vec![CellInput::new(input_out_point, since)],
            cell_deps: vec![],
            header_deps: vec![],
            outputs: vec![],
            outputs_data: vec![],
            witnesses: vec![witness],
        };

        let verifier = TransactionScriptVerifier::new(Arc::new(tx), Arc::new(provider))
            .with_abi_format(VmAbiFormat::Molecule)
            .with_version(ScriptVersion::V2)
            .with_max_cycles(100_000);

        assert!(verifier.verify().is_err());
    }

    #[test]
    fn test_htlc_sender_path_rejects_non_molecule_fixture() {
        let code_hash = htlc_code_hash();
        let input_out_point = OutPoint::new([0x33; 32], 0);

        let secret = [0xABu8; 32];
        let secret_hash = blake3::hash(&secret).into();
        let recipient_pubkey = [0x11u8; 32];
        let sender_pubkey = [0x22u8; 32];
        let signature = build_fixture_signature(sender_pubkey);

        let args = build_htlc_args(
            secret_hash,
            recipient_pubkey,
            sender_pubkey,
            1, // absolute timestamp
            TARGET_TIMESTAMP,
        );

        let provider = build_provider(code_hash, input_out_point.clone(), args);

        // Use future timestamp (after lock time)
        let since = encode_absolute_timestamp_since(TARGET_TIMESTAMP + 86400);
        let witness = build_sender_witness(signature);

        let tx = CellTx {
            version: 0xC001,
            inputs: vec![CellInput::new(input_out_point, since)],
            cell_deps: vec![],
            header_deps: vec![],
            outputs: vec![],
            outputs_data: vec![],
            witnesses: vec![witness],
        };

        let verifier = TransactionScriptVerifier::new(Arc::new(tx), Arc::new(provider))
            .with_abi_format(VmAbiFormat::Molecule)
            .with_version(ScriptVersion::V2)
            .with_max_cycles(1_000_000);

        let result = verifier.verify();
        assert!(result.is_err(), "legacy HTLC fixture should not validate under Molecule-only VM ABI");
    }

    #[test]
    fn test_htlc_sender_path_before_timeout() {
        let code_hash = htlc_code_hash();
        let input_out_point = OutPoint::new([0x34; 32], 0);

        let secret = [0xABu8; 32];
        let secret_hash = blake3::hash(&secret).into();
        let recipient_pubkey = [0x11u8; 32];
        let sender_pubkey = [0x22u8; 32];
        let signature = build_fixture_signature(sender_pubkey);

        let args = build_htlc_args(
            secret_hash,
            recipient_pubkey,
            sender_pubkey,
            1, // absolute timestamp
            TARGET_TIMESTAMP,
        );

        let provider = build_provider(code_hash, input_out_point.clone(), args);

        // Use past timestamp (before lock time)
        let since = encode_absolute_timestamp_since(TARGET_TIMESTAMP - 86400);
        let witness = build_sender_witness(signature);

        let tx = CellTx {
            version: 0xC001,
            inputs: vec![CellInput::new(input_out_point, since)],
            cell_deps: vec![],
            header_deps: vec![],
            outputs: vec![],
            outputs_data: vec![],
            witnesses: vec![witness],
        };

        let verifier = TransactionScriptVerifier::new(Arc::new(tx), Arc::new(provider))
            .with_abi_format(VmAbiFormat::Molecule)
            .with_version(ScriptVersion::V2)
            .with_max_cycles(100_000);

        assert!(verifier.verify().is_err());
    }

    #[test]
    fn test_htlc_script_size() {
        assert!(HTLC_SCRIPT.len() > 64);
        assert_eq!(&HTLC_SCRIPT[..4], b"\x7fELF");
    }

    #[test]
    fn test_htlc_recipient_path_wrong_signature() {
        let code_hash = htlc_code_hash();
        let input_out_point = OutPoint::new([0x35; 32], 0);

        let secret = [0xABu8; 32];
        let secret_hash = blake3::hash(&secret).into();
        let recipient_pubkey = [0x11u8; 32];
        let sender_pubkey = [0x22u8; 32];
        let wrong_signature = build_fixture_signature(sender_pubkey);

        let args = build_htlc_args(secret_hash, recipient_pubkey, sender_pubkey, 1, TARGET_TIMESTAMP);
        let provider = build_provider(code_hash, input_out_point.clone(), args);

        let witness = build_recipient_witness(wrong_signature, secret);
        let tx = CellTx {
            version: 0xC001,
            inputs: vec![CellInput::new(input_out_point, 0)],
            cell_deps: vec![],
            header_deps: vec![],
            outputs: vec![],
            outputs_data: vec![],
            witnesses: vec![witness],
        };

        let verifier = TransactionScriptVerifier::new(Arc::new(tx), Arc::new(provider))
            .with_abi_format(VmAbiFormat::Molecule)
            .with_version(ScriptVersion::V2)
            .with_max_cycles(100_000);

        assert!(verifier.verify().is_err());
    }
}
