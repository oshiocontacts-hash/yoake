//! Guest SDK support for `[wasm] worker` components (FR-F2, `worker-mod` in `wit-worker/yoake-worker.wit`).
//!
//! ```ignore
//! use yoake_wasm_sdk::worker::{self, Worker};
//!
//! #[derive(Default)]
//! struct MyWorker;
//! impl Worker for MyWorker {
//!     fn run(&mut self) {
//!         while let Some(msg) = worker::recv() {
//!             let _ = worker::send(&msg);
//!         }
//!     }
//! }
//! yoake_wasm_sdk::export_worker!(MyWorker);
//! ```

#[doc(hidden)]
pub mod bindings {
    wit_bindgen::generate!({
        path: "../yoake-wasm/wit-worker",
        world: "worker-mod",
        generate_all,
        pub_export_macro: true,
        default_bindings_module: "yoake_wasm_sdk::worker::bindings",
    });
}

use crate::prelude::*;
use bindings::yoake::mods::worker_host;

/// Entry trait implemented by a worker component.
pub trait Worker: Default + 'static {
    /// Runs once on the worker's dedicated OS thread. Returning stops the worker.
    fn run(&mut self);
}

/// Queues a reply message for the main mod instance. Never blocks.
pub fn send(message: &[u8]) -> Result<(), String> {
    worker_host::host_send(message)
}

/// Blocks until the next message arrives from the main instance, or returns `None` when stopping.
pub fn recv() -> Option<Vec<u8>> {
    worker_host::host_recv()
}

/// Returns the next message from the main instance without blocking, if one is queued.
pub fn try_recv() -> Option<Vec<u8>> {
    worker_host::host_try_recv()
}

/// Exports a [`Worker`] implementation as `yoake:mods/worker-run@1.6.0`.
#[macro_export]
macro_rules! export_worker {
    ($ty:ty) => {
        const _: () = {
            $crate::__export_realloc!();
            struct Shim;
            impl $crate::worker::bindings::exports::yoake::mods::worker_run::Guest for Shim {
                fn run() {
                    let mut worker = <$ty as ::core::default::Default>::default();
                    <$ty as $crate::worker::Worker>::run(&mut worker);
                }
            }
    $crate::worker::bindings::export!(Shim with_types_in $crate::worker::bindings);
        };
    };
}
