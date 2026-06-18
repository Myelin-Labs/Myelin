// SPDX-License-Identifier: MIT
// Copyright (C) 2026 Myelin developers
//
// VM Machine implementation (adapted from CKB-VM)
// Reference: ckb/script/src/types.rs

use super::{error::VMError, MAX_SCRIPT_SIZE, MAX_VM_MEMORY};
use crate::serialization::split_vm_abi_trailer;
use ckb_vm::{
    cost_model::estimate_cycles,
    machine::{VERSION0, VERSION1, VERSION2},
    Bytes, DefaultMachineBuilder, DefaultMachineRunner, SupportMachine, Syscalls, ISA_B, ISA_IMC, ISA_MOP,
};

/// CKB-VM ISA type
pub type VmIsa = u8;
/// CKB-VM version type
pub type VmVersion = u32;
/// Cycles type
pub type Cycle = u64;

/// Script version (simplified from CKB)
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub enum ScriptVersion {
    /// VM 0 with basic syscalls
    V0 = 0,
    /// VM 1 with extended syscalls (B + MOP extensions)
    V1 = 1,
    /// VM 2 (latest, full feature set)
    V2 = 2,
}

impl ScriptVersion {
    /// Returns the latest version
    pub const fn latest() -> Self {
        Self::V2
    }

    /// Returns the ISA set for this version
    pub fn vm_isa(self) -> VmIsa {
        match self {
            Self::V0 => ISA_IMC,
            Self::V1 => ISA_IMC | ISA_B | ISA_MOP,
            Self::V2 => ISA_IMC | ISA_B | ISA_MOP,
        }
    }

    /// Returns the VM version
    pub fn vm_version(self) -> VmVersion {
        match self {
            Self::V0 => VERSION0,
            Self::V1 => VERSION1,
            Self::V2 => VERSION2,
        }
    }

    /// Creates a VM core machine without cycles limit
    pub fn init_core_machine_without_limit(self) -> <Machine as DefaultMachineRunner>::Inner {
        self.init_core_machine(u64::MAX)
    }

    /// Creates a VM core machine with cycles limit
    pub fn init_core_machine(self, max_cycles: Cycle) -> <Machine as DefaultMachineRunner>::Inner {
        self.init_core_machine_with_memory(max_cycles, MAX_VM_MEMORY)
    }

    /// Creates a VM core machine with explicit memory size.
    pub fn init_core_machine_with_memory(self, max_cycles: Cycle, memory_size: usize) -> <Machine as DefaultMachineRunner>::Inner {
        let isa = self.vm_isa();
        let version = self.vm_version();
        <<Machine as DefaultMachineRunner>::Inner as SupportMachine>::new_with_memory(isa, version, max_cycles, memory_size)
    }
}

/// Default machine type
/// For simplicity, we use TraceMachine with SparseMemory
/// ASM optimization can be added later via feature flags
pub type Machine = ckb_vm::TraceMachine<ckb_vm::DefaultCoreMachine<u64, ckb_vm::WXorXMemory<ckb_vm::SparseMemory<u64>>>>;

/// VM context for execution
pub struct VmContext {
    /// Script version
    pub version: ScriptVersion,
    /// Maximum cycles
    pub max_cycles: Cycle,
    /// Maximum VM memory in bytes.
    pub max_memory: usize,
    /// Maximum script size in bytes.
    pub max_script_size: usize,
}

impl VmContext {
    /// Create a new VM context
    pub fn new(version: ScriptVersion, max_cycles: Cycle) -> Self {
        Self { version, max_cycles, max_memory: MAX_VM_MEMORY, max_script_size: MAX_SCRIPT_SIZE }
    }

    /// Create a VM context with explicit memory and script size limits.
    pub fn with_limits(version: ScriptVersion, max_cycles: Cycle, max_memory: usize, max_script_size: usize) -> Self {
        Self { version, max_cycles, max_memory, max_script_size }
    }

    /// Create a VM context with default cycles limit (10M)
    pub fn with_default_cycles(version: ScriptVersion) -> Self {
        Self::new(version, 10_000_000)
    }
}

