//! cp: copy files and directories.
//!
//!     cp [-rRpfinv] source... dest

#![no_std]
#![no_main]

use rt::fs::{self, File};
use rt::prelude::*;

rt::main!(main);

pub struct O {
    recursive: bool,
    preserve: bool,
    force: bool,
    interactive: bool,
    no_clobber: bool,
    verbose: bool,
}

fn copy_file(src: &str, dst: &str, o: &O) -> Result<(), String> {
    let m = fs::metadata(src).map_err(|e| format!("{src}: {e}"))?;
    if fs::exists(dst) {
        if o.no_clobber {
            return Ok(());
        }
        if o.interactive && !rt::util::confirm(&format!("cp: overwrite '{dst}'? ")) {
            return Ok(());
        }
        if let (Ok(a), Ok(b)) = (fs::metadata(src), fs::metadata(dst)) {
            if a.ino() == b.ino() && a.dev() == b.dev() {
                return Err(format!("'{src}' and '{dst}' are the same file"));
            }
        }
    }
    let mut inp = File::open(src).map_err(|e| format!("{src}: {e}"))?;
    let mut out = match File::open_with(dst, azsys::flags::O_WRONLY | azsys::flags::O_CREAT | azsys::flags::O_TRUNC, m.mode() & 0o777) {
        Ok(f) => f,
        Err(e) if o.force => {
            let _ = fs::remove_file(dst);
            File::open_with(dst, azsys::flags::O_WRONLY | azsys::flags::O_CREAT | azsys::flags::O_TRUNC, m.mode() & 0o777).map_err(|_| format!("{dst}: {e}"))?
        }
        Err(e) => return Err(format!("{dst}: {e}")),
    };
    let mut buf = vec![0u8; 16384];
    loop {
        let n = inp.read(&mut buf).map_err(|e| format!("{src}: {e}"))?;
        if n == 0 {
            break;
        }
        out.write_all(&buf[..n]).map_err(|e| format!("{dst}: {e}"))?;
    }
    drop(out);
    if o.preserve {
        let _ = fs::chmod(dst, m.mode() & 0o7777);
        let _ = fs::set_times(dst, Some((m.0.atime, m.mtime())));
        let _ = fs::chown(dst, m.uid(), m.gid());
    }
    if o.verbose {
        println!("'{src}' -> '{dst}'");
    }
    Ok(())
}

fn copy(src: &str, dst: &str, o: &O) -> Result<(), String> {
    let m = fs::symlink_metadata(src).map_err(|e| format!("{src}: {e}"))?;
    if m.is_symlink() && o.recursive {
        let t = fs::read_link(src).map_err(|e| format!("{src}: {e}"))?;
        let _ = fs::remove_file(dst);
        return fs::symlink(&t, dst).map_err(|e| format!("{dst}: {e}"));
    }
    if fs::is_dir(src) {
        if !o.recursive {
            return Err(format!("-r not specified; omitting directory '{src}'"));
        }
        let dabs = rt::path::absolute(dst);
        let sabs = rt::path::absolute(src);
        if dabs == sabs || dabs.starts_with(&format!("{sabs}/")) {
            return Err(format!("cannot copy a directory, '{src}', into itself, '{dst}'"));
        }
        if !fs::is_dir(dst) {
            fs::create_dir_mode(dst, m.mode() & 0o7777 | 0o700).map_err(|e| format!("{dst}: {e}"))?;
            if o.verbose {
                println!("'{src}' -> '{dst}'");
            }
        }
        let mut err = None;
        for e in fs::read_dir(src).map_err(|e| format!("{src}: {e}"))? {
            if e.name == "." || e.name == ".." {
                continue;
            }
            if let Err(x) = copy(&rt::path::join(src, &e.name), &rt::path::join(dst, &e.name), o) {
                rt::warn!("{x}");
                err = Some(String::new());
            }
        }
        if o.preserve {
            let _ = fs::chmod(dst, m.mode() & 0o7777);
            let _ = fs::set_times(dst, Some((m.0.atime, m.mtime())));
        }
        return match err {
            Some(_) => Err(String::new()),
            None => Ok(()),
        };
    }
    copy_file(src, dst, o)
}

fn main(args: &[String]) -> i32 {
    let (p, mut files) = rt::getopt::parse(&args[1..], "rRpfinva", "[-rpfinv] source... dest");
    let o = O {
        recursive: p.has('r') || p.has('R') || p.has('a'),
        preserve: p.has('p') || p.has('a'),
        force: p.has('f'),
        interactive: p.has('i'),
        no_clobber: p.has('n'),
        verbose: p.has('v'),
    };
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
        if let Err(e) = copy(f, &target, &o) {
            if !e.is_empty() {
                rt::warn!("{e}");
            }
            st = 1;
        }
    }
    st
}
