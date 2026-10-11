//! ar: create and read archives (libraries).
//!
//!     ar r|rc|rcs|q archive file...    add or replace members
//!     ar t archive                      list members
//!     ar x archive [member...]          extract members
//!     ar d archive member...            delete members

#![no_std]
#![no_main]

use rt::prelude::*;

rt::main!(main);

fn load(path: &str) -> Vec<(String, Vec<u8>)> {
    match rt::fs::read(path) {
        Ok(d) => match azld::archive::members(&d) {
            Ok(ms) => ms.into_iter().map(|m| (m.name, m.data.to_vec())).collect(),
            Err(e) => rt::die!("{path}: {e}"),
        },
        Err(_) => Vec::new(),
    }
}

fn main(args: &[String]) -> i32 {
    if args.len() < 3 {
        rt::die!("usage: ar r|t|x|d[cvs] archive [file...]");
    }
    let ops = args[1].trim_start_matches('-');
    let archive = &args[2];
    let files = &args[3..];
    let verbose = ops.contains('v');
    let save = |members: &[(String, Vec<u8>)]| {
        if let Err(e) = rt::fs::write(archive, &azld::archive::write(members)) {
            rt::die!("{archive}: {e}");
        }
    };
    match ops.chars().find(|c| "rqtxd".contains(*c)) {
        Some('r') | Some('q') => {
            let exists = rt::fs::exists(archive);
            let mut members = load(archive);
            if !exists && !ops.contains('c') {
                eprintln!("ar: creating {archive}");
            }
            for f in files {
                let data = rt::fs::read(f).unwrap_or_else(|e| rt::die!("{f}: {e}"));
                let name = String::from(rt::path::basename(f));
                match members.iter_mut().find(|(n, _)| *n == name) {
                    Some(m) => m.1 = data,
                    None => members.push((name.clone(), data)),
                }
                if verbose {
                    println!("a - {name}");
                }
            }
            save(&members);
        }
        Some('t') => {
            for (n, d) in load(archive) {
                if verbose {
                    println!("{:8} {n}", d.len());
                } else {
                    println!("{n}");
                }
            }
        }
        Some('x') => {
            for (n, d) in load(archive) {
                if files.is_empty() || files.contains(&n) {
                    if verbose {
                        println!("x - {n}");
                    }
                    if let Err(e) = rt::fs::write(&n, &d) {
                        rt::die!("{n}: {e}");
                    }
                }
            }
        }
        Some('d') => {
            let mut members = load(archive);
            members.retain(|(n, _)| !files.contains(n));
            save(&members);
        }
        _ => rt::die!("unknown operation '{ops}'"),
    }
    0
}
