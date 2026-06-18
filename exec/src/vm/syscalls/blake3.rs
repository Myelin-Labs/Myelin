// SPDX-License-Identifier: MIT
// Copyright (C) 2026 Spora developers
//
// Blake3 hash syscall (Spora-specific extension to CKB-VM)
//
// This syscall is NOT in CKB, it's our addition for Spora

use super::BLAKE3_HASH_SYSCALL_NUMBER;
use crate::vm::transferred_byte_cycles;
use ckb_vm::{
    registers::{A0, A2, A3, A7},
    Error as VMError, Memory, Register, SupportMachine, Syscalls,
};

/// Syscall: Blake3 Hash
///
/// Syscall number: 3001 (Spora extension, not in CKB)
///
/// Computes blake3 hash of input data
///
/// Args:
/// - A0: output buffer address (store_data will read this)
/// - A1: output length ptr (store_data will read this)
/// - A2: input data address
/// - A3: input data length
///
/// Returns:
/// - A0: SUCCESS (0)
pub struct Blake3Hash;

impl Blake3Hash {
    pub fn new() -> Self {
        Self
    }
}

impl<M: SupportMachine> Syscalls<M> for Blake3Hash {
    fn initialize(&mut self, _machine: &mut M) -> Result<(), VMError> {
        Ok(())
    }

    fn ecall(&mut self, machine: &mut M) -> Result<bool, VMError> {
        let syscall_number = machine.registers()[A7].to_u64();

        // BLAKE3_HASH = 3001 (Spora extension)
        if syscall_number != BLAKE3_HASH_SYSCALL_NUMBER {
            return Ok(false);
        }

        // Args
        let input_addr = machine.registers()[A2].to_u64();
        let input_len = machine.registers()[A3].to_u64() as usize;

        // Read input data from VM memory
        let input_data = machine.memory_mut().load_bytes(input_addr, input_len as u64)?;

        // Compute blake3 hash
        let hash = blake3::hash(&input_data);

        // Store hash using standard store_data
        // Note: We need to temporarily save A2 since store_data uses it for offset
        // For this syscall, we set A2=0 (no offset for hash output)
        let saved_a2 = machine.registers()[A2].clone();
        machine.set_register(A2, M::REG::from_u64(0));

        // Use internal store mechanism
        let result = super::utils::store_data(machine, hash.as_bytes())?;

        // Restore A2
        machine.set_register(A2, saved_a2);
        machine.add_cycles_no_checking(transferred_byte_cycles(result.written_size))?;
        machine.set_register(A0, M::REG::from_u8(result.return_code));

        Ok(true)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_blake3_hash_creation() {
        let _syscall = Blake3Hash::new();
        // Just ensure it compiles
    }

    #[test]
    fn test_blake3_known_vector() {
        // Test blake3 against known vector
        let data = b"hello world";
        let hash = blake3::hash(data);
        assert_eq!(hash.as_bytes().len(), 32);

        // Known blake3("hello world")
        let expected = "d74981efa70a0c880b8d8c1985d075dbcbf679b99a5f9914e5aaf96b831a9e24";
        assert_eq!(hex::encode(hash.as_bytes()), expected);
    }
}
