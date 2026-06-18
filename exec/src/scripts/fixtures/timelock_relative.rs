#![no_std]
#![no_main]

//! Relative DAA score lock script fixture
//!
//! This script verifies that the input's `since` field (as relative DAA)
//! is >= the target delta baked into the script.
//!
//! Expected since format:
//! - bit63 = 1 (relative)
//! - bit62 = 0 (DAA score, not timestamp)
//! - bits 0-55 = delta blocks (>= TARGET_DELTA)

use core::arch::asm;

/// Target delta: 100 blocks
const TARGET_DELTA: u64 = 100;

#[panic_handler]
fn panic(_: &core::panic::PanicInfo) -> ! {
    loop {}
}

#[inline(always)]
fn exit(code: usize) -> ! {
    unsafe {
        asm!(
            "ecall",
            in("a0") code,
            in("a7") 93usize,
            options(noreturn)
        );
    }
}

/// Load input since field via syscall 2083 (LOAD_INPUT_BY_FIELD)
/// Source: 0x0100 (GROUP_INPUT), Field: 0x01 (SINCE)
fn load_input_since() -> u64 {
    let mut buf = [0u8; 8];
    let mut size = buf.len() as u64;
    let ret: usize;

    unsafe {
        asm!(
            "ecall",
            inlateout("a0") buf.as_mut_ptr() as usize => ret,
            in("a1") (&mut size as *mut u64) as usize,
            in("a2") 0usize,      // offset
            in("a3") 0usize,      // index
            in("a4") 0x0100usize, // source: GROUP_INPUT
            in("a5") 0x01usize,   // field: SINCE
            in("a7") 2083usize,   // syscall: LOAD_INPUT_BY_FIELD
        );
    }

    if ret != 0 || size != 8 {
        exit(1);
    }

    // Parse little-endian u64
    let mut since: u64 = 0;
    for i in 0..8 {
        since |= (buf[i] as u64) << (8 * i);
    }
    since
}

#[unsafe(no_mangle)]
pub extern "C" fn _start() -> ! {
    // Load the since value from input
    let since = load_input_since();

    // Check flags:
    // - bit63 must be 1 (relative lock)
    // - bit62 must be 0 (DAA score, not timestamp)
    let is_relative = (since & (1u64 << 63)) != 0;
    let is_timestamp = (since & (1u64 << 62)) != 0;

    if !is_relative || is_timestamp {
        exit(2); // Wrong lock type
    }

    // Extract delta value (bits 0-55)
    let delta = since & 0x00FFFFFFFFFFFFFF;

    // Verify delta >= target
    if delta < TARGET_DELTA {
        exit(3); // Relative lock not satisfied
    }

    // Success!
    exit(0);
}
