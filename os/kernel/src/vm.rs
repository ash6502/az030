//! Virtual memory with the 68030 PMMU.
//!
//! * Supervisor accesses to 0x00000000-0x3FFFFFFF (RAM) and 0xFC000000-0xFFFFFFFF
//!   (VRAM, I/O, ROM; cache inhibited) go through the transparent translation
//!   registers TT0/TT1: the kernel sees physical memory at its own address.
//! * Everything else, user and supervisor, is translated through the current
//!   process's page tables (CRP, two levels, 4 KB pages, 4 MB per root entry). User
//!   programs live in 0x40000000-0x7FFFFFFF; their stack ends at 0x80000000.
//!
//! The kernel reads and writes user memory by translating each page to its physical
//! frame, so it works on any address space, active or not.

use crate::arch;
use crate::mem::{self, PAGE};
use alloc::string::String;
use alloc::vec::Vec;
use azsys::errno::*;

pub type KResult<T> = Result<T, i32>;

pub const USER_BASE: u32 = 0x4000_0000;
pub const USER_TOP: u32 = 0x8000_0000;
/// Initial stack size; the stack grows on demand up to STACK_MAX.
pub const STACK_INIT: u32 = 64 * 1024;
pub const STACK_MAX: u32 = 8 * 1024 * 1024;

/// TC: enable, 4 KB pages, IS=0, TIA=10, TIB=10.
const TC: u32 = 0x80C0_AA00;
/// TT0: 0x00000000-0x3FFFFFFF, supervisor (FC 4-7), read and write.
const TT0: u32 = 0x003F_8143;
/// TT1: 0xFC000000-0xFFFFFFFF, supervisor, cache inhibited.
const TT1: u32 = 0xFC03_8543;

const DT_PAGE: u32 = 1;
const DT_TABLE: u32 = 2;
const PD_WP: u32 = 1 << 2;
const ADDR_MASK: u32 = 0xFFFF_FF00;
const ENTRIES: u32 = 1024;

/// An empty root table used while no process runs (all user space invalid).
static mut EMPTY_ROOT: u32 = 0;

pub fn init() {
    let root = mem::alloc_frame().expect("no memory for page tables");
    unsafe { EMPTY_ROOT = root };
    arch::mmu_tt(TT0, TT1);
    arch::mmu_crp(root);
    arch::mmu_tc(TC);
    kprintln!("mmu: 68030 PMMU on, 4 KB pages");
}

pub fn activate_none() {
    arch::mmu_crp(unsafe { EMPTY_ROOT });
}

fn idx_a(va: u32) -> u32 {
    va >> 22
}
fn idx_b(va: u32) -> u32 {
    (va >> 12) & (ENTRIES - 1)
}

unsafe fn entry(table: u32, i: u32) -> *mut u32 {
    (table + i * 4) as *mut u32
}

pub struct AddressSpace {
    /// Physical address of the root (level A) table.
    pub root: u32,
    /// End of the loaded image; the heap starts here.
    pub heap_start: u32,
    /// Current program break.
    pub brk: u32,
    /// Lowest mapped stack address.
    pub stack_bottom: u32,
    /// Number of mapped user pages.
    pub pages: u32,
}

impl AddressSpace {
    pub fn new() -> KResult<AddressSpace> {
        let root = mem::alloc_frame().ok_or(ENOMEM)?;
        Ok(AddressSpace { root, heap_start: USER_BASE, brk: USER_BASE, stack_bottom: USER_TOP, pages: 0 })
    }

    pub fn activate(&self) {
        arch::mmu_crp(self.root);
    }

    /// Page descriptor slot for `va`, creating the level-B table if `create`.
    fn slot(&self, va: u32, create: bool) -> KResult<Option<*mut u32>> {
        unsafe {
            let a = entry(self.root, idx_a(va));
            if *a & 3 == 0 {
                if !create {
                    return Ok(None);
                }
                let t = mem::alloc_frame().ok_or(ENOMEM)?;
                *a = t | DT_TABLE;
            }
            Ok(Some(entry(*a & !0xF, idx_b(va))))
        }
    }

    /// Physical address and writability of the page holding `va`.
    pub fn translate(&self, va: u32) -> Option<(u32, bool)> {
        if !(USER_BASE..USER_TOP).contains(&va) {
            return None;
        }
        let p = self.slot(va, false).ok()??;
        let d = unsafe { *p };
        if d & 3 != DT_PAGE {
            return None;
        }
        Some(((d & ADDR_MASK & !(PAGE - 1)) | (va & (PAGE - 1)), d & PD_WP == 0))
    }

