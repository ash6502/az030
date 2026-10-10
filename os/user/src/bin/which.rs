//! which: locate commands in $PATH.
//!
//!     which [-a] name...

#![no_std]
#![no_main]

use rt::prelude::*;

rt::main!(main);

fn main(args: &[String]) -> i32 {
    let (o, names) = rt::getopt::parse(&args[1..], "a", "[-a] name...");
    let path = rt::env::var("PATH").unwrap_or_default();
    let mut st = 0;
    for n in &names {
        let mut found = false;
        if n.contains('/') {
            if rt::fs::access(n, azsys::flags::X_OK).is_ok() {
                println!("{n}");
                found = true;
            }
        } else {
            for dir in path.split(':') {
                let p = rt::path::join(if dir.is_empty() { "." } else { dir }, n);
                if rt::fs::metadata(&p).is_ok_and(|m| m.is_file() && m.mode() & 0o111 != 0) {
                    println!("{p}");
                    found = true;
                    if !o.has('a') {
                        break;
                    }
                }
            }
        }
        if !found {
            st = 1;
        }
    }
    st
}
