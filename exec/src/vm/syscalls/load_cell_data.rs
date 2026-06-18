// SPDX-License-Identifier: MIT
// Copyright (C) 2026 Spora developers
//
// Load cell data syscall

use super::utils::{store_data, INDEX_OUT_OF_BOUND, ITEM_MISSING, SLICE_OUT_OF_BOUND};
use super::Source;
use super::{LOAD_CELL_DATA_AS_CODE_SYSCALL_NUMBER, LOAD_CELL_DATA_SYSCALL_NUMBER, SUCCESS};
use crate::celltx::CellTx;
use crate::vm::transferred_byte_cycles;
use crate::vm::{CellDataProvider, VmSemantics};
use ckb_vm::{
    memory::{Memory, FLAG_EXECUTABLE, FLAG_FREEZED},
    registers::{A0, A1, A2, A3, A4, A5, A7},
    Bytes, Error as VMError, Register, SupportMachine, Syscalls,
};
use std::sync::Arc;

/// Syscall: Load Cell Data
///
/// Syscall number: 2092
pub struct LoadCellData<D: CellDataProvider> {
    tx: Arc<CellTx>,
    provider: Arc<D>,
    group_input_indices: Vec<usize>,
    group_output_indices: Vec<usize>,
    semantics: VmSemantics,
}

enum CellDataLookupResult {
    Data(Vec<u8>),
    IndexOutOfBound,
    ItemMissing,
}

impl<D: CellDataProvider> LoadCellData<D> {
    pub fn new(tx: Arc<CellTx>, provider: Arc<D>, group_input_indices: Vec<usize>, group_output_indices: Vec<usize>) -> Self {
        Self { tx, provider, group_input_indices, group_output_indices, semantics: VmSemantics::SporaExtended }
    }

    pub fn with_semantics(mut self, semantics: VmSemantics) -> Self {
        self.semantics = semantics;
        self
    }

    fn get_cell_data(&self, source: Source, index: usize) -> CellDataLookupResult {
        match source {
            Source::Input => match self.tx.inputs.get(index) {
                Some(input) => self
                    .provider
                    .load_cell_by_outpoint(&input.previous_output.tx_hash, input.previous_output.index)
                    .map(|cell| CellDataLookupResult::Data(cell.data.unwrap_or_default()))
                    .unwrap_or(CellDataLookupResult::ItemMissing),
                None => CellDataLookupResult::IndexOutOfBound,
            },
            Source::Output => {
                if self.tx.outputs.get(index).is_none() {
                    CellDataLookupResult::IndexOutOfBound
                } else {
                    self.tx
                        .outputs_data
                        .get(index)
                        .cloned()
                        .map(CellDataLookupResult::Data)
                        .unwrap_or(CellDataLookupResult::ItemMissing)
                }
            }
            Source::CellDep => match self.tx.cell_deps.get(index) {
                Some(dep) => self
                    .provider
                    .load_cell_by_outpoint(&dep.out_point.tx_hash, dep.out_point.index)
                    .map(|cell| CellDataLookupResult::Data(cell.data.unwrap_or_default()))
                    .unwrap_or(CellDataLookupResult::ItemMissing),
                None => CellDataLookupResult::IndexOutOfBound,
            },
            Source::GroupInput => match self.group_input_indices.get(index).and_then(|&idx| self.tx.inputs.get(idx)) {
                Some(input) => self
                    .provider
                    .load_cell_by_outpoint(&input.previous_output.tx_hash, input.previous_output.index)
                    .map(|cell| CellDataLookupResult::Data(cell.data.unwrap_or_default()))
                    .unwrap_or(CellDataLookupResult::ItemMissing),
                None => CellDataLookupResult::IndexOutOfBound,
            },
            Source::HeaderDep => match self.tx.header_deps.get(index) {
                Some(hash) if self.semantics.allow_header_dep_cell_lookup() => self
                    .provider
                    .load_cell_by_header(hash)
                    .map(|cell| CellDataLookupResult::Data(cell.data.unwrap_or_default()))
                    .unwrap_or(CellDataLookupResult::ItemMissing),
                Some(_) => CellDataLookupResult::IndexOutOfBound,
                None => CellDataLookupResult::IndexOutOfBound,
            },
            Source::GroupOutput => match self.group_output_indices.get(index).copied() {
                Some(output_index) => {
                    if self.tx.outputs.get(output_index).is_none() {
                        CellDataLookupResult::IndexOutOfBound
                    } else {
                        self.tx
                            .outputs_data
                            .get(output_index)
                            .cloned()
                            .map(CellDataLookupResult::Data)
                            .unwrap_or(CellDataLookupResult::ItemMissing)
                    }
                }
                None => CellDataLookupResult::IndexOutOfBound,
            },
            Source::GroupCellDep | Source::GroupHeaderDep => CellDataLookupResult::IndexOutOfBound,
        }
    }

