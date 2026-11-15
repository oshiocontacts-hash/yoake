use std::collections::{BTreeMap, BTreeSet};
use std::sync::{Arc, Mutex, OnceLock};

use yoake_inject_rt::{
    CallbackError, Guest, HeadOutcome, Mod, Registry, Side, Site, Subscription, Value,
};
use yoake_value::wire::{self, Decoder, Encoder, WireValue};

use crate::{CompiledComponent, LoadError, ModInstance, Sandbox, TimeLimits};

pub mod bindings {
    wasmtime::component::bindgen!({ path: "wit-inject", world: "inject-mod" });
}

use bindings::exports::yoake::mods::inject as wit;

pub const VALUE_LIMITS: wire::Limits = wire::Limits {
    max_nodes: 4096,
    max_depth: 32,
    max_bytes: 256 * 1024,
};

pub fn validate_target(target: &str) -> Result<&'static Site, String> {
    let (path, hash) = target.rsplit_once('#').ok_or_else(|| {
        format!("mixin target {target:?} must contain its catalog signature hash")
    })?;
    let site = yoake_inject_rt::catalog::SITES
        .iter()
        .find(|site| site.path() == path)
        .ok_or_else(|| {
            format!("unknown mixin target {path:?}; this engine has no such injection site")
        })?;
    if site.signature_hash() != hash {
        return Err(format!(
            "mixin signature mismatch for {path}: expected {}, got {hash}",
            site.signature_hash()
        ));
    }
    Ok(site)
}

struct Runtime {
    raw: ModInstance<()>,
    exports: bindings::InjectMod,
    poisoned: bool,
}

struct Handler {
    runtime: Mutex<Runtime>,
    compiled: CompiledComponent,
}

impl Handler {
    fn call<ResultValue>(
        &self,
        point: &str,
        invoke: impl FnOnce(
            &bindings::InjectMod,
            &mut wasmtime::Store<crate::HostState<()>>,
        ) -> Result<ResultValue, String>,
    ) -> Result<ResultValue, CallbackError> {
        let mut runtime = match self.runtime.try_lock() {
            Ok(runtime) => runtime,
            Err(std::sync::TryLockError::WouldBlock) => return Err(CallbackError::Skipped),
            Err(std::sync::TryLockError::Poisoned(_)) => return Err(CallbackError::Trap),
        };
        if runtime.poisoned {
            let began = std::time::Instant::now();
            let replacement = self
                .compiled
                .clone()
                .instantiate(runtime.raw.id.clone(), (), |_| Ok(()))
                .and_then(|mut raw| {
                    let exports = bindings::InjectMod::new(&mut raw.store, &raw.instance)
                        .map_err(|error| format!("mixin recovery failed: {error:#}"))?;
                    raw.limits = runtime.raw.limits;
                    raw.fuel = runtime.raw.fuel;
                    std::mem::swap(&mut raw.window, &mut runtime.raw.window);
                    raw.window.spent += began.elapsed();
                    raw.skipped = runtime.raw.skipped;
                    Ok(Runtime {
                        raw,
                        exports,
                        poisoned: false,
                    })
                });
            match replacement {
                Ok(replacement) => *runtime = replacement,
                Err(error) => {
                    eprintln!(
                        "[yoake-inject:{}] {error}; remaining callbacks skipped",
                        runtime.raw.id
                    );
                    runtime.raw.disabled = true;
                    runtime.poisoned = false;
                    return Err(CallbackError::Trap);
                }
            }
        }
        let Runtime {
            raw,
            exports,
            poisoned,
        } = &mut *runtime;
        let result = raw.call_recoverable(point, |store, _| invoke(exports, store));
        for line in raw.drain_output() {
            eprintln!("[yoake-inject:{}:{:?}] {}", raw.id, line.stream, line.text);
        }
        match result {
            Ok(Some(value)) => Ok(value),
            Ok(None) => Err(CallbackError::Skipped),
            Err(error) => {
                eprintln!("[yoake-inject:{}] {point}: {error}", raw.id);
                *poisoned = true;
                Err(CallbackError::Trap)
            }
        }
    }
}

