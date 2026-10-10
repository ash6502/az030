//! rm: remove files.
//!
//!     rm [-rRfiv] file...

#![no_std]
#![no_main]

use rt::fs;
use rt::prelude::*;

rt::main!(main);

struct O {
    recursive: bool,
    force: bool,
    interactive: bool,
    verbose: bool,
}

fn remove(path: &str, o: &O) -> bool {
    let m = match fs::symlink_metadata(path) {
        Ok(m) => m,
        Err(e) => {
            if o.force {
                return true;
            }
            rt::warn!("cannot remove '{path}': {e}");
            return false;
        }
    };
    if m.is_dir() {
        if !o.recursive {
            rt::warn!("cannot remove '{path}': Is a directory");
            return false;
        }
        if o.interactive && !rt::util::confirm(&format!("rm: descend into directory '{path}'? ")) {
            return true;
        }
        let mut ok = true;
        match fs::read_dir(path) {
            Ok(es) => {
                for e in es {
                    if e.name != "." && e.name != ".." {
                        ok &= remove(&rt::path::join(path, &e.name), o);
                    }
                }
            }
            Err(e) => {
                rt::warn!("cannot read '{path}': {e}");
                return false;
            }
        }
        if o.interactive && !rt::util::confirm(&format!("rm: remove directory '{path}'? ")) {
            return ok;
        }
        match fs::remove_dir(path) {
            Ok(()) => {
                if o.verbose {
                    println!("removed directory '{path}'");
                }
                ok
            }
            Err(e) => {
                rt::warn!("cannot remove '{path}': {e}");
                false
            }
        }
    } else {
        if o.interactive && !rt::util::confirm(&format!("rm: remove '{path}'? ")) {
            return true;
        }
        match fs::remove_file(path) {
            Ok(()) => {
                if o.verbose {
                    println!("removed '{path}'");
                }
                true
            }
            Err(e) => {
                rt::warn!("cannot remove '{path}': {e}");
                false
            }
        }
    }
}

fn main(args: &[String]) -> i32 {
    let (p, files) = rt::getopt::parse(&args[1..], "rRfiv", "[-rfiv] file...");
    let o = O { recursive: p.has('r') || p.has('R'), force: p.has('f'), interactive: p.has('i') && !p.has('f'), verbose: p.has('v') };
    if files.is_empty() {
        if o.force {
            return 0;
        }
        rt::die!("missing operand");
    }
    let mut st = 0;
    for f in &files {
        let base = rt::path::basename(f);
        if base == "." || base == ".." || rt::path::normalize(f) == "/" {
            rt::warn!("refusing to remove '{f}'");
            st = 1;
            continue;
        }
        if !remove(f, &o) {
            st = 1;
        }
    }
    st
}
