//! mount: mount a file system, or list mounts.
//!
//!     mount [-r] device dir
//!     mount

#![no_std]
#![no_main]

use rt::fs::cstr_field;
use rt::prelude::*;

rt::main!(main);

fn main(args: &[String]) -> i32 {
    let (o, rest) = rt::getopt::parse(&args[1..], "rwt:o:", "[-r] device dir");
    if rest.is_empty() {
        for m in rt::fs::mounts() {
            println!("{} on {} ({})", cstr_field(&m.source), cstr_field(&m.path), if m.flags & 1 != 0 { "ro" } else { "rw" });
        }
        return 0;
    }
    if rest.len() != 2 {
        rt::die!("usage: mount [-r] device dir");
    }
    let ro = o.has('r') || o.get('o').is_some_and(|x| x.split(',').any(|y| y == "ro"));
    match rt::fs::mount(&rest[0], &rest[1], ro) {
        Ok(()) => 0,
        Err(e) => rt::die!("{} on {}: {e}", rest[0], rest[1]),
    }
}
