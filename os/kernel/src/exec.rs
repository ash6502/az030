//! exec: load an ELF executable (or a #! script) into a fresh address space.
//!
//! Initial user stack (top of memory at 0x80000000):
//!
//! ```text
//! sp ->  argc
//!        argv[0] .. argv[argc-1], NULL
//!        envp[0] .. NULL
//!        (strings)
//! ```

use crate::fs;
use crate::mem::PAGE;
use crate::proc::Proc;
use crate::trap::TrapFrame;
use crate::util::align_up;
use crate::vm::{self, AddressSpace, KResult, USER_BASE, USER_TOP};
use alloc::vec;
use alloc::vec::Vec;
use azfs::{S_IFMT, S_IFREG, S_ISGID, S_ISUID};
use azld::elf::{Ehdr, Phdr, EM_68K, ET_EXEC, PF_W, PT_LOAD};
use azsys::errno::*;

/// Limit on the total size of arguments and environment.
const ARG_MAX: usize = 128 * 1024;

pub fn exec(p: &mut Proc, tf: &mut TrapFrame, path: &str, argv: Vec<Vec<u8>>, envp: Vec<Vec<u8>>) -> KResult<()> {
    exec_depth(p, tf, path.as_bytes(), argv, envp, 0)
}

fn exec_depth(p: &mut Proc, tf: &mut TrapFrame, path: &[u8], mut argv: Vec<Vec<u8>>, envp: Vec<Vec<u8>>, depth: u32) -> KResult<()> {
    let ip = fs::lookup(path, true)?;
    let (mode, uid, gid) = {
        let i = ip.borrow();
        if i.kind() != S_IFREG {
            return Err(EACCES);
        }
        if !fs::permitted(&i, 1) {
            return Err(EACCES);
        }
        (i.di.mode, i.di.uid, i.di.gid)
    };
    let mut head = vec![0u8; 512];
    let n = fs::disk::read(&mut ip.borrow_mut(), 0, &mut head)?;
    head.truncate(n);

    // #!interpreter [arg]
    if head.starts_with(b"#!") {
        if depth > 2 {
            return Err(ELOOP);
        }
        let end = head.iter().position(|&c| c == b'\n').unwrap_or(head.len());
        let line = core::str::from_utf8(&head[2..end]).map_err(|_| ENOEXEC)?.trim();
        let mut parts = line.splitn(2, |c: char| c == ' ' || c == '\t');
        let interp = parts.next().filter(|s| !s.is_empty()).ok_or(ENOEXEC)?;
        let mut nargv: Vec<Vec<u8>> = vec![interp.as_bytes().to_vec()];
        if let Some(a) = parts.next().map(str::trim).filter(|s| !s.is_empty()) {
            nargv.push(a.as_bytes().to_vec());
        }
        nargv.push(path.to_vec());
        if !argv.is_empty() {
            argv.remove(0);
        }
        nargv.extend(argv);
        return exec_depth(p, tf, interp.as_bytes(), nargv, envp, depth + 1);
    }

    let eh = Ehdr::parse(&head).ok_or(ENOEXEC)?;
    if eh.class != 1 || eh.data != 2 || eh.machine != EM_68K || eh.etype != ET_EXEC || eh.phentsize != 32 || eh.phnum == 0 || eh.phnum > 16 {
        return Err(ENOEXEC);
    }
    let mut ph = vec![0u8; eh.phnum as usize * 32];
    if fs::disk::read(&mut ip.borrow_mut(), eh.phoff, &mut ph)? != ph.len() {
        return Err(ENOEXEC);
    }

    let mut space = AddressSpace::new()?;
    let mut image_end = USER_BASE;
    let mut buf = vec![0u8; 16 * 1024];
    for i in 0..eh.phnum as usize {
        let h = Phdr::parse(&ph, i * 32).ok_or(ENOEXEC)?;
        if h.ptype != PT_LOAD || h.memsz == 0 {
            continue;
        }
        let end = h.vaddr.checked_add(h.memsz).ok_or(ENOEXEC)?;
        if h.vaddr < USER_BASE || end > USER_TOP - vm::STACK_MAX || h.filesz > h.memsz {
            return Err(ENOEXEC);
        }
        space.map_range(h.vaddr, end, true)?;
        let mut done = 0u32;
        while done < h.filesz {
            let chunk = (h.filesz - done).min(buf.len() as u32) as usize;
            let got = fs::disk::read(&mut ip.borrow_mut(), h.offset + done, &mut buf[..chunk])?;
            if got != chunk {
                return Err(ENOEXEC);
            }
            space.write(h.vaddr + done, &buf[..chunk], true)?;
            done += chunk as u32;
        }
        if h.flags & PF_W == 0 {
            let mut va = h.vaddr & !(PAGE - 1);
            while va < end {
                space.protect(va, false);
                va += PAGE;
            }
        }
        image_end = image_end.max(end);
    }
    if eh.entry < USER_BASE || space.translate(eh.entry).is_none() {
        return Err(ENOEXEC);
    }
    space.heap_start = align_up(image_end, PAGE);
    space.brk = space.heap_start;

    // stack with arguments
    let total: usize = argv.iter().chain(envp.iter()).map(|a| a.len() + 1 + 4).sum::<usize>() + 16;
    if total > ARG_MAX {
        return Err(E2BIG);
    }
    let stack_size = vm::STACK_INIT.max(align_up(total as u32 + 4096, PAGE));
    space.map_range(USER_TOP - stack_size, USER_TOP, true)?;
    space.stack_bottom = USER_TOP - stack_size;
    let mut sp = USER_TOP;
    let mut place = |s: &[u8], space: &AddressSpace| -> KResult<u32> {
        sp -= s.len() as u32 + 1;
        space.write(sp, s, false)?;
        space.write(sp + s.len() as u32, &[0], false)?;
        Ok(sp)
    };
    let mut envp_ptrs = Vec::with_capacity(envp.len());
    for e in envp.iter().rev() {
        envp_ptrs.push(place(e, &space)?);
    }
    envp_ptrs.reverse();
    let mut argv_ptrs = Vec::with_capacity(argv.len());
    for a in argv.iter().rev() {
        argv_ptrs.push(place(a, &space)?);
    }
    argv_ptrs.reverse();
    sp &= !3;
    let words = 1 + argv_ptrs.len() + 1 + envp_ptrs.len() + 1;
    sp -= (words * 4) as u32;
    let mut table = Vec::with_capacity(words * 4);
    table.extend_from_slice(&(argv_ptrs.len() as u32).to_be_bytes());
    for a in &argv_ptrs {
        table.extend_from_slice(&a.to_be_bytes());
    }
    table.extend_from_slice(&0u32.to_be_bytes());
    for e in &envp_ptrs {
        table.extend_from_slice(&e.to_be_bytes());
    }
    table.extend_from_slice(&0u32.to_be_bytes());
    space.write(sp, &table, false)?;

    // point of no return: switch to the new image
    let is_current = crate::proc::current_opt().is_some_and(|c| c.pid == p.pid);
    if is_current {
        space.activate();
    }
    p.vm = Some(space);
    for fd in 0..crate::proc::NOFILE {
        if p.cloexec & (1 << fd) != 0 {
            p.files[fd] = None;
        }
    }
    p.cloexec = 0;
    p.sig.exec_reset();
    let base = path.rsplit(|&c| c == b'/').next().unwrap_or(path);
    p.set_name(base);
    if mode & S_IFMT == S_IFREG && mode & S_ISUID != 0 {
        p.euid = uid as u32;
    }
    if mode & S_ISGID != 0 {
        p.egid = gid as u32;
    }
    tf.d = [0; 8];
    tf.a = [0; 7];
    tf.usp = sp;
    tf.pc = eh.entry;
    tf.sr = 0;
    Ok(())
}
