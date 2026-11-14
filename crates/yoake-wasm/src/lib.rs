use std::collections::HashMap;
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use std::sync::{Arc, Weak};
use std::thread::{self, JoinHandle};
use std::time::{Duration, Instant};
use wasmtime::component::{Component, Instance, Linker, ResourceTable};
use wasmtime::{Config, Engine, Store, StoreLimits, StoreLimitsBuilder, UpdateDeadline};
use wasmtime_wasi::{WasiCtx, WasiCtxBuilder, WasiCtxView, WasiView};

mod client;
pub mod convert;
mod full;
mod full_hook_names;
pub mod full_bindings;
mod host;
mod hook_names;
mod subscriptions;
#[cfg(feature = "test-guests")]
pub mod test_guests;
mod wasi;

pub mod bindings {
    wasmtime::component::bindgen!({
        path: "wit",
        world: "client-mod",
    });
}

pub use client::{ClientMod, LoadError};
pub use full::{ClientFullMod, SERVER_SERVICES, ServerFullMod, ServerHost, server_types};
pub use host::{ClientContext, ClientHost};
pub use full_hook_names::{FULL_HOOKS as FULL_HOOK_LIST, HookSide};
pub use subscriptions::FullWorld;

pub use subscriptions::Subscriptions;
pub use wasi::{OutputLine, OutputStream, WASI_IMPORT_INTERFACES};

pub const MEMORY_LIMIT_BYTES: usize = 64 * 1024 * 1024;
/// Elements one table may hold (a Rust guest's function table is a few thousand).
pub const TABLE_ELEMENT_LIMIT: usize = 65_536;
/// Tables one instance may have.
pub const TABLE_LIMIT: usize = 8;
const EPOCH_INTERVAL: Duration = Duration::from_millis(1);
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
    pub const CLIENT: Self = Self { call: CLIENT_HOOK_BUDGET, aggregate: CLIENT_AGGREGATE_BUDGET };
    pub const SERVER: Self = Self { call: SERVER_HOOK_BUDGET, aggregate: SERVER_AGGREGATE_BUDGET };
    /// A module's `init` (and its start function): it runs once, while the game loads.
    pub const INIT: Self = Self { call: Duration::from_millis(250), aggregate: Duration::from_secs(1) };

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
    texture_counts: HashMap<String, Weak<AtomicUsize>>,
}

impl Sandbox {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn engine_started(&self) -> bool {
        self.runtime.is_some()
    }

    fn texture_count(&mut self, id: &str) -> Arc<AtomicUsize> {
        self.texture_counts.retain(|_, count| count.strong_count() > 0);
        if let Some(count) = self.texture_counts.get(id).and_then(Weak::upgrade) {
            return count;
        }
        let count = Arc::new(AtomicUsize::new(0));
        self.texture_counts.insert(id.to_owned(), Arc::downgrade(&count));
        count
    }

    pub fn compile(&mut self, bytes: &[u8]) -> Result<CompiledComponent, String> {
        if self.runtime.is_none() {
            self.runtime = Some(Arc::new(RuntimeEngine::new()?));
        }
        let runtime = Arc::clone(self.runtime.as_ref().expect("runtime was just initialized"));
        let component = Component::new(&runtime.engine, bytes)
            .map_err(|error| format!("not a valid component: {error}"))?;
        for (name, _) in component.component_type().imports(&runtime.engine) {
            check_import(name)?;
        }
        Ok(CompiledComponent { runtime, component })
    }
}

fn check_import(name: &str) -> Result<(), String> {
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
    if version != INTERFACE_VERSION {
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
        Ok(Self { engine, stop, timer: Some(timer) })
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
    output: wasi::Output,
    /// When the running call must be over (the epoch callback stops the guest after it).
    deadline: Instant,
}

impl<T: Send> WasiView for HostState<T> {
    fn ctx(&mut self) -> WasiCtxView<'_> {
        WasiCtxView { ctx: &mut self.wasi, table: &mut self.table }
    }
}

pub struct CompiledComponent {
    runtime: Arc<RuntimeEngine>,
    component: Component,
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
        configure(&mut linker)?;
        let limits = store_limits();
        let output = wasi::Output::default();
        let wasi = WasiCtxBuilder::new()
            .stdout(output.stdout())
            .stderr(output.stderr())
            .allow_tcp(false)
            .allow_udp(false)
            .allow_ip_name_lookup(false)
            .build();
        let deadline = Instant::now() + TimeLimits::INIT.call;
        let state = HostState { host, limits, wasi, table: ResourceTable::new(), output, deadline };
        let mut store = Store::new(&self.runtime.engine, state);
        store.limiter(|state| &mut state.limits);
        store.set_fuel(FUEL_PER_CALL).map_err(|error| error.to_string())?;
        store.epoch_deadline_callback(|context| {
            if Instant::now() >= context.data().deadline {
                Ok(UpdateDeadline::Interrupt)
            } else {
                Ok(UpdateDeadline::Continue(1))
            }
        });
        store.set_epoch_deadline(1);
        let instance = linker
            .instantiate(&mut store, &self.component)
            .map_err(|error| format!("component instantiation failed: {error}"))?;
        Ok(ModInstance {
            id: id.into(),
            store,
            instance,
            disabled: false,
            limits: TimeLimits::CLIENT,
            fuel: FUEL_PER_CALL,
            window: Window { started: Instant::now(), spent: Duration::ZERO, exceeded: false, strikes: 0 },
            skipped: 0,
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

    /// How many calls were skipped because the module had used its aggregate budget.
    pub fn skipped_calls(&self) -> u64 {
        self.skipped
    }

    /// Whether the aggregate budget of the current window is used up (rolls the window first).
    fn over_aggregate(&mut self, now: Instant) -> bool {
        if now.duration_since(self.window.started) >= AGGREGATE_WINDOW {
            if self.window.exceeded {
                self.window.strikes += 1;
            } else {
                self.window.strikes = 0;
            }
            self.window.started = now;
            self.window.spent = Duration::ZERO;
            self.window.exceeded = false;
        }
        self.window.spent >= self.limits.aggregate
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
        if self.disabled {
            return None;
        }
        let began = Instant::now();
        if self.over_aggregate(began) {
            if !self.window.exceeded {
                self.window.exceeded = true;
                if self.window.strikes >= AGGREGATE_STRIKES {
                    self.disabled = true;
                    let id = self.id.replace(['\r', '\n'], " ");
                    eprintln!("WebAssembly mod {id} disabled: over its time budget in {AGGREGATE_STRIKES} windows in a row");
                    return None;
                }
            }
            self.skipped += 1;
            if self.skipped == 1 {
                let id = self.id.replace(['\r', '\n'], " ");
                eprintln!("WebAssembly mod {id}: calls skipped, it used its time budget for this window (further skips are only counted)");
            }
            return None;
        }
        self.store.data_mut().deadline = began + self.limits.call;
        let result = self
            .store
            .set_fuel(self.fuel)
            .map_err(|error| error.to_string())
            .and_then(|()| {
                self.store.set_epoch_deadline(1);
                invoke(&mut self.store, &self.instance)
            });
        self.window.spent += began.elapsed();
        match result {
            Ok(value) => Some(value),
            Err(error) => {
                self.disabled = true;
                let id = self.id.replace(['\r', '\n'], " ");
                let hook = hook.replace(['\r', '\n'], " ");
                let reason = error.replace(['\r', '\n'], " ");
                eprintln!("WebAssembly mod {id} disabled in {hook}: {reason}");
                None
            }
        }
    }
}
