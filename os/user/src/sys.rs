//! Raw system calls (ABI in lib/azsys).

pub use azsys::errno;
pub use azsys::nr;
use core::fmt;

unsafe extern "C" {
    fn rt_syscall(nr: u32, a1: u32, a2: u32, a3: u32, a4: u32, a5: u32) -> i32;
}

/// A failed system call.
#[derive(Clone, Copy, PartialEq, Eq)]
pub struct Errno(pub i32);

impl Errno {
    pub fn message(self) -> &'static str {
        errno::message(self.0)
    }
}

impl fmt::Display for Errno {
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        f.write_str(self.message())
    }
}

impl fmt::Debug for Errno {
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        write!(f, "Errno({}: {})", self.0, self.message())
    }
}

pub type Result<T> = core::result::Result<T, Errno>;

#[inline]
pub fn raw(nr: u32, a1: u32, a2: u32, a3: u32, a4: u32, a5: u32) -> i32 {
    unsafe { rt_syscall(nr, a1, a2, a3, a4, a5) }
}

/// A system call returning a non-negative value or a negated errno.
#[inline]
pub fn call(nr: u32, a: [u32; 5]) -> Result<u32> {
    let r = raw(nr, a[0], a[1], a[2], a[3], a[4]);
    if (-4095..0).contains(&r) { Err(Errno(-r)) } else { Ok(r as u32) }
}

pub fn call0(nr: u32) -> Result<u32> {
    call(nr, [0; 5])
}
pub fn call1(nr: u32, a: u32) -> Result<u32> {
    call(nr, [a, 0, 0, 0, 0])
}
pub fn call2(nr: u32, a: u32, b: u32) -> Result<u32> {
    call(nr, [a, b, 0, 0, 0])
}
pub fn call3(nr: u32, a: u32, b: u32, c: u32) -> Result<u32> {
    call(nr, [a, b, c, 0, 0])
}

/// A NUL-terminated copy of `s` for passing to the kernel.
pub fn cstr(s: &str) -> alloc::vec::Vec<u8> {
    let mut v = alloc::vec::Vec::with_capacity(s.len() + 1);
    v.extend_from_slice(s.as_bytes());
    v.push(0);
    v
}

/// Call with a path argument.
pub fn path_call(nr: u32, path: &str, b: u32, c: u32) -> Result<u32> {
    let p = cstr(path);
    call3(nr, p.as_ptr() as u32, b, c)
}
