// SPDX-License-Identifier: MIT
// Copyright (C) 2026 Myelin developers
//
// Exec syscall
// Reference: ckb/script/src/syscalls/exec.rs

use super::{Source, EXEC_SYSCALL_NUMBER, INDEX_OUT_OF_BOUND, ITEM_MISSING, SLICE_OUT_OF_BOUND, WRONG_FORMAT};
use crate::celltx::CellTx;
use crate::serialization::split_vm_abi_trailer;
use crate::vm::scheduler::{ProgramDataId, ProgramPiece, ProgramPlace, SchedulerDataSource, VmSnapshotHandle};
use crate::vm::transferred_byte_cycles;
use crate::vm::{CellDataProvider, VmSemantics};
use ckb_vm::{
    elf::parse_elf,
    memory::load_c_string_byte_by_byte,
    registers::{A0, A1, A2, A3, A4, A5, A7},
    Bytes, Error as VMError, Memory, Register, SupportMachine, Syscalls, DEFAULT_STACK_SIZE, RISCV_MAX_MEMORY,
};
use std::sync::Arc;

const MAX_ARGV_LENGTH: u64 = 1024 * 1024;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum ExecPlace {
    CellData = 0,
    Witness = 1,
}

impl ExecPlace {
    fn parse_from_u64(value: u64) -> Result<Self, VMError> {
        match value {
            0 => Ok(Self::CellData),
            1 => Ok(Self::Witness),
            _ => Err(VMError::External(format!("Place parse_from_u64 {value}"))),
        }
    }
}

impl From<ExecPlace> for ProgramPlace {
    fn from(value: ExecPlace) -> Self {
        match value {
            ExecPlace::CellData => ProgramPlace::CellData,
            ExecPlace::Witness => ProgramPlace::Witness,
        }
    }
}

/// Syscall: Exec
///
/// Syscall number: 2043
///
/// Loads a new ELF payload from Cell data or witness and resets the current VM
/// execution context while preserving cycle usage accounting.
pub struct Exec<D: CellDataProvider> {
    tx: Arc<CellTx>,
    provider: Arc<D>,
    group_input_indices: Vec<usize>,
    group_output_indices: Vec<usize>,
    snapshot2_context: Option<VmSnapshotHandle>,
    data_source: Option<SchedulerDataSource>,
    semantics: VmSemantics,
}

impl<D: CellDataProvider> Exec<D> {
    pub fn new(tx: Arc<CellTx>, provider: Arc<D>, group_input_indices: Vec<usize>, group_output_indices: Vec<usize>) -> Self {
        Self {
            tx,
            provider,
            group_input_indices,
            group_output_indices,
            snapshot2_context: None,
            data_source: None,
            semantics: VmSemantics::MyelinExtended,
        }
    }

    pub fn with_snapshot_tracking(mut self, snapshot2_context: VmSnapshotHandle, data_source: SchedulerDataSource) -> Self {
        self.snapshot2_context = Some(snapshot2_context);
        self.data_source = Some(data_source);
        self
    }

    pub fn with_semantics(mut self, semantics: VmSemantics) -> Self {
        self.semantics = semantics;
        self
    }

    fn resolve_output_index(&self, source: Source, index: usize) -> Result<usize, u8> {
        match source {
            Source::Output => {
                if self.tx.outputs.get(index).is_some() {
                    Ok(index)
                } else {
                    Err(INDEX_OUT_OF_BOUND)
                }
            }
            Source::GroupOutput => self.group_output_indices.get(index).copied().ok_or(INDEX_OUT_OF_BOUND),
            _ => Err(INDEX_OUT_OF_BOUND),
        }
    }

    fn load_cell_data(&self, source: Source, index: usize) -> Result<Vec<u8>, u8> {
        match source {
            Source::Input => {
                let input = self.tx.inputs.get(index).ok_or(INDEX_OUT_OF_BOUND)?;
                self.provider
                    .load_cell_by_outpoint(&input.previous_output.tx_hash, input.previous_output.index)
                    .map(|cell| cell.data.unwrap_or_default())
                    .ok_or(ITEM_MISSING)
            }
            Source::Output => {
                self.tx.outputs.get(index).ok_or(INDEX_OUT_OF_BOUND)?;
                self.tx.outputs_data.get(index).cloned().ok_or(ITEM_MISSING)
            }
            Source::CellDep => {
                let dep = self.tx.cell_deps.get(index).ok_or(INDEX_OUT_OF_BOUND)?;
                self.provider
                    .load_cell_by_outpoint(&dep.out_point.tx_hash, dep.out_point.index)
                    .map(|cell| cell.data.unwrap_or_default())
                    .ok_or(ITEM_MISSING)
            }
            Source::GroupInput => {
                let input_index = self.group_input_indices.get(index).copied().ok_or(INDEX_OUT_OF_BOUND)?;
                let input = self.tx.inputs.get(input_index).ok_or(INDEX_OUT_OF_BOUND)?;
                self.provider
                    .load_cell_by_outpoint(&input.previous_output.tx_hash, input.previous_output.index)
                    .map(|cell| cell.data.unwrap_or_default())
                    .ok_or(ITEM_MISSING)
            }
            Source::GroupOutput => {
                let output_index = self.resolve_output_index(source, index)?;
                self.tx.outputs.get(output_index).ok_or(INDEX_OUT_OF_BOUND)?;
                self.tx.outputs_data.get(output_index).cloned().ok_or(ITEM_MISSING)
            }
            Source::GroupCellDep | Source::GroupHeaderDep => Err(INDEX_OUT_OF_BOUND),
            Source::HeaderDep => Err(INDEX_OUT_OF_BOUND),
        }
    }

