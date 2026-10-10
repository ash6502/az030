//! The process heap: a first-fit free list (16-byte granules, coalescing on free)
//! that grows the data segment with `brk` as needed.

use crate::sys::{self, nr};
use core::alloc::{GlobalAlloc, Layout};
use core::cell::UnsafeCell;
use core::ptr::null_mut;

const GRAIN: usize = 16;
const GROW: usize = 64 * 1024;

struct Hole {
    size: usize,
    next: *mut Hole,
}

struct State {
    head: *mut Hole,
    end: usize,
}

struct Heap(UnsafeCell<State>);

unsafe impl Sync for Heap {}

#[global_allocator]
static HEAP: Heap = Heap(UnsafeCell::new(State { head: null_mut(), end: 0 }));

fn brk(addr: usize) -> usize {
    sys::call1(nr::BRK, addr as u32).map_or(0, |v| v as usize)
}

impl Heap {
    /// Add [start, start+size) to the free list (sorted, coalescing).
    unsafe fn free_range(st: &mut State, start: usize, size: usize) {
        unsafe {
            let mut prev: *mut Hole = null_mut();
            let mut next = st.head;
            while !next.is_null() && (next as usize) < start {
                prev = next;
                next = (*next).next;
            }
            let node = start as *mut Hole;
            (*node).size = size;
            (*node).next = next;
            if !next.is_null() && start + size == next as usize {
                (*node).size += (*next).size;
                (*node).next = (*next).next;
            }
            if prev.is_null() {
                st.head = node;
            } else if prev as usize + (*prev).size == start {
                (*prev).size += (*node).size;
                (*prev).next = (*node).next;
            } else {
                (*prev).next = node;
            }
        }
    }

    /// Grow the heap by at least `need` bytes.
    unsafe fn grow(st: &mut State, need: usize) -> bool {
        if st.end == 0 {
            st.end = brk(0).next_multiple_of(GRAIN);
        }
        let amount = (need + GRAIN).next_multiple_of(GROW);
        let new_end = st.end + amount;
        if brk(new_end) < new_end {
            return false;
        }
        let start = st.end;
        st.end = new_end;
        unsafe { Self::free_range(st, start, amount) };
        true
    }
}

unsafe impl GlobalAlloc for Heap {
    unsafe fn alloc(&self, layout: Layout) -> *mut u8 {
        let size = layout.size().max(1).next_multiple_of(GRAIN);
        let align = layout.align().max(GRAIN);
        let st = unsafe { &mut *self.0.get() };
        for attempt in 0..2 {
            unsafe {
                let mut prev: *mut Hole = null_mut();
                let mut cur = st.head;
                while !cur.is_null() {
                    let a = cur as usize;
                    let s = (*cur).size;
                    let start = a.next_multiple_of(align);
                    if start + size <= a + s {
                        let pad = start - a;
                        let tail = a + s - (start + size);
                        let next = (*cur).next;
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
                            st.head = after;
                        } else {
                            (*prev).next = after;
                        }
                        return start as *mut u8;
                    }
                    prev = cur;
                    cur = (*cur).next;
                }
                if attempt == 0 && !Self::grow(st, size + align) {
                    return null_mut();
                }
            }
        }
        null_mut()
    }

    unsafe fn dealloc(&self, ptr: *mut u8, layout: Layout) {
        let size = layout.size().max(1).next_multiple_of(GRAIN);
        let st = unsafe { &mut *self.0.get() };
        unsafe { Self::free_range(st, ptr as usize, size) };
    }
}
