use std::sync::{Arc, Condvar, Mutex};
use std::thread::{self, JoinHandle};
use std::time::Duration;
use wasmtime::component::{Component, Instance, Linker};
use wasmtime::{Config, Engine, Store, StoreLimits, StoreLimitsBuilder};

mod hook_names;
mod subscriptions;

pub use subscriptions::Subscriptions;

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
}

impl Sandbox {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn engine_started(&self) -> bool {
        self.runtime.is_some()
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
    let Some(import) = name.strip_prefix("yoake:mod/") else {
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
}

pub struct CompiledComponent {
    runtime: Arc<RuntimeEngine>,
    component: Component,
}

impl CompiledComponent {
    pub fn instantiate<T: 'static>(
        self,
        id: impl Into<String>,
        host: T,
        configure: impl FnOnce(&mut Linker<HostState<T>>) -> Result<(), String>,
    ) -> Result<ModInstance<T>, String> {
        let mut linker = Linker::new(&self.runtime.engine);
        configure(&mut linker)?;
        let limits = StoreLimitsBuilder::new()
            .memory_size(MEMORY_LIMIT_BYTES)
            .memories(1)
            .instances(16)
            .build();
        let mut store = Store::new(&self.runtime.engine, HostState { host, limits });
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
