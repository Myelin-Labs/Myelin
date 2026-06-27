// SPDX-License-Identifier: MIT
// Copyright (C) 2026 Myelin developers
//
// myelin-core-utils: the small surface that hot-path crates
// (myelin-hashes, myelin-math) actually need.
//
// This crate is intentionally narrow. It carries:
//   - hex      (deterministic hex codec for Myelin's 32-byte hashes)
//   - mem_size (the cell-accounting memory estimator trait)
//   - serde_bytes, serde_bytes_fixed, serde_bytes_fixed_ref
//              (serde helpers used by the kernel types)
//
// Anything heavier is historical code outside the active kernel workspace. The
// hot-path crate graph pulls only from this crate.

//! Myelin core utility surface (hot-path crate boundary).
//!
//! The hot-path crates (`myelin-hashes`, `myelin-math`) depend on this crate so
//! they do not pull in broad legacy helper surfaces.

pub mod hex;
pub mod mem_size;
pub mod serde_bytes;
pub mod serde_bytes_fixed;
pub mod serde_bytes_fixed_ref;
