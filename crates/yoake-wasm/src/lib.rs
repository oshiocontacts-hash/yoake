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
mod media_services;
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
pub use media_services::{BROWSER_PERMISSION, MediaServices, media_types};
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
    /// Host services extend both clocks. Ordinary host service time is also excluded from the aggregate;
    /// storage and counted host work remain aggregate-charged.
    static HOST_TIME: std::cell::Cell<Duration> = const { std::cell::Cell::new(Duration::ZERO) };
    static HOST_CPU_TIME: std::cell::Cell<Duration> = const { std::cell::Cell::new(Duration::ZERO) };
    static STORAGE_TIME: std::cell::Cell<Duration> = const { std::cell::Cell::new(Duration::ZERO) };
    static STORAGE_CPU_TIME: std::cell::Cell<Duration> = const { std::cell::Cell::new(Duration::ZERO) };
    static STORAGE_THROTTLED: std::cell::Cell<bool> = const { std::cell::Cell::new(false) };
    static STORAGE_WINDOW: std::cell::Cell<Option<Instant>> = const { std::cell::Cell::new(None) };
    static COUNTED_HOST_TIME: std::cell::Cell<Duration> = const { std::cell::Cell::new(Duration::ZERO) };
    static COUNTED_HOST_CPU_TIME: std::cell::Cell<Duration> = const { std::cell::Cell::new(Duration::ZERO) };
}

#[cfg(unix)]
pub(crate) fn thread_cpu_time() -> Option<Duration> {
    // SAFETY: `time` is a valid output pointer and this clock needs no other setup.
    unsafe {
        let mut time: libc::timespec = std::mem::zeroed();
        if libc::clock_gettime(libc::CLOCK_THREAD_CPUTIME_ID, &mut time) != 0 {
            return None;
        }
        Some(Duration::new(time.tv_sec as u64, time.tv_nsec as u32))
    }
}

#[cfg(windows)]
pub(crate) fn thread_cpu_time() -> Option<Duration> {
    use windows_sys::Win32::Foundation::FILETIME;
    use windows_sys::Win32::System::Threading::{GetCurrentThread, GetThreadTimes};
    fn ticks(time: FILETIME) -> u64 {
        (u64::from(time.dwHighDateTime) << 32) | u64::from(time.dwLowDateTime)
    }
    // SAFETY: all outputs are valid and the current-thread pseudo handle needs no closing.
    unsafe {
        let (mut created, mut exited, mut kernel, mut user) = (
            std::mem::zeroed(),
            std::mem::zeroed(),
            std::mem::zeroed(),
            std::mem::zeroed(),
        );
        if GetThreadTimes(GetCurrentThread(), &mut created, &mut exited, &mut kernel, &mut user) == 0 {
            return None;
        }
        Some(Duration::from_nanos((ticks(kernel) + ticks(user)) * 100))
    }
}

#[cfg(not(any(unix, windows)))]
pub(crate) fn thread_cpu_time() -> Option<Duration> {
    None
}

pub(crate) fn cpu_elapsed(started: Option<Duration>) -> Duration {
    started
        .and_then(|started| thread_cpu_time().map(|now| now.saturating_sub(started)))
        .unwrap_or(Duration::ZERO)
}

/// The most host-service time that extends one call's deadline (a guest that loops over services cannot hold the
/// server thread longer than this plus its own budget and the wall-clock safety cap).
pub const MAX_HOST_TIME: Duration = Duration::from_millis(400);

pub(crate) fn add_host_time(spent: Duration, cpu: Duration) {
    HOST_TIME.with(|time| time.set(time.get() + spent));
    HOST_CPU_TIME.with(|time| time.set(time.get() + cpu));
}

pub(crate) fn add_counted_host_time(spent: Duration, cpu: Duration) {
    COUNTED_HOST_TIME.with(|time| time.set(time.get() + spent));
    COUNTED_HOST_CPU_TIME.with(|time| time.set(time.get() + cpu));
}

