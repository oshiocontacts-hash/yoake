//! Calls test bindings through the SDK: a small result, a 300-byte result (does not fit the SDK's first
//! 256-byte buffer, so it comes back through `engine_take`), and an unknown id.
#![no_std]
extern crate alloc;

use yoake_guest_sdk::{Handle, Registrar, call, export_guest, report};

fn init(r: &mut Registrar) {
    r.hook(1, 0).unwrap();
}

fn hook(_id: u32, h: Handle) -> i64 {
    match call(h, 1, &[]) {
        Ok(bytes) => report(1, bytes.len() as i64),
        Err(_) => report(1, -1),
    }
    match call(h, 2, &[]) {
        Ok(bytes) => {
            report(2, bytes.len() as i64);
            report(3, i64::from(bytes[0]));
            report(4, i64::from(bytes[299]));
            let all = bytes.iter().enumerate().all(|(i, b)| *b == (i % 251) as u8);
            report(5, i64::from(all));
        }
        Err(_) => report(2, -1),
    }
    match call(h, 999, &[]) {
        Ok(_) => report(6, 1),
        Err(e) => report(6, match e {
            yoake_guest_sdk::Error::Status(s) => i64::from(s),
            _ => -99,
        }),
    }
    0
}

export_guest!(init, hook);
