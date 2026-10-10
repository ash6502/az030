//! chmod: change file modes.
//!
//!     chmod [-Rv] mode file...     (mode: octal or [ugoa][+-=][rwxXst],...)

#![no_std]
#![no_main]

use rt::fs;
use rt::prelude::*;

rt::main!(main);

fn apply(path: &str, spec: &str, recursive: bool, verbose: bool) -> bool {
    let m = match fs::symlink_metadata(path) {
        Ok(m) => m,
        Err(e) => {
            rt::warn!("cannot access '{path}': {e}");
            return false;
        }
    };
    if m.is_symlink() {
        return true;
    }
    let mut ok = true;
    match rt::util::parse_mode(spec, m.mode(), m.is_dir()) {
        Ok(new) => {
            if let Err(e) = fs::chmod(path, new) {
                rt::warn!("changing permissions of '{path}': {e}");
                ok = false;
            } else if verbose {
                println!("mode of '{path}' changed to {:04o}", new);
            }
        }
        Err(e) => rt::die!("{e}"),
    }
    if recursive && m.is_dir() {
        if let Ok(es) = fs::read_dir(path) {
            for e in es {
                if e.name != "." && e.name != ".." {
                    ok &= apply(&rt::path::join(path, &e.name), spec, recursive, verbose);
                }
            }
        }
    }
    ok
}

fn main(args: &[String]) -> i32 {
    // modes like -w look like options: take them as the mode operand
    let mut a: Vec<String> = args[1..].to_vec();
    let mut mode_arg = None;
    if let Some(i) = a.iter().position(|s| s.starts_with('-') && s.len() > 1 && s[1..].chars().all(|c| "rwxXst".contains(c))) {
        mode_arg = Some(a.remove(i));
    }
    let (o, mut rest) = rt::getopt::parse(&a, "Rv", "[-Rv] mode file...");
    let mode = match mode_arg {
        Some(m) => m,
        None => {
            if rest.is_empty() {
                rt::die!("missing operand");
            }
            rest.remove(0)
        }
    };
    if rest.is_empty() {
        rt::die!("missing operand after '{mode}'");
    }
    let mut st = 0;
    for f in &rest {
        if !apply(f, &mode, o.has('R'), o.has('v')) {
            st = 1;
        }
    }
    st
}
