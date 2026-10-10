//! chown / chgrp: change file owner and group.
//!
//!     chown [-R] owner[:group] file...
//!     chgrp [-R] group file...

#![no_std]
#![no_main]

use rt::fs;
use rt::prelude::*;

rt::main!(main);

fn uid_of(s: &str) -> Option<u32> {
    s.parse().ok().or_else(|| rt::users::by_name(s).map(|u| u.uid))
}

fn gid_of(s: &str) -> Option<u32> {
    s.parse().ok().or_else(|| rt::users::group_by_name(s).map(|g| g.gid))
}

fn apply(path: &str, uid: Option<u32>, gid: Option<u32>, recursive: bool) -> bool {
    let m = match fs::symlink_metadata(path) {
        Ok(m) => m,
        Err(e) => {
            rt::warn!("cannot access '{path}': {e}");
            return false;
        }
    };
    let mut ok = true;
    if let Err(e) = fs::chown(path, uid.unwrap_or(m.uid()), gid.unwrap_or(m.gid())) {
        rt::warn!("changing ownership of '{path}': {e}");
        ok = false;
    }
    if recursive && m.is_dir() {
        if let Ok(es) = fs::read_dir(path) {
            for e in es {
                if e.name != "." && e.name != ".." {
                    ok &= apply(&rt::path::join(path, &e.name), uid, gid, recursive);
                }
            }
        }
    }
    ok
}

fn main(args: &[String]) -> i32 {
    let chgrp = rt::env::progname() == "chgrp";
    let (o, mut rest) = rt::getopt::parse(&args[1..], "Rh", if chgrp { "[-R] group file..." } else { "[-R] owner[:group] file..." });
    if rest.len() < 2 {
        rt::die!("missing operand");
    }
    let spec = rest.remove(0);
    let (uid, gid) = if chgrp {
        match gid_of(&spec) {
            Some(g) => (None, Some(g)),
            None => rt::die!("invalid group: '{spec}'"),
        }
    } else {
        let (u, g) = match spec.split_once([':', '.']) {
            Some((u, g)) => (u, Some(g)),
            None => (spec.as_str(), None),
        };
        let uid = if u.is_empty() {
            None
        } else {
            match uid_of(u) {
                Some(x) => Some(x),
                None => rt::die!("invalid user: '{u}'"),
            }
        };
        let gid = match g {
            None | Some("") => None,
            Some(g) => match gid_of(g) {
                Some(x) => Some(x),
                None => rt::die!("invalid group: '{g}'"),
            },
        };
        (uid, gid)
    };
    let mut st = 0;
    for f in &rest {
        if !apply(f, uid, gid, o.has('R')) {
            st = 1;
        }
    }
    st
}