    /// Map a fresh zeroed page at `va` (page aligned). Returns its physical address.
    pub fn map_new(&mut self, va: u32, writable: bool) -> KResult<u32> {
        if let Some((pa, _)) = self.translate(va) {
            self.protect(va, writable);
            return Ok(pa & !(PAGE - 1));
        }
        let p = self.slot(va, true)?.unwrap();
        let pa = mem::alloc_frame().ok_or(ENOMEM)?;
        unsafe { *p = pa | DT_PAGE | if writable { 0 } else { PD_WP } };
        self.pages += 1;
        Ok(pa)
    }

    /// Change the write protection of a mapped page.
    pub fn protect(&mut self, va: u32, writable: bool) {
        if let Ok(Some(p)) = self.slot(va, false) {
            unsafe {
                if *p & 3 == DT_PAGE {
                    *p = (*p & !PD_WP) | if writable { 0 } else { PD_WP };
                }
            }
        }
        arch::mmu_flush_all();
    }

    /// Unmap and free the page at `va`.
    pub fn unmap(&mut self, va: u32) {
        if let Ok(Some(p)) = self.slot(va, false) {
            unsafe {
                if *p & 3 == DT_PAGE {
                    mem::free_frame(*p & !(PAGE - 1) & ADDR_MASK);
                    *p = 0;
                    self.pages -= 1;
                }
            }
        }
        arch::mmu_flush_all();
    }

    /// Map fresh pages covering [start, end).
    pub fn map_range(&mut self, start: u32, end: u32, writable: bool) -> KResult<()> {
        let mut va = start & !(PAGE - 1);
        while va < end {
            self.map_new(va, writable)?;
            va += PAGE;
        }
        Ok(())
    }

    /// A copy of this address space (for fork).
    pub fn duplicate(&self) -> KResult<AddressSpace> {
        let mut n = AddressSpace::new()?;
        n.heap_start = self.heap_start;
        n.brk = self.brk;
        n.stack_bottom = self.stack_bottom;
        for ia in idx_a(USER_BASE)..idx_a(USER_TOP) {
            let a = unsafe { *entry(self.root, ia) };
            if a & 3 == 0 {
                continue;
            }
            let table = a & !0xF;
            for ib in 0..ENTRIES {
                let d = unsafe { *entry(table, ib) };
                if d & 3 != DT_PAGE {
                    continue;
                }
                let va = (ia << 22) | (ib << 12);
                let src = d & !(PAGE - 1) & ADDR_MASK;
                let dst = n.map_new(va, d & PD_WP == 0)?;
                unsafe { core::ptr::copy_nonoverlapping(src as *const u8, dst as *mut u8, PAGE as usize) };
            }
        }
        Ok(n)
    }

    /// Copy bytes out of user memory.
    pub fn read(&self, va: u32, buf: &mut [u8]) -> KResult<()> {
        let mut done = 0usize;
        while done < buf.len() {
            let a = va.checked_add(done as u32).ok_or(EFAULT)?;
            let (pa, _) = self.translate(a).ok_or(EFAULT)?;
            let n = ((PAGE - (a & (PAGE - 1))) as usize).min(buf.len() - done);
            unsafe { core::ptr::copy_nonoverlapping(pa as *const u8, buf[done..].as_mut_ptr(), n) };
            done += n;
        }
        Ok(())
    }

    /// Copy bytes into user memory. `force` ignores write protection (used by exec).
    pub fn write(&self, va: u32, data: &[u8], force: bool) -> KResult<()> {
        let mut done = 0usize;
        while done < data.len() {
            let a = va.checked_add(done as u32).ok_or(EFAULT)?;
            let (pa, w) = self.translate(a).ok_or(EFAULT)?;
            if !w && !force {
                return Err(EFAULT);
            }
            let n = ((PAGE - (a & (PAGE - 1))) as usize).min(data.len() - done);
            unsafe { core::ptr::copy_nonoverlapping(data[done..].as_ptr(), pa as *mut u8, n) };
            done += n;
        }
        Ok(())
    }

