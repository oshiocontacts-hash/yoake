//! The guest allocator and panic handler (wasm32 only).
//!
//! Size-class free lists over `memory.grow`: a block of a power-of-two size class is popped from its list,
//! or cut from the current chunk, and goes back to its list when freed. Nothing is shared between threads
//! (a wasm guest has none), and nothing depends on the shadow stack, so a nested hook that allocates while
//! an outer hook is inside `original` cannot disturb the outer one's blocks: they stay where they are.

#[cfg(target_arch = "wasm32")]
mod imp {
    use core::alloc::{GlobalAlloc, Layout};
    use core::cell::UnsafeCell;

    const CLASSES: usize = 28;
    const MIN_SHIFT: u32 = 4;
    const PAGE: usize = 65536;

    struct State {
        free: [usize; CLASSES],
        cur: usize,
        end: usize,
    }

    pub struct GuestAlloc(UnsafeCell<State>);

    // SAFETY: a wasm guest runs on one thread; re-entry happens on the same thread between calls, never
    // inside an allocator method.
    unsafe impl Sync for GuestAlloc {}

    impl GuestAlloc {
        pub const fn new() -> Self {
            GuestAlloc(UnsafeCell::new(State { free: [0; CLASSES], cur: 0, end: 0 }))
        }
    }

    fn class_of(layout: &Layout) -> (usize, usize) {
        let size = layout.size().max(layout.align()).max(1 << MIN_SHIFT).next_power_of_two();
        (size, (size.trailing_zeros() - MIN_SHIFT) as usize)
    }

    unsafe impl GlobalAlloc for GuestAlloc {
        unsafe fn alloc(&self, layout: Layout) -> *mut u8 {
            let (size, class) = class_of(&layout);
            if class >= CLASSES {
                return core::ptr::null_mut();
            }
            // SAFETY: single-threaded, see the `Sync` impl; no other reference to the state is live.
            let state = unsafe { &mut *self.0.get() };
            let head = state.free[class];
            if head != 0 {
                // SAFETY: `head` is a block this allocator freed: it holds the next pointer in its first word.
                state.free[class] = unsafe { *(head as *const usize) };
                return head as *mut u8;
            }
            let start = (state.cur + layout.align() - 1) & !(layout.align() - 1);
            if state.cur == 0 || start + size > state.end {
                let pages = size.div_ceil(PAGE).max(16);
                let old = core::arch::wasm32::memory_grow(0, pages);
                if old == usize::MAX {
                    return core::ptr::null_mut();
                }
                state.cur = old * PAGE;
                state.end = state.cur + pages * PAGE;
                let start = (state.cur + layout.align() - 1) & !(layout.align() - 1);
                state.cur = start + size;
                return start as *mut u8;
            }
            state.cur = start + size;
            start as *mut u8
        }

        unsafe fn dealloc(&self, ptr: *mut u8, layout: Layout) {
            let (_, class) = class_of(&layout);
            if class >= CLASSES {
                return;
            }
            // SAFETY: as in `alloc`; the freed block (at least 16 bytes) is ours to write.
            let state = unsafe { &mut *self.0.get() };
            unsafe { *(ptr as *mut usize) = state.free[class] };
            state.free[class] = ptr as usize;
        }
    }

    #[global_allocator]
    static ALLOCATOR: GuestAlloc = GuestAlloc::new();

    #[cfg(not(test))]
    #[panic_handler]
    fn panic(_: &core::panic::PanicInfo<'_>) -> ! {
        // a trap: the host disables the mod and the game goes on (the trap rule)
        core::arch::wasm32::unreachable()
    }
}
