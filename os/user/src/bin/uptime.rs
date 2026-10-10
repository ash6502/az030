//! uptime: how long the system has been running.

#![no_std]
#![no_main]

use rt::prelude::*;

rt::main!(main);

fn main(_: &[String]) -> i32 {
    let s = rt::process::sysinfo();
    let t = rt::time::gmtime(rt::time::now());
    let up = s.uptime;
    let dur = if up >= 86400 {
        format!("{} day{}, {:2}:{:02}", up / 86400, if up / 86400 == 1 { "" } else { "s" }, up / 3600 % 24, up / 60 % 60)
    } else if up >= 3600 {
        format!("{:2}:{:02}", up / 3600, up / 60 % 60)
    } else {
        format!("{} min", up / 60)
    };
    println!(" {:02}:{:02}:{:02} up {dur},  {} processes,  load: {}", t.hour, t.min, t.sec, s.procs, s.load);
    0
}
