// SPDX-License-Identifier: MIT
// Copyright (C) 2026 Spora developers
//
// VM cost model for cycles calculation
// Adapted from CKB script/src/cost_model.rs

/// Cycle type
pub type Cycle = u64;

/// Transferred byte cycles calculation
///
/// Cost model: 0.5 cycle per byte
pub fn transferred_byte_cycles(bytes: usize) -> Cycle {
    // 0.5 cycles per byte = bytes / 2
    (bytes / 2) as Cycle
}

/// Instruction cycles cost
pub const INSTRUCTION_CYCLES: Cycle = 1;

/// Memory page allocation cost
pub const MEMORY_PAGE_CYCLES: Cycle = 1024;

/// System call base cost
pub const SYSCALL_BASE_CYCLES: Cycle = 500;

/// Calculate cycles for memory operations
pub fn memory_cycles(pages: usize) -> Cycle {
    pages as Cycle * MEMORY_PAGE_CYCLES
}

/// Calculate cycles for a system call
pub fn syscall_cycles(transferred_bytes: usize) -> Cycle {
    SYSCALL_BASE_CYCLES + transferred_byte_cycles(transferred_bytes)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_transferred_byte_cycles() {
        assert_eq!(transferred_byte_cycles(0), 0);
        assert_eq!(transferred_byte_cycles(100), 50);
        assert_eq!(transferred_byte_cycles(1000), 500);
    }

    #[test]
    fn test_memory_cycles() {
        assert_eq!(memory_cycles(1), 1024);
        assert_eq!(memory_cycles(10), 10240);
    }

    #[test]
    fn test_syscall_cycles() {
        assert_eq!(syscall_cycles(0), 500);
        assert_eq!(syscall_cycles(100), 550);
    }
}
