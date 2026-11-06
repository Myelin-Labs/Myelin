// SPDX-License-Identifier: MIT
// Copyright (C) 2026 Myelin developers
//
// Inherited FD syscall

use super::{INHERITED_FD_SYSCALL_NUMBER, SPAWN_YIELD_CYCLES_BASE};
use crate::vm::scheduler::{Fd, FdRequest, Message, VmId, VmRuntime};
use ckb_vm::{
    registers::{A0, A1, A7},
    Error as VMError, Register, SupportMachine, Syscalls,
};
use std::sync::Arc;

/// Syscall: Inherited FD
///
/// Syscall number: 2607
///
/// Requires a scheduler runtime and yields requests to its multi-VM loop.
#[derive(Clone)]
pub struct InheritedFd {
    id: VmId,
    message_box: Arc<std::sync::Mutex<Vec<Message>>>,
}

impl InheritedFd {
    pub fn with_runtime(id: VmId, runtime: &VmRuntime) -> Self {
        Self { id, message_box: Arc::clone(&runtime.message_box) }
    }
}

impl<M: SupportMachine> Syscalls<M> for InheritedFd {
    fn initialize(&mut self, _machine: &mut M) -> Result<(), VMError> {
        Ok(())
    }

    fn ecall(&mut self, machine: &mut M) -> Result<bool, VMError> {
        if machine.registers()[A7].to_u64() != INHERITED_FD_SYSCALL_NUMBER {
            return Ok(false);
        }

        let buffer_addr = machine.registers()[A0].to_u64();
        let length_addr = machine.registers()[A1].to_u64();
        machine.add_cycles_no_checking(SPAWN_YIELD_CYCLES_BASE)?;
        self.message_box
            .lock()
            .map_err(|e| VMError::Unexpected(e.to_string()))?
            .push(Message::InheritedFileDescriptor(self.id, FdRequest { fd: Fd(0), length: 0, buffer_addr, length_addr }));
        Err(VMError::Yield)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::vm::scheduler::VmRuntime;
    use crate::vm::ScriptVersion;
    use ckb_vm::{CoreMachine, Error as VMError};
    use std::sync::{atomic::AtomicU64, Arc, Mutex};

    fn runtime() -> VmRuntime {
        VmRuntime::with_parts(Arc::new(AtomicU64::new(0)), Arc::new(Mutex::new(Vec::new())))
    }

    #[test]
    fn test_inherited_fd_runtime_yields_and_enqueues_request() {
        let mut machine = ScriptVersion::V2.init_core_machine(10_000);
        machine.set_register(A0, 0x2000);
        machine.set_register(A1, 0x3000);
        machine.set_register(A7, INHERITED_FD_SYSCALL_NUMBER);

        let runtime = runtime();
        let mut syscall = InheritedFd::with_runtime(7, &runtime);

        let err = syscall.ecall(&mut machine).expect_err("inherited-fd should yield to scheduler");

        assert!(matches!(err, VMError::Yield));
        assert_eq!(machine.cycles(), SPAWN_YIELD_CYCLES_BASE);
        assert_eq!(
            runtime.message_box.lock().unwrap().clone(),
            vec![Message::InheritedFileDescriptor(7, FdRequest { fd: Fd(0), length: 0, buffer_addr: 0x2000, length_addr: 0x3000 },)]
        );
    }
}
