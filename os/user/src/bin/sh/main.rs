//! sh: the az030 command interpreter.
//!
//! A POSIX-style shell: pipelines, lists, `&&`/`||`, subshells and groups, `if`,
//! `while`/`until`, `for`, `case`, functions, here-documents, parameter and
//! arithmetic expansion, command substitution, globbing, aliases, job control and
//! an interactive line editor with history and tab completion.
//!
//!     sh                      interactive (if stdin is a terminal)
//!     sh -c 'cmds' [arg0 args]
//!     sh script [args]
//!     sh -l / -sh             login shell: reads /etc/profile and ~/.profile first

#![no_std]
#![no_main]

extern crate alloc;

mod arith;
mod builtins;
mod exec;
mod expand;
mod parse;

use parse::{Node, Parser};
use rt::io::{self, Fd};
use rt::prelude::*;
use rt::process::ExitStatus;
use rt::readline::{Editor, Line};
use rt::signal::{self, Handler};

rt::main!(main);

#[derive(Clone)]
pub struct Var {
    pub value: String,
    pub exported: bool,
    pub readonly: bool,
}

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Flow {
    None,
    Break(u32),
    Continue(u32),
    Return,
    Exit(i32),
}

pub struct Job {
    pub id: u32,
    pub pgid: u32,
    pub pids: Vec<u32>,
    pub status: Vec<Option<ExitStatus>>,
    pub stopped: bool,
    pub text: String,
}

impl Job {
    pub fn done(&self) -> bool {
        self.status.iter().all(|s| s.is_some_and(|s| !s.stopped()))
    }
    pub fn last_status(&self) -> i32 {
        self.status.last().copied().flatten().map(|s| s.shell_code()).unwrap_or(0)
    }
}

pub struct Shell {
    pub vars: BTreeMap<String, Var>,
    pub params: Vec<String>,
    pub arg0: String,
    pub status: i32,
    /// status of the last command substitution in the current command
    pub subst_status: Option<i32>,
    pub pid: u32,
    pub last_bg: u32,
    pub funcs: BTreeMap<String, Rc<Node>>,
    pub aliases: BTreeMap<String, String>,
    pub jobs: Vec<Job>,
    pub interactive: bool,
    pub job_control: bool,
    /// this process is a forked child of the shell (subshell, pipeline element)
    pub in_child: bool,
    pub shell_pgid: u32,
    pub opt_errexit: bool,
    pub opt_xtrace: bool,
    pub opt_noglob: bool,
    pub opt_nounset: bool,
    pub flow: Flow,
    pub loop_depth: u32,
    pub func_depth: u32,
    /// inside a condition (if/while test, left of && ||, after !): no errexit
    pub cond_depth: u32,
    /// saved variables for `local`, one frame per function call
    pub locals: Vec<Vec<(String, Option<Var>)>>,
    pub editor: Editor,
    pub hostname: String,
    pub history_file: Option<String>,
    pub saved_termios: Option<rt::term::Termios>,
}

impl Shell {
    fn new() -> Shell {
        let mut vars = BTreeMap::new();
        for (k, v) in rt::env::vars() {
            vars.insert(k, Var { value: v, exported: true, readonly: false });
        }
        let hostname = rt::fs::read_to_string("/etc/hostname").map(|s| s.trim().into()).unwrap_or_else(|_| "az030".into());
        Shell {
            vars,
            params: Vec::new(),
            arg0: "sh".into(),
            status: 0,
            subst_status: None,
            pid: rt::process::id(),
            last_bg: 0,
            funcs: BTreeMap::new(),
            aliases: BTreeMap::new(),
            jobs: Vec::new(),
            interactive: false,
            job_control: false,
            in_child: false,
            shell_pgid: 0,
            opt_errexit: false,
            opt_xtrace: false,
            opt_noglob: false,
            opt_nounset: false,
            flow: Flow::None,
            loop_depth: 0,
            func_depth: 0,
            cond_depth: 0,
            locals: Vec::new(),
            editor: Editor::new(),
            hostname,
            history_file: None,
            saved_termios: None,
        }
    }

