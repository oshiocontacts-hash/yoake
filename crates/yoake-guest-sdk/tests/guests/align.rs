//! Allocations with alignments above 16 (64, 256, 4096), growth, frees and reuse.
#![no_std]
extern crate alloc;

use alloc::boxed::Box;
use alloc::vec::Vec;
use yoake_guest_sdk::{Handle, Registrar, export_guest, report};

#[repr(align(64))]
struct A64([u8; 64]);
#[repr(align(256))]
struct A256(#[allow(dead_code)] u32);
#[repr(align(4096))]
struct A4k(#[allow(dead_code)] [u8; 100]);

fn init(r: &mut Registrar) {
    r.hook(1, 0).unwrap();
}

fn hook(_id: u32, _h: Handle) -> i64 {
    let mut bad = 0i64;
    let mut count = 0i64;
    let mut keep: Vec<Box<A64>> = Vec::new();
    for round in 0..3 {
        for i in 0..50u8 {
            let b = Box::new(A64([i; 64]));
            if (&*b as *const A64 as usize) % 64 != 0 || b.0[63] != i {
                bad += 1;
            }
            count += 1;
            keep.push(b);
        }
        // free every other one so the next round reuses blocks from the free lists
        let mut i = 0;
        keep.retain(|_| {
            i += 1;
            (i + round) % 2 == 0
        });
    }
    let mut wide: Vec<A256> = Vec::new();
    for i in 0..40u32 {
        wide.push(A256(i)); // growth reallocates a Vec of align-256 elements
        if wide.as_ptr() as usize % 256 != 0 {
            bad += 1;
        }
    }
    let big = Box::new(A4k([1; 100]));
    if (&*big as *const A4k as usize) % 4096 != 0 {
        bad += 1;
    }
    report(1, bad);
    report(2, count);
    report(3, keep.len() as i64);
    0
}

export_guest!(init, hook);
