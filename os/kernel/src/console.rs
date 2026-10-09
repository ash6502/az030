//! Kernel console: polled output on the UART, used by kprint! before and after the
//! tty driver is up. Messages are also kept in a ring buffer for `dmesg`.

use crate::arch;
use core::fmt::{self, Write};

pub const UART_DATA: u32 = 0xFE00_0000;
pub const UART_STATUS: u32 = 0xFE00_0004;
pub const UART_TX_READY: u32 = 1 << 0;
pub const UART_RX_VALID: u32 = 1 << 1;

const LOG_SIZE: usize = 8192;
static mut LOG: [u8; LOG_SIZE] = [0; LOG_SIZE];
static mut LOG_LEN: usize = 0;

pub fn init() {}

/// Called from the panic handler: nothing to unlock, the console is polled.
pub fn emergency() {}

pub fn putc(c: u8) {
    while arch::rd(UART_STATUS) & UART_TX_READY == 0 {}
    arch::wr(UART_DATA, c as u32);
}

fn log_byte(c: u8) {
    unsafe {
        LOG[LOG_LEN % LOG_SIZE] = c;
        LOG_LEN += 1;
    }
}

/// Copy the kernel log (oldest first) into `buf`, returning the length.
pub fn read_log(buf: &mut [u8]) -> usize {
    unsafe {
        let len = LOG_LEN.min(LOG_SIZE);
        let start = LOG_LEN - len;
        let n = len.min(buf.len());
        for i in 0..n {
            buf[i] = LOG[(start + len - n + i) % LOG_SIZE];
        }
        n
    }
}

pub struct Console;

impl Write for Console {
    fn write_str(&mut self, s: &str) -> fmt::Result {
        let sr = arch::irq_save();
        for b in s.bytes() {
            if b == b'\n' {
                putc(b'\r');
            }
            putc(b);
            log_byte(b);
        }
        arch::irq_restore_sr(sr);
        Ok(())
    }
}

pub fn print(args: fmt::Arguments) {
    let _ = Console.write_fmt(args);
}

#[macro_export]
macro_rules! kprint {
    ($($arg:tt)*) => { $crate::console::print(format_args!($($arg)*)) };
}

#[macro_export]
macro_rules! kprintln {
    () => { $crate::console::print(format_args!("\n")) };
    ($($arg:tt)*) => { $crate::console::print(format_args!("{}\n", format_args!($($arg)*))) };
}
