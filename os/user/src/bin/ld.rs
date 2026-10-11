//! ld: the static linker (azld).
//!
//!     ld [-o out] [-e entry] [--base addr] [--format elf|bin] [--no-gc] [--keep sym]
//!        [--map file] [-s] [-L dir]... [-l name]... file.o|lib.a...
//!
//! Libraries are searched in the -L directories and then /usr/lib.

#![no_std]
#![no_main]

use rt::prelude::*;

rt::main!(main);

fn num(s: &str) -> u32 {
    let r = if let Some(h) = s.strip_prefix("0x").or(s.strip_prefix('$')) { u32::from_str_radix(h, 16) } else { s.parse() };
    r.unwrap_or_else(|_| rt::die!("bad number '{s}'"))
}

fn main(args: &[String]) -> i32 {
    let mut opts = azld::Options::default();
    let mut out = String::from("a.out");
    let mut map: Option<String> = None;
    let mut inputs: Vec<String> = Vec::new();
    let mut libdirs: Vec<String> = Vec::new();
    let mut it = args[1..].iter();
    while let Some(a) = it.next() {
        let mut val = || it.next().cloned().unwrap_or_else(|| rt::die!("{a} needs an argument"));
        match a.as_str() {
            "-o" => out = val(),
            "--base" | "-Ttext" => opts.base = num(&val()),
            "-e" | "--entry" => opts.entry = val(),
            "--format" => {
                opts.format = match val().as_str() {
                    "elf" => azld::Format::Elf,
                    "bin" | "binary" => azld::Format::Binary,
                    f => rt::die!("unknown format '{f}'"),
                }
            }
            "--no-gc" | "--no-gc-sections" => opts.gc_sections = false,
            "--gc-sections" => opts.gc_sections = true,
            "--keep" => opts.keep.push(val()),
            "--no-page-align" => opts.page_align_data = false,
            "--map" | "-Map" => map = Some(val()),
            "-s" | "--strip-all" => opts.symbols = false,
            "--allow-undefined" => opts.allow_undefined = true,
            "-L" => libdirs.push(val()),
            "-l" => inputs.push(format!("-l{}", val())),
            _ if a.starts_with("-L") => libdirs.push(a[2..].into()),
            _ if a.starts_with("-l") => inputs.push(a.clone()),
            _ if a.starts_with('-') => rt::die!("unknown option '{a}'"),
            _ => inputs.push(a.clone()),
        }
    }
    if inputs.is_empty() {
        rt::die!("no input files");
    }
    libdirs.push("/usr/lib".into());
    let mut linker = azld::Linker::new();
    for i in &inputs {
        let path = match i.strip_prefix("-l") {
            Some(l) => {
                let name = format!("lib{l}.a");
                match libdirs.iter().map(|d| rt::path::join(d, &name)).find(|p| rt::fs::exists(p)) {
                    Some(p) => p,
                    None => rt::die!("cannot find -l{l}"),
                }
            }
            None => i.clone(),
        };
        let data = rt::fs::read(&path).unwrap_or_else(|e| rt::die!("{path}: {e}"));
        if let Err(e) = linker.add_file(&path, data) {
            rt::die!("{e}");
        }
    }
    let (bytes, lmap) = match linker.link(&opts) {
        Ok(r) => r,
        Err(e) => rt::die!("{e}"),
    };
    let mode = if opts.format == azld::Format::Elf { 0o755 } else { 0o644 };
    match rt::fs::File::open_with(&out, azsys::flags::O_WRONLY | azsys::flags::O_CREAT | azsys::flags::O_TRUNC, mode) {
        Ok(mut f) => {
            if let Err(e) = f.write_all(&bytes) {
                rt::die!("{out}: {e}");
            }
        }
        Err(e) => rt::die!("{out}: {e}"),
    }
    let _ = rt::fs::chmod(&out, mode);
    if let Some(m) = map {
        if let Err(e) = rt::fs::write(&m, azld::format_map(&lmap).as_bytes()) {
            rt::die!("{m}: {e}");
        }
    }
    0
}
