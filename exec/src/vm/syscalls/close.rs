// SPDX-License-Identifier: MIT
// Copyright (C) 2026 Myelin developers
//
// Close syscall

use super::{CLOSE_SYSCALL_NUMBER, INVALID_FD, SPAWN_YIELD_CYCLES_BASE};
use crate::vm::scheduler::{Fd, Message, VmId, VmRuntime};
use ckb_vm::{
    registers::{A0, A7},
    Error as VMError, Register, SupportMachine, Syscalls,
};
use std::sync::Arc;

/// Syscall: Close
///
/// Syscall number: 2608
///
/// When attached to a scheduler runtime this syscall yields a close request.
/// Without a runtime it preserves the placeholder behavior.
#[derive(Clone)]
pub struct Close {
    runtime: Option<CloseRuntime>,
}

#[derive(Clone)]
struct CloseRuntime {
    id: VmId,
    message_box: Arc<std::sync::Mutex<Vec<Message>>>,
}

impl Close {
    pub fn new() -> Self {
        Self { runtime: None }
    }

    pub fn with_runtime(id: VmId, runtime: &VmRuntime) -> Self {
        Self { runtime: Some(CloseRuntime { id, message_box: Arc::clone(&runtime.message_box) }) }
    }
}

impl<M: SupportMachine> Syscalls<M> for Close {
    fn initialize(&mut self, _machine: &mut M) -> Result<(), VMError> {
        Ok(())
    }

    fn ecall(&mut self, machine: &mut M) -> Result<bool, VMError> {
        if machine.registers()[A7].to_u64() != CLOSE_SYSCALL_NUMBER {
            return Ok(false);
        }

        let Some(runtime) = &self.runtime else {
            machine.add_cycles_no_checking(SPAWN_YIELD_CYCLES_BASE)?;
            machine.set_register(A0, M::REG::from_u8(INVALID_FD));
            return Ok(true);
        };

        let fd = Fd(machine.registers()[A0].to_u64());
        machine.add_cycles_no_checking(SPAWN_YIELD_CYCLES_BASE)?;
        runtime.message_box.lock().map_err(|e| VMError::Unexpected(e.to_string()))?.push(Message::Close(runtime.id, fd));
        Err(VMError::Yield)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::vm::scheduler::VmRuntime;
    use crate::vm::ScriptVersion;
    use ckb_vm::{CoreMachine, Error as VMError, Register};
    use std::sync::{atomic::AtomicU64, Arc, Mutex};

    fn runtime() -> VmRuntime {
        VmRuntime::with_parts(Arc::new(AtomicU64::new(0)), Arc::new(Mutex::new(Vec::new())))
    }

    #[test]
    fn test_close_returns_invalid_fd() {
        let mut machine = ScriptVersion::V2.init_core_machine(10_000);
        machine.set_register(A7, CLOSE_SYSCALL_NUMBER);

        let mut syscall = Close::new();
        let handled = syscall.ecall(&mut machine).expect("close syscall should succeed");

        assert!(handled);
        assert_eq!(machine.registers()[A0].to_u64(), INVALID_FD as u64);
        assert_eq!(machine.cycles(), SPAWN_YIELD_CYCLES_BASE);
    }

    #[test]
    fn test_close_ignores_other_syscalls() {
        let mut machine = ScriptVersion::V2.init_core_machine(10_000);
        machine.set_register(A7, 9999);

        let mut syscall = Close::new();
        let handled = syscall.ecall(&mut machine).expect("non-close syscall should not fail");

        assert!(!handled);
    }

    #[test]
    fn test_close_runtime_yields_and_enqueues_request() {
        let mut machine = ScriptVersion::V2.init_core_machine(10_000);
        machine.set_register(A0, 8);
        machine.set_register(A7, CLOSE_SYSCALL_NUMBER);

        let runtime = runtime();
        let mut syscall = Close::with_runtime(7, &runtime);

        let err = syscall.ecall(&mut machine).expect_err("close should yield to scheduler");

        assert!(matches!(err, VMError::Yield));
        assert_eq!(machine.cycles(), SPAWN_YIELD_CYCLES_BASE);
        assert_eq!(runtime.message_box.lock().unwrap().clone(), vec![Message::Close(7, Fd(8))]);
    }
}
