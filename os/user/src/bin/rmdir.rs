//! rmdir: remove empty directories.
//!
//!     rmdir [-p] dir...

#![no_std]
#![no_main]

use rt::prelude::*;

rt::main!(main);

fn main(args: &[String]) -> i32 {
    let (o, dirs) = rt::getopt::parse(&args[1..], "p", "[-p] dir...");
    if dirs.is_empty() {
        rt::die!("missing operand");
    }
    let mut st = 0;
    for d in &dirs {
        let mut cur = d.trim_end_matches('/').to_string();
        loop {
            if let Err(e) = rt::fs::remove_dir(&cur) {
                rt::warn!("failed to remove '{cur}': {e}");
                st = 1;
                break;
            }
            if !o.has('p') {
                break;
            }
            match rt::path::parent(&cur) {
                Some(p) if p != "/" && p != "." => cur = p.into(),
                _ => break,
            }
        }
    }
    st
}
