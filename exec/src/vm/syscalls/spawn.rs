// SPDX-License-Identifier: MIT
// Copyright (C) 2026 Myelin developers
//
// Spawn syscall

use super::{SPAWN_EXTRA_CYCLES_BASE, SPAWN_SYSCALL_NUMBER, SPAWN_YIELD_CYCLES_BASE};
use crate::vm::scheduler::{Fd, Message, ProgramLocation, ProgramPiece, ProgramResolver, SpawnRequest, VmId, VmRuntime};
use ckb_vm::{
    memory::Memory,
    registers::{A0, A1, A2, A3, A4, A7},
    Error as VMError, Register, SupportMachine, Syscalls,
};
use std::sync::Arc;

/// Syscall: Spawn
///
/// Syscall number: 2601
///
/// Requires a scheduler runtime and yields requests to its multi-VM loop.
#[derive(Clone)]
pub struct Spawn {
    id: VmId,
    message_box: Arc<std::sync::Mutex<Vec<Message>>>,
    program_resolver: ProgramResolver,
}

impl Spawn {
    pub fn with_runtime(id: VmId, runtime: &VmRuntime, program_resolver: ProgramResolver) -> Self {
        Self { id, message_box: Arc::clone(&runtime.message_box), program_resolver }
    }
}

impl<M: SupportMachine> Syscalls<M> for Spawn {
    fn initialize(&mut self, _machine: &mut M) -> Result<(), VMError> {
        Ok(())
    }

