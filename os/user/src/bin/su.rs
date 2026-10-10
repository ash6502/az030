//! su: run a shell as another user (default root).
//!
//!     su [-] [-l] [-c command] [user]

#![no_std]
#![no_main]

use rt::prelude::*;
use rt::users;

rt::main!(main);

fn main(args: &[String]) -> i32 {
    let mut login = false;
    let mut command: Option<String> = None;
    let mut user = String::from("root");
    let mut i = 1;
    while i < args.len() {
        match args[i].as_str() {
            "-" | "-l" | "--login" => login = true,
            "-c" => {
                i += 1;
                command = args.get(i).cloned();
            }
            u => user = u.into(),
        }
        i += 1;
    }
    let Some(u) = users::by_name(&user) else { rt::die!("user {user} does not exist") };
    if rt::process::uid() != 0 && users::has_password(&user) {
        let pw = users::read_password("Password: ").unwrap_or_default();
        if !users::check_password(&user, &pw) {
            let _ = rt::time::sleep_ms(1000);
            rt::die!("Authentication failure");
        }
    }
    if rt::process::euid() != 0 {
        rt::die!("must be installed setuid root");
    }
    if let Err(e) = users::become_user(&u) {
        rt::die!("cannot change to {}: {e}", u.name);
    }
    let shell = if u.shell.is_empty() { String::from("/bin/sh") } else { u.shell.clone() };
    if login {
        let _ = rt::env::set_current_dir(&u.home);
        rt::env::set_var("PATH", if u.uid == 0 { "/bin:/sbin:/usr/bin" } else { "/bin:/usr/bin" });
    }
    let arg0 = if login { format!("-{}", rt::path::basename(&shell)) } else { rt::path::basename(&shell).into() };
    let mut argv = vec![arg0];
    if let Some(c) = command {
        argv.push("-c".into());
        argv.push(c);
    }
    let e = rt::process::execve(&shell, &argv, &rt::env::environ());
    rt::die!("{shell}: {e}")
}
