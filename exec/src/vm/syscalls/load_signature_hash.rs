// SPDX-License-Identifier: MIT
// Copyright (C) 2026 Myelin developers
//
// Load canonical per-input signature hash syscall

use super::utils::{store_data, INDEX_OUT_OF_BOUND, ITEM_MISSING};
use super::{Source, LOAD_ECDSA_SIGNATURE_HASH_SYSCALL_NUMBER, LOAD_SCHNORR_SIGNATURE_HASH_SYSCALL_NUMBER};
use crate::celltx::{
    sighash::{
        calc_standard_ecdsa_signature_hash, calc_standard_signature_hash, StandardSigHashReusedValues, StandardSigHashType,
        StandardSigningInput,
    },
    CellTx,
};
use crate::vm::{transferred_byte_cycles, ResolvedCell};
use ckb_vm::{
    registers::{A0, A3, A4, A5, A7},
    Error as VMError, Register, SupportMachine, Syscalls,
};
use myelin_hashes::Hash;
use std::{cell::Cell, sync::Arc};

pub const LOAD_SIGNATURE_HASH_BASE_CYCLES: u64 = 25_000;
const DATA_HASH_DOMAIN: &[u8] = b"myelin-cell/data";

#[derive(Clone, Copy)]
struct VmSigHashType(u8);

impl VmSigHashType {
    fn from_u8(value: u8) -> Option<Self> {
        match value {
            0b0000_0001 | 0b0000_0010 | 0b0000_0100 | 0b1000_0001 | 0b1000_0010 | 0b1000_0100 => Some(Self(value)),
            _ => None,
        }
    }
}

impl StandardSigHashType for VmSigHashType {
    fn is_sighash_none(self) -> bool {
        self.0 & 0b0000_0111 == 0b0000_0010
    }

    fn is_sighash_single(self) -> bool {
        self.0 & 0b0000_0111 == 0b0000_0100
    }

    fn is_sighash_anyone_can_pay(self) -> bool {
        self.0 & 0b1000_0000 == 0b1000_0000
    }

    fn to_u8(self) -> u8 {
        self.0
    }
}

#[derive(Default)]
struct CachedSigHashReusedValues {
    previous_outputs_hash: Cell<Option<Hash>>,
    sequences_hash: Cell<Option<Hash>>,
    sig_op_counts_hash: Cell<Option<Hash>>,
    outputs_hash: Cell<Option<Hash>>,
    payload_hash: Cell<Option<Hash>>,
}

impl StandardSigHashReusedValues for CachedSigHashReusedValues {
    fn previous_outputs_hash(&self, set: impl Fn() -> Hash) -> Hash {
        self.previous_outputs_hash.get().unwrap_or_else(|| {
            let hash = set();
            self.previous_outputs_hash.set(Some(hash));
            hash
        })
    }

    fn sequences_hash(&self, set: impl Fn() -> Hash) -> Hash {
        self.sequences_hash.get().unwrap_or_else(|| {
            let hash = set();
            self.sequences_hash.set(Some(hash));
            hash
        })
    }

    fn sig_op_counts_hash(&self, set: impl Fn() -> Hash) -> Hash {
        self.sig_op_counts_hash.get().unwrap_or_else(|| {
            let hash = set();
            self.sig_op_counts_hash.set(Some(hash));
            hash
        })
    }

    fn outputs_hash(&self, set: impl Fn() -> Hash) -> Hash {
        self.outputs_hash.get().unwrap_or_else(|| {
            let hash = set();
            self.outputs_hash.set(Some(hash));
            hash
        })
    }

    fn payload_hash(&self, set: impl Fn() -> Hash) -> Hash {
        self.payload_hash.get().unwrap_or_else(|| {
            let hash = set();
            self.payload_hash.set(Some(hash));
            hash
        })
    }
}

fn compute_data_hash(data: &[u8]) -> [u8; 32] {
    if data.is_empty() {
        [0u8; 32]
    } else {
        let mut hasher = blake3::Hasher::new();
        hasher.update(DATA_HASH_DOMAIN);
        hasher.update(data);
        *hasher.finalize().as_bytes()
    }
}

pub(crate) fn standard_signing_input_from_resolved_cell(resolved: &ResolvedCell) -> StandardSigningInput {
    let data = resolved.data.as_deref().unwrap_or_default();
    StandardSigningInput {
        lock_hash: resolved.cell_output.lock.hash(),
        type_hash: resolved.cell_output.type_.as_ref().map(|script| script.hash()),
        data_hash: compute_data_hash(data),
        data_bytes: data.len() as u64,
        capacity: resolved.cell_output.capacity,
    }
}

/// Syscall: load canonical per-input CellTx signature hash.
///
/// Supported numbers:
/// - 3003: Schnorr standard-lock sighash
/// - 3004: ECDSA standard-lock sighash
///
/// Args:
/// - A0: destination buffer
/// - A1: destination length pointer
/// - A2: offset into the 32-byte hash
/// - A3: input index (interpreted according to source)
/// - A4: source (`Input` or `GroupInput`)
/// - A5: raw sighash type byte
pub struct LoadSignatureHash {
    tx: Arc<CellTx>,
    signing_inputs: Vec<StandardSigningInput>,
    group_input_indices: Vec<usize>,
    reused_values: CachedSigHashReusedValues,
}

