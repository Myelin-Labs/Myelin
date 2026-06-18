// SPDX-License-Identifier: MIT
// Copyright (C) 2026 Spora developers
//
// Write syscall

use super::{INVALID_FD, SPAWN_YIELD_CYCLES_BASE, WRITE_SYSCALL_NUMBER};
use crate::vm::scheduler::{Fd, FdRequest, Message, VmId, VmRuntime};
use ckb_vm::{
    registers::{A0, A1, A2, A7},
    Error as VMError, Memory, Register, SupportMachine, Syscalls,
};
use std::sync::Arc;

/// Syscall: Write
///
/// Syscall number: 2605
///
/// When attached to a scheduler runtime this syscall yields a write request.
/// Without a runtime it preserves the placeholder behavior.
#[derive(Clone)]
pub struct Write {
    runtime: Option<WriteRuntime>,
}

#[derive(Clone)]
struct WriteRuntime {
    id: VmId,
    message_box: Arc<std::sync::Mutex<Vec<Message>>>,
}

impl Write {
    pub fn new() -> Self {
        Self { runtime: None }
    }

    pub fn with_runtime(id: VmId, runtime: &VmRuntime) -> Self {
        Self { runtime: Some(WriteRuntime { id, message_box: Arc::clone(&runtime.message_box) }) }
    }
}

impl<M: SupportMachine> Syscalls<M> for Write {
    fn initialize(&mut self, _machine: &mut M) -> Result<(), VMError> {
        Ok(())
    }

    fn ecall(&mut self, machine: &mut M) -> Result<bool, VMError> {
        if machine.registers()[A7].to_u64() != WRITE_SYSCALL_NUMBER {
            return Ok(false);
        }

        let Some(runtime) = &self.runtime else {
            machine.set_register(A0, M::REG::from_u8(INVALID_FD));
            return Ok(true);
        };

        let fd = Fd(machine.registers()[A0].to_u64());
        let buffer_addr = machine.registers()[A1].to_u64();
        let length_addr = machine.registers()[A2].to_u64();
        let length = machine.memory_mut().load64(&M::REG::from_u64(length_addr))?.to_u64();

        if !fd.is_write() {
            machine.set_register(A0, M::REG::from_u8(INVALID_FD));
            return Ok(true);
        }

        machine.add_cycles_no_checking(SPAWN_YIELD_CYCLES_BASE)?;
        runtime
            .message_box
            .lock()
            .map_err(|e| VMError::Unexpected(e.to_string()))?
            .push(Message::FdWrite(runtime.id, FdRequest { fd, length, buffer_addr, length_addr }));
        Err(VMError::Yield)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::vm::scheduler::VmRuntime;
    use crate::vm::ScriptVersion;
    use ckb_vm::{CoreMachine, Error as VMError, Memory, Register};
    use std::sync::{atomic::AtomicU64, Arc, Mutex};

    fn runtime() -> VmRuntime {
        VmRuntime::with_parts(Arc::new(AtomicU64::new(0)), Arc::new(Mutex::new(Vec::new())))
    }

    #[test]
    fn test_write_returns_invalid_fd() {
        let mut machine = ScriptVersion::V2.init_core_machine(10_000);
        machine.set_register(A7, WRITE_SYSCALL_NUMBER);

        let mut syscall = Write::new();
        let handled = syscall.ecall(&mut machine).expect("write syscall should succeed");

        assert!(handled);
        assert_eq!(machine.registers()[A0].to_u64(), INVALID_FD as u64);
    }

    #[test]
    fn test_write_ignores_other_syscalls() {
        let mut machine = ScriptVersion::V2.init_core_machine(10_000);
        machine.set_register(A7, 9999);

        let mut syscall = Write::new();
        let handled = syscall.ecall(&mut machine).expect("non-write syscall should not fail");

        assert!(!handled);
    }

    #[test]
    fn test_write_runtime_yields_and_enqueues_request() {
        let mut machine = ScriptVersion::V2.init_core_machine(10_000);
        machine.memory_mut().store64(&0x3000u64, &12u64).unwrap();
        machine.set_register(A0, 5);
        machine.set_register(A1, 0x2000);
        machine.set_register(A2, 0x3000);
        machine.set_register(A7, WRITE_SYSCALL_NUMBER);

        let runtime = runtime();
        let mut syscall = Write::with_runtime(7, &runtime);

        let err = syscall.ecall(&mut machine).expect_err("write should yield to scheduler");

        assert!(matches!(err, VMError::Yield));
        assert_eq!(machine.cycles(), SPAWN_YIELD_CYCLES_BASE);
        assert_eq!(
            runtime.message_box.lock().unwrap().clone(),
            vec![Message::FdWrite(7, FdRequest { fd: Fd(5), length: 12, buffer_addr: 0x2000, length_addr: 0x3000 },)]
        );
    }
}
