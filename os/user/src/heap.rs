//! The process heap.
//!
//! Small blocks (up to 2 KB) come from per-size-class free lists, so allocating
//! and freeing them is O(1); a class gets more blocks by carving a 16 KB chunk.
//! Larger blocks use a first-fit, address-ordered free list with coalescing.
//! Both grow the data segment with `brk`.

use crate::sys::{self, nr};
use core::alloc::{GlobalAlloc, Layout};
use core::cell::UnsafeCell;
use core::ptr::null_mut;

const GRAIN: usize = 16;
const GROW: usize = 64 * 1024;
const CHUNK: usize = 16 * 1024;

/// Size classes for small blocks.
const CLASSES: [usize; 16] = [16, 32, 48, 64, 80, 96, 128, 160, 192, 256, 384, 512, 768, 1024, 1536, 2048];
const SMALL_MAX: usize = 2048;

struct Hole {
    size: usize,
    next: *mut Hole,
}

struct Free {
    next: *mut Free,
}

struct State {
    head: *mut Hole,
    end: usize,
    small: [*mut Free; 16],
}

struct Heap(UnsafeCell<State>);

unsafe impl Sync for Heap {}

#[global_allocator]
static HEAP: Heap = Heap(UnsafeCell::new(State { head: null_mut(), end: 0, small: [null_mut(); 16] }));

fn brk(addr: usize) -> usize {
    sys::call1(nr::BRK, addr as u32).map_or(0, |v| v as usize)
}

fn class_of(size: usize) -> Option<usize> {
    if size > SMALL_MAX {
        return None;
    }
    // CLASSES is short: a linear scan is as fast as anything here
    CLASSES.iter().position(|&c| c >= size)
}

impl Heap {
    /// Add [start, start+size) to the large free list (sorted, coalescing).
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

    /// First-fit allocation from the large free list.
    unsafe fn alloc_large(st: &mut State, size: usize, align: usize) -> *mut u8 {
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
}

unsafe impl GlobalAlloc for Heap {
    unsafe fn alloc(&self, layout: Layout) -> *mut u8 {
        let size = layout.size().max(1).next_multiple_of(GRAIN);
        let align = layout.align().max(GRAIN);
        let st = unsafe { &mut *self.0.get() };
        if align == GRAIN {
            if let Some(c) = class_of(size) {
                unsafe {
                    if st.small[c].is_null() {
                        // carve a chunk into blocks of this class
                        let bs = CLASSES[c];
                        let chunk = Self::alloc_large(st, CHUNK, GRAIN);
                        if chunk.is_null() {
                            return null_mut();
                        }
                        let n = CHUNK / bs;
                        for i in (0..n).rev() {
                            let b = chunk.add(i * bs) as *mut Free;
                            (*b).next = st.small[c];
                            st.small[c] = b;
                        }
                    }
                    let b = st.small[c];
                    st.small[c] = (*b).next;
                    return b as *mut u8;
                }
            }
        }
        unsafe { Self::alloc_large(st, size, align) }
    }

    unsafe fn dealloc(&self, ptr: *mut u8, layout: Layout) {
        let size = layout.size().max(1).next_multiple_of(GRAIN);
        let st = unsafe { &mut *self.0.get() };
        if layout.align() <= GRAIN {
            if let Some(c) = class_of(size) {
                let b = ptr as *mut Free;
                unsafe {
                    (*b).next = st.small[c];
                }
                st.small[c] = b;
                return;
            }
        }
        unsafe { Self::free_range(st, ptr as usize, size) };
    }

    unsafe fn realloc(&self, ptr: *mut u8, layout: Layout, new_size: usize) -> *mut u8 {
        // staying within the same small class needs no copy
        let old = layout.size().max(1).next_multiple_of(GRAIN);
        let new = new_size.max(1).next_multiple_of(GRAIN);
        if layout.align() <= GRAIN {
            if let (Some(a), Some(b)) = (class_of(old), class_of(new)) {
                if a == b {
                    return ptr;
                }
            }
        }
        let new_layout = unsafe { Layout::from_size_align_unchecked(new_size, layout.align()) };
        let p = unsafe { self.alloc(new_layout) };
        if !p.is_null() {
            unsafe {
                core::ptr::copy_nonoverlapping(ptr, p, layout.size().min(new_size));
                self.dealloc(ptr, layout);
            }
        }
        p
    }
}
