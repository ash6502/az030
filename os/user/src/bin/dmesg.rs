//! dmesg: print the kernel's message log.

#![no_std]
#![no_main]

use rt::prelude::*;

rt::main!(main);

fn main(_: &[String]) -> i32 {
    let mut buf = vec![0u8; 8192];
    match rt::sys::call2(azsys::nr::KLOG, buf.as_mut_ptr() as u32, buf.len() as u32) {
        Ok(n) => {
            let _ = rt::io::write_fd(1, &buf[..n as usize]);
            0
        }
        Err(e) => rt::die!("{e}"),
    }
}
