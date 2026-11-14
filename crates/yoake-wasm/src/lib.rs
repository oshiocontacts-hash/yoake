use std::collections::HashMap;
use std::sync::atomic::AtomicUsize;
use std::sync::{Arc, Condvar, Mutex, Weak};
use std::thread::{self, JoinHandle};
use std::time::Duration;
use wasmtime::component::{Component, Instance, Linker, ResourceTable};
use wasmtime::{Config, Engine, Store, StoreLimits, StoreLimitsBuilder};
use wasmtime_wasi::{WasiCtx, WasiCtxBuilder, WasiCtxView, WasiView};

mod client;
pub mod convert;
mod full;
mod full_hook_names;
pub mod full_bindings;
mod host;
mod hook_names;
mod subscriptions;
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
const CLIENT_HOOK_BUDGET_MILLIS: u64 = 2;
const EPOCH_INTERVAL_MILLIS: u64 = 1;
pub const CLIENT_HOOK_BUDGET: Duration = Duration::from_millis(CLIENT_HOOK_BUDGET_MILLIS);
pub const FUEL_PER_CALL: u64 = 10_000_000;
const EPOCH_INTERVAL: Duration = Duration::from_millis(EPOCH_INTERVAL_MILLIS);
const EPOCH_DEADLINE: u64 = CLIENT_HOOK_BUDGET_MILLIS / EPOCH_INTERVAL_MILLIS;
const INTERFACE_VERSION: &str = "1.0.0";

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
    stop: Arc<(Mutex<bool>, Condvar)>,
    timer: Option<JoinHandle<()>>,
}

impl RuntimeEngine {
    fn new() -> Result<Self, String> {
        let mut config = Config::new();
        config
            .wasm_component_model(true)
            .consume_fuel(true)
            .epoch_interruption(true);
        let engine = Engine::new(&config).map_err(|error| error.to_string())?;
        let stop = Arc::new((Mutex::new(false), Condvar::new()));
        let timer_stop = Arc::clone(&stop);
        let timer_engine = engine.clone();
        let timer = thread::Builder::new()
            .name("yoake-wasm-epoch".into())
            .spawn(move || {
                let (lock, wake) = &*timer_stop;
                let mut stopped = lock.lock().unwrap_or_else(|error| error.into_inner());
                loop {
                    let (guard, timeout) = wake
                        .wait_timeout(stopped, EPOCH_INTERVAL)
                        .unwrap_or_else(|error| error.into_inner());
                    stopped = guard;
                    if *stopped {
                        break;
                    }
                    if timeout.timed_out() {
                        timer_engine.increment_epoch();
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
        let (lock, wake) = &*self.stop;
        {
            let mut stopped = lock.lock().unwrap_or_else(|error| error.into_inner());
            *stopped = true;
            wake.notify_one();
        }
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
        let limits = StoreLimitsBuilder::new()
            .memory_size(MEMORY_LIMIT_BYTES)
            .memories(1)
            .instances(16)
            .build();
        let output = wasi::Output::default();
        let wasi = WasiCtxBuilder::new()
            .stdout(output.stdout())
            .stderr(output.stderr())
            .allow_tcp(false)
            .allow_udp(false)
            .allow_ip_name_lookup(false)
            .build();
        let state = HostState { host, limits, wasi, table: ResourceTable::new(), output };
        let mut store = Store::new(&self.runtime.engine, state);
        store.limiter(|state| &mut state.limits);
        store.set_fuel(FUEL_PER_CALL).map_err(|error| error.to_string())?;
        store.set_epoch_deadline(EPOCH_DEADLINE);
        let instance = linker
            .instantiate(&mut store, &self.component)
            .map_err(|error| format!("component instantiation failed: {error}"))?;
        Ok(ModInstance {
            id: id.into(),
            store,
            instance,
            disabled: false,
            _runtime: self.runtime,
        })
    }
}

pub struct ModInstance<T: 'static> {
    id: String,
    store: Store<HostState<T>>,
    instance: Instance,
    disabled: bool,
    _runtime: Arc<RuntimeEngine>,
}

impl<T: 'static> ModInstance<T> {
    pub fn is_disabled(&self) -> bool {
        self.disabled
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
        let result = self
            .store
            .set_fuel(FUEL_PER_CALL)
            .map_err(|error| error.to_string())
            .and_then(|()| {
                self.store.set_epoch_deadline(EPOCH_DEADLINE);
                invoke(&mut self.store, &self.instance)
            });
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
