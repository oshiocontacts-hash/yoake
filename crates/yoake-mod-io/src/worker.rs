//! A mod's worker: a component on its own OS thread with full WASI p2 (files, sockets, name lookup,
//! blocking waits) and `wasi:http` outgoing requests. It has no engine handles; it only talks to the
//! mod through two bounded byte queues. A trap or exit ends only the worker.

use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, OnceLock};
use std::thread::{self, JoinHandle};
use std::time::{Duration, Instant};

use wasmtime::component::{Component, Linker, ResourceTable};
use wasmtime::{Config, Engine, Store, StoreContextMut, StoreLimits, StoreLimitsBuilder, UpdateDeadline};
use wasmtime_wasi::{WasiCtx, WasiCtxBuilder, WasiCtxView, WasiView};
use wasmtime_wasi_http::{WasiHttpCtx, WasiHttpCtxView, WasiHttpView};

use crate::files::FileAccess;
use crate::pipe::{LinePipe, LogSink};
use crate::queue::{MAX_MESSAGES, MAX_QUEUE_BYTES, Queue, SendError};
use crate::{WORKER_HOST_INTERFACE, WORKER_RUN_INTERFACE};

/// Memory one worker may use.
pub const WORKER_MEMORY_LIMIT: usize = 256 * 1024 * 1024;
/// How long [`WorkerHandle::stop`] waits for the thread before it lets it go.
const STOP_GRACE: Duration = Duration::from_millis(500);

#[derive(Clone, Debug)]
pub struct WorkerOptions {
    pub max_messages: usize,
    pub max_bytes: usize,
    pub log: LogSink,
}

impl Default for WorkerOptions {
    fn default() -> Self {
        Self { max_messages: MAX_MESSAGES, max_bytes: MAX_QUEUE_BYTES, log: LogSink::stderr() }
    }
}

/// What a non-blocking receive found.
#[derive(Debug, PartialEq, Eq)]
pub enum RecvOutcome {
    None,
    Message(Vec<u8>),
    /// The next message is longer than the buffer; it stays queued. The value is its length.
    TooSmall(usize),
}

struct Chan {
    to_worker: Queue,
    to_main: Queue,
}

struct WorkerState {
    wasi: WasiCtx,
    http: WasiHttpCtx,
    table: ResourceTable,
    limits: StoreLimits,
    chan: Arc<Chan>,
    stop: Arc<AtomicBool>,
}

impl WasiView for WorkerState {
    fn ctx(&mut self) -> WasiCtxView<'_> {
        WasiCtxView { ctx: &mut self.wasi, table: &mut self.table }
    }
}

impl WasiHttpView for WorkerState {
    fn http(&mut self) -> WasiHttpCtxView<'_> {
        WasiHttpCtxView {
            ctx: &mut self.http,
            table: &mut self.table,
            hooks: wasmtime_wasi_http::default_hooks(),
        }
    }
}

/// The shared engine of all workers: no fuel and no time limit, only an epoch so a stopped worker
/// can be interrupted while it runs guest code.
fn engine() -> Result<&'static Engine, String> {
    static ENGINE: OnceLock<Result<Engine, String>> = OnceLock::new();
    ENGINE
        .get_or_init(|| {
            let mut config = Config::new();
            config.wasm_component_model(true).epoch_interruption(true);
            let engine = Engine::new(&config).map_err(|error| format!("{error:#}"))?;
            let ticker = engine.clone();
            thread::Builder::new()
                .name("yoake-worker-epoch".into())
                .spawn(move || loop {
                    thread::sleep(Duration::from_millis(10));
                    ticker.increment_epoch();
                })
                .map_err(|error| format!("cannot start the worker epoch thread: {error}"))?;
            Ok(engine)
        })
        .as_ref()
        .map_err(Clone::clone)
}

/// The handle to a running worker. Dropping it stops the worker.
pub struct WorkerHandle {
    name: String,
    chan: Arc<Chan>,
    stop: Arc<AtomicBool>,
    alive: Arc<AtomicBool>,
    thread: Option<JoinHandle<()>>,
    log: LogSink,
}