    /// Fill user memory with zeros.
    pub fn zero(&self, va: u32, len: u32) -> KResult<()> {
        let mut done = 0u32;
        while done < len {
            let a = va + done;
            let (pa, _) = self.translate(a).ok_or(EFAULT)?;
            let n = (PAGE - (a & (PAGE - 1))).min(len - done);
            unsafe { core::ptr::write_bytes(pa as *mut u8, 0, n as usize) };
            done += n;
        }
        Ok(())
    }

    /// Is [va, va+len) mapped (and writable if `write`)?
    pub fn check(&self, va: u32, len: u32, write: bool) -> bool {
        if len == 0 {
            return true;
        }
        let Some(end) = va.checked_add(len - 1) else { return false };
        let mut p = va & !(PAGE - 1);
        loop {
            match self.translate(p) {
                Some((_, w)) if w || !write => {}
                _ => return false,
            }
            if p >= (end & !(PAGE - 1)) {
                return true;
            }
            p += PAGE;
        }
    }

    pub fn read_u32(&self, va: u32) -> KResult<u32> {
        let mut b = [0u8; 4];
        self.read(va, &mut b)?;
        Ok(u32::from_be_bytes(b))
    }

    pub fn write_u32(&self, va: u32, v: u32) -> KResult<()> {
        self.write(va, &v.to_be_bytes(), false)
    }

    /// A NUL-terminated string from user memory (at most `max` bytes).
    pub fn read_cstr(&self, va: u32, max: usize) -> KResult<Vec<u8>> {
        let mut out = Vec::new();
        let mut a = va;
        loop {
            let (pa, _) = self.translate(a).ok_or(EFAULT)?;
            let n = (PAGE - (a & (PAGE - 1))) as usize;
            let bytes = unsafe { core::slice::from_raw_parts(pa as *const u8, n) };
            if let Some(z) = bytes.iter().position(|&b| b == 0) {
                out.extend_from_slice(&bytes[..z]);
                return Ok(out);
            }
            out.extend_from_slice(bytes);
            if out.len() > max {
                return Err(ENAMETOOLONG);
            }
            a += n as u32;
        }
    }

    pub fn read_string(&self, va: u32, max: usize) -> KResult<String> {
        let v = self.read_cstr(va, max)?;
        String::from_utf8(v).map_err(|_| EINVAL)
    }

    /// Grow or shrink the heap so that it ends at `new_brk`.
    pub fn set_brk(&mut self, new_brk: u32) -> KResult<u32> {
        if new_brk < self.heap_start || new_brk >= self.stack_bottom.saturating_sub(PAGE * 16) {
            return Err(ENOMEM);
        }
        let old_end = crate::util::align_up(self.brk, PAGE);
        let new_end = crate::util::align_up(new_brk, PAGE);
        if new_end > old_end {
            let mut va = old_end;
            while va < new_end {
                if let Err(e) = self.map_new(va, true) {
                    // roll back
                    let mut v = old_end;
                    while v < va {
                        self.unmap(v);
                        v += PAGE;
                    }
                    return Err(e);
                }
                va += PAGE;
            }
        } else {
            let mut va = new_end;
            while va < old_end {
                self.unmap(va);
                va += PAGE;
            }
        }
        self.brk = new_brk;
        Ok(new_brk)
    }

    /// Try to grow the stack to cover `va` (called on a user page fault).
    pub fn grow_stack(&mut self, va: u32) -> bool {
        if va >= self.stack_bottom || va < USER_TOP - STACK_MAX || va < crate::util::align_up(self.brk, PAGE) + PAGE * 16 {
            return false;
        }
        let target = va & !(PAGE - 1);
        let mut p = self.stack_bottom - PAGE;
        while p >= target {
            if self.map_new(p, true).is_err() {
                return false;
            }
            p -= PAGE;
        }
        self.stack_bottom = target;
        true
    }
}

impl Drop for AddressSpace {
    fn drop(&mut self) {
        for ia in idx_a(USER_BASE)..idx_a(USER_TOP) {
            let a = unsafe { *entry(self.root, ia) };
            if a & 3 == 0 {
                continue;
            }
            let table = a & !0xF;
            for ib in 0..ENTRIES {
                let d = unsafe { *entry(table, ib) };
                if d & 3 == DT_PAGE {
                    mem::free_frame(d & !(PAGE - 1) & ADDR_MASK);
                }
            }
            mem::free_frame(table);
        }
        mem::free_frame(self.root);
    }
}
