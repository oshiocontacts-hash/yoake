use std::collections::{BTreeMap, BTreeSet};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex, OnceLock};

use yoake_inject_rt::{
    CallbackError, Guest, HeadOutcome, Mod, Registry, Side, Site, Subscription, Value,
};
use yoake_value::wire::{self, Decoder, Encoder, WireValue};

use crate::package_services::{CONFIG_CAPABILITY, ServiceHandle};
use crate::{CompiledComponent, LoadError, ModInstance, Sandbox, TimeLimits};

pub mod bindings {
    wasmtime::component::bindgen!({ path: "wit-inject", world: "inject-mod" });
}

pub mod config_bindings {
    wasmtime::component::bindgen!({ path: "wit-inject-config", world: "inject-config" });
}

use bindings::exports::yoake::mods::inject as wit;
use config_bindings::yoake::mods1_2_0::config as config_wit;

#[derive(Clone)]
struct ConfigHost {
    services: Result<ServiceHandle, String>,
    allowed: bool,
    cursor: u64,
}

impl config_wit::Host for ConfigHost {
    fn config_get(&mut self, key: String) -> Result<config_wit::ConfigValue, String> {
        if !self.allowed {
            return Err(format!("permission denied: {CONFIG_CAPABILITY}"));
        }
        crate::full::service_call(self.services.clone(), |services| services.config_get(&key))
            .map(config_value)
    }
}

fn config_value(value: crate::server_types::ConfigValue) -> config_wit::ConfigValue {
    use crate::server_types::ConfigValueKind as Source;
    use config_wit::ConfigValueKind as Target;
    config_wit::ConfigValue {
        kind: match value.kind {
            Source::Bool => Target::Bool,
            Source::Int => Target::Int,
            Source::Float => Target::Float,
            Source::String => Target::String,
            Source::Enum => Target::Enum,
            Source::BoolList => Target::BoolList,
            Source::IntList => Target::IntList,
            Source::FloatList => Target::FloatList,
            Source::StringList => Target::StringList,
            Source::EnumList => Target::EnumList,
        },
        bool_value: value.bool_value,
        int_value: value.int_value,
        float_value: value.float_value,
        string_value: value.string_value,
        bool_list: value.bool_list,
        int_list: value.int_list,
        float_list: value.float_list,
        string_list: value.string_list,
    }
}

fn link_config(
    linker: &mut wasmtime::component::Linker<crate::HostState<ConfigHost>>,
) -> Result<(), String> {
    config_bindings::InjectConfig::add_to_linker::<_, wasmtime::component::HasSelf<ConfigHost>>(
        linker,
        |state| &mut state.host,
    )
    .map_err(|error| format!("mixin config linker failed: {error:#}"))
}

pub const VALUE_LIMITS: wire::Limits = wire::Limits {
    max_nodes: 4096,
    max_depth: 32,
    max_bytes: 256 * 1024,
};

pub const LIST_VALUE_LIMITS: wire::Limits = wire::Limits {
    max_nodes: 4 * 65_536 + 4096,
    max_depth: 32,
    max_bytes: 16 * 1024 * 1024,
};

fn value_limits(site: &str) -> wire::Limits {
    if validate_target(site).is_ok_and(Site::has_list_values) {
        LIST_VALUE_LIMITS
    } else {
        VALUE_LIMITS
    }
}

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
    raw: ModInstance<ConfigHost>,
    exports: bindings::InjectMod,
    config_exports: Option<config_bindings::InjectConfig>,
    poisoned: bool,
}

struct Handler {
    runtime: Mutex<Runtime>,
    compiled: CompiledComponent,
    disabled: AtomicBool,
    config_notifications: bool,
    warned_encoding: AtomicBool,
}

impl Handler {
    fn encoding_error(&self, site: &str, error: wire::Error) -> CallbackError {
        if !self.warned_encoding.swap(true, Ordering::Relaxed) {
            eprintln!("[yoake-inject] {site}: {error}; handler skipped, vanilla result retained");
        }
        CallbackError::Skipped
    }

    fn encode_arguments(
        &self,
        site: &str,
        arguments: &[Value],
    ) -> Result<Vec<wit::Value>, CallbackError> {
        let mut encoder = Encoder::new(value_limits(site));
        arguments
            .iter()
            .map(|value| {
                encoder
                    .encode(value)
                    .map(to_wit)
                    .map_err(|error| self.encoding_error(site, error))
            })
            .collect()
    }

