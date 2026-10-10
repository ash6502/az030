//! Built-in commands.

use crate::parse::{is_name, Simple};
use crate::{Flow, Shell, Var};
use rt::io;
use rt::prelude::*;
use rt::process;
use rt::signal::{self, Handler};

pub type Builtin = fn(&mut Shell, &[String]) -> i32;

const TABLE: [(&str, Builtin); 37] = [
    (":", |_, _| 0),
    ("true", |_, _| 0),
    ("false", |_, _| 1),
    (".", source),
    ("source", source),
    ("alias", alias),
    ("bg", bg),
    ("break", brk),
    ("cd", cd),
    ("command", command),
    ("continue", cont),
    ("echo", echo),
    ("eval", eval),
    ("exec", |_, _| 0),
    ("exit", exit),
    ("export", export),
    ("fg", fg),
    ("help", help),
    ("history", history),
    ("jobs", jobs),
    ("kill", kill),
    ("local", local),
    ("pwd", pwd),
    ("read", read),
    ("readonly", readonly),
    ("return", ret),
    ("set", set),
    ("shift", shift),
    ("test", |_, a| rt::test::run("test", &a[1..])),
    ("[", |_, a| rt::test::run("[", &a[1..])),
    ("trap", trap),
    ("type", type_),
    ("umask", umask),
    ("unalias", unalias),
    ("unset", unset),
    ("wait", wait),
    ("times", times),
];

pub const NAMES: [&str; 37] = {
    let mut n = [""; 37];
    let mut i = 0;
    while i < TABLE.len() {
        n[i] = TABLE[i].0;
        i += 1;
    }
    n
};

pub fn find(name: &str) -> Option<Builtin> {
    TABLE.iter().find(|(n, _)| *n == name).map(|(_, f)| *f)
}

fn usage(sh: &Shell, msg: &str) -> i32 {
    sh.error(msg);
    2
}

// ---- traps ----------------------------------------------------------------------

static mut PENDING: [u8; 32] = [0; 32];

extern "C" fn trap_handler(sig: u32) {
    unsafe {
        let p = &raw mut PENDING;
        core::ptr::write_volatile(&mut (*p)[sig as usize & 31], 1);
    }
}

impl Shell {
    /// Run the actions of traps whose signals have arrived.
    pub fn run_traps(&mut self) {
        for sig in 1..32u32 {
            let hit = unsafe {
                let p = &raw mut PENDING;
                let v = core::ptr::read_volatile(&(*p)[sig as usize]);
                if v != 0 {
                    core::ptr::write_volatile(&mut (*p)[sig as usize], 0);
                }
                v != 0
            };
            if hit {
                if let Some(action) = self.get_var(&format!("\u{1}trap{sig}")) {
                    let st = self.status;
                    self.run_source(&action);
                    self.status = st;
                }
            }
        }
    }

    pub fn run_exit_trap(&mut self) {
        if let Some(action) = self.vars.remove("\u{1}trap0") {
            let st = self.status;
            self.flow = Flow::None;
            self.run_source(&action.value);
            self.status = st;
        }
    }
}

fn trap(sh: &mut Shell, a: &[String]) -> i32 {
    if a.len() == 1 {
        for (k, v) in &sh.vars {
            if let Some(n) = k.strip_prefix("\u{1}trap") {
                let n: u32 = n.parse().unwrap_or(0);
                let name = if n == 0 { "EXIT" } else { signal::abbrev(n).unwrap_or("?") };
                println!("trap -- '{}' {}", v.value, name);
            }
        }
        return 0;
    }
    let (action, sigs) = if a.len() == 2 { ("-", &a[1..]) } else { (a[1].as_str(), &a[2..]) };
    let mut st = 0;
    for s in sigs {
        let n = if s == "EXIT" || s == "0" {
            0
        } else {
            match signal::from_name(s) {
                Some(n) if n != signal::SIGKILL && n != signal::SIGSTOP => n,
                _ => {
                    sh.error(&format!("trap: {s}: bad signal"));
                    st = 1;
                    continue;
                }
            }
        };
        let key = format!("\u{1}trap{n}");
        match action {
            "-" => {
                sh.vars.remove(&key);
                if n != 0 {
                    let _ = signal::signal(n, Handler::Default);
                }
            }
            "" => {
                sh.vars.remove(&key);
                if n != 0 {
                    let _ = signal::signal(n, Handler::Ignore);
                }
            }
            act => {
                sh.vars.insert(key, Var { value: act.into(), exported: false, readonly: false });
                if n != 0 {
                    let _ = signal::signal(n, Handler::Func(trap_handler));
                }
            }
        }
    }
    st
}

