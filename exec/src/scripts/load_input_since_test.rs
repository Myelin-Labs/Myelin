// SPDX-License-Identifier: MIT
// Copyright (C) 2026 Myelin developers
//
// Load-input-since lock script test

#[cfg(all(test, feature = "vm"))]
mod tests {
    use crate::celltx::{CellInput, CellOutput, CellTx, OutPoint, Script};
    use crate::scripts::{load_input_since_code_hash, LOAD_INPUT_SINCE_SCRIPT};
    use crate::vm::{ResolvedCell, ScriptVersion, SimpleDataProvider, TransactionScriptVerifier};
    use std::sync::Arc;

    const EXPECTED_SINCE: u64 = 0x1122_3344_5566_7788;

    fn build_provider(code_hash: [u8; 32], input_out_point: OutPoint) -> SimpleDataProvider {
        let mut provider = SimpleDataProvider::new();
        provider.add_script(code_hash, LOAD_INPUT_SINCE_SCRIPT.to_vec());
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
    fn test_load_input_since_verification() {
        let code_hash = load_input_since_code_hash();
        let input_out_point = OutPoint::new([0x44; 32], 0);
        let provider = build_provider(code_hash, input_out_point);
        let tx = CellTx {
            version: 0xC001,
            inputs: vec![CellInput::new(input_out_point, EXPECTED_SINCE)],
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
    fn test_load_input_since_rejects_wrong_since() {
        let code_hash = load_input_since_code_hash();
        let input_out_point = OutPoint::new([0x55; 32], 0);
        let provider = build_provider(code_hash, input_out_point);
        let tx = CellTx {
            version: 0xC001,
            inputs: vec![CellInput::new(input_out_point, 7)],
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