    fn ecall(&mut self, machine: &mut M) -> Result<bool, VMError> {
        if machine.registers()[A7].to_u64() != SPAWN_SYSCALL_NUMBER {
            return Ok(false);
        }

        let index = machine.registers()[A0].to_u64() as usize;
        let source = super::Source::parse_from_u64(machine.registers()[A1].to_u64())?;
        let Some(place) = crate::vm::scheduler::ProgramPlace::parse(machine.registers()[A2].to_u64()) else {
            machine.set_register(A0, M::REG::from_u8(super::INDEX_OUT_OF_BOUND));
            return Ok(true);
        };
        if matches!(source, super::Source::HeaderDep | super::Source::GroupCellDep | super::Source::GroupHeaderDep) {
            machine.set_register(A0, M::REG::from_u8(super::INDEX_OUT_OF_BOUND));
            return Ok(true);
        }
        let bounds = machine.registers()[A3].to_u64();
        let offset = (bounds >> 32) as u32 as usize;
        let length = bounds as u32 as usize;
        let spgs_addr = machine.registers()[A4].to_u64();
        let argc = machine.memory_mut().load64(&M::REG::from_u64(spgs_addr))?.to_u64();
        let argv = machine.memory_mut().load64(&M::REG::from_u64(spgs_addr.wrapping_add(8)))?.to_u64();
        let process_id_addr = machine.memory_mut().load64(&M::REG::from_u64(spgs_addr.wrapping_add(16)))?.to_u64();
        let mut fds_addr = machine.memory_mut().load64(&M::REG::from_u64(spgs_addr.wrapping_add(24)))?.to_u64();

        let mut fds = Vec::new();
        if fds_addr != 0 {
            loop {
                let fd = machine.memory_mut().load64(&M::REG::from_u64(fds_addr))?.to_u64();
                if fd == 0 {
                    break;
                }
                fds.push(Fd(fd));
                fds_addr = fds_addr.checked_add(8).ok_or(VMError::MemOutOfBound)?;
            }
        }

        let piece = ProgramPiece { source, index, place };
        let payload = match (self.program_resolver)(&piece) {
            Ok(payload) => payload,
            Err(code) => {
                machine.set_register(A0, M::REG::from_u8(code));
                return Ok(true);
            }
        };
        if crate::vm::scheduler::slice_program(&payload, offset, length).is_err() {
            machine.set_register(A0, M::REG::from_u8(super::SLICE_OUT_OF_BOUND));
            return Ok(true);
        }

        machine.add_cycles_no_checking(SPAWN_EXTRA_CYCLES_BASE)?;
        machine.add_cycles_no_checking(SPAWN_YIELD_CYCLES_BASE)?;
        self.message_box.lock().map_err(|e| VMError::Unexpected(e.to_string()))?.push(Message::Spawn(
            self.id,
            SpawnRequest { location: ProgramLocation { piece, offset, length }, argc, argv, fds, process_id_addr },
        ));
        Err(VMError::Yield)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::vm::scheduler::{ProgramLocation, ProgramPiece, ProgramPlace, ProgramResolver, VmRuntime};
    use crate::vm::syscalls::Source;
    use crate::vm::ScriptVersion;
    use ckb_vm::{CoreMachine, Error as VMError, Memory};
    use std::sync::{atomic::AtomicU64, Arc, Mutex};

    const SPGS_ADDR: u64 = 0x1000;
    const PROCESS_ID_ADDR: u64 = 0x2000;

    fn runtime() -> VmRuntime {
        VmRuntime::with_parts(Arc::new(AtomicU64::new(0)), Arc::new(Mutex::new(Vec::new())))
    }

    #[test]
    fn test_spawn_runtime_yields_and_enqueues_request() {
        let mut machine = ScriptVersion::V2.init_core_machine(200_000);
        machine.memory_mut().store64(&SPGS_ADDR, &0u64).unwrap();
        machine.memory_mut().store64(&(SPGS_ADDR + 8), &0u64).unwrap();
        machine.memory_mut().store64(&(SPGS_ADDR + 16), &PROCESS_ID_ADDR).unwrap();
        machine.memory_mut().store64(&(SPGS_ADDR + 24), &0u64).unwrap();
        machine.set_register(A0, 3);
        machine.set_register(A1, Source::CellDep as u64);
        machine.set_register(A2, ProgramPlace::Witness as u64);
        machine.set_register(A3, (1u64 << 32) | 3);
        machine.set_register(A4, SPGS_ADDR);
        machine.set_register(A7, SPAWN_SYSCALL_NUMBER);

        let runtime = runtime();
        let resolver: ProgramResolver = Arc::new(|_| Ok(vec![0xAA, 0xBB, 0xCC, 0xDD, 0xEE]));
        let mut syscall = Spawn::with_runtime(7, &runtime, resolver);

        let err = syscall.ecall(&mut machine).expect_err("spawn should yield to scheduler");

        assert!(matches!(err, VMError::Yield));
        assert_eq!(machine.cycles(), SPAWN_EXTRA_CYCLES_BASE + SPAWN_YIELD_CYCLES_BASE);
        assert_eq!(
            runtime.message_box.lock().unwrap().clone(),
            vec![Message::Spawn(
                7,
                SpawnRequest {
                    location: ProgramLocation {
                        piece: ProgramPiece { source: Source::CellDep, index: 3, place: ProgramPlace::Witness },
                        offset: 1,
                        length: 3,
                    },
                    argc: 0,
                    argv: 0,
                    fds: vec![],
                    process_id_addr: PROCESS_ID_ADDR,
                },
            )]
        );
    }

    #[test]
    fn test_spawn_runtime_traps_on_invalid_source_encoding() {
        let mut machine = ScriptVersion::V2.init_core_machine(200_000);
        machine.set_register(A0, 0);
        machine.set_register(A1, 0x99);
        machine.set_register(A2, ProgramPlace::CellData as u64);
        machine.set_register(A3, 0);
        machine.set_register(A4, SPGS_ADDR);
        machine.set_register(A7, SPAWN_SYSCALL_NUMBER);
        machine.memory_mut().store64(&SPGS_ADDR, &0u64).unwrap();
        machine.memory_mut().store64(&(SPGS_ADDR + 8), &0u64).unwrap();
        machine.memory_mut().store64(&(SPGS_ADDR + 16), &0u64).unwrap();
        machine.memory_mut().store64(&(SPGS_ADDR + 24), &0u64).unwrap();

        let runtime = runtime();
        let resolver: ProgramResolver = Arc::new(|_| Ok(vec![0xAA]));
        let mut syscall = Spawn::with_runtime(7, &runtime, resolver);

        let err = syscall.ecall(&mut machine).expect_err("invalid source should trap");

        assert_eq!(err, VMError::External("SourceEntry parse_from_u64 153".to_string()));
        assert!(runtime.message_box.lock().unwrap().is_empty());
    }
}
