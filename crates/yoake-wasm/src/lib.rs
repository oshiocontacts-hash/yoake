use std::collections::HashMap;

#[cfg(test)]
#[path = "../../../tests/support/temp_dir.rs"]
mod test_support;
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use std::sync::{Arc, Mutex, Weak};
use std::thread::{self, JoinHandle};
use std::time::{Duration, Instant};
use wasmtime::component::{Component, Instance, Linker, ResourceTable};
use wasmtime_wasi_http::{WasiHttpCtx, WasiHttpCtxView, WasiHttpView};
use yoake_mod_io::{WorkerHandle, WorkerOptions};
use wasmtime::{Config, Engine, Store, StoreLimits, StoreLimitsBuilder, UpdateDeadline};
use wasmtime_wasi::{WasiCtx, WasiCtxBuilder, WasiCtxView, WasiView};

mod client;
mod client_services;
pub mod config_sync;
pub mod convert;
mod full;
pub mod full_bindings;
mod full_hook_names;
mod gpu_services;
mod hook_names;
mod host;
pub mod host_services;
pub mod inject;
pub mod package_services;
mod subscriptions;
#[cfg(feature = "test-guests")]
pub mod test_guests;
mod test_mod;
mod wasi;

pub mod bindings {
    wasmtime::component::bindgen!({
        path: "wit",
        world: "client-mod",
    });
}

pub use client::{ClientMod, LoadError};
pub use client_services::{ClientServices, client_types};
pub use full::{ClientFullMod, SERVER_SERVICES, ServerFullMod, ServerHost, server_types};
pub use full_hook_names::{FULL_HOOKS as FULL_HOOK_LIST, HookSide};
pub use gpu_services::{GpuServices, gpu_types};
pub use host::{ClientContext, ClientHost};
pub use subscriptions::FullWorld;

pub use subscriptions::Subscriptions;
pub use test_mod::{TestHost, TestMod, TestOutcome, test_driver, validate_test_names};
pub use wasi::{OutputLine, OutputStream, WASI_IMPORT_INTERFACES};
pub use yoake_mod_io::{FileAccess, LogSink, ModIo};

pub const MEMORY_LIMIT_BYTES: usize = 64 * 1024 * 1024;
/// Elements one table may hold (a Rust guest's function table is a few thousand).
pub const TABLE_ELEMENT_LIMIT: usize = 65_536;
/// Tables one instance may have.
pub const TABLE_LIMIT: usize = 8;
const EPOCH_INTERVAL: Duration = Duration::from_millis(1);

thread_local! {
    /// Wall-clock time the running call spent inside host services (the game doing what the guest asked:
    /// breaking a block, reading a container). It is the game's time, not the guest's: it extends the
    /// call's deadline and does not count against the aggregate budget. The guest's own work is still
    /// bounded by fuel, and every service call burns some.
    static HOST_TIME: std::cell::Cell<Duration> = const { std::cell::Cell::new(Duration::ZERO) };
    static STORAGE_TIME: std::cell::Cell<Duration> = const { std::cell::Cell::new(Duration::ZERO) };
    static STORAGE_THROTTLED: std::cell::Cell<bool> = const { std::cell::Cell::new(false) };
    static STORAGE_WINDOW: std::cell::Cell<Option<Instant>> = const { std::cell::Cell::new(None) };
    static COUNTED_HOST_TIME: std::cell::Cell<Duration> = const { std::cell::Cell::new(Duration::ZERO) };
}

/// The most host-service time that extends one call's deadline (a guest that loops over services cannot hold the
/// server thread longer than this plus its own budget).
pub const MAX_HOST_TIME: Duration = Duration::from_millis(400);

/// Adds `spent` to the running call's host-service time (see [`HOST_TIME`]).
pub(crate) fn add_host_time(spent: Duration) {
    HOST_TIME.with(|time| time.set(time.get() + spent));
}

pub(crate) fn add_counted_host_time(spent: Duration) {
    COUNTED_HOST_TIME.with(|time| time.set(time.get() + spent));
}

