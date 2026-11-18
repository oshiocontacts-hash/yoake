//! `memory.grow` while an outer hook is waiting in `original`: the outer hook's blocks survive and it can go on.
#![no_std]
extern crate alloc;

use alloc::vec;
use alloc::vec::Vec;
use yoake_guest_sdk::{Handle, Registrar, export_guest, original, report};

fn init(r: &mut Registrar) {
    r.hook(1, 0).unwrap();
    r.hook(2, 0).unwrap();
}

fn pages() -> i64 {
    core::arch::wasm32::memory_size(0) as i64
}

fn hook(id: u32, h: Handle) -> i64 {
    if id == 1 {
        let a: Vec<u32> = (0..1000).collect();
        let before = pages();
        let _ = original(h);
        let after = pages();
        let intact = a.iter().enumerate().all(|(i, x)| *x == i as u32);
        let more: Vec<u32> = vec![5; 100_000];
        report(1, i64::from(intact));
        report(2, after - before);
        report(3, more.len() as i64 + i64::from(more[99_999]));
    } else {
        let big = vec![7u8; 3 << 20];
        report(10, i64::from(big[0]) + i64::from(big[big.len() - 1]) + (big.len() >> 20) as i64);
    }
    0
}

export_guest!(init, hook);
