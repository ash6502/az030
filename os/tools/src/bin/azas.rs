//! Host front end for the azas assembler.
//!
//!   azas [-o OUT] [-f elf|bin] [-I DIR]... [-D NAME[=VALUE]]... FILE.s

use std::path::PathBuf;
use std::process::ExitCode;

struct HostFiles {
    dirs: Vec<PathBuf>,
}

impl azas::Files for HostFiles {
    fn read(&mut self, name: &str) -> Option<Vec<u8>> {
        self.dirs.iter().find_map(|d| std::fs::read(d.join(name)).ok())
    }
}

fn main() -> ExitCode {
    let mut out: Option<PathBuf> = None;
    let mut format = azas::Format::Elf;
    let mut input: Option<PathBuf> = None;
    let mut dirs: Vec<PathBuf> = Vec::new();
    let mut defines = Vec::new();
    let mut args = std::env::args().skip(1);
    while let Some(a) = args.next() {
        match a.as_str() {
            "-o" => out = args.next().map(PathBuf::from),
            "-f" => {
                format = match args.next().as_deref() {
                    Some("bin") | Some("binary") => azas::Format::Binary,
                    Some("elf") => azas::Format::Elf,
                    f => {
                        eprintln!("azas: unknown format {f:?}");
                        return ExitCode::FAILURE;
                    }
                }
            }
            "-I" => dirs.extend(args.next().map(PathBuf::from)),
            "-D" => {
                if let Some(d) = args.next() {
                    let (n, v) = d.split_once('=').unwrap_or((&d, "1"));
                    let v = if let Some(h) = v.strip_prefix("0x").or(v.strip_prefix('$')) {
                        i64::from_str_radix(h, 16).unwrap_or(0)
                    } else {
                        v.parse().unwrap_or(0)
                    };
                    defines.push((n.to_string(), v));
                }
            }
            _ if a.starts_with("-I") => dirs.push(a[2..].into()),
            _ => input = Some(a.into()),
        }
    }
    let Some(input) = input else {
        eprintln!("usage: azas [-o OUT] [-f elf|bin] [-I DIR] [-D NAME=VAL] FILE.s");
        return ExitCode::FAILURE;
    };
    let src = match std::fs::read_to_string(&input) {
        Ok(s) => s,
        Err(e) => {
            eprintln!("azas: {}: {e}", input.display());
            return ExitCode::FAILURE;
        }
    };
    dirs.insert(0, input.parent().map(PathBuf::from).unwrap_or_default());
    let out = out.unwrap_or_else(|| input.with_extension(if format == azas::Format::Elf { "o" } else { "bin" }));
    let mut files = HostFiles { dirs };
    match azas::assemble(&input.display().to_string(), &src, azas::Options { format, defines }, &mut files) {
        Ok(bytes) => {
            if let Err(e) = std::fs::write(&out, bytes) {
                eprintln!("azas: {}: {e}", out.display());
                return ExitCode::FAILURE;
            }
            ExitCode::SUCCESS
        }
        Err(errs) => {
            for e in errs.iter().take(50) {
                eprintln!("{e}");
            }
            ExitCode::FAILURE
        }
    }
}
