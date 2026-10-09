//! Small helpers.

use core::cell::UnsafeCell;

/// A global for a single-CPU kernel. Code that can race with interrupt handlers
/// must mask interrupts around its accesses.
pub struct Global<T>(UnsafeCell<T>);

unsafe impl<T> Sync for Global<T> {}

impl<T> Global<T> {
    pub const fn new(v: T) -> Self {
        Global(UnsafeCell::new(v))
    }

    #[allow(clippy::mut_from_ref)]
    pub fn get(&self) -> &mut T {
        unsafe { &mut *self.0.get() }
    }
}

/// The NUL-terminated prefix of `b` as a str (lossy).
pub fn cstr(b: &[u8]) -> &str {
    let end = b.iter().position(|&c| c == 0).unwrap_or(b.len());
    core::str::from_utf8(&b[..end]).unwrap_or("?")
}

pub const fn align_up(v: u32, a: u32) -> u32 {
    (v + a - 1) & !(a - 1)
}

pub const fn align_down(v: u32, a: u32) -> u32 {
    v & !(a - 1)
}

/// Copy `s` into a fixed buffer, NUL-terminated and truncated if needed.
pub fn set_cstr(dst: &mut [u8], s: &[u8]) {
    let n = s.len().min(dst.len().saturating_sub(1));
    dst[..n].copy_from_slice(&s[..n]);
    dst[n..].fill(0);
}
