//! umount: unmount file systems.

#![no_std]
#![no_main]

use rt::prelude::*;

rt::main!(main);

fn main(args: &[String]) -> i32 {
    if args.len() < 2 {
        rt::die!("usage: umount dir...");
    }
    let mut st = 0;
    for d in &args[1..] {
        if let Err(e) = rt::fs::umount(d) {
            rt::warn!("{d}: {e}");
            st = 1;
        }
    }
    st
}
