// SPDX-License-Identifier: MIT
// Copyright (C) 2026 Spora developers
//
// Absolute timestamp lock script test

#[cfg(all(test, feature = "vm"))]
mod tests {
    use crate::celltx::{CellInput, CellOutput, CellTx, OutPoint, Script};
    use crate::scripts::timelock::encode_absolute_timestamp_since;
    use crate::scripts::{timelock_absolute_code_hash, TIMELOCK_ABSOLUTE_SCRIPT};
    use crate::vm::{ResolvedCell, ScriptVersion, SimpleDataProvider, TransactionScriptVerifier};
    use std::sync::Arc;

    /// Target timestamp baked into the fixture: 2025-01-01 00:00:00 UTC
    const TARGET_TIMESTAMP: u64 = 1735689600;

    fn build_provider(code_hash: [u8; 32], input_out_point: OutPoint) -> SimpleDataProvider {
        let mut provider = SimpleDataProvider::new();
        provider.add_script(code_hash, TIMELOCK_ABSOLUTE_SCRIPT.to_vec());
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
    fn test_timelock_absolute_accepts_valid_timestamp() {
        let code_hash = timelock_absolute_code_hash();
        let input_out_point = OutPoint::new([0x11; 32], 0);
        let provider = build_provider(code_hash, input_out_point.clone());

        // Use exact target timestamp
        let since = encode_absolute_timestamp_since(TARGET_TIMESTAMP);
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
    fn test_timelock_absolute_accepts_future_timestamp() {
        let code_hash = timelock_absolute_code_hash();
        let input_out_point = OutPoint::new([0x12; 32], 0);
        let provider = build_provider(code_hash, input_out_point.clone());

        // Use a future timestamp (target + 1 day)
        let since = encode_absolute_timestamp_since(TARGET_TIMESTAMP + 86400);
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
    fn test_timelock_absolute_rejects_past_timestamp() {
        let code_hash = timelock_absolute_code_hash();
        let input_out_point = OutPoint::new([0x13; 32], 0);
        let provider = build_provider(code_hash, input_out_point.clone());

        // Use a past timestamp (target - 1 day)
        let since = encode_absolute_timestamp_since(TARGET_TIMESTAMP - 86400);
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
    fn test_timelock_absolute_rejects_relative_lock() {
        let code_hash = timelock_absolute_code_hash();
        let input_out_point = OutPoint::new([0x14; 32], 0);
        let provider = build_provider(code_hash, input_out_point.clone());

        // Use relative lock (bit63 = 1)
        let since = (1u64 << 63) | (1u64 << 62) | TARGET_TIMESTAMP;
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
    fn test_timelock_absolute_rejects_daa_lock() {
        let code_hash = timelock_absolute_code_hash();
        let input_out_point = OutPoint::new([0x15; 32], 0);
        let provider = build_provider(code_hash, input_out_point.clone());

        // Use DAA lock instead of timestamp (bit62 = 0)
        let since = (0u64 << 63) | (0u64 << 62) | TARGET_TIMESTAMP;
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
