//! kill: send a signal to processes.
//!
//!     kill [-s signal | -signal] pid...
//!     kill -l

#![no_std]
#![no_main]

use rt::prelude::*;
use rt::signal;

rt::main!(main);

fn main(args: &[String]) -> i32 {
    let mut sig = signal::SIGTERM;
    let mut i = 1;
    match args.get(1).map(|s| s.as_str()) {
        Some("-l") | Some("-L") => {
            if let Some(n) = args.get(2) {
                match n.parse::<u32>().ok().and_then(|n| signal::abbrev(n & 0x7F)) {
                    Some(s) => println!("{s}"),
                    None => rt::die!("{n}: invalid signal"),
                }
                return 0;
            }
            for (n, name) in signal::all() {
                println!("{n:2}) SIG{name}");
            }
            return 0;
        }
        Some("-s") | Some("-n") => {
            sig = args.get(2).and_then(|s| signal::from_name(s)).unwrap_or_else(|| rt::die!("invalid signal"));
            i = 3;
        }
        Some(s) if s.starts_with('-') && s.len() > 1 && s != "--" => {
            sig = signal::from_name(&s[1..]).unwrap_or_else(|| rt::die!("{}: invalid signal", &s[1..]));
            i = 2;
        }
        Some("--") => i = 2,
        _ => {}
    }
    if i >= args.len() {
        rt::die!("usage: kill [-s signal | -signal] pid...");
    }
    let mut st = 0;
    for a in &args[i..] {
        match a.parse::<i32>() {
            Ok(pid) => {
                if let Err(e) = rt::process::kill(pid, sig) {
                    rt::warn!("({pid}) - {e}");
                    st = 1;
                }
            }
            Err(_) => {
                rt::warn!("{a}: arguments must be process IDs");
                st = 1;
            }
        }
    }
    st
}
