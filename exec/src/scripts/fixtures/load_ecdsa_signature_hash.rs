#![no_std]
#![no_main]

use core::arch::asm;

const SYS_EXIT: usize = 93;
const SYS_LOAD_WITNESS: usize = 2074;
const SYS_LOAD_ECDSA_SIGNATURE_HASH: usize = 3004;
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
    let mut witness = [0u8; 64];
    let mut witness_len = witness.len() as u64;
    let witness_ret: usize;

    unsafe {
        asm!(
            "ecall",
            inlateout("a0") witness.as_mut_ptr() as usize => witness_ret,
            in("a1") (&mut witness_len as *mut u64) as usize,
            in("a2") 0usize,
            in("a3") 0usize,
            in("a4") SOURCE_GROUP_INPUT,
            in("a7") SYS_LOAD_WITNESS,
        );
    }

    if witness_ret != 0 || witness_len < 33 {
        exit(1);
    }

    let mut sighash = [0u8; 32];
    let mut sighash_len = sighash.len() as u64;
    let sighash_ret: usize;
    unsafe {
        asm!(
            "ecall",
            inlateout("a0") sighash.as_mut_ptr() as usize => sighash_ret,
            in("a1") (&mut sighash_len as *mut u64) as usize,
            in("a2") 0usize,
            in("a3") 0usize,
            in("a4") SOURCE_GROUP_INPUT,
            in("a5") witness[32] as usize,
            in("a7") SYS_LOAD_ECDSA_SIGNATURE_HASH,
        );
    }

    if sighash_ret != 0 || sighash_len != 32 {
        exit(1);
    }

    let mut idx = 0usize;
    while idx < 32 {
        if witness[idx] != sighash[idx] {
            exit(1);
        }
        idx += 1;
    }

    exit(0);
}
