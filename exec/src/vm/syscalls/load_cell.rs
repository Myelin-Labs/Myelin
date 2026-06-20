// SPDX-License-Identifier: MIT
// Copyright (C) 2026 Myelin developers
//
// Load cell syscall
// Reference: ckb/script/src/syscalls/load_cell.rs

use super::utils::{store_data, INDEX_OUT_OF_BOUND, ITEM_MISSING};
use super::{CellField, Source, LOAD_CELL_BY_FIELD_SYSCALL_NUMBER, LOAD_CELL_SYSCALL_NUMBER};
use crate::celltx::{CellTx, Script};
use crate::serialization::molecule_compat::{
    ckb_cell_data_hash, ckb_script_hash_molecule, serialize_cell_output_molecule, serialize_script_molecule,
};
use crate::serialization::VmAbiFormat;
use crate::vm::transferred_byte_cycles;
use crate::vm::{CellDataProvider, ResolvedCell, VmSemantics};
use ckb_vm::{
    registers::{A0, A3, A4, A5, A7},
    Error as VMError, Register, SupportMachine, Syscalls,
};
use std::sync::Arc;

/// Syscall: Load Cell
///
/// Syscall number: 2071
///
/// Load cell data by source and index
pub struct LoadCell<D: CellDataProvider> {
    tx: Arc<CellTx>,
    provider: Arc<D>,
    group_input_indices: Vec<usize>,
    group_output_indices: Vec<usize>,
    semantics: VmSemantics,
    abi_format: VmAbiFormat,
}

enum CellLookupResult {
    Cell(ResolvedCell),
    IndexOutOfBound,
    ItemMissing,
}

impl<D: CellDataProvider> LoadCell<D> {
    pub fn new(tx: Arc<CellTx>, provider: Arc<D>, group_input_indices: Vec<usize>, group_output_indices: Vec<usize>) -> Self {
        Self {
            tx,
            provider,
            group_input_indices,
            group_output_indices,
            semantics: VmSemantics::MyelinExtended,
            abi_format: VmAbiFormat::Molecule,
        }
    }

    pub fn with_semantics(mut self, semantics: VmSemantics) -> Self {
        self.semantics = semantics;
        self
    }

    /// Select the VM ABI wire format used by full cell and script field loads.
    pub fn with_abi_format(mut self, abi_format: VmAbiFormat) -> Self {
        self.abi_format = abi_format;
        self
    }

    fn resolve_cell(&self, source: Source, index: usize) -> CellLookupResult {
        match source {
            Source::Input => match self.tx.inputs.get(index) {
                Some(input) => self
                    .provider
                    .load_cell_by_outpoint(&input.previous_output.tx_hash, input.previous_output.index)
                    .map(CellLookupResult::Cell)
                    .unwrap_or(CellLookupResult::ItemMissing),
                None => CellLookupResult::IndexOutOfBound,
            },
            Source::Output => self
                .tx
                .outputs
                .get(index)
                .cloned()
                .map(|cell_output| {
                    CellLookupResult::Cell(ResolvedCell { cell_output, data: self.tx.outputs_data.get(index).cloned() })
                })
                .unwrap_or(CellLookupResult::IndexOutOfBound),
            Source::CellDep => match self.tx.cell_deps.get(index) {
                Some(dep) => self
                    .provider
                    .load_cell_by_outpoint(&dep.out_point.tx_hash, dep.out_point.index)
                    .map(CellLookupResult::Cell)
                    .unwrap_or(CellLookupResult::ItemMissing),
                None => CellLookupResult::IndexOutOfBound,
            },
            Source::GroupInput => match self.group_input_indices.get(index).and_then(|&idx| self.tx.inputs.get(idx)) {
                Some(input) => self
                    .provider
                    .load_cell_by_outpoint(&input.previous_output.tx_hash, input.previous_output.index)
                    .map(CellLookupResult::Cell)
                    .unwrap_or(CellLookupResult::ItemMissing),
                None => CellLookupResult::IndexOutOfBound,
            },
            Source::HeaderDep => match self.tx.header_deps.get(index) {
                Some(hash) if self.semantics.allow_header_dep_cell_lookup() => {
                    self.provider.load_cell_by_header(hash).map(CellLookupResult::Cell).unwrap_or(CellLookupResult::ItemMissing)
                }
                Some(_) => CellLookupResult::IndexOutOfBound,
                None => CellLookupResult::IndexOutOfBound,
            },
            Source::GroupOutput => match self.group_output_indices.get(index).and_then(|&idx| self.tx.outputs.get(idx)).cloned() {
                Some(cell_output) => {
                    let output_index = self.group_output_indices[index];
                    CellLookupResult::Cell(ResolvedCell { cell_output, data: self.tx.outputs_data.get(output_index).cloned() })
                }
                None => CellLookupResult::IndexOutOfBound,
            },
            Source::GroupCellDep | Source::GroupHeaderDep => CellLookupResult::IndexOutOfBound,
        }
    }

