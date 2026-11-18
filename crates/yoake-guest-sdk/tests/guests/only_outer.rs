//! Registers only hook 1: hook 2 must never enter this guest.
#![no_std]
extern crate alloc;

use yoake_guest_sdk::{Handle, Registrar, export_guest, original, report};

fn init(r: &mut Registrar) {
    r.hook(1, 0);
}

fn hook(id: u32, h: Handle) -> i64 {
    report(i32::try_from(id).unwrap_or(-1) + 100, 1);
    if id == 1 {
        let _ = original(h);
    }
    0
}

export_guest!(init, hook);
