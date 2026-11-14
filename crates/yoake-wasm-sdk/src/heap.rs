//! Global allocator and panic handler for the `#![no_std]` guest (feature `runtime`).
//!
//! Power-of-two size classes with intrusive free lists on top of `memory.grow`. Freed blocks are
//! reused by later allocations of the same class; memory is never returned to the host, so the
//! footprint is bounded by the peak per class.

#[cfg(all(feature = "runtime", target_arch = "wasm32"))]
mod imp {
    use core::alloc::{GlobalAlloc, Layout};
    use core::arch::wasm32;
    use core::cell::UnsafeCell;

    const PAGE: usize = 65536;
    const MIN_SHIFT: u32 = 4; // 16-byte minimum block
    const CLASSES: usize = 29; // up to 2^32

    struct State {
        free: [*mut u8; CLASSES],
        cur: usize,
        end: usize,
    }

    struct Heap(UnsafeCell<State>);
    // Components are single-threaded.
    unsafe impl Sync for Heap {}

    #[global_allocator]
    static HEAP: Heap = Heap(UnsafeCell::new(State {
        free: [core::ptr::null_mut(); CLASSES],
        cur: 0,
        end: 0,
    }));

    fn class_of(layout: Layout) -> (usize, usize) {
        let size = layout.size().max(layout.align()).max(1 << MIN_SHIFT);
        let shift = size.next_power_of_two().trailing_zeros();
        ((shift - MIN_SHIFT) as usize, 1usize << shift)
    }

    unsafe impl GlobalAlloc for Heap {
        unsafe fn alloc(&self, layout: Layout) -> *mut u8 {
            let st = unsafe { &mut *self.0.get() };
            let (class, size) = class_of(layout);
            if class >= CLASSES {
                return core::ptr::null_mut();
            }
            let head = st.free[class];
            if !head.is_null() {
                st.free[class] = unsafe { *(head as *mut *mut u8) };
                return head;
            }
            // Blocks are size-aligned so any layout up to its own size is satisfied.
            let start = (st.cur + size - 1) & !(size - 1);
            if st.cur == 0 || start + size > st.end {
                let pages = (size + size).div_ceil(PAGE).max(4);
                let old = wasm32::memory_grow(0, pages);
                if old == usize::MAX {
                    return core::ptr::null_mut();
                }
                // Abandon the tail of the old region; start fresh in the new one.
                st.cur = old * PAGE;
                st.end = st.cur + pages * PAGE;
                let start = (st.cur + size - 1) & !(size - 1);
                st.cur = start + size;
                return start as *mut u8;
            }
            st.cur = start + size;
            start as *mut u8
        }

        unsafe fn dealloc(&self, ptr: *mut u8, layout: Layout) {
            let st = unsafe { &mut *self.0.get() };
            let (class, _) = class_of(layout);
            unsafe { *(ptr as *mut *mut u8) = st.free[class] };
            st.free[class] = ptr;
        }
    }

    #[panic_handler]
    fn panic(_: &core::panic::PanicInfo) -> ! {
        wasm32::unreachable()
    }
}