impl WorkerHandle {
    /// Validates a worker component without spawning a thread or running guest code.
    pub fn validate(component_bytes: &[u8]) -> Result<(), String> {
        let engine = engine()?;
        let component = Component::new(engine, component_bytes)
            .map_err(|error| format!("worker is not a valid component: {error:#}"))?;
        if !component
            .component_type()
            .exports(engine)
            .any(|(export, _)| export == WORKER_RUN_INTERFACE)
        {
            return Err(format!(
                "the worker component must export {WORKER_RUN_INTERFACE}"
            ));
        }
        let linker = worker_linker(engine)?;
        linker
            .instantiate_pre(&component)
            .map_err(|error| format!("worker component imports cannot be satisfied: {error:#}"))?;
        Ok(())
    }

    /// Compiles the worker component and starts its thread. A component without the
    /// `worker-run` export, or that does not compile, is an error; a failure while it instantiates or
    /// runs only ends the worker (logged).
    pub fn spawn(name: &str, component_bytes: &[u8], files: FileAccess, options: WorkerOptions) -> Result<Self, String> {
        let engine = engine()?;
        let component =
            Component::new(engine, component_bytes).map_err(|error| format!("worker is not a valid component: {error:#}"))?;
        if !component.component_type().exports(engine).any(|(export, _)| export == WORKER_RUN_INTERFACE) {
            return Err(format!("the worker component must export {WORKER_RUN_INTERFACE}"));
        }
        let chan = Arc::new(Chan {
            to_worker: Queue::with_limits(options.max_messages, options.max_bytes),
            to_main: Queue::with_limits(options.max_messages, options.max_bytes),
        });
        let stop = Arc::new(AtomicBool::new(false));
        let alive = Arc::new(AtomicBool::new(true));
        let prefix = format!("wasm mod {name} worker");
        let thread = {
            let (chan, stop, alive, log, prefix) = (chan.clone(), stop.clone(), alive.clone(), options.log.clone(), prefix.clone());
            thread::Builder::new()
                .name(format!("yoake-worker-{name}"))
                .stack_size(8 * 1024 * 1024)
                .spawn(move || {
                    let result = run(engine, &component, &files, &chan, &stop, &log, &prefix);
                    alive.store(false, Ordering::SeqCst);
                    chan.to_worker.close();
                    match result {
                        Ok(()) => log.log(&format!("{prefix}: finished")),
                        Err(error) if stop.load(Ordering::SeqCst) => {
                            let _ = error;
                            log.log(&format!("{prefix}: stopped"));
                        }
                        Err(error) => log.log(&format!("{prefix}: ended, the worker is disabled: {}", error.replace(['\r', '\n'], " "))),
                    }
                })
                .map_err(|error| format!("cannot start the worker thread: {error}"))?
        };
        Ok(Self { name: name.to_owned(), chan, stop, alive, thread: Some(thread), log: options.log })
    }

    /// Whether the worker thread is still running.
    pub fn is_alive(&self) -> bool {
        self.alive.load(Ordering::SeqCst)
    }

    /// Queues a message for the worker; never blocks.
    pub fn send(&self, message: Vec<u8>) -> Result<(), SendError> {
        self.chan.to_worker.push(message)
    }

    /// The next message from the worker, without waiting.
    pub fn recv(&self) -> Option<Vec<u8>> {
        self.chan.to_main.try_pop()
    }

    /// Like [`recv`](Self::recv) for a buffer of `cap` bytes: a longer message stays queued.
    pub fn recv_into(&self, cap: usize) -> RecvOutcome {
        match self.chan.to_main.peek_len() {
            None => RecvOutcome::None,
            Some(len) if len > cap => RecvOutcome::TooSmall(len),
            Some(_) => self.chan.to_main.try_pop().map_or(RecvOutcome::None, RecvOutcome::Message),
        }
    }

