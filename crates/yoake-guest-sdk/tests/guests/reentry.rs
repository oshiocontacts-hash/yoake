//! A guest that proves the shadow stack and the allocator survive re-entry:
//! host -> guest hook OUTER -> `original` -> engine body -> hooked fn -> the same guest, hook INNER -> back.
#![no_std]
extern crate alloc;

use alloc::vec::Vec;
use yoake_guest_sdk::{Handle, Registrar, export_guest, original, report, set};

const OUTER: u32 = 1;
const INNER: u32 = 2;

fn init(r: &mut Registrar) {
    r.hook(OUTER, 0);
    r.hook(INNER, 5);
}

/// Burns shadow stack: every level keeps a 32-word array alive across the recursive call.
#[inline(never)]
fn burn(depth: u32, seed: u32) -> u32 {
    let mut a = [0u32; 32];
    for (i, slot) in a.iter_mut().enumerate() {
        *slot = seed.wrapping_mul(i as u32 + 1);
    }
    if depth == 0 {
        return a.iter().fold(0u32, |x, y| x.wrapping_add(*y));
    }
    let below = burn(depth - 1, seed + 1);
    below.wrapping_add(a[(depth as usize) % 32])
}

fn hook(id: u32, h: Handle) -> i64 {
    match id {
        OUTER => {
            // live across the nested activation: heap blocks and shadow-stack arrays
            let mut v: Vec<u32> = (0..200).collect();
            let stack = [0xA5u8; 64];
            let before = burn(20, 3);
            report(1, 1);
            let _ = original(h);
            let heap_ok = v.iter().enumerate().all(|(i, x)| *x == i as u32);
            let stack_ok = stack.iter().all(|b| *b == 0xA5);
            let after = burn(20, 3);
            v.push(9);
            report(2, i64::from(heap_ok && stack_ok && before == after));
            report(3, v.len() as i64);
            report(4, i64::from(before));
            0
        }
        INNER => {
            let w: Vec<u32> = (0..500).map(|x| x * 2).collect();
            report(10, i64::from(burn(30, 7)));
            report(11, i64::from(w.iter().sum::<u32>()));
            let _ = set(h, 0, 77);
            0
        }
        _ => -1,
    }
}

export_guest!(init, hook);
