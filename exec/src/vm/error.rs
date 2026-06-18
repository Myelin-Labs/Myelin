// SPDX-License-Identifier: MIT
// Copyright (C) 2026 Spora developers
//
// VM error types

use thiserror::Error;

/// VM execution errors
#[derive(Error, Debug)]
pub enum VMError {
    /// VM backend is wired into the call path but the actual executor is not implemented yet.
    #[error("VM backend not implemented: {0}")]
    BackendUnimplemented(String),

    /// Failed to load program
    #[error("Failed to load program: {0}")]
    LoadProgramError(String),

    /// VM execution error
    #[error("VM execution error: {0}")]
    ExecutionError(String),

    /// VM execution paused by an external pause signal
    #[error("VM execution paused")]
    Paused,

    /// Script exited with non-zero code
    #[error("Script exited with code {0}")]
    NonZeroExitCode(i8),

    /// Cycles limit exceeded
    #[error("Cycles exceeded: limit={limit}, actual={actual}")]
    CyclesExceeded { limit: u64, actual: u64 },

    /// Script binary exceeds the configured size limit
    #[error("Script too large: size={size}, limit={limit}")]
    ScriptTooLarge { size: usize, limit: usize },

    /// Invalid syscall number
    #[error("Invalid syscall number: {0}")]
    InvalidSyscall(u64),

    /// Syscall error
    #[error("Syscall error: {0}")]
    SyscallError(String),

    /// Memory access error
    #[error("Memory access error: {0}")]
    MemoryError(String),

    /// Index out of bounds
    #[error("Index out of bounds: index={index}, max={max}")]
    IndexOutOfBounds { index: usize, max: usize },

    /// Item missing
    #[error("Item missing: {0}")]
    ItemMissing(String),

    /// Invalid data
    #[error("Invalid data: {0}")]
    InvalidData(String),
}

/// Script execution error
#[derive(Error, Debug)]
pub enum ScriptError {
    /// VM error
    #[error("VM error: {0}")]
    VM(#[from] VMError),

    /// Script not found
    #[error("Script code not found: {0:?}")]
    ScriptNotFound([u8; 32]),

    /// Lock script verification failed
    #[error("Lock script verification failed")]
    LockScriptFailed,

    /// Type script verification failed
    #[error("Type script verification failed")]
    TypeScriptFailed,

    /// Invalid script hash type
    #[error("Invalid script hash type: {0}")]
    InvalidHashType(u8),
}

/// Result type for VM operations
pub type VMResult<T> = Result<T, VMError>;

/// Result type for script operations
pub type ScriptResult<T> = Result<T, ScriptError>;
