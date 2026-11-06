// SPDX-License-Identifier: MIT
// Copyright (C) 2026 Myelin developers
//
// Pipe syscall

use super::{PIPE_SYSCALL_NUMBER, SPAWN_YIELD_CYCLES_BASE};
use crate::vm::scheduler::{Message, PipeRequest, VmId, VmRuntime};
use ckb_vm::{
    registers::{A0, A7},
    Error as VMError, Register, SupportMachine, Syscalls,
};
use std::sync::Arc;

/// Syscall: Pipe
///
/// Syscall number: 2604
///
/// Requires a scheduler runtime and yields requests to its multi-VM loop.
#[derive(Clone)]
pub struct Pipe {
    id: VmId,
    message_box: Arc<std::sync::Mutex<Vec<Message>>>,
}

impl Pipe {
    pub fn with_runtime(id: VmId, runtime: &VmRuntime) -> Self {
        Self { id, message_box: Arc::clone(&runtime.message_box) }
    }
}

impl<M: SupportMachine> Syscalls<M> for Pipe {
    fn initialize(&mut self, _machine: &mut M) -> Result<(), VMError> {
        Ok(())
    }

    fn ecall(&mut self, machine: &mut M) -> Result<bool, VMError> {
        if machine.registers()[A7].to_u64() != PIPE_SYSCALL_NUMBER {
            return Ok(false);
        }

        let fd1_addr = machine.registers()[A0].to_u64();
        let fd2_addr = fd1_addr.wrapping_add(8);
        machine.add_cycles_no_checking(SPAWN_YIELD_CYCLES_BASE)?;
        self.message_box
            .lock()
            .map_err(|e| VMError::Unexpected(e.to_string()))?
            .push(Message::Pipe(self.id, PipeRequest { fd1_addr, fd2_addr }));
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
    fn test_pipe_runtime_yields_and_enqueues_request() {
        let mut machine = ScriptVersion::V2.init_core_machine(10_000);
        machine.set_register(A0, 0x1000);
        machine.set_register(A7, PIPE_SYSCALL_NUMBER);

        let runtime = runtime();
        let mut syscall = Pipe::with_runtime(7, &runtime);

        let err = syscall.ecall(&mut machine).expect_err("pipe should yield to scheduler");

        assert!(matches!(err, VMError::Yield));
        assert_eq!(machine.cycles(), SPAWN_YIELD_CYCLES_BASE);
        assert_eq!(
            runtime.message_box.lock().unwrap().clone(),
            vec![Message::Pipe(7, PipeRequest { fd1_addr: 0x1000, fd2_addr: 0x1008 },)]
        );
    }
}
