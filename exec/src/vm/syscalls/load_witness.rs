// SPDX-License-Identifier: MIT
// Copyright (C) 2026 Spora developers
//
// Load witness syscall

use super::utils::{store_data, INDEX_OUT_OF_BOUND};
use super::Source;
use super::LOAD_WITNESS_SYSCALL_NUMBER;
use crate::celltx::CellTx;
use crate::vm::{transferred_byte_cycles, VmSemantics};
use ckb_vm::{
    registers::{A0, A3, A4, A7},
    Error as VMError, Register, SupportMachine, Syscalls,
};
use std::sync::Arc;

/// Syscall: Load Witness
///
/// Syscall number: 2074
pub struct LoadWitness {
    tx: Arc<CellTx>,
    group_input_indices: Vec<usize>,
    group_output_indices: Vec<usize>,
    semantics: VmSemantics,
}

impl LoadWitness {
    pub fn new(tx: Arc<CellTx>, group_input_indices: Vec<usize>, group_output_indices: Vec<usize>) -> Self {
        Self { tx, group_input_indices, group_output_indices, semantics: VmSemantics::SporaExtended }
    }

    pub fn with_semantics(mut self, semantics: VmSemantics) -> Self {
        self.semantics = semantics;
        self
    }

    fn source_witness_index(&self, source: Source, index: usize) -> Option<usize> {
        match source {
            Source::Input => self.tx.inputs.get(index).map(|_| index),
            Source::Output => self.tx.outputs.get(index).map(|_| self.tx.inputs.len().saturating_add(index)),
            Source::CellDep => {
                self.tx.cell_deps.get(index).map(|_| self.tx.inputs.len().saturating_add(self.tx.outputs.len()).saturating_add(index))
            }
            Source::HeaderDep => None,
            Source::GroupInput => self.group_input_indices.get(index).and_then(|&idx| self.tx.inputs.get(idx).map(|_| idx)),
            Source::GroupOutput => self
                .group_output_indices
                .get(index)
                .and_then(|&idx| self.tx.outputs.get(idx).map(|_| self.tx.inputs.len().saturating_add(idx))),
            Source::GroupCellDep | Source::GroupHeaderDep => None,
        }
    }

    fn get_witness(&self, source: Source, index: usize) -> Option<&[u8]> {
        let witness_index = self.source_witness_index(source, index)?;
        self.tx.witnesses.get(witness_index).map(|w| w.as_slice())
    }
}

impl<M: SupportMachine> Syscalls<M> for LoadWitness {
    fn initialize(&mut self, _machine: &mut M) -> Result<(), VMError> {
        Ok(())
    }

