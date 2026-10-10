//! POSIX-style option parsing.
//!
//! `spec` lists the option letters; a letter followed by `:` takes an argument.
//! Options may be combined (`-la`), arguments attached or separate (`-n5`, `-n 5`),
//! `--` ends the options and a lone `-` is an operand. Parsing stops at the first
//! operand unless `permute` is set (GNU style: options anywhere).

use alloc::string::String;
use alloc::vec::Vec;

#[derive(Default)]
pub struct Opts {
    /// (letter, argument) in command-line order.
    pub list: Vec<(char, Option<String>)>,
}

impl Opts {
    pub fn has(&self, c: char) -> bool {
        self.list.iter().any(|(o, _)| *o == c)
    }
    /// How many times the option was given (`-vv`).
    pub fn count(&self, c: char) -> usize {
        self.list.iter().filter(|(o, _)| *o == c).count()
    }
    /// The last argument given to the option.
    pub fn get(&self, c: char) -> Option<&str> {
        self.list.iter().rev().find(|(o, _)| *o == c).and_then(|(_, a)| a.as_deref())
    }
    pub fn all(&self, c: char) -> Vec<&str> {
        self.list.iter().filter(|(o, _)| *o == c).filter_map(|(_, a)| a.as_deref()).collect()
    }
    /// A numeric option argument, with a default.
    pub fn num(&self, c: char, default: i64) -> Result<i64, String> {
        match self.get(c) {
            None => Ok(default),
            Some(s) => s.parse().map_err(|_| alloc::format!("invalid number '{s}' for -{c}")),
        }
    }
}

fn parse_inner(args: &[String], spec: &str, permute: bool) -> Result<(Opts, Vec<String>), String> {
    let mut opts = Opts::default();
    let mut rest = Vec::new();
    let mut i = 0;
    while i < args.len() {
        let a = &args[i];
        i += 1;
        if a == "--" {
            rest.extend(args[i..].iter().cloned());
            break;
        }
        if !a.starts_with('-') || a == "-" {
            rest.push(a.clone());
            if !permute {
                rest.extend(args[i..].iter().cloned());
                break;
            }
            continue;
        }
        let chars: Vec<char> = a[1..].chars().collect();
        let mut j = 0;
        while j < chars.len() {
            let c = chars[j];
            j += 1;
            let Some(pos) = spec.find(c) else { return Err(alloc::format!("invalid option -- '{c}'")) };
            if spec[pos + c.len_utf8()..].starts_with(':') {
                let arg = if j < chars.len() {
                    let s: String = chars[j..].iter().collect();
                    j = chars.len();
                    s
                } else if i < args.len() {
                    i += 1;
                    args[i - 1].clone()
                } else {
                    return Err(alloc::format!("option requires an argument -- '{c}'"));
                };
                opts.list.push((c, Some(arg)));
            } else {
                opts.list.push((c, None));
            }
        }
    }
    Ok((opts, rest))
}

/// Parse `args` (without the program name). On error, prints a message and a usage
/// line and exits with status 2.
pub fn parse(args: &[String], spec: &str, usage: &str) -> (Opts, Vec<String>) {
    match parse_inner(args, spec, true) {
        Ok(r) => r,
        Err(e) => {
            crate::warn!("{e}");
            crate::eprintln!("usage: {} {}", crate::env::progname(), usage);
            crate::process::exit(2)
        }
    }
}

/// Like `parse`, but options end at the first operand (for commands like `env` and
/// `nice` whose operands are another command line).
pub fn parse_strict(args: &[String], spec: &str, usage: &str) -> (Opts, Vec<String>) {
    match parse_inner(args, spec, false) {
        Ok(r) => r,
        Err(e) => {
            crate::warn!("{e}");
            crate::eprintln!("usage: {} {}", crate::env::progname(), usage);
            crate::process::exit(2)
        }
    }
}

/// Non-exiting variant (for shell builtins).
pub fn try_parse(args: &[String], spec: &str) -> Result<(Opts, Vec<String>), String> {
    parse_inner(args, spec, false)
}
