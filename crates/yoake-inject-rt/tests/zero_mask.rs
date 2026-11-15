use std::alloc::{GlobalAlloc, Layout, System};
use std::cell::Cell;
use std::hint::black_box;
use std::time::{Duration, Instant};

use yoake_inject_macro::yoake_inject;

thread_local! {
    static TRACK_ALLOCATIONS: Cell<bool> = const { Cell::new(false) };
    static ALLOCATIONS: Cell<usize> = const { Cell::new(0) };
}

struct CountingAllocator;

fn track_allocation() {
    if TRACK_ALLOCATIONS.try_with(Cell::get).unwrap_or(false) {
        let _ = ALLOCATIONS.try_with(|count| count.set(count.get() + 1));
    }
}

unsafe impl GlobalAlloc for CountingAllocator {
    unsafe fn alloc(&self, layout: Layout) -> *mut u8 {
        track_allocation();
        unsafe { System.alloc(layout) }
    }

    unsafe fn dealloc(&self, pointer: *mut u8, layout: Layout) {
        unsafe { System.dealloc(pointer, layout) }
    }

    unsafe fn realloc(&self, pointer: *mut u8, layout: Layout, size: usize) -> *mut u8 {
        track_allocation();
        unsafe { System.realloc(pointer, layout, size) }
    }

    unsafe fn alloc_zeroed(&self, layout: Layout) -> *mut u8 {
        track_allocation();
        unsafe { System.alloc_zeroed(layout) }
    }
}

#[global_allocator]
static ALLOCATOR: CountingAllocator = CountingAllocator;

#[inline(never)]
fn plain(value: u64) -> u64 {
    value.wrapping_mul(31).wrapping_add(7)
}

#[inline(never)]
#[yoake_inject(id = "tests::zero_mask_allocation", side = "server", allow = "head")]
fn annotated(value: u64) -> u64 {
    value.wrapping_mul(31).wrapping_add(7)
}

#[test]
fn zero_mask_has_no_allocations() {
    assert_eq!(__yoake_inject_site_annotated().mask(), 0);
    ALLOCATIONS.set(0);
    TRACK_ALLOCATIONS.set(true);
    let mut result = 0;
    for value in 0..1024 {
        result ^= annotated(black_box(value));
    }
    TRACK_ALLOCATIONS.set(false);
    black_box(result);
    assert_eq!(ALLOCATIONS.get(), 0);
}

fn measure(function: fn(u64) -> u64, iterations: u64) -> Duration {
    let started = Instant::now();
    let mut result = 0;
    for value in 0..iterations {
        result ^= function(black_box(value));
    }
    black_box(result);
    started.elapsed()
}

#[test]
#[ignore = "timing benchmark: run with --release --ignored --nocapture"]
fn zero_mask_microbenchmark() {
    let iterations = 20_000_000;
    let mut plain_times = Vec::new();
    let mut annotated_times = Vec::new();
    black_box(measure(plain, iterations / 10));
    black_box(measure(annotated, iterations / 10));
    for trial in 0..11 {
        if trial % 2 == 0 {
            plain_times.push(measure(plain, iterations));
            annotated_times.push(measure(annotated, iterations));
        } else {
            annotated_times.push(measure(annotated, iterations));
            plain_times.push(measure(plain, iterations));
        }
    }
    plain_times.sort_unstable();
    annotated_times.sort_unstable();
    let plain_ns = plain_times[5].as_secs_f64() * 1e9 / iterations as f64;
    let annotated_ns = annotated_times[5].as_secs_f64() * 1e9 / iterations as f64;
    println!(
        "zero-mask: median of 11 alternating trials, {iterations} calls/trial; plain={plain_ns:.4} ns/call; annotated={annotated_ns:.4} ns/call; delta={:.4} ns/call ({:.2}%)",
        annotated_ns - plain_ns,
        (annotated_ns / plain_ns - 1.0) * 100.0,
    );
}
