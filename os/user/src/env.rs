//! Program arguments, environment variables and the working directory.

use crate::sys::{self, nr, Result};
use alloc::string::String;
use alloc::vec::Vec;
use core::cell::UnsafeCell;

struct State {
    progname: String,
    vars: Vec<(String, String)>,
}

struct Global(UnsafeCell<State>);
unsafe impl Sync for Global {}

static STATE: Global = Global(UnsafeCell::new(State { progname: String::new(), vars: Vec::new() }));

fn state() -> &'static mut State {
    unsafe { &mut *STATE.0.get() }
}

unsafe fn c_string(p: *const u8) -> String {
    let mut n = 0;
    unsafe {
        while *p.add(n) != 0 {
            n += 1;
        }
        String::from_utf8_lossy(core::slice::from_raw_parts(p, n)).into_owned()
    }
}

/// Collect argv and envp (called once, by `rt_entry`).
pub unsafe fn init(argc: u32, argv: *const *const u8, envp: *const *const u8) -> Vec<String> {
    let mut args = Vec::with_capacity(argc as usize);
    unsafe {
        for i in 0..argc as usize {
            args.push(c_string(*argv.add(i)));
        }
        let st = state();
        let mut i = 0;
        while !(*envp.add(i)).is_null() {
            let s = c_string(*envp.add(i));
            if let Some((k, v)) = s.split_once('=') {
                st.vars.push((k.into(), v.into()));
            }
            i += 1;
        }
        st.progname = match args.first() {
            Some(a) => crate::path::basename(a).into(),
            None => "?".into(),
        };
    }
    args
}

/// The program's name (the last component of argv[0]).
pub fn progname() -> &'static str {
    let s = &state().progname;
    if s.is_empty() { "?" } else { s }
}

pub fn var(key: &str) -> Option<String> {
    state().vars.iter().find(|(k, _)| k == key).map(|(_, v)| v.clone())
}

pub fn set_var(key: &str, value: &str) {
    let vars = &mut state().vars;
    match vars.iter_mut().find(|(k, _)| k == key) {
        Some(e) => e.1 = value.into(),
        None => vars.push((key.into(), value.into())),
    }
}

pub fn remove_var(key: &str) {
    state().vars.retain(|(k, _)| k != key);
}

pub fn vars() -> Vec<(String, String)> {
    state().vars.clone()
}

/// The environment as `KEY=value` strings, for `execve`.
pub fn environ() -> Vec<String> {
    state().vars.iter().map(|(k, v)| alloc::format!("{k}={v}")).collect()
}

pub fn current_dir() -> Result<String> {
    let mut buf = alloc::vec![0u8; 1024];
    let n = sys::call2(nr::GETCWD, buf.as_mut_ptr() as u32, buf.len() as u32)? as usize;
    buf.truncate(n);
    Ok(String::from_utf8_lossy(&buf).into_owned())
}

pub fn set_current_dir(path: &str) -> Result<()> {
    sys::path_call(nr::CHDIR, path, 0, 0).map(|_| ())
}

pub fn home() -> String {
    var("HOME").unwrap_or_else(|| "/".into())
}