    fn load_cell_data_as_code<M: SupportMachine>(&self, machine: &mut M) -> Result<(), VMError> {
        let addr = machine.registers()[A0].to_u64();
        let memory_size = machine.registers()[A1].to_u64();
        let content_offset = machine.registers()[A2].to_u64();
        let content_size = machine.registers()[A3].to_u64();
        let index = machine.registers()[A4].to_u64() as usize;
        let source = Source::parse_from_u64_for_semantics(machine.registers()[A5].to_u64(), self.semantics)?;

        let cell_data = match self.get_cell_data(source, index) {
            CellDataLookupResult::Data(data) => data,
            CellDataLookupResult::IndexOutOfBound => {
                machine.set_register(A0, M::REG::from_u8(INDEX_OUT_OF_BOUND));
                return Ok(());
            }
            CellDataLookupResult::ItemMissing => {
                machine.set_register(A0, M::REG::from_u8(ITEM_MISSING));
                return Ok(());
            }
        };

        let content_end = match content_offset.checked_add(content_size) {
            Some(end) => end,
            None => {
                machine.set_register(A0, M::REG::from_u8(SLICE_OUT_OF_BOUND));
                return Ok(());
            }
        };

        if content_offset >= cell_data.len() as u64 || content_end > cell_data.len() as u64 || content_size > memory_size {
            machine.set_register(A0, M::REG::from_u8(SLICE_OUT_OF_BOUND));
            return Ok(());
        }

        let source = if content_size == 0 {
            None
        } else {
            Some(Bytes::copy_from_slice(&cell_data[content_offset as usize..content_end as usize]))
        };

        machine.memory_mut().init_pages(addr, memory_size, FLAG_EXECUTABLE | FLAG_FREEZED, source, 0)?;
        let billed_size = usize::try_from(memory_size).map_err(|_| VMError::MemOutOfBound)?;
        machine.add_cycles_no_checking(transferred_byte_cycles(billed_size))?;
        machine.set_register(A0, M::REG::from_u8(SUCCESS));
        Ok(())
    }
}

impl<D: CellDataProvider, M: SupportMachine> Syscalls<M> for LoadCellData<D> {
    fn initialize(&mut self, _machine: &mut M) -> Result<(), VMError> {
        Ok(())
    }

