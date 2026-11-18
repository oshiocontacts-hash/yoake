//! The nested hook traps (a panic): the outer hook, waiting in `original`, continues.
#![no_std]
extern crate alloc;

use alloc::vec::Vec;
use yoake_guest_sdk::{Handle, Registrar, export_guest, original, report};

fn init(r: &mut Registrar) {
    r.hook(1, 0).unwrap();
    r.hook(2, 0).unwrap();
}

fn hook(id: u32, h: Handle) -> i64 {
    if id == 1 {
        let data: Vec<u32> = (0..64).collect();
        report(1, 1);
        let _ = original(h);
        report(2, i64::from(data.iter().enumerate().all(|(i, x)| *x == i as u32)));
    } else {
        report(10, 1);
        panic!("the nested hook gives up");
    }
    0
}

export_guest!(init, hook);
