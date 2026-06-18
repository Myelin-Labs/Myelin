// SPDX-License-Identifier: MIT
// Copyright (C) 2026 Spora developers
//
// Minimal HTLC witness-loading script test

#[cfg(all(test, feature = "vm"))]
mod tests {
    use crate::celltx::{CellInput, CellOutput, CellTx, OutPoint, Script};
    use crate::scripts::{htlc_minimal_code_hash, HTLC_MINIMAL_SCRIPT};
    use crate::vm::{ResolvedCell, ScriptVersion, SimpleDataProvider, TransactionScriptVerifier};
    use std::sync::Arc;

    fn build_provider(code_hash: [u8; 32], input_out_point: OutPoint) -> SimpleDataProvider {
        let mut provider = SimpleDataProvider::new();
        provider.add_script(code_hash, HTLC_MINIMAL_SCRIPT.to_vec());
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
    fn test_htlc_minimal_loads_group_witness() {
        let code_hash = htlc_minimal_code_hash();
        let input_out_point = OutPoint::new([0x61; 32], 0);
        let provider = build_provider(code_hash, input_out_point.clone());
        let tx = CellTx {
            version: 0xC001,
            inputs: vec![CellInput::new(input_out_point, 0)],
            cell_deps: vec![],
            header_deps: vec![],
            outputs: vec![],
            outputs_data: vec![],
            witnesses: vec![vec![0xAB]],
        };

        let verifier =
            TransactionScriptVerifier::new(Arc::new(tx), Arc::new(provider)).with_version(ScriptVersion::V2).with_max_cycles(20_000);

        assert!(verifier.verify().is_ok());
    }

    #[test]
    fn test_htlc_minimal_rejects_missing_group_witness() {
        let code_hash = htlc_minimal_code_hash();
        let input_out_point = OutPoint::new([0x62; 32], 0);
        let provider = build_provider(code_hash, input_out_point.clone());
        let tx = CellTx {
            version: 0xC001,
            inputs: vec![CellInput::new(input_out_point, 0)],
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
