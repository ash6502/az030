//! Executing syntax trees.

use crate::parse::{Item, Node, Redir, RedirKind, Simple};
use crate::{builtins, Flow, Job, Shell, Var};
use azsys::flags::*;
use rt::io::{self, Fd};
use rt::prelude::*;
use rt::process::{self, ExitStatus};
use rt::signal::{self, Handler};
use rt::sys::{errno, Errno};

pub fn signal_message(sig: u32) -> &'static str {
    use rt::signal::*;
    match sig {
        SIGHUP => "Hangup",
        SIGINT => "Interrupt",
        SIGQUIT => "Quit",
        SIGILL => "Illegal instruction",
        SIGTRAP => "Trace/breakpoint trap",
        SIGABRT => "Aborted",
        SIGBUS => "Bus error",
        SIGFPE => "Floating point exception",
        SIGKILL => "Killed",
        SIGUSR1 => "User defined signal 1",
        SIGSEGV => "Segmentation fault",
        SIGUSR2 => "User defined signal 2",
        SIGPIPE => "Broken pipe",
        SIGALRM => "Alarm clock",
        SIGTERM => "Terminated",
        SIGSTOP | SIGTSTP | SIGTTIN | SIGTTOU => "Stopped",
        _ => "Killed by signal",
    }
}

/// Saved descriptors to restore after a builtin's or compound command's redirections.
pub type Saved = Vec<(Fd, Option<Fd>)>;

impl Shell {
    pub fn exec(&mut self, n: &Node) -> i32 {
        let st = match n {
            Node::Simple(s) => self.exec_simple(s, false),
            Node::Pipe(cmds, bang) => {
                if *bang {
                    self.cond_depth += 1;
                }
                let st = if cmds.len() == 1 { self.exec(&cmds[0]) } else { self.exec_pipe(cmds, false, "") };
                if *bang {
                    self.cond_depth -= 1;
                    (st == 0) as i32
                } else {
                    st
                }
            }
            Node::AndOr(first, rest) => {
                self.cond_depth += 1;
                let mut st = self.exec(first);
                for (k, (is_and, n)) in rest.iter().enumerate() {
                    if self.flow != Flow::None {
                        break;
                    }
                    if (st == 0) == *is_and {
                        if k == rest.len() - 1 {
                            self.cond_depth -= 1;
                            st = self.exec(n);
                            self.cond_depth += 1;
                        } else {
                            st = self.exec(n);
                        }
                    }
                }
                self.cond_depth -= 1;
                st
            }
            Node::List(items) => self.exec_list(items),
            Node::Subshell(body, redirs) => self.exec_subshell(body, redirs),
            Node::Group(body, redirs) => self.with_redirs(redirs, |sh| sh.exec(body)),
            Node::If(branches, else_part, redirs) => self.with_redirs(redirs, |sh| {
                for (cond, body) in branches {
                    sh.cond_depth += 1;
                    let c = sh.exec(cond);
                    sh.cond_depth -= 1;
                    if sh.flow != Flow::None {
                        return c;
                    }
                    if c == 0 {
                        return sh.exec(body);
                    }
                }
                match else_part {
                    Some(e) => sh.exec(e),
                    None => 0,
                }
            }),
            Node::While(cond, body, until, redirs) => self.with_redirs(redirs, |sh| {
                let mut st = 0;
                sh.loop_depth += 1;
                loop {
                    sh.cond_depth += 1;
                    let c = sh.exec(cond);
                    sh.cond_depth -= 1;
                    if sh.flow != Flow::None || (c == 0) == *until {
                        break;
                    }
                    st = sh.exec(body);
                    if sh.loop_flow() {
                        break;
                    }
                }
                sh.loop_depth -= 1;
                st
            }),
            Node::For(name, words, body, redirs) => self.with_redirs(redirs, |sh| {
                let items = match words {
                    Some(w) => match sh.expand_words(w) {
                        Ok(v) => v,
                        Err(e) => {
                            sh.error(&e);
                            return 1;
                        }
                    },
                    None => sh.params.clone(),
                };
                let mut st = 0;
                sh.loop_depth += 1;
                for it in items {
                    if let Err(e) = sh.set_var(name, &it) {
                        sh.error(&e);
                        st = 1;
                        break;
                    }
                    st = sh.exec(body);
                    if sh.loop_flow() {
                        break;
                    }
                }
                sh.loop_depth -= 1;
                st
            }),
            Node::Case(word, arms, redirs) => self.with_redirs(redirs, |sh| {
                let w = match sh.expand_one(word) {
                    Ok(w) => w,
                    Err(e) => {
                        sh.error(&e);
                        return 1;
                    }
                };
                for (pats, body) in arms {
                    for p in pats {
                        let pat = sh.expand_pattern(p).unwrap_or_default();
                        if rt::glob::matches(&pat, &w) {
                            return sh.exec(body);
                        }
                    }
                }
                0
            }),
            Node::Func(name, body) => {
                self.funcs.insert(name.clone(), body.clone());
                0
            }
        };
        self.status = st;
        st
    }