// ---- simple builtins --------------------------------------------------------------

fn echo(_: &mut Shell, a: &[String]) -> i32 {
    let mut newline = true;
    let mut escapes = false;
    let mut i = 1;
    while i < a.len() && a[i].starts_with('-') && a[i].len() > 1 && a[i][1..].chars().all(|c| matches!(c, 'n' | 'e' | 'E')) {
        for c in a[i][1..].chars() {
            match c {
                'n' => newline = false,
                'e' => escapes = true,
                _ => escapes = false,
            }
        }
        i += 1;
    }
    let mut out = String::new();
    for (k, w) in a[i..].iter().enumerate() {
        if k > 0 {
            out.push(' ');
        }
        if escapes {
            let (s, stop) = rt_unescape(w);
            out.push_str(&s);
            if stop {
                print!("{out}");
                return 0;
            }
        } else {
            out.push_str(w);
        }
    }
    if newline {
        out.push('\n');
    }
    print!("{out}");
    0
}

/// Interpret `\n`-style escapes; the bool is true if `\c` (stop output) was seen.
pub fn rt_unescape(s: &str) -> (String, bool) {
    let mut out = String::new();
    let mut it = s.chars().peekable();
    while let Some(c) = it.next() {
        if c != '\\' {
            out.push(c);
            continue;
        }
        match it.next() {
            Some('n') => out.push('\n'),
            Some('t') => out.push('\t'),
            Some('r') => out.push('\r'),
            Some('a') => out.push('\x07'),
            Some('b') => out.push('\x08'),
            Some('f') => out.push('\x0c'),
            Some('v') => out.push('\x0b'),
            Some('e') => out.push('\x1b'),
            Some('\\') => out.push('\\'),
            Some('c') => return (out, true),
            Some('0') => {
                let mut v = 0u32;
                for _ in 0..3 {
                    match it.peek() {
                        Some(d @ '0'..='7') => {
                            v = v * 8 + (*d as u32 - '0' as u32);
                            it.next();
                        }
                        _ => break,
                    }
                }
                out.push(char::from_u32(v).unwrap_or('?'));
            }
            Some(c) => {
                out.push('\\');
                out.push(c);
            }
            None => out.push('\\'),
        }
    }
    (out, false)
}

fn pwd(_: &mut Shell, _: &[String]) -> i32 {
    match rt::env::current_dir() {
        Ok(d) => {
            println!("{d}");
            0
        }
        Err(e) => {
            eprintln!("pwd: {e}");
            1
        }
    }
}

fn cd(sh: &mut Shell, a: &[String]) -> i32 {
    let mut print = false;
    let dir = match a.get(1).map(|s| s.as_str()) {
        None => sh.get_var("HOME").unwrap_or_else(|| "/".into()),
        Some("-") => {
            print = true;
            match sh.get_var("OLDPWD") {
                Some(d) => d,
                None => return usage(sh, "cd: OLDPWD not set"),
            }
        }
        Some(d) => d.into(),
    };
    let old = rt::env::current_dir().unwrap_or_default();
    if let Err(e) = rt::env::set_current_dir(&dir) {
        sh.error(&format!("cd: {dir}: {e}"));
        return 1;
    }
    let new = rt::env::current_dir().unwrap_or_default();
    let _ = sh.set_var("OLDPWD", &old);
    let _ = sh.set_var("PWD", &new);
    if print {
        println!("{new}");
    }
    0
}

fn exit(sh: &mut Shell, a: &[String]) -> i32 {
    let code = match a.get(1) {
        Some(s) => match s.parse::<i32>() {
            Ok(n) => n & 0xFF,
            Err(_) => return usage(sh, &format!("exit: {s}: numeric argument required")),
        },
        None => sh.status,
    };
    if sh.interactive && !sh.in_child && sh.jobs.iter().any(|j| j.stopped) && sh.get_var("\u{1}warned").is_none() {
        eprintln!("There are stopped jobs.");
        let _ = sh.set_var("\u{1}warned", "1");
        return 1;
    }
    sh.flow = Flow::Exit(code);
    code
}