pub(crate) fn add_storage_time(spent: Duration) {
    STORAGE_TIME.with(|time| time.set(time.get() + spent));
}

pub(crate) fn storage_throttled() {
    STORAGE_THROTTLED.with(|throttled| throttled.set(true));
}

pub(crate) fn storage_window_start() -> Instant {
    STORAGE_WINDOW
        .with(std::cell::Cell::get)
        .unwrap_or_else(Instant::now)
}

fn deadline_extension() -> Duration {
    (HOST_TIME.with(std::cell::Cell::get)
        + STORAGE_TIME.with(std::cell::Cell::get)
        + COUNTED_HOST_TIME.with(std::cell::Cell::get))
    .min(MAX_HOST_TIME)
}
/// One client hook call may run this long of wall-clock time (plus one epoch tick) before it is
/// stopped and the module disabled. A call is stopped by the clock, not by its CPU time, and a busy
/// machine can pause a thread for a whole scheduler quantum (15.6 ms on Windows): a tighter limit
/// (the design's 2 ms) disabled healthy mods in practice. How much of the frame a mod may use is
/// [`CLIENT_AGGREGATE_BUDGET`]'s business.
pub const CLIENT_HOOK_BUDGET: Duration = Duration::from_millis(20);
/// The same for one server hook call (a server tick is 50 ms).
pub const SERVER_HOOK_BUDGET: Duration = Duration::from_millis(25);
/// All the calls of one mod in one [`AGGREGATE_WINDOW`] may use this much guest time (client).
pub const CLIENT_AGGREGATE_BUDGET: Duration = Duration::from_millis(10);
/// The same for a server module.
pub const SERVER_AGGREGATE_BUDGET: Duration = Duration::from_millis(25);
/// The span the aggregate budgets are counted over (one server tick).
pub const AGGREGATE_WINDOW: Duration = Duration::from_millis(50);
/// A mod that overruns its aggregate budget in this many windows in a row is disabled.
pub const AGGREGATE_STRIKES: u32 = 20;
pub const FUEL_PER_CALL: u64 = 10_000_000;
const INTERFACE_VERSION: &str = "1.0.0";

/// The per-call and aggregate time limits of one module.
#[derive(Clone, Copy, Debug)]
pub struct TimeLimits {
    pub call: Duration,
    pub aggregate: Duration,
}

impl TimeLimits {
    pub const CLIENT: Self = Self {
        call: CLIENT_HOOK_BUDGET,
        aggregate: CLIENT_AGGREGATE_BUDGET,
    };
    pub const SERVER: Self = Self {
        call: SERVER_HOOK_BUDGET,
        aggregate: SERVER_AGGREGATE_BUDGET,
    };
    /// A module's `init` (and its start function): it runs once, while the game loads.
    pub const INIT: Self = Self {
        call: Duration::from_millis(250),
        aggregate: Duration::from_secs(1),
    };
}

/// The store limits every instance gets (also what the tests check).
pub fn store_limits() -> StoreLimits {
    StoreLimitsBuilder::new()
        .memory_size(MEMORY_LIMIT_BYTES)
        .memories(1)
        .table_elements(TABLE_ELEMENT_LIMIT)
        .tables(TABLE_LIMIT)
        .instances(16)
        .build()
}

#[derive(Default)]
pub struct Sandbox {
    runtime: Option<Arc<RuntimeEngine>>,
    io: Arc<IoRegistry>,
    texture_counts: HashMap<String, Weak<AtomicUsize>>,
}

impl Sandbox {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn engine_started(&self) -> bool {
        self.runtime.is_some()
    }

    /// Gives the mod `id` its files (`/data`, `/game`, maybe `/host/<letter>`) and, when `io.worker` is set, its worker.
    /// Call it before the mod is compiled and instantiated; it applies to every later instance of that id.
    pub fn set_mod_io(&mut self, id: &str, io: ModIo) {
        self.io.mods.lock().unwrap_or_else(|error| error.into_inner()).insert(id.to_owned(), io);
    }