    fn witness_index_for_source(&self, source: Source, index: usize) -> Result<usize, u8> {
        match source {
            Source::Input => Ok(index),
            Source::Output => self.tx.inputs.len().checked_add(index).ok_or(INDEX_OUT_OF_BOUND),
            Source::CellDep => self
                .tx
                .inputs
                .len()
                .checked_add(self.tx.outputs.len())
                .and_then(|base| base.checked_add(index))
                .ok_or(INDEX_OUT_OF_BOUND),
            Source::GroupInput => self.group_input_indices.get(index).copied().ok_or(INDEX_OUT_OF_BOUND),
            Source::GroupOutput => self
                .resolve_output_index(source, index)
                .and_then(|output_index| self.tx.inputs.len().checked_add(output_index).ok_or(INDEX_OUT_OF_BOUND)),
            Source::GroupCellDep | Source::GroupHeaderDep => Err(INDEX_OUT_OF_BOUND),
            Source::HeaderDep => Err(INDEX_OUT_OF_BOUND),
        }
    }

    fn load_witness_data(&self, source: Source, index: usize) -> Result<Vec<u8>, u8> {
        let witness_index = self.witness_index_for_source(source, index)?;
        self.tx.witnesses.get(witness_index).cloned().ok_or(INDEX_OUT_OF_BOUND)
    }
}

impl<D: CellDataProvider, M: SupportMachine> Syscalls<M> for Exec<D> {
    fn initialize(&mut self, _machine: &mut M) -> Result<(), VMError> {
        Ok(())
    }

