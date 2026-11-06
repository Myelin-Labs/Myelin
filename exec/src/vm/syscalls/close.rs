// SPDX-License-Identifier: MIT
// Copyright (C) 2026 Myelin developers
//
// Close syscall

use super::{CLOSE_SYSCALL_NUMBER, SPAWN_YIELD_CYCLES_BASE};
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
/// Requires a scheduler runtime and yields requests to its multi-VM loop.
#[derive(Clone)]
pub struct Close {
    id: VmId,
    message_box: Arc<std::sync::Mutex<Vec<Message>>>,
}

impl Close {
    pub fn with_runtime(id: VmId, runtime: &VmRuntime) -> Self {
        Self { id, message_box: Arc::clone(&runtime.message_box) }
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

        let fd = Fd(machine.registers()[A0].to_u64());
        machine.add_cycles_no_checking(SPAWN_YIELD_CYCLES_BASE)?;
        self.message_box.lock().map_err(|e| VMError::Unexpected(e.to_string()))?.push(Message::Close(self.id, fd));
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
