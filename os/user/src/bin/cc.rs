//! cc: the C compiler driver.
//!
//!     cc [-c | -S | -E] [-o out] [-I dir] [-D name[=val]] [-U name] [-L dir] [-l lib]
//!        [-s] [-v] [-w] [-O*] [-g] file.c|file.s|file.o|lib.a...
//!
//! C files are compiled with azcc, assembly with azas, and everything is linked
//! with azld against /usr/lib/crt0.o and /usr/lib/libc.a.

#![no_std]
#![no_main]

use rt::prelude::*;

rt::main!(main);

struct Files;

impl azcc::Files for Files {
    fn read(&mut self, path: &str) -> Option<String> {
        rt::fs::read_to_string(path).ok()
    }
}

struct AsFiles {
    dirs: Vec<String>,
}

impl azas::Files for AsFiles {
    fn read(&mut self, name: &str) -> Option<Vec<u8>> {
        if name.starts_with('/') {
            return rt::fs::read(name).ok();
        }
        self.dirs.iter().find_map(|d| rt::fs::read(&rt::path::join(d, name)).ok())
    }
}

fn stem(path: &str) -> &str {
    let b = rt::path::basename(path);
    match b.rfind('.') {
        Some(i) if i > 0 => &b[..i],
        _ => b,
    }
}

