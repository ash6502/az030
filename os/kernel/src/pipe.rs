//! Pipes.

use crate::file::{new_file, FileRef, Kind};
use crate::proc;
use crate::vm::KResult;
use crate::{arch, signal};
use alloc::collections::VecDeque;
use alloc::rc::Rc;
use alloc::vec::Vec;
use azsys::errno::*;
use core::cell::RefCell;

const CAPACITY: usize = 4096;

pub struct Pipe {
    buf: VecDeque<u8>,
    readers: u32,
    writers: u32,
}

fn chan(p: &Rc<RefCell<Pipe>>) -> usize {
    Rc::as_ptr(p) as usize
}

pub fn new() -> (FileRef, FileRef) {
    let p = Rc::new(RefCell::new(Pipe { buf: VecDeque::with_capacity(CAPACITY), readers: 1, writers: 1 }));
    let r = new_file(Kind::PipeRead(p.clone()), azsys::flags::O_RDONLY, None);
    let w = new_file(Kind::PipeWrite(p), azsys::flags::O_WRONLY, None);
    (r, w)
}

pub fn close_end(p: &Rc<RefCell<Pipe>>, write_end: bool) {
    let mut q = p.borrow_mut();
    if write_end {
        q.writers -= 1;
    } else {
        q.readers -= 1;
    }
    drop(q);
    proc::wakeup(chan(p));
}

pub fn available(p: &Rc<RefCell<Pipe>>) -> usize {
    p.borrow().buf.len()
}

pub fn read(p: &Rc<RefCell<Pipe>>, len: usize, nonblock: bool) -> KResult<Vec<u8>> {
    let sr = arch::irq_save();
    loop {
        {
            let mut q = p.borrow_mut();
            if !q.buf.is_empty() {
                let n = len.min(q.buf.len());
                let out: Vec<u8> = q.buf.drain(..n).collect();
                drop(q);
                proc::wakeup(chan(p));
                arch::irq_restore_sr(sr);
                return Ok(out);
            }
            if q.writers == 0 || len == 0 {
                arch::irq_restore_sr(sr);
                return Ok(Vec::new());
            }
        }
        if nonblock {
            arch::irq_restore_sr(sr);
            return Err(EAGAIN);
        }
        if !proc::sleep(chan(p), true) {
            arch::irq_restore_sr(sr);
            return Err(EINTR);
        }
    }
}

pub fn write(p: &Rc<RefCell<Pipe>>, data: &[u8], nonblock: bool) -> KResult<usize> {
    let sr = arch::irq_save();
    let mut done = 0;
    loop {
        {
            let mut q = p.borrow_mut();
            if q.readers == 0 {
                drop(q);
                arch::irq_restore_sr(sr);
                signal::send(proc::current(), azsys::sig::SIGPIPE);
                return if done > 0 { Ok(done) } else { Err(EPIPE) };
            }
            let room = CAPACITY - q.buf.len();
            if room > 0 {
                let n = room.min(data.len() - done);
                q.buf.extend(&data[done..done + n]);
                done += n;
                drop(q);
                proc::wakeup(chan(p));
                if done == data.len() {
                    arch::irq_restore_sr(sr);
                    return Ok(done);
                }
                continue;
            }
        }
        if nonblock {
            arch::irq_restore_sr(sr);
            return if done > 0 { Ok(done) } else { Err(EAGAIN) };
        }
        if !proc::sleep(chan(p), true) {
            arch::irq_restore_sr(sr);
            return if done > 0 { Ok(done) } else { Err(EINTR) };
        }
    }
}
