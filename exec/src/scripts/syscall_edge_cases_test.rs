// SPDX-License-Identifier: MIT
// Copyright (C) 2026 Myelin developers
//
// Edge-case syscall scenario tests
//
// Covers boundary conditions and cross-syscall combinations that are not
// exercised by the per-script integration tests:
//   - LOAD_CELL_DATA offset == data length / offset > data length
//   - LOAD_WITNESS with empty and oversized witnesses
//   - LOAD_HEADER with invalid index and Source::HeaderDep path
//   - Multi-syscall combination in a single test context

#[cfg(all(test, feature = "vm"))]
mod tests {
    use crate::celltx::{CellDep, CellInput, CellOutput, CellTx, DepType, OutPoint, Script};
    use crate::vm::syscalls::*;
    use crate::vm::{ResolvedCell, ResolvedHeader, ScriptVersion, SimpleDataProvider};
    use ckb_vm::{
        registers::{A0, A1, A2, A3, A4, A5, A7},
        CoreMachine, Memory, Register, Syscalls,
    };
    use std::sync::Arc;

    const BUFFER_ADDR: u64 = 0x1000;
    const SIZE_ADDR: u64 = 0x2000;

    // -----------------------------------------------------------------------
    // LOAD_CELL_DATA boundary conditions
    // -----------------------------------------------------------------------

    /// When offset == data length, zero bytes should be returned (SUCCESS).
    #[test]
    fn test_load_cell_data_offset_equals_data_length() {
        let dep_out_point = OutPoint::new([0xA1; 32], 0);
        let tx = Arc::new(CellTx {
            version: 0xC001,
            inputs: vec![],
            cell_deps: vec![CellDep { out_point: dep_out_point, dep_type: DepType::Code }],
            header_deps: vec![],
            outputs: vec![],
            outputs_data: vec![],
            witnesses: vec![],
        });
        let mut provider = SimpleDataProvider::new();
        provider.add_cell(
            dep_out_point.tx_hash,
            dep_out_point.index,
            ResolvedCell {
                cell_output: CellOutput { capacity: 1000, lock: Script::new([1u8; 32], 0, vec![]), type_: None },
                data: Some(vec![0x10, 0x20, 0x30]),
            },
        );

        let mut machine = ScriptVersion::V2.init_core_machine(10_000);
        machine.memory_mut().store64(&SIZE_ADDR, &8u64).unwrap();
        machine.set_register(A0, BUFFER_ADDR);
        machine.set_register(A1, SIZE_ADDR);
        machine.set_register(A2, 3); // offset == data length
        machine.set_register(A3, 0);
        machine.set_register(A4, Source::CellDep as u64);
        machine.set_register(A7, LOAD_CELL_DATA_SYSCALL_NUMBER);

        let mut syscall = LoadCellData::new(tx, Arc::new(provider), vec![], vec![]);
        let handled = syscall.ecall(&mut machine).expect("ecall should succeed");

        assert!(handled);
        assert_eq!(machine.registers()[A0].to_u64(), SUCCESS as u64);
        // full_size should be 0 (3 - min(3,3) == 0)
        assert_eq!(machine.memory_mut().load64(&SIZE_ADDR).unwrap().to_u64(), 0);
    }

    /// When offset > data length, it is clamped; zero bytes returned (SUCCESS).
    #[test]
    fn test_load_cell_data_offset_exceeds_data_length() {
        let dep_out_point = OutPoint::new([0xA2; 32], 0);
        let tx = Arc::new(CellTx {
            version: 0xC001,
            inputs: vec![],
            cell_deps: vec![CellDep { out_point: dep_out_point, dep_type: DepType::Code }],
            header_deps: vec![],
            outputs: vec![],
            outputs_data: vec![],
            witnesses: vec![],
        });
        let mut provider = SimpleDataProvider::new();
        provider.add_cell(
            dep_out_point.tx_hash,
            dep_out_point.index,
            ResolvedCell {
                cell_output: CellOutput { capacity: 1000, lock: Script::new([1u8; 32], 0, vec![]), type_: None },
                data: Some(vec![0x10, 0x20]),
            },
        );

        let mut machine = ScriptVersion::V2.init_core_machine(10_000);
        machine.memory_mut().store64(&SIZE_ADDR, &8u64).unwrap();
        machine.set_register(A0, BUFFER_ADDR);
        machine.set_register(A1, SIZE_ADDR);
        machine.set_register(A2, 100); // offset far beyond data length
        machine.set_register(A3, 0);
        machine.set_register(A4, Source::CellDep as u64);
        machine.set_register(A7, LOAD_CELL_DATA_SYSCALL_NUMBER);

        let mut syscall = LoadCellData::new(tx, Arc::new(provider), vec![], vec![]);
        let handled = syscall.ecall(&mut machine).expect("ecall should succeed");

        assert!(handled);
        assert_eq!(machine.registers()[A0].to_u64(), SUCCESS as u64);
        assert_eq!(machine.memory_mut().load64(&SIZE_ADDR).unwrap().to_u64(), 0);
    }