    fn ecall(&mut self, machine: &mut M) -> Result<bool, VMError> {
        let syscall_number = machine.registers()[A7].to_u64();

        // LOAD_WITNESS = 2074
        if syscall_number != LOAD_WITNESS_SYSCALL_NUMBER {
            return Ok(false);
        }

        let index = machine.registers()[A3].to_u64() as usize;
        let source = Source::parse_from_u64_for_semantics(machine.registers()[A4].to_u64(), self.semantics)?;

        // Get witness data
        let witness = match self.get_witness(source, index) {
            Some(w) => w,
            None => {
                machine.set_register(A0, M::REG::from_u8(INDEX_OUT_OF_BOUND));
                return Ok(true);
            }
        };

        // Store data using CKB-style store_data
        let result = store_data(machine, witness)?;
        machine.add_cycles_no_checking(transferred_byte_cycles(result.written_size))?;
        machine.set_register(A0, M::REG::from_u8(result.return_code));

        Ok(true)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::vm::syscalls::SUCCESS;
    use crate::vm::ScriptVersion;
    use ckb_vm::{
        registers::{A1, A2},
        CoreMachine, Memory, Register,
    };

    const BUFFER_ADDR: u64 = 0x1000;
    const SIZE_ADDR: u64 = 0x2000;

    #[test]
    fn test_load_witness_supports_partial_reads() {
        let tx = Arc::new(CellTx {
            version: 0xC001,
            inputs: vec![crate::celltx::CellInput::new(crate::celltx::OutPoint::new([0x01; 32], 0), 0)],
            cell_deps: vec![],
            header_deps: vec![],
            outputs: vec![],
            outputs_data: vec![],
            witnesses: vec![vec![1, 2, 3, 4, 5]],
        });

        let mut machine = ScriptVersion::V2.init_core_machine(10_000);
        machine.memory_mut().store64(&SIZE_ADDR, &2u64).unwrap();
        machine.set_register(A0, BUFFER_ADDR);
        machine.set_register(A1, SIZE_ADDR);
        machine.set_register(A2, 2);
        machine.set_register(A3, 0);
        machine.set_register(A4, 0x01);
        machine.set_register(A7, LOAD_WITNESS_SYSCALL_NUMBER);

        let mut syscall = LoadWitness::new(tx, vec![], vec![]);
        let handled = syscall.ecall(&mut machine).expect("load witness syscall should succeed");

        assert!(handled);
        assert_eq!(machine.registers()[A0].to_u64(), SUCCESS as u64);
        assert_eq!(machine.memory_mut().load64(&SIZE_ADDR).unwrap().to_u64(), 3);
        assert_eq!(machine.memory_mut().load_bytes(BUFFER_ADDR, 2).unwrap().as_ref(), &[3, 4]);
    }

    #[test]
    fn test_load_witness_rejects_invalid_source() {
        let tx = Arc::new(CellTx {
            version: 0xC001,
            inputs: vec![],
            cell_deps: vec![],
            header_deps: vec![],
            outputs: vec![],
            outputs_data: vec![],
            witnesses: vec![vec![1, 2, 3]],
        });

        let mut machine = ScriptVersion::V2.init_core_machine(10_000);
        machine.memory_mut().store64(&SIZE_ADDR, &3u64).unwrap();
        machine.set_register(A0, BUFFER_ADDR);
        machine.set_register(A1, SIZE_ADDR);
        machine.set_register(A2, 0);
        machine.set_register(A3, 0);
        machine.set_register(A4, 0x99);
        machine.set_register(A7, LOAD_WITNESS_SYSCALL_NUMBER);

        let mut syscall = LoadWitness::new(tx, vec![], vec![]);
        let err = syscall.ecall(&mut machine).expect_err("invalid source should trap");

        assert_eq!(err, VMError::External("SourceEntry parse_from_u64 153".to_string()));
    }

    #[test]
    fn test_load_witness_supports_output_and_group_output_sources() {
        let tx = Arc::new(CellTx {
            version: 0xC001,
            inputs: vec![
                crate::celltx::CellInput::new(crate::celltx::OutPoint::new([0x01; 32], 0), 0),
                crate::celltx::CellInput::new(crate::celltx::OutPoint::new([0x02; 32], 0), 0),
            ],
            cell_deps: vec![crate::celltx::CellDep {
                out_point: crate::celltx::OutPoint::new([0x03; 32], 0),
                dep_type: crate::celltx::DepType::Code,
            }],
            header_deps: vec![],
            outputs: vec![
                crate::celltx::CellOutput { capacity: 1, lock: crate::celltx::Script::new([0x11; 32], 0, vec![]), type_: None },
                crate::celltx::CellOutput { capacity: 2, lock: crate::celltx::Script::new([0x12; 32], 0, vec![]), type_: None },
                crate::celltx::CellOutput { capacity: 3, lock: crate::celltx::Script::new([0x13; 32], 0, vec![]), type_: None },
            ],
            outputs_data: vec![],
            witnesses: vec![
                vec![0xA0], // input 0
                vec![0xA1], // input 1
                vec![0xB0], // output 0
                vec![0xB1], // output 1
                vec![0xB2], // output 2
                vec![0xC0], // cell_dep 0
            ],
        });

        let syscall = LoadWitness::new(tx, vec![1], vec![2]);
        assert_eq!(syscall.get_witness(Source::Output, 1).unwrap(), &[0xB1]);
        assert_eq!(syscall.get_witness(Source::GroupOutput, 0).unwrap(), &[0xB2]);
        assert_eq!(syscall.get_witness(Source::CellDep, 0).unwrap(), &[0xC0]);
        assert_eq!(syscall.get_witness(Source::GroupInput, 0).unwrap(), &[0xA1]);
        assert_eq!(syscall.get_witness(Source::parse(0x0100_0000_0000_0001).unwrap(), 0).unwrap(), &[0xA1]);
        assert_eq!(syscall.get_witness(Source::parse(0x0100_0000_0000_0002).unwrap(), 0).unwrap(), &[0xB2]);
    }

    #[test]
    fn test_load_witness_group_output_partial_read() {
        let tx = Arc::new(CellTx {
            version: 0xC001,
            inputs: vec![crate::celltx::CellInput::new(crate::celltx::OutPoint::new([0x10; 32], 0), 0)],
            cell_deps: vec![],
            header_deps: vec![],
            outputs: vec![
                crate::celltx::CellOutput { capacity: 1, lock: crate::celltx::Script::new([0x21; 32], 0, vec![]), type_: None },
                crate::celltx::CellOutput { capacity: 2, lock: crate::celltx::Script::new([0x22; 32], 0, vec![]), type_: None },
                crate::celltx::CellOutput { capacity: 3, lock: crate::celltx::Script::new([0x23; 32], 0, vec![]), type_: None },
            ],
            outputs_data: vec![],
            witnesses: vec![
                vec![0x10],             // input 0
                vec![0x20],             // output 0
                vec![0x21],             // output 1
                vec![0x30, 0x31, 0x32], // output 2
            ],
        });

        let mut machine = ScriptVersion::V2.init_core_machine(10_000);
        machine.memory_mut().store64(&SIZE_ADDR, &2u64).unwrap();
        machine.set_register(A0, BUFFER_ADDR);
        machine.set_register(A1, SIZE_ADDR);
        machine.set_register(A2, 1);
        machine.set_register(A3, 0);
        machine.set_register(A4, Source::GroupOutput as u64);
        machine.set_register(A7, LOAD_WITNESS_SYSCALL_NUMBER);

        let mut syscall = LoadWitness::new(tx, vec![], vec![2]);
        let handled = syscall.ecall(&mut machine).expect("group output load witness syscall should succeed");

        assert!(handled);
        assert_eq!(machine.registers()[A0].to_u64(), SUCCESS as u64);
        assert_eq!(machine.memory_mut().load64(&SIZE_ADDR).unwrap().to_u64(), 2);
        assert_eq!(machine.memory_mut().load_bytes(BUFFER_ADDR, 2).unwrap().as_ref(), &[0x31, 0x32]);
    }

    #[test]
    fn test_load_witness_output_source_returns_index_out_when_witness_segment_missing() {
        let tx = Arc::new(CellTx {
            version: 0xC001,
            inputs: vec![crate::celltx::CellInput::new(crate::celltx::OutPoint::new([0x01; 32], 0), 0)],
            cell_deps: vec![],
            header_deps: vec![],
            outputs: vec![crate::celltx::CellOutput {
                capacity: 1,
                lock: crate::celltx::Script::new([0x11; 32], 0, vec![]),
                type_: None,
            }],
            outputs_data: vec![],
            witnesses: vec![vec![0xAA]], // only input witness
        });

        let mut machine = ScriptVersion::V2.init_core_machine(10_000);
        machine.memory_mut().store64(&SIZE_ADDR, &8u64).unwrap();
        machine.set_register(A0, BUFFER_ADDR);
        machine.set_register(A1, SIZE_ADDR);
        machine.set_register(A2, 0);
        machine.set_register(A3, 0);
        machine.set_register(A4, Source::Output as u64);
        machine.set_register(A7, LOAD_WITNESS_SYSCALL_NUMBER);

        let mut syscall = LoadWitness::new(tx, vec![], vec![]);
        let handled = syscall.ecall(&mut machine).expect("load witness syscall should be handled");

        assert!(handled);
        assert_eq!(machine.registers()[A0].to_u64(), INDEX_OUT_OF_BOUND as u64);
    }
}
