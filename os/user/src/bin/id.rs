//! id: print user and group ids.
//!
//!     id [-u | -g | -G] [-n] [user]
//!     groups [user]

#![no_std]
#![no_main]

use rt::prelude::*;
use rt::users;

rt::main!(main);

fn main(args: &[String]) -> i32 {
    let groups_cmd = rt::env::progname() == "groups";
    let (o, rest) = rt::getopt::parse(&args[1..], "ugGnr", "[-u | -g | -G] [-n] [user]");
    let (uid, gid, euid, egid, name) = match rest.first() {
        Some(n) => match users::by_name(n) {
            Some(u) => (u.uid, u.gid, u.uid, u.gid, u.name),
            None => rt::die!("'{n}': no such user"),
        },
        None => {
            let uid = rt::process::uid();
            (uid, rt::process::gid(), rt::process::euid(), rt::process::egid(), users::user_name(uid))
        }
    };
    let mut gids = vec![egid];
    for g in users::groups() {
        if g.members.contains(&name) && !gids.contains(&g.gid) {
            gids.push(g.gid);
        }
    }
    let n = o.has('n');
    if groups_cmd || o.has('G') {
        let v: Vec<String> = gids.iter().map(|g| if n || groups_cmd { users::group_name(*g) } else { format!("{g}") }).collect();
        println!("{}", v.join(" "));
        return 0;
    }
    if o.has('u') {
        println!("{}", if n { users::user_name(euid) } else { format!("{euid}") });
        return 0;
    }
    if o.has('g') {
        println!("{}", if n { users::group_name(egid) } else { format!("{egid}") });
        return 0;
    }
    let mut s = format!("uid={uid}({}) gid={gid}({})", users::user_name(uid), users::group_name(gid));
    if euid != uid {
        s.push_str(&format!(" euid={euid}({})", users::user_name(euid)));
    }
    if egid != gid {
        s.push_str(&format!(" egid={egid}({})", users::group_name(egid)));
    }
    let gl: Vec<String> = gids.iter().map(|g| format!("{g}({})", users::group_name(*g))).collect();
    s.push_str(&format!(" groups={}", gl.join(",")));
    println!("{s}");
    0
}