    pub fn get_var(&self, name: &str) -> Option<String> {
        self.vars.get(name).map(|v| v.value.clone())
    }

    pub fn set_var(&mut self, name: &str, value: &str) -> Result<(), String> {
        match self.vars.get_mut(name) {
            Some(v) if v.readonly => return Err(format!("{name}: readonly variable")),
            Some(v) => v.value = value.into(),
            None => {
                self.vars.insert(name.into(), Var { value: value.into(), exported: false, readonly: false });
            }
        }
        if name == "PATH" {
            rt::env::set_var("PATH", value);
        }
        Ok(())
    }

    pub fn unset_var(&mut self, name: &str) -> Result<(), String> {
        if self.vars.get(name).is_some_and(|v| v.readonly) {
            return Err(format!("{name}: readonly variable"));
        }
        self.vars.remove(name);
        Ok(())
    }

    pub fn export(&mut self, name: &str) {
        self.vars.entry(name.into()).or_insert(Var { value: String::new(), exported: true, readonly: false }).exported = true;
    }

    /// The environment for a new program.
    pub fn environ(&self) -> Vec<String> {
        self.vars.iter().filter(|(_, v)| v.exported).map(|(k, v)| format!("{k}={}", v.value)).collect()
    }

    pub fn error(&self, msg: &str) {
        if self.interactive || self.arg0 == "sh" || self.arg0 == "-sh" {
            eprintln!("sh: {msg}");
        } else {
            eprintln!("{}: {msg}", self.arg0);
        }
    }

    /// Parse and run source text; used for -c, scripts, eval and `.`.
    pub fn run_source(&mut self, src: &str) -> i32 {
        let aliases = self.aliases.clone();
        let mut p = Parser::new(src, true, Some(&aliases));
        loop {
            match p.next_command() {
                Ok(Some(n)) => {
                    self.exec(&n);
                    if self.flow != Flow::None {
                        break;
                    }
                }
                Ok(None) => break,
                Err(parse::Error::Syntax(e)) => {
                    self.error(&e);
                    self.status = 2;
                    if !self.interactive {
                        self.flow = Flow::Exit(2);
                    }
                    break;
                }
                Err(parse::Error::Incomplete) => {
                    self.error("unexpected end of file");
                    self.status = 2;
                    break;
                }
            }
        }
        self.status
    }

    pub fn run_file(&mut self, path: &str) -> Result<i32, String> {
        let text = rt::fs::read_to_string(path).map_err(|e| format!("{path}: {e}"))?;
        let text = if text.starts_with("#!") { text.split_once('\n').map(|x| x.1).unwrap_or("").into() } else { text };
        Ok(self.run_source(&text))
    }

    /// Terminate the shell.
    pub fn exit(&mut self, code: i32) -> ! {
        if !self.in_child {
            self.run_exit_trap();
        }
        if self.interactive && !self.in_child {
            for j in &self.jobs {
                let _ = rt::process::kill(-(j.pgid as i32), rt::signal::SIGHUP);
                if j.stopped {
                    let _ = rt::process::kill(-(j.pgid as i32), rt::signal::SIGCONT);
                }
            }
        }
        rt::process::exit(code)
    }

