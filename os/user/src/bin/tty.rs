//! tty: print the terminal name of standard input.

#![no_std]
#![no_main]

use rt::prelude::*;

rt::main!(main);

fn main(args: &[String]) -> i32 {
    let silent = args.get(1).is_some_and(|a| a == "-s");
    if rt::io::isatty(0) {
        if !silent {
            println!("/dev/console");
        }
        0
    } else {
        if !silent {
            println!("not a tty");
        }
        1
    }
}