    pub fn clear_mod_io(&mut self, id: &str) {
        self.io.mods.lock().unwrap_or_else(|error| error.into_inner()).remove(id);
    }

    /// Lets the *main* instances use sockets and `wasi:http` (default: they may not; a connect or request can wait
    /// for seconds and would stop the game thread). Only for tools and tests.
    pub fn set_main_network(&self, allow: bool) {
        self.io.main_network.store(allow, Ordering::Relaxed);
    }

    fn texture_count(&mut self, id: &str) -> Arc<AtomicUsize> {
        self.texture_counts
            .retain(|_, count| count.strong_count() > 0);
        if let Some(count) = self.texture_counts.get(id).and_then(Weak::upgrade) {
            return count;
        }
        let count = Arc::new(AtomicUsize::new(0));
        self.texture_counts
            .insert(id.to_owned(), Arc::downgrade(&count));
        count
    }

    pub fn compile(&mut self, bytes: &[u8]) -> Result<CompiledComponent, String> {
        self.compile_as(bytes, false, false)
    }

    pub(crate) fn compile_injection(&mut self, bytes: &[u8]) -> Result<CompiledComponent, String> {
        self.compile_as(bytes, false, true)
    }

    /// Compiles a *test module* (world `test-mod`): the only compile that accepts the `test-driver`
    /// import. Only [`TestMod`] calls this; a mod's components go through [`Sandbox::compile`].
    pub(crate) fn compile_test(&mut self, bytes: &[u8]) -> Result<CompiledComponent, String> {
        self.compile_as(bytes, true, false)
    }

    fn compile_as(
        &mut self,
        bytes: &[u8],
        test_module: bool,
        injection: bool,
    ) -> Result<CompiledComponent, String> {
        if self.runtime.is_none() {
            self.runtime = Some(Arc::new(RuntimeEngine::new()?));
        }
        let runtime = Arc::clone(self.runtime.as_ref().expect("runtime was just initialized"));
        let component = Component::new(&runtime.engine, bytes)
            .map_err(|error| format!("not a valid component: {error:#}"))?;
        let component_type = component.component_type();
        let supports_config = injection
            && component_type
                .exports(&runtime.engine)
                .any(|(name, _)| name == "yoake:mods/inject@1.1.0")
            && component_type
                .exports(&runtime.engine)
                .any(|(name, _)| name == "yoake:mods/config-events@1.2.0");
        for (name, _) in component_type.imports(&runtime.engine) {
            check_import(name, test_module, supports_config)?;
        }
        Ok(CompiledComponent { runtime, component, io: Arc::clone(&self.io) })
    }
}

/// The interface only a test module may import (`wit/yoake-test.wit`).
const TEST_INTERFACE: &str = "test-driver";

fn check_import(name: &str, test_module: bool, supports_config: bool) -> Result<(), String> {
    if name == yoake_mod_io::WORKER_INTERFACE {
        return Ok(());
    }
    if let Some(wasi) = name.strip_prefix("wasi:") {
        return if wasi::allowed_import(wasi) {
            Ok(())
        } else {
            Err(format!("forbidden component import: {name}"))
        };
    }
    let Some(import) = name.strip_prefix("yoake:mods/") else {
        return Err(format!("forbidden component import: {name}"));
    };
    let Some((interface, version)) = import.rsplit_once('@') else {
        return Err(format!("missing component interface version: {name}"));
    };
    if interface.is_empty() || interface.contains('/') || interface.contains('@') {
        return Err(format!("invalid component import: {name}"));
    }
    if interface == TEST_INTERFACE && !test_module {
        return Err(format!(
            "forbidden component import: {name} (only the engine's test runner links it)"
        ));
    }
    if version != INTERFACE_VERSION
        && !(supports_config && interface == "config" && version == "1.2.0")
    {
        return Err(format!("unsupported component interface version: {name}"));
    }
    Ok(())
}

struct RuntimeEngine {
    engine: Engine,
    stop: Arc<AtomicBool>,
    timer: Option<JoinHandle<()>>,
}

