#![no_std]
#![no_main]

use core::arch::asm;

const SYS_EXIT: usize = 93;
const SYS_LOAD_WITNESS: usize = 2074;
const SYS_LOAD_SCRIPT: usize = 2075;
const SYS_BLAKE3_HASH: usize = 3001;
const SYS_SECP256K1_VERIFY: usize = 3002;
const SYS_LOAD_ECDSA_SIGNATURE_HASH: usize = 3004;
const SOURCE_GROUP_INPUT: usize = 0x0100;
const SIG_HASH_ALL: u8 = 0x01;

const SUCCESS: usize = 0;
const INDEX_OUT_OF_BOUND: usize = 1;

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

#[inline(always)]
fn load_script(buf: &mut [u8], len: &mut u64) -> usize {
    let ret: usize;
    unsafe {
        asm!(
            "ecall",
            inlateout("a0") buf.as_mut_ptr() as usize => ret,
            in("a1") len as *mut u64 as usize,
            in("a2") 0usize,
            in("a7") SYS_LOAD_SCRIPT,
        );
    }
    ret
}

#[inline(always)]
fn load_witness(buf: &mut [u8], len: &mut u64, index: usize) -> usize {
    let ret: usize;
    unsafe {
        asm!(
            "ecall",
            inlateout("a0") buf.as_mut_ptr() as usize => ret,
            in("a1") len as *mut u64 as usize,
            in("a2") 0usize,
            in("a3") index,
            in("a4") SOURCE_GROUP_INPUT,
            in("a7") SYS_LOAD_WITNESS,
        );
    }
    ret
}

#[inline(always)]
fn load_ecdsa_sighash(buf: &mut [u8], len: &mut u64, index: usize, hash_type: u8) -> usize {
    let ret: usize;
    unsafe {
        asm!(
            "ecall",
            inlateout("a0") buf.as_mut_ptr() as usize => ret,
            in("a1") len as *mut u64 as usize,
            in("a2") 0usize,
            in("a3") index,
            in("a4") SOURCE_GROUP_INPUT,
            in("a5") hash_type as usize,
            in("a7") SYS_LOAD_ECDSA_SIGNATURE_HASH,
        );
    }
    ret
}

#[inline(always)]
fn secp256k1_verify(pubkey_hash: &[u8; 20], signature: &[u8; 65], message_hash: &[u8; 32]) -> usize {
    let ret: usize;
    unsafe {
        asm!(
            "ecall",
            inlateout("a0") pubkey_hash.as_ptr() as usize => ret,
            in("a1") signature.as_ptr() as usize,
            in("a2") message_hash.as_ptr() as usize,
            in("a7") SYS_SECP256K1_VERIFY,
        );
    }
    ret
}

#[unsafe(no_mangle)]
pub extern "C" fn _start() -> ! {
    let mut script = [0u8; 64];
    let mut script_len = script.len() as u64;
    
    if load_script(&mut script, &mut script_len) != SUCCESS || script_len < 57 {
        exit(1);
    }

    let args_len = read_u32_le(&script[33..37]);
    if args_len != 20 || 37 + args_len > script_len as usize {
        exit(1);
    }

    let mut pubkey_hash = [0u8; 20];
    pubkey_hash.copy_from_slice(&script[37..57]);

    let mut group_index = 0usize;
    let mut saw_group_witness = false;
    
    loop {
        let mut witness = [0u8; 256];
        let mut witness_len = witness.len() as u64;
        let ret = load_witness(&mut witness, &mut witness_len, group_index);
        
        if ret == INDEX_OUT_OF_BOUND {
            break;
        }
        if ret != SUCCESS {
            exit(1);
        }

        saw_group_witness = true;
        if witness_len != 65 && witness_len != 66 {
            exit(1);
        }

        let hash_type = if witness_len == 66 { witness[65] } else { SIG_HASH_ALL };
        let mut sighash = [0u8; 32];
        let mut sighash_len = sighash.len() as u64;
        
        if load_ecdsa_sighash(&mut sighash, &mut sighash_len, group_index, hash_type) != SUCCESS || sighash_len != 32 {
            exit(1);
        }

        let mut signature = [0u8; 65];
        signature.copy_from_slice(&witness[0..65]);
        
        if secp256k1_verify(&pubkey_hash, &signature, &sighash) != 0 {
            exit(1);
        }

        group_index += 1;
    }

    exit(if saw_group_witness { 0 } else { 1 });
}