fn export(sh: &mut Shell, a: &[String]) -> i32 {
    if a.len() == 1 || a[1] == "-p" {
        for (k, v) in &sh.vars {
            if v.exported {
                println!("export {k}='{}'", v.value.replace('\'', "'\\''"));
            }
        }
        return 0;
    }
    let mut st = 0;
    for w in &a[1..] {
        let (k, v) = match w.split_once('=') {
            Some((k, v)) => (k, Some(v)),
            None => (w.as_str(), None),
        };
        if !is_name(k) {
            sh.error(&format!("export: {k}: bad variable name"));
            st = 1;
            continue;
        }
        if let Some(v) = v {
            if let Err(e) = sh.set_var(k, v) {
                sh.error(&e);
                st = 1;
            }
        }
        sh.export(k);
    }
    st
}

fn readonly(sh: &mut Shell, a: &[String]) -> i32 {
    if a.len() == 1 {
        for (k, v) in &sh.vars {
            if v.readonly {
                println!("readonly {k}='{}'", v.value);
            }
        }
        return 0;
    }
    for w in &a[1..] {
        let (k, v) = match w.split_once('=') {
            Some((k, v)) => (k, Some(v)),
            None => (w.as_str(), None),
        };
        if let Some(v) = v {
            if let Err(e) = sh.set_var(k, v) {
                sh.error(&e);
                return 1;
            }
        }
        sh.vars.entry(k.into()).or_insert(Var { value: String::new(), exported: false, readonly: false }).readonly = true;
    }
    0
}

fn unset(sh: &mut Shell, a: &[String]) -> i32 {
    let mut funcs = false;
    let mut st = 0;
    for w in &a[1..] {
        match w.as_str() {
            "-f" => funcs = true,
            "-v" => funcs = false,
            name => {
                if funcs {
                    sh.funcs.remove(name);
                } else if let Err(e) = sh.unset_var(name) {
                    sh.error(&format!("unset: {e}"));
                    st = 1;
                }
            }
        }
    }
    st
}

fn set(sh: &mut Shell, a: &[String]) -> i32 {
    if a.len() == 1 {
        for (k, v) in &sh.vars {
            if !k.starts_with('\u{1}') {
                println!("{k}='{}'", v.value.replace('\'', "'\\''"));
            }
        }
        return 0;
    }
    let mut i = 1;
    while i < a.len() {
        let w = &a[i];
        if w == "--" {
            sh.params = a[i + 1..].to_vec();
            return 0;
        }
        let (on, flags) = match w.chars().next() {
            Some('-') if w.len() > 1 => (true, &w[1..]),
            Some('+') if w.len() > 1 => (false, &w[1..]),
            _ => {
                sh.params = a[i..].to_vec();
                return 0;
            }
        };
        if flags == "o" {
            i += 1;
            match a.get(i).map(|s| s.as_str()) {
                Some("errexit") => sh.opt_errexit = on,
                Some("xtrace") => sh.opt_xtrace = on,
                Some("noglob") => sh.opt_noglob = on,
                Some("nounset") => sh.opt_nounset = on,
                None => {
                    println!("errexit\t{}\nxtrace\t{}\nnoglob\t{}\nnounset\t{}", onoff(sh.opt_errexit), onoff(sh.opt_xtrace), onoff(sh.opt_noglob), onoff(sh.opt_nounset));
                }
                Some(o) => return usage(sh, &format!("set: {o}: unknown option")),
            }
        } else {
            for c in flags.chars() {
                match c {
                    'e' => sh.opt_errexit = on,
                    'x' => sh.opt_xtrace = on,
                    'f' => sh.opt_noglob = on,
                    'u' => sh.opt_nounset = on,
                    _ => return usage(sh, &format!("set: -{c}: unknown option")),
                }
            }
        }
        i += 1;
    }
    0
}

fn onoff(b: bool) -> &'static str {
    if b { "on" } else { "off" }
}

fn shift(sh: &mut Shell, a: &[String]) -> i32 {
    let n = a.get(1).and_then(|s| s.parse().ok()).unwrap_or(1usize);
    if n > sh.params.len() {
        sh.error("shift: can't shift that many");
        return 1;
    }
    sh.params.drain(..n);
    0
}

fn eval(sh: &mut Shell, a: &[String]) -> i32 {
    let src = a[1..].join(" ");
    sh.run_source(&src)
}

