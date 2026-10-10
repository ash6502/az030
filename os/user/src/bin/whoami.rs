//! whoami: print the effective user name.

#![no_std]
#![no_main]

use rt::prelude::*;

rt::main!(main);

fn main(_: &[String]) -> i32 {
    println!("{}", rt::users::user_name(rt::process::euid()));
    0
}
