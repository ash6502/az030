//! Standard I/O: `Read`/`Write`/`BufRead` traits, buffered stdout (line-buffered on
//! a terminal), unbuffered stderr, and a buffered stdin.

use crate::sys::{self, nr, Errno, Result};
use alloc::string::String;
use alloc::vec::Vec;
use core::cell::UnsafeCell;
use core::fmt;

pub type Fd = u32;
pub const STDIN: Fd = 0;
pub const STDOUT: Fd = 1;
pub const STDERR: Fd = 2;

/// read(2): returns the number of bytes read (0 at end of file). Retries on EINTR.
pub fn read_fd(fd: Fd, buf: &mut [u8]) -> Result<usize> {
    loop {
        match sys::call3(nr::READ, fd, buf.as_mut_ptr() as u32, buf.len() as u32) {
            Err(Errno(e)) if e == sys::errno::EINTR => continue,
            r => return r.map(|n| n as usize),
        }
    }
}

/// write(2) of the whole buffer.
pub fn write_fd(fd: Fd, mut buf: &[u8]) -> Result<()> {
    while !buf.is_empty() {
        match sys::call3(nr::WRITE, fd, buf.as_ptr() as u32, buf.len() as u32) {
            Ok(0) => return Err(Errno(sys::errno::EIO)),
            Ok(n) => buf = &buf[n as usize..],
            Err(Errno(e)) if e == sys::errno::EINTR => continue,
            Err(e) => return Err(e),
        }
    }
    Ok(())
}

pub fn close(fd: Fd) -> Result<()> {
    sys::call1(nr::CLOSE, fd).map(|_| ())
}

pub fn isatty(fd: Fd) -> bool {
    crate::term::get_attr(fd).is_ok()
}

pub trait Write {
    fn write(&mut self, buf: &[u8]) -> Result<usize>;
    fn flush(&mut self) -> Result<()> {
        Ok(())
    }
    fn write_all(&mut self, mut buf: &[u8]) -> Result<()> {
        while !buf.is_empty() {
            let n = self.write(buf)?;
            if n == 0 {
                return Err(Errno(sys::errno::EIO));
            }
            buf = &buf[n..];
        }
        Ok(())
    }
    fn write_str(&mut self, s: &str) -> Result<()> {
        self.write_all(s.as_bytes())
    }
    fn write_fmt(&mut self, args: fmt::Arguments) -> Result<()> {
        struct Adapter<'a, W: Write + ?Sized> {
            w: &'a mut W,
            err: Option<Errno>,
        }
        impl<W: Write + ?Sized> fmt::Write for Adapter<'_, W> {
            fn write_str(&mut self, s: &str) -> fmt::Result {
                self.w.write_all(s.as_bytes()).map_err(|e| {
                    self.err = Some(e);
                    fmt::Error
                })
            }
        }
        let mut a = Adapter { w: self, err: None };
        match fmt::write(&mut a, args) {
            Ok(()) => Ok(()),
            Err(_) => Err(a.err.unwrap_or(Errno(sys::errno::EIO))),
        }
    }
}

pub trait Read {
    fn read(&mut self, buf: &mut [u8]) -> Result<usize>;
    fn read_to_end(&mut self, out: &mut Vec<u8>) -> Result<usize> {
        let start = out.len();
        let mut chunk = [0u8; 4096];
        loop {
            let n = self.read(&mut chunk)?;
            if n == 0 {
                return Ok(out.len() - start);
            }
            out.extend_from_slice(&chunk[..n]);
        }
    }
    fn read_to_string(&mut self, out: &mut String) -> Result<usize> {
        let mut v = Vec::new();
        let n = self.read_to_end(&mut v)?;
        out.push_str(&String::from_utf8_lossy(&v));
        Ok(n)
    }
    fn read_exact(&mut self, mut buf: &mut [u8]) -> Result<()> {
        while !buf.is_empty() {
            let n = self.read(buf)?;
            if n == 0 {
                return Err(Errno(sys::errno::EIO));
            }
            buf = &mut buf[n..];
        }
        Ok(())
    }
}