fn source(sh: &mut Shell, a: &[String]) -> i32 {
    let Some(file) = a.get(1) else { return usage(sh, ".: filename argument required") };
    let path = if file.contains('/') {
        file.clone()
    } else {
        sh.find_command(file).filter(|p| rt::fs::exists(p)).unwrap_or_else(|| file.clone())
    };
    let saved = if a.len() > 2 { Some(core::mem::replace(&mut sh.params, a[2..].to_vec())) } else { None };
    sh.func_depth += 1;
    let st = match sh.run_file(&path) {
        Ok(st) => st,
        Err(e) => {
            sh.error(&e);
            1
        }
    };
    sh.func_depth -= 1;
    if sh.flow == Flow::Return {
        sh.flow = Flow::None;
    }
    if let Some(p) = saved {
        sh.params = p;
    }
    st
}

pub fn exec_builtin(sh: &mut Shell, s: &Simple, words: &[String]) -> i32 {
    if words.len() == 1 {
        return match sh.apply_redirs(&s.redirs, false) {
            Ok(_) => 0,
            Err(e) => {
                sh.error(&e);
                1
            }
        };
    }
    if sh.interactive {
        for sig in [signal::SIGINT, signal::SIGQUIT, signal::SIGTSTP, signal::SIGTTIN, signal::SIGTTOU] {
            let _ = signal::signal(sig, Handler::Default);
        }
    }
    sh.exec_external(s, &words[1..])
}

fn ret(sh: &mut Shell, a: &[String]) -> i32 {
    if sh.func_depth == 0 {
        sh.error("return: can only `return' from a function or sourced script");
        return 1;
    }
    let st = a.get(1).and_then(|s| s.parse().ok()).unwrap_or(sh.status);
    sh.status = st;
    sh.flow = Flow::Return;
    st
}

fn brk(sh: &mut Shell, a: &[String]) -> i32 {
    if sh.loop_depth == 0 {
        return 0;
    }
    let n = a.get(1).and_then(|s| s.parse().ok()).unwrap_or(1u32).clamp(1, sh.loop_depth);
    sh.flow = Flow::Break(n);
    0
}

fn cont(sh: &mut Shell, a: &[String]) -> i32 {
    if sh.loop_depth == 0 {
        return 0;
    }
    let n = a.get(1).and_then(|s| s.parse().ok()).unwrap_or(1u32).clamp(1, sh.loop_depth);
    sh.flow = Flow::Continue(n);
    0
}

fn local(sh: &mut Shell, a: &[String]) -> i32 {
    if sh.locals.is_empty() {
        sh.error("local: can only be used in a function");
        return 1;
    }
    for w in &a[1..] {
        let (k, v) = match w.split_once('=') {
            Some((k, v)) => (k, Some(v)),
            None => (w.as_str(), None),
        };
        let old = sh.vars.get(k).cloned();
        if !sh.locals.last().unwrap().iter().any(|(n, _)| n == k) {
            sh.locals.last_mut().unwrap().push((k.into(), old));
        }
        let _ = sh.set_var(k, v.unwrap_or(""));
    }
    0
}

