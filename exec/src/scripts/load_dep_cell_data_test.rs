// SPDX-License-Identifier: MIT
// Copyright (C) 2026 Myelin developers
//
// Load-dep-cell-data lock script test

#[cfg(all(test, feature = "vm"))]
mod tests {
    use crate::celltx::{CellDep, CellInput, CellOutput, CellTx, DepType, OutPoint, Script};
    use crate::scripts::{load_dep_cell_data_code_hash, LOAD_DEP_CELL_DATA_SCRIPT};
    use crate::vm::{ResolvedCell, ScriptVersion, SimpleDataProvider, TransactionScriptVerifier};
    use std::sync::Arc;

    const EXPECTED_DATA: [u8; 4] = [0xDE, 0xAD, 0xBE, 0xEF];

    fn build_provider(
        code_hash: [u8; 32],
        input_out_point: OutPoint,
        dep_out_point: OutPoint,
        dep_data: Vec<u8>,
    ) -> SimpleDataProvider {
        let mut provider = SimpleDataProvider::new();
        provider.add_script(code_hash, LOAD_DEP_CELL_DATA_SCRIPT.to_vec());
        provider.add_cell(
            input_out_point.tx_hash,
            input_out_point.index,
            ResolvedCell {
                cell_output: CellOutput { capacity: 1000, lock: Script { code_hash, hash_type: 0, args: vec![] }, type_: None },
                data: Some(vec![]),
            },
        );
        provider.add_cell(
            dep_out_point.tx_hash,
            dep_out_point.index,
            ResolvedCell {
                cell_output: CellOutput {
                    capacity: 2000,
                    lock: Script { code_hash: [0xAB; 32], hash_type: 0, args: vec![] },
                    type_: None,
                },
                data: Some(dep_data),
            },
        );
        provider
    }

    #[test]
    fn test_load_dep_cell_data_verification() {
        let code_hash = load_dep_cell_data_code_hash();
        let input_out_point = OutPoint::new([0x81; 32], 0);
        let dep_out_point = OutPoint::new([0x82; 32], 1);
        let provider = build_provider(code_hash, input_out_point, dep_out_point, EXPECTED_DATA.to_vec());
        let tx = CellTx {
            version: 0xC001,
            inputs: vec![CellInput::new(input_out_point, 0)],
            cell_deps: vec![CellDep { out_point: dep_out_point, dep_type: DepType::Code }],
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
    fn test_load_dep_cell_data_rejects_wrong_dep_data() {
        let code_hash = load_dep_cell_data_code_hash();
        let input_out_point = OutPoint::new([0x83; 32], 0);
        let dep_out_point = OutPoint::new([0x84; 32], 1);
        let provider = build_provider(code_hash, input_out_point, dep_out_point, vec![0x00, 0x01, 0x02, 0x03]);
        let tx = CellTx {
            version: 0xC001,
            inputs: vec![CellInput::new(input_out_point, 0)],
            cell_deps: vec![CellDep { out_point: dep_out_point, dep_type: DepType::Code }],
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