    /// After a loop body: handle break/continue. Returns true to leave the loop.
    fn loop_flow(&mut self) -> bool {
        match self.flow {
            Flow::Break(n) => {
                self.flow = if n > 1 { Flow::Break(n - 1) } else { Flow::None };
                true
            }
            Flow::Continue(n) => {
                if n > 1 {
                    self.flow = Flow::Continue(n - 1);
                    true
                } else {
                    self.flow = Flow::None;
                    false
                }
            }
            Flow::None => false,
            _ => true,
        }
    }

    fn exec_list(&mut self, items: &[Item]) -> i32 {
        let mut st = self.status;
        for it in items {
            if it.background {
                st = self.exec_background(&it.node, &it.text);
            } else {
                st = self.exec(&it.node);
            }
            self.run_traps();
            if self.flow != Flow::None {
                break;
            }
            self.check_errexit(st);
        }
        st
    }

    pub fn check_errexit(&mut self, st: i32) {
        if self.opt_errexit && st != 0 && self.cond_depth == 0 && self.flow == Flow::None {
            self.flow = Flow::Exit(st);
        }
    }

    /// Become a child process: default signal handling, no job control.
    fn child_setup(&mut self, pgid: u32, foreground: bool) {
        if self.job_control {
            let me = process::id();
            let _ = process::setpgid(0, if pgid == 0 { me } else { pgid });
            if foreground {
                let _ = rt::term::set_foreground(0, if pgid == 0 { me } else { pgid });
            }
        }
        if self.interactive {
            for s in [signal::SIGINT, signal::SIGQUIT, signal::SIGTSTP, signal::SIGTTIN, signal::SIGTTOU] {
                let _ = signal::signal(s, Handler::Default);
            }
        }
        self.in_child = true;
        self.interactive = false;
        self.job_control = false;
        self.jobs.clear();
        self.pid = process::id();
    }

    /// Fork for a job member. Returns Ok(0) in the child.
    fn fork_member(&mut self, pgid: &mut u32, foreground: bool) -> Result<u32, Errno> {
        let pid = process::fork()?;
        if pid == 0 {
            self.child_setup(*pgid, foreground);
            return Ok(0);
        }
        if self.job_control {
            let _ = process::setpgid(pid, if *pgid == 0 { pid } else { *pgid });
        }
        if *pgid == 0 {
            *pgid = pid;
        }
        Ok(pid)
    }

    /// Finish a child: flush output and exit with the status (or the `exit` code).
    fn child_exit(&mut self, st: i32) -> ! {
        let code = match self.flow {
            Flow::Exit(c) => c,
            _ => st,
        };
        process::exit(code)
    }

