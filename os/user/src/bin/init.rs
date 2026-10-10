//! init: process 1.
//!
//! Runs /etc/rc, then keeps a login session on the console: each session is a new
//! process session whose leader opens /dev/console (making it the controlling
//! terminal) and runs /bin/login. Orphaned processes are reaped as they exit.
//!
//! The kernel command line option `single` (passed as an argument) skips /etc/rc
//! and login and starts a root shell directly.

#![no_std]
#![no_main]

use rt::prelude::*;
use rt::process;
use rt::signal::{self, Handler};

rt::main!(main);

const LOGIN: &str = "/bin/login";
const SHELL: &str = "/bin/sh";

fn child_signals() {
    for s in [signal::SIGINT, signal::SIGQUIT, signal::SIGTSTP, signal::SIGTTIN, signal::SIGTTOU, signal::SIGHUP, signal::SIGTERM] {
        let _ = signal::signal(s, Handler::Default);
    }
}

/// Run a command with the console as stdio and wait for it (reaping orphans).
fn run_wait(args: &[&str]) -> i32 {
    let args: Vec<String> = args.iter().map(|s| String::from(*s)).collect();
    match process::fork() {
        Ok(0) => {
            child_signals();
            let e = process::execve(&args[0], &args, &rt::env::environ());
            eprintln!("init: {}: {}", args[0], e);
            process::exit(127)
        }
        Ok(pid) => wait_for(pid),
        Err(e) => {
            eprintln!("init: fork: {e}");
            1
        }
    }
}

fn wait_for(pid: u32) -> i32 {
    loop {
        match process::waitpid(-1, 0) {
            Ok((p, st)) if p == pid => return st.shell_code(),
            Ok(_) => {}
            Err(_) => return 1,
        }
    }
}

/// Start a session on the console running `prog`.
fn session(prog: &str, arg0: &str) -> Option<u32> {
    match process::fork() {
        Ok(0) => {
            child_signals();
            let _ = process::setsid();
            for fd in 0..3 {
                let _ = rt::io::close(fd);
            }
            // the first terminal a session leader opens becomes its controlling terminal
            let con = match rt::fs::File::open_with("/dev/console", azsys::flags::O_RDWR, 0) {
                Ok(f) => f.into_raw(),
                Err(_) => process::exit(1),
            };
            for fd in 0..3 {
                if fd != con {
                    let _ = process::dup2(con, fd);
                }
            }
            if con > 2 {
                let _ = rt::io::close(con);
            }
            let _ = process::set_cloexec(0, false);
            let _ = rt::term::set_foreground(0, process::id());
            let args = vec![String::from(arg0)];
            let e = process::execve(prog, &args, &rt::env::environ());
            eprintln!("init: {prog}: {e}");
            process::exit(127)
        }
        Ok(pid) => Some(pid),
        Err(e) => {
            eprintln!("init: fork: {e}");
            None
        }
    }
}

fn main(args: &[String]) -> i32 {
    if process::id() != 1 {
        eprintln!("init: must be run as process 1 by the kernel");
        return 1;
    }
    for s in [signal::SIGINT, signal::SIGQUIT, signal::SIGTSTP, signal::SIGTTIN, signal::SIGTTOU, signal::SIGHUP, signal::SIGTERM] {
        let _ = signal::signal(s, Handler::Ignore);
    }
    rt::env::set_var("PATH", "/bin:/usr/bin:/sbin");
    rt::env::set_var("TERM", &rt::env::var("TERM").unwrap_or_else(|| "vt100".into()));
    let single = args.iter().any(|a| a == "single" || a == "-s");
    if !single && rt::fs::exists("/etc/rc") {
        run_wait(&[SHELL, "/etc/rc"]);
    }
    let (prog, arg0) = if single || !rt::fs::exists(LOGIN) {
        rt::env::set_var("HOME", "/root");
        rt::env::set_var("USER", "root");
        let _ = rt::env::set_current_dir("/root");
        (SHELL, "-sh")
    } else {
        (LOGIN, "login")
    };
    let mut quick_fails = 0;
    loop {
        let started = rt::time::now();
        let Some(pid) = session(prog, arg0) else {
            let _ = rt::time::sleep_ms(5000);
            continue;
        };
        let st = wait_for(pid);
        if rt::time::now().saturating_sub(started) < 2 && st != 0 {
            quick_fails += 1;
            if quick_fails >= 5 {
                eprintln!("init: {prog} is failing; waiting 30 seconds");
                let _ = rt::time::sleep_ms(30_000);
                quick_fails = 0;
            }
        } else {
            quick_fails = 0;
        }
    }
}
