//! The VM must not allocate while evaluating: a counting allocator proves it.

use horizon_mobmodel::expr::{compile, ExprInputs, Symbols};
use std::alloc::{GlobalAlloc, Layout, System};
use std::cell::Cell;

struct Counting;

thread_local! {
    static ALLOCS: Cell<usize> = const { Cell::new(0) };
}

unsafe impl GlobalAlloc for Counting {
    unsafe fn alloc(&self, l: Layout) -> *mut u8 {
        let _ = ALLOCS.try_with(|c| c.set(c.get() + 1));
        unsafe { System.alloc(l) }
    }
    unsafe fn dealloc(&self, p: *mut u8, l: Layout) {
        unsafe { System.dealloc(p, l) }
    }
    unsafe fn realloc(&self, p: *mut u8, l: Layout, n: usize) -> *mut u8 {
        let _ = ALLOCS.try_with(|c| c.set(c.get() + 1));
        unsafe { System.realloc(p, l, n) }
    }
}

#[global_allocator]
static A: Counting = Counting;

#[test]
fn evaluation_does_not_allocate() {
    let mut syms = Symbols::new(&["mode"]);
    let small = compile(
        "q.health > 3 ? math.sin(q.anim_time * 90) * 2 : math.random(0, 1) + v.x + query.synced('mode')",
        &mut syms,
    )
    .unwrap();
    let deep_src = format!("{}0{}", "q.health + (".repeat(40), ")".repeat(40));
    let deep = compile(&deep_src, &mut syms).unwrap();
    let mut inp = ExprInputs::default();
    inp.query[8] = 5.0;
    let before = ALLOCS.with(|c| c.get());
    let mut acc = 0.0;
    for i in 0..1000 {
        inp.random_epoch = i;
        acc += small.eval(&inp) + deep.eval(&inp);
    }
    let after = ALLOCS.with(|c| c.get());
    assert!(acc.is_finite());
    assert_eq!(after - before, 0, "the VM allocated");
}
