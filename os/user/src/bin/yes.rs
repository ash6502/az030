//! yes: print a string (default `y`) forever.

#![no_std]
#![no_main]

use rt::prelude::*;

rt::main!(main);

fn main(args: &[String]) -> i32 {
    let s = if args.len() > 1 { args[1..].join(" ") } else { "y".into() };
    let mut block = String::new();
    while block.len() < 4096 {
        block.push_str(&s);
        block.push('\n');
    }
    loop {
        if rt::io::write_fd(1, block.as_bytes()).is_err() {
            return 1;
        }
    }
}