    fn prompt(&self, var: &str, default: &str) -> String {
        let ps = self.get_var(var).unwrap_or_else(|| default.into());
        let mut out = String::new();
        let mut it = ps.chars();
        while let Some(c) = it.next() {
            if c != '\\' {
                out.push(c);
                continue;
            }
            match it.next() {
                Some('u') => out.push_str(&self.get_var("USER").unwrap_or_else(|| rt::users::user_name(rt::process::uid()))),
                Some('h') | Some('H') => out.push_str(&self.hostname),
                Some('w') => {
                    let cwd = rt::env::current_dir().unwrap_or_else(|_| "?".into());
                    let home = self.get_var("HOME").unwrap_or_default();
                    if !home.is_empty() && home != "/" && (cwd == home || cwd.starts_with(&format!("{home}/"))) {
                        out.push('~');
                        out.push_str(&cwd[home.len()..]);
                    } else {
                        out.push_str(&cwd);
                    }
                }
                Some('W') => {
                    let cwd = rt::env::current_dir().unwrap_or_else(|_| "?".into());
                    out.push_str(rt::path::basename(&cwd));
                }
                Some('$') => out.push(if rt::process::euid() == 0 { '#' } else { '$' }),
                Some('n') => out.push('\n'),
                Some('e') => out.push('\x1b'),
                Some('[') | Some(']') => {}
                Some('\\') => out.push('\\'),
                Some(c) => {
                    out.push('\\');
                    out.push(c);
                }
                None => out.push('\\'),
            }
        }
        out
    }

    /// Names for completing a command word.
    fn command_names(&self, prefix: &str) -> Vec<String> {
        let mut v: Vec<String> = builtins::NAMES.iter().filter(|n| n.starts_with(prefix)).map(|s| String::from(*s)).collect();
        v.extend(self.funcs.keys().filter(|n| n.starts_with(prefix)).cloned());
        v.extend(self.aliases.keys().filter(|n| n.starts_with(prefix)).cloned());
        let path = self.get_var("PATH").unwrap_or_default();
        for dir in path.split(':').filter(|d| !d.is_empty()) {
            if let Ok(es) = rt::fs::read_dir(dir) {
                v.extend(es.into_iter().map(|e| e.name).filter(|n| n.starts_with(prefix) && !n.starts_with('.')));
            }
        }
        v.sort();
        v.dedup();
        v
    }

    fn complete(&self, line: &str, pos: usize) -> (usize, Vec<String>) {
        let before = &line[..pos];
        let start = before.rfind([' ', '\t', '|', ';', '&', '(', '<', '>']).map(|i| i + 1).unwrap_or(0);
        let word = &before[start..];
        let pre = before[..start].trim_end();
        let command_pos = pre.is_empty() || pre.ends_with(['|', ';', '&', '(']) || pre.ends_with("then") || pre.ends_with("do");
        if let Some(name) = word.strip_prefix('$') {
            let v = self.vars.keys().filter(|k| k.starts_with(name)).map(|k| format!("${k}")).collect();
            return (start, v);
        }
        if command_pos && !word.contains('/') {
            return (start, self.command_names(word));
        }
        let dirs_only = pre.split_whitespace().last().is_some_and(|w| w == "cd" || w == "rmdir");
        // expand ~ in the word for lookup, but keep it in the result
        if let Some(rest) = word.strip_prefix('~') {
            if rest.is_empty() || rest.starts_with('/') {
                let home = self.get_var("HOME").unwrap_or_default();
                let full = format!("{home}{rest}");
                let v = rt::readline::complete_path(&full, dirs_only).into_iter().map(|p| format!("~{}", &p[home.len()..])).collect();
                return (start, v);
            }
        }
        (start, rt::readline::complete_path(word, dirs_only))
    }

    /// Report finished and stopped background jobs.
    pub fn reap_jobs(&mut self, report: bool) {
        self.poll_jobs();
        let mut i = 0;
        while i < self.jobs.len() {
            if self.jobs[i].done() {
                let j = self.jobs.remove(i);
                if report && self.interactive {
                    let st = j.status.last().copied().flatten();
                    let what = match st {
                        Some(s) if s.success() => String::from("Done"),
                        Some(s) if s.code().is_some() => format!("Exit {}", s.code().unwrap()),
                        Some(s) => exec::signal_message(s.signal().unwrap_or(0)).into(),
                        None => "Done".into(),
                    };
                    eprintln!("[{}]  {:<24}{}", j.id, what, j.text);
                }
            } else {
                i += 1;
            }
        }
    }