    fn call<ResultValue>(
        &self,
        point: &str,
        invoke: impl FnOnce(
            &bindings::InjectMod,
            &mut wasmtime::Store<crate::HostState<ConfigHost>>,
        ) -> Result<ResultValue, String>,
    ) -> Result<ResultValue, CallbackError> {
        if self.is_disabled() {
            return Err(CallbackError::Skipped);
        }
        let mut runtime = match self.runtime.try_lock() {
            Ok(runtime) => runtime,
            Err(std::sync::TryLockError::WouldBlock) => return Err(CallbackError::Skipped),
            Err(std::sync::TryLockError::Poisoned(_)) => return Err(CallbackError::Trap),
        };
        if runtime.poisoned {
            let began = std::time::Instant::now();
            let mut host = runtime.raw.store.data().host.clone();
            host.cursor = 0;
            let replacement = self
                .compiled
                .clone()
                .instantiate(runtime.raw.id.clone(), host, link_config)
                .and_then(|mut raw| {
                    let exports = bindings::InjectMod::new(&mut raw.store, &raw.instance)
                        .map_err(|error| format!("mixin recovery failed: {error:#}"))?;
                    let config_exports =
                        config_bindings::InjectConfig::new(&mut raw.store, &raw.instance).ok();
                    raw.limits = runtime.raw.limits;
                    raw.fuel = runtime.raw.fuel;
                    std::mem::swap(&mut raw.window, &mut runtime.raw.window);
                    raw.window.spent += began.elapsed();
                    raw.skipped = runtime.raw.skipped;
                    Ok(Runtime {
                        raw,
                        exports,
                        config_exports,
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
                    self.disabled.store(true, Ordering::Relaxed);
                    runtime.poisoned = false;
                    return Err(CallbackError::Trap);
                }
            }
        }
        let Runtime {
            raw,
            exports,
            config_exports,
            poisoned,
        } = &mut *runtime;
        let mut config_trapped = false;
        let result = raw.call_recoverable(point, |store, _| {
            if let Some(config_exports) = config_exports
                .as_ref()
                .filter(|_| self.config_notifications)
            {
                let host = &store.data().host;
                let (cursor, changes) =
                    crate::full::service_call(host.services.clone(), |services| {
                        services.config_changes_since(host.cursor)
                    })?;
                for (key, value) in changes {
                    config_exports
                        .yoake_mods1_2_0_config_events()
                        .call_config_changed(&mut *store, &key, &config_value(value))
                        .map_err(|error| {
                            config_trapped = true;
                            format!("config-changed: {error:#}")
                        })?;
                }
                store.data_mut().host.cursor = cursor;
            }
            invoke(exports, store)
        });
        if config_trapped {
            raw.disabled = true;
        }
        self.disabled.store(raw.disabled, Ordering::Relaxed);
        for line in raw.drain_output() {
            eprintln!("[yoake-inject:{}:{:?}] {}", raw.id, line.stream, line.text);
        }
        match result {
            Ok(Some(value)) => Ok(value),
            Ok(None) => Err(CallbackError::Skipped),
            Err(error) => {
                eprintln!("[yoake-inject:{}] {point}: {error}", raw.id);
                *poisoned = !raw.disabled;
                Err(CallbackError::Trap)
            }
        }
    }
}

impl Guest for Handler {
    fn is_disabled(&self) -> bool {
        self.disabled.load(Ordering::Relaxed)
    }

    fn head(&self, site: &str, arguments: &[Value]) -> Result<HeadOutcome, CallbackError> {
        let arguments = self.encode_arguments(site, arguments)?;
        let outcome = self.call("inject-head", |exports, store| {
            exports
                .yoake_mods_inject()
                .call_inject_head(store, site, &arguments)
                .map_err(|error| format!("{error:#}"))
        })?;
        let mut decoder = Decoder::new(value_limits(site));
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
        let result = Encoder::new(value_limits(site))
            .encode(result)
            .map_err(|error| self.encoding_error(site, error))?;
        let result = to_wit(result);
        let replacement = self.call("inject-return", |exports, store| {
            exports
                .yoake_mods_inject()
                .call_inject_return(store, site, &result)
                .map_err(|error| format!("{error:#}"))
        })?;
        replacement
            .map(|value| decode(&mut Decoder::new(value_limits(site)), value))
            .transpose()
    }

    fn overwrite(&self, site: &str, arguments: &[Value]) -> Result<Value, CallbackError> {
        let arguments = self.encode_arguments(site, arguments)?;
        let result = self.call("inject-overwrite", |exports, store| {
            exports
                .yoake_mods_inject()
                .call_inject_overwrite(store, site, &arguments)
                .map_err(|error| format!("{error:#}"))
        })?;
        decode(&mut Decoder::new(value_limits(site)), result)
    }
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
            .prepare(subscriptions, |module, site, _| module.permits(site))
            .map_err(|error| format!("mixin registration rejected: {error:?}"))
    }
}

static HUB: OnceLock<Mutex<Option<Hub>>> = OnceLock::new();

pub struct Injection {
    key: Option<String>,
    handler: Option<Arc<Handler>>,
    module: Option<Arc<Mod>>,
}

impl Injection {
    pub fn instantiate(
        sandbox: &mut Sandbox,
        id: &str,
        bytes: &[u8],
        targets: &[String],
        side: Side,
    ) -> Result<Self, LoadError> {
        Self::instantiate_with_permissions(sandbox, id, bytes, targets, side, &[])
    }