    fn exec_background(&mut self, n: &Node, text: &str) -> i32 {
        if let Node::Pipe(cmds, false) = n {
            if cmds.len() > 1 {
                return self.exec_pipe(cmds, true, text);
            }
        }
        let mut pgid = 0;
        match self.fork_member(&mut pgid, false) {
            Ok(0) => {
                if !self.job_control {
                    // background jobs without job control read from /dev/null
                    if let Ok(f) = rt::fs::File::open("/dev/null") {
                        let _ = process::dup2(f.fd(), 0);
                    }
                }
                let st = self.exec(n);
                self.child_exit(st)
            }
            Ok(pid) => {
                self.add_job(pgid, alloc::vec![pid], text, false);
                0
            }
            Err(e) => {
                self.error(&format!("fork: {e}"));
                1
            }
        }
    }

    fn add_job(&mut self, pgid: u32, pids: Vec<u32>, text: &str, stopped: bool) -> u32 {
        let id = self.jobs.iter().map(|j| j.id).max().unwrap_or(0) + 1;
        let n = pids.len();
        self.last_bg = *pids.last().unwrap_or(&0);
        if self.interactive && !stopped {
            eprintln!("[{id}] {}", self.last_bg);
        }
        self.jobs.push(Job { id, pgid, pids, status: alloc::vec![None; n], stopped, text: text.into() });
        id
    }

    /// Wait for a foreground job; returns its status. A stopped job joins the job table.
    pub fn wait_fg(&mut self, pgid: u32, pids: &[u32], mut status: Vec<Option<ExitStatus>>, text: &str) -> i32 {
        let mut stopped = false;
        for (k, &pid) in pids.iter().enumerate() {
            if status[k].is_some_and(|s| !s.stopped()) {
                continue;
            }
            let opts = if self.job_control { WUNTRACED } else { 0 };
            match process::waitpid(pid as i32, opts) {
                Ok((_, st)) => {
                    status[k] = Some(st);
                    if st.stopped() {
                        stopped = true;
                        break;
                    }
                }
                Err(_) => status[k] = Some(ExitStatus(0)),
            }
        }
        if self.job_control {
            let _ = rt::term::set_foreground(0, self.shell_pgid);
            if let Some(t) = &self.saved_termios {
                let _ = rt::term::set_attr(0, t);
            }
        }
        if stopped {
            let id = self.add_job(pgid, pids.to_vec(), text, true);
            if let Some(j) = self.jobs.iter_mut().find(|j| j.id == id) {
                j.status = status;
            }
            eprintln!("\n[{id}]+  Stopped                 {text}");
            return 128 + signal::SIGTSTP as i32;
        }
        let last = status.last().copied().flatten().unwrap_or(ExitStatus(0));
        if let Some(sig) = last.signal() {
            if sig == signal::SIGINT {
                if self.interactive {
                    eprintln!();
                }
            } else if sig != signal::SIGPIPE {
                eprintln!("{}{}", signal_message(sig), if last.core_dumped() { " (core dumped)" } else { "" });
            }
        }
        last.shell_code()
    }

    fn exec_pipe(&mut self, cmds: &[Node], background: bool, text: &str) -> i32 {
        let mut prev: Option<Fd> = None;
        let mut pgid = 0;
        let mut pids = Vec::new();
        for (i, cmd) in cmds.iter().enumerate() {
            let last = i == cmds.len() - 1;
            let pipe = if last {
                None
            } else {
                match process::pipe() {
                    Ok(p) => Some(p),
                    Err(e) => {
                        self.error(&format!("pipe: {e}"));
                        break;
                    }
                }
            };
            match self.fork_member(&mut pgid, !background) {
                Ok(0) => {
                    if let Some(r) = prev {
                        let _ = process::dup2(r, 0);
                        let _ = io::close(r);
                    }
                    if let Some((r, w)) = pipe {
                        let _ = process::dup2(w, 1);
                        let _ = io::close(w);
                        let _ = io::close(r);
                    }
                    let st = match cmd {
                        Node::Simple(s) => self.exec_simple(s, true),
                        n => self.exec(n),
                    };
                    self.child_exit(st);
                }
                Ok(pid) => pids.push(pid),
                Err(e) => {
                    self.error(&format!("fork: {e}"));
                    break;
                }
            }
            if let Some(r) = prev.take() {
                let _ = io::close(r);
            }
            if let Some((r, w)) = pipe {
                let _ = io::close(w);
                prev = Some(r);
            }
        }
        if let Some(r) = prev {
            let _ = io::close(r);
        }
        if pids.is_empty() {
            return 1;
        }
        if background {
            self.add_job(pgid, pids, text, false);
            return 0;
        }
        let n = pids.len();
        let text = if text.is_empty() { "pipeline" } else { text };
        self.wait_fg(pgid, &pids, alloc::vec![None; n], text)
    }