fn main(args: &[String]) -> i32 {
    let mut opts = azcc::Options { system_dirs: vec!["/usr/include".into()], ..Default::default() };
    let mut mode = 'l'; // link
    let mut out: Option<String> = None;
    let mut inputs: Vec<String> = Vec::new();
    let mut libdirs: Vec<String> = Vec::new();
    let mut strip = false;
    let mut verbose = false;
    let mut quiet = false;
    let mut it = args[1..].iter();
    while let Some(a) = it.next() {
        let mut val = |opt: &str| -> String { it.next().cloned().unwrap_or_else(|| rt::die!("{opt} needs an argument")) };
        match a.as_str() {
            "-c" | "-S" | "-E" => mode = a.chars().nth(1).unwrap(),
            "-o" => out = Some(val("-o")),
            "-I" => opts.include_dirs.push(val("-I")),
            "-D" => {
                let d = val("-D");
                let (n, v) = d.split_once('=').unwrap_or((&d, "1"));
                opts.defines.push((n.into(), v.into()));
            }
            "-U" => opts.undefs.push(val("-U")),
            "-L" => libdirs.push(val("-L")),
            "-l" => inputs.push(format!("-l{}", val("-l"))),
            "-s" => strip = true,
            "-v" => verbose = true,
            "-w" => quiet = true,
            "-isystem" => opts.system_dirs.insert(0, val("-isystem")),
            "-std=c89" | "-std=c99" | "-std=c11" | "-ansi" | "-pedantic" | "-g" | "-pipe" => {}
            _ if a.starts_with("-I") => opts.include_dirs.push(a[2..].into()),
            _ if a.starts_with("-D") => {
                let d = &a[2..];
                let (n, v) = d.split_once('=').unwrap_or((d, "1"));
                opts.defines.push((n.into(), v.into()));
            }
            _ if a.starts_with("-U") => opts.undefs.push(a[2..].into()),
            _ if a.starts_with("-L") => libdirs.push(a[2..].into()),
            _ if a.starts_with("-l") => inputs.push(a.clone()),
            _ if a.starts_with("-O") || a.starts_with("-W") || a.starts_with("-f") || a.starts_with("-m") => {}
            _ if a.starts_with('-') => rt::die!("unknown option '{a}'"),
            _ => inputs.push(a.clone()),
        }
    }
    if inputs.is_empty() {
        rt::die!("no input files");
    }
    let sources = inputs.iter().filter(|i| !i.starts_with("-l")).count();
    if out.is_some() && mode != 'l' && sources > 1 {
        rt::die!("cannot specify -o with -c, -S or -E with multiple files");
    }
    // compile and assemble
    let mut objects: Vec<(String, Vec<u8>)> = Vec::new();
    let mut failed = false;
    for input in &inputs {
        if input.starts_with("-l") {
            objects.push((input.clone(), Vec::new()));
            continue;
        }
        let ext = rt::path::extension(input).unwrap_or("");
        let asm_text: Option<String> = match ext {
            "c" | "i" => {
                let src = match rt::fs::read_to_string(input) {
                    Ok(s) => s,
                    Err(e) => {
                        rt::warn!("{input}: {e}");
                        failed = true;
                        continue;
                    }
                };
                if verbose {
                    eprintln!("cc: compiling {input}");
                }
                let r = if mode == 'E' { azcc::preprocess(input, &src, &opts, &mut Files) } else { azcc::compile(input, &src, &opts, &mut Files) };
                match r {
                    Ok(o) => {
                        if !quiet {
                            for w in &o.warnings {
                                eprintln!("{w}");
                            }
                        }
                        if mode == 'E' {
                            match &out {
                                Some(f) => {
                                    if let Err(e) = rt::fs::write(f, o.text.as_bytes()) {
                                        rt::die!("{f}: {e}");
                                    }
                                }
                                None => print!("{}", o.text),
                            }
                            continue;
                        }
                        if mode == 'S' {
                            let f = out.clone().unwrap_or_else(|| format!("{}.s", stem(input)));
                            if let Err(e) = rt::fs::write(&f, o.text.as_bytes()) {
                                rt::die!("{f}: {e}");
                            }
                            continue;
                        }
                        Some(o.text)
                    }
                    Err(errs) => {
                        for e in errs.iter().take(40) {
                            eprintln!("{e}");
                        }
                        failed = true;
                        continue;
                    }
                }
            }
            "s" | "S" | "asm" => {
                if mode == 'E' || mode == 'S' {
                    continue;
                }
                match rt::fs::read_to_string(input) {
                    Ok(s) => Some(s),
                    Err(e) => {
                        rt::warn!("{input}: {e}");
                        failed = true;
                        continue;
                    }
                }
            }
            _ => None,
        };
        match asm_text {
            Some(text) => {
                if verbose {
                    eprintln!("cc: assembling {input}");
                }
                let mut files = AsFiles { dirs: vec![String::from(rt::path::dirname(input)), "/usr/include".into()] };
                match azas::assemble(input, &text, azas::Options { format: azas::Format::Elf, defines: Vec::new() }, &mut files) {
                    Ok(obj) => {
                        if mode == 'c' {
                            let f = out.clone().unwrap_or_else(|| format!("{}.o", stem(input)));
                            if let Err(e) = rt::fs::write(&f, &obj) {
                                rt::die!("{f}: {e}");
                            }
                        } else {
                            objects.push((input.clone(), obj));
                        }
                    }
                    Err(errs) => {
                        for e in errs.iter().take(20) {
                            eprintln!("{e}");
                        }
                        failed = true;
                    }
                }
            }
            None => {
                if mode == 'l' {
                    match rt::fs::read(input) {
                        Ok(d) => objects.push((input.clone(), d)),
                        Err(e) => {
                            rt::warn!("{input}: {e}");
                            failed = true;
                        }
                    }
                }
            }
        }
    }
    if failed {
        return 1;
    }
    if mode != 'l' {
        return 0;
    }
    // link
    libdirs.push("/usr/lib".into());
    let mut linker = azld::Linker::new();
    let crt0 = "/usr/lib/crt0.o";
    match rt::fs::read(crt0) {
        Ok(d) => {
            if let Err(e) = linker.add_file(crt0, d) {
                rt::die!("{e}");
            }
        }
        Err(e) => rt::die!("{crt0}: {e}"),
    }
    let mut libs: Vec<String> = Vec::new();
    for (name, data) in objects {
        if let Some(l) = name.strip_prefix("-l") {
            libs.push(l.into());
            continue;
        }
        if let Err(e) = linker.add_file(&name, data) {
            rt::die!("{e}");
        }
    }
    libs.push("c".into());
    for l in libs {
        let file = format!("lib{l}.a");
        let Some(path) = libdirs.iter().map(|d| rt::path::join(d, &file)).find(|p| rt::fs::exists(p)) else { rt::die!("cannot find -l{l}") };
        let data = rt::fs::read(&path).unwrap_or_else(|e| rt::die!("{path}: {e}"));
        if let Err(e) = linker.add_file(&path, data) {
            rt::die!("{e}");
        }
    }
    let lopts = azld::Options { symbols: !strip, ..Default::default() };
    if verbose {
        eprintln!("cc: linking");
    }
    let (bytes, _) = match linker.link(&lopts) {
        Ok(r) => r,
        Err(e) => rt::die!("{e}"),
    };
    let out = out.unwrap_or_else(|| "a.out".into());
    match rt::fs::File::open_with(&out, azsys::flags::O_WRONLY | azsys::flags::O_CREAT | azsys::flags::O_TRUNC, 0o755) {
        Ok(mut f) => {
            if let Err(e) = f.write_all(&bytes) {
                rt::die!("{out}: {e}");
            }
        }
        Err(e) => rt::die!("{out}: {e}"),
    }
    let _ = rt::fs::chmod(&out, 0o755);
    0
}
