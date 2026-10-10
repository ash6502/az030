//! dirname: strip the last component from file names.

#![no_std]
#![no_main]

use rt::prelude::*;

rt::main!(main);

fn main(args: &[String]) -> i32 {
    if args.len() < 2 {
        rt::die!("missing operand");
    }
    for a in &args[1..] {
        println!("{}", rt::path::dirname(a));
    }
    0
}