    fn ecall(&mut self, machine: &mut M) -> Result<bool, VMError> {
        if machine.registers()[A7].to_u64() != EXEC_SYSCALL_NUMBER {
            return Ok(false);
        }

        let index = machine.registers()[A0].to_u64() as usize;
        let source = Source::parse_from_u64_for_semantics(machine.registers()[A1].to_u64(), self.semantics)?;
        let place = ExecPlace::parse_from_u64(machine.registers()[A2].to_u64())?;

        let payload = match place {
            ExecPlace::CellData => match self.load_cell_data(source, index) {
                Ok(data) => data,
                Err(code) => {
                    machine.set_register(A0, M::REG::from_u8(code));
                    return Ok(true);
                }
            },
            ExecPlace::Witness => match self.load_witness_data(source, index) {
                Ok(data) => data,
                Err(code) => {
                    machine.set_register(A0, M::REG::from_u8(code));
                    return Ok(true);
                }
            },
        };

        let bounds = machine.registers()[A3].to_u64();
        let offset = (bounds >> 32) as u32 as usize;
        let length = bounds as u32 as usize;

        if offset >= payload.len() {
            machine.set_register(A0, M::REG::from_u8(SLICE_OUT_OF_BOUND));
            return Ok(true);
        }

        let program_slice = if length == 0 {
            &payload[offset..]
        } else {
            let Some(end) = offset.checked_add(length) else {
                machine.set_register(A0, M::REG::from_u8(SLICE_OUT_OF_BOUND));
                return Ok(true);
            };
            if end > payload.len() {
                machine.set_register(A0, M::REG::from_u8(SLICE_OUT_OF_BOUND));
                return Ok(true);
            }
            &payload[offset..end]
        };
        let program_slice = match split_vm_abi_trailer(program_slice) {
            Ok((program_slice, _)) => program_slice,
            Err(_) => {
                machine.set_register(A0, M::REG::from_u8(WRONG_FORMAT));
                return Ok(true);
            }
        };
        let program = Bytes::copy_from_slice(program_slice);
        let metadata = match parse_elf::<u64>(&program, machine.version()) {
            Ok(metadata) => metadata,
            Err(_) => {
                machine.set_register(A0, M::REG::from_u8(WRONG_FORMAT));
                return Ok(true);
            }
        };

        let argc = machine.registers()[A4].to_u64();
        let mut argv_ptr_addr = machine.registers()[A5].to_u64();
        let mut argv = Vec::new();
        let mut argv_length: u64 = 0;
        for _ in 0..argc {
            let arg_addr = machine.memory_mut().load64(&M::REG::from_u64(argv_ptr_addr))?;
            let arg = load_c_string_byte_by_byte(machine.memory_mut(), &arg_addr)?;
            argv_length = argv_length.saturating_add(8).saturating_add(arg.len() as u64);
            if argv_length > MAX_ARGV_LENGTH {
                return Err(VMError::Unexpected("argv length exceeds MAX_ARGV_LENGTH".to_string()));
            }
            argv.push(arg);
            argv_ptr_addr = argv_ptr_addr.checked_add(8).ok_or(VMError::MemOutOfBound)?;
        }

        let consumed_cycles = machine.cycles();
        let max_cycles = machine.max_cycles();
        machine.reset(max_cycles);
        machine.set_cycles(consumed_cycles);

        let loaded_bytes = match machine.load_elf(&program, true) {
            Ok(size) => size,
            Err(_) => {
                machine.set_register(A0, M::REG::from_u8(WRONG_FORMAT));
                return Ok(true);
            }
        };
        let loaded_bytes = usize::try_from(loaded_bytes).map_err(|_| VMError::MemOutOfBound)?;
        machine.add_cycles_no_checking(transferred_byte_cycles(loaded_bytes))?;

        let stack_bytes = match machine.initialize_stack(
            argv.into_iter().map(Ok),
            (RISCV_MAX_MEMORY - DEFAULT_STACK_SIZE) as u64,
            DEFAULT_STACK_SIZE as u64,
        ) {
            Ok(size) => size,
            Err(_) => {
                machine.set_register(A0, M::REG::from_u8(WRONG_FORMAT));
                return Ok(true);
            }
        };
        let stack_bytes = usize::try_from(stack_bytes).map_err(|_| VMError::MemOutOfBound)?;
        machine.add_cycles_no_checking(transferred_byte_cycles(stack_bytes))?;

        if let (Some(snapshot2_context), Some(data_source)) = (&self.snapshot2_context, &self.data_source) {
            let mut snapshot_context =
                snapshot2_context.lock().map_err(|err| VMError::Unexpected(format!("snapshot2 context poisoned: {err}")))?;
            *snapshot_context = data_source.snapshot_context();
            snapshot_context.mark_program(
                machine,
                &metadata,
                &ProgramDataId::Piece(ProgramPiece { source, index, place: place.into() }),
                offset as u64,
            )?;
        }

        Ok(true)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::celltx::{CellDep, CellOutput, DepType, OutPoint, Script};
    use crate::scripts::ALWAYS_SUCCESS_SCRIPT;
    use crate::vm::{ResolvedCell, ScriptVersion, SimpleDataProvider};
    use ckb_vm::{CoreMachine, Register};

    fn sample_dep_tx(out_point: OutPoint) -> Arc<CellTx> {
        Arc::new(CellTx {
            version: 0xC001,
            inputs: vec![],
            cell_deps: vec![CellDep { out_point, dep_type: DepType::Code }],
            header_deps: vec![],
            outputs: vec![],
            outputs_data: vec![],
            witnesses: vec![],
        })
    }

    #[test]
    fn test_exec_loads_elf_from_cell_dep_data() {
        let dep = OutPoint::new([0x88; 32], 0);
        let tx = sample_dep_tx(dep);

        let mut provider = SimpleDataProvider::new();
        provider.add_cell(
            dep.tx_hash,
            dep.index,
            ResolvedCell {
                cell_output: CellOutput { capacity: 1000, lock: Script::new([1u8; 32], 0, vec![]), type_: None },
                data: Some(ALWAYS_SUCCESS_SCRIPT.to_vec()),
            },
        );

        let mut machine = ScriptVersion::V2.init_core_machine(1_000_000);
        let pc_before = machine.pc().to_u64();
        machine.set_register(A0, 0);
        machine.set_register(A1, Source::CellDep as u64);
        machine.set_register(A2, ExecPlace::CellData as u64);
        machine.set_register(A3, 0);
        machine.set_register(A4, 0);
        machine.set_register(A5, 0);
        machine.set_register(A7, EXEC_SYSCALL_NUMBER);

        let mut syscall = Exec::new(tx, Arc::new(provider), vec![], vec![]);
        let handled = syscall.ecall(&mut machine).expect("exec syscall should succeed");

        assert!(handled);
        assert_ne!(machine.pc().to_u64(), pc_before);
        assert!(machine.cycles() > 0);
    }

    #[test]
    fn test_exec_rejects_out_of_bound_slice() {
        let dep = OutPoint::new([0x88; 32], 0);
        let tx = sample_dep_tx(dep);

        let mut provider = SimpleDataProvider::new();
        provider.add_cell(
            dep.tx_hash,
            dep.index,
            ResolvedCell {
                cell_output: CellOutput { capacity: 1000, lock: Script::new([1u8; 32], 0, vec![]), type_: None },
                data: Some(ALWAYS_SUCCESS_SCRIPT.to_vec()),
            },
        );

        let mut machine = ScriptVersion::V2.init_core_machine(1_000_000);
        machine.set_register(A0, 0);
        machine.set_register(A1, Source::CellDep as u64);
        machine.set_register(A2, ExecPlace::CellData as u64);
        // offset in high 32 bits, length in low 32 bits
        machine.set_register(A3, (ALWAYS_SUCCESS_SCRIPT.len() as u64 + 1) << 32);
        machine.set_register(A4, 0);
        machine.set_register(A5, 0);
        machine.set_register(A7, EXEC_SYSCALL_NUMBER);

        let mut syscall = Exec::new(tx, Arc::new(provider), vec![], vec![]);
        let handled = syscall.ecall(&mut machine).expect("exec syscall should be handled");

        assert!(handled);
        assert_eq!(machine.registers()[A0].to_u64(), SLICE_OUT_OF_BOUND as u64);
    }

    #[test]
    fn test_exec_reports_wrong_format_for_non_elf_payload() {
        let dep = OutPoint::new([0x88; 32], 0);
        let tx = sample_dep_tx(dep);

        let mut provider = SimpleDataProvider::new();
        provider.add_cell(
            dep.tx_hash,
            dep.index,
            ResolvedCell {
                cell_output: CellOutput { capacity: 1000, lock: Script::new([1u8; 32], 0, vec![]), type_: None },
                data: Some(vec![1, 2, 3, 4]),
            },
        );

        let mut machine = ScriptVersion::V2.init_core_machine(1_000_000);
        machine.set_register(A0, 0);
        machine.set_register(A1, Source::CellDep as u64);
        machine.set_register(A2, ExecPlace::CellData as u64);
        machine.set_register(A3, 0);
        machine.set_register(A4, 0);
        machine.set_register(A5, 0);
        machine.set_register(A7, EXEC_SYSCALL_NUMBER);

        let mut syscall = Exec::new(tx, Arc::new(provider), vec![], vec![]);
        let handled = syscall.ecall(&mut machine).expect("exec syscall should be handled");

        assert!(handled);
        assert_eq!(machine.registers()[A0].to_u64(), WRONG_FORMAT as u64);
    }

    #[test]
    fn test_exec_traps_on_invalid_source_encoding() {
        let out_point = OutPoint::new([0xAB; 32], 0);
        let tx = sample_dep_tx(out_point);
        let provider = Arc::new(SimpleDataProvider::new());
        let mut machine = ScriptVersion::V2.init_core_machine(10_000);
        machine.set_register(A0, 0);
        machine.set_register(A1, 0x99);
        machine.set_register(A2, 0);
        machine.set_register(A3, 0);
        machine.set_register(A4, 0);
        machine.set_register(A5, 0);
        machine.set_register(A7, EXEC_SYSCALL_NUMBER);

        let mut syscall = Exec::new(tx, provider, vec![], vec![]);
        let err = syscall.ecall(&mut machine).expect_err("invalid source should trap");

        assert_eq!(err, VMError::External("SourceEntry parse_from_u64 153".to_string()));
    }

    #[test]
    fn test_exec_traps_on_invalid_place_encoding() {
        let out_point = OutPoint::new([0xAB; 32], 0);
        let tx = sample_dep_tx(out_point);
        let provider = Arc::new(SimpleDataProvider::new());
        let mut machine = ScriptVersion::V2.init_core_machine(10_000);
        machine.set_register(A0, 0);
        machine.set_register(A1, Source::CellDep as u64);
        machine.set_register(A2, 99);
        machine.set_register(A3, 0);
        machine.set_register(A4, 0);
        machine.set_register(A5, 0);
        machine.set_register(A7, EXEC_SYSCALL_NUMBER);

        let mut syscall = Exec::new(tx, provider, vec![], vec![]);
        let err = syscall.ecall(&mut machine).expect_err("invalid place should trap");

        assert_eq!(err, VMError::External("Place parse_from_u64 99".to_string()));
    }
}
