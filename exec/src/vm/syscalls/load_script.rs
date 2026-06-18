// SPDX-License-Identifier: MIT
// Copyright (C) 2026 Spora developers
//
// Load script syscall

use super::utils::store_data;
use super::{CKB_LOAD_SCRIPT_SYSCALL_NUMBER, LOAD_SCRIPT_HASH_SYSCALL_NUMBER, LOAD_SCRIPT_SYSCALL_NUMBER};
use crate::celltx::Script;
use crate::serialization::molecule_compat::{ckb_script_hash_molecule, serialize_script_molecule};
use crate::serialization::vm_abi::serialize_script;
use crate::serialization::VmAbiFormat;
use crate::vm::{transferred_byte_cycles, VmSemantics};
use ckb_vm::{
    registers::{A0, A7},
    Error as VMError, Register, SupportMachine, Syscalls,
};
use std::sync::Arc;

/// Syscall: Load Script
///
/// Syscall number: 2075 under Spora semantics, 2052 under CKB strict semantics.
///
/// Loads the current script being executed
pub struct LoadScript {
    script: Arc<Script>,
    abi_format: VmAbiFormat,
    semantics: VmSemantics,
}

impl LoadScript {
    pub fn new(script: Arc<Script>) -> Self {
        Self { script, abi_format: VmAbiFormat::Molecule, semantics: VmSemantics::SporaExtended }
    }

    /// Select the VM ABI wire format used by full script loads.
    pub fn with_abi_format(mut self, abi_format: VmAbiFormat) -> Self {
        self.abi_format = abi_format;
        self
    }

    /// Select the hash/syscall semantics used by hash field loads.
    pub fn with_semantics(mut self, semantics: VmSemantics) -> Self {
        self.semantics = semantics;
        self
    }

    fn serialize_script(&self) -> Result<Vec<u8>, VMError> {
        match self.abi_format {
            VmAbiFormat::Legacy => Ok(serialize_script(&self.script)),
            VmAbiFormat::Molecule => serialize_script_molecule(&self.script).map_err(|e| VMError::External(e.to_string())),
        }
    }

    fn script_hash(&self) -> Result<[u8; 32], VMError> {
        match self.semantics {
            VmSemantics::SporaExtended => Ok(self.script.hash()),
            VmSemantics::CkbStrict => ckb_script_hash_molecule(&self.script).map_err(|e| VMError::External(e.to_string())),
        }
    }

    fn load_script_syscall_number(&self) -> u64 {
        match self.semantics {
            VmSemantics::SporaExtended => LOAD_SCRIPT_SYSCALL_NUMBER,
            VmSemantics::CkbStrict => CKB_LOAD_SCRIPT_SYSCALL_NUMBER,
        }
    }
}

impl<M: SupportMachine> Syscalls<M> for LoadScript {
    fn initialize(&mut self, _machine: &mut M) -> Result<(), VMError> {
        Ok(())
    }

