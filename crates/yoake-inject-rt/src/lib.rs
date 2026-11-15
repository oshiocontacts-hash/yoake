#[cfg(test)]
extern crate self as yoake_inject_rt;

use std::cell::Cell;
use std::collections::{BTreeMap, HashMap};
#[cfg(test)]
use std::sync::atomic::AtomicUsize;
use std::sync::atomic::{AtomicBool, AtomicU8, Ordering};
use std::sync::{Arc, RwLock};

pub use yoake_value::{InjectSnapshot, InjectValue, Scope, Value, ValueType};

pub mod catalog;

const fn same_bytes(left: &str, right: &str) -> bool {
    let left = left.as_bytes();
    let right = right.as_bytes();
    if left.len() != right.len() {
        return false;
    }
    let mut index = 0;
    while index < left.len() {
        if left[index] != right[index] {
            return false;
        }
        index += 1;
    }
    true
}

pub const fn catalog_index(path: &str, signature_hash: &str) -> Option<usize> {
    let mut index = 0;
    while index < catalog::KEYS.len() {
        if same_bytes(catalog::KEYS[index].0, path) {
            assert!(
                same_bytes(catalog::KEYS[index].1, signature_hash),
                "stale injection catalog"
            );
            return Some(index);
        }
        index += 1;
    }
    None
}

pub const HEAD: u8 = 1;
pub const RETURN: u8 = 2;
pub const OVERWRITE: u8 = 4;
const ALL_POINTS: u8 = HEAD | RETURN | OVERWRITE;

#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
pub enum Side {
    Client,
    Server,
    Both,
    Unspecified,
}

thread_local! {
    static ACTIVE_SIDE: Cell<Side> = const { Cell::new(Side::Client) };
}

pub struct SideScope(Side, std::marker::PhantomData<*mut ()>);

pub fn enter_side(side: Side) -> SideScope {
    SideScope(ACTIVE_SIDE.replace(side), std::marker::PhantomData)
}

impl Drop for SideScope {
    fn drop(&mut self) {
        ACTIVE_SIDE.set(self.0);
    }
}

pub struct Site {
    path: &'static str,
    signature_hash: &'static str,
    side: Side,
    arguments: &'static [ValueType],
    result: ValueType,
    allowed: u8,
    mask: AtomicU8,
    claimed: AtomicBool,
    handlers: RwLock<Option<Arc<[Handler]>>>,
    #[cfg(test)]
    invocations: AtomicUsize,
}

impl Site {
    pub const fn new(
        path: &'static str,
        signature_hash: &'static str,
        side: Side,
        arguments: &'static [ValueType],
        result: ValueType,
        allowed: u8,
    ) -> Self {
        Self {
            path,
            signature_hash,
            side,
            arguments,
            result,
            allowed,
            mask: AtomicU8::new(0),
            claimed: AtomicBool::new(false),
            handlers: RwLock::new(None),
            #[cfg(test)]
            invocations: AtomicUsize::new(0),
        }
    }

    #[inline(always)]
    pub fn mask(&self) -> u8 {
        self.mask.load(Ordering::Relaxed)
    }

    pub fn id(&self) -> String {
        format!("{}#{}", self.path, self.signature_hash)
    }

    pub fn path(&self) -> &'static str {
        self.path
    }

    pub fn signature_hash(&self) -> &'static str {
        self.signature_hash
    }

    pub fn side(&self) -> Side {
        self.side
    }

    pub fn allowed_points(&self) -> u8 {
        self.allowed
    }

    fn accepts_arguments(&self, arguments: &[Value]) -> bool {
        arguments.len() == self.arguments.len()
            && arguments
                .iter()
                .zip(self.arguments)
                .all(|(value, expected)| expected.accepts(value))
    }
}

#[derive(Clone, Debug, PartialEq)]
pub enum HeadOutcome {
    Continue,
    ReplaceArguments(Vec<Value>),
    Cancel(Value),
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum CallbackError {
    Trap,
    Skipped,
}

pub trait Guest: Send + Sync {
    fn head(&self, _site: &str, _arguments: &[Value]) -> Result<HeadOutcome, CallbackError> {
        Ok(HeadOutcome::Continue)
    }

