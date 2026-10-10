//! logname: print the login name.

#![no_std]
#![no_main]

use rt::prelude::*;

rt::main!(main);

fn main(_: &[String]) -> i32 {
    match rt::env::var("LOGNAME").or_else(|| rt::env::var("USER")) {
        Some(n) => {
            println!("{n}");
            0
        }
        None => rt::die!("no login name"),
    }
}
