//! Host front end for the azcc C compiler (used to build libc for AZ/UX).
//!
//!   azcc [-E | -S | -c] [-o OUT] [-I DIR]... [-isystem DIR]... [-D NAME[=VAL]]... [-U NAME]... FILE.c
//!
//! -S writes assembly, -c an object (assembled with azas), -E preprocessed text.

use std::path::PathBuf;
use std::process::ExitCode;

struct HostFiles;

impl azcc::Files for HostFiles {
    fn read(&mut self, path: &str) -> Option<String> {
        std::fs::read_to_string(path).ok()
    }
}

struct AsFiles;

impl azas::Files for AsFiles {
    fn read(&mut self, name: &str) -> Option<Vec<u8>> {
        std::fs::read(name).ok()
    }
}

fn main() -> ExitCode {
    let mut opts = azcc::Options::default();
    let (mut mode, mut out, mut input) = ('c', None::<PathBuf>, None::<PathBuf>);
    let mut args = std::env::args().skip(1);
    while let Some(a) = args.next() {
        match a.as_str() {
            "-E" | "-S" | "-c" => mode = a.chars().nth(1).unwrap(),
            "-o" => out = args.next().map(PathBuf::from),
            "-I" => opts.include_dirs.extend(args.next()),
            "-isystem" => opts.system_dirs.extend(args.next()),
            "-D" => {
                if let Some(d) = args.next() {
                    let (n, v) = d.split_once('=').unwrap_or((&d, "1"));
                    opts.defines.push((n.into(), v.into()));
                }
            }
            "-U" => opts.undefs.extend(args.next()),
            _ if a.starts_with("-I") => opts.include_dirs.push(a[2..].into()),
            _ if a.starts_with("-D") => {
                let d = &a[2..];
                let (n, v) = d.split_once('=').unwrap_or((d, "1"));
                opts.defines.push((n.into(), v.into()));
            }
            _ if a.starts_with("-O") || a.starts_with("-W") || a == "-g" => {}
            _ if a.starts_with('-') => {
                eprintln!("azcc: unknown option {a}");
                return ExitCode::FAILURE;
            }
            _ => input = Some(a.into()),
        }
    }
    let Some(input) = input else {
        eprintln!("usage: azcc [-E|-S|-c] [-o OUT] [-I DIR] [-D NAME=VAL] FILE.c");
        return ExitCode::FAILURE;
    };
    let src = match std::fs::read_to_string(&input) {
        Ok(s) => s,
        Err(e) => {
            eprintln!("azcc: {}: {e}", input.display());
            return ExitCode::FAILURE;
        }
    };
    let name = input.display().to_string();
    let r = if mode == 'E' { azcc::preprocess(&name, &src, &opts, &mut HostFiles) } else { azcc::compile(&name, &src, &opts, &mut HostFiles) };
    let o = match r {
        Ok(o) => o,
        Err(errs) => {
            for e in errs.iter().take(50) {
                eprintln!("{e}");
            }
            return ExitCode::FAILURE;
        }
    };
    for w in &o.warnings {
        eprintln!("{w}");
    }
    let bytes = match mode {
        'E' | 'S' => o.text.into_bytes(),
        _ => match azas::assemble(&name, &o.text, azas::Options { format: azas::Format::Elf, defines: Vec::new() }, &mut AsFiles) {
            Ok(b) => b,
            Err(errs) => {
                for e in errs.iter().take(20) {
                    eprintln!("azcc: assembler: {e}");
                }
                if std::env::var_os("AZCC_KEEP_ASM").is_some() {
                    let _ = std::fs::write(input.with_extension("s"), &o.text);
                }
                return ExitCode::FAILURE;
            }
        },
    };
    let out = out.unwrap_or_else(|| input.with_extension(match mode {
        'E' => "i",
        'S' => "s",
        _ => "o",
    }));
    if mode == 'E' && out.as_os_str() == "-" {
        print!("{}", String::from_utf8_lossy(&bytes));
        return ExitCode::SUCCESS;
    }
    if let Err(e) = std::fs::write(&out, bytes) {
        eprintln!("azcc: {}: {e}", out.display());
        return ExitCode::FAILURE;
    }
    ExitCode::SUCCESS
}