    fn returned(&self, _site: &str, _result: &Value) -> Result<Option<Value>, CallbackError> {
        Ok(None)
    }

    fn overwrite(&self, _site: &str, _arguments: &[Value]) -> Result<Value, CallbackError> {
        Err(CallbackError::Trap)
    }
}

pub struct Mod {
    id: String,
    guest: Arc<dyn Guest>,
    disabled: AtomicBool,
}

impl Mod {
    pub fn new(id: impl Into<String>, guest: Arc<dyn Guest>) -> Self {
        Self {
            id: id.into(),
            guest,
            disabled: AtomicBool::new(false),
        }
    }

    pub fn id(&self) -> &str {
        &self.id
    }

    pub fn has_disabled_handlers(&self) -> bool {
        self.disabled.load(Ordering::Acquire)
    }

    fn disable(&self) {
        self.disabled.store(true, Ordering::Release);
    }
}

pub struct Subscription {
    pub site_id: String,
    pub side: Side,
    pub points: u8,
    pub module: Arc<Mod>,
}

#[derive(Debug, Eq, PartialEq)]
pub enum RegistryError {
    InvalidSite(String),
    DuplicateSite(String),
    SiteAlreadyClaimed(String),
    UnknownSite(String),
    SignatureMismatch { requested: String, expected: String },
    WrongSide(String),
    UnsupportedPoint(String),
    DuplicateSubscription { site: String, module: String },
    DuplicateModId(String),
    ConflictingOverwrite(String),
    PermissionDenied { site: String, module: String },
    WrongRegistry,
}

#[derive(Clone)]
struct Handler {
    module: Arc<Mod>,
    side: Side,
    points: u8,
    disabled: Arc<AtomicU8>,
}

impl Handler {
    fn enabled(&self, point: u8) -> bool {
        self.side == ACTIVE_SIDE.get()
            && self.points & point != 0
            && self.disabled.load(Ordering::Relaxed) & point == 0
    }

    fn disable(&self, site: &str, point: u8) {
        if self.disabled.fetch_or(point, Ordering::Relaxed) & point == 0 {
            self.module.disable();
            eprintln!(
                "[yoake-inject] mod={} site={site} point={point}: handler trapped or returned invalid values; disabled",
                self.module.id
            );
        }
    }
}

#[derive(Default)]
struct Snapshot {
    handlers: BTreeMap<String, Vec<Handler>>,
}

pub struct Prepared {
    owner: Arc<()>,
    snapshot: Snapshot,
}

pub struct Registry {
    sites: BTreeMap<String, &'static Site>,
    owner: Arc<()>,
    snapshot: Snapshot,
}

impl Registry {
    pub fn new(sites: impl IntoIterator<Item = &'static Site>) -> Result<Self, RegistryError> {
        let mut registered = BTreeMap::new();
        let mut paths = HashMap::new();
        for site in sites {
            let valid_hash = site.signature_hash.len() == 64
                && site
                    .signature_hash
                    .bytes()
                    .all(|byte| byte.is_ascii_digit() || (b'a'..=b'f').contains(&byte));
            if site.path.is_empty()
                || site.path.contains('#')
                || !valid_hash
                || site.side == Side::Unspecified
                || site.allowed == 0
                || site.allowed & !ALL_POINTS != 0
            {
                return Err(RegistryError::InvalidSite(site.id()));
            }
            let id = site.id();
            if paths.insert(site.path, ()).is_some()
                || registered.insert(id.clone(), site).is_some()
            {
                return Err(RegistryError::DuplicateSite(id));
            }
        }

        let mut claimed: Vec<&'static Site> = Vec::new();
        for site in registered.values() {
            if site
                .claimed
                .compare_exchange(false, true, Ordering::AcqRel, Ordering::Acquire)
                .is_err()
            {
                for previous in claimed {
                    previous.claimed.store(false, Ordering::Release);
                }
                return Err(RegistryError::SiteAlreadyClaimed(site.id()));
            }
            claimed.push(*site);
        }
        Ok(Self {
            sites: registered,
            owner: Arc::new(()),
            snapshot: Snapshot::default(),
        })
    }