pub(crate) fn add_storage_time(spent: Duration, cpu: Duration) {
    STORAGE_TIME.with(|time| time.set(time.get() + spent));
    STORAGE_CPU_TIME.with(|time| time.set(time.get() + cpu));
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

/// How coarse the thread CPU clock is. Windows charges thread time in scheduler ticks (`GetThreadTimes`
/// advances by about 15.6 ms), so a reading can be one tick ahead of the real work: the CPU limit allows
/// one tick more there, or a healthy 10 ms call could read as 15.6 + 15.6 and be stopped.
#[cfg(windows)]
const CPU_CLOCK_GRANULARITY: Duration = Duration::from_micros(15_625);
#[cfg(not(windows))]
const CPU_CLOCK_GRANULARITY: Duration = Duration::ZERO;

fn cpu_deadline_extension() -> Duration {
    (HOST_CPU_TIME.with(std::cell::Cell::get)
        + STORAGE_CPU_TIME.with(std::cell::Cell::get)
        + COUNTED_HOST_CPU_TIME.with(std::cell::Cell::get))
    .min(MAX_HOST_TIME)
}

#[derive(Clone, Copy)]
struct CallClock {
    wall: fn(Instant) -> Duration,
    cpu: fn() -> Option<Duration>,
}

impl Default for CallClock {
    fn default() -> Self {
        Self { wall: |started| started.elapsed(), cpu: thread_cpu_time }
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum LimitHit {
    Cpu,
    WallFallback,
    WallCap,
}

fn wall_cap(limit: Duration) -> Duration {
    limit.saturating_mul(20).max(Duration::from_secs(1))
}

fn limit_reason(kind: LimitHit, cpu: Option<Duration>, wall: Duration, limit: Duration) -> String {
    let cpu = cpu.map_or_else(|| "unavailable".to_owned(), |cpu| format!("{:.1}", cpu.as_secs_f64() * 1000.0));
    match kind {
        LimitHit::Cpu => format!(
            "over its per-call CPU limit (cpu {cpu} ms, wall {:.1} ms, limit {} ms)",
            wall.as_secs_f64() * 1000.0,
            limit.as_millis()
        ),
        LimitHit::WallCap => format!(
            "over its wall-clock safety cap (cpu {cpu} ms, wall {:.1} ms, cap {} ms)",
            wall.as_secs_f64() * 1000.0,
            wall_cap(limit).as_millis()
        ),
        LimitHit::WallFallback => format!(
            "over its per-call wall-clock limit (CPU clock unavailable; wall {:.1} ms, limit {} ms)",
            wall.as_secs_f64() * 1000.0,
            limit.as_millis()
        ),
    }
}

/// One client hook call may use this much calling-thread CPU time (plus one epoch tick). A separate
/// wall-clock safety cap, at least one second and twenty times the CPU limit, bounds scheduler starvation
/// and sleeping host services. [`CLIENT_AGGREGATE_BUDGET`] limits the frame's total guest CPU time.
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
    if name == yoake_mod_io::WORKER_INTERFACE || name == yoake_mod_io::WORKER_FULL_INTERFACE {
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
    call_started: Instant,
    cpu_started: Option<Duration>,
    call_limit: Duration,
    wall_cap: Duration,
    limit_hit: Option<LimitHit>,
    clock: CallClock,
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

/// The `worker` interface of the full worlds' bindings (`with` in `full_bindings`): linked by hand in
/// [`link_worker_interface`], so bindgen generates no host trait or linker entry for it.
pub mod worker_link {
    use wasmtime::component::{HasData, Linker};

    /// The names the bindgen code of the full worlds refers to for a mapped interface.
    pub trait Host {}
    impl<T: ?Sized> Host for T {}
    pub trait HostWithStore<T: ?Sized>: HasData {}
    impl<S: ?Sized + HasData, T: ?Sized> HostWithStore<T> for S {}

    /// Links nothing: [`super::link_worker_interface`] defines the functions.
    pub fn add_to_linker<T, D: HasData>(_linker: &mut Linker<T>, _get: impl Fn(&mut T) -> D::Data<'_>) -> wasmtime::Result<()> {
        Ok(())
    }
}

/// `yoake:mods/worker@1.6.0` (and the full worlds' `worker@1.0.0`): the main instance's two calls to its worker.
/// Neither blocks.
fn link_worker_interface<T: Send + 'static>(linker: &mut Linker<HostState<T>>) -> wasmtime::Result<()> {
    use wasmtime::StoreContextMut;
    for name in [yoake_mod_io::WORKER_INTERFACE, yoake_mod_io::WORKER_FULL_INTERFACE] {
        let mut instance = linker.instance(name)?;
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
    }
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
        let call_started = Instant::now();
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
            call_started,
            cpu_started: thread_cpu_time(),
            call_limit: TimeLimits::INIT.call,
            wall_cap: wall_cap(TimeLimits::INIT.call),
            limit_hit: None,
            clock: CallClock::default(),
        };
        let mut store = Store::new(&self.runtime.engine, state);
        store.limiter(|state| &mut state.limits);
        store
            .set_fuel(FUEL_PER_CALL)
            .map_err(|error| error.to_string())?;
        store.epoch_deadline_callback(|mut context| {
            let data = context.data();
            let wall = (data.clock.wall)(data.call_started);
            let host_wall = deadline_extension();
            let hit = if wall >= data.wall_cap + host_wall {
                Some(LimitHit::WallCap)
            } else {
                let cpu = data.cpu_started.and_then(|started| {
                    (data.clock.cpu)().map(|now| {
                        now.saturating_sub(started)
                            .saturating_sub(cpu_deadline_extension())
                    })
                });
                match cpu {
                    Some(cpu) if cpu >= data.call_limit + CPU_CLOCK_GRANULARITY => Some(LimitHit::Cpu),
                    None if wall >= data.call_limit + host_wall => Some(LimitHit::WallFallback),
                    _ => None,
                }
            };
            if let Some(hit) = hit {
                context.data_mut().limit_hit = Some(hit);
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
        let clock = self.store.data().clock;
        let state = self.store.data_mut();
        state.call_started = began;
        state.cpu_started = (clock.cpu)();
        state.call_limit = self.limits.call;
        state.wall_cap = wall_cap(self.limits.call);
        state.limit_hit = None;
        let cpu_began = state.cpu_started;
        // Calls nest (a `mod-call` runs another module inside a service): each has its own host time.
        let outer_host_time = HOST_TIME.with(|time| time.replace(Duration::ZERO));
        let outer_host_cpu_time = HOST_CPU_TIME.with(|time| time.replace(Duration::ZERO));
        let outer_storage_time = STORAGE_TIME.with(|time| time.replace(Duration::ZERO));
        let outer_storage_cpu_time = STORAGE_CPU_TIME.with(|time| time.replace(Duration::ZERO));
        let outer_storage_throttled = STORAGE_THROTTLED.with(|throttled| throttled.replace(false));
        let outer_storage_window =
            STORAGE_WINDOW.with(|window| window.replace(Some(self.window.started)));
        let outer_counted_host_time = COUNTED_HOST_TIME.with(|time| time.replace(Duration::ZERO));
        let outer_counted_host_cpu_time = COUNTED_HOST_CPU_TIME.with(|time| time.replace(Duration::ZERO));
        let result = self
            .store
            .set_fuel(self.fuel)
            .map_err(|error| error.to_string())
            .and_then(|()| {
                self.store.set_epoch_deadline(1);
                invoke(&mut self.store, &self.instance)
            });
        let host_time = HOST_TIME.with(|time| time.replace(outer_host_time));
        let host_cpu_time = HOST_CPU_TIME.with(|time| time.replace(outer_host_cpu_time));
        STORAGE_TIME.with(|time| time.set(outer_storage_time));
        let storage_cpu_time = STORAGE_CPU_TIME.with(|time| time.replace(outer_storage_cpu_time));
        STORAGE_WINDOW.with(|window| window.set(outer_storage_window));
        self.window.exceeded |=
            STORAGE_THROTTLED.with(|throttled| throttled.replace(outer_storage_throttled));
        COUNTED_HOST_TIME.with(|time| time.set(outer_counted_host_time));
        let counted_host_cpu_time =
            COUNTED_HOST_CPU_TIME.with(|time| time.replace(outer_counted_host_cpu_time));
        let wall = (clock.wall)(began);
        let cpu = cpu_began.and_then(|started| {
            (clock.cpu)().map(|now| {
                now.saturating_sub(started)
                    .saturating_sub(host_cpu_time + storage_cpu_time + counted_host_cpu_time)
            })
        });
        if !self.aggregate_exempt {
            self.window.spent += cpu.map_or_else(
                || wall.saturating_sub(host_time),
                |cpu| cpu + storage_cpu_time + counted_host_cpu_time,
            );
        }
        let limit_hit = self.store.data().limit_hit;
        match result {
            Ok(value) => Ok(Some(value)),
            Err(error) => {
                if !disable_on_trap {
                    return Err(error);
                }
                self.disabled = true;
                let id = self.id.replace(['\r', '\n'], " ");
                let hook = hook.replace(['\r', '\n'], " ");
                let reason = limit_hit.map_or_else(
                    || error.replace(['\r', '\n'], " "),
                    |hit| limit_reason(hit, cpu, wall, self.limits.call),
                );
                eprintln!("WebAssembly mod {id} disabled in {hook}: {reason}");
                Err(error)
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn test_component(spin: bool) -> Vec<u8> {
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
        let mut module = b"\0asm\x01\0\0\0\x01\x04\x01\x60\0\0\x03\x02\x01\0\x07\x10\x01\x0ccm32p2||tick\0\0".to_vec();
        module.extend_from_slice(if spin {
            b"\x0a\x09\x01\x07\0\x03\x40\x0c\0\x0b\x0b"
        } else {
            b"\x0a\x04\x01\x02\0\x0b"
        });
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

    fn tick_component() -> Vec<u8> {
        test_component(false)
    }

    fn call_tick(module: &mut ModInstance<()>) -> Option<()> {
        module.call("tick", |store, instance| {
            let tick = instance
                .get_typed_func::<(), ()>(&mut *store, "tick")
                .map_err(|error| error.to_string())?;
            tick.call(&mut *store, ())
                .map_err(|error| error.to_string())
        })
    }

    fn no_cpu_progress() -> Option<Duration> {
        Some(Duration::ZERO)
    }

    thread_local! {
        static TEST_WALL: std::cell::Cell<Duration> = const { std::cell::Cell::new(Duration::ZERO) };
    }

    fn advancing_wall(_: Instant) -> Duration {
        TEST_WALL.with(|wall| {
            let now = wall.get();
            wall.set(now + Duration::from_millis(250));
            now
        })
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
        module.store.data_mut().clock.cpu = || None;
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
            module.store.data_mut().clock.cpu = || None;
            module.set_limits(limits);
            let answer = module.call("tick", |store, instance| {
                let began = Instant::now();
                std::thread::sleep(Duration::from_millis(80));
                add_storage_time(began.elapsed(), Duration::ZERO);
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
    fn spinning_guest_hits_the_cpu_limit() {
        let mut module = Sandbox::new()
            .compile(&test_component(true))
            .unwrap()
            .instantiate("cpu-limit", (), |_| Ok(()))
            .unwrap();
        module.set_limits(TimeLimits { call: Duration::from_millis(5), aggregate: Duration::from_secs(1) });
        module.set_fuel_per_call(u64::MAX);
        assert_eq!(call_tick(&mut module), None);
        assert!(module.is_disabled());
        assert_eq!(module.store.data().limit_hit, Some(LimitHit::Cpu));
        assert!(module.window.spent >= module.limits.call);
    }

    #[test]
    fn starved_thread_does_not_hit_the_cpu_limit() {
        let mut module = Sandbox::new()
            .compile(&tick_component())
            .unwrap()
            .instantiate("starved", (), |_| Ok(()))
            .unwrap();
        module.store.data_mut().clock.cpu = no_cpu_progress;
        module.set_limits(TimeLimits { call: Duration::from_millis(5), aggregate: Duration::from_millis(10) });
        let answer = module.call("tick", |store, instance| {
            std::thread::sleep(Duration::from_millis(30));
            let tick = instance
                .get_typed_func::<(), ()>(&mut *store, "tick")
                .map_err(|error| error.to_string())?;
            tick.call(&mut *store, ())
                .map_err(|error| error.to_string())
        });
        assert_eq!(answer, Some(()));
        assert!(!module.is_disabled());
        assert_eq!(module.store.data().limit_hit, None);
        assert_eq!(module.window.spent, Duration::ZERO);
    }

    #[test]
    fn wall_cap_interrupts_even_without_cpu_progress() {
        TEST_WALL.with(|wall| wall.set(Duration::ZERO));
        let mut module = Sandbox::new()
            .compile(&test_component(true))
            .unwrap()
            .instantiate("wall-cap", (), |_| Ok(()))
            .unwrap();
        module.store.data_mut().clock = CallClock { wall: advancing_wall, cpu: no_cpu_progress };
        module.set_limits(TimeLimits { call: Duration::from_millis(5), aggregate: Duration::from_secs(1) });
        module.set_fuel_per_call(u64::MAX);
        assert_eq!(call_tick(&mut module), None);
        assert!(module.is_disabled());
        assert_eq!(module.store.data().limit_hit, Some(LimitHit::WallCap));
    }

    #[test]
    fn limit_messages_name_the_limit_and_measurements() {
        assert_eq!(
            limit_reason(
                LimitHit::Cpu,
                Some(Duration::from_micros(31_200)),
                Duration::from_millis(212),
                Duration::from_millis(25),
            ),
            "over its per-call CPU limit (cpu 31.2 ms, wall 212.0 ms, limit 25 ms)"
        );
        assert_eq!(
            limit_reason(
                LimitHit::WallCap,
                Some(Duration::from_millis(2)),
                Duration::from_millis(1100),
                Duration::from_millis(50),
            ),
            "over its wall-clock safety cap (cpu 2.0 ms, wall 1100.0 ms, cap 1000 ms)"
        );
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
        instance.store.data_mut().clock.cpu = || None;
        let mut measured = Duration::ZERO;
        assert_eq!(
            instance.call("compile", |_, _| {
                let began = Instant::now();
                std::thread::sleep(Duration::from_millis(2));
                measured = began.elapsed();
                add_counted_host_time(measured, Duration::ZERO);
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

/// FR-REM05: the worker's two calls are linked under the mod's 1.6.0 name and the full worlds' 1.0.0 name, and both
/// names pass the component import check.
#[cfg(test)]
mod worker_link_tests {
    use super::*;

    #[test]
    fn the_worker_interface_links_under_both_names() {
        let engine = wasmtime::Engine::default();
        let mut linker = wasmtime::component::Linker::<HostState<()>>::new(&engine);
        link_worker_interface(&mut linker).expect("the worker calls link");
        assert!(check_import(yoake_mod_io::WORKER_INTERFACE, false, false).is_ok());
        assert!(check_import(yoake_mod_io::WORKER_FULL_INTERFACE, false, false).is_ok());
        assert_eq!(yoake_mod_io::WORKER_FULL_INTERFACE, "yoake:mods/worker@1.0.0");
    }
}
