// SPDX-License-Identifier: MIT
// Copyright (C) 2026 Myelin developers
//
// Read syscall

use super::{INVALID_FD, READ_SYSCALL_NUMBER, SPAWN_YIELD_CYCLES_BASE};
use crate::vm::scheduler::{Fd, FdRequest, Message, VmId, VmRuntime};
use ckb_vm::{
    registers::{A0, A1, A2, A7},
    Error as VMError, Memory, Register, SupportMachine, Syscalls,
};
use std::sync::Arc;

/// Syscall: Read
///
/// Syscall number: 2606
///
/// When attached to a scheduler runtime this syscall yields a read request.
/// Without a runtime it preserves the placeholder behavior.
#[derive(Clone)]
pub struct Read {
    runtime: Option<ReadRuntime>,
}

#[derive(Clone)]
struct ReadRuntime {
    id: VmId,
    message_box: Arc<std::sync::Mutex<Vec<Message>>>,
}

impl Default for Read {
    fn default() -> Self {
        Self::new()
    }
}

impl Read {
    pub fn new() -> Self {
        Self { runtime: None }
    }

    pub fn with_runtime(id: VmId, runtime: &VmRuntime) -> Self {
        Self { runtime: Some(ReadRuntime { id, message_box: Arc::clone(&runtime.message_box) }) }
    }
}

impl<M: SupportMachine> Syscalls<M> for Read {
    fn initialize(&mut self, _machine: &mut M) -> Result<(), VMError> {
        Ok(())
    }

    fn ecall(&mut self, machine: &mut M) -> Result<bool, VMError> {
        if machine.registers()[A7].to_u64() != READ_SYSCALL_NUMBER {
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

        if !fd.is_read() {
            machine.set_register(A0, M::REG::from_u8(INVALID_FD));
            return Ok(true);
        }

        machine.add_cycles_no_checking(SPAWN_YIELD_CYCLES_BASE)?;
        runtime
            .message_box
            .lock()
            .map_err(|e| VMError::Unexpected(e.to_string()))?
            .push(Message::FdRead(runtime.id, FdRequest { fd, length, buffer_addr, length_addr }));
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
    fn test_read_returns_invalid_fd() {
        let mut machine = ScriptVersion::V2.init_core_machine(10_000);
        machine.set_register(A7, READ_SYSCALL_NUMBER);

        let mut syscall = Read::new();
        let handled = syscall.ecall(&mut machine).expect("read syscall should succeed");

        assert!(handled);
        assert_eq!(machine.registers()[A0].to_u64(), INVALID_FD as u64);
    }

    #[test]
    fn test_read_ignores_other_syscalls() {
        let mut machine = ScriptVersion::V2.init_core_machine(10_000);
        machine.set_register(A7, 9999);

        let mut syscall = Read::new();
        let handled = syscall.ecall(&mut machine).expect("non-read syscall should not fail");

        assert!(!handled);
    }

    #[test]
    fn test_read_runtime_yields_and_enqueues_request() {
        let mut machine = ScriptVersion::V2.init_core_machine(10_000);
        machine.memory_mut().store64(&0x3000u64, &12u64).unwrap();
        machine.set_register(A0, 4);
        machine.set_register(A1, 0x2000);
        machine.set_register(A2, 0x3000);
        machine.set_register(A7, READ_SYSCALL_NUMBER);

        let runtime = runtime();
        let mut syscall = Read::with_runtime(7, &runtime);

        let err = syscall.ecall(&mut machine).expect_err("read should yield to scheduler");

        assert!(matches!(err, VMError::Yield));
        assert_eq!(machine.cycles(), SPAWN_YIELD_CYCLES_BASE);
        assert_eq!(
            runtime.message_box.lock().unwrap().clone(),
            vec![Message::FdRead(7, FdRequest { fd: Fd(4), length: 12, buffer_addr: 0x2000, length_addr: 0x3000 },)]
        );
    }
}