    fn exec_subshell(&mut self, body: &Node, redirs: &[Redir]) -> i32 {
        let mut pgid = 0;
        match self.fork_member(&mut pgid, true) {
            Ok(0) => {
                if let Err(e) = self.apply_redirs(redirs, false) {
                    self.error(&e);
                    process::exit(1);
                }
                let st = self.exec(body);
                self.child_exit(st)
            }
            Ok(pid) => self.wait_fg(pgid, &[pid], alloc::vec![None], "( ... )"),
            Err(e) => {
                self.error(&format!("fork: {e}"));
                1
            }
        }
    }

    fn with_redirs(&mut self, redirs: &[Redir], f: impl FnOnce(&mut Shell) -> i32) -> i32 {
        if redirs.is_empty() {
            return f(self);
        }
        match self.apply_redirs(redirs, true) {
            Ok(saved) => {
                let st = f(self);
                self.restore(saved);
                st
            }
            Err(e) => {
                self.error(&e);
                1
            }
        }
    }

    /// Open a file for a redirection (close-on-exec, moved into place by the caller).
    fn open_redir(&mut self, r: &Redir) -> Result<Option<Fd>, String> {
        if let RedirKind::HereDoc(body, expand) = &r.kind {
            let text = if *expand { self.expand_heredoc(&body.borrow())? } else { body.borrow().clone() };
            let (rd, wr) = process::pipe().map_err(|e| format!("pipe: {e}"))?;
            // small documents fit in the pipe; larger ones are fed by a child
            if text.len() <= 4096 {
                let _ = io::write_fd(wr, text.as_bytes());
                let _ = io::close(wr);
            } else {
                match process::fork() {
                    Ok(0) => {
                        let _ = io::close(rd);
                        let _ = io::write_fd(wr, text.as_bytes());
                        process::exit(0);
                    }
                    _ => {
                        let _ = io::close(wr);
                    }
                }
            }
            return Ok(Some(rd));
        }
        let target = self.expand_one(&r.target)?;
        let (flags, mode) = match r.kind {
            RedirKind::In => (O_RDONLY, 0),
            RedirKind::Out | RedirKind::Clobber => (O_WRONLY | O_CREAT | O_TRUNC, 0o666),
            RedirKind::Append => (O_WRONLY | O_CREAT | O_APPEND, 0o666),
            RedirKind::ReadWrite => (O_RDWR | O_CREAT, 0o666),
            RedirKind::Dup => {
                if target == "-" {
                    return Ok(None);
                }
                let n: Fd = target.parse().map_err(|_| format!("{target}: ambiguous redirect"))?;
                let d = process::dup_above(n, 10).map_err(|e| format!("{n}: {e}"))?;
                return Ok(Some(d));
            }
            RedirKind::HereDoc(..) => unreachable!(),
        };
        match rt::fs::File::open_with(&target, flags, mode) {
            Ok(f) => Ok(Some(f.into_raw())),
            Err(e) => Err(format!("{target}: {e}")),
        }
    }