    pub fn poll_jobs(&mut self) {
        use azsys::flags::{WNOHANG, WUNTRACED};
        for j in self.jobs.iter_mut() {
            for (k, pid) in j.pids.iter().enumerate() {
                if j.status[k].is_some_and(|s| !s.stopped()) {
                    continue;
                }
                if let Ok((p, st)) = rt::process::waitpid(*pid as i32, WNOHANG | WUNTRACED) {
                    if p != 0 {
                        j.status[k] = Some(st);
                        if st.stopped() {
                            j.stopped = true;
                        }
                    }
                } else {
                    // already reaped elsewhere
                    j.status[k] = Some(ExitStatus(0));
                }
            }
        }
    }

    fn interactive_loop(&mut self) -> ! {
        let mut buf = String::new();
        loop {
            self.run_traps();
            self.reap_jobs(true);
            let p = if buf.is_empty() { self.prompt("PS1", "\\u@\\h:\\w\\$ ") } else { self.prompt("PS2", "> ") };
            let mut ed = core::mem::take(&mut self.editor);
            let line = {
                let comp = |l: &str, pos: usize| self.complete(l, pos);
                ed.read_line(&p, Some(&comp))
            };
            self.editor = ed;
            match line {
                Line::Eof => {
                    if !buf.is_empty() {
                        buf.clear();
                        continue;
                    }
                    if !self.jobs.is_empty() && self.jobs.iter().any(|j| j.stopped) {
                        eprintln!("There are stopped jobs.");
                        self.jobs.retain(|j| !j.stopped || { let _ = rt::process::kill(-(j.pgid as i32), rt::signal::SIGKILL); false });
                        continue;
                    }
                    println!("logout");
                    self.exit(self.status);
                }
                Line::Interrupted => {
                    buf.clear();
                    self.status = 130;
                    continue;
                }
                Line::Text(l) => {
                    buf.push_str(&l);
                    buf.push('\n');
                }
            }
            let aliases = self.aliases.clone();
            let mut parser = Parser::new(&buf, false, Some(&aliases));
            let mut cmds = Vec::new();
            let mut incomplete = false;
            loop {
                match parser.next_command() {
                    Ok(Some(n)) => cmds.push(n),
                    Ok(None) => break,
                    Err(parse::Error::Incomplete) => {
                        incomplete = true;
                        break;
                    }
                    Err(parse::Error::Syntax(e)) => {
                        self.error(&e);
                        self.status = 2;
                        cmds.clear();
                        break;
                    }
                }
            }
            if incomplete {
                continue;
            }
            let text = core::mem::take(&mut buf);
            self.editor.add_history(text.trim_end_matches('\n'));
            self.append_history(text.trim_end_matches('\n'));
            for n in &cmds {
                self.exec(n);
                match self.flow {
                    Flow::Exit(c) => self.exit(c),
                    Flow::None => {}
                    _ => self.flow = Flow::None,
                }
            }
        }
    }

    fn load_history(&mut self) {
        let home = self.get_var("HOME").unwrap_or_default();
        if home.is_empty() {
            return;
        }
        let f = rt::path::join(&home, ".sh_history");
        if let Ok(t) = rt::fs::read_to_string(&f) {
            for l in t.lines() {
                self.editor.add_history(l);
            }
        }
        self.history_file = Some(f);
    }

    fn append_history(&self, line: &str) {
        if line.trim().is_empty() {
            return;
        }
        if let Some(f) = &self.history_file {
            if let Ok(mut fh) = rt::fs::File::append(f) {
                let _ = fh.write_all(format!("{}\n", line.replace('\n', " ")).as_bytes());
            }
        }
    }

