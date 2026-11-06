// SPDX-License-Identifier: MIT
// Copyright (C) 2026 Myelin developers
//
// Wait syscall

use super::{SPAWN_YIELD_CYCLES_BASE, WAIT_SYSCALL_NUMBER};
use crate::vm::scheduler::{Message, VmId, VmRuntime, WaitRequest};
use ckb_vm::{
    registers::{A0, A1, A7},
    Error as VMError, Register, SupportMachine, Syscalls,
};
use std::sync::Arc;

/// Syscall: Wait
///
/// Syscall number: 2602
///
/// Requires a scheduler runtime and yields requests to its multi-VM loop.
#[derive(Clone)]
pub struct Wait {
    id: VmId,
    message_box: Arc<std::sync::Mutex<Vec<Message>>>,
}

impl Wait {
    pub fn with_runtime(id: VmId, runtime: &VmRuntime) -> Self {
        Self { id, message_box: Arc::clone(&runtime.message_box) }
    }
}

impl<M: SupportMachine> Syscalls<M> for Wait {
    fn initialize(&mut self, _machine: &mut M) -> Result<(), VMError> {
        Ok(())
    }

    fn ecall(&mut self, machine: &mut M) -> Result<bool, VMError> {
        if machine.registers()[A7].to_u64() != WAIT_SYSCALL_NUMBER {
            return Ok(false);
        }

        let target_id = machine.registers()[A0].to_u64();
        let exit_code_addr = machine.registers()[A1].to_u64();
        machine.add_cycles_no_checking(SPAWN_YIELD_CYCLES_BASE)?;
        self.message_box
            .lock()
            .map_err(|e| VMError::Unexpected(e.to_string()))?
            .push(Message::Wait(self.id, WaitRequest { target_id, exit_code_addr }));
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
    fn test_wait_runtime_yields_and_enqueues_request() {
        let mut machine = ScriptVersion::V2.init_core_machine(10_000);
        machine.set_register(A0, 42);
        machine.set_register(A1, 0x3000);
        machine.set_register(A7, WAIT_SYSCALL_NUMBER);

        let runtime = runtime();
        let mut syscall = Wait::with_runtime(7, &runtime);

        let err = syscall.ecall(&mut machine).expect_err("wait should yield to scheduler");

        assert!(matches!(err, VMError::Yield));
        assert_eq!(machine.cycles(), SPAWN_YIELD_CYCLES_BASE);
        assert_eq!(
            runtime.message_box.lock().unwrap().clone(),
            vec![Message::Wait(7, WaitRequest { target_id: 42, exit_code_addr: 0x3000 },)]
        );
    }
}