    pub fn instantiate_with_permissions(
        sandbox: &mut Sandbox,
        id: &str,
        bytes: &[u8],
        targets: &[String],
        side: Side,
        permissions: &[String],
    ) -> Result<Self, LoadError> {
        Self::instantiate_inner(sandbox, id, bytes, targets, side, permissions, None)
    }

    pub fn instantiate_with_services(
        sandbox: &mut Sandbox,
        id: &str,
        bytes: &[u8],
        targets: &[String],
        side: Side,
        permissions: &[String],
        services: ServiceHandle,
    ) -> Result<Self, LoadError> {
        Self::instantiate_inner(
            sandbox,
            id,
            bytes,
            targets,
            side,
            permissions,
            Some(services),
        )
    }

    fn instantiate_inner(
        sandbox: &mut Sandbox,
        id: &str,
        bytes: &[u8],
        targets: &[String],
        side: Side,
        permissions: &[String],
        services: Option<ServiceHandle>,
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
        if !expected
            .values()
            .any(|site| site.side() == Side::Both || site.side() == side)
        {
            return Ok(Self {
                key: None,
                handler: None,
                module: None,
            });
        }
        let cursor = services
            .as_ref()
            .map(|services| {
                services
                    .lock()
                    .map_err(|_| "package service lock poisoned".to_owned())?
                    .config_cursor(id)
            })
            .transpose()
            .map_err(LoadError::new)?
            .unwrap_or(0);
        let host = ConfigHost {
            services: services.ok_or_else(|| {
                format!("permission denied: {CONFIG_CAPABILITY} (package services unavailable)")
            }),
            allowed: permissions
                .iter()
                .any(|permission| permission == CONFIG_CAPABILITY),
            cursor,
        };
        let compiled = sandbox.compile_injection(bytes).map_err(LoadError::new)?;
        let mut raw = compiled
            .clone()
            .instantiate(id.to_owned(), host, link_config)
            .map_err(LoadError::new)?;
        let exports = bindings::InjectMod::new(&mut raw.store, &raw.instance).map_err(|error| {
            LoadError::new(format!(
                "mixin component must implement yoake:mods/inject@1.1.0: {error:#}"
            ))
        })?;
        let config_exports = config_bindings::InjectConfig::new(&mut raw.store, &raw.instance).ok();
        let config_notifications = config_exports.is_some()
            && raw.store.data().host.allowed
            && raw.store.data().host.services.is_ok();
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
        let key = format!("{id}:{side:?}");
        let handler = Arc::new(Handler {
            runtime: Mutex::new(Runtime {
                raw,
                exports,
                config_exports,
                poisoned: false,
            }),
            compiled,
            disabled: AtomicBool::new(false),
            config_notifications,
            warned_encoding: AtomicBool::new(false),
        });
        let entry = Entry {
            side,
            registrations: selected,
            module: Arc::new(Mod::with_permissions(
                id,
                handler.clone(),
                side,
                permissions.iter().cloned(),
            )),
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
        let module = Arc::clone(&entry.module);
        hub.entries.insert(key.clone(), entry);
        Ok(Self {
            key: Some(key),
            handler: Some(handler),
            module: Some(module),
        })
    }

    pub fn has_config_notifications(&self) -> bool {
        self.handler
            .as_ref()
            .is_some_and(|handler| handler.config_notifications && !handler.is_disabled())
    }

    #[cfg(feature = "test-guests")]
    pub fn set_test_limits(&self, limits: TimeLimits) {
        if let Some(handler) = &self.handler {
            handler.runtime.lock().unwrap().raw.set_limits(limits);
        }
    }

    pub fn notify_config_changes(&self) {
        if self.has_config_notifications() {
            let handler = self.handler.as_ref().unwrap();
            let _ = handler.call("config-changed", |_, _| Ok(()));
            if handler.is_disabled() {
                self.module.as_ref().unwrap().disable();
            }
        }
    }
}

impl Drop for Injection {
    fn drop(&mut self) {
        let Some(key) = &self.key else { return };
        let Some(mut hub) = HUB.get().and_then(|hub| hub.lock().ok()) else {
            return;
        };
        let Some(hub) = hub.as_mut() else { return };
        if let Ok(prepared) = hub.prepare(None, Some(key)) {
            if hub.registry.publish(prepared).is_ok() {
                hub.entries.remove(key);
            }
        }
    }
}
