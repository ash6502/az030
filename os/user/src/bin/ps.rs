//! ps: report processes.
//!
//!     ps [-ef] [-l] [aux]

#![no_std]
#![no_main]

use rt::prelude::*;

rt::main!(main);

fn state(s: u32) -> char {
    match s {
        azsys::PS_RUN => 'R',
        azsys::PS_SLEEP => 'S',
        azsys::PS_STOP => 'T',
        _ => 'Z',
    }
}

fn cputime(ticks: u32) -> String {
    let s = ticks / azsys::HZ;
    format!("{}:{:02}", s / 60, s % 60)
}

fn main(args: &[String]) -> i32 {
    let flags: String = args[1..].iter().map(|a| a.trim_start_matches('-')).collect();
    let full = flags.contains('f') || flags.contains('u');
    let long = flags.contains('l');
    let all = flags.contains('e') || flags.contains('a') || flags.contains('A') || flags.contains('x');
    let me = rt::process::id();
    let my_sid = rt::process::processes().iter().find(|p| p.pid == me).map_or(0, |p| p.sid);
    let procs = rt::process::processes();
    let si = rt::process::sysinfo();
    let mut out = String::new();
    if long {
        out.push_str("S   UID   PID  PPID  PGID   SID   MEM     TIME CMD\n");
    } else if full {
        out.push_str("USER       PID  PPID S   MEM  STIME     TIME CMD\n");
    } else {
        out.push_str("  PID TTY          TIME CMD\n");
    }
    for p in procs {
        if !all && p.sid != my_sid {
            continue;
        }
        let name = rt::fs::cstr_field(&p.name);
        let tty = if p.tty != 0 { "console" } else { "?" };
        let t = cputime(p.utime + p.stime);
        if long {
            out.push_str(&format!("{}  {:4} {:5} {:5} {:5} {:5} {:4}K {:>8} {}\n", state(p.state), p.uid, p.pid, p.ppid, p.pgid, p.sid, p.mem_kb, t, name));
        } else if full {
            let age = si.uptime.saturating_sub(p.start);
            let stime = if age < 3600 { format!("{}m{:02}s", age / 60, age % 60) } else { format!("{}h{:02}m", age / 3600, age / 60 % 60) };
            out.push_str(&format!("{:<8} {:5} {:5} {} {:4}K {:>6} {:>8} {}\n", rt::users::user_name(p.uid), p.pid, p.ppid, state(p.state), p.mem_kb, stime, t, name));
        } else {
            out.push_str(&format!("{:5} {:<8} {:>8} {}\n", p.pid, tty, t, name));
        }
    }
    print!("{out}");
    0
}
