// SPDX-License-Identifier: MIT
// Copyright (C) 2026 Spora developers
//
// VM version syscall
// Reference: ckb/script/src/syscalls/vm_version.rs

use super::VM_VERSION_SYSCALL_NUMBER;
use ckb_vm::{
    registers::{A0, A7},
    Error as VMError, Register, SupportMachine, Syscalls,
};

#[derive(Debug, Default)]
pub struct VMVersion {}

impl VMVersion {
    pub fn new() -> Self {
        Self {}
    }
}

impl<M: SupportMachine> Syscalls<M> for VMVersion {
    fn initialize(&mut self, _machine: &mut M) -> Result<(), VMError> {
        Ok(())
    }

    fn ecall(&mut self, machine: &mut M) -> Result<bool, VMError> {
        if machine.registers()[A7].to_u64() != VM_VERSION_SYSCALL_NUMBER {
            return Ok(false);
        }

        machine.set_register(A0, M::REG::from_u32(machine.version()));
        Ok(true)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::vm::ScriptVersion;
    use ckb_vm::CoreMachine;

    #[test]
    fn test_vm_version_returns_machine_version() {
        let mut machine = ScriptVersion::V2.init_core_machine(10_000);
        machine.set_register(A7, VM_VERSION_SYSCALL_NUMBER);

        let mut syscall = VMVersion::new();
        let handled = syscall.ecall(&mut machine).expect("vm version syscall should succeed");

        assert!(handled);
        assert_eq!(machine.registers()[A0].to_u64(), machine.version() as u64);
    }

    #[test]
    fn test_vm_version_ignores_other_syscalls() {
        let mut machine = ScriptVersion::V2.init_core_machine(10_000);
        machine.set_register(A7, 9999);

        let mut syscall = VMVersion::new();
        let handled = syscall.ecall(&mut machine).expect("non-vm-version syscall should not fail");

        assert!(!handled);
    }
}
