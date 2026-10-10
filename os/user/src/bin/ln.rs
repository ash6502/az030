//! ln: make links.
//!
//!     ln [-sfv] target [link]
//!     ln [-sfv] target... dir

#![no_std]
#![no_main]

use rt::fs;
use rt::prelude::*;

rt::main!(main);

fn main(args: &[String]) -> i32 {
    let (o, mut files) = rt::getopt::parse(&args[1..], "sfvn", "[-sfv] target [link | dir]");
    if files.is_empty() {
        rt::die!("missing file operand");
    }
    if files.len() == 1 {
        files.push(".".into());
    }
    let dest = files.pop().unwrap();
    let to_dir = fs::is_dir(&dest) && !(o.has('n') && fs::symlink_metadata(&dest).is_ok_and(|m| m.is_symlink()));
    if files.len() > 1 && !to_dir {
        rt::die!("target '{dest}' is not a directory");
    }
    let mut st = 0;
    for t in &files {
        let link = if to_dir { rt::path::join(&dest, rt::path::basename(t)) } else { dest.clone() };
        if o.has('f') && fs::symlink_metadata(&link).is_ok() {
            let _ = fs::remove_file(&link);
        }
        let r = if o.has('s') { fs::symlink(t, &link) } else { fs::hard_link(t, &link) };
        match r {
            Ok(()) => {
                if o.has('v') {
                    println!("'{link}' -> '{t}'");
                }
            }
            Err(e) => {
                rt::warn!("cannot link '{link}' to '{t}': {e}");
                st = 1;
            }
        }
    }
    st
}
