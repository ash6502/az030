//! time: run a command and report the time it took.

#![no_std]
#![no_main]

use rt::prelude::*;

rt::main!(main);

fn main(args: &[String]) -> i32 {
    if args.len() < 2 {
        rt::die!("usage: time command [arg...]");
    }
    let start = rt::time::ticks_ms();
    let st = rt::process::run(&args[1..]);
    let real = rt::time::ticks_ms() - start;
    let (t, _) = rt::process::times();
    let f = |ms: u64| format!("{}m{}.{:03}s", ms / 60000, ms / 1000 % 60, ms % 1000);
    let tick = 1000 / azsys::HZ as u64;
    eprintln!("\nreal\t{}\nuser\t{}\nsys\t{}", f(real), f(t.cutime as u64 * tick), f(t.cstime as u64 * tick));
    match st {
        Ok(s) => s.shell_code(),
        Err(e) => rt::die!("{e}"),
    }
}