    fn ecall(&mut self, machine: &mut M) -> Result<bool, VMError> {
        let syscall_number = machine.registers()[A7].to_u64();

        // LOAD_SCRIPT is profile-specific; LOAD_SCRIPT_HASH keeps the shared CKB number.
        let load_script_syscall_number = self.load_script_syscall_number();
        if syscall_number != load_script_syscall_number && syscall_number != LOAD_SCRIPT_HASH_SYSCALL_NUMBER {
            return Ok(false);
        }

        let data = if syscall_number == LOAD_SCRIPT_HASH_SYSCALL_NUMBER {
            // LOAD_SCRIPT_HASH
            self.script_hash()?.to_vec()
        } else {
            // LOAD_SCRIPT (full script)
            self.serialize_script()?
        };

        // Store data using CKB-style store_data
        let result = store_data(machine, &data)?;
        machine.add_cycles_no_checking(transferred_byte_cycles(result.written_size))?;
        machine.set_register(A0, M::REG::from_u8(result.return_code));

        Ok(true)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::serialization::molecule_compat::{ckb_script_hash_molecule, serialize_script_molecule};
    use crate::serialization::VmAbiFormat;
    use crate::vm::syscalls::SUCCESS;
    use crate::vm::{ScriptVersion, VmSemantics};
    use ckb_vm::{
        registers::{A1, A2},
        CoreMachine, Memory, Register,
    };

    const BUFFER_ADDR: u64 = 0x1000;
    const SIZE_ADDR: u64 = 0x2000;

    #[test]
    fn test_load_script_supports_partial_reads() {
        let script = Arc::new(Script::new([0xAA; 32], 1, vec![0x10, 0x20, 0x30]));
        let mut machine = ScriptVersion::V2.init_core_machine(10_000);
        machine.memory_mut().store64(&SIZE_ADDR, &7u64).unwrap();
        machine.set_register(A0, BUFFER_ADDR);
        machine.set_register(A1, SIZE_ADDR);
        machine.set_register(A2, 33);
        machine.set_register(A7, LOAD_SCRIPT_SYSCALL_NUMBER);

        let mut syscall = LoadScript::new(script).with_abi_format(VmAbiFormat::Legacy);
        let handled = syscall.ecall(&mut machine).expect("load script syscall should succeed");

        assert!(handled);
        assert_eq!(machine.registers()[A0].to_u64(), SUCCESS as u64);
        assert_eq!(machine.memory_mut().load64(&SIZE_ADDR).unwrap().to_u64(), 7);
        assert_eq!(machine.memory_mut().load_bytes(BUFFER_ADDR, 7).unwrap().as_ref(), &[3, 0, 0, 0, 0x10, 0x20, 0x30]);
    }

    #[test]
    fn test_load_script_molecule_abi_full_load() {
        let script = Arc::new(Script::new([0xAA; 32], 1, vec![0x10, 0x20, 0x30]));
        let expected = serialize_script_molecule(&script).unwrap();
        let mut machine = ScriptVersion::V2.init_core_machine(10_000);
        machine.memory_mut().store64(&SIZE_ADDR, &(expected.len() as u64)).unwrap();
        machine.set_register(A0, BUFFER_ADDR);
        machine.set_register(A1, SIZE_ADDR);
        machine.set_register(A2, 0);
        machine.set_register(A7, LOAD_SCRIPT_SYSCALL_NUMBER);

        let mut syscall = LoadScript::new(script).with_abi_format(VmAbiFormat::Molecule);
        let handled = syscall.ecall(&mut machine).expect("load script syscall should succeed");

        assert!(handled);
        assert_eq!(machine.registers()[A0].to_u64(), SUCCESS as u64);
        assert_eq!(machine.memory_mut().load64(&SIZE_ADDR).unwrap().to_u64(), expected.len() as u64);
        assert_eq!(machine.memory_mut().load_bytes(BUFFER_ADDR, expected.len() as u64).unwrap().as_ref(), expected.as_slice());
    }

    #[test]
    fn test_load_script_ckb_strict_uses_ckb_syscall_number() {
        let script = Arc::new(Script::new([0xAA; 32], 1, vec![0x10, 0x20, 0x30]));
        let expected = serialize_script_molecule(&script).unwrap();
        let mut machine = ScriptVersion::V2.init_core_machine(10_000);
        machine.memory_mut().store64(&SIZE_ADDR, &(expected.len() as u64)).unwrap();
        machine.set_register(A0, BUFFER_ADDR);
        machine.set_register(A1, SIZE_ADDR);
        machine.set_register(A2, 0);
        machine.set_register(A7, CKB_LOAD_SCRIPT_SYSCALL_NUMBER);

        let mut syscall = LoadScript::new(script).with_abi_format(VmAbiFormat::Molecule).with_semantics(VmSemantics::CkbStrict);
        let handled = syscall.ecall(&mut machine).expect("ckb load script syscall should succeed");

        assert!(handled);
        assert_eq!(machine.registers()[A0].to_u64(), SUCCESS as u64);
        assert_eq!(machine.memory_mut().load64(&SIZE_ADDR).unwrap().to_u64(), expected.len() as u64);
        assert_eq!(machine.memory_mut().load_bytes(BUFFER_ADDR, expected.len() as u64).unwrap().as_ref(), expected.as_slice());
    }

    #[test]
    fn test_load_script_syscall_number_is_profile_strict() {
        let script = Arc::new(Script::new([0xAA; 32], 1, vec![0x10, 0x20, 0x30]));

        let mut ckb_machine = ScriptVersion::V2.init_core_machine(10_000);
        ckb_machine.set_register(A7, LOAD_SCRIPT_SYSCALL_NUMBER);
        let mut ckb_syscall = LoadScript::new(Arc::clone(&script)).with_semantics(VmSemantics::CkbStrict);
        assert!(!ckb_syscall.ecall(&mut ckb_machine).expect("spora load_script number should be unhandled under ckb strict"));

        let mut spora_machine = ScriptVersion::V2.init_core_machine(10_000);
        spora_machine.set_register(A7, CKB_LOAD_SCRIPT_SYSCALL_NUMBER);
        let mut spora_syscall = LoadScript::new(script).with_semantics(VmSemantics::SporaExtended);
        assert!(!spora_syscall.ecall(&mut spora_machine).expect("ckb load_script number should be unhandled under spora semantics"));
    }

    #[test]
    fn test_load_script_hash_supports_partial_reads() {
        let script = Arc::new(Script::new([0xAA; 32], 1, vec![0x10, 0x20, 0x30]));
        let expected_hash = script.hash();
        let mut machine = ScriptVersion::V2.init_core_machine(10_000);
        machine.memory_mut().store64(&SIZE_ADDR, &6u64).unwrap();
        machine.set_register(A0, BUFFER_ADDR);
        machine.set_register(A1, SIZE_ADDR);
        machine.set_register(A2, 10);
        machine.set_register(A7, LOAD_SCRIPT_HASH_SYSCALL_NUMBER);

        let mut syscall = LoadScript::new(script);
        let handled = syscall.ecall(&mut machine).expect("load script hash syscall should succeed");

        assert!(handled);
        assert_eq!(machine.registers()[A0].to_u64(), SUCCESS as u64);
        assert_eq!(machine.memory_mut().load64(&SIZE_ADDR).unwrap().to_u64(), 22);
        assert_eq!(machine.memory_mut().load_bytes(BUFFER_ADDR, 6).unwrap().as_ref(), &expected_hash[10..16]);
    }

    #[test]
    fn test_load_script_hash_uses_ckb_hash_under_ckb_strict_semantics() {
        let script = Arc::new(Script::new([0xAA; 32], 1, vec![0x10, 0x20, 0x30]));
        let expected_hash = ckb_script_hash_molecule(&script).unwrap();
        assert_ne!(expected_hash, script.hash());
        let mut machine = ScriptVersion::V2.init_core_machine(10_000);
        machine.memory_mut().store64(&SIZE_ADDR, &32u64).unwrap();
        machine.set_register(A0, BUFFER_ADDR);
        machine.set_register(A1, SIZE_ADDR);
        machine.set_register(A2, 0);
        machine.set_register(A7, LOAD_SCRIPT_HASH_SYSCALL_NUMBER);

        let mut syscall = LoadScript::new(script).with_semantics(VmSemantics::CkbStrict);
        let handled = syscall.ecall(&mut machine).expect("load script hash syscall should succeed");

        assert!(handled);
        assert_eq!(machine.registers()[A0].to_u64(), SUCCESS as u64);
        assert_eq!(machine.memory_mut().load64(&SIZE_ADDR).unwrap().to_u64(), 32);
        assert_eq!(machine.memory_mut().load_bytes(BUFFER_ADDR, 32).unwrap().as_ref(), expected_hash.as_ref());
    }
}