impl RuntimeEngine {
    fn new() -> Result<Self, String> {
        let mut config = Config::new();
        config
            .wasm_component_model(true)
            .consume_fuel(true)
            .epoch_interruption(true)
            // Threads are compiled out of this build (the `threads` feature is off); named so a
            // future feature flip does not silently allow guest threads or shared memories.
            .wasm_shared_everything_threads(false);
        let engine = Engine::new(&config).map_err(|error| error.to_string())?;
        let stop = Arc::new(AtomicBool::new(false));
        let timer_stop = Arc::clone(&stop);
        let timer_engine = engine.clone();
        let timer = thread::Builder::new()
            .name("yoake-wasm-epoch".into())
            .spawn(move || {
                // The epoch only says "time passed": whether a call is over its limit is decided by
                // the clock in its deadline callback, so a late or bunched tick (the Windows
                // scheduler rounds a sleep up to 15.6 ms unless a high-resolution timer exists)
                // delays the stop but never stops a call early. The epoch is counted from elapsed
                // time so a late wake-up catches up.
                let started = Instant::now();
                let mut counted = 0u64;
                while !timer_stop.load(Ordering::Relaxed) {
                    thread::sleep(EPOCH_INTERVAL);
                    let elapsed = started.elapsed().as_millis() as u64;
                    while counted < elapsed {
                        timer_engine.increment_epoch();
                        counted += 1;
                    }
                }
            })
            .map_err(|error| format!("could not start WebAssembly epoch timer: {error}"))?;
        Ok(Self {
            engine,
            stop,
            timer: Some(timer),
        })
    }
}

impl Drop for RuntimeEngine {
    fn drop(&mut self) {
        self.stop.store(true, Ordering::Relaxed);
        if let Some(timer) = self.timer.take() {
            let _ = timer.join();
        }
    }
}

pub struct HostState<T> {
    pub host: T,
    limits: StoreLimits,
    wasi: WasiCtx,
    table: ResourceTable,
    http: WasiHttpCtx,
    http_hooks: wasi::MainHttpHooks,
    main_network: bool,
    /// The mod's worker, if it declares one (stopped when this instance is dropped).
    worker: Option<WorkerHandle>,
    output: wasi::Output,
    /// When the running call must be over (the epoch callback stops the guest after it).
    deadline: Instant,
}

/// Per-mod files and workers (FR-F2), keyed by mod id; filled by the loader before a mod is compiled.
#[derive(Default)]
struct IoRegistry {
    mods: Mutex<HashMap<String, ModIo>>,
    /// Whether the main instance may use sockets and `wasi:http` (default: no; use the worker).
    main_network: AtomicBool,
}

impl<T: Send> WasiHttpView for HostState<T> {
    fn http(&mut self) -> WasiHttpCtxView<'_> {
        let hooks: &mut dyn wasmtime_wasi_http::WasiHttpHooks = if self.main_network {
            wasmtime_wasi_http::default_hooks()
        } else {
            &mut self.http_hooks
        };
        WasiHttpCtxView { ctx: &mut self.http, table: &mut self.table, hooks }
    }
}

/// `yoake:mods/worker@1.6.0`: the main instance's two calls to its worker. Neither blocks.
fn link_worker_interface<T: Send + 'static>(linker: &mut Linker<HostState<T>>) -> wasmtime::Result<()> {
    use wasmtime::StoreContextMut;
    let mut instance = linker.instance(yoake_mod_io::WORKER_INTERFACE)?;
    instance.func_wrap("worker-send", |cx: StoreContextMut<'_, HostState<T>>, (message,): (Vec<u8>,)| {
        let result = match &cx.data().worker {
            Some(worker) => worker.send(message).map_err(|error| error.to_string()),
            None => Err("no-worker".to_owned()),
        };
        Ok((result,))
    })?;
    instance.func_wrap("worker-recv", |cx: StoreContextMut<'_, HostState<T>>, (): ()| {
        Ok((cx.data().worker.as_ref().and_then(WorkerHandle::recv),))
    })?;
    Ok(())
}