impl LoadSignatureHash {
    pub fn new(tx: Arc<CellTx>, signing_inputs: Vec<StandardSigningInput>, group_input_indices: Vec<usize>) -> Self {
        Self { tx, signing_inputs, group_input_indices, reused_values: CachedSigHashReusedValues::default() }
    }

    fn resolve_input_index(&self, source: u64, index: usize) -> Option<usize> {
        match Source::parse(source)? {
            Source::Input => self.tx.inputs.get(index).map(|_| index),
            Source::GroupInput => self.group_input_indices.get(index).copied(),
            _ => None,
        }
    }
}

impl<M: SupportMachine> Syscalls<M> for LoadSignatureHash {
    fn initialize(&mut self, _machine: &mut M) -> Result<(), VMError> {
        Ok(())
    }

    fn ecall(&mut self, machine: &mut M) -> Result<bool, VMError> {
        let syscall_number = machine.registers()[A7].to_u64();
        if syscall_number != LOAD_SCHNORR_SIGNATURE_HASH_SYSCALL_NUMBER && syscall_number != LOAD_ECDSA_SIGNATURE_HASH_SYSCALL_NUMBER {
            return Ok(false);
        }

        let requested_index = machine.registers()[A3].to_u64() as usize;
        let source = machine.registers()[A4].to_u64();
        let raw_hash_type = machine.registers()[A5].to_u64() as u8;

        let Some(input_index) = self.resolve_input_index(source, requested_index) else {
            machine.set_register(A0, M::REG::from_u8(INDEX_OUT_OF_BOUND));
            return Ok(true);
        };

        let Some(hash_type) = VmSigHashType::from_u8(raw_hash_type) else {
            machine.set_register(A0, M::REG::from_u8(ITEM_MISSING));
            return Ok(true);
        };

        let Some(signing_input) = self.signing_inputs.get(input_index) else {
            machine.set_register(A0, M::REG::from_u8(ITEM_MISSING));
            return Ok(true);
        };

        let signature_hash = if syscall_number == LOAD_ECDSA_SIGNATURE_HASH_SYSCALL_NUMBER {
            calc_standard_ecdsa_signature_hash(&self.tx, input_index, hash_type, signing_input, &self.reused_values)
        } else {
            calc_standard_signature_hash(&self.tx, input_index, hash_type, signing_input, &self.reused_values)
        };

        let result = store_data(machine, &signature_hash.as_bytes()[..])?;
        machine.add_cycles_no_checking(LOAD_SIGNATURE_HASH_BASE_CYCLES + transferred_byte_cycles(result.written_size))?;
        machine.set_register(A0, M::REG::from_u8(result.return_code));

        Ok(true)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::celltx::{CellInput, CellOutput, OutPoint, Script};
    use crate::vm::syscalls::SUCCESS;
    use crate::vm::ScriptVersion;
    use ckb_vm::{
        registers::{A1, A2},
        CoreMachine, Memory, Register,
    };

    const BUFFER_ADDR: u64 = 0x1000;
    const SIZE_ADDR: u64 = 0x2000;

    fn sample_tx() -> Arc<CellTx> {
        Arc::new(
            CellTx::new(
                vec![CellInput::new(OutPoint::new([0x11; 32], 0), 7), CellInput::new(OutPoint::new([0x12; 32], 1), 9)],
                vec![],
                vec![CellOutput { capacity: 1_000, lock: Script::new([0x21; 32], 0, vec![0xAA]), type_: None }],
                vec![vec![0xBB; 4]],
                vec![vec![0x01], vec![0x02]],
            )
            .unwrap(),
        )
    }

    fn sample_resolved_inputs() -> Vec<StandardSigningInput> {
        vec![
            standard_signing_input_from_resolved_cell(&ResolvedCell {
                cell_output: CellOutput { capacity: 5_000, lock: Script::new([0x31; 32], 0, vec![0xA1]), type_: None },
                data: Some(vec![0xC1, 0xC2]),
            }),
            standard_signing_input_from_resolved_cell(&ResolvedCell {
                cell_output: CellOutput {
                    capacity: 6_000,
                    lock: Script::new([0x32; 32], 0, vec![0xA2]),
                    type_: Some(Script::new([0x41; 32], 0, vec![0xD1])),
                },
                data: Some(vec![0xE1, 0xE2, 0xE3]),
            }),
        ]
    }

    #[test]
    fn test_load_schnorr_signature_hash_for_input_source() {
        let tx = sample_tx();
        let signing_inputs = sample_resolved_inputs();
        let expected = calc_standard_signature_hash(
            &tx,
            0,
            VmSigHashType(0b0000_0001),
            &signing_inputs[0],
            &CachedSigHashReusedValues::default(),
        );

        let mut machine = ScriptVersion::V2.init_core_machine(200_000);
        machine.memory_mut().store64(&SIZE_ADDR, &32u64).unwrap();
        machine.set_register(A0, BUFFER_ADDR);
        machine.set_register(A1, SIZE_ADDR);
        machine.set_register(A2, 0);
        machine.set_register(A3, 0);
        machine.set_register(A4, Source::Input as u64);
        machine.set_register(A5, 0b0000_0001);
        machine.set_register(A7, LOAD_SCHNORR_SIGNATURE_HASH_SYSCALL_NUMBER);

        let mut syscall = LoadSignatureHash::new(tx, signing_inputs, vec![0]);
        let handled = syscall.ecall(&mut machine).expect("load schnorr signature hash syscall should succeed");

        assert!(handled);
        assert_eq!(machine.registers()[A0].to_u64(), SUCCESS as u64);
        assert_eq!(machine.memory_mut().load64(&SIZE_ADDR).unwrap().to_u64(), 32);
        assert_eq!(machine.memory_mut().load_bytes(BUFFER_ADDR, 32).unwrap().as_ref(), expected.as_bytes());
    }

    #[test]
    fn test_load_ecdsa_signature_hash_for_group_input_source() {
        let tx = sample_tx();
        let signing_inputs = sample_resolved_inputs();
        let expected = calc_standard_ecdsa_signature_hash(
            &tx,
            1,
            VmSigHashType(0b1000_0001),
            &signing_inputs[1],
            &CachedSigHashReusedValues::default(),
        );

        let mut machine = ScriptVersion::V2.init_core_machine(200_000);
        machine.memory_mut().store64(&SIZE_ADDR, &16u64).unwrap();
        machine.set_register(A0, BUFFER_ADDR);
        machine.set_register(A1, SIZE_ADDR);
        machine.set_register(A2, 8);
        machine.set_register(A3, 0);
        machine.set_register(A4, Source::GroupInput as u64);
        machine.set_register(A5, 0b1000_0001);
        machine.set_register(A7, LOAD_ECDSA_SIGNATURE_HASH_SYSCALL_NUMBER);

        let mut syscall = LoadSignatureHash::new(tx, signing_inputs, vec![1]);
        let handled = syscall.ecall(&mut machine).expect("load ecdsa signature hash syscall should succeed");

        assert!(handled);
        assert_eq!(machine.registers()[A0].to_u64(), SUCCESS as u64);
        assert_eq!(machine.memory_mut().load64(&SIZE_ADDR).unwrap().to_u64(), 24);
        assert_eq!(machine.memory_mut().load_bytes(BUFFER_ADDR, 16).unwrap().as_ref(), &expected.as_bytes()[8..24]);
    }

    #[test]
    fn test_load_signature_hash_rejects_invalid_hash_type() {
        let tx = sample_tx();
        let signing_inputs = sample_resolved_inputs();
        let mut machine = ScriptVersion::V2.init_core_machine(200_000);
        machine.memory_mut().store64(&SIZE_ADDR, &32u64).unwrap();
        machine.set_register(A0, BUFFER_ADDR);
        machine.set_register(A1, SIZE_ADDR);
        machine.set_register(A2, 0);
        machine.set_register(A3, 0);
        machine.set_register(A4, Source::Input as u64);
        machine.set_register(A5, 0xFF);
        machine.set_register(A7, LOAD_SCHNORR_SIGNATURE_HASH_SYSCALL_NUMBER);

        let mut syscall = LoadSignatureHash::new(tx, signing_inputs, vec![0]);
        let handled = syscall.ecall(&mut machine).expect("invalid sighash type should not trap");

        assert!(handled);
        assert_eq!(machine.registers()[A0].to_u64(), ITEM_MISSING as u64);
    }

    #[test]
    fn test_load_signature_hash_clamps_large_offset() {
        let tx = sample_tx();
        let signing_inputs = sample_resolved_inputs();
        let mut machine = ScriptVersion::V2.init_core_machine(200_000);
        machine.memory_mut().store64(&SIZE_ADDR, &8u64).unwrap();
        machine.set_register(A0, BUFFER_ADDR);
        machine.set_register(A1, SIZE_ADDR);
        machine.set_register(A2, 64);
        machine.set_register(A3, 0);
        machine.set_register(A4, Source::Input as u64);
        machine.set_register(A5, 0b0000_0001);
        machine.set_register(A7, LOAD_SCHNORR_SIGNATURE_HASH_SYSCALL_NUMBER);

        let mut syscall = LoadSignatureHash::new(tx, signing_inputs, vec![0]);
        let handled = syscall.ecall(&mut machine).expect("large offset should not trap");

        assert!(handled);
        assert_eq!(machine.registers()[A0].to_u64(), SUCCESS as u64);
        assert_eq!(machine.memory_mut().load64(&SIZE_ADDR).unwrap().to_u64(), 0);
    }
}
