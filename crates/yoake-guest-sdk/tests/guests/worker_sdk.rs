//! A no_std full-access guest that talks to its worker through the SDK wrappers (FR-F2).
#![no_std]
extern crate alloc;

use yoake_guest_sdk::worker::{self, RecvError, SendError};
use yoake_guest_sdk::{Handle, Registrar, export_guest, hooks, report};

fn init(r: &mut Registrar) {
    r.hook(hooks::SERVER_TICK, 0).unwrap();
}

fn hook(_id: u32, _root: Handle) -> i64 {
    // slot 1: send result (0 ok, 1 queue full, 2 no worker, 3 too large)
    report(
        1,
        match worker::send(b"echo:ping") {
            Ok(()) => 0,
            Err(SendError::QueueFull) => 1,
            Err(SendError::NoWorker) => 2,
            Err(SendError::TooLarge) => 3,
            Err(SendError::BadPointer) => 4,
        },
    );
    // slot 2: length of a received message, 0 none, -1 no worker
    match worker::recv() {
        Ok(Some(bytes)) => report(2, bytes.len() as i64),
        Ok(None) => report(2, 0),
        Err(RecvError::NoWorker) => report(2, -1),
        Err(RecvError::BadPointer) => report(2, -2),
    }
    0
}

export_guest!(init, hook);