    /// Apply redirections; with `save`, returns what is needed to undo them.
    pub fn apply_redirs(&mut self, redirs: &[Redir], save: bool) -> Result<Saved, String> {
        io::stdout().flush_quiet();
        let mut saved: Saved = Vec::new();
        for r in redirs {
            let fd = match self.open_redir(r) {
                Ok(fd) => fd,
                Err(e) => {
                    self.restore(saved);
                    return Err(e);
                }
            };
            if save && !saved.iter().any(|(t, _)| *t == r.fd) {
                let old = process::dup_above(r.fd, 10).ok();
                saved.push((r.fd, old));
            }
            match fd {
                Some(fd) => {
                    let _ = process::dup2(fd, r.fd);
                    if fd != r.fd {
                        let _ = io::close(fd);
                    } else {
                        let _ = process::set_cloexec(fd, false);
                    }
                }
                None => {
                    let _ = io::close(r.fd);
                }
            }
        }
        Ok(saved)
    }

    pub fn restore(&mut self, saved: Saved) {
        io::stdout().flush_quiet();
        for (fd, old) in saved.into_iter().rev() {
            match old {
                Some(o) => {
                    let _ = process::dup2(o, fd);
                    let _ = io::close(o);
                }
                None => {
                    let _ = io::close(fd);
                }
            }
        }
    }

    /// `$(...)`: run in a child, collect its output.
    pub fn command_subst(&mut self, cmd: &str) -> String {
        let Ok((r, w)) = process::pipe() else {
            self.error("pipe failed");
            return String::new();
        };
        match process::fork() {
            Ok(0) => {
                let _ = io::close(r);
                let _ = process::dup2(w, 1);
                let _ = io::close(w);
                self.child_setup(0, false);
                let st = self.run_source(cmd);
                self.child_exit(st)
            }
            Ok(pid) => {
                let _ = io::close(w);
                let mut out = Vec::new();
                let _ = rt::io::FdIo(r).read_to_end(&mut out);
                let _ = io::close(r);
                if let Ok(st) = process::wait(pid) {
                    self.subst_status = Some(st.shell_code());
                }
                let mut s = String::from_utf8_lossy(&out).into_owned();
                while s.ends_with('\n') {
                    s.pop();
                }
                s
            }
            Err(e) => {
                let _ = io::close(r);
                let _ = io::close(w);
                self.error(&format!("fork: {e}"));
                String::new()
            }
        }
    }

    fn assign_all(&mut self, assigns: &[(String, String)]) -> Result<(), String> {
        for (k, v) in assigns {
            let val = self.expand_one(v)?;
            self.set_var(k, &val)?;
        }
        Ok(())
    }

    pub fn exec_simple(&mut self, s: &Simple, in_child: bool) -> i32 {
        self.subst_status = None;
        let words = match self.expand_words(&s.words) {
            Ok(w) => w,
            Err(e) => {
                self.error(&e);
                if !self.interactive {
                    self.flow = Flow::Exit(1);
                }
                return 1;
            }
        };
        if words.is_empty() {
            // assignments and/or redirections only
            if let Err(e) = self.assign_all(&s.assigns) {
                self.error(&e);
                return 1;
            }
            // the status is that of the last command substitution, if any
            let st = self.subst_status.unwrap_or(0);
            return match self.apply_redirs(&s.redirs, true) {
                Ok(saved) => {
                    self.restore(saved);
                    st
                }
                Err(e) => {
                    self.error(&e);
                    1
                }
            };
        }
        if self.opt_xtrace {
            let mut line = String::from("+");
            for (k, v) in &s.assigns {
                line.push_str(&format!(" {k}={v}"));
            }
            for w in &words {
                line.push(' ');
                line.push_str(w);
            }
            eprintln!("{line}");
        }
        let name = words[0].as_str();
        if let Some(body) = self.funcs.get(name).cloned() {
            return self.with_temp_assigns(&s.assigns, |sh| sh.with_redirs(&s.redirs, |sh| sh.call_function(&body, &words)));
        }
        if name == "exec" {
            return builtins::exec_builtin(self, s, &words);
        }
        if let Some(b) = builtins::find(name) {
            return self.with_temp_assigns(&s.assigns, |sh| match sh.apply_redirs(&s.redirs, true) {
                Ok(saved) => {
                    let st = b(sh, &words);
                    sh.restore(saved);
                    st
                }
                Err(e) => {
                    sh.error(&e);
                    1
                }
            });
        }
        // an external program
        if in_child {
            self.exec_external(s, &words);
        }
        let mut pgid = 0;
        match self.fork_member(&mut pgid, true) {
            Ok(0) => self.exec_external(s, &words),
            Ok(pid) => self.wait_fg(pgid, &[pid], alloc::vec![None], &words.join(" ")),
            Err(e) => {
                self.error(&format!("fork: {e}"));
                1
            }
        }
    }

