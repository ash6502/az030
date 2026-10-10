//! `test` / `[` expressions (shared by the shell builtin and /bin/test).

use crate::fs;
use alloc::format;
use alloc::string::String;
use azsys::flags::*;
use azsys::mode::*;

struct P<'a> {
    a: &'a [String],
    i: usize,
}

const UNARY: [&str; 19] = ["-e", "-f", "-d", "-r", "-w", "-x", "-s", "-L", "-h", "-b", "-c", "-p", "-S", "-z", "-n", "-t", "-u", "-g", "-k"];
const BINARY: [&str; 14] = ["=", "==", "!=", "<", ">", "-eq", "-ne", "-lt", "-le", "-gt", "-ge", "-nt", "-ot", "-ef"];

fn int(s: &str) -> Result<i64, String> {
    s.trim().parse().map_err(|_| format!("{s}: integer expression expected"))
}

fn unary(op: &str, x: &str) -> bool {
    let m = || fs::metadata(x);
    match op {
        "-z" => x.is_empty(),
        "-n" => !x.is_empty(),
        "-e" => m().is_ok(),
        "-f" => m().is_ok_and(|m| m.is_file()),
        "-d" => m().is_ok_and(|m| m.is_dir()),
        "-s" => m().is_ok_and(|m| m.len() > 0),
        "-L" | "-h" => fs::symlink_metadata(x).is_ok_and(|m| m.is_symlink()),
        "-b" => m().is_ok_and(|m| m.kind() == S_IFBLK),
        "-c" => m().is_ok_and(|m| m.kind() == S_IFCHR),
        "-p" => m().is_ok_and(|m| m.kind() == S_IFIFO),
        "-S" => m().is_ok_and(|m| m.kind() == S_IFSOCK),
        "-u" => m().is_ok_and(|m| m.mode() & S_ISUID != 0),
        "-g" => m().is_ok_and(|m| m.mode() & S_ISGID != 0),
        "-k" => m().is_ok_and(|m| m.mode() & S_ISVTX != 0),
        "-r" => fs::access(x, R_OK).is_ok(),
        "-w" => fs::access(x, W_OK).is_ok(),
        "-x" => fs::access(x, X_OK).is_ok(),
        "-t" => x.parse::<u32>().is_ok_and(crate::io::isatty),
        _ => false,
    }
}

fn binary(a: &str, op: &str, b: &str) -> Result<bool, String> {
    Ok(match op {
        "=" | "==" => a == b,
        "!=" => a != b,
        "<" => a < b,
        ">" => a > b,
        "-eq" => int(a)? == int(b)?,
        "-ne" => int(a)? != int(b)?,
        "-lt" => int(a)? < int(b)?,
        "-le" => int(a)? <= int(b)?,
        "-gt" => int(a)? > int(b)?,
        "-ge" => int(a)? >= int(b)?,
        "-nt" | "-ot" => {
            let (x, y) = (fs::metadata(a).map(|m| m.mtime()), fs::metadata(b).map(|m| m.mtime()));
            match (x, y) {
                (Ok(x), Ok(y)) => (op == "-nt" && x > y) || (op == "-ot" && x < y),
                (Ok(_), Err(_)) => op == "-nt",
                (Err(_), Ok(_)) => op == "-ot",
                _ => false,
            }
        }
        "-ef" => match (fs::metadata(a), fs::metadata(b)) {
            (Ok(x), Ok(y)) => x.dev() == y.dev() && x.ino() == y.ino(),
            _ => false,
        },
        _ => return Err(format!("{op}: unknown operator")),
    })
}

impl<'a> P<'a> {
    fn left(&self) -> usize {
        self.a.len() - self.i
    }
    fn at(&self, k: usize) -> Option<&'a str> {
        self.a.get(self.i + k).map(|s| s.as_str())
    }

    fn or(&mut self) -> Result<bool, String> {
        let mut v = self.and()?;
        while self.at(0) == Some("-o") {
            self.i += 1;
            let r = self.and()?;
            v = v || r;
        }
        Ok(v)
    }

    fn and(&mut self) -> Result<bool, String> {
        let mut v = self.not()?;
        while self.at(0) == Some("-a") {
            self.i += 1;
            let r = self.not()?;
            v = v && r;
        }
        Ok(v)
    }

    fn not(&mut self) -> Result<bool, String> {
        if self.at(0) == Some("!") && self.left() > 1 {
            self.i += 1;
            return Ok(!self.not()?);
        }
        self.primary()
    }

    fn primary(&mut self) -> Result<bool, String> {
        let Some(x) = self.at(0) else { return Err("argument expected".into()) };
        if self.left() >= 3 {
            if let Some(op) = self.at(1) {
                if BINARY.contains(&op) {
                    let (a, b) = (x, self.at(2).unwrap());
                    self.i += 3;
                    return binary(a, op, b);
                }
            }
        }
        if x == "(" && self.left() >= 2 {
            self.i += 1;
            let v = self.or()?;
            if self.at(0) != Some(")") {
                return Err("')' expected".into());
            }
            self.i += 1;
            return Ok(v);
        }
        if UNARY.contains(&x) && self.left() >= 2 {
            let arg = self.at(1).unwrap();
            self.i += 2;
            return Ok(unary(x, arg));
        }
        self.i += 1;
        Ok(!x.is_empty())
    }
}

/// Evaluate a test expression. Err for syntax errors.
pub fn eval(args: &[String]) -> Result<bool, String> {
    if args.is_empty() {
        return Ok(false);
    }
    let mut p = P { a: args, i: 0 };
    let v = p.or()?;
    if p.i < args.len() {
        return Err(format!("{}: unexpected argument", args[p.i]));
    }
    Ok(v)
}

/// Run `test`/`[` with its arguments (without the command name); returns the exit
/// status (0 true, 1 false, 2 error).
pub fn run(name: &str, args: &[String]) -> i32 {
    let args = if name == "[" {
        match args.split_last() {
            Some((l, rest)) if l == "]" => rest,
            _ => {
                crate::eprintln!("[: missing ']'");
                return 2;
            }
        }
    } else {
        args
    };
    match eval(args) {
        Ok(true) => 0,
        Ok(false) => 1,
        Err(e) => {
            crate::eprintln!("{name}: {e}");
            2
        }
    }
}