impl Guest for Handler {
    fn head(&self, site: &str, arguments: &[Value]) -> Result<HeadOutcome, CallbackError> {
        let arguments = encode_arguments(arguments)?;
        let outcome = self.call("inject-head", |exports, store| {
            exports
                .yoake_mods_inject()
                .call_inject_head(store, site, &arguments)
                .map_err(|error| format!("{error:#}"))
        })?;
        let mut decoder = Decoder::new(VALUE_LIMITS);
        match outcome {
            wit::HeadOutcome::Proceed => Ok(HeadOutcome::Continue),
            wit::HeadOutcome::ReplaceArguments(arguments) => arguments
                .into_iter()
                .map(|value| decode(&mut decoder, value))
                .collect::<Result<Vec<_>, _>>()
                .map(HeadOutcome::ReplaceArguments),
            wit::HeadOutcome::Cancel(value) => decode(&mut decoder, value).map(HeadOutcome::Cancel),
        }
    }

    fn returned(&self, site: &str, result: &Value) -> Result<Option<Value>, CallbackError> {
        let result = Encoder::new(VALUE_LIMITS)
            .encode(result)
            .map_err(|_| CallbackError::Trap)?;
        let result = to_wit(result);
        let replacement = self.call("inject-return", |exports, store| {
            exports
                .yoake_mods_inject()
                .call_inject_return(store, site, &result)
                .map_err(|error| format!("{error:#}"))
        })?;
        replacement
            .map(|value| decode(&mut Decoder::new(VALUE_LIMITS), value))
            .transpose()
    }

    fn overwrite(&self, site: &str, arguments: &[Value]) -> Result<Value, CallbackError> {
        let arguments = encode_arguments(arguments)?;
        let result = self.call("inject-overwrite", |exports, store| {
            exports
                .yoake_mods_inject()
                .call_inject_overwrite(store, site, &arguments)
                .map_err(|error| format!("{error:#}"))
        })?;
        decode(&mut Decoder::new(VALUE_LIMITS), result)
    }
}

fn encode_arguments(arguments: &[Value]) -> Result<Vec<wit::Value>, CallbackError> {
    let mut encoder = Encoder::new(VALUE_LIMITS);
    arguments
        .iter()
        .map(|value| {
            encoder
                .encode(value)
                .map(to_wit)
                .map_err(|_| CallbackError::Trap)
        })
        .collect()
}

fn decode(decoder: &mut Decoder, value: wit::Value) -> Result<Value, CallbackError> {
    decoder
        .decode(&from_wit(value))
        .map_err(|_| CallbackError::Trap)
}

fn to_wit(value: WireValue) -> wit::Value {
    match value {
        WireValue::Unit => wit::Value::Unit,
        WireValue::Boolean(value) => wit::Value::Boolean(value),
        WireValue::Signed(value) => wit::Value::Signed(value),
        WireValue::Unsigned(value) => wit::Value::Unsigned(value),
        WireValue::Float(value) => wit::Value::Float(value),
        WireValue::Text(value) => wit::Value::Text(value),
        WireValue::Bytes(value) => wit::Value::Bytes(value),
        WireValue::Sequence(tree) => wit::Value::Sequence(to_wit_tree(tree)),
        WireValue::Object(tree) => wit::Value::Object(to_wit_tree(tree)),
    }
}

fn to_wit_tree(tree: wire::ValueTree) -> wit::ValueTree {
    wit::ValueTree {
        root: tree.root,
        nodes: tree
            .nodes
            .into_iter()
            .map(|node| match node {
                wire::ValueNode::Unit => wit::ValueNode::Unit,
                wire::ValueNode::Boolean(value) => wit::ValueNode::Boolean(value),
                wire::ValueNode::Signed(value) => wit::ValueNode::Signed(value),
                wire::ValueNode::Unsigned(value) => wit::ValueNode::Unsigned(value),
                wire::ValueNode::Float(value) => wit::ValueNode::Float(value),
                wire::ValueNode::Text(value) => wit::ValueNode::Text(value),
                wire::ValueNode::Bytes(value) => wit::ValueNode::Bytes(value),
                wire::ValueNode::Sequence(value) => wit::ValueNode::Sequence(value),
                wire::ValueNode::Object(value) => wit::ValueNode::Object(
                    value
                        .into_iter()
                        .map(|entry| wit::RecordEntry {
                            key: entry.key,
                            node: entry.node,
                        })
                        .collect(),
                ),
            })
            .collect(),
    }
}