/// Run a script with given program and syscalls
///
/// Run a script using the real CKB-VM execution loop.
///
/// This currently provides the minimal executable backend:
/// - instantiate a trace machine
/// - register the provided syscalls
/// - load the program as an ELF payload
/// - run until exit
///
/// Higher-level runtime completeness still depends on syscall coverage and script
/// fixtures being valid ELF programs.
pub fn run_script(
    program: &[u8],
    args: &[Vec<u8>],
    syscalls: Vec<Box<dyn Syscalls<<Machine as DefaultMachineRunner>::Inner>>>,
    context: &VmContext,
) -> Result<Cycle, VMError> {
    if program.len() > context.max_script_size {
        return Err(VMError::ScriptTooLarge { size: program.len(), limit: context.max_script_size });
    }

    let core_machine = context.version.init_core_machine_with_memory(context.max_cycles, context.max_memory);
    let builder = syscalls
        .into_iter()
        .fold(DefaultMachineBuilder::new(core_machine).instruction_cycle_func(Box::new(estimate_cycles)), |builder, syscall| {
            builder.syscall(syscall)
        });
    let mut machine = Machine::new(builder.build());

    let (program, _) =
        split_vm_abi_trailer(program).map_err(|err| VMError::InvalidData(format!("invalid VM ABI artifact trailer: {}", err)))?;
    let program = Bytes::copy_from_slice(program);
    let args = args.iter().cloned().map(Bytes::from).map(Ok);

    machine.load_program(&program, args).map_err(|err| VMError::LoadProgramError(err.to_string()))?;

    let exit_code = machine.run().map_err(|err| match err {
        // ckb-vm reports CyclesExceeded before committing the overflowing value
        // back into the machine, so `machine.cycles()` is the last accepted count,
        // not the attempted total. Report the minimal strictly-over-limit value
        // instead of the stale pre-overflow counter.
        ckb_vm::Error::CyclesExceeded => {
            VMError::CyclesExceeded { limit: context.max_cycles, actual: context.max_cycles.saturating_add(1) }
        }
        other => VMError::ExecutionError(other.to_string()),
    })?;

    if exit_code != 0 {
        return Err(VMError::NonZeroExitCode(exit_code));
    }

    Ok(machine.machine.cycles())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::scripts::ALWAYS_SUCCESS_SCRIPT;
    use ckb_vm::{CoreMachine, Memory};

    #[test]
    fn test_script_version_isa() {
        assert_eq!(ScriptVersion::V0.vm_isa(), ISA_IMC);
        assert_eq!(ScriptVersion::V1.vm_isa(), ISA_IMC | ISA_B | ISA_MOP);
        assert_eq!(ScriptVersion::V2.vm_isa(), ISA_IMC | ISA_B | ISA_MOP);
    }

    #[test]
    fn test_script_version_latest() {
        assert_eq!(ScriptVersion::latest(), ScriptVersion::V2);
    }

    #[test]
    fn test_vm_context_creation() {
        let ctx = VmContext::new(ScriptVersion::V2, 1_000_000);
        assert_eq!(ctx.version, ScriptVersion::V2);
        assert_eq!(ctx.max_cycles, 1_000_000);
        assert_eq!(ctx.max_memory, MAX_VM_MEMORY);
        assert_eq!(ctx.max_script_size, MAX_SCRIPT_SIZE);
    }

    #[test]
    fn test_vm_context_with_limits() {
        let ctx = VmContext::with_limits(ScriptVersion::V1, 2_000_000, 4 * 1024 * 1024, 64 * 1024);
        assert_eq!(ctx.version, ScriptVersion::V1);
        assert_eq!(ctx.max_cycles, 2_000_000);
        assert_eq!(ctx.max_memory, 4 * 1024 * 1024);
        assert_eq!(ctx.max_script_size, 64 * 1024);
    }

    #[test]
    fn test_init_core_machine_with_memory_uses_requested_limit() {
        let machine = ScriptVersion::V2.init_core_machine_with_memory(10_000, 4 * 1024 * 1024);
        assert_eq!(machine.memory().memory_size(), 4 * 1024 * 1024);
    }

    #[test]
    fn test_run_script_rejects_oversized_program_before_loading() {
        let context = VmContext::with_limits(ScriptVersion::V2, 10_000, MAX_VM_MEMORY, 16);
        let oversized_program = vec![0u8; 17];

        let result = run_script(&oversized_program, &[], vec![], &context);

        assert!(matches!(result, Err(VMError::ScriptTooLarge { size: 17, limit: 16 })));
    }

    #[test]
    fn test_run_script_counts_instruction_cycles() {
        let context = VmContext::with_limits(ScriptVersion::V2, 100_000, MAX_VM_MEMORY, MAX_SCRIPT_SIZE);

        let cycles = run_script(ALWAYS_SUCCESS_SCRIPT, &[], vec![], &context).expect("always-success script should run");

        assert!(cycles > 0, "instruction cycles should be tracked");
    }
}
