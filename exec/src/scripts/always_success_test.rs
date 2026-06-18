// SPDX-License-Identifier: MIT
// Copyright (C) 2026 Myelin developers
//
// Always-success lock script test

#[cfg(all(test, feature = "vm"))]
mod tests {
    use crate::celltx::{CellInput, CellOutput, CellTx, OutPoint, Script};
    use crate::scripts::{always_success_code_hash, ALWAYS_SUCCESS_SCRIPT};
    use crate::vm::{ResolvedCell, ScriptVersion, SimpleDataProvider, TransactionScriptVerifier};
    use std::sync::Arc;

    #[test]
    fn test_always_success_verification() {
        // Create data provider with always-success script
        let mut provider = SimpleDataProvider::new();
        let code_hash = always_success_code_hash();
        provider.add_script(code_hash, ALWAYS_SUCCESS_SCRIPT.to_vec());
        let input_out_point = OutPoint::new([0x11; 32], 0);
        provider.add_cell(
            input_out_point.tx_hash,
            input_out_point.index,
            ResolvedCell {
                cell_output: CellOutput { capacity: 1000, lock: Script { code_hash, hash_type: 0, args: vec![] }, type_: None },
                data: Some(vec![]),
            },
        );

        // Create transaction spending an input protected by the always-success lock
        let tx = CellTx {
            version: 0xC001,
            inputs: vec![CellInput::new(input_out_point, 0)],
            cell_deps: vec![],
            header_deps: vec![],
            outputs: vec![CellOutput { capacity: 1000, lock: Script { code_hash, hash_type: 0, args: vec![] }, type_: None }],
            outputs_data: vec![vec![]],
            witnesses: vec![],
        };

        // Create verifier
        let verifier =
            TransactionScriptVerifier::new(Arc::new(tx), Arc::new(provider)).with_version(ScriptVersion::V2).with_max_cycles(10_000);

        let result = verifier.verify();
        assert!(result.is_ok(), "Verifier should execute the ELF always-success fixture: {:?}", result);
    }

    #[test]
    fn test_script_not_found() {
        let mut provider = SimpleDataProvider::new();
        let input_out_point = OutPoint::new([0x22; 32], 0);
        provider.add_cell(
            input_out_point.tx_hash,
            input_out_point.index,
            ResolvedCell {
                cell_output: CellOutput {
                    capacity: 1000,
                    lock: Script { code_hash: [0xFF; 32], hash_type: 0, args: vec![] },
                    type_: None,
                },
                data: Some(vec![]),
            },
        );
        // Don't add the referenced script bytes

        let tx = CellTx {
            version: 0xC001,
            inputs: vec![CellInput::new(input_out_point, 0)],
            cell_deps: vec![],
            header_deps: vec![],
            outputs: vec![CellOutput {
                capacity: 1000,
                lock: Script {
                    code_hash: [0xFF; 32], // Non-existent script
                    hash_type: 0,
                    args: vec![],
                },
                type_: None,
            }],
            outputs_data: vec![vec![]],
            witnesses: vec![],
        };

        let verifier = TransactionScriptVerifier::new(Arc::new(tx), Arc::new(provider));

        // Should fail with ScriptNotFound
        let result = verifier.verify();
        assert!(result.is_err());
    }
}
