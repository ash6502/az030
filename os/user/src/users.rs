//! User and group databases (/etc/passwd, /etc/group, /etc/shadow) and password
//! hashing.
//!
//! /etc/shadow lines are `name:hash`, where hash is empty (no password), `!` or `*`
//! (locked) or `$s256$SALT$HEX` with HEX = SHA-256 applied 1000 times to
//! SALT + password (each round hashing the previous digest + SALT + password).

use crate::sha256::{self, Sha256};
use alloc::string::String;
use alloc::vec::Vec;

#[derive(Clone, Debug)]
pub struct User {
    pub name: String,
    pub uid: u32,
    pub gid: u32,
    pub gecos: String,
    pub home: String,
    pub shell: String,
}

#[derive(Clone, Debug)]
pub struct Group {
    pub name: String,
    pub gid: u32,
    pub members: Vec<String>,
}

pub fn users() -> Vec<User> {
    let text = crate::fs::read_to_string("/etc/passwd").unwrap_or_default();
    text.lines()
        .filter_map(|l| {
            let f: Vec<&str> = l.split(':').collect();
            if f.len() < 7 {
                return None;
            }
            Some(User {
                name: f[0].into(),
                uid: f[2].parse().ok()?,
                gid: f[3].parse().ok()?,
                gecos: f[4].into(),
                home: f[5].into(),
                shell: f[6].into(),
            })
        })
        .collect()
}

pub fn groups() -> Vec<Group> {
    let text = crate::fs::read_to_string("/etc/group").unwrap_or_default();
    text.lines()
        .filter_map(|l| {
            let f: Vec<&str> = l.split(':').collect();
            if f.len() < 4 {
                return None;
            }
            Some(Group {
                name: f[0].into(),
                gid: f[2].parse().ok()?,
                members: f[3].split(',').filter(|s| !s.is_empty()).map(String::from).collect(),
            })
        })
        .collect()
}

pub fn by_name(name: &str) -> Option<User> {
    users().into_iter().find(|u| u.name == name)
}

pub fn by_uid(uid: u32) -> Option<User> {
    users().into_iter().find(|u| u.uid == uid)
}

pub fn user_name(uid: u32) -> String {
    by_uid(uid).map(|u| u.name).unwrap_or_else(|| alloc::format!("{uid}"))
}

pub fn group_name(gid: u32) -> String {
    groups().into_iter().find(|g| g.gid == gid).map(|g| g.name).unwrap_or_else(|| alloc::format!("{gid}"))
}

pub fn group_by_name(name: &str) -> Option<Group> {
    groups().into_iter().find(|g| g.name == name)
}

fn hash_with_salt(salt: &str, password: &str) -> String {
    let mut d = [0u8; 32];
    for i in 0..1000 {
        let mut h = Sha256::new();
        if i > 0 {
            h.update(&d);
        }
        h.update(salt.as_bytes());
        h.update(password.as_bytes());
        d = h.finish();
    }
    alloc::format!("$s256${salt}${}", sha256::hex(&d))
}

/// Hash a new password with a fresh salt.
pub fn hash_password(password: &str) -> String {
    let (s, us) = crate::time::now_precise();
    let seed = sha256::digest(&[s.to_be_bytes(), us.to_be_bytes(), crate::process::id().to_be_bytes()].concat());
    const CH: &[u8] = b"./0123456789ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz";
    let salt: String = seed[..8].iter().map(|b| CH[(*b & 63) as usize] as char).collect();
    hash_with_salt(&salt, password)
}

/// The stored hash for a user (None if the user has no shadow entry).
pub fn shadow_hash(name: &str) -> Option<String> {
    let text = crate::fs::read_to_string("/etc/shadow").ok()?;
    text.lines().find_map(|l| {
        let (n, h) = l.split_once(':')?;
        if n == name { Some(h.split(':').next().unwrap_or("").into()) } else { None }
    })
}

pub fn check_password(name: &str, password: &str) -> bool {
    let h = shadow_hash(name).unwrap_or_default();
    if h.is_empty() {
        return true;
    }
    let mut parts = h.split('$');
    match (parts.next(), parts.next(), parts.next(), parts.next()) {
        (Some(""), Some("s256"), Some(salt), Some(_)) => hash_with_salt(salt, password) == h,
        _ => false,
    }
}

/// Does the account have a password at all?
pub fn has_password(name: &str) -> bool {
    shadow_hash(name).is_some_and(|h| !h.is_empty())
}

/// Replace a user's entry in /etc/shadow.
pub fn set_shadow(name: &str, hash: &str) -> crate::Result<()> {
    let text = crate::fs::read_to_string("/etc/shadow").unwrap_or_default();
    let mut out = String::new();
    let mut found = false;
    for l in text.lines() {
        if l.split(':').next() == Some(name) {
            out.push_str(&alloc::format!("{name}:{hash}\n"));
            found = true;
        } else {
            out.push_str(l);
            out.push('\n');
        }
    }
    if !found {
        out.push_str(&alloc::format!("{name}:{hash}\n"));
    }
    let tmp = "/etc/shadow.new";
    {
        let mut f = crate::fs::File::open_with(tmp, azsys::flags::O_WRONLY | azsys::flags::O_CREAT | azsys::flags::O_TRUNC, 0o600)?;
        crate::io::Write::write_all(&mut f, out.as_bytes())?;
    }
    crate::fs::rename(tmp, "/etc/shadow")
}

/// Become `u`: set the group, then the user id, and the usual environment.
pub fn become_user(u: &User) -> crate::Result<()> {
    crate::process::set_gid(u.gid)?;
    crate::process::set_uid(u.uid)?;
    crate::env::set_var("HOME", &u.home);
    crate::env::set_var("USER", &u.name);
    crate::env::set_var("LOGNAME", &u.name);
    crate::env::set_var("SHELL", &u.shell);
    Ok(())
}

/// Read a password from the terminal without echo.
pub fn read_password(prompt: &str) -> Option<String> {
    use crate::term;
    crate::print!("{prompt}");
    crate::io::stdout().flush_quiet();
    let fd = crate::io::STDIN;
    let saved = term::get_attr(fd).ok();
    if let Some(mut t) = saved {
        t.lflag &= !term::ECHO;
        t.lflag |= term::ECHONL;
        let _ = term::set_attr(fd, &t);
    }
    let line = crate::io::stdin().line();
    if let Some(t) = saved {
        let _ = term::set_attr(fd, &t);
    }
    line
}
