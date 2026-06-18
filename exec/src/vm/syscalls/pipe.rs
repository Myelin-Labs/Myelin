// SPDX-License-Identifier: MIT
// Copyright (C) 2026 Myelin developers
//
// Pipe syscall

use super::{MAX_FDS_CREATED, PIPE_SYSCALL_NUMBER, SPAWN_YIELD_CYCLES_BASE};
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
/// When attached to a scheduler runtime this syscall yields a pipe request.
/// Without a runtime it preserves the placeholder behavior.
#[derive(Clone)]
pub struct Pipe {
    runtime: Option<PipeRuntime>,
}

#[derive(Clone)]
struct PipeRuntime {
    id: VmId,
    message_box: Arc<std::sync::Mutex<Vec<Message>>>,
}

impl Pipe {
    pub fn new() -> Self {
        Self { runtime: None }
    }

    pub fn with_runtime(id: VmId, runtime: &VmRuntime) -> Self {
        Self { runtime: Some(PipeRuntime { id, message_box: Arc::clone(&runtime.message_box) }) }
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

        let Some(runtime) = &self.runtime else {
            machine.add_cycles_no_checking(SPAWN_YIELD_CYCLES_BASE)?;
            machine.set_register(A0, M::REG::from_u8(MAX_FDS_CREATED));
            return Ok(true);
        };

        let fd1_addr = machine.registers()[A0].to_u64();
        let fd2_addr = fd1_addr.wrapping_add(8);
        machine.add_cycles_no_checking(SPAWN_YIELD_CYCLES_BASE)?;
        runtime
            .message_box
            .lock()
            .map_err(|e| VMError::Unexpected(e.to_string()))?
            .push(Message::Pipe(runtime.id, PipeRequest { fd1_addr, fd2_addr }));
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
    fn test_pipe_returns_max_fds_created() {
        let mut machine = ScriptVersion::V2.init_core_machine(10_000);
        machine.set_register(A7, PIPE_SYSCALL_NUMBER);

        let mut syscall = Pipe::new();
        let handled = syscall.ecall(&mut machine).expect("pipe syscall should succeed");

        assert!(handled);
        assert_eq!(machine.registers()[A0].to_u64(), MAX_FDS_CREATED as u64);
        assert_eq!(machine.cycles(), SPAWN_YIELD_CYCLES_BASE);
    }

    #[test]
    fn test_pipe_ignores_other_syscalls() {
        let mut machine = ScriptVersion::V2.init_core_machine(10_000);
        machine.set_register(A7, 9999);

        let mut syscall = Pipe::new();
        let handled = syscall.ecall(&mut machine).expect("non-pipe syscall should not fail");

        assert!(!handled);
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
