//! halt / reboot / poweroff: stop the system.
//!
//! Sends SIGTERM to all processes, waits a moment, sends SIGKILL, syncs the disks
//! and asks the kernel to halt, restart or power off.

#![no_std]
#![no_main]

use rt::prelude::*;

rt::main!(main);

fn main(args: &[String]) -> i32 {
    let name = rt::env::progname();
    let cmd = match name {
        "reboot" => azsys::flags::REBOOT_RESTART,
        "poweroff" => azsys::flags::REBOOT_POWEROFF,
        _ => {
            if args.iter().any(|a| a == "-p") {
                azsys::flags::REBOOT_POWEROFF
            } else {
                azsys::flags::REBOOT_HALT
            }
        }
    };
    if rt::process::euid() != 0 {
        rt::die!("must be superuser");
    }
    let force = args.iter().any(|a| a == "-f");
    if !force {
        let me = rt::process::id();
        let _ = rt::signal::signal(rt::signal::SIGTERM, rt::signal::Handler::Ignore);
        let _ = rt::signal::signal(rt::signal::SIGHUP, rt::signal::Handler::Ignore);
        eprintln!("{name}: stopping processes...");
        for p in rt::process::processes() {
            if p.pid > 1 && p.pid != me {
                let _ = rt::process::kill(p.pid as i32, rt::signal::SIGTERM);
            }
        }
        let _ = rt::time::sleep_ms(1000);
        for p in rt::process::processes() {
            if p.pid > 1 && p.pid != me {
                let _ = rt::process::kill(p.pid as i32, rt::signal::SIGKILL);
            }
        }
    }
    rt::fs::sync();
    let e = rt::process::reboot(cmd);
    rt::die!("{e}")
}