impl<T: Send> WasiView for HostState<T> {
    fn ctx(&mut self) -> WasiCtxView<'_> {
        WasiCtxView {
            ctx: &mut self.wasi,
            table: &mut self.table,
        }
    }
}

#[derive(Clone)]
pub struct CompiledComponent {
    runtime: Arc<RuntimeEngine>,
    component: Component,
    io: Arc<IoRegistry>,
}

impl CompiledComponent {
    pub fn instantiate<T: 'static + Send>(
        self,
        id: impl Into<String>,
        host: T,
        configure: impl FnOnce(&mut Linker<HostState<T>>) -> Result<(), String>,
    ) -> Result<ModInstance<T>, String> {
        let mut linker = Linker::new(&self.runtime.engine);
        wasi::add_to_linker(&mut linker).map_err(|error| error.to_string())?;
        link_worker_interface(&mut linker).map_err(|error| error.to_string())?;
        configure(&mut linker)?;
        let limits = store_limits();
        let output = wasi::Output::default();
        let id = id.into();
        let main_network = self.io.main_network.load(Ordering::Relaxed);
        let mod_io = self.io.mods.lock().unwrap_or_else(|error| error.into_inner()).get(&id).cloned();
        let mut builder = WasiCtxBuilder::new();
        builder
            .stdout(output.stdout())
            .stderr(output.stderr())
            .allow_tcp(main_network)
            .allow_udp(main_network)
            .allow_ip_name_lookup(main_network);
        if main_network {
            builder.inherit_network();
        }
        let mut worker = None;
        if let Some(mod_io) = &mod_io {
            mod_io.files.apply(&mut builder).map_err(|error| format!("mod files: {error}"))?;
            if let Some(bytes) = &mod_io.worker {
                let options = WorkerOptions { log: mod_io.log.clone(), ..WorkerOptions::default() };
                worker = Some(
                    WorkerHandle::spawn(&id, bytes, mod_io.files.clone(), options)
                        .map_err(|error| format!("worker: {error}"))?,
                );
            }
        }
        let wasi = builder.build();
        let deadline = Instant::now() + TimeLimits::INIT.call;
        let state = HostState {
            host,
            limits,
            wasi,
            table: ResourceTable::new(),
            http: WasiHttpCtx::new(),
            http_hooks: wasi::MainHttpHooks,
            main_network,
            worker,
            output,
            deadline,
        };
        let mut store = Store::new(&self.runtime.engine, state);
        store.limiter(|state| &mut state.limits);
        store
            .set_fuel(FUEL_PER_CALL)
            .map_err(|error| error.to_string())?;
        store.epoch_deadline_callback(|context| {
            if Instant::now() >= context.data().deadline + deadline_extension() {
                Ok(UpdateDeadline::Interrupt)
            } else {
                Ok(UpdateDeadline::Continue(1))
            }
        });
        store.set_epoch_deadline(1);
        let instance = linker
            .instantiate(&mut store, &self.component)
            .map_err(|error| format!("component instantiation failed: {error:#}"))?;
        Ok(ModInstance {
            id: id,
            store,
            instance,
            disabled: false,
            limits: TimeLimits::CLIENT,
            fuel: FUEL_PER_CALL,
            window: Window {
                started: Instant::now(),
                spent: Duration::ZERO,
                exceeded: false,
                strikes: 0,
            },
            skipped: 0,
            aggregate_exempt: false,
            _runtime: self.runtime,
        })
    }
}

pub struct ModInstance<T: 'static> {
    id: String,
    store: Store<HostState<T>>,
    instance: Instance,
    disabled: bool,
    limits: TimeLimits,
    fuel: u64,
    window: Window,
    skipped: u64,
    /// The calls being made do not count against the aggregate budget (still against the per-call one):
    /// the engine's chunk generator calls, which a module cannot choose to make fewer of (F3-8).
    aggregate_exempt: bool,
    _runtime: Arc<RuntimeEngine>,
}

/// The guest time a module used in the current aggregate window.
struct Window {
    started: Instant,
    spent: Duration,
    exceeded: bool,
    strikes: u32,
}