/// Line-oriented reading.
pub trait BufRead {
    /// Append the next line (including its `\n`) to `out`; 0 at end of input.
    fn read_line(&mut self, out: &mut String) -> Result<usize>;
    /// Raw bytes up to and including `delim`.
    fn read_until(&mut self, delim: u8, out: &mut Vec<u8>) -> Result<usize>;
    fn lines(self) -> Lines<Self>
    where
        Self: Sized,
    {
        Lines(self)
    }
}

pub struct Lines<B>(B);

impl<B: BufRead> Iterator for Lines<B> {
    type Item = Result<String>;
    fn next(&mut self) -> Option<Result<String>> {
        let mut s = String::new();
        match self.0.read_line(&mut s) {
            Ok(0) => None,
            Ok(_) => {
                if s.ends_with('\n') {
                    s.pop();
                }
                Some(Ok(s))
            }
            Err(e) => Some(Err(e)),
        }
    }
}

/// A buffered reader over any `Read`.
pub struct BufReader<R> {
    inner: R,
    buf: Vec<u8>,
    pos: usize,
    len: usize,
}

impl<R: Read> BufReader<R> {
    pub fn new(inner: R) -> Self {
        BufReader { inner, buf: alloc::vec![0; 4096], pos: 0, len: 0 }
    }

    fn fill(&mut self) -> Result<&[u8]> {
        if self.pos >= self.len {
            self.len = self.inner.read(&mut self.buf)?;
            self.pos = 0;
        }
        Ok(&self.buf[self.pos..self.len])
    }

    pub fn into_inner(self) -> R {
        self.inner
    }
}

impl<R: Read> Read for BufReader<R> {
    fn read(&mut self, out: &mut [u8]) -> Result<usize> {
        let avail = self.fill()?;
        let n = avail.len().min(out.len());
        out[..n].copy_from_slice(&avail[..n]);
        self.pos += n;
        Ok(n)
    }
}

impl<R: Read> BufRead for BufReader<R> {
    fn read_until(&mut self, delim: u8, out: &mut Vec<u8>) -> Result<usize> {
        let mut total = 0;
        loop {
            let avail = self.fill()?;
            if avail.is_empty() {
                return Ok(total);
            }
            match avail.iter().position(|&b| b == delim) {
                Some(i) => {
                    out.extend_from_slice(&avail[..=i]);
                    self.pos += i + 1;
                    return Ok(total + i + 1);
                }
                None => {
                    let n = avail.len();
                    out.extend_from_slice(avail);
                    self.pos += n;
                    total += n;
                }
            }
        }
    }

    fn read_line(&mut self, out: &mut String) -> Result<usize> {
        let mut v = Vec::new();
        let n = self.read_until(b'\n', &mut v)?;
        out.push_str(&String::from_utf8_lossy(&v));
        Ok(n)
    }
}

/// Unbuffered access to a file descriptor.
pub struct FdIo(pub Fd);

impl Read for FdIo {
    fn read(&mut self, buf: &mut [u8]) -> Result<usize> {
        read_fd(self.0, buf)
    }
}

impl Write for FdIo {
    fn write(&mut self, buf: &[u8]) -> Result<usize> {
        write_fd(self.0, buf).map(|_| buf.len())
    }
}

// ---- the standard streams -----------------------------------------------------------

struct OutState {
    buf: Vec<u8>,
    /// 0 unknown, 1 terminal (line buffered), 2 not a terminal (block buffered)
    mode: u8,
}

struct Global<T>(UnsafeCell<T>);
unsafe impl<T> Sync for Global<T> {}

static OUT: Global<OutState> = Global(UnsafeCell::new(OutState { buf: Vec::new(), mode: 0 }));
static IN: Global<Option<BufReader<FdIo>>> = Global(UnsafeCell::new(None));

/// Buffered standard output.
pub struct Stdout;

pub fn stdout() -> Stdout {
    Stdout
}

impl Stdout {
    fn state(&self) -> &'static mut OutState {
        unsafe { &mut *OUT.0.get() }
    }

    /// Flush, ignoring errors (used before writing to stderr and at exit).
    pub fn flush_quiet(&mut self) {
        let _ = self.flush();
    }
}

