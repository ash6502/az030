//! Physical memory: the kernel heap and the page-frame allocator.
//!
//! ```text
//! 0x00000000  low memory: ROM variables, bootloader, boot info
//! 0x00010000  kernel image (text, rodata, data, bss)
//! _end        kernel heap (RAM / 8, 2-32 MB)
//! heap end    page frames for user memory and page tables
//! RAM top
//! ```
//!
//! The kernel addresses physical memory directly (identity mapping through TT0).

use crate::arch;
use crate::util::{align_up, Global};
use core::alloc::{GlobalAlloc, Layout};
use core::ptr::null_mut;

pub const PAGE: u32 = 4096;

unsafe extern "C" {
    static __end: u8;
}

struct Hole {
    size: usize,
    next: *mut Hole,
}

struct Heap {
    head: *mut Hole,
    start: usize,
    end: usize,
    used: usize,
}

struct Frames {
    head: u32,
    free: u32,
    total: u32,
    base: u32,
}

static HEAP: Global<Heap> = Global::new(Heap { head: null_mut(), start: 0, end: 0, used: 0 });
static FRAMES: Global<Frames> = Global::new(Frames { head: 0, free: 0, total: 0, base: 0 });
static RAM: Global<u32> = Global::new(0);
static KERNEL_END: Global<u32> = Global::new(0);

const GRAIN: usize = 16;

pub fn init(ram_size: u32) {
    let kend = align_up(core::ptr::addr_of!(__end) as u32, PAGE);
    let heap_size = (ram_size / 8).clamp(2 << 20, 32 << 20) & !(PAGE - 1);
    let heap_end = kend + heap_size;
    if heap_end + (1 << 20) > ram_size {
        panic!("not enough memory ({} KB)", ram_size / 1024);
    }
    *RAM.get() = ram_size;
    *KERNEL_END.get() = kend;
    let h = HEAP.get();
    h.start = kend as usize;
    h.end = heap_end as usize;
    let hole = kend as *mut Hole;
    unsafe {
        (*hole).size = heap_size as usize;
        (*hole).next = null_mut();
    }
    h.head = hole;

    // every frame above the heap goes on the free list
    let f = FRAMES.get();
    f.base = heap_end;
    let mut pa = ram_size - PAGE;
    loop {
        unsafe { *(pa as *mut u32) = f.head };
        f.head = pa;
        f.free += 1;
        if pa == heap_end {
            break;
        }
        pa -= PAGE;
    }
    f.total = f.free;
    kprintln!(
        "mem: {} KB RAM, kernel {} KB, heap {} KB, {} KB free",
        ram_size / 1024,
        (kend - 0x10000) / 1024,
        heap_size / 1024,
        f.free * PAGE / 1024
    );
}

/// Allocate a zeroed 4 KB page frame.
pub fn alloc_frame() -> Option<u32> {
    let sr = arch::irq_save();
    let f = FRAMES.get();
    let pa = f.head;
    if pa == 0 {
        arch::irq_restore_sr(sr);
        return None;
    }
    f.head = unsafe { *(pa as *const u32) };
    f.free -= 1;
    arch::irq_restore_sr(sr);
    unsafe { core::ptr::write_bytes(pa as *mut u8, 0, PAGE as usize) };
    Some(pa)
}

pub fn free_frame(pa: u32) {
    let f = FRAMES.get();
    debug_assert!(pa >= f.base && pa % PAGE == 0);
    let sr = arch::irq_save();
    unsafe { *(pa as *mut u32) = f.head };
    f.head = pa;
    f.free += 1;
    arch::irq_restore_sr(sr);
}

pub struct MemStats {
    pub total_kb: u32,
    pub free_kb: u32,
    pub kernel_kb: u32,
    pub heap_used_kb: u32,
}

pub fn stats() -> MemStats {
    let f = FRAMES.get();
    let h = HEAP.get();
    MemStats {
        total_kb: *RAM.get() / 1024,
        free_kb: f.free * PAGE / 1024 + ((h.end - h.start - h.used) / 1024) as u32,
        kernel_kb: (*KERNEL_END.get() - 0x10000) / 1024,
        heap_used_kb: (h.used / 1024) as u32,
    }
}

struct KernelAlloc;

#[global_allocator]
static ALLOC: KernelAlloc = KernelAlloc;

unsafe impl GlobalAlloc for KernelAlloc {
    unsafe fn alloc(&self, layout: Layout) -> *mut u8 {
        let size = layout.size().max(1).next_multiple_of(GRAIN);
        let align = layout.align().max(GRAIN);
        let sr = arch::irq_save();
        let h = HEAP.get();
        let mut prev: *mut Hole = null_mut();
        let mut cur = h.head;
        let mut result = null_mut();
        unsafe {
            while !cur.is_null() {
                let a = cur as usize;
                let s = (*cur).size;
                let start = a.next_multiple_of(align);
                let pad = start - a;
                if start + size <= a + s {
                    let tail = a + s - (start + size);
                    let next = (*cur).next;
                    // the part after the allocation stays free
                    let after = if tail > 0 {
                        let t = (start + size) as *mut Hole;
                        (*t).size = tail;
                        (*t).next = next;
                        t
                    } else {
                        next
                    };
                    if pad > 0 {
                        (*cur).size = pad;
                        (*cur).next = after;
                    } else if prev.is_null() {
                        h.head = after;
                    } else {
                        (*prev).next = after;
                    }
                    h.used += size;
                    result = start as *mut u8;
                    break;
                }
                prev = cur;
                cur = (*cur).next;
            }
        }
        arch::irq_restore_sr(sr);
        result
    }

    unsafe fn dealloc(&self, ptr: *mut u8, layout: Layout) {
        let size = layout.size().max(1).next_multiple_of(GRAIN);
        let a = ptr as usize;
        let sr = arch::irq_save();
        let h = HEAP.get();
        h.used -= size;
        unsafe {
            // find the holes around the freed block (list is sorted by address)
            let mut prev: *mut Hole = null_mut();
            let mut next = h.head;
            while !next.is_null() && (next as usize) < a {
                prev = next;
                next = (*next).next;
            }
            let node = a as *mut Hole;
            (*node).size = size;
            (*node).next = next;
            if !next.is_null() && a + size == next as usize {
                (*node).size += (*next).size;
                (*node).next = (*next).next;
            }
            if prev.is_null() {
                h.head = node;
            } else if prev as usize + (*prev).size == a {
                (*prev).size += (*node).size;
                (*prev).next = (*node).next;
            } else {
                (*prev).next = node;
            }
        }
        arch::irq_restore_sr(sr);
    }
}
