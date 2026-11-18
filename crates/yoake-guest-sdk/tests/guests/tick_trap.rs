//! A server-tick mod that traps at once (priority 15, between the two others).
#![no_std]
extern crate alloc;

use yoake_guest_sdk::{Handle, Registrar, export_guest, hooks};

fn init(r: &mut Registrar) {
    r.hook(hooks::SERVER_TICK, 15).unwrap();
}

fn hook(_id: u32, _root: Handle) -> i64 {
    panic!("this mod gives up")
}

export_guest!(init, hook);
