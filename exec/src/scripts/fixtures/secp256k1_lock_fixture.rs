#![no_std]
#![no_main]

use core::arch::asm;

const SYS_EXIT: usize = 93;
const SYS_LOAD_WITNESS: usize = 2074;
const SYS_LOAD_SCRIPT: usize = 2075;
const SYS_SECP256K1_VERIFY: usize = 3002;
const SYS_LOAD_ECDSA_SIGNATURE_HASH: usize = 3004;
const SOURCE_GROUP_INPUT: usize = 0x0100;
const SIG_HASH_ALL: u8 = 0x01;

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

#[inline(always)]
fn read_u32_le(bytes: &[u8]) -> usize {
    (bytes[0] as usize)
        | ((bytes[1] as usize) << 8)
        | ((bytes[2] as usize) << 16)
        | ((bytes[3] as usize) << 24)
}

#[unsafe(no_mangle)]
pub extern "C" fn _start() -> ! {
    let mut script = [0u8; 64];
    let mut script_len = script.len() as u64;
    let load_script_ret: usize;
    unsafe {
        asm!(
            "ecall",
            inlateout("a0") script.as_mut_ptr() as usize => load_script_ret,
            in("a1") (&mut script_len as *mut u64) as usize,
            in("a2") 0usize,
            in("a7") SYS_LOAD_SCRIPT,
        );
    }
    if load_script_ret != 0 || script_len < 57 {
        exit(1);
    }

    let args_len = read_u32_le(&script[33..37]);
    if args_len != 20 || 37 + args_len > script_len as usize {
        exit(1);
    }

    let pubkey_hash = &script[37..57];

    let mut witness = [0u8; 66];
    let mut witness_len = witness.len() as u64;
    let load_witness_ret: usize;
    unsafe {
        asm!(
            "ecall",
            inlateout("a0") witness.as_mut_ptr() as usize => load_witness_ret,
            in("a1") (&mut witness_len as *mut u64) as usize,
            in("a2") 0usize,
            in("a3") 0usize,
            in("a4") SOURCE_GROUP_INPUT,
            in("a7") SYS_LOAD_WITNESS,
        );
    }
    if load_witness_ret != 0 || (witness_len != 65 && witness_len != 66) {
        exit(1);
    }

    let hash_type = if witness_len == 66 { witness[65] } else { SIG_HASH_ALL };
    let mut sighash = [0u8; 32];
    let mut sighash_len = sighash.len() as u64;
    let load_sighash_ret: usize;
    unsafe {
        asm!(
            "ecall",
            inlateout("a0") sighash.as_mut_ptr() as usize => load_sighash_ret,
            in("a1") (&mut sighash_len as *mut u64) as usize,
            in("a2") 0usize,
            in("a3") 0usize,
            in("a4") SOURCE_GROUP_INPUT,
            in("a5") hash_type as usize,
            in("a7") SYS_LOAD_ECDSA_SIGNATURE_HASH,
        );
    }
    if load_sighash_ret != 0 || sighash_len != 32 {
        exit(1);
    }

    let verify_ret: usize;
    unsafe {
        asm!(
            "ecall",
            inlateout("a0") pubkey_hash.as_ptr() as usize => verify_ret,
            in("a1") witness.as_ptr() as usize,
            in("a2") sighash.as_ptr() as usize,
            in("a7") SYS_SECP256K1_VERIFY,
        );
    }

    exit(if verify_ret == 0 { 0 } else { 1 });
}
