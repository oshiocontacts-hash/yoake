//! Files, network and workers for Yoake WebAssembly mods (FR-F2).
//!
//! The owner's rule (2026-11-20): mods are used inside one group, so they can do what a Forge or
//! Fabric mod can: files and network. What stays: nothing changes without a mod, and the main game
//! thread never blocks on a mod. Files are synchronous and short, so a mod may use them on the
//! game thread. Network can wait for a long time, so it runs in a **worker**: a component on its
//! own OS thread, talking to the mod through a bounded byte-message channel.
//!
//! * [`files`]: the per-mod preopens (`/data`, `/game`, optionally `/host/<letter>`).
//! * [`queue`]: the bounded byte-message queue.
//! * [`worker`]: the worker component's thread, with full WASI p2 and `wasi:http`.

pub mod files;
pub mod queue;
pub mod worker;

mod pipe;

pub use files::{FileAccess, valid_mod_id};
pub use pipe::{LinePipe, LogSink};
pub use queue::{MAX_MESSAGES, MAX_QUEUE_BYTES, Queue, SendError};
pub use worker::{RecvOutcome, WorkerHandle, WorkerOptions};

/// The interface a mod's main instance imports to talk to its worker.
pub const WORKER_INTERFACE: &str = "yoake:mods/worker@1.6.0";
/// The same two calls for the full worlds (FR-REM05), in `yoake-full.wit`.
pub const WORKER_FULL_INTERFACE: &str = "yoake:mods/worker@1.7.0";
/// The interface the worker component imports.
pub const WORKER_HOST_INTERFACE: &str = "yoake:mods/worker-host@1.6.0";
/// The interface the worker component exports (`run: func()`).
pub const WORKER_RUN_INTERFACE: &str = "yoake:mods/worker-run@1.6.0";

/// What a mod gets: its folders and, optionally, a worker component.
#[derive(Clone, Debug)]
pub struct ModIo {
    pub files: FileAccess,
    /// The bytes of the `[wasm] worker` component.
    pub worker: Option<Vec<u8>>,
    pub log: LogSink,
}

impl ModIo {
    pub fn new(files: FileAccess) -> Self {
        Self { files, worker: None, log: LogSink::stderr() }
    }
}

/// The WASI crate this one was built with (the slice's core-module runtime links WASI p1 from it).
pub use wasmtime_wasi;