impl Write for Stdout {
    fn write(&mut self, data: &[u8]) -> Result<usize> {
        let st = self.state();
        if st.mode == 0 {
            st.mode = if isatty(STDOUT) { 1 } else { 2 };
        }
        st.buf.extend_from_slice(data);
        let flush = st.buf.len() >= 4096 || (st.mode == 1 && data.contains(&b'\n'));
        if flush {
            self.flush()?;
        }
        Ok(data.len())
    }

    fn flush(&mut self) -> Result<()> {
        let st = self.state();
        if st.buf.is_empty() {
            return Ok(());
        }
        let r = write_fd(STDOUT, &st.buf);
        st.buf.clear();
        r
    }
}

/// Unbuffered standard error.
pub struct Stderr;

pub fn stderr() -> Stderr {
    Stderr
}

impl Write for Stderr {
    fn write(&mut self, data: &[u8]) -> Result<usize> {
        write_fd(STDERR, data).map(|_| data.len())
    }
}

/// Buffered standard input.
pub struct Stdin;

pub fn stdin() -> Stdin {
    Stdin
}

impl Stdin {
    fn reader(&self) -> &'static mut BufReader<FdIo> {
        let r = unsafe { &mut *IN.0.get() };
        r.get_or_insert_with(|| BufReader::new(FdIo(STDIN)))
    }

    /// Read one line (without the newline); None at end of input.
    pub fn line(&self) -> Option<String> {
        stdout().flush_quiet();
        let mut s = String::new();
        match (&mut &*self).read_line(&mut s) {
            Ok(0) | Err(_) => None,
            Ok(_) => {
                if s.ends_with('\n') {
                    s.pop();
                }
                Some(s)
            }
        }
    }
}

impl Read for Stdin {
    fn read(&mut self, buf: &mut [u8]) -> Result<usize> {
        stdout().flush_quiet();
        self.reader().read(buf)
    }
}

impl BufRead for Stdin {
    fn read_until(&mut self, delim: u8, out: &mut Vec<u8>) -> Result<usize> {
        stdout().flush_quiet();
        self.reader().read_until(delim, out)
    }
    fn read_line(&mut self, out: &mut String) -> Result<usize> {
        stdout().flush_quiet();
        self.reader().read_line(out)
    }
}

impl BufRead for &Stdin {
    fn read_until(&mut self, delim: u8, out: &mut Vec<u8>) -> Result<usize> {
        (**self).reader().read_until(delim, out)
    }
    fn read_line(&mut self, out: &mut String) -> Result<usize> {
        stdout().flush_quiet();
        (**self).reader().read_line(out)
    }
}

impl<R: Read + ?Sized> Read for alloc::boxed::Box<R> {
    fn read(&mut self, buf: &mut [u8]) -> Result<usize> {
        (**self).read(buf)
    }
}

impl<W: Write + ?Sized> Write for alloc::boxed::Box<W> {
    fn write(&mut self, buf: &[u8]) -> Result<usize> {
        (**self).write(buf)
    }
    fn flush(&mut self) -> Result<()> {
        (**self).flush()
    }
}

/// Open an input file; `-` is standard input.
pub fn open_input(path: &str) -> Result<alloc::boxed::Box<dyn Read>> {
    if path == "-" {
        Ok(alloc::boxed::Box::new(FdIo(STDIN)))
    } else {
        let f = crate::fs::File::open(path)?;
        if f.metadata().is_ok_and(|m| m.is_dir()) {
            return Err(Errno(sys::errno::EISDIR));
        }
        Ok(alloc::boxed::Box::new(f))
    }
}

/// A buffered reader over an input file (`-` = standard input).
pub fn reader(path: &str) -> Result<BufReader<alloc::boxed::Box<dyn Read>>> {
    open_input(path).map(BufReader::new)
}

/// The input files named on a command line, or `-` if there are none.
pub fn inputs(args: &[String]) -> Vec<String> {
    if args.is_empty() { alloc::vec![String::from("-")] } else { args.to_vec() }
}
