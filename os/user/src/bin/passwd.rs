//! passwd: change a password.
//!
//!     passwd [-d] [user]     (-d: remove the password; root only)

#![no_std]
#![no_main]

use rt::prelude::*;
use rt::users;

rt::main!(main);

fn main(args: &[String]) -> i32 {
    let (o, rest) = rt::getopt::parse(&args[1..], "d", "[-d] [user]");
    let me = rt::process::uid();
    let user = match rest.first() {
        Some(u) => u.clone(),
        None => users::user_name(me),
    };
    let Some(u) = users::by_name(&user) else { rt::die!("user '{user}' does not exist") };
    if me != 0 && me != u.uid {
        rt::die!("you may not change the password for {user}");
    }
    if rt::process::euid() != 0 {
        rt::die!("must be installed setuid root");
    }
    if o.has('d') {
        if me != 0 {
            rt::die!("only root can remove passwords");
        }
        if let Err(e) = users::set_shadow(&user, "") {
            rt::die!("cannot update /etc/shadow: {e}");
        }
        println!("passwd: password for {user} removed");
        return 0;
    }
    println!("Changing password for {user}.");
    if me != 0 && users::has_password(&user) {
        let old = users::read_password("Current password: ").unwrap_or_default();
        if !users::check_password(&user, &old) {
            let _ = rt::time::sleep_ms(1000);
            rt::die!("Authentication token manipulation error");
        }
    }
    let new = users::read_password("New password: ").unwrap_or_default();
    let again = users::read_password("Retype new password: ").unwrap_or_default();
    if new != again {
        rt::die!("Sorry, passwords do not match.");
    }
    if new.is_empty() {
        rt::die!("password unchanged (empty)");
    }
    if let Err(e) = users::set_shadow(&user, &users::hash_password(&new)) {
        rt::die!("cannot update /etc/shadow: {e}");
    }
    println!("passwd: password updated successfully");
    0
}
