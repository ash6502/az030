//! as: the 68030/68882 assembler (azas).
//!
//!     as [-o out] [-f elf|bin] [-I dir]... [-D name[=value]]... file.s

#![no_std]
#![no_main]

use rt::prelude::*;

rt::main!(main);

struct Files {
    dirs: Vec<String>,
}

impl azas::Files for Files {
    fn read(&mut self, name: &str) -> Option<Vec<u8>> {
        if name.starts_with('/') {
            return rt::fs::read(name).ok();
        }
        self.dirs.iter().find_map(|d| rt::fs::read(&rt::path::join(d, name)).ok())
    }
}

fn parse_num(v: &str) -> i64 {
    if let Some(h) = v.strip_prefix("0x").or(v.strip_prefix('$')) {
        i64::from_str_radix(h, 16).unwrap_or(0)
    } else {
        v.parse().unwrap_or(0)
    }
}

fn main(args: &[String]) -> i32 {
    let (o, files) = rt::getopt::parse(&args[1..], "o:f:I:D:", "[-o out] [-f elf|bin] [-I dir] [-D name=value] file.s");
    let [input] = files.as_slice() else { rt::die!("usage: as [-o out] [-f elf|bin] [-I dir] [-D name=value] file.s") };
    let format = match o.get('f') {
        None | Some("elf") => azas::Format::Elf,
        Some("bin") | Some("binary") => azas::Format::Binary,
        Some(f) => rt::die!("unknown format '{f}'"),
    };
    let defines = o
        .all('D')
        .iter()
        .map(|d| {
            let (n, v) = d.split_once('=').unwrap_or((d, "1"));
            (String::from(n), parse_num(v))
        })
        .collect();
    let src = match rt::fs::read_to_string(input) {
        Ok(s) => s,
        Err(e) => rt::die!("{input}: {e}"),
    };
    let mut dirs = vec![String::from(rt::path::dirname(input))];
    dirs.extend(o.all('I').iter().map(|s| String::from(*s)));
    dirs.push("/usr/include".into());
    let out = match o.get('o') {
        Some(o) => o.to_string(),
        None => {
            let stem = input.strip_suffix(".s").or(input.strip_suffix(".S")).or(input.strip_suffix(".asm")).unwrap_or(input);
            format!("{}.{}", rt::path::basename(stem), if format == azas::Format::Elf { "o" } else { "bin" })
        }
    };
    match azas::assemble(input, &src, azas::Options { format, defines }, &mut Files { dirs }) {
        Ok(bytes) => {
            if let Err(e) = rt::fs::write(&out, &bytes) {
                rt::die!("{out}: {e}");
            }
            0
        }
        Err(errs) => {
            for e in errs.iter().take(50) {
                eprintln!("{e}");
            }
            1
        }
    }
}