    // -----------------------------------------------------------------------
    // LOAD_WITNESS boundary conditions
    // -----------------------------------------------------------------------

    /// Empty witness should be loaded successfully with size == 0.
    #[test]
    fn test_load_witness_empty_witness() {
        let tx = Arc::new(CellTx {
            version: 0xC001,
            inputs: vec![CellInput::new(OutPoint::new([0xB1; 32], 0), 0)],
            cell_deps: vec![],
            header_deps: vec![],
            outputs: vec![],
            outputs_data: vec![],
            witnesses: vec![vec![]], // empty witness
        });

        let mut machine = ScriptVersion::V2.init_core_machine(10_000);
        machine.memory_mut().store64(&SIZE_ADDR, &8u64).unwrap();
        machine.set_register(A0, BUFFER_ADDR);
        machine.set_register(A1, SIZE_ADDR);
        machine.set_register(A2, 0);
        machine.set_register(A3, 0);
        machine.set_register(A4, Source::Input as u64);
        machine.set_register(A7, LOAD_WITNESS_SYSCALL_NUMBER);

        let mut syscall = LoadWitness::new(tx, vec![0], vec![]);
        let handled = syscall.ecall(&mut machine).expect("ecall should succeed");

        assert!(handled);
        assert_eq!(machine.registers()[A0].to_u64(), SUCCESS as u64);
        assert_eq!(machine.memory_mut().load64(&SIZE_ADDR).unwrap().to_u64(), 0);
    }

    /// Large witness: only the requested portion (buffer size) is copied.
    #[test]
    fn test_load_witness_large_witness_partial_read() {
        let large_witness = vec![0xAA; 4096];
        let tx = Arc::new(CellTx {
            version: 0xC001,
            inputs: vec![CellInput::new(OutPoint::new([0xB2; 32], 0), 0)],
            cell_deps: vec![],
            header_deps: vec![],
            outputs: vec![],
            outputs_data: vec![],
            witnesses: vec![large_witness.clone()],
        });

        let buf_size = 16u64;
        let mut machine = ScriptVersion::V2.init_core_machine(50_000);
        machine.memory_mut().store64(&SIZE_ADDR, &buf_size).unwrap();
        machine.set_register(A0, BUFFER_ADDR);
        machine.set_register(A1, SIZE_ADDR);
        machine.set_register(A2, 0);
        machine.set_register(A3, 0);
        machine.set_register(A4, Source::Input as u64);
        machine.set_register(A7, LOAD_WITNESS_SYSCALL_NUMBER);

        let mut syscall = LoadWitness::new(tx, vec![0], vec![]);
        let handled = syscall.ecall(&mut machine).expect("ecall should succeed");

        assert!(handled);
        assert_eq!(machine.registers()[A0].to_u64(), SUCCESS as u64);
        // full_size is written to SIZE_ADDR (total remaining after offset)
        assert_eq!(machine.memory_mut().load64(&SIZE_ADDR).unwrap().to_u64(), 4096);
        // Only buf_size bytes are actually written
        let written = machine.memory_mut().load_bytes(BUFFER_ADDR, buf_size).unwrap();
        assert_eq!(written.as_ref(), &[0xAA; 16]);
    }

    /// Loading witness for Source::HeaderDep returns INDEX_OUT_OF_BOUND.
    #[test]
    fn test_load_witness_header_dep_returns_index_out_of_bound() {
        let tx = Arc::new(CellTx {
            version: 0xC001,
            inputs: vec![],
            cell_deps: vec![],
            header_deps: vec![[0xDD; 32]],
            outputs: vec![],
            outputs_data: vec![],
            witnesses: vec![],
        });

        let mut machine = ScriptVersion::V2.init_core_machine(10_000);
        machine.memory_mut().store64(&SIZE_ADDR, &8u64).unwrap();
        machine.set_register(A0, BUFFER_ADDR);
        machine.set_register(A1, SIZE_ADDR);
        machine.set_register(A2, 0);
        machine.set_register(A3, 0);
        machine.set_register(A4, Source::HeaderDep as u64);
        machine.set_register(A7, LOAD_WITNESS_SYSCALL_NUMBER);

        let mut syscall = LoadWitness::new(tx, vec![], vec![]);
        let handled = syscall.ecall(&mut machine).expect("ecall should succeed");

        assert!(handled);
        assert_eq!(machine.registers()[A0].to_u64(), INDEX_OUT_OF_BOUND as u64);
    }

