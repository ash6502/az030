//! uname: print system information.
//!
//!     uname [-asnrvm]

#![no_std]
#![no_main]

use rt::fs::cstr_field;
use rt::prelude::*;

rt::main!(main);

fn main(args: &[String]) -> i32 {
    let (o, _) = rt::getopt::parse(&args[1..], "asnrvmpio", "[-asnrvm]");
    let u = rt::process::uname();
    let host = rt::fs::read_to_string("/etc/hostname").map(|s| s.trim().to_string()).unwrap_or_else(|_| cstr_field(&u.nodename).into());
    let all = o.has('a');
    let mut parts = Vec::new();
    if all || o.has('s') || o.has('o') || !(o.has('n') || o.has('r') || o.has('v') || o.has('m') || o.has('p') || o.has('i')) {
        parts.push(cstr_field(&u.sysname).to_string());
    }
    if all || o.has('n') {
        parts.push(host);
    }
    if all || o.has('r') {
        parts.push(cstr_field(&u.release).into());
    }
    if all || o.has('v') {
        parts.push(cstr_field(&u.version).into());
    }
    if all || o.has('m') || o.has('p') || o.has('i') {
        parts.push(cstr_field(&u.machine).into());
    }
    println!("{}", parts.join(" "));
    0
}
