// SPDX-License-Identifier: MIT
// Copyright (C) 2026 Spora developers
//
// Debug print syscall

use super::DEBUG_PRINT_SYSCALL_NUMBER;
use ckb_vm::{
    registers::{A0, A1, A7},
    Error as VMError, Memory, Register, SupportMachine, Syscalls,
};

/// Syscall: Debug Print
///
/// Syscall number: 2177
///
/// Prints debug message (only in debug builds)
pub struct Debugger {
    script_hash: [u8; 32],
}

impl Debugger {
    pub fn new(script_hash: [u8; 32]) -> Self {
        Self { script_hash }
    }
}

impl<M: SupportMachine> Syscalls<M> for Debugger {
    fn initialize(&mut self, _machine: &mut M) -> Result<(), VMError> {
        Ok(())
    }

    fn ecall(&mut self, machine: &mut M) -> Result<bool, VMError> {
        let syscall_number = machine.registers()[A7].to_u64();

        // DEBUG_PRINT = 2177
        if syscall_number != DEBUG_PRINT_SYSCALL_NUMBER {
            return Ok(false);
        }

        let addr = machine.registers()[A0].to_u64();
        let len = machine.registers()[A1].to_u64() as usize;

        // Read debug message from VM memory
        let message = machine.memory_mut().load_bytes(addr, len as u64)?;

        // Print debug message (only in debug mode)
        #[cfg(debug_assertions)]
        {
            let msg_str = String::from_utf8_lossy(message.as_ref());
            log::debug!("Script {:?} DEBUG: {}", hex::encode(&self.script_hash[..8]), msg_str);
        }

        // Return success
        machine.set_register(A0, M::REG::from_u8(0));

        Ok(true)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::vm::ScriptVersion;
    use ckb_vm::{registers::A7, CoreMachine, Memory, Register};

    const MESSAGE_ADDR: u64 = 0x1000;

    #[test]
    fn test_debugger_reads_message_without_mutating_memory() {
        let original = b"spora-debug";
        let mut machine = ScriptVersion::V2.init_core_machine(10_000);
        machine.memory_mut().store_bytes(MESSAGE_ADDR, original).unwrap();
        machine.set_register(A0, MESSAGE_ADDR);
        machine.set_register(A1, original.len() as u64);
        machine.set_register(A7, DEBUG_PRINT_SYSCALL_NUMBER);

        let mut syscall = Debugger::new([0xAB; 32]);
        let handled = syscall.ecall(&mut machine).expect("debugger syscall should succeed");

        assert!(handled);
        assert_eq!(machine.registers()[A0].to_u64(), 0);
        assert_eq!(machine.memory_mut().load_bytes(MESSAGE_ADDR, original.len() as u64).unwrap().as_ref(), original);
    }

    #[test]
    fn test_debugger_ignores_other_syscalls() {
        let mut machine = ScriptVersion::V2.init_core_machine(10_000);
        machine.set_register(A7, 9999);

        let mut syscall = Debugger::new([0xCD; 32]);
        let handled = syscall.ecall(&mut machine).expect("non-debug syscall should not fail");

        assert!(!handled);
    }
}