    // -----------------------------------------------------------------------
    // LOAD_HEADER boundary conditions
    // -----------------------------------------------------------------------

    fn test_header(hash: [u8; 32]) -> ResolvedHeader {
        ResolvedHeader {
            hash,
            version: 1,
            parent_hash: [0x99; 32],
            transactions_root: [0x11; 32],
            proposals_hash: [0x12; 32],
            cell_commitment: [0x13; 32],
            cell_root: [0x14; 32],
            segment_root: [0x15; 32],
            timestamp: 1_700_000_000,
            compact_target: 0x1d00_ffff,
            nonce: 42,
            number: 100,
            dao: [0x16; 32],
            epoch: 7,
            uncles_hash: [0x17; 32],
        }
    }

    /// Invalid index (beyond header_deps length) returns INDEX_OUT_OF_BOUND.
    #[test]
    fn test_load_header_invalid_index_returns_index_out_of_bound() {
        let tx = Arc::new(CellTx {
            version: 0xC001,
            inputs: vec![],
            cell_deps: vec![],
            header_deps: vec![[0xC1; 32]],
            outputs: vec![],
            outputs_data: vec![],
            witnesses: vec![],
        });
        let mut provider = SimpleDataProvider::new();
        provider.add_header([0xC1; 32], test_header([0xC1; 32]));

        let mut machine = ScriptVersion::V2.init_core_machine(10_000);
        machine.memory_mut().store64(&SIZE_ADDR, &8u64).unwrap();
        machine.set_register(A0, BUFFER_ADDR);
        machine.set_register(A1, SIZE_ADDR);
        machine.set_register(A2, 0);
        machine.set_register(A3, 5); // index 5, only 1 header dep
        machine.set_register(A4, Source::HeaderDep as u64);
        machine.set_register(A7, LOAD_HEADER_SYSCALL_NUMBER);

        let mut syscall = LoadHeader::new(tx, Arc::new(provider), vec![], vec![]);
        let handled = syscall.ecall(&mut machine).expect("ecall should succeed");

        assert!(handled);
        assert_eq!(machine.registers()[A0].to_u64(), INDEX_OUT_OF_BOUND as u64);
    }

    /// Load header via Source::HeaderDep by field (timestamp).
    #[test]
    fn test_load_header_header_dep_by_field_timestamp() {
        let header_hash = [0xC2; 32];
        let tx = Arc::new(CellTx {
            version: 0xC001,
            inputs: vec![],
            cell_deps: vec![],
            header_deps: vec![header_hash],
            outputs: vec![],
            outputs_data: vec![],
            witnesses: vec![],
        });
        let header = test_header(header_hash);
        let mut provider = SimpleDataProvider::new();
        provider.add_header(header_hash, header.clone());

        let mut machine = ScriptVersion::V2.init_core_machine(10_000);
        machine.memory_mut().store64(&SIZE_ADDR, &8u64).unwrap();
        machine.set_register(A0, BUFFER_ADDR);
        machine.set_register(A1, SIZE_ADDR);
        machine.set_register(A2, 0);
        machine.set_register(A3, 0);
        machine.set_register(A4, Source::HeaderDep as u64);
        machine.set_register(A5, HeaderField::Timestamp as u64);
        machine.set_register(A7, LOAD_HEADER_BY_FIELD_SYSCALL_NUMBER);

        let mut syscall = LoadHeader::new(tx, Arc::new(provider), vec![], vec![]);
        let handled = syscall.ecall(&mut machine).expect("ecall should succeed");

        assert!(handled);
        assert_eq!(machine.registers()[A0].to_u64(), SUCCESS as u64);
        assert_eq!(machine.memory_mut().load_bytes(BUFFER_ADDR, 8).unwrap().as_ref(), &header.timestamp.to_le_bytes());
    }

    // -----------------------------------------------------------------------
    // Multi-syscall combination
    // -----------------------------------------------------------------------

