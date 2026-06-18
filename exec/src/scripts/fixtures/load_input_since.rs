#![no_std]
#![no_main]

use core::arch::asm;

const EXPECTED_SINCE_BYTES: [u8; 8] = [0x88, 0x77, 0x66, 0x55, 0x44, 0x33, 0x22, 0x11];

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
    let mut buf = [0u8; 8];
    let mut size = buf.len() as u64;
    let ret: usize;

    unsafe {
        asm!(
            "ecall",
            inlateout("a0") buf.as_mut_ptr() as usize => ret,
            in("a1") (&mut size as *mut u64) as usize,
            in("a2") 0usize,
            in("a3") 0usize,
            in("a4") 0x01usize,
            in("a5") 0x01usize,
            in("a7") 2083usize,
        );
    }

    if ret != 0 || size != 8 || buf != EXPECTED_SINCE_BYTES {
        exit(1);
    }

    exit(0);
}
