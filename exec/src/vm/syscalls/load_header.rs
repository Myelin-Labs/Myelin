// SPDX-License-Identifier: MIT
// Copyright (C) 2026 Myelin developers
//
// Load header syscall for CKB-style session headers.

use super::utils::{store_data, INDEX_OUT_OF_BOUND, ITEM_MISSING, WRONG_FORMAT};
use super::{HeaderField, Source, LOAD_HEADER_BY_FIELD_SYSCALL_NUMBER, LOAD_HEADER_SYSCALL_NUMBER};
use crate::celltx::CellTx;
use crate::serialization::molecule_compat::{
    ckb_header_epoch_length, ckb_header_epoch_number, ckb_header_epoch_start_block_number, serialize_ckb_header_molecule,
    serialize_resolved_header_molecule, CkbHeader,
};
use crate::serialization::VmAbiFormat;
use crate::vm::transferred_byte_cycles;
use crate::vm::{CellDataProvider, ResolvedHeader, VmSemantics};
use ckb_vm::{
    registers::{A0, A3, A4, A5, A7},
    Error as VMError, Register, SupportMachine, Syscalls,
};
use std::sync::Arc;

/// Syscall: Load Header
///
/// Syscall number: 2072
///
/// Note: Myelin-extended mode exposes a richer session header view through
/// `ResolvedHeader`; CKB-strict mode uses provider-supplied CKB header bytes.
pub struct LoadHeader<D: CellDataProvider> {
    tx: Arc<CellTx>,
    provider: Arc<D>,
    group_input_indices: Vec<usize>,
    group_output_indices: Vec<usize>,
    abi_format: VmAbiFormat,
    semantics: VmSemantics,
}

enum HeaderLookupResult<H> {
    Header(H),
    IndexOutOfBound,
    ItemMissing,
}

impl<D: CellDataProvider> LoadHeader<D> {
    pub fn new(tx: Arc<CellTx>, provider: Arc<D>, group_input_indices: Vec<usize>, group_output_indices: Vec<usize>) -> Self {
        Self {
            tx,
            provider,
            group_input_indices,
            group_output_indices,
            abi_format: VmAbiFormat::Molecule,
            semantics: VmSemantics::MyelinExtended,
        }
    }

    /// Select the VM ABI wire format used by full header loads.
    pub fn with_abi_format(mut self, abi_format: VmAbiFormat) -> Self {
        self.abi_format = abi_format;
        self
    }

    /// Select target-chain semantics for header loads.
    pub fn with_semantics(mut self, semantics: VmSemantics) -> Self {
        self.semantics = semantics;
        self
    }

    fn get_header(&self, source: Source, index: usize) -> HeaderLookupResult<ResolvedHeader> {
        match source {
            Source::Input => match self.tx.inputs.get(index) {
                Some(input) => self
                    .provider
                    .load_header_by_outpoint(&input.previous_output.tx_hash, input.previous_output.index)
                    .map(HeaderLookupResult::Header)
                    .unwrap_or(HeaderLookupResult::ItemMissing),
                None => HeaderLookupResult::IndexOutOfBound,
            },
            Source::CellDep => match self.tx.cell_deps.get(index) {
                Some(dep) => self
                    .provider
                    .load_header_by_outpoint(&dep.out_point.tx_hash, dep.out_point.index)
                    .map(HeaderLookupResult::Header)
                    .unwrap_or(HeaderLookupResult::ItemMissing),
                None => HeaderLookupResult::IndexOutOfBound,
            },
            Source::HeaderDep => match self.tx.header_deps.get(index) {
                Some(hash) => {
                    self.provider.load_header(hash).map(HeaderLookupResult::Header).unwrap_or(HeaderLookupResult::ItemMissing)
                }
                None => HeaderLookupResult::IndexOutOfBound,
            },
            Source::GroupInput => match self.group_input_indices.get(index).and_then(|&idx| self.tx.inputs.get(idx)) {
                Some(input) => self
                    .provider
                    .load_header_by_outpoint(&input.previous_output.tx_hash, input.previous_output.index)
                    .map(HeaderLookupResult::Header)
                    .unwrap_or(HeaderLookupResult::ItemMissing),
                None => HeaderLookupResult::IndexOutOfBound,
            },
            Source::Output => {
                if self.tx.outputs.get(index).is_some() {
                    HeaderLookupResult::ItemMissing
                } else {
                    HeaderLookupResult::IndexOutOfBound
                }
            }
            Source::GroupOutput => match self.group_output_indices.get(index).and_then(|&idx| self.tx.outputs.get(idx)) {
                Some(_) => HeaderLookupResult::ItemMissing,
                None => HeaderLookupResult::IndexOutOfBound,
            },
            Source::GroupCellDep | Source::GroupHeaderDep => HeaderLookupResult::IndexOutOfBound,
        }
    }

