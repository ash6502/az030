//! Runs `tests::run()` on the bare machine, prints the transcript on the UART between
//! markers, and powers the emulator off.

#![no_std]
#![no_main]

extern crate alloc;
extern crate azrt;

mod tests;

use core::alloc::{GlobalAlloc, Layout};

const UART: u32 = 0xFE00_0000;
const POWER: u32 = 0xFE00_4020;

fn putc(c: u8) {
    unsafe { core::ptr::write_volatile(UART as *mut u32, c as u32) }
}

fn puts(s: &str) {
    for b in s.bytes() {
        if b == b'\n' {
            putc(b'\r');
        }
        putc(b);
    }
}

/// Bump allocator over RAM above 1 MB (the test is short-lived).
struct Bump;
static mut NEXT: usize = 0x0010_0000;
unsafe impl GlobalAlloc for Bump {
    unsafe fn alloc(&self, l: Layout) -> *mut u8 {
        unsafe {
            let p = (NEXT + l.align() - 1) & !(l.align() - 1);
            NEXT = p + l.size();
            p as *mut u8
        }
    }
    unsafe fn dealloc(&self, _: *mut u8, _: Layout) {}
}
#[global_allocator]
static A: Bump = Bump;

fn power_off() -> ! {
    unsafe { core::ptr::write_volatile(POWER as *mut u32, 0x504F_4646) };
    loop {}
}

#[panic_handler]
fn panic(i: &core::panic::PanicInfo) -> ! {
    use core::fmt::Write;
    struct W;
    impl core::fmt::Write for W {
        fn write_str(&mut self, s: &str) -> core::fmt::Result {
            puts(s);
            Ok(())
        }
    }
    let _ = write!(W, "\nPANIC: {}\n=== END ===\n", i);
    power_off()
}

#[unsafe(no_mangle)]
pub extern "C" fn abort() -> ! {
    puts("\nabort\n=== END ===\n");
    power_off()
}

#[unsafe(no_mangle)]
pub extern "C" fn _start() -> ! {
    puts("=== BEGIN ===\n");
    tests::run(&mut tests::Out(&mut |s| puts(s)));
    puts("=== END ===\n");
    power_off()
}
