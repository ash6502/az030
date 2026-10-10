//! df: report file system space.
//!
//!     df [-hki] [file...]

#![no_std]
#![no_main]

use rt::fs::cstr_field;
use rt::prelude::*;

rt::main!(main);

fn main(args: &[String]) -> i32 {
    let (o, files) = rt::getopt::parse(&args[1..], "hkiP", "[-hki] [file...]");
    let mounts = rt::fs::mounts();
    let targets: Vec<(String, String)> = if files.is_empty() {
        mounts.iter().map(|m| (cstr_field(&m.source).to_string(), cstr_field(&m.path).to_string())).collect()
    } else {
        files
            .iter()
            .map(|f| {
                // the mount with the longest path prefix
                let abs = rt::path::absolute(f);
                let m = mounts
                    .iter()
                    .filter(|m| {
                        let p = cstr_field(&m.path);
                        abs == p || p == "/" || abs.starts_with(&format!("{p}/"))
                    })
                    .max_by_key(|m| cstr_field(&m.path).len());
                match m {
                    Some(m) => (cstr_field(&m.source).to_string(), cstr_field(&m.path).to_string()),
                    None => (String::from("?"), abs),
                }
            })
            .collect()
    };
    let mut out = String::new();
    if o.has('i') {
        out.push_str(&format!("{:<12} {:>9} {:>9} {:>9} {:>5} {}\n", "Filesystem", "Inodes", "IUsed", "IFree", "IUse%", "Mounted on"));
    } else {
        out.push_str(&format!("{:<12} {:>9} {:>9} {:>9} {:>5} {}\n", "Filesystem", if o.has('h') { "Size" } else { "1K-blocks" }, "Used", "Avail", "Use%", "Mounted on"));
    }
    let mut st = 0;
    for (src, path) in targets {
        let s = match rt::fs::statfs(&path) {
            Ok(s) => s,
            Err(e) => {
                rt::warn!("{path}: {e}");
                st = 1;
                continue;
            }
        };
        if o.has('i') {
            let used = s.files - s.ffree;
            let pct = if s.files > 0 { (used as u64 * 100).div_ceil(s.files as u64) } else { 0 };
            out.push_str(&format!("{:<12} {:>9} {:>9} {:>9} {:>4}% {}\n", src, s.files, used, s.ffree, pct, path));
            continue;
        }
        let total = s.blocks as u64 * s.bsize as u64;
        let free = s.bfree as u64 * s.bsize as u64;
        let used = total - free;
        let pct = if total > 0 { (used * 100).div_ceil(total) } else { 0 };
        let f = |b: u64| if o.has('h') { rt::util::human_size(b) } else { format!("{}", b / 1024) };
        out.push_str(&format!("{:<12} {:>9} {:>9} {:>9} {:>4}% {}\n", src, f(total), f(used), f(free), pct, path));
    }
    print!("{out}");
    st
}