fn from_wit(value: wit::Value) -> WireValue {
    match value {
        wit::Value::Unit => WireValue::Unit,
        wit::Value::Boolean(value) => WireValue::Boolean(value),
        wit::Value::Signed(value) => WireValue::Signed(value),
        wit::Value::Unsigned(value) => WireValue::Unsigned(value),
        wit::Value::Float(value) => WireValue::Float(value),
        wit::Value::Text(value) => WireValue::Text(value),
        wit::Value::Bytes(value) => WireValue::Bytes(value),
        wit::Value::Sequence(tree) => WireValue::Sequence(from_wit_tree(tree)),
        wit::Value::Object(tree) => WireValue::Object(from_wit_tree(tree)),
    }
}

fn from_wit_tree(tree: wit::ValueTree) -> wire::ValueTree {
    wire::ValueTree {
        root: tree.root,
        nodes: tree
            .nodes
            .into_iter()
            .map(|node| match node {
                wit::ValueNode::Unit => wire::ValueNode::Unit,
                wit::ValueNode::Boolean(value) => wire::ValueNode::Boolean(value),
                wit::ValueNode::Signed(value) => wire::ValueNode::Signed(value),
                wit::ValueNode::Unsigned(value) => wire::ValueNode::Unsigned(value),
                wit::ValueNode::Float(value) => wire::ValueNode::Float(value),
                wit::ValueNode::Text(value) => wire::ValueNode::Text(value),
                wit::ValueNode::Bytes(value) => wire::ValueNode::Bytes(value),
                wit::ValueNode::Sequence(value) => wire::ValueNode::Sequence(value),
                wit::ValueNode::Object(value) => wire::ValueNode::Object(
                    value
                        .into_iter()
                        .map(|entry| wire::RecordEntry {
                            key: entry.key,
                            node: entry.node,
                        })
                        .collect(),
                ),
            })
            .collect(),
    }
}

struct Entry {
    side: Side,
    registrations: Vec<(String, u8)>,
    module: Arc<Mod>,
}

struct Hub {
    registry: Registry,
    entries: BTreeMap<String, Entry>,
}

impl Hub {
    fn prepare(
        &self,
        added: Option<&Entry>,
        removed: Option<&str>,
    ) -> Result<yoake_inject_rt::Prepared, String> {
        let subscriptions = self
            .entries
            .iter()
            .filter(|(key, _)| Some(key.as_str()) != removed)
            .map(|(_, entry)| entry)
            .chain(added)
            .flat_map(|entry| {
                entry
                    .registrations
                    .iter()
                    .map(|(site, points)| Subscription {
                        site_id: site.clone(),
                        side: entry.side,
                        points: *points,
                        module: Arc::clone(&entry.module),
                    })
            });
        self.registry
            .prepare(subscriptions, |_, _, _| true)
            .map_err(|error| format!("mixin registration rejected: {error:?}"))
    }
}

static HUB: OnceLock<Mutex<Option<Hub>>> = OnceLock::new();

pub struct Injection {
    key: String,
}

