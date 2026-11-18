//! Registers one function hook and one call-site hook for the engine bridge test.
#![no_std]

use yoake_guest_sdk::{Handle, Registrar, export_guest, original, report};

const TICK_SETTLE: u32 = 71;
const ENVIRONMENT_HURT: u32 = 215;

fn init(r: &mut Registrar) {
    r.hook(TICK_SETTLE, 0).unwrap();
    r.hook(ENVIRONMENT_HURT, 5).unwrap();
}

fn hook(id: u32, handle: Handle) -> i64 {
    let entered = match id {
        TICK_SETTLE => 10,
        ENVIRONMENT_HURT => 20,
        _ => {
            report(99, i64::from(id));
            let _ = original(handle);
            return 0;
        }
    };
    report(entered, 1);
    report(entered + 1, i64::from(original(handle).is_ok()));
    0
}

export_guest!(init, hook);