fn read(sh: &mut Shell, a: &[String]) -> i32 {
    let mut raw = false;
    let mut names = Vec::new();
    let mut i = 1;
    while i < a.len() {
        match a[i].as_str() {
            "-r" => raw = true,
            "-p" => {
                i += 1;
                if let Some(p) = a.get(i) {
                    eprint!("{p}");
                }
            }
            n => names.push(n.to_string()),
        }
        i += 1;
    }
    if names.is_empty() {
        names.push("REPLY".into());
    }
    io::stdout().flush_quiet();
    // read a byte at a time so that nothing past the line is consumed
    let mut line = Vec::new();
    let mut got_any = false;
    let mut eof = false;
    loop {
        let mut b = [0u8; 1];
        match io::read_fd(0, &mut b) {
            Ok(1) => {
                got_any = true;
                if b[0] == b'\n' {
                    break;
                }
                if b[0] == b'\\' && !raw {
                    let mut n = [0u8; 1];
                    if io::read_fd(0, &mut n) == Ok(1) {
                        if n[0] != b'\n' {
                            line.push(0x01); // marks an escaped character
                            line.push(n[0]);
                        }
                    }
                    continue;
                }
                line.push(b[0]);
            }
            _ => {
                eof = true;
                break;
            }
        }
    }
    let text = String::from_utf8_lossy(&line).into_owned();
    let ifs = sh.get_var("IFS").unwrap_or_else(|| " \t\n".into());
    let mut fields: Vec<String> = Vec::new();
    let mut cur = String::new();
    let mut esc = false;
    let mut rest_start = Vec::new();
    for (pos, c) in text.char_indices() {
        if c == '\u{1}' {
            esc = true;
            continue;
        }
        if !esc && ifs.contains(c) {
            if !cur.is_empty() {
                fields.push(core::mem::take(&mut cur));
                rest_start.push(pos);
            }
        } else {
            if cur.is_empty() && fields.len() == rest_start.len() {
                rest_start.push(pos);
            }
            cur.push(c);
        }
        esc = false;
    }
    if !cur.is_empty() {
        fields.push(cur);
    }
    for (k, name) in names.iter().enumerate() {
        let v = if k == names.len() - 1 && fields.len() > k {
            // the last variable gets the rest of the line
            let start = rest_start.get(k).copied().unwrap_or(0);
            text[start..].trim_end_matches(|c| ifs.contains(c)).replace('\u{1}', "")
        } else {
            fields.get(k).cloned().unwrap_or_default()
        };
        if let Err(e) = sh.set_var(name, &v) {
            sh.error(&e);
            return 1;
        }
    }
    if eof && !got_any { 1 } else if eof { (line.is_empty()) as i32 } else { 0 }
}

fn alias(sh: &mut Shell, a: &[String]) -> i32 {
    if a.len() == 1 {
        for (k, v) in &sh.aliases {
            println!("alias {k}='{v}'");
        }
        return 0;
    }
    let mut st = 0;
    for w in &a[1..] {
        match w.split_once('=') {
            Some((k, v)) => {
                sh.aliases.insert(k.into(), v.into());
            }
            None => match sh.aliases.get(w) {
                Some(v) => println!("alias {w}='{v}'"),
                None => {
                    sh.error(&format!("alias: {w}: not found"));
                    st = 1;
                }
            },
        }
    }
    st
}

fn unalias(sh: &mut Shell, a: &[String]) -> i32 {
    for w in &a[1..] {
        if w == "-a" {
            sh.aliases.clear();
        } else {
            sh.aliases.remove(w);
        }
    }
    0
}

fn describe(sh: &Shell, name: &str, short: bool) -> bool {
    if let Some(v) = sh.aliases.get(name) {
        if short {
            println!("alias {name}='{v}'");
        } else {
            println!("{name} is an alias for '{v}'");
        }
    } else if crate::parse::is_reserved(name) {
        println!("{}", if short { name.to_string() } else { format!("{name} is a shell keyword") });
    } else if sh.funcs.contains_key(name) {
        println!("{}", if short { name.to_string() } else { format!("{name} is a shell function") });
    } else if find(name).is_some() {
        println!("{}", if short { name.to_string() } else { format!("{name} is a shell builtin") });
    } else if let Some(p) = sh.find_command(name).filter(|p| rt::fs::exists(p)) {
        println!("{}", if short { p } else { format!("{name} is {p}") });
    } else {
        if !short {
            eprintln!("{name}: not found");
        }
        return false;
    }
    true
}

fn type_(sh: &mut Shell, a: &[String]) -> i32 {
    let mut st = 0;
    for n in &a[1..] {
        if !describe(sh, n, false) {
            st = 1;
        }
    }
    st
}

fn command(sh: &mut Shell, a: &[String]) -> i32 {
    match a.get(1).map(|s| s.as_str()) {
        Some("-v") | Some("-V") => {
            let short = a[1] == "-v";
            let mut st = 0;
            for n in &a[2..] {
                if !describe(sh, n, short) {
                    st = 1;
                }
            }
            st
        }
        None => 0,
        Some(_) => {
            // run without function lookup
            let words = a[1..].to_vec();
            if let Some(b) = find(&words[0]) {
                return b(sh, &words);
            }
            let s = Simple { assigns: Vec::new(), words: Vec::new(), redirs: Vec::new() };
            match process::fork() {
                Ok(0) => sh.exec_external(&s, &words),
                Ok(pid) => process::wait(pid).map(|s| s.shell_code()).unwrap_or(1),
                Err(e) => {
                    sh.error(&format!("fork: {e}"));
                    1
                }
            }
        }
    }
}