impl<T: 'static> ModInstance<T> {
    pub fn is_disabled(&self) -> bool {
        self.disabled
    }

    /// Runs the module's `init` under the generous load-time limits, then switches to `after`
    /// (the limits of its hooks): [`TimeLimits::CLIENT`] or [`TimeLimits::SERVER`].
    pub fn call_init<R>(
        &mut self,
        after: TimeLimits,
        invoke: impl FnOnce(&mut Store<HostState<T>>, &Instance) -> Result<R, String>,
    ) -> Option<R> {
        self.limits = TimeLimits::INIT;
        let result = self.call("init", invoke);
        self.limits = after;
        self.window.spent = Duration::ZERO;
        self.window.started = Instant::now();
        self.window.exceeded = false;
        self.window.strikes = 0;
        result
    }

    /// Sets the fuel one call may burn (default [`FUEL_PER_CALL`]); for tests of the clock limits,
    /// which fuel would otherwise reach first.
    #[doc(hidden)]
    pub fn set_fuel_per_call(&mut self, fuel: u64) {
        self.fuel = fuel;
    }

    /// Sets the per-call and aggregate time limits (a server module gets the server's).
    pub fn set_limits(&mut self, limits: TimeLimits) {
        self.limits = limits;
    }

    /// Calls made while this is on are neither skipped for the aggregate budget nor counted in it.
    pub fn set_aggregate_exempt(&mut self, exempt: bool) {
        self.aggregate_exempt = exempt;
    }

    /// How many calls were skipped because the module had used its aggregate budget.
    pub fn skipped_calls(&self) -> u64 {
        self.skipped
    }

    /// Whether the aggregate budget of the current window is used up (rolls the window first).
    fn over_aggregate(&mut self, now: Instant) -> bool {
        if now.duration_since(self.window.started) >= AGGREGATE_WINDOW {
            if self.window.exceeded || self.window.spent >= self.limits.aggregate {
                self.window.strikes += 1;
            } else {
                self.window.strikes = 0;
            }
            self.window.started = now;
            self.window.spent = Duration::ZERO;
            self.window.exceeded = false;
        }
        self.window.exceeded || self.window.spent >= self.limits.aggregate
    }

    /// Complete lines the guest wrote to its WASI stdout and stderr since the last drain.
    /// A partial trailing line stays buffered until it is finished (or grows too long).
    pub fn drain_output(&mut self) -> Vec<OutputLine> {
        self.store.data().output.drain()
    }

    pub fn call<R>(
        &mut self,
        hook: &str,
        invoke: impl FnOnce(&mut Store<HostState<T>>, &Instance) -> Result<R, String>,
    ) -> Option<R> {
        self.call_bounded(hook, true, invoke).ok().flatten()
    }

    pub fn call_recoverable<R>(
        &mut self,
        hook: &str,
        invoke: impl FnOnce(&mut Store<HostState<T>>, &Instance) -> Result<R, String>,
    ) -> Result<Option<R>, String> {
        self.call_bounded(hook, false, invoke)
    }

    fn call_bounded<R>(
        &mut self,
        hook: &str,
        disable_on_trap: bool,
        invoke: impl FnOnce(&mut Store<HostState<T>>, &Instance) -> Result<R, String>,
    ) -> Result<Option<R>, String> {
        if self.disabled {
            return Ok(None);
        }
        let began = Instant::now();
        let over_aggregate = !self.aggregate_exempt && self.over_aggregate(began);
        if self.window.strikes >= AGGREGATE_STRIKES && !self.aggregate_exempt {
            self.disabled = true;
            let id = self.id.replace(['\r', '\n'], " ");
            eprintln!(
                "WebAssembly mod {id} disabled: over its time budget in {AGGREGATE_STRIKES} windows in a row"
            );
            return Ok(None);
        }
        if over_aggregate {
            self.window.exceeded = true;
            self.skipped += 1;
            if self.skipped == 1 {
                let id = self.id.replace(['\r', '\n'], " ");
                eprintln!(
                    "WebAssembly mod {id}: calls skipped, it used its time budget for this window (further skips are only counted)"
                );
            }
            return Ok(None);
        }
        self.store.data_mut().deadline = began + self.limits.call;
        // Calls nest (a `mod-call` runs another module inside a service): each has its own host time.
        let outer_host_time = HOST_TIME.with(|time| time.replace(Duration::ZERO));
        let outer_storage_time = STORAGE_TIME.with(|time| time.replace(Duration::ZERO));
        let outer_storage_throttled = STORAGE_THROTTLED.with(|throttled| throttled.replace(false));
        let outer_storage_window =
            STORAGE_WINDOW.with(|window| window.replace(Some(self.window.started)));
        let outer_counted_host_time = COUNTED_HOST_TIME.with(|time| time.replace(Duration::ZERO));
        let result = self
            .store
            .set_fuel(self.fuel)
            .map_err(|error| error.to_string())
            .and_then(|()| {
                self.store.set_epoch_deadline(1);
                invoke(&mut self.store, &self.instance)
            });
        let host_time = HOST_TIME.with(|time| time.replace(outer_host_time));
        STORAGE_TIME.with(|time| time.set(outer_storage_time));
        STORAGE_WINDOW.with(|window| window.set(outer_storage_window));
        self.window.exceeded |=
            STORAGE_THROTTLED.with(|throttled| throttled.replace(outer_storage_throttled));
        COUNTED_HOST_TIME.with(|time| time.set(outer_counted_host_time));
        if !self.aggregate_exempt {
            self.window.spent += began.elapsed().saturating_sub(host_time);
        }
        match result {
            Ok(value) => Ok(Some(value)),
            Err(error) => {
                if !disable_on_trap {
                    return Err(error);
                }
                self.disabled = true;
                let id = self.id.replace(['\r', '\n'], " ");
                let hook = hook.replace(['\r', '\n'], " ");
                let reason = error.replace(['\r', '\n'], " ");
                eprintln!("WebAssembly mod {id} disabled in {hook}: {reason}");
                Err(error)
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn tick_component() -> Vec<u8> {
        let mut resolve = wit_parser::Resolve::default();
        let package = resolve
            .push_str(
                "test.wit",
                "package clock:test; world client-mod { export tick: func(); }",
            )
            .unwrap();
        let world = resolve
            .select_world(&[package], Some("client-mod"))
            .unwrap();
        let mut module = b"\0asm\x01\0\0\0\x01\x04\x01\x60\0\0\x03\x02\x01\0\x07\x10\x01\x0ccm32p2||tick\0\0\x0a\x04\x01\x02\0\x0b".to_vec();
        wit_component::embed_component_metadata(
            &mut module,
            &resolve,
            world,
            wit_component::StringEncoding::UTF8,
        )
        .unwrap();
        wit_component::ComponentEncoder::default()
            .module(&module)
            .unwrap()
            .encode()
            .unwrap()
    }

    /// F3-8: calls made exempt from the aggregate budget (the chunk generator's) are neither skipped for it nor
    /// counted in it; the same calls not exempt are skipped once the window's budget is used.
    #[test]
    fn exempt_calls_are_not_counted_against_the_aggregate_budget() {
        let mut runtime = Sandbox::new();
        let mut module = runtime
            .compile(&tick_component())
            .unwrap()
            .instantiate("exempt-clock", (), |_| Ok(()))
            .unwrap();
        module.set_limits(TimeLimits { call: Duration::from_millis(500), aggregate: Duration::from_millis(5) });
        let slow = |module: &mut ModInstance<()>| {
            module.call("tick", |_, _| {
                std::thread::sleep(Duration::from_millis(20));
                Ok(())
            })
        };
        module.set_aggregate_exempt(true);
        for _ in 0..4 {
            assert_eq!(slow(&mut module), Some(()), "exempt calls always run");
        }
        assert_eq!(module.skipped_calls(), 0);
        module.set_aggregate_exempt(false);
        assert_eq!(slow(&mut module), Some(()), "the first counted call runs");
        assert_eq!(slow(&mut module), None, "and uses the window's budget, so the next is skipped");
        assert_eq!(module.skipped_calls(), 1);
    }

    #[test]
    fn storage_time_extends_both_hook_deadlines_but_remains_aggregate_charged() {
        let mut runtime = Sandbox::new();
        for limits in [TimeLimits::CLIENT, TimeLimits::SERVER] {
            let mut module = runtime
                .compile(&tick_component())
                .unwrap()
                .instantiate("storage-clock", (), |_| Ok(()))
                .unwrap();
            module.set_limits(limits);
            let answer = module.call("tick", |store, instance| {
                let began = Instant::now();
                std::thread::sleep(Duration::from_millis(80));
                add_storage_time(began.elapsed());
                let tick = instance
                    .get_typed_func::<(), ()>(&mut *store, "tick")
                    .map_err(|error| error.to_string())?;
                tick.call(&mut *store, ())
                    .map_err(|error| error.to_string())
            });
            assert_eq!(answer, Some(()));
            assert!(!module.disabled);
            assert!(module.window.spent >= Duration::from_millis(80));
        }
    }

    #[test]
    fn slow_calls_accumulate_strikes_without_skipping_a_call() {
        let mut module = Sandbox::new()
            .compile(&tick_component())
            .unwrap()
            .instantiate("slow-storage", (), |_| Ok(()))
            .unwrap();
        for strike in 1..=AGGREGATE_STRIKES {
            module.window.started = Instant::now() - AGGREGATE_WINDOW;
            module.window.spent = module.limits.aggregate + Duration::from_millis(400);
            let answer = module.call("tick", |_, _| Ok(()));
            assert_eq!(module.window.strikes, strike);
            assert_eq!(answer, (strike < AGGREGATE_STRIKES).then_some(()));
        }
        assert!(module.disabled);
        assert_eq!(module.skipped, 0);
    }

    #[test]
    fn storage_and_other_host_time_share_one_bounded_deadline_extension() {
        let previous_host = HOST_TIME.with(|time| time.replace(Duration::from_millis(250)));
        let previous_storage = STORAGE_TIME.with(|time| time.replace(Duration::from_millis(300)));
        assert_eq!(deadline_extension(), Duration::from_millis(400));
        HOST_TIME.with(|time| time.set(previous_host));
        STORAGE_TIME.with(|time| time.set(previous_storage));
    }
}

#[cfg(test)]
mod budget_tests {
    use super::*;

    #[test]
    fn shader_host_time_stays_in_the_aggregate_budget() {
        let mut resolve = wit_parser::Resolve::default();
        let package = resolve
            .push_str("test.wit", "package test:budget; world client-mod {}")
            .unwrap();
        let world = resolve
            .select_world(&[package], Some("client-mod"))
            .unwrap();
        let mut module =
            wit_component::dummy_module(&resolve, world, wit_parser::ManglingAndAbi::Standard32);
        wit_component::embed_component_metadata(
            &mut module,
            &resolve,
            world,
            wit_component::StringEncoding::UTF8,
        )
        .unwrap();
        let bytes = wit_component::ComponentEncoder::default()
            .module(&module)
            .unwrap()
            .encode()
            .unwrap();
        let mut instance = Sandbox::new()
            .compile(&bytes)
            .unwrap()
            .instantiate("budget", (), |_| Ok(()))
            .unwrap();
        let mut measured = Duration::ZERO;
        assert_eq!(
            instance.call("compile", |_, _| {
                let began = Instant::now();
                std::thread::sleep(Duration::from_millis(2));
                measured = began.elapsed();
                add_counted_host_time(measured);
                assert!(deadline_extension() >= measured);
                Ok(())
            }),
            Some(())
        );
        assert!(instance.window.spent >= measured);
        instance.window.started = Instant::now();
        instance.limits.aggregate = Duration::from_nanos(1);
        assert_eq!(instance.call("tick", |_, _| Ok(())), None);
        assert_eq!(instance.skipped, 1);
        assert!(!instance.is_disabled());
    }
}
