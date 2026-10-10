//! test / [: evaluate conditional expressions (see `help test` in sh).

#![no_std]
#![no_main]

use rt::prelude::*;

rt::main!(main);

fn main(args: &[String]) -> i32 {
    let name = rt::env::progname();
    rt::test::run(if name == "[" { "[" } else { "test" }, &args[1..])
}
