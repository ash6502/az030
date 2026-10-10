//! mv: move (rename) files.
//!
//!     mv [-finv] source... dest

#![no_std]
#![no_main]

use rt::fs;
use rt::prelude::*;
use rt::sys::errno;

rt::main!(main);

fn main(args: &[String]) -> i32 {
    let (o, mut files) = rt::getopt::parse(&args[1..], "finv", "[-finv] source... dest");
    if files.len() < 2 {
        rt::die!("missing destination file operand");
    }
    let dest = files.pop().unwrap();
    let to_dir = fs::is_dir(&dest);
    if files.len() > 1 && !to_dir {
        rt::die!("target '{dest}' is not a directory");
    }
    let mut st = 0;
    for f in &files {
        let target = if to_dir { rt::path::join(&dest, rt::path::basename(f)) } else { dest.clone() };
        if fs::exists(&target) {
            if o.has('n') {
                continue;
            }
            if o.has('i') && !o.has('f') && !rt::util::confirm(&format!("mv: overwrite '{target}'? ")) {
                continue;
            }
        }
        match fs::rename(f, &target) {
            Ok(()) => {}
            Err(e) if e.0 == errno::EXDEV => {
                // across file systems: copy, then remove
                let r = rt::process::run(&[String::from("cp"), String::from("-a"), f.clone(), target.clone()]);
                if !r.is_ok_and(|s| s.success()) {
                    rt::warn!("cannot move '{f}' to '{target}'");
                    st = 1;
                    continue;
                }
                let r = if fs::is_dir(f) { fs::remove_dir_all(f) } else { fs::remove_file(f) };
                if let Err(e) = r {
                    rt::warn!("cannot remove '{f}': {e}");
                    st = 1;
                }
            }
            Err(e) => {
                rt::warn!("cannot move '{f}' to '{target}': {e}");
                st = 1;
                continue;
            }
        }
        if o.has('v') {
            println!("renamed '{f}' -> '{target}'");
        }
    }
    st
}
