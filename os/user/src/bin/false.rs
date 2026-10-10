//! false: fail.

#![no_std]
#![no_main]

rt::main!(main);

fn main(_: &[rt::String]) -> i32 {
    1
}
