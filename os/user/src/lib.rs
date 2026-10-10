//! `rt`: the runtime library for Rust programs on the az030 OS.
//!
//! A small `std` substitute: system calls, a heap, buffered standard I/O with
//! `print!`-style macros, files and directories, arguments and environment,
//! processes, time, terminal control and signals.
//!
//! A program looks like this:
//!
//! ```ignore
//! #![no_std]
//! #![no_main]
//! rt::main!(main);
//! fn main(args: &[String]) -> i32 {
//!     rt::println!("hello from {}", args[0]);
//!     0
//! }
//! ```

#![no_std]

extern crate alloc;
extern crate azrt;

pub mod env;
pub mod fs;
pub mod getopt;
pub mod glob;
pub mod heap;
pub mod io;
pub mod path;
pub mod process;
pub mod readline;
pub mod regex;
pub mod sha256;
pub mod signal;
pub mod sys;
pub mod term;
pub mod test;
pub mod time;
pub mod users;
pub mod util;

pub use alloc::borrow::ToOwned;
pub use alloc::boxed::Box;
pub use alloc::format;
pub use alloc::string::{String, ToString};
pub use alloc::vec;
pub use alloc::vec::Vec;
pub use sys::{Errno, Result};

/// Everything a typical program wants in scope.
pub mod prelude {
    pub use crate::io::{BufRead, Read, Write};
    pub use crate::{eprint, eprintln, print, println};
    pub use crate::Errno;
    pub use alloc::rc::Rc;
    pub use alloc::borrow::ToOwned;
    pub use alloc::boxed::Box;
    pub use alloc::collections::{BTreeMap, BTreeSet, VecDeque};
    pub use alloc::format;
    pub use alloc::string::{String, ToString};
    pub use alloc::vec;
    pub use alloc::vec::Vec;
}

/// Declare the program's entry point: `fn main(args: &[String]) -> i32`.
#[macro_export]
macro_rules! main {
    ($f:path) => {
        #[unsafe(no_mangle)]
        pub extern "Rust" fn rt_main(args: &[$crate::String]) -> i32 {
            $f(args)
        }
    };
}

unsafe extern "Rust" {
    fn rt_main(args: &[String]) -> i32;
}

/// Called by `_start` in asm/crt0.s.
#[unsafe(no_mangle)]
pub extern "C" fn rt_entry(argc: u32, argv: *const *const u8, envp: *const *const u8) -> ! {
    let args = unsafe { env::init(argc, argv, envp) };
    let code = unsafe { rt_main(&args) };
    process::exit(code)
}

#[macro_export]
macro_rules! print {
    ($($t:tt)*) => {{ let _ = $crate::io::Write::write_fmt(&mut $crate::io::stdout(), format_args!($($t)*)); }};
}

#[macro_export]
macro_rules! println {
    () => { $crate::print!("\n") };
    ($($t:tt)*) => {{ $crate::print!("{}\n", format_args!($($t)*)); }};
}

#[macro_export]
macro_rules! eprint {
    ($($t:tt)*) => {{
        $crate::io::stdout().flush_quiet();
        let _ = $crate::io::Write::write_fmt(&mut $crate::io::stderr(), format_args!($($t)*));
    }};
}

#[macro_export]
macro_rules! eprintln {
    () => { $crate::eprint!("\n") };
    ($($t:tt)*) => {{ $crate::eprint!("{}\n", format_args!($($t)*)); }};
}

/// Print `prog: message` to stderr (the program name from argv[0]).
#[macro_export]
macro_rules! warn {
    ($($t:tt)*) => {{ $crate::eprintln!("{}: {}", $crate::env::progname(), format_args!($($t)*)); }};
}

/// Print `prog: message` and exit with status 1.
#[macro_export]
macro_rules! die {
    ($($t:tt)*) => {{ $crate::warn!($($t)*); $crate::process::exit(1) }};
}

#[panic_handler]
fn panic(info: &core::panic::PanicInfo) -> ! {
    io::stdout().flush_quiet();
    let _ = io::Write::write_fmt(&mut io::stderr(), format_args!("{}: {}\n", env::progname(), info));
    process::exit(101)
}

/// LLVM lowers traps to calls to abort().
#[unsafe(no_mangle)]
pub extern "C" fn abort() -> ! {
    io::stdout().flush_quiet();
    let _ = signal::raise(azsys::sig::SIGABRT);
    process::exit(134)
}
