//! env: run a program in a modified environment, or print the environment.
//!
//!     env [-i] [-u name] [name=value]... [command [arg...]]

#![no_std]
#![no_main]

use rt::prelude::*;

rt::main!(main);

fn main(args: &[String]) -> i32 {
    let mut i = 1;
    let mut clear = false;
    while i < args.len() {
        match args[i].as_str() {
            "-i" | "-" => clear = true,
            "-u" => {
                i += 1;
                if let Some(n) = args.get(i) {
                    rt::env::remove_var(n);
                }
            }
            _ => break,
        }
        i += 1;
    }
    if clear {
        for (k, _) in rt::env::vars() {
            rt::env::remove_var(&k);
        }
    }
    while i < args.len() {
        match args[i].split_once('=') {
            Some((k, v)) if !k.is_empty() => rt::env::set_var(k, v),
            _ => break,
        }
        i += 1;
    }
    if i >= args.len() {
        for e in rt::env::environ() {
            println!("{e}");
        }
        return 0;
    }
    let e = rt::process::exec(&args[i..]);
    rt::warn!("{}: {e}", args[i]);
    if e.0 == azsys::errno::ENOENT { 127 } else { 126 }
}
