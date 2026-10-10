//! xargs: build and run command lines from standard input.
//!
//!     xargs [-0rt] [-n max-args] [-I replace] [-d delim] [command [arg...]]

#![no_std]
#![no_main]

use rt::prelude::*;

rt::main!(main);

fn split_args(input: &str) -> Vec<String> {
    // whitespace-separated, with quotes and backslash escapes
    let mut out = Vec::new();
    let mut cur = String::new();
    let mut have = false;
    let mut it = input.chars();
    while let Some(c) = it.next() {
        match c {
            '\'' | '"' => {
                have = true;
                for q in it.by_ref() {
                    if q == c {
                        break;
                    }
                    cur.push(q);
                }
            }
            '\\' => {
                have = true;
                if let Some(n) = it.next() {
                    cur.push(n);
                }
            }
            c if c.is_whitespace() => {
                if have {
                    out.push(core::mem::take(&mut cur));
                    have = false;
                }
            }
            c => {
                cur.push(c);
                have = true;
            }
        }
    }
    if have {
        out.push(cur);
    }
    out
}

fn main(args: &[String]) -> i32 {
    let (o, cmd) = rt::getopt::parse_strict(&args[1..], "0rtn:I:d:L:", "[-0rt] [-n max] [-I replace] [-d delim] [command [arg...]]");
    let cmd = if cmd.is_empty() { vec![String::from("echo")] } else { cmd };
    let mut input = Vec::new();
    let _ = rt::io::FdIo(0).read_to_end(&mut input);
    let text = String::from_utf8_lossy(&input).into_owned();
    let items: Vec<String> = if o.has('0') {
        text.split('\0').filter(|s| !s.is_empty()).map(String::from).collect()
    } else if let Some(d) = o.get('d') {
        let d = if d == "\\n" { '\n' } else { d.chars().next().unwrap_or('\n') };
        text.split(d).filter(|s| !s.is_empty()).map(String::from).collect()
    } else if o.has('I') {
        text.lines().filter(|l| !l.trim().is_empty()).map(String::from).collect()
    } else {
        split_args(&text)
    };
    if items.is_empty() && o.has('r') {
        return 0;
    }
    let max = o.get('n').or(o.get('L')).and_then(|n| n.parse::<usize>().ok()).unwrap_or(if o.has('I') { 1 } else { 1000 }).max(1);
    let mut st = 0;
    let groups: Vec<&[String]> = if items.is_empty() { vec![&[][..]] } else { items.chunks(max).collect() };
    for g in groups {
        let argv: Vec<String> = match o.get('I') {
            Some(rep) => cmd.iter().map(|a| a.replace(rep, &g.join(" "))).collect(),
            None => cmd.iter().cloned().chain(g.iter().cloned()).collect(),
        };
        if o.has('t') {
            eprintln!("{}", argv.join(" "));
        }
        match rt::process::run(&argv) {
            Ok(s) => {
                if s.code() == Some(255) {
                    rt::warn!("{}: exited with status 255; aborting", argv[0]);
                    return 124;
                }
                if s.code() == Some(127) {
                    return 127;
                }
                if !s.success() {
                    st = 123;
                }
            }
            Err(e) => rt::die!("{e}"),
        }
    }
    st
}