    fn serialize_cell_field(&self, cell: &ResolvedCell, field: u64) -> Result<Option<Vec<u8>>, VMError> {
        match CellField::parse_from_u64(field)? {
            CellField::Capacity => Ok(Some(cell.cell_output.capacity.to_le_bytes().to_vec())),
            CellField::DataHash => {
                let data = cell.data.as_deref().unwrap_or(&[]);
                Ok(Some(self.cell_data_hash(data).to_vec()))
            }
            CellField::Lock => Ok(Some(self.serialize_script(&cell.cell_output.lock)?)),
            CellField::LockHash => Ok(Some(self.script_hash(&cell.cell_output.lock)?.to_vec())),
            CellField::Type => cell.cell_output.type_.as_ref().map(|s| self.serialize_script(s)).transpose(),
            CellField::TypeHash => cell.cell_output.type_.as_ref().map(|s| self.script_hash(s).map(|hash| hash.to_vec())).transpose(),
            CellField::OccupiedCapacity => {
                let data_len = cell.data.as_ref().map_or(0, Vec::len);
                Ok(Some(cell.cell_output.occupied_capacity(data_len).to_le_bytes().to_vec()))
            }
        }
    }

    fn serialize_script(&self, script: &Script) -> Result<Vec<u8>, VMError> {
        match self.abi_format {
            VmAbiFormat::Molecule => serialize_script_molecule(script).map_err(|e| VMError::External(e.to_string())),
        }
    }

    fn script_hash(&self, script: &Script) -> Result<[u8; 32], VMError> {
        match self.semantics {
            VmSemantics::MyelinExtended => Ok(script.hash()),
            VmSemantics::CkbStrict => ckb_script_hash_molecule(script).map_err(|e| VMError::External(e.to_string())),
        }
    }

    fn cell_data_hash(&self, data: &[u8]) -> [u8; 32] {
        match self.semantics {
            VmSemantics::MyelinExtended => {
                if data.is_empty() {
                    [0u8; 32]
                } else {
                    let mut hasher = blake3::Hasher::new();
                    hasher.update(b"myelin-cell/data");
                    hasher.update(data);
                    *hasher.finalize().as_bytes()
                }
            }
            VmSemantics::CkbStrict => ckb_cell_data_hash(data),
        }
    }

    fn serialize_cell(&self, cell: &ResolvedCell) -> Result<Vec<u8>, VMError> {
        match self.abi_format {
            VmAbiFormat::Molecule => serialize_cell_output_molecule(&cell.cell_output).map_err(|e| VMError::External(e.to_string())),
        }
    }
}

impl<D: CellDataProvider, M: SupportMachine> Syscalls<M> for LoadCell<D> {
    fn initialize(&mut self, _machine: &mut M) -> Result<(), VMError> {
        Ok(())
    }