fn umask(sh: &mut Shell, a: &[String]) -> i32 {
    match a.get(1) {
        None => {
            let m = process::umask(0);
            process::umask(m);
            println!("{m:04o}");
            0
        }
        Some(s) => match u32::from_str_radix(s, 8) {
            Ok(m) if m <= 0o777 => {
                process::umask(m);
                0
            }
            _ => usage(sh, &format!("umask: {s}: invalid mode")),
        },
    }
}

fn history(sh: &mut Shell, a: &[String]) -> i32 {
    if a.get(1).is_some_and(|s| s == "-c") {
        sh.editor.history.clear();
        if let Some(f) = &sh.history_file {
            let _ = rt::fs::write(f, b"");
        }
        return 0;
    }
    let n = sh.editor.history.len();
    let count = a.get(1).and_then(|s| s.parse().ok()).unwrap_or(n);
    for (i, h) in sh.editor.history.iter().enumerate().skip(n.saturating_sub(count)) {
        println!("{:5}  {h}", i + 1);
    }
    0
}

fn help(_: &mut Shell, _: &[String]) -> i32 {
    println!("az030 sh built-in commands:\n");
    let mut names: Vec<&str> = NAMES.to_vec();
    names.sort();
    let mut line = String::new();
    for n in names {
        if line.len() + n.len() > 70 {
            println!("  {line}");
            line.clear();
        }
        line.push_str(n);
        line.push(' ');
    }
    if !line.is_empty() {
        println!("  {line}");
    }
    println!("\nSyntax: pipelines (|), lists (; & && ||), ( ) {{ }}, if/then/elif/else/fi,");
    println!("while/until/do/done, for/in/do/done, case/in/esac, name() {{ ... }},");
    println!("redirections (< > >> 2>&1 <<EOF), $var ${{var:-x}} $(cmd) $((expr)) * ? [ ].");
    println!("Line editing: arrows, ^A ^E ^K ^U ^W ^Y ^L, Tab completes; ^Z stops a job.");
    0
}

fn times(_: &mut Shell, _: &[String]) -> i32 {
    let (t, _) = process::times();
    let f = |x: u32| format!("{}m{}.{:02}s", x / 100 / 60, x / 100 % 60, x % 100);
    println!("{} {}\n{} {}", f(t.utime), f(t.stime), f(t.cutime), f(t.cstime));
    0
}

// ---- jobs -----------------------------------------------------------------------

/// Resolve a job spec (`%1`, `%+`, `%-`, `%name`, or nothing = current job).
fn job_index(sh: &mut Shell, spec: Option<&String>) -> Result<usize, String> {
    if sh.jobs.is_empty() {
        return Err("no current job".into());
    }
    let Some(spec) = spec else { return Ok(sh.jobs.len() - 1) };
    let s = spec.strip_prefix('%').ok_or_else(|| format!("{spec}: no such job"))?;
    let i = match s {
        "" | "+" | "%" => Some(sh.jobs.len() - 1),
        "-" => sh.jobs.len().checked_sub(2),
        _ => match s.parse::<u32>() {
            Ok(n) => sh.jobs.iter().position(|j| j.id == n),
            Err(_) => sh.jobs.iter().position(|j| j.text.starts_with(s)),
        },
    };
    i.ok_or_else(|| format!("{spec}: no such job"))
}

fn jobs(sh: &mut Shell, a: &[String]) -> i32 {
    sh.poll_jobs();
    let long = a.get(1).is_some_and(|s| s == "-l");
    let n = sh.jobs.len();
    for (i, j) in sh.jobs.iter().enumerate() {
        let mark = if i + 1 == n { '+' } else if i + 2 == n { '-' } else { ' ' };
        let state = if j.done() {
            "Done"
        } else if j.stopped {
            "Stopped"
        } else {
            "Running"
        };
        if long {
            println!("[{}]{mark} {} {:<10} {}", j.id, j.pgid, state, j.text);
        } else {
            println!("[{}]{mark}  {:<22}  {}", j.id, state, j.text);
        }
    }
    sh.reap_jobs(false);
    0
}