    /// Stops the worker: closes the queues (a blocked `host-recv` returns `none`), interrupts guest
    /// code at the next epoch tick and waits a short while for the thread. A worker blocked inside a
    /// socket call is let go (the thread ends when that call returns).
    pub fn stop(&mut self) {
        self.stop.store(true, Ordering::SeqCst);
        self.chan.to_worker.close();
        let Some(thread) = self.thread.take() else { return };
        let started = Instant::now();
        while !thread.is_finished() && started.elapsed() < STOP_GRACE {
            thread::sleep(Duration::from_millis(2));
        }
        if thread.is_finished() {
            let _ = thread.join();
        } else {
            self.log.log(&format!("wasm mod {} worker: still blocked in a host call; letting it go", self.name));
        }
    }
}

impl Drop for WorkerHandle {
    fn drop(&mut self) {
        self.stop();
    }
}

fn worker_err(context: &'static str) -> impl FnOnce(wasmtime::Error) -> String {
    move |error| format!("{context}: {error:#}")
}

fn worker_linker(engine: &Engine) -> Result<Linker<WorkerState>, String> {
    let mut linker = Linker::<WorkerState>::new(engine);
    wasmtime_wasi::p2::add_to_linker_sync(&mut linker).map_err(worker_err("linking WASI"))?;
    wasmtime_wasi_http::p2::add_only_http_to_linker_sync(&mut linker)
        .map_err(worker_err("linking wasi:http"))?;
    {
        let mut host = linker
            .instance(WORKER_HOST_INTERFACE)
            .map_err(worker_err("linking the worker interface"))?;
        host.func_wrap(
            "host-send",
            |cx: StoreContextMut<'_, WorkerState>, (message,): (Vec<u8>,)| {
                Ok((cx
                    .data()
                    .chan
                    .to_main
                    .push(message)
                    .map_err(|error| error.to_string()),))
            },
        )
        .map_err(worker_err("linking host-send"))?;
        host.func_wrap(
            "host-recv",
            |cx: StoreContextMut<'_, WorkerState>, (): ()| {
                let chan = cx.data().chan.clone();
                Ok((chan.to_worker.pop_blocking(),))
            },
        )
        .map_err(worker_err("linking host-recv"))?;
        host.func_wrap(
            "host-try-recv",
            |cx: StoreContextMut<'_, WorkerState>, (): ()| {
                Ok((cx.data().chan.to_worker.try_pop(),))
            },
        )
        .map_err(worker_err("linking host-try-recv"))?;
    }
    Ok(linker)
}

fn run(
    engine: &Engine,
    component: &Component,
    files: &FileAccess,
    chan: &Arc<Chan>,
    stop: &Arc<AtomicBool>,
    log: &LogSink,
    prefix: &str,
) -> Result<(), String> {
    let linker = worker_linker(engine)?;
    let mut builder = WasiCtxBuilder::new();
    builder
        .stdout(LinePipe::new(prefix, "stdout", log.clone()))
        .stderr(LinePipe::new(prefix, "stderr", log.clone()))
        .inherit_network()
        .allow_tcp(true)
        .allow_udp(true)
        .allow_ip_name_lookup(true);
    files.apply(&mut builder)?;
    let state = WorkerState {
        wasi: builder.build(),
        http: WasiHttpCtx::new(),
        table: ResourceTable::new(),
        limits: StoreLimitsBuilder::new().memory_size(WORKER_MEMORY_LIMIT).instances(16).build(),
        chan: chan.clone(),
        stop: stop.clone(),
    };
    let mut store = Store::new(engine, state);
    store.limiter(|state| &mut state.limits);
    store.epoch_deadline_callback(|context| {
        if context.data().stop.load(Ordering::SeqCst) {
            Err(wasmtime::format_err!("the worker was stopped"))
        } else {
            Ok(UpdateDeadline::Continue(1))
        }
    });
    store.set_epoch_deadline(1);
    let instance = linker.instantiate(&mut store, component).map_err(worker_err("instantiating the worker"))?;
    let interface = instance
        .get_export_index(&mut store, None, WORKER_RUN_INTERFACE)
        .ok_or_else(|| format!("missing export {WORKER_RUN_INTERFACE}"))?;
    let run = instance
        .get_export_index(&mut store, Some(&interface), "run")
        .ok_or("missing export run")?;
    let run = instance.get_typed_func::<(), ()>(&mut store, run).map_err(worker_err("worker run"))?;
    run.call(&mut store, ()).map_err(|error| format!("{error:#}"))
}
