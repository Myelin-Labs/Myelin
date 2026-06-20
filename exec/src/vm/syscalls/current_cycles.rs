// SPDX-License-Identifier: MIT
// Copyright (C) 2026 Myelin developers
//
// Current cycles syscall

use super::CURRENT_CYCLES_SYSCALL_NUMBER;
use ckb_vm::{
    registers::{A0, A7},
    Error as VMError, Register, SupportMachine, Syscalls,
};
use std::sync::{
    atomic::{AtomicU64, Ordering},
    Arc,
};

/// Syscall: Current Cycles
///
/// Syscall number: 2042
///
/// Returns the current cycle count in A0 register
pub struct CurrentCycles {
    base_cycles: Arc<AtomicU64>,
}

impl Default for CurrentCycles {
    fn default() -> Self {
        Self::new()
    }
}

impl CurrentCycles {
    pub fn new() -> Self {
        Self { base_cycles: Arc::new(AtomicU64::new(0)) }
    }

    pub fn with_base_cycles(base_cycles: Arc<AtomicU64>) -> Self {
        Self { base_cycles }
    }
}

impl<M: SupportMachine> Syscalls<M> for CurrentCycles {
    fn initialize(&mut self, _machine: &mut M) -> Result<(), VMError> {
        Ok(())
    }

    fn ecall(&mut self, machine: &mut M) -> Result<bool, VMError> {
        let syscall_number = machine.registers()[A7].to_u64();

        // CURRENT_CYCLES = 2042
        if syscall_number != CURRENT_CYCLES_SYSCALL_NUMBER {
            return Ok(false);
        }

        let cycles = self.base_cycles.load(Ordering::Acquire).checked_add(machine.cycles()).ok_or(VMError::CyclesOverflow)?;

        // Return cycles in A0
        machine.set_register(A0, M::REG::from_u64(cycles));

        Ok(true)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::vm::ScriptVersion;
    use ckb_vm::{
        registers::{A0, A7},
        CoreMachine, Register, SupportMachine, Syscalls,
    };
    use std::sync::{atomic::AtomicU64, Arc};

    #[test]
    fn test_current_cycles_returns_machine_cycles() {
        let mut machine = ScriptVersion::V2.init_core_machine(10_000);
        machine.set_cycles(4242);
        machine.set_register(A7, CURRENT_CYCLES_SYSCALL_NUMBER);

        let mut syscall = CurrentCycles::new();
        let handled = syscall.ecall(&mut machine).expect("current cycles syscall should succeed");

        assert!(handled);
        assert_eq!(machine.registers()[A0].to_u64(), 4242);
    }

    #[test]
    fn test_current_cycles_ignores_other_syscalls() {
        let mut machine = ScriptVersion::V2.init_core_machine(10_000);
        machine.set_cycles(99);
        machine.set_register(A7, 1);

        let mut syscall = CurrentCycles::new();
        let handled = syscall.ecall(&mut machine).expect("non-matching syscall should not error");

        assert!(!handled);
    }

    #[test]
    fn test_current_cycles_includes_base_cycles() {
        let mut machine = ScriptVersion::V2.init_core_machine(10_000);
        machine.set_cycles(42);
        machine.set_register(A7, CURRENT_CYCLES_SYSCALL_NUMBER);

        let mut syscall = CurrentCycles::with_base_cycles(Arc::new(AtomicU64::new(1_000)));
        let handled = syscall.ecall(&mut machine).expect("current cycles syscall should succeed");

        assert!(handled);
        assert_eq!(machine.registers()[A0].to_u64(), 1_042);
    }
}