    fn get_ckb_header(&self, source: Source, index: usize) -> HeaderLookupResult<CkbHeader> {
        match source {
            Source::Input => match self.tx.inputs.get(index) {
                Some(input) => self.get_ckb_header_by_outpoint(&input.previous_output.tx_hash, input.previous_output.index),
                None => HeaderLookupResult::IndexOutOfBound,
            },
            Source::CellDep => match self.tx.cell_deps.get(index) {
                Some(dep) => self.get_ckb_header_by_outpoint(&dep.out_point.tx_hash, dep.out_point.index),
                None => HeaderLookupResult::IndexOutOfBound,
            },
            Source::HeaderDep => match self.tx.header_deps.get(index) {
                Some(hash) => {
                    self.provider.load_ckb_header(hash).map(HeaderLookupResult::Header).unwrap_or(HeaderLookupResult::ItemMissing)
                }
                None => HeaderLookupResult::IndexOutOfBound,
            },
            Source::GroupInput => match self.group_input_indices.get(index).and_then(|&idx| self.tx.inputs.get(idx)) {
                Some(input) => self.get_ckb_header_by_outpoint(&input.previous_output.tx_hash, input.previous_output.index),
                None => HeaderLookupResult::IndexOutOfBound,
            },
            Source::Output | Source::GroupOutput | Source::GroupCellDep | Source::GroupHeaderDep => {
                HeaderLookupResult::IndexOutOfBound
            }
        }
    }

    fn get_ckb_header_by_outpoint(&self, tx_hash: &[u8; 32], index: u32) -> HeaderLookupResult<CkbHeader> {
        let Some(header_hash) = self.provider.load_ckb_header_hash_by_outpoint(tx_hash, index) else {
            return HeaderLookupResult::ItemMissing;
        };
        if !self.tx.header_deps.iter().any(|hash| *hash == header_hash) {
            return HeaderLookupResult::ItemMissing;
        }
        self.provider.load_ckb_header(&header_hash).map(HeaderLookupResult::Header).unwrap_or(HeaderLookupResult::ItemMissing)
    }

    fn serialize_header_field(&self, header: &ResolvedHeader, field: u64) -> Result<Vec<u8>, VMError> {
        match HeaderField::parse_from_u64(field)? {
            HeaderField::Number => Ok(header.number.to_le_bytes().to_vec()),
            HeaderField::Timestamp => Ok(header.timestamp.to_le_bytes().to_vec()),
            HeaderField::Hash => Ok(header.hash.to_vec()),
            HeaderField::ParentHash => Ok(header.parent_hash.to_vec()),
            HeaderField::Version => Ok(header.version.to_le_bytes().to_vec()),
            HeaderField::CompactTarget => Ok(header.compact_target.to_le_bytes().to_vec()),
            HeaderField::Nonce => Ok(header.nonce.to_le_bytes().to_vec()),
            HeaderField::TransactionsRoot => Ok(header.transactions_root.to_vec()),
            HeaderField::ProposalsHash => Ok(header.proposals_hash.to_vec()),
            HeaderField::CellCommitment => Ok(header.cell_commitment.to_vec()),
            HeaderField::CellRoot => Ok(header.cell_root.to_vec()),
            HeaderField::SegmentRoot => Ok(header.segment_root.to_vec()),
            HeaderField::Epoch => Ok(header.epoch.to_le_bytes().to_vec()),
            HeaderField::Dao => Ok(header.dao.to_vec()),
            HeaderField::UnclesHash => Ok(header.uncles_hash.to_vec()),
        }
    }

    fn serialize_header(&self, header: &ResolvedHeader) -> Result<Vec<u8>, VMError> {
        match self.abi_format {
            VmAbiFormat::Molecule => serialize_resolved_header_molecule(header).map_err(|e| VMError::External(e.to_string())),
        }
    }

    fn serialize_ckb_header_field(&self, header: &CkbHeader, field: u64) -> Result<Vec<u8>, VMError> {
        let value = match field {
            0 => ckb_header_epoch_number(&header.raw),
            1 => ckb_header_epoch_start_block_number(&header.raw).map_err(|e| VMError::Unexpected(e.to_string()))?,
            2 => ckb_header_epoch_length(&header.raw),
            _ => return Err(VMError::External(format!("HeaderField parse_from_u64 {field}"))),
        };
        Ok(value.to_le_bytes().to_vec())
    }

    fn serialize_ckb_header(&self, header: &CkbHeader) -> Result<Vec<u8>, VMError> {
        serialize_ckb_header_molecule(header).map_err(|e| VMError::External(e.to_string()))
    }
}

impl<D: CellDataProvider, M: SupportMachine> Syscalls<M> for LoadHeader<D> {
    fn initialize(&mut self, _machine: &mut M) -> Result<(), VMError> {
        Ok(())
    }