    /// Put the shell in its own process group in the foreground of the terminal.
    fn init_job_control(&mut self) {
        let tty: Fd = 0;
        loop {
            let pg = rt::process::getpgid(0).unwrap_or(0);
            match rt::term::foreground(tty) {
                Ok(fg) if fg != pg && fg != 0 => {
                    let _ = rt::process::kill(-(pg as i32), rt::signal::SIGTTIN);
                }
                _ => break,
            }
        }
        for s in [rt::signal::SIGINT, rt::signal::SIGQUIT, rt::signal::SIGTSTP, rt::signal::SIGTTIN, rt::signal::SIGTTOU] {
            let _ = signal::signal(s, Handler::Ignore);
        }
        let pid = self.pid;
        let _ = rt::process::setpgid(0, 0);
        let _ = rt::term::set_foreground(tty, pid);
        self.shell_pgid = pid;
        self.job_control = true;
        self.saved_termios = rt::term::get_attr(tty).ok();
    }
}

fn usage() -> ! {
    eprintln!("usage: sh [-eilx] [-c command [arg0 [args...]] | script [args...]]");
    rt::process::exit(2)
}

fn main(args: &[String]) -> i32 {
    let mut sh = Shell::new();
    let mut login = args[0].starts_with('-');
    let mut command: Option<String> = None;
    let mut force_interactive = false;
    let mut i = 1;
    while i < args.len() && args[i].starts_with('-') && args[i] != "-" && command.is_none() {
        let a = &args[i];
        i += 1;
        if a == "--" {
            break;
        }
        for c in a[1..].chars() {
            match c {
                'c' => {
                    let Some(cmd) = args.get(i) else { usage() };
                    command = Some(cmd.clone());
                    i += 1;
                }
                'l' => login = true,
                'i' => force_interactive = true,
                'e' => sh.opt_errexit = true,
                'x' => sh.opt_xtrace = true,
                'f' => sh.opt_noglob = true,
                'u' => sh.opt_nounset = true,
                's' => {}
                _ => usage(),
            }
        }
    }
    let rest = &args[i.min(args.len())..];
    if !sh.vars.contains_key("PATH") {
        let _ = sh.set_var("PATH", "/bin:/usr/bin:/sbin");
        sh.export("PATH");
    }
    let _ = sh.set_var("PPID", &format!("{}", rt::process::parent_id()));
    if let Some(cmd) = command {
        if let Some(a0) = rest.first() {
            sh.arg0 = a0.clone();
            sh.params = rest[1..].to_vec();
        }
        let st = sh.run_source(&cmd);
        if let Flow::Exit(c) = sh.flow {
            sh.exit(c);
        }
        sh.exit(st);
    }
    if let Some(script) = rest.first() {
        sh.arg0 = script.clone();
        sh.params = rest[1..].to_vec();
        let st = match sh.run_file(script) {
            Ok(s) => s,
            Err(e) => {
                sh.error(&e);
                127
            }
        };
        if let Flow::Exit(c) = sh.flow {
            sh.exit(c);
        }
        sh.exit(st);
    }
    sh.arg0 = args[0].clone();
    sh.interactive = force_interactive || (io::isatty(0) && io::isatty(2));
    if sh.interactive {
        sh.init_job_control();
    }
    if login {
        for f in ["/etc/profile".into(), rt::path::join(&sh.get_var("HOME").unwrap_or_default(), ".profile")] {
            if rt::fs::exists(&f) {
                let _ = sh.run_file(&f);
                if let Flow::Exit(c) = sh.flow {
                    sh.exit(c);
                }
                sh.flow = Flow::None;
            }
        }
    }
    if sh.interactive {
        if let Some(env) = sh.get_var("ENV") {
            if rt::fs::exists(&env) {
                let _ = sh.run_file(&env);
                sh.flow = Flow::None;
            }
        }
        sh.load_history();
        sh.interactive_loop();
    }
    // non-interactive standard input: read it all as a script
    let mut text = String::new();
    let _ = io::stdin().read_to_string(&mut text);
    let st = sh.run_source(&text);
    if let Flow::Exit(c) = sh.flow {
        sh.exit(c);
    }
    sh.exit(st)
}
