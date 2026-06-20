// SPDX-License-Identifier: MIT
// Copyright (C) 2026 Myelin developers
//
// Multi-VM scheduler for Spawn/Wait/FD syscalls

use super::error::{ScriptError, ScriptResult, VMError};
use super::machine::{Machine, ScriptVersion};
use super::syscalls::{
    INVALID_FD, MAX_FDS_CREATED, MAX_VMS_SPAWNED, OTHER_END_CLOSED, SPAWN_EXTRA_CYCLES_BASE, SUCCESS, WAIT_FAILURE,
};
use super::transferred_byte_cycles;
use crate::serialization::split_vm_abi_trailer;
use ckb_vm::{
    bytes::Bytes,
    cost_model::estimate_cycles,
    elf::parse_elf,
    machine::Pause,
    memory::load_c_string_byte_by_byte,
    registers::A0,
    snapshot2::{DataSource, Snapshot2, Snapshot2Context},
    CoreMachine, DefaultMachineBuilder, DefaultMachineRunner, Error as CkbVmError, Memory, Register, SupportMachine, Syscalls,
    RISCV_GENERAL_REGISTER_NUMBER,
};
use std::collections::{BTreeMap, BTreeSet, HashMap};
use std::mem::size_of;
use std::sync::{
    atomic::{AtomicU64, Ordering},
    Arc, Mutex,
};

macro_rules! vm_try {
    ($limit:expr, $expr:expr) => {
        $expr.map_err(|err| map_vm_error($limit, err))?
    };
}

fn map_vm_error(max_cycles: u64, err: CkbVmError) -> ScriptError {
    match err {
        CkbVmError::CyclesExceeded => {
            ScriptError::VM(VMError::CyclesExceeded { limit: max_cycles, actual: max_cycles.saturating_add(1) })
        }
        other => ScriptError::VM(VMError::ExecutionError(other.to_string())),
    }
}

fn lock_err(label: &str, err: impl std::fmt::Display) -> ScriptError {
    ScriptError::VM(VMError::ExecutionError(format!("{label} poisoned: {err}")))
}

pub type VmId = u64;
pub const ROOT_VM_ID: VmId = 0;
pub const MAX_VMS_COUNT: usize = 16;
pub const MAX_INSTANTIATED_VMS: usize = 4;
pub const MAX_FDS: usize = 64;
pub const FIRST_FD_SLOT: u64 = 2;

const MAX_ARGV_LENGTH: u64 = 1024 * 1024;

pub type MachineInner = <Machine as DefaultMachineRunner>::Inner;
pub type BoxedSyscall = Box<dyn Syscalls<MachineInner>>;
pub type SyscallFactory = Arc<dyn Fn(VmId, &VmRuntime) -> ScriptResult<Vec<BoxedSyscall>> + Send + Sync>;
pub type ProgramResolver = Arc<dyn Fn(&ProgramPiece) -> Result<Vec<u8>, u8> + Send + Sync>;
pub type VmSnapshotContext = Snapshot2Context<ProgramDataId, SchedulerDataSource>;
pub type VmSnapshotHandle = Arc<Mutex<VmSnapshotContext>>;

#[derive(Clone)]
pub struct VmRuntime {
    pub base_cycles: Arc<AtomicU64>,
    pub message_box: Arc<Mutex<Vec<Message>>>,
    pub snapshot2_context: VmSnapshotHandle,
    pub data_source: SchedulerDataSource,
}