    fn ecall(&mut self, machine: &mut M) -> Result<bool, VMError> {
        let syscall_number = machine.registers()[A7].to_u64();

        // LOAD_CELL_DATA_AS_CODE = 2091, LOAD_CELL_DATA = 2092
        if syscall_number == LOAD_CELL_DATA_AS_CODE_SYSCALL_NUMBER {
            self.load_cell_data_as_code(machine)?;
            return Ok(true);
        }
        if syscall_number != LOAD_CELL_DATA_SYSCALL_NUMBER {
            return Ok(false);
        }

        let index = machine.registers()[A3].to_u64() as usize;
        let source = Source::parse_from_u64_for_semantics(machine.registers()[A4].to_u64(), self.semantics)?;

        // Get cell data
        let cell_data = match self.get_cell_data(source, index) {
            CellDataLookupResult::Data(data) => data,
            CellDataLookupResult::IndexOutOfBound => {
                machine.set_register(A0, M::REG::from_u8(INDEX_OUT_OF_BOUND));
                return Ok(true);
            }
            CellDataLookupResult::ItemMissing => {
                machine.set_register(A0, M::REG::from_u8(ITEM_MISSING));
                return Ok(true);
            }
        };

        // Store data using CKB-style store_data
        let result = store_data(machine, &cell_data)?;
        machine.add_cycles_no_checking(transferred_byte_cycles(result.written_size))?;
        machine.set_register(A0, M::REG::from_u8(result.return_code));

        Ok(true)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::celltx::{CellDep, CellInput, CellOutput, DepType, OutPoint, Script};
    use crate::vm::syscalls::SUCCESS;
    use crate::vm::{ResolvedCell, ScriptVersion, SimpleDataProvider, VmSemantics};
    use ckb_vm::{
        registers::{A1, A2},
        CoreMachine, Memory, Register,
    };

    const BUFFER_ADDR: u64 = 0x1000;
    const SIZE_ADDR: u64 = 0x2000;

    #[test]
    fn test_load_cell_data_resolves_input_and_dep_sources() {
        let input_out_point = OutPoint::new([7u8; 32], 0);
        let dep_out_point = OutPoint::new([8u8; 32], 1);
        let tx = Arc::new(CellTx {
            version: 0xC001,
            inputs: vec![CellInput::new(input_out_point.clone(), 0)],
            cell_deps: vec![CellDep { out_point: dep_out_point.clone(), dep_type: DepType::Code }],
            header_deps: vec![],
            outputs: vec![CellOutput { capacity: 1000, lock: Script::new([1u8; 32], 0, vec![]), type_: None }],
            outputs_data: vec![vec![0xAA; 10]],
            witnesses: vec![],
        });
        let mut provider = SimpleDataProvider::new();
        provider.add_cell(
            input_out_point.tx_hash,
            input_out_point.index,
            ResolvedCell {
                cell_output: CellOutput { capacity: 2000, lock: Script::new([2u8; 32], 0, vec![0x11]), type_: None },
                data: Some(vec![0x10, 0x20]),
            },
        );
        provider.add_cell(
            dep_out_point.tx_hash,
            dep_out_point.index,
            ResolvedCell {
                cell_output: CellOutput { capacity: 3000, lock: Script::new([3u8; 32], 0, vec![0x22]), type_: None },
                data: Some(vec![0x30, 0x40, 0x50]),
            },
        );

        let syscall = LoadCellData::new(tx, Arc::new(provider), vec![0], vec![0]);
        let input_data = match syscall.get_cell_data(Source::Input, 0) {
            CellDataLookupResult::Data(data) => data,
            _ => panic!("input data should resolve"),
        };
        let dep_data = match syscall.get_cell_data(Source::CellDep, 0) {
            CellDataLookupResult::Data(data) => data,
            _ => panic!("dep data should resolve"),
        };
        let output_data = match syscall.get_cell_data(Source::Output, 0) {
            CellDataLookupResult::Data(data) => data,
            _ => panic!("output data should resolve"),
        };

        assert_eq!(input_data, vec![0x10, 0x20]);
        assert_eq!(dep_data, vec![0x30, 0x40, 0x50]);
        assert_eq!(output_data, vec![0xAA; 10]);
    }

    #[test]
    fn test_load_cell_data_supports_partial_reads() {
        let dep_out_point = OutPoint::new([8u8; 32], 1);
        let tx = Arc::new(CellTx {
            version: 0xC001,
            inputs: vec![],
            cell_deps: vec![CellDep { out_point: dep_out_point.clone(), dep_type: DepType::Code }],
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
                cell_output: CellOutput { capacity: 3000, lock: Script::new([3u8; 32], 0, vec![]), type_: None },
                data: Some(vec![0x30, 0x40, 0x50]),
            },
        );

        let mut machine = ScriptVersion::V2.init_core_machine(10_000);
        machine.memory_mut().store64(&SIZE_ADDR, &2u64).unwrap();
        machine.set_register(A0, BUFFER_ADDR);
        machine.set_register(A1, SIZE_ADDR);
        machine.set_register(A2, 1);
        machine.set_register(A3, 0);
        machine.set_register(A4, 0x03);
        machine.set_register(A7, LOAD_CELL_DATA_SYSCALL_NUMBER);

        let mut syscall = LoadCellData::new(tx, Arc::new(provider), vec![], vec![]);
        let handled = syscall.ecall(&mut machine).expect("load cell data syscall should succeed");

        assert!(handled);
        assert_eq!(machine.registers()[A0].to_u64(), SUCCESS as u64);
        assert_eq!(machine.memory_mut().load64(&SIZE_ADDR).unwrap().to_u64(), 2);
        assert_eq!(machine.memory_mut().load_bytes(BUFFER_ADDR, 2).unwrap().as_ref(), &[0x40, 0x50]);
    }

    #[test]
    fn test_load_cell_data_treats_missing_resolved_payload_as_empty() {
        let input_out_point = OutPoint::new([7u8; 32], 0);
        let tx = Arc::new(CellTx {
            version: 0xC001,
            inputs: vec![CellInput::new(input_out_point.clone(), 0)],
            cell_deps: vec![],
            header_deps: vec![],
            outputs: vec![],
            outputs_data: vec![],
            witnesses: vec![],
        });
        let mut provider = SimpleDataProvider::new();
        provider.add_cell(
            input_out_point.tx_hash,
            input_out_point.index,
            ResolvedCell {
                cell_output: CellOutput { capacity: 2000, lock: Script::new([2u8; 32], 0, vec![]), type_: None },
                data: None,
            },
        );

        let mut machine = ScriptVersion::V2.init_core_machine(10_000);
        machine.memory_mut().store64(&SIZE_ADDR, &8u64).unwrap();
        machine.set_register(A0, BUFFER_ADDR);
        machine.set_register(A1, SIZE_ADDR);
        machine.set_register(A2, 0);
        machine.set_register(A3, 0);
        machine.set_register(A4, Source::Input as u64);
        machine.set_register(A7, LOAD_CELL_DATA_SYSCALL_NUMBER);

        let mut syscall = LoadCellData::new(tx, Arc::new(provider), vec![0], vec![]);
        let handled = syscall.ecall(&mut machine).expect("load cell data syscall should succeed for empty resolved payload");

        assert!(handled);
        assert_eq!(machine.registers()[A0].to_u64(), SUCCESS as u64);
        assert_eq!(machine.memory_mut().load64(&SIZE_ADDR).unwrap().to_u64(), 0);
    }

    #[test]
    fn test_load_cell_data_rejects_invalid_source() {
        let tx = Arc::new(CellTx {
            version: 0xC001,
            inputs: vec![],
            cell_deps: vec![],
            header_deps: vec![],
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
        machine.set_register(A4, 0x99);
        machine.set_register(A7, LOAD_CELL_DATA_SYSCALL_NUMBER);

        let mut syscall = LoadCellData::new(tx, Arc::new(SimpleDataProvider::new()), vec![], vec![]);
        let err = syscall.ecall(&mut machine).expect_err("invalid source should trap");

        assert_eq!(err, VMError::External("SourceEntry parse_from_u64 153".to_string()));
    }

    #[test]
    fn test_load_cell_data_returns_item_missing_when_resolved_input_cell_not_found() {
        let input_out_point = OutPoint::new([7u8; 32], 0);
        let tx = Arc::new(CellTx {
            version: 0xC001,
            inputs: vec![CellInput::new(input_out_point, 0)],
            cell_deps: vec![],
            header_deps: vec![],
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
        machine.set_register(A4, Source::Input as u64);
        machine.set_register(A7, LOAD_CELL_DATA_SYSCALL_NUMBER);

        let mut syscall = LoadCellData::new(tx, Arc::new(SimpleDataProvider::new()), vec![0], vec![]);
        let handled = syscall.ecall(&mut machine).expect("load cell data syscall should be handled");

        assert!(handled);
        assert_eq!(machine.registers()[A0].to_u64(), ITEM_MISSING as u64);
    }

    #[test]
    fn test_load_cell_data_output_source_returns_item_missing_when_output_data_absent() {
        let tx = Arc::new(CellTx {
            version: 0xC001,
            inputs: vec![],
            cell_deps: vec![],
            header_deps: vec![],
            outputs: vec![CellOutput { capacity: 1000, lock: Script::new([1u8; 32], 0, vec![]), type_: None }],
            outputs_data: vec![],
            witnesses: vec![],
        });

        let mut machine = ScriptVersion::V2.init_core_machine(10_000);
        machine.memory_mut().store64(&SIZE_ADDR, &8u64).unwrap();
        machine.set_register(A0, BUFFER_ADDR);
        machine.set_register(A1, SIZE_ADDR);
        machine.set_register(A2, 0);
        machine.set_register(A3, 0);
        machine.set_register(A4, Source::Output as u64);
        machine.set_register(A7, LOAD_CELL_DATA_SYSCALL_NUMBER);

        let mut syscall = LoadCellData::new(tx, Arc::new(SimpleDataProvider::new()), vec![], vec![]);
        let handled = syscall.ecall(&mut machine).expect("load cell data syscall should be handled");

        assert!(handled);
        assert_eq!(machine.registers()[A0].to_u64(), ITEM_MISSING as u64);
    }

    #[test]
    fn test_load_cell_data_as_code_maps_bytes_into_executable_pages() {
        let dep_out_point = OutPoint::new([8u8; 32], 1);
        let tx = Arc::new(CellTx {
            version: 0xC001,
            inputs: vec![],
            cell_deps: vec![CellDep { out_point: dep_out_point.clone(), dep_type: DepType::Code }],
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
                cell_output: CellOutput { capacity: 3000, lock: Script::new([3u8; 32], 0, vec![]), type_: None },
                data: Some(vec![0x30, 0x40, 0x50]),
            },
        );

        let code_addr = 0x3000;
        let mut machine = ScriptVersion::V2.init_core_machine(10_000);
        machine.set_register(A0, code_addr);
        machine.set_register(A1, 4096);
        machine.set_register(A2, 1);
        machine.set_register(A3, 2);
        machine.set_register(A4, 0);
        machine.set_register(A5, Source::CellDep as u64);
        machine.set_register(A7, LOAD_CELL_DATA_AS_CODE_SYSCALL_NUMBER);

        let mut syscall = LoadCellData::new(tx, Arc::new(provider), vec![], vec![]);
        let handled = syscall.ecall(&mut machine).expect("load cell data as code syscall should succeed");

        assert!(handled);
        assert_eq!(machine.registers()[A0].to_u64(), SUCCESS as u64);
        assert_eq!(machine.memory_mut().load_bytes(code_addr, 4).unwrap().as_ref(), &[0x40, 0x50, 0x00, 0x00]);
        assert_eq!(machine.cycles(), crate::vm::transferred_byte_cycles(4096));
    }

    #[test]
    fn test_load_cell_data_as_code_reports_slice_out_of_bound() {
        let dep_out_point = OutPoint::new([8u8; 32], 1);
        let tx = Arc::new(CellTx {
            version: 0xC001,
            inputs: vec![],
            cell_deps: vec![CellDep { out_point: dep_out_point.clone(), dep_type: DepType::Code }],
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
                cell_output: CellOutput { capacity: 3000, lock: Script::new([3u8; 32], 0, vec![]), type_: None },
                data: Some(vec![0x30, 0x40, 0x50]),
            },
        );

        let code_addr = 0x3000;
        let mut machine = ScriptVersion::V2.init_core_machine(10_000);
        machine.set_register(A0, code_addr);
        machine.set_register(A1, 2);
        machine.set_register(A2, 2);
        machine.set_register(A3, 2);
        machine.set_register(A4, 0);
        machine.set_register(A5, Source::CellDep as u64);
        machine.set_register(A7, LOAD_CELL_DATA_AS_CODE_SYSCALL_NUMBER);

        let mut syscall = LoadCellData::new(tx, Arc::new(provider), vec![], vec![]);
        let handled = syscall.ecall(&mut machine).expect("load cell data as code syscall should be handled");

        assert!(handled);
        assert_eq!(machine.registers()[A0].to_u64(), SLICE_OUT_OF_BOUND as u64);
    }

    #[test]
    fn test_load_cell_data_ckb_strict_rejects_header_dep_source() {
        let header_hash = [0xDD; 32];
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
                cell_output: CellOutput { capacity: 8000, lock: Script::new([8u8; 32], 0, vec![]), type_: None },
                data: Some(vec![0xFA, 0xCE]),
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

        let mut syscall = LoadCellData::new(tx, Arc::new(provider), vec![], vec![]).with_semantics(VmSemantics::CkbStrict);
        let handled = syscall.ecall(&mut machine).expect("strict load cell data syscall should be handled");

        assert!(handled);
        assert_eq!(machine.registers()[A0].to_u64(), INDEX_OUT_OF_BOUND as u64);
    }
}
