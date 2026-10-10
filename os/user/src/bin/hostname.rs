//! hostname: print or set the host name (kept in /etc/hostname).

#![no_std]
#![no_main]

use rt::prelude::*;

rt::main!(main);

fn main(args: &[String]) -> i32 {
    match args.get(1) {
        Some(n) => {
            if let Err(e) = rt::fs::write("/etc/hostname", format!("{n}\n").as_bytes()) {
                rt::die!("{e}");
            }
        }
        None => println!("{}", rt::fs::read_to_string("/etc/hostname").map(|s| s.trim().to_string()).unwrap_or_else(|_| "az030".into())),
    }
    0
}