    /// Exercise multiple syscall handlers against the same transaction context:
    /// LOAD_CELL_BY_FIELD (capacity), LOAD_CELL_DATA, LOAD_WITNESS, LOAD_HEADER_BY_FIELD.
    #[test]
    fn test_multi_syscall_combination() {
        let input_out_point = OutPoint::new([0xD1; 32], 0);
        let header_hash = [0xD2; 32];
        let cell_data = vec![0xCA, 0xFE, 0xBA, 0xBE];
        let witness_data = vec![0x01, 0x02, 0x03];

        let tx = Arc::new(CellTx {
            version: 0xC001,
            inputs: vec![CellInput::new(input_out_point, 0)],
            cell_deps: vec![],
            header_deps: vec![header_hash],
            outputs: vec![],
            outputs_data: vec![],
            witnesses: vec![witness_data.clone()],
        });

        let capacity_val = 42_000u64;
        let mut provider = SimpleDataProvider::new();
        provider.add_cell(
            input_out_point.tx_hash,
            input_out_point.index,
            ResolvedCell {
                cell_output: CellOutput { capacity: capacity_val, lock: Script::new([0xE1; 32], 0, vec![]), type_: None },
                data: Some(cell_data.clone()),
            },
        );
        let header = test_header(header_hash);
        provider.add_header(header_hash, header.clone());

        let provider = Arc::new(provider);

        // --- 1) LOAD_CELL_BY_FIELD: read capacity of input 0 ---
        {
            let mut machine = ScriptVersion::V2.init_core_machine(10_000);
            machine.memory_mut().store64(&SIZE_ADDR, &8u64).unwrap();
            machine.set_register(A0, BUFFER_ADDR);
            machine.set_register(A1, SIZE_ADDR);
            machine.set_register(A2, 0);
            machine.set_register(A3, 0);
            machine.set_register(A4, Source::Input as u64);
            machine.set_register(A5, CellField::Capacity as u64);
            machine.set_register(A7, LOAD_CELL_BY_FIELD_SYSCALL_NUMBER);

            let mut syscall = LoadCell::new(tx.clone(), provider.clone(), vec![0], vec![]);
            let handled = syscall.ecall(&mut machine).unwrap();
            assert!(handled);
            assert_eq!(machine.registers()[A0].to_u64(), SUCCESS as u64);
            assert_eq!(machine.memory_mut().load_bytes(BUFFER_ADDR, 8).unwrap().as_ref(), &capacity_val.to_le_bytes());
        }

        // --- 2) LOAD_CELL_DATA: read cell data of input 0 ---
        {
            let mut machine = ScriptVersion::V2.init_core_machine(10_000);
            machine.memory_mut().store64(&SIZE_ADDR, &8u64).unwrap();
            machine.set_register(A0, BUFFER_ADDR);
            machine.set_register(A1, SIZE_ADDR);
            machine.set_register(A2, 0);
            machine.set_register(A3, 0);
            machine.set_register(A4, Source::Input as u64);
            machine.set_register(A7, LOAD_CELL_DATA_SYSCALL_NUMBER);

            let mut syscall = LoadCellData::new(tx.clone(), provider.clone(), vec![0], vec![]);
            let handled = syscall.ecall(&mut machine).unwrap();
            assert!(handled);
            assert_eq!(machine.registers()[A0].to_u64(), SUCCESS as u64);
            assert_eq!(machine.memory_mut().load64(&SIZE_ADDR).unwrap().to_u64(), cell_data.len() as u64);
            assert_eq!(machine.memory_mut().load_bytes(BUFFER_ADDR, cell_data.len() as u64).unwrap().as_ref(), cell_data.as_slice());
        }

        // --- 3) LOAD_WITNESS: read witness of input 0 ---
        {
            let mut machine = ScriptVersion::V2.init_core_machine(10_000);
            machine.memory_mut().store64(&SIZE_ADDR, &8u64).unwrap();
            machine.set_register(A0, BUFFER_ADDR);
            machine.set_register(A1, SIZE_ADDR);
            machine.set_register(A2, 0);
            machine.set_register(A3, 0);
            machine.set_register(A4, Source::Input as u64);
            machine.set_register(A7, LOAD_WITNESS_SYSCALL_NUMBER);

            let mut syscall = LoadWitness::new(tx.clone(), vec![0], vec![]);
            let handled = syscall.ecall(&mut machine).unwrap();
            assert!(handled);
            assert_eq!(machine.registers()[A0].to_u64(), SUCCESS as u64);
            assert_eq!(
                machine.memory_mut().load_bytes(BUFFER_ADDR, witness_data.len() as u64).unwrap().as_ref(),
                witness_data.as_slice()
            );
        }

        // --- 4) LOAD_HEADER_BY_FIELD: read block_number via HeaderDep ---
        {
            let mut machine = ScriptVersion::V2.init_core_machine(10_000);
            machine.memory_mut().store64(&SIZE_ADDR, &8u64).unwrap();
            machine.set_register(A0, BUFFER_ADDR);
            machine.set_register(A1, SIZE_ADDR);
            machine.set_register(A2, 0);
            machine.set_register(A3, 0);
            machine.set_register(A4, Source::HeaderDep as u64);
            machine.set_register(A5, HeaderField::Number as u64);
            machine.set_register(A7, LOAD_HEADER_BY_FIELD_SYSCALL_NUMBER);

            let mut syscall = LoadHeader::new(tx.clone(), provider.clone(), vec![0], vec![]);
            let handled = syscall.ecall(&mut machine).unwrap();
            assert!(handled);
            assert_eq!(machine.registers()[A0].to_u64(), SUCCESS as u64);
            assert_eq!(machine.memory_mut().load_bytes(BUFFER_ADDR, 8).unwrap().as_ref(), &header.number.to_le_bytes());
        }
    }

