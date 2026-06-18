#![no_std]
#![no_main]

use core::arch::asm;

const EXPECTED_DATA: [u8; 4] = [0xDE, 0xAD, 0xBE, 0xEF];

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

#[unsafe(no_mangle)]
pub extern "C" fn _start() -> ! {
    let mut buf = [0u8; 4];
    let mut size = buf.len() as u64;
    let ret: usize;

    unsafe {
        asm!(
            "ecall",
            inlateout("a0") buf.as_mut_ptr() as usize => ret,
            in("a1") (&mut size as *mut u64) as usize,
            in("a2") 0usize,
            in("a3") 0usize,
            in("a4") 0x03usize,
            in("a7") 2092usize,
        );
    }

    if ret != 0 || size != EXPECTED_DATA.len() as u64 || buf != EXPECTED_DATA {
        exit(1);
    }

    exit(0);
}
