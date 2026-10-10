//! readlink: print symbolic link targets.
//!
//!     readlink [-fn] file...

#![no_std]
#![no_main]

use rt::prelude::*;

rt::main!(main);

/// Resolve every symlink in a path (like realpath).
pub fn canonical(p: &str) -> Option<String> {
    let mut parts: Vec<String> = Vec::new();
    let mut todo: Vec<String> = rt::path::absolute(p).split('/').filter(|s| !s.is_empty()).rev().map(String::from).collect();
    let mut hops = 0;
    while let Some(c) = todo.pop() {
        match c.as_str() {
            "." => continue,
            ".." => {
                parts.pop();
                continue;
            }
            _ => {}
        }
        let cur = format!("/{}", parts.iter().chain(core::iter::once(&c)).cloned().collect::<Vec<_>>().join("/"));
        match rt::fs::symlink_metadata(&cur) {
            Ok(m) if m.is_symlink() => {
                hops += 1;
                if hops > 40 {
                    return None;
                }
                let t = rt::fs::read_link(&cur).ok()?;
                if t.starts_with('/') {
                    parts.clear();
                }
                todo.extend(t.split('/').filter(|s| !s.is_empty()).rev().map(String::from));
            }
            Ok(_) => parts.push(c),
            Err(_) => {
                if todo.is_empty() {
                    parts.push(c);
                } else {
                    return None;
                }
            }
        }
    }
    Some(format!("/{}", parts.join("/")))
}

fn main(args: &[String]) -> i32 {
    let realpath = rt::env::progname() == "realpath";
    let (o, files) = rt::getopt::parse(&args[1..], "fenmq", "[-fn] file...");
    let mut st = 0;
    for f in &files {
        let r = if realpath || o.has('f') || o.has('e') || o.has('m') {
            canonical(f).ok_or(rt::Errno(azsys::errno::ENOENT))
        } else {
            rt::fs::read_link(f)
        };
        match r {
            Ok(t) => {
                if o.has('n') {
                    print!("{t}");
                } else {
                    println!("{t}");
                }
            }
            Err(e) => {
                if realpath && !o.has('q') {
                    rt::warn!("{f}: {e}");
                }
                st = 1;
            }
        }
    }
    st
}