    pub fn prepare(
        &self,
        subscriptions: impl IntoIterator<Item = Subscription>,
        mut permitted: impl FnMut(&Mod, &Site, u8) -> bool,
    ) -> Result<Prepared, RegistryError> {
        let mut snapshot = Snapshot::default();
        let mut modules: HashMap<(String, Side), Arc<Mod>> = HashMap::new();
        for subscription in subscriptions {
            let Some(site) = self.sites.get(&subscription.site_id) else {
                if let Some((path, _)) = subscription.site_id.rsplit_once('#') {
                    if let Some(expected) = self.sites.values().find(|site| site.path == path) {
                        return Err(RegistryError::SignatureMismatch {
                            requested: subscription.site_id,
                            expected: expected.id(),
                        });
                    }
                }
                return Err(RegistryError::UnknownSite(subscription.site_id));
            };
            if !matches!(subscription.side, Side::Client | Side::Server)
                || (site.side != Side::Both && site.side != subscription.side)
            {
                return Err(RegistryError::WrongSide(subscription.site_id));
            }
            if subscription.points == 0 || subscription.points & !site.allowed != 0 {
                return Err(RegistryError::UnsupportedPoint(subscription.site_id));
            }
            if subscription.module.id.is_empty() {
                return Err(RegistryError::DuplicateModId(
                    subscription.module.id.clone(),
                ));
            }
            let module_key = (subscription.module.id.clone(), subscription.side);
            if let Some(existing) = modules.get(&module_key) {
                if !Arc::ptr_eq(existing, &subscription.module) {
                    return Err(RegistryError::DuplicateModId(
                        subscription.module.id.clone(),
                    ));
                }
            } else {
                modules.insert(module_key, Arc::clone(&subscription.module));
            }
            if !permitted(&subscription.module, site, subscription.points) {
                return Err(RegistryError::PermissionDenied {
                    site: subscription.site_id,
                    module: subscription.module.id.clone(),
                });
            }
            let handlers = snapshot
                .handlers
                .entry(subscription.site_id.clone())
                .or_default();
            if handlers.iter().any(|handler| {
                handler.side == subscription.side && handler.module.id == subscription.module.id
            }) {
                return Err(RegistryError::DuplicateSubscription {
                    site: subscription.site_id,
                    module: subscription.module.id.clone(),
                });
            }
            if subscription.points & OVERWRITE != 0
                && handlers.iter().any(|handler| {
                    handler.side == subscription.side && handler.points & OVERWRITE != 0
                })
            {
                return Err(RegistryError::ConflictingOverwrite(subscription.site_id));
            }
            let disabled = self
                .snapshot
                .handlers
                .get(&subscription.site_id)
                .and_then(|previous| {
                    previous.iter().find(|handler| {
                        handler.side == subscription.side
                            && Arc::ptr_eq(&handler.module, &subscription.module)
                    })
                })
                .map_or_else(
                    || Arc::new(AtomicU8::new(0)),
                    |handler| Arc::clone(&handler.disabled),
                );
            handlers.push(Handler {
                module: subscription.module,
                side: subscription.side,
                points: subscription.points,
                disabled,
            });
        }
        for handlers in snapshot.handlers.values_mut() {
            handlers.sort_by(|left, right| left.module.id.cmp(&right.module.id));
        }
        Ok(Prepared {
            owner: Arc::clone(&self.owner),
            snapshot,
        })
    }

    pub fn publish(&mut self, prepared: Prepared) -> Result<(), RegistryError> {
        if !Arc::ptr_eq(&self.owner, &prepared.owner) {
            return Err(RegistryError::WrongRegistry);
        }
        self.snapshot = prepared.snapshot;
        for (id, site) in &self.sites {
            let handlers = self.snapshot.handlers.get(id);
            let mask = handlers.map_or(0, |handlers| {
                handlers
                    .iter()
                    .fold(0, |mask, handler| mask | handler.points)
            });
            *site
                .handlers
                .write()
                .unwrap_or_else(|poison| poison.into_inner()) =
                handlers.map(|handlers| Arc::from(handlers.clone()));
            site.mask.store(mask, Ordering::Relaxed);
        }
        Ok(())
    }