fn fg(sh: &mut Shell, a: &[String]) -> i32 {
    if !sh.job_control {
        sh.error("fg: no job control");
        return 1;
    }
    let i = match job_index(sh, a.get(1)) {
        Ok(i) => i,
        Err(e) => {
            sh.error(&format!("fg: {e}"));
            return 1;
        }
    };
    let j = sh.jobs.remove(i);
    println!("{}", j.text);
    let _ = rt::term::set_foreground(0, j.pgid);
    if j.stopped {
        let _ = process::kill(-(j.pgid as i32), signal::SIGCONT);
    }
    let status = j.status.iter().map(|s| s.filter(|s| !s.stopped())).collect();
    sh.wait_fg(j.pgid, &j.pids, status, &j.text)
}

fn bg(sh: &mut Shell, a: &[String]) -> i32 {
    let i = match job_index(sh, a.get(1)) {
        Ok(i) => i,
        Err(e) => {
            sh.error(&format!("bg: {e}"));
            return 1;
        }
    };
    let j = &mut sh.jobs[i];
    j.stopped = false;
    for s in j.status.iter_mut() {
        if s.is_some_and(|s| s.stopped()) {
            *s = None;
        }
    }
    println!("[{}]+ {} &", j.id, j.text);
    let _ = process::kill(-(j.pgid as i32), signal::SIGCONT);
    0
}

fn wait(sh: &mut Shell, a: &[String]) -> i32 {
    if a.len() == 1 {
        let mut st = 0;
        while let Some(j) = sh.jobs.first() {
            let (pgid, pids, text) = (j.pgid, j.pids.clone(), j.text.clone());
            let status = j.status.clone();
            sh.jobs.remove(0);
            let saved = sh.job_control;
            sh.job_control = false;
            st = sh.wait_fg(pgid, &pids, status, &text);
            sh.job_control = saved;
        }
        return st;
    }
    let mut st = 0;
    for w in &a[1..] {
        let pid = if w.starts_with('%') {
            match job_index(sh, Some(w)) {
                Ok(i) => *sh.jobs[i].pids.last().unwrap(),
                Err(e) => {
                    sh.error(&format!("wait: {e}"));
                    st = 127;
                    continue;
                }
            }
        } else {
            match w.parse() {
                Ok(p) => p,
                Err(_) => {
                    sh.error(&format!("wait: {w}: invalid process id"));
                    st = 2;
                    continue;
                }
            }
        };
        st = match process::wait(pid) {
            Ok(s) => s.shell_code(),
            Err(_) => 127,
        };
        for j in sh.jobs.iter_mut() {
            if let Some(k) = j.pids.iter().position(|p| *p == pid) {
                j.status[k] = Some(process::ExitStatus((st as u32 & 0xFF) << 8));
            }
        }
        sh.jobs.retain(|j| !j.done());
    }
    st
}

fn kill(sh: &mut Shell, a: &[String]) -> i32 {
    let mut sig = signal::SIGTERM;
    let mut i = 1;
    match a.get(1).map(|s| s.as_str()) {
        Some("-l") => {
            for (n, name) in signal::all() {
                println!("{n:2}) SIG{name}");
            }
            return 0;
        }
        Some("-s") => {
            match a.get(2).and_then(|s| signal::from_name(s)) {
                Some(s) => sig = s,
                None => return usage(sh, "kill: bad signal"),
            }
            i = 3;
        }
        Some(s) if s.starts_with('-') && s.len() > 1 => {
            match signal::from_name(&s[1..]) {
                Some(n) => sig = n,
                None => return usage(sh, &format!("kill: {s}: bad signal")),
            }
            i = 2;
        }
        _ => {}
    }
    if i >= a.len() {
        return usage(sh, "usage: kill [-s sig | -sig] pid | %job ...");
    }
    let mut st = 0;
    for w in &a[i..] {
        let target: i32 = if w.starts_with('%') {
            match job_index(sh, Some(w)) {
                Ok(k) => -(sh.jobs[k].pgid as i32),
                Err(e) => {
                    sh.error(&format!("kill: {e}"));
                    st = 1;
                    continue;
                }
            }
        } else {
            match w.parse() {
                Ok(p) => p,
                Err(_) => {
                    sh.error(&format!("kill: {w}: arguments must be process or job IDs"));
                    st = 1;
                    continue;
                }
            }
        };
        if let Err(e) = process::kill(target, sig) {
            sh.error(&format!("kill: {w}: {e}"));
            st = 1;
        } else if target < 0 && sig != signal::SIGCONT {
            let _ = process::kill(target, signal::SIGCONT);
        }
    }
    st
}
