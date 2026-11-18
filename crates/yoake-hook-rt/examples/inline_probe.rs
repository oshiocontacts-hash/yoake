use std::ops::ControlFlow;
use std::sync::atomic::{AtomicUsize, Ordering};
use yoake_hook_macro::yoake_hook;

static CALLS: AtomicUsize = AtomicUsize::new(0);

#[yoake_hook]
#[inline(always)]
fn inlined_leaf(value: u64) -> u64 {
    value.wrapping_add(1)
}

fn observe(_: &mut u64) -> ControlFlow<u64> {
    CALLS.fetch_add(1, Ordering::Relaxed);
    ControlFlow::Continue(())
}

#[unsafe(no_mangle)]
#[inline(never)]
pub fn yoake_inline_driver(value: u64) -> u64 {
    inlined_leaf(std::hint::black_box(value))
}

fn main() {
    __yoake_hook_inlined_leaf()
        .register_head(observe, yoake_hook_rt::Order::default())
        .unwrap();
    let mut checksum = 0;
    for value in 0..100 {
        checksum += yoake_inline_driver(value);
    }
    assert_eq!(checksum, 5050);
    assert_eq!(CALLS.load(Ordering::Relaxed), 100);
    println!("100 inlined calls observed; checksum={checksum}");
}