impl Injection {
    pub fn instantiate(
        sandbox: &mut Sandbox,
        id: &str,
        bytes: &[u8],
        targets: &[String],
        side: Side,
    ) -> Result<Self, LoadError> {
        if targets.is_empty() || targets.len() > 1024 {
            return Err(LoadError::new("mixin requires between 1 and 1024 targets"));
        }
        let expected = targets
            .iter()
            .map(|target| {
                let site = validate_target(target)?;
                Ok((target.clone(), site))
            })
            .collect::<Result<BTreeMap<_, _>, String>>()
            .map_err(LoadError::new)?;
        if expected.len() != targets.len() {
            return Err(LoadError::new("duplicate manifest mixin target"));
        }
        let limits = match side {
            Side::Client => TimeLimits::CLIENT,
            Side::Server => TimeLimits::SERVER,
            Side::Both | Side::Unspecified => {
                return Err(LoadError::new("mixin side must be client or server"));
            }
        };
        let compiled = sandbox.compile(bytes).map_err(LoadError::new)?;
        let mut raw = compiled
            .clone()
            .instantiate(id.to_owned(), (), |_| Ok(()))
            .map_err(LoadError::new)?;
        let exports = bindings::InjectMod::new(&mut raw.store, &raw.instance).map_err(|error| {
            LoadError::new(format!(
                "mixin component must implement yoake:mods/inject@1.1.0: {error:#}"
            ))
        })?;
        let registrations = raw
            .call_init(limits, |store, _| {
                exports
                    .yoake_mods_inject()
                    .call_registrations(store)
                    .map_err(|error| format!("{error:#}"))
            })
            .ok_or_else(|| {
                LoadError::new("mixin registrations trapped or exceeded initialization limits")
            })?;
        let mut seen = BTreeSet::new();
        let mut selected = Vec::new();
        for registration in registrations {
            let site = expected.get(&registration.site).ok_or_else(|| {
                LoadError::new(format!("undeclared mixin target {}", registration.site))
            })?;
            if !seen.insert(registration.site.clone()) {
                return Err(LoadError::new(format!(
                    "duplicate mixin registration {}",
                    registration.site
                )));
            }
            if registration.points == 0 || registration.points & !site.allowed_points() != 0 {
                return Err(LoadError::new(format!(
                    "unsupported mixin injection points for {}",
                    registration.site
                )));
            }
            if site.side() == Side::Both || site.side() == side {
                selected.push((registration.site, registration.points));
            }
        }
        if seen.len() != expected.len() {
            return Err(LoadError::new(
                "component registrations must cover every manifest mixin target",
            ));
        }
        if selected.is_empty() {
            return Err(LoadError::new(
                "component has no mixin targets for this side",
            ));
        }
        let key = format!("{id}:{side:?}");
        let handler = Arc::new(Handler {
            runtime: Mutex::new(Runtime {
                raw,
                exports,
                poisoned: false,
            }),
            compiled,
        });
        let entry = Entry {
            side,
            registrations: selected,
            module: Arc::new(Mod::new(key.clone(), handler)),
        };
        let mut hub = HUB
            .get_or_init(|| Mutex::new(None))
            .lock()
            .map_err(|_| LoadError::new("mixin registry poisoned"))?;
        if hub.is_none() {
            *hub = Some(Hub {
                registry: Registry::new(yoake_inject_rt::catalog::SITES.iter()).map_err(
                    |error| LoadError::new(format!("invalid engine injection catalog: {error:?}")),
                )?,
                entries: BTreeMap::new(),
            });
        }
        let hub = hub.as_mut().unwrap();
        if hub.entries.contains_key(&key) {
            return Err(LoadError::new(format!(
                "mixin component {id} already loaded for {side:?}"
            )));
        }
        let prepared = hub.prepare(Some(&entry), None).map_err(LoadError::new)?;
        hub.registry
            .publish(prepared)
            .map_err(|error| LoadError::new(format!("mixin publish failed: {error:?}")))?;
        hub.entries.insert(key.clone(), entry);
        Ok(Self { key })
    }
}

impl Drop for Injection {
    fn drop(&mut self) {
        let Some(mut hub) = HUB.get().and_then(|hub| hub.lock().ok()) else {
            return;
        };
        let Some(hub) = hub.as_mut() else { return };
        if let Ok(prepared) = hub.prepare(None, Some(&self.key)) {
            if hub.registry.publish(prepared).is_ok() {
                hub.entries.remove(&self.key);
            }
        }
    }
}
