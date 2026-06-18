// SPDX-License-Identifier: MIT
// Copyright (C) 2026 Spora developers
//
// Relative DAA score lock script test

#[cfg(all(test, feature = "vm"))]
mod tests {
    use crate::celltx::{CellInput, CellOutput, CellTx, OutPoint, Script};
    use crate::scripts::timelock::encode_relative_daa_since;
    use crate::scripts::{timelock_relative_code_hash, TIMELOCK_RELATIVE_SCRIPT};
    use crate::vm::{ResolvedCell, ScriptVersion, SimpleDataProvider, TransactionScriptVerifier};
    use std::sync::Arc;

    /// Target delta baked into the fixture: 100 blocks
    const TARGET_DELTA: u64 = 100;

    fn build_provider(code_hash: [u8; 32], input_out_point: OutPoint) -> SimpleDataProvider {
        let mut provider = SimpleDataProvider::new();
        provider.add_script(code_hash, TIMELOCK_RELATIVE_SCRIPT.to_vec());
        provider.add_cell(
            input_out_point.tx_hash,
            input_out_point.index,
            ResolvedCell {
                cell_output: CellOutput { capacity: 1000, lock: Script { code_hash, hash_type: 0, args: vec![] }, type_: None },
                data: Some(vec![]),
            },
        );
        provider
    }

    #[test]
    fn test_timelock_relative_accepts_valid_delta() {
        let code_hash = timelock_relative_code_hash();
        let input_out_point = OutPoint::new([0x21; 32], 0);
        let provider = build_provider(code_hash, input_out_point.clone());

        // Use exact target delta
        let since = encode_relative_daa_since(TARGET_DELTA);
        let tx = CellTx {
            version: 0xC001,
            inputs: vec![CellInput::new(input_out_point, since)],
            cell_deps: vec![],
            header_deps: vec![],
            outputs: vec![],
            outputs_data: vec![],
            witnesses: vec![],
        };

        let verifier =
            TransactionScriptVerifier::new(Arc::new(tx), Arc::new(provider)).with_version(ScriptVersion::V2).with_max_cycles(20_000);

        assert!(verifier.verify().is_ok());
    }

    #[test]
    fn test_timelock_relative_accepts_larger_delta() {
        let code_hash = timelock_relative_code_hash();
        let input_out_point = OutPoint::new([0x22; 32], 0);
        let provider = build_provider(code_hash, input_out_point.clone());

        // Use a larger delta
        let since = encode_relative_daa_since(TARGET_DELTA + 50);
        let tx = CellTx {
            version: 0xC001,
            inputs: vec![CellInput::new(input_out_point, since)],
            cell_deps: vec![],
            header_deps: vec![],
            outputs: vec![],
            outputs_data: vec![],
            witnesses: vec![],
        };

        let verifier =
            TransactionScriptVerifier::new(Arc::new(tx), Arc::new(provider)).with_version(ScriptVersion::V2).with_max_cycles(20_000);

        assert!(verifier.verify().is_ok());
    }

    #[test]
    fn test_timelock_relative_rejects_small_delta() {
        let code_hash = timelock_relative_code_hash();
        let input_out_point = OutPoint::new([0x23; 32], 0);
        let provider = build_provider(code_hash, input_out_point.clone());

        // Use a smaller delta
        let since = encode_relative_daa_since(TARGET_DELTA - 50);
        let tx = CellTx {
            version: 0xC001,
            inputs: vec![CellInput::new(input_out_point, since)],
            cell_deps: vec![],
            header_deps: vec![],
            outputs: vec![],
            outputs_data: vec![],
            witnesses: vec![],
        };

        let verifier =
            TransactionScriptVerifier::new(Arc::new(tx), Arc::new(provider)).with_version(ScriptVersion::V2).with_max_cycles(20_000);

        assert!(verifier.verify().is_err());
    }

    #[test]
    fn test_timelock_relative_rejects_absolute_lock() {
        let code_hash = timelock_relative_code_hash();
        let input_out_point = OutPoint::new([0x24; 32], 0);
        let provider = build_provider(code_hash, input_out_point.clone());

        // Use absolute lock (bit63 = 0)
        let since = (0u64 << 63) | (0u64 << 62) | TARGET_DELTA;
        let tx = CellTx {
            version: 0xC001,
            inputs: vec![CellInput::new(input_out_point, since)],
            cell_deps: vec![],
            header_deps: vec![],
            outputs: vec![],
            outputs_data: vec![],
            witnesses: vec![],
        };

        let verifier =
            TransactionScriptVerifier::new(Arc::new(tx), Arc::new(provider)).with_version(ScriptVersion::V2).with_max_cycles(20_000);

        assert!(verifier.verify().is_err());
    }

    #[test]
    fn test_timelock_relative_rejects_timestamp_lock() {
        let code_hash = timelock_relative_code_hash();
        let input_out_point = OutPoint::new([0x25; 32], 0);
        let provider = build_provider(code_hash, input_out_point.clone());

        // Use timestamp lock instead of DAA (bit62 = 1)
        let since = (1u64 << 63) | (1u64 << 62) | TARGET_DELTA;
        let tx = CellTx {
            version: 0xC001,
            inputs: vec![CellInput::new(input_out_point, since)],
            cell_deps: vec![],
            header_deps: vec![],
            outputs: vec![],
            outputs_data: vec![],
            witnesses: vec![],
        };

        let verifier =
            TransactionScriptVerifier::new(Arc::new(tx), Arc::new(provider)).with_version(ScriptVersion::V2).with_max_cycles(20_000);

        assert!(verifier.verify().is_err());
    }
}