    fn ecall(&mut self, machine: &mut M) -> Result<bool, VMError> {
        let syscall_number = machine.registers()[A7].to_u64();

        // LOAD_CELL = 2071 or LOAD_CELL_BY_FIELD = 2081
        if syscall_number != LOAD_CELL_SYSCALL_NUMBER && syscall_number != LOAD_CELL_BY_FIELD_SYSCALL_NUMBER {
            return Ok(false);
        }

        // Args (store_data reads A0, A1, A2):
        // A2: offset
        // A3: index
        // A4: source
        // A5: field (only for 2081)
        let index = machine.registers()[A3].to_u64() as usize;
        let source = Source::parse_from_u64_for_semantics(machine.registers()[A4].to_u64(), self.semantics)?;

        // Get cell
        let cell = match self.resolve_cell(source, index) {
            CellLookupResult::Cell(cell) => cell,
            CellLookupResult::IndexOutOfBound => {
                machine.set_register(A0, M::REG::from_u8(INDEX_OUT_OF_BOUND));
                return Ok(true);
            }
            CellLookupResult::ItemMissing => {
                machine.set_register(A0, M::REG::from_u8(ITEM_MISSING));
                return Ok(true);
            }
        };

        // Get field data
        let data = if syscall_number == LOAD_CELL_BY_FIELD_SYSCALL_NUMBER {
            // LOAD_CELL_BY_FIELD
            let field = machine.registers()[A5].to_u64();
            match self.serialize_cell_field(&cell, field)? {
                Some(d) => d,
                None => {
                    machine.set_register(A0, M::REG::from_u8(ITEM_MISSING));
                    return Ok(true);
                }
            }
        } else {
            // LOAD_CELL (full cell output data)
            self.serialize_cell(&cell)?
        };

        // Store data using CKB-style store_data
        let result = store_data(machine, &data)?;
        machine.add_cycles_no_checking(transferred_byte_cycles(result.written_size))?;
        machine.set_register(A0, M::REG::from_u8(result.return_code));

        Ok(true)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::celltx::{CellDep, CellInput, CellOutput, DepType, OutPoint};
    use crate::serialization::molecule_compat::{
        ckb_cell_data_hash, ckb_script_hash_molecule, deserialize_cell_output_molecule, serialize_cell_output_molecule,
    };
    use crate::serialization::VmAbiFormat;
    use crate::vm::syscalls::SUCCESS;
    use crate::vm::{ResolvedCell, ScriptVersion, SimpleDataProvider, VmSemantics};
    use ckb_vm::{
        registers::{A1, A2},
        CoreMachine, Memory, Register,
    };

    const BUFFER_ADDR: u64 = 0x1000;
    const SIZE_ADDR: u64 = 0x2000;

    #[test]
    fn test_load_cell_creation() {
        let input_out_point = OutPoint::new([7u8; 32], 0);
        let dep_out_point = OutPoint::new([8u8; 32], 1);
        let tx = Arc::new(CellTx {
            version: 0xC001,
            inputs: vec![CellInput::new(input_out_point, 0)],
            cell_deps: vec![CellDep { out_point: dep_out_point, dep_type: DepType::Code }],
            header_deps: vec![],
            outputs: vec![CellOutput { capacity: 1000, lock: Script::new([1u8; 32], 0, vec![]), type_: None }],
            outputs_data: vec![vec![0xAA; 10]],
            witnesses: vec![],
        });
        let provider = Arc::new(SimpleDataProvider::new());

        let _syscall = LoadCell::new(tx, provider, vec![0], vec![0]);
        // Just ensure it compiles
    }

    #[test]
    fn test_load_cell_by_field_uses_ckb_hashes_under_ckb_strict_semantics() {
        let output = CellOutput {
            capacity: 1000,
            lock: Script::new([1u8; 32], 0, vec![0xAA]),
            type_: Some(Script::new([2u8; 32], 1, vec![0xBB])),
        };
        let tx = Arc::new(CellTx {
            version: 0xC001,
            inputs: vec![],
            cell_deps: vec![],
            header_deps: vec![],
            outputs: vec![output.clone()],
            outputs_data: vec![vec![0xCC, 0xDD]],
            witnesses: vec![],
        });
        let provider = Arc::new(SimpleDataProvider::new());

        let expected_data_hash = ckb_cell_data_hash(&[0xCC, 0xDD]);
        let expected_lock_hash = ckb_script_hash_molecule(&output.lock).unwrap();
        let expected_type_hash = ckb_script_hash_molecule(output.type_.as_ref().unwrap()).unwrap();
        assert_ne!(expected_lock_hash, output.lock.hash());
        assert_ne!(expected_type_hash, output.type_.as_ref().unwrap().hash());

        for (field, expected) in [
            (CellField::DataHash as u64, expected_data_hash),
            (CellField::LockHash as u64, expected_lock_hash),
            (CellField::TypeHash as u64, expected_type_hash),
        ] {
            let mut machine = ScriptVersion::V2.init_core_machine(20_000);
            machine.memory_mut().store64(&SIZE_ADDR, &32u64).unwrap();
            machine.set_register(A0, BUFFER_ADDR);
            machine.set_register(A1, SIZE_ADDR);
            machine.set_register(A2, 0);
            machine.set_register(A3, 0);
            machine.set_register(A4, Source::Output as u64);
            machine.set_register(A5, field);
            machine.set_register(A7, LOAD_CELL_BY_FIELD_SYSCALL_NUMBER);

            let mut syscall =
                LoadCell::new(Arc::clone(&tx), Arc::clone(&provider), vec![], vec![0]).with_semantics(VmSemantics::CkbStrict);
            let handled = syscall.ecall(&mut machine).expect("load cell field syscall should succeed");

            assert!(handled);
            assert_eq!(machine.registers()[A0].to_u64(), SUCCESS as u64);
            assert_eq!(machine.memory_mut().load64(&SIZE_ADDR).unwrap().to_u64(), 32);
            assert_eq!(machine.memory_mut().load_bytes(BUFFER_ADDR, 32).unwrap().as_ref(), expected.as_ref());
        }
    }

    #[test]
    fn test_load_cell_molecule_abi_full_load() {
        let output = CellOutput {
            capacity: 1000,
            lock: Script::new([1u8; 32], 0, vec![0xAA]),
            type_: Some(Script::new([2u8; 32], 1, vec![0xBB, 0xCC])),
        };
        let expected = serialize_cell_output_molecule(&output).unwrap();
        let tx = Arc::new(CellTx {
            version: 0xC001,
            inputs: vec![],
            cell_deps: vec![],
            header_deps: vec![],
            outputs: vec![output],
            outputs_data: vec![vec![]],
            witnesses: vec![],
        });

        let mut machine = ScriptVersion::V2.init_core_machine(10_000);
        machine.memory_mut().store64(&SIZE_ADDR, &(expected.len() as u64)).unwrap();
        machine.set_register(A0, BUFFER_ADDR);
        machine.set_register(A1, SIZE_ADDR);
        machine.set_register(A2, 0);
        machine.set_register(A3, 0);
        machine.set_register(A4, Source::Output as u64);
        machine.set_register(A7, LOAD_CELL_SYSCALL_NUMBER);

        let mut syscall =
            LoadCell::new(tx, Arc::new(SimpleDataProvider::new()), vec![], vec![]).with_abi_format(VmAbiFormat::Molecule);
        let handled = syscall.ecall(&mut machine).expect("load cell syscall should succeed");

        assert!(handled);
        assert_eq!(machine.registers()[A0].to_u64(), SUCCESS as u64);
        assert_eq!(machine.memory_mut().load64(&SIZE_ADDR).unwrap().to_u64(), expected.len() as u64);
        assert_eq!(machine.memory_mut().load_bytes(BUFFER_ADDR, expected.len() as u64).unwrap().as_ref(), expected.as_slice());
    }

    #[test]
    fn test_load_cell_resolves_input_and_dep_sources() {
        let input_out_point = OutPoint::new([7u8; 32], 0);
        let dep_out_point = OutPoint::new([8u8; 32], 1);
        let tx = Arc::new(CellTx {
            version: 0xC001,
            inputs: vec![CellInput::new(input_out_point, 0)],
            cell_deps: vec![CellDep { out_point: dep_out_point, dep_type: DepType::Code }],
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

        let syscall = LoadCell::new(tx, Arc::new(provider), vec![0], vec![0]);
        let input_cell = match syscall.resolve_cell(Source::Input, 0) {
            CellLookupResult::Cell(cell) => cell,
            _ => panic!("resolved input cell"),
        };
        let dep_cell = match syscall.resolve_cell(Source::CellDep, 0) {
            CellLookupResult::Cell(cell) => cell,
            _ => panic!("resolved dep cell"),
        };
        assert_eq!(input_cell.cell_output.capacity, 2000);
        assert_eq!(dep_cell.cell_output.capacity, 3000);
        assert_eq!(
            syscall.serialize_cell_field(&input_cell, 6).unwrap().unwrap(),
            input_cell.cell_output.occupied_capacity(2).to_le_bytes()
        );
    }

    #[test]
    fn test_load_cell_by_field_supports_partial_reads() {
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
        let mut provider = SimpleDataProvider::new();
        provider.add_cell(
            input_out_point.tx_hash,
            input_out_point.index,
            ResolvedCell {
                cell_output: CellOutput { capacity: 0x1122_3344_5566_7788, lock: Script::new([2u8; 32], 0, vec![]), type_: None },
                data: Some(vec![]),
            },
        );

        let mut machine = ScriptVersion::V2.init_core_machine(10_000);
        machine.memory_mut().store64(&SIZE_ADDR, &1024u64).unwrap();
        machine.set_register(A0, BUFFER_ADDR);
        machine.set_register(A1, SIZE_ADDR);
        machine.set_register(A2, 4);
        machine.set_register(A3, 0);
        machine.set_register(A4, 0x01);
        machine.set_register(A5, CellField::Capacity as u64);
        machine.set_register(A7, LOAD_CELL_BY_FIELD_SYSCALL_NUMBER);

        let mut syscall = LoadCell::new(tx, Arc::new(provider), vec![0], vec![]).with_abi_format(VmAbiFormat::Molecule);
        let handled = syscall.ecall(&mut machine).expect("load cell syscall should succeed");

        assert!(handled);
        assert_eq!(machine.registers()[A0].to_u64(), SUCCESS as u64);
        assert_eq!(machine.memory_mut().load64(&SIZE_ADDR).unwrap().to_u64(), 4);
        assert_eq!(machine.memory_mut().load_bytes(BUFFER_ADDR, 4).unwrap().as_ref(), &0x1122_3344_5566_7788u64.to_le_bytes()[4..]);
    }

    #[test]
    fn test_load_cell_by_field_rejects_unknown_field() {
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
        let mut provider = SimpleDataProvider::new();
        provider.add_cell(
            input_out_point.tx_hash,
            input_out_point.index,
            ResolvedCell {
                cell_output: CellOutput { capacity: 1, lock: Script::new([2u8; 32], 0, vec![]), type_: None },
                data: Some(vec![]),
            },
        );

        let mut machine = ScriptVersion::V2.init_core_machine(10_000);
        machine.memory_mut().store64(&SIZE_ADDR, &1024u64).unwrap();
        machine.set_register(A0, BUFFER_ADDR);
        machine.set_register(A1, SIZE_ADDR);
        machine.set_register(A2, 0);
        machine.set_register(A3, 0);
        machine.set_register(A4, Source::Input as u64);
        machine.set_register(A5, 99);
        machine.set_register(A7, LOAD_CELL_BY_FIELD_SYSCALL_NUMBER);

        let mut syscall = LoadCell::new(tx, Arc::new(provider), vec![0], vec![]).with_abi_format(VmAbiFormat::Molecule);
        let err = syscall.ecall(&mut machine).expect_err("unknown field should trap");

        assert_eq!(err, VMError::External("CellField parse_from_u64 99".to_string()));
    }

    #[test]
    fn test_load_cell_returns_item_missing_when_resolved_input_cell_not_found() {
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
        machine.memory_mut().store64(&SIZE_ADDR, &1024u64).unwrap();
        machine.set_register(A0, BUFFER_ADDR);
        machine.set_register(A1, SIZE_ADDR);
        machine.set_register(A2, 0);
        machine.set_register(A3, 0);
        machine.set_register(A4, Source::Input as u64);
        machine.set_register(A7, LOAD_CELL_SYSCALL_NUMBER);

        let mut syscall = LoadCell::new(tx, Arc::new(SimpleDataProvider::new()), vec![0], vec![]);
        let handled = syscall.ecall(&mut machine).expect("load cell syscall should be handled");

        assert!(handled);
        assert_eq!(machine.registers()[A0].to_u64(), ITEM_MISSING as u64);
    }

    #[test]
    fn test_load_cell_full_serialization_starts_with_capacity() {
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
        let mut provider = SimpleDataProvider::new();
        let capacity = 0x1122_3344_5566_7788u64;
        provider.add_cell(
            input_out_point.tx_hash,
            input_out_point.index,
            ResolvedCell {
                cell_output: CellOutput {
                    capacity,
                    lock: Script::new([2u8; 32], 0, vec![0xAA, 0xBB]),
                    type_: Some(Script::new([3u8; 32], 1, vec![0xCC])),
                },
                data: Some(vec![]),
            },
        );

        let mut machine = ScriptVersion::V2.init_core_machine(10_000);
        machine.memory_mut().store64(&SIZE_ADDR, &1024u64).unwrap();
        machine.set_register(A0, BUFFER_ADDR);
        machine.set_register(A1, SIZE_ADDR);
        machine.set_register(A2, 0);
        machine.set_register(A3, 0);
        machine.set_register(A4, Source::Input as u64);
        machine.set_register(A7, LOAD_CELL_SYSCALL_NUMBER);

        let mut syscall = LoadCell::new(tx, Arc::new(provider), vec![0], vec![]).with_abi_format(VmAbiFormat::Molecule);
        let handled = syscall.ecall(&mut machine).expect("load cell syscall should succeed");

        assert!(handled);
        assert_eq!(machine.registers()[A0].to_u64(), SUCCESS as u64);
        let loaded_size = machine.memory_mut().load64(&SIZE_ADDR).unwrap().to_u64();
        assert!(loaded_size > 8);
        let loaded = machine.memory_mut().load_bytes(BUFFER_ADDR, loaded_size).unwrap();
        let decoded = deserialize_cell_output_molecule(loaded.as_ref()).expect("Molecule CellOutput");
        assert_eq!(decoded.capacity, capacity);
    }

    #[test]
    fn test_load_cell_resolves_header_dep_source() {
        let header_hash = [0xAA; 32];
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
                cell_output: CellOutput { capacity: 5000, lock: Script::new([5u8; 32], 0, vec![0x55]), type_: None },
                data: Some(vec![0xDE, 0xAD]),
            },
        );

        let syscall = LoadCell::new(tx, Arc::new(provider), vec![], vec![]);
        let cell = match syscall.resolve_cell(Source::HeaderDep, 0) {
            CellLookupResult::Cell(c) => c,
            _ => panic!("expected Cell for HeaderDep source"),
        };
        assert_eq!(cell.cell_output.capacity, 5000);
        assert_eq!(cell.data, Some(vec![0xDE, 0xAD]));
    }

    #[test]
    fn test_load_cell_header_dep_returns_index_out_of_bound() {
        let tx = Arc::new(CellTx {
            version: 0xC001,
            inputs: vec![],
            cell_deps: vec![],
            header_deps: vec![],
            outputs: vec![],
            outputs_data: vec![],
            witnesses: vec![],
        });
        let provider = Arc::new(SimpleDataProvider::new());
        let syscall = LoadCell::new(tx, provider, vec![], vec![]);
        assert!(matches!(syscall.resolve_cell(Source::HeaderDep, 0), CellLookupResult::IndexOutOfBound));
    }

    #[test]
    fn test_load_cell_header_dep_returns_item_missing_when_cell_not_found() {
        let header_hash = [0xBB; 32];
        let tx = Arc::new(CellTx {
            version: 0xC001,
            inputs: vec![],
            cell_deps: vec![],
            header_deps: vec![header_hash],
            outputs: vec![],
            outputs_data: vec![],
            witnesses: vec![],
        });
        // No cell registered for this header
        let provider = Arc::new(SimpleDataProvider::new());
        let syscall = LoadCell::new(tx, provider, vec![], vec![]);
        assert!(matches!(syscall.resolve_cell(Source::HeaderDep, 0), CellLookupResult::ItemMissing));
    }

    #[test]
    fn test_load_cell_header_dep_ecall_writes_capacity() {
        let header_hash = [0xCC; 32];
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
        let capacity = 0xDEAD_BEEF_CAFE_BABEu64;
        provider.add_cell_by_header(
            header_hash,
            ResolvedCell {
                cell_output: CellOutput { capacity, lock: Script::new([6u8; 32], 0, vec![]), type_: None },
                data: Some(vec![]),
            },
        );

        let mut machine = ScriptVersion::V2.init_core_machine(10_000);
        machine.memory_mut().store64(&SIZE_ADDR, &8u64).unwrap();
        machine.set_register(A0, BUFFER_ADDR);
        machine.set_register(A1, SIZE_ADDR);
        machine.set_register(A2, 0);
        machine.set_register(A3, 0);
        machine.set_register(A4, Source::HeaderDep as u64);
        machine.set_register(A5, CellField::Capacity as u64);
        machine.set_register(A7, LOAD_CELL_BY_FIELD_SYSCALL_NUMBER);

        let mut syscall = LoadCell::new(tx, Arc::new(provider), vec![], vec![]);
        let handled = syscall.ecall(&mut machine).expect("load cell by field via header dep should succeed");

        assert!(handled);
        assert_eq!(machine.registers()[A0].to_u64(), SUCCESS as u64);
        assert_eq!(machine.memory_mut().load_bytes(BUFFER_ADDR, 8).unwrap().as_ref(), &capacity.to_le_bytes());
    }

    #[test]
    fn test_load_cell_ckb_strict_rejects_header_dep_source() {
        let header_hash = [0xCD; 32];
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
                data: Some(vec![1, 2, 3]),
            },
        );

        let mut machine = ScriptVersion::V2.init_core_machine(10_000);
        machine.memory_mut().store64(&SIZE_ADDR, &8u64).unwrap();
        machine.set_register(A0, BUFFER_ADDR);
        machine.set_register(A1, SIZE_ADDR);
        machine.set_register(A2, 0);
        machine.set_register(A3, 0);
        machine.set_register(A4, Source::HeaderDep as u64);
        machine.set_register(A7, LOAD_CELL_SYSCALL_NUMBER);

        let mut syscall = LoadCell::new(tx, Arc::new(provider), vec![], vec![]).with_semantics(VmSemantics::CkbStrict);
        let handled = syscall.ecall(&mut machine).expect("strict load cell syscall should be handled");

        assert!(handled);
        assert_eq!(machine.registers()[A0].to_u64(), INDEX_OUT_OF_BOUND as u64);
    }
}