impl VmRuntime {
    pub fn with_parts(base_cycles: Arc<AtomicU64>, message_box: Arc<Mutex<Vec<Message>>>) -> Self {
        let data_source = SchedulerDataSource::new(Bytes::new(), Arc::new(|_| Err(0)));
        let snapshot2_context = Arc::new(Mutex::new(data_source.snapshot_context()));
        Self { base_cycles, message_box, snapshot2_context, data_source }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ProgramPlace {
    CellData = 0,
    Witness = 1,
}

impl ProgramPlace {
    pub fn parse(value: u64) -> Option<Self> {
        match value {
            0 => Some(Self::CellData),
            1 => Some(Self::Witness),
            _ => None,
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ProgramPiece {
    pub source: super::syscalls::Source,
    pub index: usize,
    pub place: ProgramPlace,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ProgramLocation {
    pub piece: ProgramPiece,
    pub offset: usize,
    pub length: usize,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum ProgramDataId {
    Root,
    Piece(ProgramPiece),
}

#[derive(Clone)]
pub struct SchedulerDataSource {
    root_program: Bytes,
    program_resolver: ProgramResolver,
}

impl SchedulerDataSource {
    pub fn new(root_program: Bytes, program_resolver: ProgramResolver) -> Self {
        Self { root_program, program_resolver }
    }

    pub fn snapshot_context(&self) -> VmSnapshotContext {
        Snapshot2Context::new(self.clone())
    }

    fn slice_bytes(payload: &[u8], offset: u64, length: u64) -> Option<(Bytes, u64)> {
        let offset = std::cmp::min(offset as usize, payload.len());
        let full_length = payload.len().checked_sub(offset)?;
        let real_length = if length > 0 { std::cmp::min(full_length, length as usize) } else { full_length };
        Some((Bytes::copy_from_slice(&payload[offset..offset + real_length]), full_length as u64))
    }
}

impl DataSource<ProgramDataId> for SchedulerDataSource {
    fn load_data(&self, id: &ProgramDataId, offset: u64, length: u64) -> Option<(Bytes, u64)> {
        match id {
            ProgramDataId::Root => Self::slice_bytes(self.root_program.as_ref(), offset, length),
            ProgramDataId::Piece(piece) => {
                let payload = (self.program_resolver)(piece).ok()?;
                Self::slice_bytes(&payload, offset, length)
            }
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct Fd(pub u64);

impl Fd {
    pub fn create(slot: u64) -> (Fd, Fd, u64) {
        (Fd(slot), Fd(slot + 1), slot + 2)
    }

    pub fn other_fd(self) -> Fd {
        Fd(self.0 ^ 0x1)
    }

    pub fn is_read(self) -> bool {
        self.0.is_multiple_of(2)
    }

    pub fn is_write(self) -> bool {
        self.0 % 2 == 1
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ReadState {
    pub fd: Fd,
    pub length: u64,
    pub buffer_addr: u64,
    pub length_addr: u64,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct WriteState {
    pub fd: Fd,
    pub consumed: u64,
    pub length: u64,
    pub buffer_addr: u64,
    pub length_addr: u64,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum VmState {
    Runnable,
    Terminated,
    Wait { target_vm_id: VmId, exit_code_addr: u64 },
    WaitForWrite(WriteState),
    WaitForRead(ReadState),
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct SpawnRequest {
    pub location: ProgramLocation,
    pub argc: u64,
    pub argv: u64,
    pub fds: Vec<Fd>,
    pub process_id_addr: u64,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct WaitRequest {
    pub target_id: VmId,
    pub exit_code_addr: u64,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct PipeRequest {
    pub fd1_addr: u64,
    pub fd2_addr: u64,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct FdRequest {
    pub fd: Fd,
    pub length: u64,
    pub buffer_addr: u64,
    pub length_addr: u64,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Message {
    Spawn(VmId, SpawnRequest),
    Wait(VmId, WaitRequest),
    Pipe(VmId, PipeRequest),
    FdRead(VmId, FdRequest),
    FdWrite(VmId, FdRequest),
    InheritedFileDescriptor(VmId, FdRequest),
    Close(VmId, Fd),
}

#[derive(Clone)]
enum VmArgs {
    Vector(Vec<Bytes>),
    Reader { vm_id: VmId, argc: u64, argv: u64 },
}

#[derive(Clone)]
struct VmContext {
    snapshot2_context: VmSnapshotHandle,
}

type VmSlot = (VmContext, Machine);

#[derive(Clone)]
pub enum RunMode {
    LimitCycles(u64),
    Pause(Pause, u64),
}

#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord)]
pub struct TerminatedResult {
    pub exit_code: i8,
    pub consumed_cycles: u64,
}

#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord)]
pub struct IterationResult {
    pub executed_vm: VmId,
    pub terminated_status: Option<TerminatedResult>,
}

#[derive(Clone, Debug)]
pub struct FullSuspendedState {
    pub total_cycles: u64,
    pub iteration_cycles: u64,
    pub next_vm_id: VmId,
    pub next_fd_slot: u64,
    pub vms: Vec<(VmId, VmState, Snapshot2<ProgramDataId>)>,
    pub fds: Vec<(Fd, VmId)>,
    pub inherited_fd: Vec<(VmId, Vec<Fd>)>,
    pub terminated_vms: Vec<(VmId, i8)>,
    pub instantiated_ids: Vec<VmId>,
}

impl FullSuspendedState {
    pub fn size(&self) -> u64 {
        (size_of::<u64>()
            + size_of::<u64>()
            + size_of::<VmId>()
            + size_of::<u64>()
            + self.vms.iter().fold(0usize, |mut acc, (_, _, snapshot)| {
                acc += size_of::<VmId>() + size_of::<VmState>();
                acc += snapshot.pages_from_source.len()
                    * (size_of::<u64>() + size_of::<u8>() + size_of::<ProgramDataId>() + size_of::<u64>() + size_of::<u64>());
                for dirty_page in &snapshot.dirty_pages {
                    acc += size_of::<u64>() + size_of::<u8>() + dirty_page.2.len();
                }
                acc += size_of::<u32>()
                    + RISCV_GENERAL_REGISTER_NUMBER * size_of::<u64>()
                    + size_of::<u64>()
                    + size_of::<u64>()
                    + size_of::<u64>();
                acc
            })
            + self.fds.len() * (size_of::<Fd>() + size_of::<VmId>())
            + self.inherited_fd.iter().fold(0usize, |acc, (_, fds)| acc + size_of::<VmId>() + fds.len() * size_of::<Fd>())
            + self.terminated_vms.len() * (size_of::<VmId>() + size_of::<i8>())
            + self.instantiated_ids.len() * size_of::<VmId>()) as u64
    }
}

pub struct VmScheduler {
    version: ScriptVersion,
    max_cycles: u64,
    max_memory: usize,
    max_script_size: usize,
    root_program: Bytes,
    root_args: Vec<Bytes>,
    syscall_factory: SyscallFactory,
    program_resolver: ProgramResolver,
    total_cycles: Arc<AtomicU64>,
    iteration_cycles: u64,
    next_vm_id: VmId,
    next_fd_slot: u64,
    states: BTreeMap<VmId, VmState>,
    fds: BTreeMap<Fd, VmId>,
    inherited_fd: BTreeMap<VmId, Vec<Fd>>,
    instantiated: BTreeMap<VmId, VmSlot>,
    suspended: BTreeMap<VmId, Snapshot2<ProgramDataId>>,
    terminated_vms: BTreeMap<VmId, i8>,
    message_box: Arc<Mutex<Vec<Message>>>,
}

impl VmScheduler {
    pub fn new(
        version: ScriptVersion,
        max_cycles: u64,
        max_memory: usize,
        max_script_size: usize,
        root_program: Vec<u8>,
        root_args: Vec<Vec<u8>>,
        syscall_factory: SyscallFactory,
        program_resolver: ProgramResolver,
    ) -> Self {
        Self {
            version,
            max_cycles,
            max_memory,
            max_script_size,
            root_program: Bytes::from(root_program),
            root_args: root_args.into_iter().map(Bytes::from).collect(),
            syscall_factory,
            program_resolver,
            total_cycles: Arc::new(AtomicU64::new(0)),
            iteration_cycles: 0,
            next_vm_id: ROOT_VM_ID,
            next_fd_slot: FIRST_FD_SLOT,
            states: BTreeMap::new(),
            fds: BTreeMap::new(),
            inherited_fd: BTreeMap::new(),
            instantiated: BTreeMap::new(),
            suspended: BTreeMap::new(),
            terminated_vms: BTreeMap::new(),
            message_box: Arc::new(Mutex::new(Vec::new())),
        }
    }

    pub fn consumed_cycles(&self) -> u64 {
        self.total_cycles.load(Ordering::Acquire)
    }

    pub fn state(&self, vm_id: &VmId) -> Option<VmState> {
        self.states.get(vm_id).cloned()
    }

    pub fn run(&mut self) -> ScriptResult<u64> {
        let terminated = self.run_with_mode(RunMode::LimitCycles(self.max_cycles))?;
        if terminated.exit_code != 0 {
            return Err(ScriptError::VM(VMError::NonZeroExitCode(terminated.exit_code)));
        }
        Ok(terminated.consumed_cycles)
    }

    pub fn run_with_mode(&mut self, mode: RunMode) -> ScriptResult<TerminatedResult> {
        self.boot_root_vm_if_needed()?;

        let (pause, mut limit_cycles) = match mode {
            RunMode::LimitCycles(limit_cycles) => (Pause::new(), limit_cycles),
            RunMode::Pause(pause, limit_cycles) => (pause, limit_cycles),
        };

        while !self.terminated() {
            limit_cycles = self.iterate_outer(&pause, limit_cycles)?.1;
        }

        self.terminated_result()
    }

    pub fn iterate(&mut self) -> ScriptResult<IterationResult> {
        self.boot_root_vm_if_needed()?;

        if self.terminated() {
            return Ok(IterationResult { executed_vm: ROOT_VM_ID, terminated_status: Some(self.terminated_result()?) });
        }

        let (executed_vm, _) = self.iterate_outer(&Pause::new(), u64::MAX)?;
        let terminated_status = if self.terminated() { Some(self.terminated_result()?) } else { None };

        Ok(IterationResult { executed_vm, terminated_status })
    }

    fn iterate_outer(&mut self, pause: &Pause, limit_cycles: u64) -> ScriptResult<(VmId, u64)> {
        let iterate_result = self.iterate_inner(pause.clone(), limit_cycles);
        let spent_cycles = self.consume_iteration_cycles()?;
        let remaining_cycles = limit_cycles
            .checked_sub(spent_cycles)
            .ok_or(ScriptError::VM(VMError::CyclesExceeded { limit: limit_cycles, actual: spent_cycles }))?;
        self.process_io()?;
        let executed_vm = iterate_result?;
        Ok((executed_vm, remaining_cycles))
    }

    fn iterate_inner(&mut self, pause: Pause, limit_cycles: u64) -> ScriptResult<VmId> {
        let vm_id_to_run = self.iterate_prepare_machine()?;
        let remaining_cycles = self.remaining_cycles()?.min(limit_cycles);

        let result = {
            let machine = self.ensure_get_machine_mut(vm_id_to_run)?;
            machine.inner_mut().set_max_cycles(remaining_cycles);
            machine.machine_mut().set_pause(pause);
            let result = machine.run();
            let cycles = machine.machine().cycles();
            machine.inner_mut().set_cycles(0);
            self.iteration_cycles = self.iteration_cycles.checked_add(cycles).ok_or_else(|| {
                ScriptError::VM(VMError::CyclesExceeded { limit: self.max_cycles, actual: self.max_cycles.saturating_add(1) })
            })?;
            result
        };

        self.process_message_box()?;
        self.iterate_process_results(vm_id_to_run, result)?;
        Ok(vm_id_to_run)
    }

    fn iterate_prepare_machine(&mut self) -> ScriptResult<VmId> {
        let vm_id_to_run = self
            .states
            .iter()
            .rev()
            .find_map(|(id, state)| matches!(state, VmState::Runnable).then_some(*id))
            .ok_or_else(|| ScriptError::VM(VMError::ExecutionError("scheduler deadlock: no runnable VMs".to_string())))?;
        self.ensure_vms_instantiated(&[vm_id_to_run])?;
        Ok(vm_id_to_run)
    }

    fn iterate_process_results(&mut self, vm_id_to_run: VmId, result: Result<i8, CkbVmError>) -> ScriptResult<()> {
        let max_cycles = self.max_cycles;
        match result {
            Ok(code) => {
                self.terminated_vms.insert(vm_id_to_run, code);
                if vm_id_to_run == ROOT_VM_ID {
                    self.instantiated.retain(|id, _| *id == vm_id_to_run);
                    self.suspended.clear();
                    self.states.clear();
                    self.states.insert(vm_id_to_run, VmState::Terminated);
                    self.fds.clear();
                    self.inherited_fd.clear();
                } else {
                    let joining_vms: Vec<(VmId, u64)> = self
                        .states
                        .iter()
                        .filter_map(|(vm_id, state)| match state {
                            VmState::Wait { target_vm_id, exit_code_addr } if *target_vm_id == vm_id_to_run => {
                                Some((*vm_id, *exit_code_addr))
                            }
                            _ => None,
                        })
                        .collect();

                    for (vm_id, exit_code_addr) in joining_vms {
                        let machine = self.ensure_get_machine_mut(vm_id)?;
                        vm_try!(
                            max_cycles,
                            machine.inner_mut().memory_mut().store8(&Self::u64_to_reg(exit_code_addr), &Self::i8_to_reg(code))
                        );
                        machine.inner_mut().set_register(A0, Self::u8_to_reg(SUCCESS));
                        self.states.insert(vm_id, VmState::Runnable);
                    }

                    self.fds.retain(|_, owner| *owner != vm_id_to_run);
                    self.inherited_fd.remove(&vm_id_to_run);
                    self.states.remove(&vm_id_to_run);
                    self.instantiated.remove(&vm_id_to_run);
                    self.suspended.remove(&vm_id_to_run);
                }
                Ok(())
            }
            Err(CkbVmError::Yield) => Ok(()),
            Err(CkbVmError::Pause) => Err(ScriptError::VM(VMError::Paused)),
            Err(CkbVmError::CyclesExceeded) => {
                Err(ScriptError::VM(VMError::CyclesExceeded { limit: self.max_cycles, actual: self.max_cycles.saturating_add(1) }))
            }
            Err(other) => Err(ScriptError::VM(VMError::ExecutionError(other.to_string()))),
        }
    }

    fn consume_iteration_cycles(&mut self) -> ScriptResult<u64> {
        let spent_cycles = self.iteration_cycles;
        let total = self.total_cycles.load(Ordering::Acquire);
        let next_total = total.checked_add(spent_cycles).ok_or_else(|| {
            ScriptError::VM(VMError::CyclesExceeded { limit: self.max_cycles, actual: self.max_cycles.saturating_add(1) })
        })?;
        if next_total > self.max_cycles {
            return Err(ScriptError::VM(VMError::CyclesExceeded { limit: self.max_cycles, actual: next_total }));
        }
        self.total_cycles.store(next_total, Ordering::Release);
        self.iteration_cycles = 0;
        Ok(spent_cycles)
    }

    fn process_message_box(&mut self) -> ScriptResult<()> {
        let max_cycles = self.max_cycles;
        let messages: Vec<Message> = self.message_box.lock().map_err(|err| lock_err("message box", err))?.drain(..).collect();

        for message in messages {
            match message {
                Message::Spawn(vm_id, args) => {
                    if args.fds.iter().any(|fd| self.fds.get(fd) != Some(&vm_id)) {
                        let machine = self.ensure_get_machine_mut(vm_id)?;
                        machine.inner_mut().set_register(A0, Self::u8_to_reg(INVALID_FD));
                        continue;
                    }
                    if self.states.len() >= MAX_VMS_COUNT {
                        let machine = self.ensure_get_machine_mut(vm_id)?;
                        machine.inner_mut().set_register(A0, Self::u8_to_reg(MAX_VMS_SPAWNED));
                        continue;
                    }

                    let spawned_vm_id =
                        self.boot_vm(args.location.clone(), VmArgs::Reader { vm_id, argc: args.argc, argv: args.argv })?;

                    for fd in &args.fds {
                        self.fds.insert(*fd, spawned_vm_id);
                    }
                    self.inherited_fd.insert(spawned_vm_id, args.fds.clone());

                    let machine = self.ensure_get_machine_mut(vm_id)?;
                    vm_try!(
                        max_cycles,
                        machine
                            .inner_mut()
                            .memory_mut()
                            .store64(&Self::u64_to_reg(args.process_id_addr), &Self::u64_to_reg(spawned_vm_id),)
                    );
                    machine.inner_mut().set_register(A0, Self::u8_to_reg(SUCCESS));
                }
                Message::Wait(vm_id, args) => {
                    if let Some(exit_code) = self.terminated_vms.get(&args.target_id).copied() {
                        let machine = self.ensure_get_machine_mut(vm_id)?;
                        vm_try!(
                            max_cycles,
                            machine
                                .inner_mut()
                                .memory_mut()
                                .store8(&Self::u64_to_reg(args.exit_code_addr), &Self::i8_to_reg(exit_code),)
                        );
                        machine.inner_mut().set_register(A0, Self::u8_to_reg(SUCCESS));
                        self.states.insert(vm_id, VmState::Runnable);
                        self.terminated_vms.retain(|id, _| *id != args.target_id);
                        continue;
                    }
                    if !self.states.contains_key(&args.target_id) {
                        let machine = self.ensure_get_machine_mut(vm_id)?;
                        machine.inner_mut().set_register(A0, Self::u8_to_reg(WAIT_FAILURE));
                        continue;
                    }
                    self.states.insert(vm_id, VmState::Wait { target_vm_id: args.target_id, exit_code_addr: args.exit_code_addr });
                }
                Message::Pipe(vm_id, args) => {
                    if self.fds.len() >= MAX_FDS {
                        let machine = self.ensure_get_machine_mut(vm_id)?;
                        machine.inner_mut().set_register(A0, Self::u8_to_reg(MAX_FDS_CREATED));
                        continue;
                    }
                    let (fd1, fd2, next_slot) = Fd::create(self.next_fd_slot);
                    self.next_fd_slot = next_slot;
                    self.fds.insert(fd1, vm_id);
                    self.fds.insert(fd2, vm_id);
                    let machine = self.ensure_get_machine_mut(vm_id)?;
                    vm_try!(
                        max_cycles,
                        machine.inner_mut().memory_mut().store64(&Self::u64_to_reg(args.fd1_addr), &Self::u64_to_reg(fd1.0))
                    );
                    vm_try!(
                        max_cycles,
                        machine.inner_mut().memory_mut().store64(&Self::u64_to_reg(args.fd2_addr), &Self::u64_to_reg(fd2.0))
                    );
                    machine.inner_mut().set_register(A0, Self::u8_to_reg(SUCCESS));
                }
                Message::FdRead(vm_id, args) => {
                    if self.fds.get(&args.fd) != Some(&vm_id) {
                        let machine = self.ensure_get_machine_mut(vm_id)?;
                        machine.inner_mut().set_register(A0, Self::u8_to_reg(INVALID_FD));
                        continue;
                    }
                    if !self.fds.contains_key(&args.fd.other_fd()) {
                        let machine = self.ensure_get_machine_mut(vm_id)?;
                        machine.inner_mut().set_register(A0, Self::u8_to_reg(OTHER_END_CLOSED));
                        continue;
                    }
                    self.states.insert(
                        vm_id,
                        VmState::WaitForRead(ReadState {
                            fd: args.fd,
                            length: args.length,
                            buffer_addr: args.buffer_addr,
                            length_addr: args.length_addr,
                        }),
                    );
                }
                Message::FdWrite(vm_id, args) => {
                    if self.fds.get(&args.fd) != Some(&vm_id) {
                        let machine = self.ensure_get_machine_mut(vm_id)?;
                        machine.inner_mut().set_register(A0, Self::u8_to_reg(INVALID_FD));
                        continue;
                    }
                    if !self.fds.contains_key(&args.fd.other_fd()) {
                        let machine = self.ensure_get_machine_mut(vm_id)?;
                        machine.inner_mut().set_register(A0, Self::u8_to_reg(OTHER_END_CLOSED));
                        continue;
                    }
                    self.states.insert(
                        vm_id,
                        VmState::WaitForWrite(WriteState {
                            fd: args.fd,
                            consumed: 0,
                            length: args.length,
                            buffer_addr: args.buffer_addr,
                            length_addr: args.length_addr,
                        }),
                    );
                }
                Message::InheritedFileDescriptor(vm_id, args) => {
                    let inherited_fd =
                        if vm_id == ROOT_VM_ID { Vec::new() } else { self.inherited_fd.get(&vm_id).cloned().unwrap_or_default() };
                    let machine = self.ensure_get_machine_mut(vm_id)?;
                    let full_length =
                        vm_try!(max_cycles, machine.inner_mut().memory_mut().load64(&Self::u64_to_reg(args.length_addr))).to_u64();
                    let real_length = inherited_fd.len() as u64;
                    let copy_length = u64::min(full_length, real_length);
                    for i in 0..copy_length {
                        let addr = args
                            .buffer_addr
                            .checked_add(i * 8)
                            .ok_or_else(|| ScriptError::VM(VMError::ExecutionError("inherited fd buffer overflow".to_string())))?;
                        vm_try!(
                            max_cycles,
                            machine
                                .inner_mut()
                                .memory_mut()
                                .store64(&Self::u64_to_reg(addr), &Self::u64_to_reg(inherited_fd[i as usize].0))
                        );
                    }
                    vm_try!(
                        max_cycles,
                        machine.inner_mut().memory_mut().store64(&Self::u64_to_reg(args.length_addr), &Self::u64_to_reg(real_length))
                    );
                    machine.inner_mut().set_register(A0, Self::u8_to_reg(SUCCESS));
                }
                Message::Close(vm_id, fd) => {
                    let is_owner = self.fds.get(&fd) == Some(&vm_id);
                    if is_owner {
                        self.fds.remove(&fd);
                    }
                    let machine = self.ensure_get_machine_mut(vm_id)?;
                    if !is_owner {
                        machine.inner_mut().set_register(A0, Self::u8_to_reg(INVALID_FD));
                    } else {
                        machine.inner_mut().set_register(A0, Self::u8_to_reg(SUCCESS));
                    }
                }
            }
        }
        Ok(())
    }

    fn process_io(&mut self) -> ScriptResult<()> {
        let max_cycles = self.max_cycles;
        let mut reads: HashMap<Fd, (VmId, ReadState)> = HashMap::new();
        let mut closed_vms: BTreeSet<VmId> = BTreeSet::new();

        for (vm_id, state) in &self.states {
            if let VmState::WaitForRead(inner_state) = state {
                if self.fds.contains_key(&inner_state.fd.other_fd()) {
                    reads.insert(inner_state.fd, (*vm_id, inner_state.clone()));
                } else {
                    closed_vms.insert(*vm_id);
                }
            }
        }

        let mut pairs: Vec<(VmId, ReadState, VmId, WriteState)> = Vec::new();
        for (vm_id, state) in &self.states {
            if let VmState::WaitForWrite(inner_state) = state {
                if self.fds.contains_key(&inner_state.fd.other_fd()) {
                    if let Some((read_vm_id, read_state)) = reads.get(&inner_state.fd.other_fd()) {
                        pairs.push((*read_vm_id, read_state.clone(), *vm_id, inner_state.clone()));
                    }
                } else {
                    closed_vms.insert(*vm_id);
                }
            }
        }

        for vm_id in closed_vms {
            match self.states.get(&vm_id).cloned() {
                Some(VmState::WaitForRead(ReadState { length_addr, .. })) => {
                    let machine = self.ensure_get_machine_mut(vm_id)?;
                    vm_try!(
                        max_cycles,
                        machine.inner_mut().memory_mut().store64(&Self::u64_to_reg(length_addr), &Self::u64_to_reg(0))
                    );
                    machine.inner_mut().set_register(A0, Self::u8_to_reg(SUCCESS));
                    self.states.insert(vm_id, VmState::Runnable);
                }
                Some(VmState::WaitForWrite(WriteState { consumed, length_addr, .. })) => {
                    let machine = self.ensure_get_machine_mut(vm_id)?;
                    vm_try!(
                        max_cycles,
                        machine.inner_mut().memory_mut().store64(&Self::u64_to_reg(length_addr), &Self::u64_to_reg(consumed))
                    );
                    machine.inner_mut().set_register(A0, Self::u8_to_reg(SUCCESS));
                    self.states.insert(vm_id, VmState::Runnable);
                }
                _ => {}
            }
        }

        for (read_vm_id, read_state, write_vm_id, write_state) in pairs {
            self.ensure_vms_instantiated(&[read_vm_id, write_vm_id])?;

            let ReadState { length: read_length, buffer_addr: read_buffer_addr, length_addr: read_length_addr, .. } = read_state;
            let WriteState {
                fd: write_fd,
                mut consumed,
                length: write_length,
                buffer_addr: write_buffer_addr,
                length_addr: write_length_addr,
            } = write_state;

            let copiable = u64::min(read_length, write_length.saturating_sub(consumed));
            if copiable == 0 {
                continue;
            }

            let data = {
                let write_machine = self.ensure_get_machine_mut(write_vm_id)?;
                vm_try!(max_cycles, write_machine.inner_mut().add_cycles_no_checking(transferred_byte_cycles(copiable as usize)));
                vm_try!(
                    max_cycles,
                    write_machine.inner_mut().memory_mut().load_bytes(write_buffer_addr.wrapping_add(consumed), copiable)
                )
            };

            {
                let read_machine = self.ensure_get_machine_mut(read_vm_id)?;
                vm_try!(max_cycles, read_machine.inner_mut().add_cycles_no_checking(transferred_byte_cycles(copiable as usize)));
                vm_try!(max_cycles, read_machine.inner_mut().memory_mut().store_bytes(read_buffer_addr, &data));
                vm_try!(
                    max_cycles,
                    read_machine.inner_mut().memory_mut().store64(&Self::u64_to_reg(read_length_addr), &Self::u64_to_reg(copiable))
                );
                read_machine.inner_mut().set_register(A0, Self::u8_to_reg(SUCCESS));
                self.states.insert(read_vm_id, VmState::Runnable);
            }

            consumed = consumed.saturating_add(copiable);
            if consumed >= write_length {
                let write_machine = self.ensure_get_machine_mut(write_vm_id)?;
                vm_try!(
                    max_cycles,
                    write_machine
                        .inner_mut()
                        .memory_mut()
                        .store64(&Self::u64_to_reg(write_length_addr), &Self::u64_to_reg(write_length))
                );
                write_machine.inner_mut().set_register(A0, Self::u8_to_reg(SUCCESS));
                self.states.insert(write_vm_id, VmState::Runnable);
            } else {
                self.states.insert(
                    write_vm_id,
                    VmState::WaitForWrite(WriteState {
                        fd: write_fd,
                        consumed,
                        length: write_length,
                        buffer_addr: write_buffer_addr,
                        length_addr: write_length_addr,
                    }),
                );
            }
        }

        Ok(())
    }

    fn terminated(&self) -> bool {
        matches!(self.states.get(&ROOT_VM_ID), Some(VmState::Terminated))
    }

    fn terminated_result(&mut self) -> ScriptResult<TerminatedResult> {
        let exit_code = {
            let machine = self.ensure_get_machine_mut(ROOT_VM_ID)?;
            machine.machine().exit_code()
        };
        Ok(TerminatedResult { exit_code, consumed_cycles: self.consumed_cycles() })
    }

    fn remaining_cycles(&self) -> ScriptResult<u64> {
        let consumed = self.total_cycles.load(Ordering::Acquire);
        if consumed >= self.max_cycles {
            return Err(ScriptError::VM(VMError::CyclesExceeded { limit: self.max_cycles, actual: consumed }));
        }
        Ok(self.max_cycles - consumed)
    }

    pub fn suspend(mut self) -> ScriptResult<FullSuspendedState> {
        if !self.message_box.lock().map_err(|err| lock_err("message box", err))?.is_empty() {
            return Err(ScriptError::VM(VMError::ExecutionError("cannot suspend scheduler with pending messages".to_string())));
        }

        let instantiated_ids: Vec<_> = self.instantiated.keys().copied().collect();
        for id in &instantiated_ids {
            self.suspend_vm(id)?;
        }

        let mut vms = Vec::with_capacity(self.states.len());
        for (id, state) in self.states {
            let snapshot = self
                .suspended
                .remove(&id)
                .ok_or_else(|| ScriptError::VM(VMError::ExecutionError(format!("missing suspended snapshot for VM {id}"))))?;
            vms.push((id, state, snapshot));
        }

        Ok(FullSuspendedState {
            total_cycles: self.total_cycles.load(Ordering::Acquire),
            iteration_cycles: self.iteration_cycles,
            next_vm_id: self.next_vm_id,
            next_fd_slot: self.next_fd_slot,
            vms,
            fds: self.fds.into_iter().collect(),
            inherited_fd: self.inherited_fd.into_iter().collect(),
            terminated_vms: self.terminated_vms.into_iter().collect(),
            instantiated_ids,
        })
    }

    pub fn resume(
        version: ScriptVersion,
        max_cycles: u64,
        max_memory: usize,
        max_script_size: usize,
        root_program: Vec<u8>,
        root_args: Vec<Vec<u8>>,
        syscall_factory: SyscallFactory,
        program_resolver: ProgramResolver,
        suspended: FullSuspendedState,
    ) -> ScriptResult<Self> {
        let FullSuspendedState {
            total_cycles,
            iteration_cycles,
            next_vm_id,
            next_fd_slot,
            vms,
            fds,
            inherited_fd,
            terminated_vms,
            instantiated_ids,
        } = suspended;
        let mut scheduler = Self {
            version,
            max_cycles,
            max_memory,
            max_script_size,
            root_program: Bytes::from(root_program),
            root_args: root_args.into_iter().map(Bytes::from).collect(),
            syscall_factory,
            program_resolver,
            total_cycles: Arc::new(AtomicU64::new(total_cycles)),
            iteration_cycles,
            next_vm_id,
            next_fd_slot,
            states: vms.iter().map(|(id, state, _)| (*id, state.clone())).collect(),
            fds: fds.into_iter().collect(),
            inherited_fd: inherited_fd.into_iter().collect(),
            instantiated: BTreeMap::new(),
            suspended: vms.into_iter().map(|(id, _, snapshot)| (id, snapshot)).collect(),
            terminated_vms: terminated_vms.into_iter().collect(),
            message_box: Arc::new(Mutex::new(Vec::new())),
        };
        scheduler.ensure_vms_instantiated(&instantiated_ids)?;
        scheduler.iteration_cycles = 0;
        Ok(scheduler)
    }

    fn boot_root_vm_if_needed(&mut self) -> ScriptResult<()> {
        if self.states.is_empty() {
            let root_program = self.root_program.clone();
            let root_args = VmArgs::Vector(self.root_args.clone());
            let root_id = self.boot_vm_from_bytes(ProgramDataId::Root, 0, root_program.as_ref(), root_args)?;
            debug_assert_eq!(root_id, ROOT_VM_ID);
        }
        Ok(())
    }

    fn boot_vm(&mut self, location: ProgramLocation, args: VmArgs) -> ScriptResult<VmId> {
        let payload = (self.program_resolver)(&location.piece).map_err(|code| {
            ScriptError::VM(VMError::SyscallError(format!("failed to resolve spawned program payload: return code {code}")))
        })?;
        let program = slice_program(&payload, location.offset, location.length)
            .map_err(|code| ScriptError::VM(VMError::SyscallError(format!("invalid spawned program slice: return code {code}"))))?;
        self.boot_vm_from_bytes(ProgramDataId::Piece(location.piece), location.offset as u64, program, args)
    }

    fn boot_vm_from_bytes(&mut self, program_id: ProgramDataId, data_offset: u64, program: &[u8], args: VmArgs) -> ScriptResult<VmId> {
        let vm_id = self.next_vm_id;
        self.next_vm_id = self.next_vm_id.saturating_add(1);
        let (context, mut machine) = self.create_machine(vm_id)?;
        self.load_program_into_machine(&context, &mut machine, &program_id, data_offset, program, args)?;

        while self.instantiated.len() >= MAX_INSTANTIATED_VMS {
            let suspended_id = *self
                .instantiated
                .first_key_value()
                .ok_or_else(|| ScriptError::VM(VMError::ExecutionError("instantiated VM map unexpectedly empty".to_string())))?
                .0;
            self.suspend_vm(&suspended_id)?;
        }

        self.instantiated.insert(vm_id, (context, machine));
        self.states.insert(vm_id, VmState::Runnable);
        Ok(vm_id)
    }

    fn create_machine(&self, vm_id: VmId) -> ScriptResult<(VmContext, Machine)> {
        let core_machine = self.version.init_core_machine_with_memory(u64::MAX, self.max_memory);
        let data_source = SchedulerDataSource::new(self.root_program.clone(), Arc::clone(&self.program_resolver));
        let snapshot2_context = Arc::new(Mutex::new(data_source.snapshot_context()));
        let runtime = VmRuntime {
            base_cycles: Arc::clone(&self.total_cycles),
            message_box: Arc::clone(&self.message_box),
            snapshot2_context: Arc::clone(&snapshot2_context),
            data_source: data_source.clone(),
        };
        let syscalls = (self.syscall_factory)(vm_id, &runtime)?;
        let builder = syscalls
            .into_iter()
            .fold(DefaultMachineBuilder::new(core_machine).instruction_cycle_func(Box::new(estimate_cycles)), |builder, syscall| {
                builder.syscall(syscall)
            });
        Ok((VmContext { snapshot2_context }, Machine::new(builder.build())))
    }

    fn ensure_vms_instantiated(&mut self, ids: &[VmId]) -> ScriptResult<()> {
        if ids.len() > MAX_INSTANTIATED_VMS {
            return Err(ScriptError::VM(VMError::ExecutionError(format!(
                "at most {MAX_INSTANTIATED_VMS} VMs can be instantiated, requested {}",
                ids.len()
            ))));
        }

        let mut uninstantiated_ids: Vec<VmId> = ids.iter().filter(|id| !self.instantiated.contains_key(id)).copied().collect();

        while !uninstantiated_ids.is_empty() && self.instantiated.len() < MAX_INSTANTIATED_VMS {
            let id = uninstantiated_ids
                .pop()
                .ok_or_else(|| ScriptError::VM(VMError::ExecutionError("uninstantiated VM queue unexpectedly empty".to_string())))?;
            self.resume_vm(&id)?;
        }

        if uninstantiated_ids.is_empty() {
            return Ok(());
        }

        let suspendable_ids: Vec<VmId> = self.instantiated.keys().filter(|id| !ids.contains(id)).copied().collect();

        if suspendable_ids.len() < uninstantiated_ids.len() {
            return Err(ScriptError::VM(VMError::ExecutionError(
                "unable to suspend enough VMs to satisfy instantiation request".to_string(),
            )));
        }

        for (suspend_id, resume_id) in suspendable_ids.iter().zip(uninstantiated_ids.iter()) {
            self.suspend_vm(suspend_id)?;
            self.resume_vm(resume_id)?;
        }

        Ok(())
    }

    fn ensure_get_instantiated(&mut self, id: &VmId) -> ScriptResult<&mut VmSlot> {
        self.ensure_vms_instantiated(&[*id])?;
        self.instantiated.get_mut(id).ok_or_else(|| ScriptError::VM(VMError::ExecutionError(format!("VM {id} is missing"))))
    }

    fn resume_vm(&mut self, id: &VmId) -> ScriptResult<()> {
        let snapshot = self
            .suspended
            .get(id)
            .cloned()
            .ok_or_else(|| ScriptError::VM(VMError::ExecutionError(format!("VM {id} is not suspended"))))?;
        self.iteration_cycles = self.iteration_cycles.checked_add(SPAWN_EXTRA_CYCLES_BASE).ok_or_else(|| {
            ScriptError::VM(VMError::CyclesExceeded { limit: self.max_cycles, actual: self.max_cycles.saturating_add(1) })
        })?;
        let (context, mut machine) = self.create_machine(*id)?;
        {
            let mut snapshot_context = context.snapshot2_context.lock().map_err(|err| lock_err("snapshot2 context", err))?;
            vm_try!(self.max_cycles, snapshot_context.resume(machine.inner_mut(), &snapshot));
        }
        self.instantiated.insert(*id, (context, machine));
        self.suspended.remove(id);
        Ok(())
    }

    fn suspend_vm(&mut self, id: &VmId) -> ScriptResult<()> {
        self.iteration_cycles = self.iteration_cycles.checked_add(SPAWN_EXTRA_CYCLES_BASE).ok_or_else(|| {
            ScriptError::VM(VMError::CyclesExceeded { limit: self.max_cycles, actual: self.max_cycles.saturating_add(1) })
        })?;
        let snapshot = {
            let (context, machine) = self
                .instantiated
                .get_mut(id)
                .ok_or_else(|| ScriptError::VM(VMError::ExecutionError(format!("VM {id} is not instantiated"))))?;
            let snapshot_context = context.snapshot2_context.lock().map_err(|err| lock_err("snapshot2 context", err))?;
            vm_try!(self.max_cycles, snapshot_context.make_snapshot(machine.inner_mut()))
        };
        self.suspended.insert(*id, snapshot);
        self.instantiated.remove(id);
        Ok(())
    }

    fn ensure_get_machine_mut(&mut self, vm_id: VmId) -> ScriptResult<&mut Machine> {
        Ok(&mut self.ensure_get_instantiated(&vm_id)?.1)
    }

    fn load_program_into_machine(
        &mut self,
        context: &VmContext,
        machine: &mut Machine,
        program_id: &ProgramDataId,
        data_offset: u64,
        program: &[u8],
        args: VmArgs,
    ) -> ScriptResult<()> {
        if program.len() > self.max_script_size {
            return Err(ScriptError::VM(VMError::ScriptTooLarge { size: program.len(), limit: self.max_script_size }));
        }

        let collected_args: Vec<Bytes> = match args {
            VmArgs::Vector(args) => args,
            VmArgs::Reader { vm_id, argc, argv } => self.collect_args_from_vm(vm_id, argc, argv)?,
        };

        let (program, _) = split_vm_abi_trailer(program)
            .map_err(|err| ScriptError::VM(VMError::InvalidData(format!("invalid VM ABI artifact trailer: {}", err))))?;
        let program = Bytes::copy_from_slice(program);
        let metadata = parse_elf::<u64>(&program, machine.inner_mut().version())
            .map_err(|err| ScriptError::VM(VMError::LoadProgramError(err.to_string())))?;
        let arg_bytes = collected_args.into_iter().map(Ok);
        machine.load_program(&program, arg_bytes).map_err(|err| ScriptError::VM(VMError::LoadProgramError(err.to_string())))?;
        let mut snapshot_context = context.snapshot2_context.lock().map_err(|err| lock_err("snapshot2 context", err))?;
        *snapshot_context = SchedulerDataSource::new(self.root_program.clone(), Arc::clone(&self.program_resolver)).snapshot_context();
        vm_try!(self.max_cycles, snapshot_context.mark_program(machine.inner_mut(), &metadata, program_id, data_offset));
        Ok(())
    }

    fn collect_args_from_vm(&mut self, vm_id: VmId, argc: u64, mut argv_addr: u64) -> ScriptResult<Vec<Bytes>> {
        let max_cycles = self.max_cycles;
        let machine = self.ensure_get_machine_mut(vm_id)?;
        let mut total_length = 0u64;
        let mut args = Vec::new();
        for _ in 0..argc {
            let arg_addr = vm_try!(max_cycles, machine.inner_mut().memory_mut().load64(&Self::u64_to_reg(argv_addr)));
            let arg = vm_try!(max_cycles, load_c_string_byte_by_byte(machine.inner_mut().memory_mut(), &arg_addr));
            total_length = total_length
                .checked_add(8)
                .and_then(|v| v.checked_add(arg.len() as u64))
                .ok_or_else(|| ScriptError::VM(VMError::ExecutionError("argv length overflow".to_string())))?;
            if total_length > MAX_ARGV_LENGTH {
                return Err(ScriptError::VM(VMError::ExecutionError("argv length exceeds MAX_ARGV_LENGTH".to_string())));
            }
            args.push(arg);
            argv_addr = argv_addr
                .checked_add(8)
                .ok_or_else(|| ScriptError::VM(VMError::ExecutionError("argv pointer overflow".to_string())))?;
        }
        Ok(args)
    }

    fn i8_to_reg(v: i8) -> <MachineInner as CoreMachine>::REG {
        <MachineInner as CoreMachine>::REG::from_i8(v)
    }

    fn u8_to_reg(v: u8) -> <MachineInner as CoreMachine>::REG {
        <MachineInner as CoreMachine>::REG::from_u8(v)
    }

    fn u64_to_reg(v: u64) -> <MachineInner as CoreMachine>::REG {
        <MachineInner as CoreMachine>::REG::from_u64(v)
    }
}

pub fn slice_program(payload: &[u8], offset: usize, length: usize) -> Result<&[u8], u8> {
    use super::syscalls::SLICE_OUT_OF_BOUND;

    if offset >= payload.len() {
        return Err(SLICE_OUT_OF_BOUND);
    }
    if length == 0 {
        return Ok(&payload[offset..]);
    }
    let end = offset.checked_add(length).ok_or(SLICE_OUT_OF_BOUND)?;
    if end > payload.len() {
        return Err(SLICE_OUT_OF_BOUND);
    }
    Ok(&payload[offset..end])
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::scripts::ALWAYS_SUCCESS_SCRIPT;

    fn scheduler() -> VmScheduler {
        VmScheduler::new(
            ScriptVersion::V2,
            10_000_000,
            4 * 1024 * 1024,
            1024 * 1024,
            ALWAYS_SUCCESS_SCRIPT.to_vec(),
            vec![],
            Arc::new(|_, _| Ok(Vec::new())),
            Arc::new(|_| Ok(ALWAYS_SUCCESS_SCRIPT.to_vec())),
        )
    }

    fn sample_location(index: usize) -> ProgramLocation {
        ProgramLocation {
            piece: ProgramPiece { source: super::super::syscalls::Source::CellDep, index, place: ProgramPlace::CellData },
            offset: 0,
            length: 0,
        }
    }

    #[test]
    fn test_boot_vm_suspends_oldest_when_instantiated_limit_reached() {
        let mut scheduler = scheduler();
        scheduler.boot_root_vm_if_needed().expect("root VM should boot");

        for i in 0..MAX_INSTANTIATED_VMS {
            scheduler.boot_vm(sample_location(i), VmArgs::Vector(vec![])).expect("child VM should boot");
        }

        assert_eq!(scheduler.states.len(), MAX_INSTANTIATED_VMS + 1);
        assert_eq!(scheduler.instantiated.len(), MAX_INSTANTIATED_VMS);
        assert_eq!(scheduler.suspended.len(), 1);
        assert!(scheduler.suspended.contains_key(&ROOT_VM_ID));
    }

    #[test]
    fn test_ensure_vms_instantiated_resumes_suspended_vm() {
        let mut scheduler = scheduler();
        scheduler.boot_root_vm_if_needed().expect("root VM should boot");

        for i in 0..MAX_INSTANTIATED_VMS {
            scheduler.boot_vm(sample_location(i), VmArgs::Vector(vec![])).expect("child VM should boot");
        }

        scheduler.ensure_vms_instantiated(&[ROOT_VM_ID]).expect("root VM should resume");

        assert!(scheduler.instantiated.contains_key(&ROOT_VM_ID));
        assert_eq!(scheduler.instantiated.len(), MAX_INSTANTIATED_VMS);
        assert_eq!(scheduler.suspended.len(), 1);
    }

    #[test]
    fn test_run_with_pause_interrupts_before_execution() {
        let mut scheduler = scheduler();
        let pause = Pause::new();
        pause.interrupt();

        let err = scheduler.run_with_mode(RunMode::Pause(pause, 10_000_000)).expect_err("paused scheduler should return an error");

        assert!(matches!(err, ScriptError::VM(VMError::Paused)));
    }

    #[test]
    fn test_suspend_and_resume_roundtrip_vm_state() {
        let mut scheduler = scheduler();
        scheduler.boot_root_vm_if_needed().expect("root VM should boot");

        for i in 0..MAX_INSTANTIATED_VMS {
            scheduler.boot_vm(sample_location(i), VmArgs::Vector(vec![])).expect("child VM should boot");
        }

        let total_states = scheduler.states.len();
        let instantiated_count = scheduler.instantiated.len();
        let suspended_state = scheduler.suspend().expect("scheduler should suspend");
        assert_eq!(suspended_state.vms.len(), total_states);
        assert_eq!(suspended_state.instantiated_ids.len(), instantiated_count);
        assert!(suspended_state.size() > 0);

        let resumed = VmScheduler::resume(
            ScriptVersion::V2,
            10_000_000,
            4 * 1024 * 1024,
            1024 * 1024,
            ALWAYS_SUCCESS_SCRIPT.to_vec(),
            vec![],
            Arc::new(|_, _| Ok(Vec::new())),
            Arc::new(|_| Ok(ALWAYS_SUCCESS_SCRIPT.to_vec())),
            suspended_state,
        )
        .expect("scheduler should resume");

        assert_eq!(resumed.states.len(), total_states);
        assert_eq!(resumed.instantiated.len(), instantiated_count);
    }
}
