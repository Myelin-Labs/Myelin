// SPDX-License-Identifier: MIT
// Copyright (C) 2026 Spora developers
//
// Load transaction hash syscall
// Reference: ckb/script/src/syscalls/load_tx.rs

use super::utils::store_data;
use super::{LOAD_TRANSACTION_SYSCALL_NUMBER, LOAD_TX_HASH_SYSCALL_NUMBER};
use crate::vm::transferred_byte_cycles;
use ckb_vm::{
    registers::{A0, A7},
    Error as VMError, Register, SupportMachine, Syscalls,
};

/// Syscall: Load Transaction Hash
///
/// Syscall number: 2061
///
/// Returns the transaction hash (32 bytes)
pub struct LoadTx {
    tx_hash: [u8; 32],
    tx_data: Vec<u8>,
}

impl LoadTx {
    pub fn new(tx_hash: [u8; 32], tx_data: Vec<u8>) -> Self {
        Self { tx_hash, tx_data }
    }
}

impl<M: SupportMachine> Syscalls<M> for LoadTx {
    fn initialize(&mut self, _machine: &mut M) -> Result<(), VMError> {
        Ok(())
    }

    fn ecall(&mut self, machine: &mut M) -> Result<bool, VMError> {
        let syscall_number = machine.registers()[A7].to_u64();

        let data = match syscall_number {
            LOAD_TX_HASH_SYSCALL_NUMBER => self.tx_hash.as_slice(),
            LOAD_TRANSACTION_SYSCALL_NUMBER => self.tx_data.as_slice(),
            _ => return Ok(false),
        };

        // Store tx payload using CKB-style store_data.
        // It reads A0, A1, A2 from registers internally.
        let result = store_data(machine, data)?;
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
    const SAMPLE_TX_DATA: &[u8] = &[0x10, 0x20, 0x30, 0x40, 0x50, 0x60];

    #[test]
    fn test_load_tx_creation() {
        let tx_hash = [0x42u8; 32];
        let syscall = LoadTx::new(tx_hash, SAMPLE_TX_DATA.to_vec());
        assert_eq!(syscall.tx_hash.len(), 32);
        assert_eq!(syscall.tx_data, SAMPLE_TX_DATA);
    }

    #[test]
    fn test_load_tx_supports_partial_reads() {
        let tx_hash = [0x42u8; 32];
        let mut machine = ScriptVersion::V2.init_core_machine(10_000);
        machine.memory_mut().store64(&SIZE_ADDR, &8u64).unwrap();
        machine.set_register(A0, BUFFER_ADDR);
        machine.set_register(A1, SIZE_ADDR);
        machine.set_register(A2, 24);
        machine.set_register(A7, LOAD_TX_HASH_SYSCALL_NUMBER);

        let mut syscall = LoadTx::new(tx_hash, SAMPLE_TX_DATA.to_vec());
        let handled = syscall.ecall(&mut machine).expect("load tx syscall should succeed");

        assert!(handled);
        assert_eq!(machine.registers()[A0].to_u64(), SUCCESS as u64);
        assert_eq!(machine.memory_mut().load64(&SIZE_ADDR).unwrap().to_u64(), 8);
        assert_eq!(machine.memory_mut().load_bytes(BUFFER_ADDR, 8).unwrap().as_ref(), &[0x42; 8]);
    }

    #[test]
    fn test_load_transaction_supports_partial_reads() {
        let mut machine = ScriptVersion::V2.init_core_machine(10_000);
        machine.memory_mut().store64(&SIZE_ADDR, &3u64).unwrap();
        machine.set_register(A0, BUFFER_ADDR);
        machine.set_register(A1, SIZE_ADDR);
        machine.set_register(A2, 2);
        machine.set_register(A7, LOAD_TRANSACTION_SYSCALL_NUMBER);

        let mut syscall = LoadTx::new([0x42u8; 32], SAMPLE_TX_DATA.to_vec());
        let handled = syscall.ecall(&mut machine).expect("load transaction syscall should succeed");

        assert!(handled);
        assert_eq!(machine.registers()[A0].to_u64(), SUCCESS as u64);
        assert_eq!(machine.memory_mut().load64(&SIZE_ADDR).unwrap().to_u64(), 4);
        assert_eq!(machine.memory_mut().load_bytes(BUFFER_ADDR, 3).unwrap().as_ref(), &[0x30, 0x40, 0x50]);
    }

    #[test]
    fn test_load_tx_ignores_other_syscalls() {
        let mut machine = ScriptVersion::V2.init_core_machine(10_000);
        machine.set_register(A7, 9999);

        let mut syscall = LoadTx::new([0x42u8; 32], SAMPLE_TX_DATA.to_vec());
        let handled = syscall.ecall(&mut machine).expect("non-load-tx syscall should not fail");

        assert!(!handled);
    }

    #[test]
    fn test_load_tx_clamps_large_offset() {
        let tx_hash = [0x42u8; 32];
        let mut machine = ScriptVersion::V2.init_core_machine(10_000);
        machine.memory_mut().store64(&SIZE_ADDR, &8u64).unwrap();
        machine.set_register(A0, BUFFER_ADDR);
        machine.set_register(A1, SIZE_ADDR);
        machine.set_register(A2, 33);
        machine.set_register(A7, LOAD_TX_HASH_SYSCALL_NUMBER);

        let mut syscall = LoadTx::new(tx_hash, SAMPLE_TX_DATA.to_vec());
        let handled = syscall.ecall(&mut machine).expect("load tx syscall should succeed");

        assert!(handled);
        assert_eq!(machine.registers()[A0].to_u64(), SUCCESS as u64);
        assert_eq!(machine.memory_mut().load64(&SIZE_ADDR).unwrap().to_u64(), 0);
    }

    #[test]
    fn test_load_tx_charges_cycles_for_written_bytes() {
        let tx_hash = [0x42u8; 32];
        let mut machine = ScriptVersion::V2.init_core_machine(10_000);
        machine.memory_mut().store64(&SIZE_ADDR, &8u64).unwrap();
        machine.set_register(A0, BUFFER_ADDR);
        machine.set_register(A1, SIZE_ADDR);
        machine.set_register(A2, 24);
        machine.set_register(A7, LOAD_TX_HASH_SYSCALL_NUMBER);

        let mut syscall = LoadTx::new(tx_hash, SAMPLE_TX_DATA.to_vec());
        let handled = syscall.ecall(&mut machine).expect("load tx syscall should succeed");

        assert!(handled);
        assert_eq!(machine.cycles(), crate::vm::transferred_byte_cycles(8));
    }
}
