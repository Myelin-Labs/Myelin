// SPDX-License-Identifier: MIT
// Copyright (C) 2026 Myelin developers
//
// Syscall utility functions
// Reference: ckb/script/src/syscalls/utils.rs

use ckb_vm::{
    registers::{A0, A1, A2},
    Error as VMError, Memory, Register, SupportMachine,
};
use std::cmp;

/// Success return code
pub const SUCCESS: u8 = 0;
/// Index out of bound
pub const INDEX_OUT_OF_BOUND: u8 = 1;
/// Item missing
pub const ITEM_MISSING: u8 = 2;
/// Slice out of bound (offset exceeds available data)
pub const SLICE_OUT_OF_BOUND: u8 = 3;
/// Wrong format
pub const WRONG_FORMAT: u8 = 4;

/// Result of copying data into VM memory.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct StoreData {
    /// Syscall-style return code.
    pub return_code: u8,
    /// Actual number of bytes copied into guest memory.
    pub written_size: usize,
}

/// Store data to VM memory (CKB-aligned implementation)
///
/// This follows CKB's exact pattern:
/// offset is clamped to data length and never returns a slice-out-of-bound error.
pub fn store_data<Mac: SupportMachine>(machine: &mut Mac, data: &[u8]) -> Result<StoreData, VMError> {
    let addr = machine.registers()[A0].to_u64();
    let size_addr = machine.registers()[A1].clone();
    let data_len = data.len() as u64;
    let offset = cmp::min(data_len, machine.registers()[A2].to_u64());

    let size = machine.memory_mut().load64(&size_addr)?.to_u64();
    let full_size = data_len - offset;
    let real_size = cmp::min(size, full_size);
    machine.memory_mut().store64(&size_addr, &Mac::REG::from_u64(full_size))?;
    machine.memory_mut().store_bytes(addr, &data[offset as usize..(offset + real_size) as usize])?;
    Ok(StoreData { return_code: SUCCESS, written_size: real_size as usize })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::vm::ScriptVersion;
    use ckb_vm::{registers::A7, CoreMachine, Memory, Register};

    const BUFFER_ADDR: u64 = 0x1000;
    const SIZE_ADDR: u64 = 0x2000;

    #[test]
    fn test_return_codes() {
        assert_eq!(SUCCESS, 0);
        assert_eq!(INDEX_OUT_OF_BOUND, 1);
        assert_eq!(ITEM_MISSING, 2);
        assert_eq!(SLICE_OUT_OF_BOUND, 3);
        assert_eq!(WRONG_FORMAT, 4);
    }

    #[test]
    fn test_store_data_clamps_offset_past_end() {
        let mut machine = ScriptVersion::V2.init_core_machine(10_000);
        machine.memory_mut().store64(&SIZE_ADDR, &8u64).unwrap();
        machine.set_register(A0, BUFFER_ADDR);
        machine.set_register(A1, SIZE_ADDR);
        machine.set_register(A2, 5);
        machine.set_register(A7, 0);

        let result = store_data(&mut machine, &[1, 2, 3, 4]).expect("store_data should not trap");

        assert_eq!(result.return_code, SUCCESS);
        assert_eq!(result.written_size, 0);
        assert_eq!(machine.memory_mut().load64(&SIZE_ADDR).unwrap().to_u64(), 0);
    }

    #[test]
    fn test_store_data_allows_offset_at_end() {
        let mut machine = ScriptVersion::V2.init_core_machine(10_000);
        machine.memory_mut().store64(&SIZE_ADDR, &8u64).unwrap();
        machine.set_register(A0, BUFFER_ADDR);
        machine.set_register(A1, SIZE_ADDR);
        machine.set_register(A2, 4);

        let result = store_data(&mut machine, &[1, 2, 3, 4]).expect("store_data should not trap");

        assert_eq!(result.return_code, SUCCESS);
        assert_eq!(result.written_size, 0);
        assert_eq!(machine.memory_mut().load64(&SIZE_ADDR).unwrap().to_u64(), 0);
    }
}