    fn with_temp_assigns(&mut self, assigns: &[(String, String)], f: impl FnOnce(&mut Shell) -> i32) -> i32 {
        if assigns.is_empty() {
            return f(self);
        }
        let old: Vec<(String, Option<Var>)> = assigns.iter().map(|(k, _)| (k.clone(), self.vars.get(k).cloned())).collect();
        if let Err(e) = self.assign_all(assigns) {
            self.error(&e);
            return 1;
        }
        for (k, _) in assigns {
            self.export(k);
        }
        let st = f(self);
        for (k, v) in old {
            match v {
                Some(v) => {
                    self.vars.insert(k, v);
                }
                None => {
                    self.vars.remove(&k);
                }
            }
        }
        st
    }

    pub fn call_function(&mut self, body: &Node, words: &[String]) -> i32 {
        if self.func_depth >= 200 {
            self.error(&format!("{}: maximum function nesting exceeded", words[0]));
            return 1;
        }
        let saved = core::mem::replace(&mut self.params, words[1..].to_vec());
        self.func_depth += 1;
        self.locals.push(Vec::new());
        let st = self.exec(body);
        for (k, v) in self.locals.pop().unwrap_or_default().into_iter().rev() {
            match v {
                Some(v) => {
                    self.vars.insert(k, v);
                }
                None => {
                    self.vars.remove(&k);
                }
            }
        }
        self.func_depth -= 1;
        self.params = saved;
        let st = if self.flow == Flow::Return {
            self.flow = Flow::None;
            self.status
        } else {
            st
        };
        self.status = st;
        st
    }

    /// Replace this (child) process with an external program.
    pub fn exec_external(&mut self, s: &Simple, words: &[String]) -> ! {
        if let Err(e) = self.apply_redirs(&s.redirs, false) {
            self.error(&e);
            process::exit(1);
        }
        if let Err(e) = self.assign_all(&s.assigns) {
            self.error(&e);
            process::exit(1);
        }
        for (k, _) in &s.assigns {
            self.export(k);
        }
        let name = &words[0];
        let Some(path) = self.find_command(name) else {
            self.error(&format!("{name}: not found"));
            process::exit(127)
        };
        let env = self.environ();
        let e = process::execve(&path, words, &env);
        if e.0 == errno::ENOEXEC {
            // no #! line: run it as a shell script in this process
            self.arg0 = path.clone();
            self.params = words[1..].to_vec();
            self.funcs.clear();
            let st = self.run_file(&path).unwrap_or(126);
            self.child_exit(st);
        }
        self.error(&format!("{name}: {e}"));
        process::exit(if e.0 == errno::ENOENT { 127 } else { 126 })
    }

    pub fn find_command(&self, name: &str) -> Option<String> {
        if name.contains('/') {
            return Some(name.into());
        }
        let path = self.get_var("PATH").unwrap_or_default();
        for dir in path.split(':') {
            let p = rt::path::join(if dir.is_empty() { "." } else { dir }, name);
            if let Ok(m) = rt::fs::metadata(&p) {
                if m.is_file() && m.mode() & 0o111 != 0 {
                    return Some(p);
                }
            }
        }
        None
    }
}
