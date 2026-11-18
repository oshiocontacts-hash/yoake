use std::sync::atomic::{AtomicUsize, Ordering};
use yoake_hook_macro::yoake_hook;

static CALLS: AtomicUsize = AtomicUsize::new(0);

#[yoake_hook(test_id = 1)]
#[inline(always)]
fn inlined_leaf(value: u64) -> u64 {
    value.wrapping_add(1)
}

fn boost(original: &mut dyn FnMut(u64) -> u64, value: u64) -> u64 {
    CALLS.fetch_add(1, Ordering::Relaxed);
    original(value) + 10
}

#[unsafe(no_mangle)]
#[inline(never)]
pub fn yoake_inline_driver(value: u64) -> u64 {
    inlined_leaf(std::hint::black_box(value))
}

fn main() {
    assert_eq!(yoake_inline_driver(4), 5);
    __yoake_register_inlined_leaf(boost, yoake_hook_rt::Order::default()).unwrap();
    let mut checksum = 0;
    for value in 0..100 {
        checksum += yoake_inline_driver(value);
    }
    assert_eq!(checksum, 6050);
    assert_eq!(CALLS.load(Ordering::Relaxed), 100);
    println!("100 inlined calls observed; checksum={checksum}");
}
