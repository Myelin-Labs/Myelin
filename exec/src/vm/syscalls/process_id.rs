// SPDX-License-Identifier: MIT
// Copyright (C) 2026 Myelin developers
//
// Process ID syscall

use super::PROCESS_ID_SYSCALL_NUMBER;
use ckb_vm::{
    registers::{A0, A7},
    Error as VMError, Register, SupportMachine, Syscalls,
};

/// Syscall: Process ID
///
/// Syscall number: 2603
///
/// Myelin currently executes one VM context per script verification path, so we
/// expose a fixed root process id (`0`) for scripts that
/// probe process identity.
#[derive(Debug, Clone, Copy, Default)]
pub struct ProcessId {
    id: u64,
}

impl ProcessId {
    pub fn new(id: u64) -> Self {
        Self { id }
    }
}

impl<M: SupportMachine> Syscalls<M> for ProcessId {
    fn initialize(&mut self, _machine: &mut M) -> Result<(), VMError> {
        Ok(())
    }

    fn ecall(&mut self, machine: &mut M) -> Result<bool, VMError> {
        if machine.registers()[A7].to_u64() != PROCESS_ID_SYSCALL_NUMBER {
            return Ok(false);
        }

        machine.set_register(A0, M::REG::from_u64(self.id));
        Ok(true)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::vm::ScriptVersion;
    use ckb_vm::{CoreMachine, Register};

    #[test]
    fn test_process_id_returns_configured_id() {
        let mut machine = ScriptVersion::V2.init_core_machine(10_000);
        machine.set_register(A7, PROCESS_ID_SYSCALL_NUMBER);

        let mut syscall = ProcessId::new(42);
        let handled = syscall.ecall(&mut machine).expect("process id syscall should succeed");

        assert!(handled);
        assert_eq!(machine.registers()[A0].to_u64(), 42);
    }

    #[test]
    fn test_process_id_ignores_other_syscalls() {
        let mut machine = ScriptVersion::V2.init_core_machine(10_000);
        machine.set_register(A7, 9999);

        let mut syscall = ProcessId::default();
        let handled = syscall.ecall(&mut machine).expect("non-process-id syscall should not fail");

        assert!(!handled);
    }
}
