//! F1+F2 `yoake-guest-sdk` demo:
//! - `--features server` compiles to a `#![no_std]` core module (`world = "full-access"`) using
//!   `yoake_guest_sdk::worker::{send, recv}` + `hooks::SERVER_TICK`.
//! - `--features worker` compiles to a WASI p2 `worker-mod` component echoing messages back.
#![cfg_attr(feature = "server", no_std)]

#[cfg(feature = "server")]
mod server_guest {
    extern crate alloc;

    use core::sync::atomic::{AtomicBool, Ordering};
    use yoake_guest_sdk::{export_guest, hooks, original, report, worker, Handle, Registrar};

    static SENT: AtomicBool = AtomicBool::new(false);

    fn init(r: &mut Registrar) {
        let _ = r.hook(hooks::SERVER_TICK, 0);
    }

    fn hook(id: u32, root: Handle) -> i64 {
        if id == hooks::SERVER_TICK {
            if !SENT.swap(true, Ordering::Relaxed) {
                let ok = worker::send(b"hello-f1-f2").is_ok();
                report(1, i64::from(ok));
            }
            if let Ok(Some(reply)) = worker::recv() {
                report(2, reply.len() as i64);
            }
        }
        original(root).unwrap_or(0)
    }

    export_guest!(init, hook);
}

#[cfg(feature = "worker")]
mod worker_guest {
    use yoake_wasm_sdk::worker::{self, Worker};

    #[derive(Default)]
    struct EchoWorker;

    impl Worker for EchoWorker {
        fn run(&mut self) {
            while let Some(msg) = worker::recv() {
                let _ = worker::send(&msg);
            }
        }
    }

    yoake_wasm_sdk::export_worker!(EchoWorker);
}
