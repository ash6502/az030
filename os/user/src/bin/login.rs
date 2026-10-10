//! login: sign on to the system.
//!
//!     login [user]
//!
//! Prints /etc/issue, asks for a user name and password (checked against
//! /etc/shadow), prints /etc/motd and starts the user's shell as a login shell.

#![no_std]
#![no_main]

use rt::prelude::*;
use rt::users;

rt::main!(main);

fn main(args: &[String]) -> i32 {
    if rt::process::euid() != 0 {
        eprintln!("login: must be run as root (try su)");
        return 1;
    }
    let host = rt::fs::read_to_string("/etc/hostname").map(|s| s.trim().into()).unwrap_or_else(|_| String::from("az030"));
    if let Ok(issue) = rt::fs::read_to_string("/etc/issue") {
        print!("\n{}", issue.replace("\\n", &host));
    }
    let mut preset = args.get(1).cloned();
    let mut failures = 0;
    loop {
        let name = match preset.take() {
            Some(n) => n,
            None => {
                print!("\n{host} login: ");
                match rt::io::stdin().line() {
                    Some(l) => l.trim().into(),
                    None => return 1,
                }
            }
        };
        if name.is_empty() {
            continue;
        }
        let user = users::by_name(&name);
        let ok = match &user {
            Some(_) if !users::has_password(&name) => true,
            _ => {
                let pw = users::read_password("Password: ").unwrap_or_default();
                user.is_some() && users::check_password(&name, &pw)
            }
        };
        if !ok {
            let _ = rt::time::sleep_ms(1000);
            println!("Login incorrect");
            failures += 1;
            if failures >= 5 {
                return 1;
            }
            continue;
        }
        let u = user.unwrap();
        if let Ok(motd) = rt::fs::read_to_string("/etc/motd") {
            print!("{motd}");
        }
        let _ = rt::fs::chown("/dev/console", u.uid, u.gid);
        if let Err(e) = users::become_user(&u) {
            eprintln!("login: cannot change to {}: {e}", u.name);
            return 1;
        }
        let home = if rt::fs::is_dir(&u.home) { u.home.clone() } else { "/".into() };
        let _ = rt::env::set_current_dir(&home);
        rt::env::set_var("HOME", &home);
        rt::env::set_var("PATH", if u.uid == 0 { "/bin:/sbin:/usr/bin" } else { "/bin:/usr/bin" });
        let shell = if u.shell.is_empty() { String::from("/bin/sh") } else { u.shell.clone() };
        let arg0 = format!("-{}", rt::path::basename(&shell));
        let e = rt::process::execve(&shell, &[arg0], &rt::env::environ());
        eprintln!("login: {shell}: {e}");
        return 1;
    }
}