    fn ecall(&mut self, machine: &mut M) -> Result<bool, VMError> {
        let syscall_number = machine.registers()[A7].to_u64();

        // LOAD_HEADER = 2072 or LOAD_HEADER_BY_FIELD = 2082
        if syscall_number != LOAD_HEADER_SYSCALL_NUMBER && syscall_number != LOAD_HEADER_BY_FIELD_SYSCALL_NUMBER {
            return Ok(false);
        }

        let index = machine.registers()[A3].to_u64() as usize;
        let source = Source::parse_from_u64_for_semantics(machine.registers()[A4].to_u64(), self.semantics)?;

        if self.semantics == VmSemantics::CkbStrict {
            let header = match self.get_ckb_header(source, index) {
                HeaderLookupResult::Header(header) => header,
                HeaderLookupResult::IndexOutOfBound => {
                    machine.set_register(A0, M::REG::from_u8(INDEX_OUT_OF_BOUND));
                    return Ok(true);
                }
                HeaderLookupResult::ItemMissing => {
                    machine.set_register(A0, M::REG::from_u8(ITEM_MISSING));
                    return Ok(true);
                }
            };

            let data = if syscall_number == LOAD_HEADER_BY_FIELD_SYSCALL_NUMBER {
                let field = machine.registers()[A5].to_u64();
                self.serialize_ckb_header_field(&header, field)?
            } else {
                self.serialize_ckb_header(&header)?
            };
            let result = store_data(machine, &data)?;
            machine.add_cycles_no_checking(transferred_byte_cycles(result.written_size))?;
            machine.set_register(A0, M::REG::from_u8(result.return_code));
            return Ok(true);
        }

        let header = match self.get_header(source, index) {
            HeaderLookupResult::Header(header) => header,
            HeaderLookupResult::IndexOutOfBound => {
                machine.set_register(A0, M::REG::from_u8(INDEX_OUT_OF_BOUND));
                return Ok(true);
            }
            HeaderLookupResult::ItemMissing => {
                machine.set_register(A0, M::REG::from_u8(ITEM_MISSING));
                return Ok(true);
            }
        };

        if !self.semantics.allow_myelin_header_abi() {
            machine.set_register(A0, M::REG::from_u8(WRONG_FORMAT));
            return Ok(true);
        }

        let data = if syscall_number == LOAD_HEADER_BY_FIELD_SYSCALL_NUMBER {
            let field = machine.registers()[A5].to_u64();
            self.serialize_header_field(&header, field)?
        } else {
            self.serialize_header(&header)?
        };
        let result = store_data(machine, &data)?;
        machine.add_cycles_no_checking(transferred_byte_cycles(result.written_size))?;
        machine.set_register(A0, M::REG::from_u8(result.return_code));

        Ok(true)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::serialization::molecule_compat::{
        ckb_epoch_number_with_fraction_full_value, ckb_header_hash_molecule, serialize_ckb_header_molecule,
        serialize_resolved_header_molecule, CkbHeader, CkbRawHeader,
    };
    use crate::serialization::{VmAbiFormat, VmSerializable};
    use crate::vm::syscalls::SUCCESS;
    use crate::vm::{ScriptVersion, SimpleDataProvider, VmSemantics};
    use ckb_vm::{
        registers::{A1, A2},
        CoreMachine, Memory, Register,
    };

    const BUFFER_ADDR: u64 = 0x1000;
    const SIZE_ADDR: u64 = 0x2000;

    fn resolved_header(header_hash: [u8; 32]) -> ResolvedHeader {
        ResolvedHeader {
            hash: header_hash,
            version: 7,
            parent_hash: [0xAA; 32],
            transactions_root: [0x10; 32],
            proposals_hash: [0x20; 32],
            cell_commitment: [0x30; 32],
            cell_root: [0x40; 32],
            segment_root: [0x50; 32],
            timestamp: 0x0102_0304_0506_0708,
            compact_target: 0x1d00_ffff,
            nonce: 0x8877_6655_4433_2211,
            number: 0x1122_3344_5566_7788,
            dao: [0x60; 32],
            epoch: 0x99AA_BBCC_DDEE_FF00,
            uncles_hash: [0x70; 32],
        }
    }

    fn ckb_header(number: u64, epoch_index: u64) -> CkbHeader {
        CkbHeader {
            raw: CkbRawHeader {
                version: 7,
                compact_target: 0x1d00_ffff,
                timestamp: 0x0102_0304_0506_0708,
                number,
                epoch: ckb_epoch_number_with_fraction_full_value(1, epoch_index, 1000).unwrap(),
                parent_hash: [0xAA; 32],
                transactions_root: [0x10; 32],
                proposals_hash: [0x20; 32],
                extra_hash: [0x30; 32],
                dao: [0x40; 32],
            },
            nonce: 0x8877_6655_4433_2211,
        }
    }

    fn build_ckb_tx_and_provider() -> (Arc<CellTx>, Arc<SimpleDataProvider>, [u8; 32], CkbHeader, [u8; 32], CkbHeader) {
        let header = ckb_header(1234, 40);
        let header_hash = ckb_header_hash_molecule(&header).unwrap();
        let input_header = ckb_header(4567, 7);
        let input_header_hash = ckb_header_hash_molecule(&input_header).unwrap();
        let input_out_point = crate::celltx::OutPoint::new([0x11; 32], 0);
        let tx = Arc::new(CellTx {
            version: 0xC001,
            inputs: vec![crate::celltx::CellInput::new(input_out_point, 0)],
            cell_deps: vec![],
            header_deps: vec![header_hash, input_header_hash],
            outputs: vec![],
            outputs_data: vec![],
            witnesses: vec![],
        });
        let mut provider = SimpleDataProvider::new();
        provider.add_cell_with_header(
            [0x11; 32],
            0,
            crate::vm::ResolvedCell {
                cell_output: crate::celltx::CellOutput {
                    capacity: 1_000,
                    lock: crate::celltx::Script::new([0x01; 32], 0, vec![]),
                    type_: None,
                },
                data: Some(vec![]),
            },
            input_header_hash,
        );
        provider.add_ckb_header(header_hash, header.clone());
        provider.add_ckb_header(input_header_hash, input_header.clone());
        (tx, Arc::new(provider), header_hash, header, input_header_hash, input_header)
    }

    fn build_tx_and_provider() -> (Arc<CellTx>, Arc<SimpleDataProvider>) {
        let input_out_point = crate::celltx::OutPoint::new([0x11; 32], 0);
        let dep_out_point = crate::celltx::OutPoint::new([0x22; 32], 1);
        let input_header_hash = [0x55; 32];
        let dep_header_hash = [0x66; 32];
        let header_hash = [0x77; 32];
        let tx = Arc::new(CellTx {
            version: 0xC001,
            inputs: vec![crate::celltx::CellInput::new(input_out_point, 0)],
            cell_deps: vec![crate::celltx::CellDep { out_point: dep_out_point, dep_type: crate::celltx::DepType::Code }],
            header_deps: vec![header_hash],
            outputs: vec![],
            outputs_data: vec![],
            witnesses: vec![],
        });
        let mut provider = SimpleDataProvider::new();
        provider.add_cell_with_header(
            [0x11; 32],
            0,
            crate::vm::ResolvedCell {
                cell_output: crate::celltx::CellOutput {
                    capacity: 1_000,
                    lock: crate::celltx::Script::new([0x01; 32], 0, vec![]),
                    type_: None,
                },
                data: Some(vec![]),
            },
            input_header_hash,
        );
        provider.add_cell_with_header(
            [0x22; 32],
            1,
            crate::vm::ResolvedCell {
                cell_output: crate::celltx::CellOutput {
                    capacity: 2_000,
                    lock: crate::celltx::Script::new([0x02; 32], 0, vec![]),
                    type_: None,
                },
                data: Some(vec![]),
            },
            dep_header_hash,
        );
        provider.add_header(input_header_hash, resolved_header(input_header_hash));
        provider.add_header(dep_header_hash, resolved_header(dep_header_hash));
        provider.add_header(header_hash, resolved_header(header_hash));
        (tx, Arc::new(provider))
    }

    #[test]
    fn test_load_header_by_field_supports_partial_reads() {
        let (tx, provider) = build_tx_and_provider();
        let mut machine = ScriptVersion::V2.init_core_machine(10_000);
        machine.memory_mut().store64(&SIZE_ADDR, &4u64).unwrap();
        machine.set_register(A0, BUFFER_ADDR);
        machine.set_register(A1, SIZE_ADDR);
        machine.set_register(A2, 2);
        machine.set_register(A3, 0);
        machine.set_register(A4, Source::HeaderDep as u64);
        machine.set_register(A5, HeaderField::Timestamp as u64);
        machine.set_register(A7, LOAD_HEADER_BY_FIELD_SYSCALL_NUMBER);

        let mut syscall = LoadHeader::new(tx, provider, vec![0], vec![]);
        let handled = syscall.ecall(&mut machine).expect("load header syscall should succeed");

        assert!(handled);
        assert_eq!(machine.registers()[A0].to_u64(), SUCCESS as u64);
        assert_eq!(machine.memory_mut().load64(&SIZE_ADDR).unwrap().to_u64(), 6);
        assert_eq!(machine.memory_mut().load_bytes(BUFFER_ADDR, 4).unwrap().as_ref(), &[0x06, 0x05, 0x04, 0x03]);
    }

    #[test]
    fn test_load_header_output_source_reports_item_missing_when_output_exists() {
        let (tx, provider) = build_tx_and_provider();
        let tx = Arc::new(CellTx {
            outputs: vec![crate::celltx::CellOutput {
                capacity: 42,
                lock: crate::celltx::Script::new([0x33; 32], 0, vec![]),
                type_: None,
            }],
            outputs_data: vec![vec![]],
            ..(*tx).clone()
        });
        let mut machine = ScriptVersion::V2.init_core_machine(10_000);
        machine.memory_mut().store64(&SIZE_ADDR, &8u64).unwrap();
        machine.set_register(A0, BUFFER_ADDR);
        machine.set_register(A1, SIZE_ADDR);
        machine.set_register(A2, 0);
        machine.set_register(A3, 0);
        machine.set_register(A4, Source::Output as u64);
        machine.set_register(A7, LOAD_HEADER_SYSCALL_NUMBER);

        let mut syscall = LoadHeader::new(tx, provider, vec![0], vec![]);
        let handled = syscall.ecall(&mut machine).expect("load header syscall should be handled");

        assert!(handled);
        assert_eq!(machine.registers()[A0].to_u64(), ITEM_MISSING as u64);
    }

    #[test]
    fn test_load_header_by_field_rejects_unknown_field() {
        let (tx, provider) = build_tx_and_provider();
        let mut machine = ScriptVersion::V2.init_core_machine(10_000);
        machine.memory_mut().store64(&SIZE_ADDR, &8u64).unwrap();
        machine.set_register(A0, BUFFER_ADDR);
        machine.set_register(A1, SIZE_ADDR);
        machine.set_register(A2, 0);
        machine.set_register(A3, 0);
        machine.set_register(A4, Source::HeaderDep as u64);
        machine.set_register(A5, 99);
        machine.set_register(A7, LOAD_HEADER_BY_FIELD_SYSCALL_NUMBER);

        let mut syscall = LoadHeader::new(tx, provider, vec![0], vec![]);
        let err = syscall.ecall(&mut machine).expect_err("unknown field should trap");

        assert_eq!(err, VMError::External("HeaderField parse_from_u64 99".to_string()));
    }

    #[test]
    fn test_load_header_returns_richer_header_view() {
        let (tx, provider) = build_tx_and_provider();
        let mut machine = ScriptVersion::V2.init_core_machine(10_000);
        machine.memory_mut().store64(&SIZE_ADDR, &512u64).unwrap();
        machine.set_register(A0, BUFFER_ADDR);
        machine.set_register(A1, SIZE_ADDR);
        machine.set_register(A2, 0);
        machine.set_register(A3, 0);
        machine.set_register(A4, Source::HeaderDep as u64);
        machine.set_register(A7, LOAD_HEADER_SYSCALL_NUMBER);

        let mut syscall = LoadHeader::new(tx, provider, vec![0], vec![]);
        let handled = syscall.ecall(&mut machine).expect("load header syscall should succeed");

        assert!(handled);
        assert_eq!(machine.registers()[A0].to_u64(), SUCCESS as u64);
        let size = machine.memory_mut().load64(&SIZE_ADDR).unwrap().to_u64();
        let bytes = machine.memory_mut().load_bytes(BUFFER_ADDR, size).unwrap();
        // Use VmSerializable for deserialization to match syscall serialization
        let header = ResolvedHeader::from_vm_bytes(bytes.as_ref()).expect("header should deserialize");
        assert_eq!(header, resolved_header([0x77; 32]));
    }

    #[test]
    fn test_load_header_by_field_supports_dao() {
        let (tx, provider) = build_tx_and_provider();
        let mut machine = ScriptVersion::V2.init_core_machine(10_000);
        machine.memory_mut().store64(&SIZE_ADDR, &24u64).unwrap();
        machine.set_register(A0, BUFFER_ADDR);
        machine.set_register(A1, SIZE_ADDR);
        machine.set_register(A2, 0);
        machine.set_register(A3, 0);
        machine.set_register(A4, Source::HeaderDep as u64);
        machine.set_register(A5, HeaderField::Dao as u64);
        machine.set_register(A7, LOAD_HEADER_BY_FIELD_SYSCALL_NUMBER);

        let mut syscall = LoadHeader::new(tx, provider, vec![0], vec![]);
        let handled = syscall.ecall(&mut machine).expect("load header by field should succeed");

        assert!(handled);
        assert_eq!(machine.registers()[A0].to_u64(), SUCCESS as u64);
        assert_eq!(machine.memory_mut().load64(&SIZE_ADDR).unwrap().to_u64(), 32);
        assert_eq!(machine.memory_mut().load_bytes(BUFFER_ADDR, 24).unwrap().as_ref(), &[0x60; 24]);
    }

    #[test]
    fn test_load_header_supports_input_source() {
        let (tx, provider) = build_tx_and_provider();
        let mut machine = ScriptVersion::V2.init_core_machine(10_000);
        machine.memory_mut().store64(&SIZE_ADDR, &512u64).unwrap();
        machine.set_register(A0, BUFFER_ADDR);
        machine.set_register(A1, SIZE_ADDR);
        machine.set_register(A2, 0);
        machine.set_register(A3, 0);
        machine.set_register(A4, Source::Input as u64);
        machine.set_register(A7, LOAD_HEADER_SYSCALL_NUMBER);

        let mut syscall = LoadHeader::new(tx, provider, vec![0], vec![]);
        let handled = syscall.ecall(&mut machine).expect("load header syscall should succeed");

        assert!(handled);
        assert_eq!(machine.registers()[A0].to_u64(), SUCCESS as u64);
        let size = machine.memory_mut().load64(&SIZE_ADDR).unwrap().to_u64();
        let bytes = machine.memory_mut().load_bytes(BUFFER_ADDR, size).unwrap();
        let header = ResolvedHeader::from_vm_bytes(bytes.as_ref()).expect("header should deserialize");
        assert_eq!(header.hash, [0x55; 32]);
    }

    #[test]
    fn test_load_header_molecule_abi_full_load() {
        let (tx, provider) = build_tx_and_provider();
        let header = provider.load_header(&tx.header_deps[0]).expect("header dep should exist");
        let expected = serialize_resolved_header_molecule(&header).unwrap();
        let mut machine = ScriptVersion::V2.init_core_machine(10_000);
        machine.memory_mut().store64(&SIZE_ADDR, &(expected.len() as u64)).unwrap();
        machine.set_register(A0, BUFFER_ADDR);
        machine.set_register(A1, SIZE_ADDR);
        machine.set_register(A2, 0);
        machine.set_register(A3, 0);
        machine.set_register(A4, Source::HeaderDep as u64);
        machine.set_register(A7, LOAD_HEADER_SYSCALL_NUMBER);

        let mut syscall = LoadHeader::new(tx, provider, vec![0], vec![]).with_abi_format(VmAbiFormat::Molecule);
        let handled = syscall.ecall(&mut machine).expect("load header syscall should succeed");

        assert!(handled);
        assert_eq!(machine.registers()[A0].to_u64(), SUCCESS as u64);
        assert_eq!(machine.memory_mut().load64(&SIZE_ADDR).unwrap().to_u64(), expected.len() as u64);
        assert_eq!(machine.memory_mut().load_bytes(BUFFER_ADDR, expected.len() as u64).unwrap().as_ref(), expected.as_slice());
    }

    #[test]
    fn test_load_header_ckb_strict_does_not_fallback_to_myelin_header_abi() {
        let (tx, provider) = build_tx_and_provider();

        for syscall_number in [LOAD_HEADER_SYSCALL_NUMBER, LOAD_HEADER_BY_FIELD_SYSCALL_NUMBER] {
            let mut machine = ScriptVersion::V2.init_core_machine(10_000);
            machine.memory_mut().store64(&SIZE_ADDR, &8u64).unwrap();
            machine.set_register(A0, BUFFER_ADDR);
            machine.set_register(A1, SIZE_ADDR);
            machine.set_register(A2, 0);
            machine.set_register(A3, 0);
            machine.set_register(A4, Source::HeaderDep as u64);
            machine.set_register(A5, HeaderField::Number as u64);
            machine.set_register(A7, syscall_number);

            let mut syscall =
                LoadHeader::new(Arc::clone(&tx), Arc::clone(&provider), vec![0], vec![]).with_semantics(VmSemantics::CkbStrict);
            let handled = syscall.ecall(&mut machine).expect("load header syscall should be handled");

            assert!(handled);
            assert_eq!(machine.registers()[A0].to_u64(), ITEM_MISSING as u64);
        }
    }

    #[test]
    fn test_load_header_ckb_strict_loads_packed_ckb_header_dep() {
        let (tx, provider, _header_hash, header, _input_header_hash, _input_header) = build_ckb_tx_and_provider();
        let expected = serialize_ckb_header_molecule(&header).unwrap();
        let mut machine = ScriptVersion::V2.init_core_machine(10_000);
        machine.memory_mut().store64(&SIZE_ADDR, &(expected.len() as u64)).unwrap();
        machine.set_register(A0, BUFFER_ADDR);
        machine.set_register(A1, SIZE_ADDR);
        machine.set_register(A2, 0);
        machine.set_register(A3, 0);
        machine.set_register(A4, Source::HeaderDep as u64);
        machine.set_register(A7, LOAD_HEADER_SYSCALL_NUMBER);

        let mut syscall = LoadHeader::new(tx, provider, vec![0], vec![]).with_semantics(VmSemantics::CkbStrict);
        let handled = syscall.ecall(&mut machine).expect("load CKB header syscall should succeed");

        assert!(handled);
        assert_eq!(machine.registers()[A0].to_u64(), SUCCESS as u64);
        assert_eq!(machine.memory_mut().load64(&SIZE_ADDR).unwrap().to_u64(), expected.len() as u64);
        assert_eq!(machine.memory_mut().load_bytes(BUFFER_ADDR, expected.len() as u64).unwrap().as_ref(), expected.as_slice());
    }

    #[test]
    fn test_load_header_by_field_ckb_strict_uses_ckb_epoch_fields() {
        let (tx, provider, _header_hash, _header, _input_header_hash, _input_header) = build_ckb_tx_and_provider();
        let expected = [(0, 1u64), (1, 1194u64), (2, 1000u64)];

        for (field, value) in expected {
            let mut machine = ScriptVersion::V2.init_core_machine(10_000);
            machine.memory_mut().store64(&SIZE_ADDR, &8u64).unwrap();
            machine.set_register(A0, BUFFER_ADDR);
            machine.set_register(A1, SIZE_ADDR);
            machine.set_register(A2, 0);
            machine.set_register(A3, 0);
            machine.set_register(A4, Source::HeaderDep as u64);
            machine.set_register(A5, field);
            machine.set_register(A7, LOAD_HEADER_BY_FIELD_SYSCALL_NUMBER);

            let mut syscall =
                LoadHeader::new(Arc::clone(&tx), Arc::clone(&provider), vec![0], vec![]).with_semantics(VmSemantics::CkbStrict);
            let handled = syscall.ecall(&mut machine).expect("load CKB header field should succeed");

            assert!(handled);
            assert_eq!(machine.registers()[A0].to_u64(), SUCCESS as u64);
            assert_eq!(machine.memory_mut().load64(&SIZE_ADDR).unwrap().to_u64(), 8);
            assert_eq!(machine.memory_mut().load_bytes(BUFFER_ADDR, 8).unwrap().as_ref(), value.to_le_bytes().as_ref());
        }
    }

    #[test]
    fn test_load_header_ckb_strict_input_requires_header_dep_membership() {
        let (tx, provider, _header_hash, _header, _input_header_hash, input_header) = build_ckb_tx_and_provider();
        let expected = serialize_ckb_header_molecule(&input_header).unwrap();
        let mut machine = ScriptVersion::V2.init_core_machine(10_000);
        machine.memory_mut().store64(&SIZE_ADDR, &(expected.len() as u64)).unwrap();
        machine.set_register(A0, BUFFER_ADDR);
        machine.set_register(A1, SIZE_ADDR);
        machine.set_register(A2, 0);
        machine.set_register(A3, 0);
        machine.set_register(A4, Source::Input as u64);
        machine.set_register(A7, LOAD_HEADER_SYSCALL_NUMBER);

        let mut syscall =
            LoadHeader::new(Arc::clone(&tx), Arc::clone(&provider), vec![0], vec![]).with_semantics(VmSemantics::CkbStrict);
        let handled = syscall.ecall(&mut machine).expect("load CKB input header should succeed");
        assert!(handled);
        assert_eq!(machine.registers()[A0].to_u64(), SUCCESS as u64);
        assert_eq!(machine.memory_mut().load_bytes(BUFFER_ADDR, expected.len() as u64).unwrap().as_ref(), expected.as_slice());

        let tx_without_input_header_dep = Arc::new(CellTx { header_deps: vec![], ..(*tx).clone() });
        let mut machine = ScriptVersion::V2.init_core_machine(10_000);
        machine.memory_mut().store64(&SIZE_ADDR, &8u64).unwrap();
        machine.set_register(A0, BUFFER_ADDR);
        machine.set_register(A1, SIZE_ADDR);
        machine.set_register(A2, 0);
        machine.set_register(A3, 0);
        machine.set_register(A4, Source::Input as u64);
        machine.set_register(A7, LOAD_HEADER_SYSCALL_NUMBER);

        let mut syscall = LoadHeader::new(tx_without_input_header_dep, Arc::clone(&provider), vec![0], vec![])
            .with_semantics(VmSemantics::CkbStrict);
        let handled = syscall.ecall(&mut machine).expect("load CKB input header should be handled");
        assert!(handled);
        assert_eq!(machine.registers()[A0].to_u64(), ITEM_MISSING as u64);
    }

    #[test]
    fn test_load_header_ckb_strict_output_source_is_index_out_of_bound() {
        let (tx, provider, _header_hash, _header, _input_header_hash, _input_header) = build_ckb_tx_and_provider();
        let tx = Arc::new(CellTx {
            outputs: vec![crate::celltx::CellOutput {
                capacity: 42,
                lock: crate::celltx::Script::new([0x33; 32], 0, vec![]),
                type_: None,
            }],
            outputs_data: vec![vec![]],
            ..(*tx).clone()
        });
        let mut machine = ScriptVersion::V2.init_core_machine(10_000);
        machine.memory_mut().store64(&SIZE_ADDR, &8u64).unwrap();
        machine.set_register(A0, BUFFER_ADDR);
        machine.set_register(A1, SIZE_ADDR);
        machine.set_register(A2, 0);
        machine.set_register(A3, 0);
        machine.set_register(A4, Source::Output as u64);
        machine.set_register(A7, LOAD_HEADER_SYSCALL_NUMBER);

        let mut syscall = LoadHeader::new(tx, provider, vec![0], vec![]).with_semantics(VmSemantics::CkbStrict);
        let handled = syscall.ecall(&mut machine).expect("load CKB output header should be handled");

        assert!(handled);
        assert_eq!(machine.registers()[A0].to_u64(), INDEX_OUT_OF_BOUND as u64);
    }

    #[test]
    fn test_load_header_supports_cell_dep_source() {
        let (tx, provider) = build_tx_and_provider();
        let mut machine = ScriptVersion::V2.init_core_machine(10_000);
        machine.memory_mut().store64(&SIZE_ADDR, &32u64).unwrap();
        machine.set_register(A0, BUFFER_ADDR);
        machine.set_register(A1, SIZE_ADDR);
        machine.set_register(A2, 0);
        machine.set_register(A3, 0);
        machine.set_register(A4, Source::CellDep as u64);
        machine.set_register(A5, HeaderField::Hash as u64);
        machine.set_register(A7, LOAD_HEADER_BY_FIELD_SYSCALL_NUMBER);

        let mut syscall = LoadHeader::new(tx, provider, vec![0], vec![]);
        let handled = syscall.ecall(&mut machine).expect("load header by field should succeed");

        assert!(handled);
        assert_eq!(machine.registers()[A0].to_u64(), SUCCESS as u64);
        assert_eq!(machine.memory_mut().load_bytes(BUFFER_ADDR, 32).unwrap().as_ref(), &[0x66; 32]);
    }

    #[test]
    fn test_load_header_supports_group_input_source() {
        let (tx, provider) = build_tx_and_provider();
        let mut machine = ScriptVersion::V2.init_core_machine(10_000);
        machine.memory_mut().store64(&SIZE_ADDR, &32u64).unwrap();
        machine.set_register(A0, BUFFER_ADDR);
        machine.set_register(A1, SIZE_ADDR);
        machine.set_register(A2, 0);
        machine.set_register(A3, 0);
        machine.set_register(A4, Source::GroupInput as u64);
        machine.set_register(A5, HeaderField::Hash as u64);
        machine.set_register(A7, LOAD_HEADER_BY_FIELD_SYSCALL_NUMBER);

        let mut syscall = LoadHeader::new(tx, provider, vec![0], vec![]);
        let handled = syscall.ecall(&mut machine).expect("load header by field should succeed");

        assert!(handled);
        assert_eq!(machine.registers()[A0].to_u64(), SUCCESS as u64);
        assert_eq!(machine.memory_mut().load_bytes(BUFFER_ADDR, 32).unwrap().as_ref(), &[0x55; 32]);
    }

    #[test]
    fn test_load_header_returns_item_missing_when_input_header_not_found() {
        let input_out_point = crate::celltx::OutPoint::new([0x11; 32], 0);
        let tx = Arc::new(CellTx {
            version: 0xC001,
            inputs: vec![crate::celltx::CellInput::new(input_out_point, 0)],
            cell_deps: vec![],
            header_deps: vec![],
            outputs: vec![],
            outputs_data: vec![],
            witnesses: vec![],
        });
        let mut provider = SimpleDataProvider::new();
        provider.add_cell_with_header(
            [0x11; 32],
            0,
            crate::vm::ResolvedCell {
                cell_output: crate::celltx::CellOutput {
                    capacity: 1_000,
                    lock: crate::celltx::Script::new([0x01; 32], 0, vec![]),
                    type_: None,
                },
                data: Some(vec![]),
            },
            [0xAA; 32],
        );

        let mut machine = ScriptVersion::V2.init_core_machine(10_000);
        machine.memory_mut().store64(&SIZE_ADDR, &8u64).unwrap();
        machine.set_register(A0, BUFFER_ADDR);
        machine.set_register(A1, SIZE_ADDR);
        machine.set_register(A2, 0);
        machine.set_register(A3, 0);
        machine.set_register(A4, Source::Input as u64);
        machine.set_register(A7, LOAD_HEADER_SYSCALL_NUMBER);

        let mut syscall = LoadHeader::new(tx, Arc::new(provider), vec![0], vec![]);
        let handled = syscall.ecall(&mut machine).expect("load header syscall should be handled");
        assert!(handled);
        assert_eq!(machine.registers()[A0].to_u64(), ITEM_MISSING as u64);
    }

    #[test]
    fn test_load_header_group_output_reports_item_missing_when_output_exists() {
        let (tx, provider) = build_tx_and_provider();
        let tx = Arc::new(CellTx {
            outputs: vec![crate::celltx::CellOutput {
                capacity: 1_000,
                lock: crate::celltx::Script::new([0x44; 32], 0, vec![]),
                type_: None,
            }],
            outputs_data: vec![vec![]],
            ..(*tx).clone()
        });
        let mut machine = ScriptVersion::V2.init_core_machine(10_000);
        machine.memory_mut().store64(&SIZE_ADDR, &8u64).unwrap();
        machine.set_register(A0, BUFFER_ADDR);
        machine.set_register(A1, SIZE_ADDR);
        machine.set_register(A2, 0);
        machine.set_register(A3, 0);
        machine.set_register(A4, Source::GroupOutput as u64);
        machine.set_register(A7, LOAD_HEADER_SYSCALL_NUMBER);

        let mut syscall = LoadHeader::new(tx, provider, vec![0], vec![0]);
        let handled = syscall.ecall(&mut machine).expect("load header syscall should be handled");
        assert!(handled);
        assert_eq!(machine.registers()[A0].to_u64(), ITEM_MISSING as u64);
    }

    #[test]
    fn test_load_header_by_field_covers_all_supported_fields() {
        let (tx, provider) = build_tx_and_provider();
        let syscall = LoadHeader::new(tx, provider, vec![0], vec![]);
        let header = resolved_header([0x77; 32]);

        let expected = vec![
            (HeaderField::Number as u64, header.number.to_le_bytes().to_vec()),
            (HeaderField::Timestamp as u64, header.timestamp.to_le_bytes().to_vec()),
            (HeaderField::Hash as u64, header.hash.to_vec()),
            (HeaderField::ParentHash as u64, header.parent_hash.to_vec()),
            (HeaderField::Version as u64, header.version.to_le_bytes().to_vec()),
            (HeaderField::CompactTarget as u64, header.compact_target.to_le_bytes().to_vec()),
            (HeaderField::Nonce as u64, header.nonce.to_le_bytes().to_vec()),
            (HeaderField::TransactionsRoot as u64, header.transactions_root.to_vec()),
            (HeaderField::ProposalsHash as u64, header.proposals_hash.to_vec()),
            (HeaderField::CellCommitment as u64, header.cell_commitment.to_vec()),
            (HeaderField::CellRoot as u64, header.cell_root.to_vec()),
            (HeaderField::SegmentRoot as u64, header.segment_root.to_vec()),
            (HeaderField::Epoch as u64, header.epoch.to_le_bytes().to_vec()),
            (HeaderField::Dao as u64, header.dao.to_vec()),
            (HeaderField::UnclesHash as u64, header.uncles_hash.to_vec()),
        ];

        for (field, bytes) in expected {
            let actual = syscall.serialize_header_field(&header, field).expect("known field should serialize");
            assert_eq!(actual, bytes, "field {field} serialization mismatch");
        }
    }

    #[test]
    fn test_load_header_returns_item_missing_when_header_dep_not_found() {
        let missing_header_hash = [0x99; 32];
        let tx = Arc::new(CellTx {
            version: 0xC001,
            inputs: vec![],
            cell_deps: vec![],
            header_deps: vec![missing_header_hash],
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
        machine.set_register(A7, LOAD_HEADER_SYSCALL_NUMBER);

        let mut syscall = LoadHeader::new(tx, Arc::new(SimpleDataProvider::new()), vec![], vec![]);
        let handled = syscall.ecall(&mut machine).expect("load header syscall should be handled");

        assert!(handled);
        assert_eq!(machine.registers()[A0].to_u64(), ITEM_MISSING as u64);
    }
}
