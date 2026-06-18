#![no_std]
#![no_main]

//! Minimal HTLC test script - only tests witness loading

use core::arch::asm;

const SYS_EXIT: usize = 93;
const SYS_LOAD_WITNESS: usize = 2074;
const SOURCE_GROUP_INPUT: usize = 0x0100;

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
            in("a7") SYS_EXIT,
            options(noreturn)
        );
    }
}

#[unsafe(no_mangle)]
pub extern "C" fn _start() -> ! {
    let mut buf = [0u8; 128];
    let mut size = buf.len() as u64;
    let ret: usize;

    // Try to load witness from index 0
    unsafe {
        asm!(
            "ecall",
            inlateout("a0") buf.as_mut_ptr() as usize => ret,
            in("a1") (&mut size as *mut u64) as usize,
            in("a2") 0usize,      // offset
            in("a3") 0usize,      // index
            in("a4") SOURCE_GROUP_INPUT,
            in("a7") SYS_LOAD_WITNESS,
        );
    }

    // If we can load witness (any size), exit with 0
    if ret == 0 {
        exit(0);
    } else {
        exit(1);
    }
}