    pub fn invoke(
        &self,
        site: &'static Site,
        arguments: Vec<Value>,
        original: impl FnOnce(Vec<Value>) -> Value,
    ) -> Value {
        let id = site.id();
        if !self
            .sites
            .get(&id)
            .is_some_and(|registered| std::ptr::eq(*registered, site))
        {
            return original(arguments);
        }
        site.invoke(arguments, original)
    }
}

impl Site {
    pub fn invoke(
        &'static self,
        arguments: Vec<Value>,
        original: impl FnOnce(Vec<Value>) -> Value,
    ) -> Value {
        self.invoke_fallible(arguments, |arguments| {
            Ok::<_, std::convert::Infallible>(original(arguments))
        })
        .unwrap_or_else(|never| match never {})
    }

    pub fn invoke_fallible<Error>(
        &'static self,
        arguments: Vec<Value>,
        original: impl FnOnce(Vec<Value>) -> Result<Value, Error>,
    ) -> Result<Value, Error> {
        #[cfg(test)]
        self.invocations.fetch_add(1, Ordering::Relaxed);
        if !self.accepts_arguments(&arguments) {
            return original(arguments);
        }
        let handlers = self
            .handlers
            .read()
            .unwrap_or_else(|poison| poison.into_inner())
            .clone();
        let Some(handlers) = handlers else {
            return original(arguments);
        };
        let site = self;
        let id = site.id();
        let original_arguments = arguments.clone();
        let mut arguments = arguments;
        let mut original = Some(original);
        let mut injected_result = None;

        for handler in handlers.iter() {
            if !handler.enabled(HEAD) {
                continue;
            }
            match handler.module.guest.head(&id, &arguments) {
                Err(CallbackError::Skipped) => continue,
                Ok(HeadOutcome::Continue) => {}
                Ok(HeadOutcome::ReplaceArguments(replacement))
                    if site.accepts_arguments(&replacement)
                        && site
                            .arguments
                            .iter()
                            .zip(arguments.iter().zip(&replacement))
                            .all(|(kind, (before, after))| {
                                *kind != ValueType::Handle || before == after
                            }) =>
                {
                    arguments = replacement;
                }
                Ok(HeadOutcome::Cancel(value)) if site.result.accepts(&value) => {
                    injected_result = Some(value);
                    break;
                }
                _ => {
                    handler.disable(&id, HEAD);
                    return call_original(&mut original, original_arguments);
                }
            }
        }

        if injected_result.is_none() {
            for handler in handlers.iter() {
                if !handler.enabled(OVERWRITE) {
                    continue;
                }
                match handler.module.guest.overwrite(&id, &arguments) {
                    Err(CallbackError::Skipped) => continue,
                    Ok(value) if site.result.accepts(&value) => injected_result = Some(value),
                    _ => {
                        handler.disable(&id, OVERWRITE);
                        return call_original(&mut original, original_arguments);
                    }
                }
                break;
            }
        }

        let mut native_result = None;
        let mut result = if let Some(value) = injected_result {
            value
        } else {
            let value = call_original(&mut original, arguments)?;
            native_result = Some(value.clone());
            value
        };
        for handler in handlers.iter() {
            if !handler.enabled(RETURN) {
                continue;
            }
            match handler.module.guest.returned(&id, &result) {
                Err(CallbackError::Skipped) => continue,
                Ok(None) => {}
                Ok(Some(value)) if site.result.accepts(&value) => result = value,
                _ => {
                    handler.disable(&id, RETURN);
                    return native_result
                        .map(Ok)
                        .unwrap_or_else(|| call_original(&mut original, original_arguments));
                }
            }
        }
        Ok(result)
    }
}

fn call_original<Error>(
    original: &mut Option<impl FnOnce(Vec<Value>) -> Result<Value, Error>>,
    arguments: Vec<Value>,
) -> Result<Value, Error> {
    original.take().expect("original already called")(arguments)
}

impl Drop for Registry {
    fn drop(&mut self) {
        for site in self.sites.values() {
            site.mask.store(0, Ordering::Relaxed);
            *site
                .handlers
                .write()
                .unwrap_or_else(|poison| poison.into_inner()) = None;
            site.claimed.store(false, Ordering::Release);
        }
    }
}

#[cfg(test)]
mod tests;