    // -----------------------------------------------------------------------
    // LOAD_CELL via HeaderDep in load_cell_data
    // -----------------------------------------------------------------------

    /// LOAD_CELL_DATA with Source::HeaderDep loads the cell associated with
    /// the header hash at the given index.
    #[test]
    fn test_load_cell_data_header_dep_source() {
        let header_hash = [0xE1; 32];
        let cell_data = vec![0xBE, 0xEF];
        let tx = Arc::new(CellTx {
            version: 0xC001,
            inputs: vec![],
            cell_deps: vec![],
            header_deps: vec![header_hash],
            outputs: vec![],
            outputs_data: vec![],
            witnesses: vec![],
        });
        let mut provider = SimpleDataProvider::new();
        provider.add_cell_by_header(
            header_hash,
            ResolvedCell {
                cell_output: CellOutput { capacity: 7000, lock: Script::new([7u8; 32], 0, vec![]), type_: None },
                data: Some(cell_data.clone()),
            },
        );

        let mut machine = ScriptVersion::V2.init_core_machine(10_000);
        machine.memory_mut().store64(&SIZE_ADDR, &8u64).unwrap();
        machine.set_register(A0, BUFFER_ADDR);
        machine.set_register(A1, SIZE_ADDR);
        machine.set_register(A2, 0);
        machine.set_register(A3, 0);
        machine.set_register(A4, Source::HeaderDep as u64);
        machine.set_register(A7, LOAD_CELL_DATA_SYSCALL_NUMBER);

        let mut syscall = LoadCellData::new(tx, Arc::new(provider), vec![], vec![]);
        let handled = syscall.ecall(&mut machine).expect("ecall should succeed");

        assert!(handled);
        assert_eq!(machine.registers()[A0].to_u64(), SUCCESS as u64);
        assert_eq!(machine.memory_mut().load64(&SIZE_ADDR).unwrap().to_u64(), cell_data.len() as u64);
        assert_eq!(machine.memory_mut().load_bytes(BUFFER_ADDR, cell_data.len() as u64).unwrap().as_ref(), cell_data.as_slice());
    }

    /// LOAD_CELL_DATA with Source::HeaderDep returns ITEM_MISSING when no cell
    /// is associated with the header hash.
    #[test]
    fn test_load_cell_data_header_dep_item_missing() {
        let header_hash = [0xE2; 32];
        let tx = Arc::new(CellTx {
            version: 0xC001,
            inputs: vec![],
            cell_deps: vec![],
            header_deps: vec![header_hash],
            outputs: vec![],
            outputs_data: vec![],
            witnesses: vec![],
        });
        let provider = Arc::new(SimpleDataProvider::new());

        let mut machine = ScriptVersion::V2.init_core_machine(10_000);
        machine.memory_mut().store64(&SIZE_ADDR, &8u64).unwrap();
        machine.set_register(A0, BUFFER_ADDR);
        machine.set_register(A1, SIZE_ADDR);
        machine.set_register(A2, 0);
        machine.set_register(A3, 0);
        machine.set_register(A4, Source::HeaderDep as u64);
        machine.set_register(A7, LOAD_CELL_DATA_SYSCALL_NUMBER);

        let mut syscall = LoadCellData::new(tx, provider, vec![], vec![]);
        let handled = syscall.ecall(&mut machine).expect("ecall should succeed");

        assert!(handled);
        assert_eq!(machine.registers()[A0].to_u64(), ITEM_MISSING as u64);
    }
}
