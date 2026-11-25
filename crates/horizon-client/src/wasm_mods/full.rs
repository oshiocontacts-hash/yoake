//! Full-catalog WebAssembly mods (`client-full` and `server-full`, `crates/yoake-wasm/wit/yoake-full.wit`).
//!
//! One [`FullMod`] per `.yak` holds the module instances of that package (a client one and a server one,
//! each optional) and implements every hook of the catalog the generator could wire. The hook methods
//! are generated into the committed `full_adapters.rs` (included below): each converts the native
//! event to the WIT values, calls the guest only if it subscribed (one atomic load otherwise), and
//! applies the answer per the hook's contract. Nothing here exists, and no hook of the game is wrapped,
//! unless a `.yak` with such a module is installed.
//!
//! A hook of the game's shared code runs on the client thread or the server thread; the thread decides
//! which module gets it, by the explicit server-thread scope the server loops enter
//! (`crate::mod_guard::enter_thread`), not by the thread's name. The two modules have their own locks,
//! so a client hook never waits for the server thread. A hook that finds its module busy (a re-entry,
//! or the other thread holding the lock) is dropped with the hook's documented fallback (no answer
//! applied), and counted: the first drop of a module is logged.

use super::server_services::{ServerSlot, register_endpoint};
use super::*;
use crate::gfx::native_render::*;
use horizon_client_core::text::Text;
use horizon_items::items::ItemStack;
use horizon_world::nbt::{Compound, Tag};
pub(super) use yoake_inject_rt::command_gate;
pub(super) use yoake_wasm::full_bindings::client::yoake::mods::engine_types as et;
use yoake_wasm::{ClientFullMod, ServerFullMod, convert, server_types as st};

/// Commands a guest may return for one object in one call.
const MAX_COMMANDS: usize = 8192;

#[derive(Clone, Copy, PartialEq, Eq)]
pub(super) enum Side {
    Client = 0,
    Server = 1,
}

/// Whether the current thread runs a (dedicated or integrated) server loop: the loop entered the
/// server scope (`mod_guard::enter_thread(ModThread::Server)`).
fn on_server_thread() -> bool {
    crate::mod_guard::current_thread() == crate::mod_guard::ModThread::Server
}

// ---------------------------------------------------------------------------------------------
// The module of one .yak
// ---------------------------------------------------------------------------------------------

const MASK_WORDS: usize = 8;

pub(super) struct SideState<M> {
    pub(super) module: Option<M>,
    reported: bool,
}

pub(super) struct FullMod {
    id: String,
    client: Arc<Mutex<SideState<ClientFullMod>>>,
    gpu_owner: Option<crate::gfx::mod_gpu::ModId>,
    gpu_version: String,
    pub(super) server: Arc<ServerSlot>,
    /// One bit per catalog hook, per side: the guest subscribed and has not been disabled.
    masks: Arc<[[AtomicU64; MASK_WORDS]; 2]>,
    /// Hooks dropped because the module was busy, per side.
    dropped: [AtomicU64; 2],
    client_reload: Option<Arc<Reload>>,
    /// What starts the server module afresh at each world start.
    server_source: Option<ServerSource>,
    /// F4M-13 R3: the watched file's new bytes, swapped in at a server tick.
    server_reload: Option<Arc<Reload>>,
    /// The bytes of the last swapped-in server module: the next world starts from them, not from the first ones.
    server_latest: Mutex<Option<Arc<Vec<u8>>>>,
}

/// The bytes of a server module and its grants, kept so every world starts with a fresh instance.
pub(super) struct ServerSource {
    id: String,
    bytes: Arc<Vec<u8>>,
    commands: bool,
}

impl ServerSource {
    pub(super) fn new(id: &str, bytes: Vec<u8>, commands: bool) -> Self {
        Self {
            id: id.to_owned(),
            bytes: Arc::new(bytes),
            commands,
        }
    }

    pub(super) fn module(&self) -> ModuleSource<'_> {
        ModuleSource {
            id: &self.id,
            bytes: &self.bytes,
            commands: self.commands,
            gpu: false,
        }
    }
}

use std::sync::atomic::AtomicU64;

fn mask_of(subscribed: impl Fn(&str) -> bool) -> [AtomicU64; MASK_WORDS] {
    let words: [AtomicU64; MASK_WORDS] = std::array::from_fn(|_| AtomicU64::new(0));
    for (ordinal, (name, _)) in yoake_wasm::FULL_HOOK_LIST.iter().enumerate() {
        if subscribed(name) || (*name == "tick_start" && subscribed("config_changed")) {
            words[ordinal / 64].fetch_or(1 << (ordinal % 64), Ordering::Relaxed);
        }
    }
    words
}

fn side_name(side: Side) -> &'static str {
    match side {
        Side::Client => "client",
        Side::Server => "server",
    }
}

impl FullMod {
    pub(super) fn new(
        id: String,
        client: Option<ClientFullMod>,
        server: Option<ServerFullMod>,
    ) -> Self {
        let client_mask = match &client {
            Some(module) => mask_of(|name| module.subscribed(name)),
            None => mask_of(|_| false),
        };
        let server_mask = match &server {
            Some(module) => mask_of(|name| module.subscribed(name)),
            None => mask_of(|_| false),
        };
        let server = Arc::new(ServerSlot {
            state: Mutex::new(SideState {
                module: server,
                reported: false,
            }),
            dirty: std::sync::atomic::AtomicBool::new(false),
        });
        register_endpoint(&id, &server);
        let client_slot = Arc::new(Mutex::new(SideState {
            module: client,
            reported: false,
        }));
        super::mob_runtime::register_client(&client_slot);
        super::client_services::register_client_endpoint(&id, &client_slot);
        Self {
            id,
            client: client_slot,
            gpu_owner: None,
            gpu_version: String::new(),
            server,
            masks: Arc::new([client_mask, server_mask]),
            dropped: [AtomicU64::new(0), AtomicU64::new(0)],
            client_reload: None,
            server_source: None,
            server_reload: None,
            server_latest: Mutex::new(None),
        }
    }

    pub(super) fn with_gpu(mut self, enabled: bool, version: &str, fingerprint: &str) -> Self {
        if !enabled {
            return self;
        }
        let Some(owner) = super::gpu_services::owner(&self.id) else {
            return self;
        };
        let client = Arc::downgrade(&self.client);
        let id = self.id.clone();
        let masks = Arc::clone(&self.masks);
        let reload = self.client_reload.clone();
        let version = version.to_owned();
        self.gpu_version = version.clone();
        crate::gfx::mod_gpu_runtime::register_package(
            owner,
            &self.id,
            fingerprint,
            Arc::new(move || {
                let Some(client) = client.upgrade() else {
                    return;
                };
                reload_client(
                    &id,
                    &client,
                    &masks,
                    reload.as_deref(),
                    Some(owner),
                    &version,
                );
                let mut state = match client.try_lock() {
                    Ok(state) => state,
                    Err(std::sync::TryLockError::WouldBlock) => return,
                    Err(std::sync::TryLockError::Poisoned(error)) => {
                        for word in &masks[Side::Client as usize] {
                            word.store(0, Ordering::Relaxed);
                        }
                        crate::gfx::mod_gpu_runtime::disable(owner, "WASM client lock poisoned");
                        let mut state = error.into_inner();
                        if !state.reported {
                            state.reported = true;
                            log::error!(
                                "wasm mod {id} (client) disabled after its GPU callback lock was poisoned"
                            );
                        }
                        return;
                    }
                };
                let Some(module) = state.module.as_mut() else {
                    return;
                };
                if !module.subscribed("gpu_frame") {
                    return;
                }
                let mut invocation = Invocation {
                    host: HostAccess::None,
                    batch: None,
                    batch_start: 0,
                    client: ClientExtras::default(),
                };
                {
                    let _scope = Scope::enter(&mut invocation);
                    module.gpu_frame();
                }
                if module.is_disabled() {
                    for word in &masks[Side::Client as usize] {
                        word.store(0, Ordering::Relaxed);
                    }
                    crate::gfx::mod_gpu_runtime::disable(owner, "WASM client trapped");
                    if !state.reported {
                        state.reported = true;
                        log::error!(
                            "wasm mod {id} (client) disabled during its GPU frame callback"
                        );
                    }
                }
            }),
        );
        self.gpu_owner = Some(owner);
        self
    }

    pub(super) fn with_server_source(mut self, source: ServerSource) -> Self {
        self.server_source = Some(source);
        self
    }

    pub(super) fn with_server_reload(mut self, reload: Option<Arc<Reload>>) -> Self {
        self.server_reload = reload;
        self
    }

    /// R3 (F4M-13): when the watched server module file changed, the new build becomes the running
    /// instance here, at a server tick, between two hook calls (a hook that is running keeps the
    /// module locked, and the swap waits for the next tick). The engine owns the mobs, so only what the
    /// guest keeps in its own instance starts afresh; the goals are called by name, so the new
    /// instance's exports take their place and the subscription masks are read again. A build that does
    /// not load keeps the old instance running, with one warning.
    fn reload_server(&self) {
        let Some(reload) = &self.server_reload else { return };
        if !reload.ready.load(Ordering::Acquire) {
            return;
        }
        let Ok(mut state) = self.server.state.try_lock() else { return };
        if !reload.ready.swap(false, Ordering::Acquire) {
            return;
        }
        let Some(bytes) = reload.bytes.lock().unwrap_or_else(|error| error.into_inner()).take() else {
            return;
        };
        let commands = self.server_source.as_ref().is_some_and(|source| source.commands);
        match load_server(&ModuleSource { id: &self.id, bytes: &bytes, commands, gpu: false }) {
            Ok(module) => {
                for (word, fresh) in self.masks[Side::Server as usize].iter().zip(mask_of(|name| module.subscribed(name))) {
                    word.store(fresh.load(Ordering::Relaxed), Ordering::Relaxed);
                }
                state.module = Some(module);
                state.reported = false;
                *self.server_latest.lock().unwrap_or_else(|error| error.into_inner()) = Some(Arc::new(bytes));
                log::info!(
                    "wasm mod {} (server): reloaded at a tick boundary (the guest's own state starts afresh; the mobs keep theirs)",
                    self.id
                );
            }
            Err(error) => log::warn!(
                "wasm mod {} (server) not reloaded, the old instance keeps running: {error}",
                self.id
            ),
        }
    }

    pub(super) fn with_client_reload(mut self, reload: Option<Arc<Reload>>) -> Self {
        self.client_reload = reload;
        self
    }

    fn reload_client(&self) {
        reload_client(
            &self.id,
            &self.client,
            &self.masks,
            self.client_reload.as_deref(),
            self.gpu_owner,
            &self.gpu_version,
        );
    }

    /// Whether this package takes part in the render pass.
    fn render_wanted(&self) -> bool {
        self.reload_client();
        RENDER_HOOKS.iter().any(|hook| {
            self.masks[Side::Client as usize][hook / 64].load(Ordering::Relaxed) >> (hook % 64) & 1
                != 0
        })
    }

    /// How many hooks of `side` were dropped because the module was busy.
    #[allow(dead_code)]
    pub(super) fn dropped_hooks(&self, side: Side) -> u64 {
        self.dropped[side as usize].load(Ordering::Relaxed)
    }

    /// A hook found the module busy: its answer is the hook's fallback, and the drop is counted.
    fn busy(&self, side: Side, hook: usize) {
        if self.dropped[side as usize].fetch_add(1, Ordering::Relaxed) == 0 {
            let name = yoake_wasm::FULL_HOOK_LIST
                .get(hook)
                .map_or("?", |(name, _)| *name);
            log::warn!(
                "wasm mod {} ({}): hook {name} dropped, the module was busy or re-entered; the hook's fallback applies (further drops are only counted)",
                self.id,
                side_name(side)
            );
        }
    }

    /// Starts the server module afresh for every world (the server runs one world at
    /// a time and a guest keeps its state in its instance): called as a world's server starts.
    fn reset_server(&self, world: Option<&std::path::Path>) {
        super::package::bind_world(&self.id, world);
        let Some(source) = &self.server_source else { return };
        let Ok(mut state) = self.server.state.try_lock() else { return };
        self.server.dirty.store(false, Ordering::Relaxed);
        let latest = self.server_latest.lock().unwrap_or_else(|error| error.into_inner()).clone();
        let module = match &latest {
            Some(bytes) => ModuleSource { bytes, ..source.module() },
            None => source.module(),
        };
        match load_server(&module) {
            Ok(module) => {
                for (word, fresh) in self.masks[Side::Server as usize]
                    .iter()
                    .zip(mask_of(|name| module.subscribed(name)))
                {
                    word.store(fresh.load(Ordering::Relaxed), Ordering::Relaxed);
                }
                state.module = Some(module);
                state.reported = false;
                log::info!(
                    "wasm mod {} (server): started afresh for the new world",
                    self.id
                );
            }
            Err(error) => {
                state.module = None;
                self.disable(Side::Server);
                log::error!(
                    "wasm mod {} (server) could not start afresh and is off: {error}",
                    self.id
                );
            }
        }
    }

    /// Whether this module's `side` instance subscribed to the hook with this catalog ordinal.
    #[inline]
    fn wants(&self, side: Side, hook: usize) -> bool {
        // F4M-10: the script hooks of custom mobs reach the WASM modules through `mob_runtime` (which also
        // gives native mods the hooks through `mod_api::mods()`); the adapter of the module stays silent so a
        // module is never called twice.
        if (H_MOB_GOAL_CAN_USE..=H_SYNCED_CHANGED_CLIENT).contains(&hook) {
            return false;
        }
        if side == Side::Client {
            self.reload_client();
        } else if hook == H_SERVER_TICK {
            self.reload_server();
        }
        self.masks[side as usize][hook / 64].load(Ordering::Relaxed) >> (hook % 64) & 1 != 0
    }

    fn disable(&self, side: Side) {
        super::package::abort_tickets(
            &self.id,
            match side {
                Side::Server => horizon_mods::yak::ConfigRuntimeSide::Server,
                Side::Client => horizon_mods::yak::ConfigRuntimeSide::Client,
            },
        );
        for word in &self.masks[side as usize] {
            word.store(0, Ordering::Relaxed);
        }
        if matches!(side, Side::Client) {
            if let Some(owner) = self.gpu_owner {
                crate::gfx::mod_gpu_runtime::disable(owner, "WASM client disabled");
            }
        }
    }

    fn with_client<'call, R>(
        &self,
        hook: usize,
        host: HostAccess<'call>,
        batch: Option<&'call mut HudBatch>,
        client: ClientExtras<'call>,
        call: impl FnOnce(&mut ClientFullMod) -> Option<R>,
    ) -> Option<R> {
        self.reload_client();
        let mut state = match self.client.try_lock() {
            Ok(state) => state,
            Err(TryLockError::WouldBlock) => {
                self.busy(Side::Client, hook);
                return None;
            }
            Err(TryLockError::Poisoned(_)) => {
                self.disable(Side::Client);
                log::error!("wasm mod {} disabled: poisoned client instance", self.id);
                return None;
            }
        };
        let module = state.module.as_mut()?;
        let batch_start = batch.as_deref().map_or(0, |batch| batch.vertices.len());
        let mut invocation = Invocation {
            host,
            batch,
            batch_start,
            client,
        };
        let result = {
            let _scope = Scope::enter(&mut invocation);
            call(module)
        };
        if module.is_disabled() {
            self.disable(Side::Client);
            if !state.reported {
                state.reported = true;
                log::error!(
                    "wasm mod {} (client) disabled: trapped or exceeded its limits (see the line above)",
                    self.id
                );
            }
        }
        result
    }

    fn with_server<'call, R>(
        &self,
        hook: usize,
        host: HostAccess<'call>,
        batch: Option<&'call mut HudBatch>,
        call: impl FnOnce(&mut ServerFullMod) -> Option<R>,
    ) -> Option<R> {
        let mut state = match self.server.state.try_lock() {
            Ok(state) => state,
            Err(TryLockError::WouldBlock) => {
                self.busy(Side::Server, hook);
                return None;
            }
            Err(TryLockError::Poisoned(_)) => {
                self.disable(Side::Server);
                log::error!("wasm mod {} disabled: poisoned server instance", self.id);
                return None;
            }
        };
        let module = state.module.as_mut()?;
        self.server.dirty.store(true, Ordering::Relaxed);
        let batch_start = batch.as_deref().map_or(0, |batch| batch.vertices.len());
        let mut invocation = Invocation {
            host,
            batch,
            batch_start,
            client: ClientExtras::default(),
        };
        let result = {
            let _scope = Scope::enter(&mut invocation);
            call(module)
        };
        if module.is_disabled() {
            self.disable(Side::Server);
            if !state.reported {
                state.reported = true;
                log::error!(
                    "wasm mod {} (server) disabled: trapped or exceeded its limits (see the line above)",
                    self.id
                );
            }
        }
        result
    }
}

impl Drop for FullMod {
    fn drop(&mut self) {
        if let Some(owner) = self.gpu_owner {
            crate::gfx::mod_gpu_runtime::remove(owner);
        }
    }
}

fn reload_client(
    id: &str,
    client: &Mutex<SideState<ClientFullMod>>,
    masks: &[[AtomicU64; MASK_WORDS]; 2],
    reload: Option<&Reload>,
    owner: Option<crate::gfx::mod_gpu::ModId>,
    version: &str,
) {
    let Some(reload) = reload else { return };
    if !reload.ready.load(Ordering::Acquire) {
        return;
    }
    let Ok(mut state) = client.try_lock() else {
        return;
    };
    if !reload.ready.swap(false, Ordering::Acquire) {
        return;
    }
    let Some(bytes) = reload
        .bytes
        .lock()
        .unwrap_or_else(|error| error.into_inner())
        .take()
    else {
        return;
    };
    match load_client_with_shaders(
        &ModuleSource {
            id,
            bytes: &bytes,
            commands: false,
            gpu: owner.is_some(),
        },
        reload.shaders.clone(),
    ) {
        Ok(module) => {
            #[cfg(test)]
            let module = {
                let mut module = module;
                if let Some(limits) = reload.test_limits {
                    module.set_test_limits(limits);
                }
                module
            };
            if let Some(owner) = owner {
                let fingerprint = super::gpu_fingerprint(version, &bytes);
                if let Err(error) =
                    crate::gfx::mod_gpu_runtime::replace_package(owner, &fingerprint)
                {
                    log::warn!(
                        "wasm mod {id} (client): reload rejected; keeping the previous module: {error}"
                    );
                    return;
                }
            }
            for (word, fresh) in masks[Side::Client as usize]
                .iter()
                .zip(mask_of(|name| module.subscribed(name)))
            {
                word.store(fresh.load(Ordering::Relaxed), Ordering::Relaxed);
            }
            state.module = Some(module);
            state.reported = false;
            log::info!("wasm mod {id} (client): reloaded successfully");
        }
        Err(error) => log::warn!(
            "wasm mod {id} (client): reload rejected; keeping the previous module: {error}"
        ),
    }
}

// ---------------------------------------------------------------------------------------------
// Value conversions the generated adapters use
// ---------------------------------------------------------------------------------------------

/// Reads a scalar through any number of references.
trait Val {
    type Out;
    fn val(&self) -> Self::Out;
}

macro_rules! val {
    ($($ty:ty),*) => {$(impl Val for $ty { type Out = $ty; #[inline] fn val(&self) -> $ty { *self } })*};
}
val!(
    bool, char, i8, i16, i32, i64, i128, isize, u8, u16, u32, u64, u128, usize, f32, f64
);

impl<T: Val + ?Sized> Val for &T {
    type Out = T::Out;
    #[inline]
    fn val(&self) -> T::Out {
        (**self).val()
    }
}

impl<T: Val + ?Sized> Val for &mut T {
    type Out = T::Out;
    #[inline]
    fn val(&self) -> T::Out {
        (**self).val()
    }
}

static HANDLES: AtomicU64 = AtomicU64::new(1);

/// An opaque id for a live engine object: it only names the object for the length of the call.
fn next_handle() -> u64 {
    HANDLES.fetch_add(1, Ordering::Relaxed)
}

thread_local! {
    /// An encoding of a hook's arguments failed on this thread: the hook is skipped, never called
    /// with an empty stand-in value.
    static ENCODE_FAILED: std::cell::Cell<bool> = const { std::cell::Cell::new(false) };
}

/// Whether an argument of the hook being built failed to encode (and clears the mark).
fn take_encode_failure() -> bool {
    ENCODE_FAILED.with(|failed| failed.replace(false))
}

/// Counted, logged once: a hook skipped for an argument that would not encode.
fn note_encode_skip(hook: usize) {
    static SKIPPED: AtomicU64 = AtomicU64::new(0);
    if SKIPPED.fetch_add(1, Ordering::Relaxed) == 0 {
        let name = yoake_wasm::FULL_HOOK_LIST
            .get(hook)
            .map_or("?", |(name, _)| *name);
        log::warn!(
            "wasm: hook {name} skipped, an NBT argument did not encode (further skips are only counted)"
        );
    }
}

fn nbt_tag(tag: &Tag) -> Vec<u8> {
    convert::tag_to_nbt(tag).unwrap_or_else(|_| {
        ENCODE_FAILED.with(|failed| failed.set(true));
        Vec::new()
    })
}

fn nbt_compound(compound: &Compound) -> Vec<u8> {
    convert::compound_to_nbt(compound).unwrap_or_else(|_| {
        ENCODE_FAILED.with(|failed| failed.set(true));
        Vec::new()
    })
}

fn nbt_tag_from(bytes: &[u8]) -> Option<Tag> {
    convert::nbt_to_tag(bytes).ok()
}

pub(super) fn nbt_compound_from(bytes: &[u8]) -> Option<Compound> {
    convert::nbt_to_compound(bytes).ok()
}

fn text_json(text: &Text) -> String {
    convert::text_to_json(text)
}

fn text_from(json: &str) -> Option<Text> {
    convert::text_from_json(json).ok()
}

fn json_text(value: &serde_json::Value) -> String {
    convert::value_to_json(value)
}

fn json_from(json: &str) -> Option<serde_json::Value> {
    convert::value_from_json(json).ok()
}

fn json_map_text(map: &serde_json::Map<String, serde_json::Value>) -> String {
    serde_json::Value::Object(map.clone()).to_string()
}

fn json_map_from(json: &str) -> Option<serde_json::Map<String, serde_json::Value>> {
    match convert::value_from_json(json).ok()? {
        serde_json::Value::Object(map) => Some(map),
        _ => None,
    }
}

pub(super) fn item_w(stack: &ItemStack) -> et::ItemStack {
    match convert::item_to_wire(stack) {
        Ok(wire) => et::ItemStack {
            id: wire.id,
            count: wire.count,
            components: wire.components,
        },
        Err(_) => et::ItemStack {
            id: String::new(),
            count: 0,
            components: Vec::new(),
        },
    }
}

/// Largest string a guest's answer may carry, in bytes.
const MAX_ANSWER_STRING: usize = 64 * 1024;
/// Most elements of a list or map a guest's answer may carry.
const MAX_ANSWER_LIST: usize = 65_536;
/// Longest item id, and largest component blob, of a guest's item stack.
const MAX_ITEM_ID: usize = 256;
const MAX_ITEM_COMPONENTS: usize = 64 * 1024;
/// The most any stack may hold (vanilla's `max_stack_size` component tops out here); a registered
/// item's own limit applies when the item table is known.
const MAX_STACK_COUNT: u32 = 99;

fn finite_f32(value: f32) -> Option<f32> {
    value.is_finite().then_some(value)
}

fn finite_f64(value: f64) -> Option<f64> {
    value.is_finite().then_some(value)
}

fn bounded_string(text: String) -> Option<String> {
    (text.len() <= MAX_ANSWER_STRING).then_some(text)
}

/// Whether `id` is a syntactically valid `namespace:path` resource location of reasonable length.
pub(super) fn valid_item_id(id: &str) -> bool {
    let Some((namespace, path)) = id.split_once(':') else {
        return false;
    };
    id.len() <= MAX_ITEM_ID
        && !namespace.is_empty()
        && !path.is_empty()
        && namespace.bytes().all(|b| {
            b.is_ascii_lowercase() || b.is_ascii_digit() || matches!(b, b'_' | b'-' | b'.')
        })
        && path.bytes().all(|b| {
            b.is_ascii_lowercase() || b.is_ascii_digit() || matches!(b, b'_' | b'-' | b'.' | b'/')
        })
}

/// A guest's item stack is only accepted when its id is a registered item (when the table is
/// known) and its count is between 1 and the item's stack limit.
fn valid_item_stack(id: &str, count: u32, components: usize) -> bool {
    if !valid_item_id(id) || components > MAX_ITEM_COMPONENTS || count == 0 {
        return false;
    }
    match horizon_items::items::published_stack_limit(id) {
        Some(None) => false,
        Some(Some(limit)) => count <= limit,
        None => count <= MAX_STACK_COUNT,
    }
}

/// A block state id the registry has (a guest's answer may name no other).
fn valid_block_state(id: u32) -> bool {
    id < horizon_client_render::chunk::block_state_limit()
}

pub(super) fn item_from(stack: &et::ItemStack) -> Option<ItemStack> {
    if !valid_item_stack(&stack.id, stack.count, stack.components.len()) {
        return None;
    }
    convert::item_from_wire(&convert::WireItemStack {
        id: stack.id.clone(),
        count: stack.count,
        components: stack.components.clone(),
    })
    .ok()
}

/// Longest string, most strings and most bytes in all that a guest can make the process keep.
const LEAK_MAX_LEN: usize = 256;
const LEAK_MAX_COUNT: usize = 4096;
const LEAK_MAX_BYTES: usize = 256 * 1024;

/// A string that lives for the rest of the process, deduplicated (for answers the engine keeps as
/// `&'static str`, such as command names). Bounded: a string of printable characters up to 256
/// bytes, at most 4096 distinct strings and 256 KiB in all; anything past that is refused (and the
/// guest's answer with it).
fn leak_static(text: String) -> Option<&'static str> {
    static POOL: Mutex<Option<(HashSet<&'static str>, usize)>> = Mutex::new(None);
    if text.len() > LEAK_MAX_LEN || text.chars().any(char::is_control) {
        return None;
    }
    let mut pool = POOL.lock().unwrap_or_else(|error| error.into_inner());
    let (set, bytes) = pool.get_or_insert_with(|| (HashSet::new(), 0));
    if let Some(existing) = set.get(text.as_str()) {
        return Some(existing);
    }
    if set.len() >= LEAK_MAX_COUNT || *bytes + text.len() > LEAK_MAX_BYTES {
        return None;
    }
    *bytes += text.len();
    let leaked: &'static str = Box::leak(text.into_boxed_str());
    set.insert(leaked);
    Some(leaked)
}

// ---------------------------------------------------------------------------------------------
// Applying command lists
// ---------------------------------------------------------------------------------------------

fn gui_scalar(value: f32) -> f32 {
    if value.is_finite() { value } else { 0.0 }
}

fn mix(from: u32, to: u32, t: f32) -> u32 {
    let channel = |shift: u32| -> u32 {
        let a = ((from >> shift) & 0xff) as f32;
        let b = ((to >> shift) & 0xff) as f32;
        (a + (b - a) * t).round().clamp(0.0, 255.0) as u32
    };
    channel(24) << 24 | channel(16) << 16 | channel(8) << 8 | channel(0)
}

enum Pending {
    Clip([f32; 4]),
    Scale([f32; 4]),
    Blend(crate::hud::Blend),
}

/// Applies a guest's HUD commands to `batch` through the game's own draw services. Commands that
/// have no service (textures registered by another API, text highlights) are skipped.
pub(super) fn apply_hud(
    host: Option<&dyn Host>,
    batch: &mut HudBatch,
    commands: &[et::HudCommand],
) {
    let Some(host) = host else { return };
    let mut pending: Vec<(Pending, usize)> = Vec::new();
    for command in commands.iter().take(MAX_COMMANDS) {
        match command {
            et::HudCommand::Text(text) => {
                let line = text_from(&text.text)
                    .map(|text| text.plain())
                    .unwrap_or_default();
                if line.len() <= abi::MAX_TEXT_BYTES {
                    host.text(
                        batch,
                        &line,
                        [gui_scalar(text.x), gui_scalar(text.y)],
                        text.color & 0x00ff_ffff,
                        text.shadow,
                    );
                }
            }
            et::HudCommand::Rect(rect) => {
                host.fill(
                    batch,
                    [
                        gui_scalar(rect.x),
                        gui_scalar(rect.y),
                        gui_scalar(rect.width),
                        gui_scalar(rect.height),
                    ],
                    rect.color,
                );
            }
            et::HudCommand::Gradient(gradient) => {
                let (x, y, w, h) = (
                    gui_scalar(gradient.x),
                    gui_scalar(gradient.y),
                    gui_scalar(gradient.width),
                    gui_scalar(gradient.height),
                );
                // The game's fill has no gradient service: strips of at most 2 pixels.
                let strips = (h / 2.0).ceil().clamp(1.0, 128.0) as u32;
                let step = h / strips as f32;
                for index in 0..strips {
                    let t = if strips == 1 {
                        0.0
                    } else {
                        index as f32 / (strips - 1) as f32
                    };
                    host.fill(
                        batch,
                        [x, y + step * index as f32, w, step.max(1.0)],
                        mix(gradient.top, gradient.bottom, t),
                    );
                }
            }
            et::HudCommand::Quad(quad) => {
                if quad.corners.len() == 4 {
                    let p =
                        |i: usize| [gui_scalar(quad.corners[i].0), gui_scalar(quad.corners[i].1)];
                    let corners = [p(0), p(1), p(2), p(3)];
                    match quad.sprite.as_deref() {
                        None => host.quad(batch, corners, quad.color),
                        Some(sprite) => draw_sprite_quad(host, batch, sprite, corners, quad),
                    }
                }
            }
            et::HudCommand::Sprite(sprite) => {
                let (width, height) = sprite.size.unwrap_or((16.0, 16.0));
                let src = match &sprite.region {
                    Some(region) => [region.x, region.y, region.width, region.height],
                    None => [0.0, 0.0, gui_scalar(width), gui_scalar(height)],
                };
                host.sprite(
                    batch,
                    &sprite.name,
                    src,
                    [
                        gui_scalar(sprite.x),
                        gui_scalar(sprite.y),
                        gui_scalar(width),
                        gui_scalar(height),
                    ],
                );
            }
            et::HudCommand::Item(item) => {
                if let Some(stack) = item_from(&item.stack) {
                    host.item_stack(
                        batch,
                        &stack,
                        [gui_scalar(item.x), gui_scalar(item.y)],
                        16.0,
                    );
                }
            }
            et::HudCommand::Texture(texture) => {
                // The id a mod got from `gui_texture_id`; an unknown id draws nothing.
                if let Some(handle) = client_services::named_texture(&texture.id) {
                    host.gui_texture(
                        batch,
                        &handle,
                        [
                            gui_scalar(texture.x),
                            gui_scalar(texture.y),
                            gui_scalar(texture.width),
                            gui_scalar(texture.height),
                        ],
                        [0.0, 0.0, 1.0, 1.0],
                        texture.color,
                        None,
                    );
                }
            }
            et::HudCommand::Highlight(highlight) => draw_highlight(host, batch, highlight),
            et::HudCommand::BeginClip(bounds) => {
                pending.push((
                    Pending::Clip([bounds.x0, bounds.y0, bounds.x1, bounds.y1]),
                    batch.vertices.len(),
                ));
            }
            et::HudCommand::BeginScale(scale) => {
                pending.push((
                    Pending::Scale([scale.center_x, scale.center_y, scale.scale_x, scale.scale_y]),
                    batch.vertices.len(),
                ));
            }
            et::HudCommand::BeginBlend(blend) => {
                let blend = match blend {
                    et::HudBlend::Alpha => crate::hud::Blend::Alpha,
                    et::HudBlend::Invert => crate::hud::Blend::Invert,
                    et::HudBlend::Additive => crate::hud::Blend::Additive,
                    et::HudBlend::Vignette => crate::hud::Blend::Vignette,
                };
                pending.push((Pending::Blend(blend), batch.vertices.len()));
            }
            et::HudCommand::EndClip | et::HudCommand::EndScale | et::HudCommand::EndBlend => {
                let Some((run, start)) = pending.pop() else {
                    continue;
                };
                match (command, run) {
                    (et::HudCommand::EndClip, Pending::Clip(b)) => {
                        batch.clip_since(start, b[0], b[1], b[2], b[3])
                    }
                    (et::HudCommand::EndScale, Pending::Scale(s)) => {
                        batch.scale_since(start, s[0], s[1], s[2], s[3])
                    }
                    (et::HudCommand::EndBlend, Pending::Blend(blend)) => {
                        batch.blend_since(start, blend)
                    }
                    // A mismatched end closes nothing.
                    _ => {}
                }
            }
        }
    }
}

/// REM-12: the HUD commands of a full-access (F) module as the S route's wire commands.
pub(super) fn f_hud_commands(commands: &[crate::game::yoake2_f_services::FHud]) -> Vec<et::HudCommand> {
    use crate::game::yoake2_f_services::{FBlend, FHud};
    commands
        .iter()
        .map(|command| match command {
            FHud::Text { text, x, y, color, shadow } => et::HudCommand::Text(et::HudText {
                text: text_json(&Text::literal(text.clone())),
                x: *x,
                y: *y,
                color: *color,
                shadow: *shadow,
            }),
            FHud::Rect { x, y, width, height, color } => et::HudCommand::Rect(et::HudRect { x: *x, y: *y, width: *width, height: *height, color: *color }),
            FHud::Gradient { x, y, width, height, top, bottom } => et::HudCommand::Gradient(et::HudGradient {
                x: *x,
                y: *y,
                width: *width,
                height: *height,
                top: *top,
                bottom: *bottom,
            }),
            FHud::Quad { corners, sprite, uv, color } => et::HudCommand::Quad(et::HudQuad {
                corners: corners.clone(),
                sprite: sprite.clone(),
                uv: uv.clone(),
                color: *color,
            }),
            FHud::Sprite { name, x, y, size, color, fraction, region } => et::HudCommand::Sprite(et::HudSprite {
                name: name.clone(),
                x: *x,
                y: *y,
                size: *size,
                color: *color,
                fraction: *fraction,
                region: region.as_ref().map(|r| et::HudRegion {
                    x: r.x,
                    y: r.y,
                    width: r.width,
                    height: r.height,
                    texture_width: r.texture_width,
                    texture_height: r.texture_height,
                }),
            }),
            FHud::Item { item, count, x, y, decorations } => et::HudCommand::Item(et::HudItem {
                stack: et::ItemStack { id: item.clone(), count: *count, components: Vec::new() },
                x: *x,
                y: *y,
                decorations: *decorations,
            }),
            FHud::Texture { id, x, y, width, height, color } => et::HudCommand::Texture(et::HudTexture {
                id: id.clone(),
                x: *x,
                y: *y,
                width: *width,
                height: *height,
                color: *color,
            }),
            FHud::Highlight { x, y, width, height, invert_text } => et::HudCommand::Highlight(et::HudHighlight {
                x: *x,
                y: *y,
                width: *width,
                height: *height,
                invert_text: *invert_text,
            }),
            FHud::BeginClip { x0, y0, x1, y1 } => et::HudCommand::BeginClip(et::HudBounds { x0: *x0, y0: *y0, x1: *x1, y1: *y1 }),
            FHud::EndClip => et::HudCommand::EndClip,
            FHud::BeginScale { center_x, center_y, scale_x, scale_y } => et::HudCommand::BeginScale(et::HudScale {
                center_x: *center_x,
                center_y: *center_y,
                scale_x: *scale_x,
                scale_y: *scale_y,
            }),
            FHud::EndScale => et::HudCommand::EndScale,
            FHud::BeginBlend(blend) => et::HudCommand::BeginBlend(match blend {
                FBlend::Alpha => et::HudBlend::Alpha,
                FBlend::Invert => et::HudBlend::Invert,
                FBlend::Additive => et::HudBlend::Additive,
                FBlend::Vignette => et::HudBlend::Vignette,
            }),
            FHud::EndBlend => et::HudCommand::EndBlend,
        })
        .collect()
}

/// REM-12: draws a full-access module's HUD commands with the S route's code ([`apply_hud`]).
pub(crate) fn apply_f_hud(host: Option<&dyn Host>, batch: &mut HudBatch, commands: &[crate::game::yoake2_f_services::FHud]) {
    apply_hud(host, batch, &f_hud_commands(commands));
}

/// Logs the first time each kind of model command (cube, item, model, skeleton) is drawn, once per process.
fn note_applied(slot: usize, kind: &str) {
    static SEEN: [std::sync::atomic::AtomicBool; 4] = [const { std::sync::atomic::AtomicBool::new(false) }; 4];
    if !SEEN[slot].swap(true, Ordering::Relaxed) {
        log::info!("wasm: model command {kind} applied");
    }
}

/// Colour of a highlight's tint and of its outline (ARGB).
const HIGHLIGHT_TINT: u32 = 0x6633_77ff;
const HIGHLIGHT_OUTLINE: u32 = 0xff77_aaff;

/// A `highlight` command: the rectangle is tinted and outlined one pixel wide; with `invert_text`
/// the tint is the inverting blend instead (what is under the rectangle, text included, turns
/// inverted, as a selection in a text field). An empty rectangle draws nothing.
fn draw_highlight(host: &dyn Host, batch: &mut HudBatch, highlight: &et::HudHighlight) {
    let (x, y, w, h) = (
        gui_scalar(highlight.x),
        gui_scalar(highlight.y),
        gui_scalar(highlight.width),
        gui_scalar(highlight.height),
    );
    if w <= 0.0 || h <= 0.0 {
        return;
    }
    if highlight.invert_text {
        let start = batch.vertices.len();
        host.fill(batch, [x, y, w, h], 0xffff_ffff);
        batch.blend_since(start, crate::hud::Blend::Invert);
    } else {
        host.fill(batch, [x, y, w, h], HIGHLIGHT_TINT);
    }
    let edge = 1.0f32.min(w).min(h);
    let inner = (h - 2.0 * edge).max(0.0);
    for rect in [
        [x, y, w, edge],
        [x, y + h - edge, w, edge],
        [x, y + edge, edge, inner],
        [x + w - edge, y + edge, edge, inner],
    ] {
        if rect[2] > 0.0 && rect[3] > 0.0 {
            host.fill(batch, rect, HIGHLIGHT_OUTLINE);
        }
    }
}

/// A model id a guest may name: `namespace:path` (or a path), printable and bounded.
fn valid_model_id(id: &str) -> bool {
    !id.is_empty()
        && id.len() <= 128
        && id
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || matches!(c, ':' | '_' | '-' | '/' | '.'))
}

/// The cube of a guest's command as the renderer takes it: box corners in texels and one optional
/// face per direction (down, up, north, south, west, east). A non-finite corner, a UV list that is
/// not four finite numbers or more than six faces make it draw nothing.
type CubeParts<'a> = ([f32; 3], [f32; 3], [Option<CubeFace<'a>>; 6]);

pub(super) fn cube_from(cube: &et::ModelCube) -> Option<CubeParts<'_>> {
    let from = [cube.min.0, cube.min.1, cube.min.2];
    let to = [cube.max.0, cube.max.1, cube.max.2];
    if from.iter().chain(&to).any(|v| !v.is_finite()) || cube.faces.len() > 6 {
        return None;
    }
    let mut faces: [Option<CubeFace<'_>>; 6] = Default::default();
    for (slot, face) in faces.iter_mut().zip(&cube.faces) {
        let Some(face) = face else { continue };
        let uv = match face.uv.as_deref() {
            None => None,
            Some(&[a, b, c, d]) if [a, b, c, d].iter().all(|v| v.is_finite()) => {
                Some([a, b, c, d])
            }
            Some(_) => return None,
        };
        *slot = Some(CubeFace {
            sprite: &face.sprite,
            uv,
        });
    }
    Some((from, to, faces))
}

fn display_from(display: et::DisplayContext) -> DisplayContext {
    match display {
        et::DisplayContext::None => DisplayContext::None,
        et::DisplayContext::ThirdPersonLeftHand => DisplayContext::ThirdPersonLeftHand,
        et::DisplayContext::ThirdPersonRightHand => DisplayContext::ThirdPersonRightHand,
        et::DisplayContext::FirstPersonLeftHand => DisplayContext::FirstPersonLeftHand,
        et::DisplayContext::FirstPersonRightHand => DisplayContext::FirstPersonRightHand,
        et::DisplayContext::Head => DisplayContext::Head,
        et::DisplayContext::Gui => DisplayContext::Gui,
        et::DisplayContext::Ground => DisplayContext::Ground,
        et::DisplayContext::Fixed => DisplayContext::Fixed,
        et::DisplayContext::OnShelf => DisplayContext::OnShelf,
    }
}

/// A point `s` of the way from `a` to `b`.
fn lerp2(a: [f32; 2], b: [f32; 2], s: f32) -> [f32; 2] {
    [a[0] + (b[0] - a[0]) * s, a[1] + (b[1] - a[1]) * s]
}

/// Where the point `(s, t)` of the unit square lands on a quad whose corners go round it from
/// the top left: along the top edge by `s`, down by `t`. A square turned about a pivot keeps its
/// shape; a squeezed or sheared quad stretches the picture over it.
fn quad_point(corners: &[[f32; 2]; 4], s: f32, t: f32) -> [f32; 2] {
    let top = lerp2(corners[0], corners[1], s);
    let bottom = lerp2(corners[3], corners[2], s);
    lerp2(top, bottom, t)
}

/// Side of the upright square a textured quad is first drawn as (GUI pixels).
const TEXTURE_QUAD_SIDE: f32 = 1024.0;

/// A `quad` command with a sprite: `texture:<id>` names a GUI texture the mod got from
/// `gui_texture_id`, anything else a sprite of the GUI atlas. The corners are in order round the
/// shape from the top left, so a turned quad is the same four corners turned about its pivot.
/// `uv` are the corners' UVs, 0 to 1 over the whole image (none takes its own corners); a texture
/// takes the rectangle between the first and third. An unknown sprite or id draws nothing.
fn draw_sprite_quad(
    host: &dyn Host,
    batch: &mut HudBatch,
    sprite: &str,
    corners: [[f32; 2]; 4],
    quad: &et::HudQuad,
) {
    let uv: [[f32; 2]; 4] = match quad.uv.as_deref() {
        Some([a, b, c, d]) => [[a.0, a.1], [b.0, b.1], [c.0, c.1], [d.0, d.1]],
        _ => [[0.0, 0.0], [1.0, 0.0], [1.0, 1.0], [0.0, 1.0]],
    };
    let Some(id) = sprite.strip_prefix("texture:") else {
        host.sprite_quad(batch, sprite, corners, uv, quad.color);
        return;
    };
    let Some(handle) = client_services::named_texture(id) else {
        return;
    };
    let start = batch.vertices.len();
    let drawn = host.gui_texture(
        batch,
        &handle,
        [0.0, 0.0, TEXTURE_QUAD_SIDE, TEXTURE_QUAD_SIDE],
        [uv[0][0], uv[0][1], uv[2][0], uv[2][1]],
        quad.color,
        None,
    );
    if !drawn {
        return;
    }
    for vertex in &mut batch.vertices[start..] {
        vertex.pos = quad_point(
            &corners,
            vertex.pos[0] / TEXTURE_QUAD_SIDE,
            vertex.pos[1] / TEXTURE_QUAD_SIDE,
        );
    }
}

/// Applies a guest's pose-stack commands to a block entity, mob, player or item draw.
pub(super) fn apply_model(
    host: Option<&dyn Host>,
    draw: &mut BlockEntityDraw,
    commands: &[et::ModelCommand],
) {
    let mut depth = 0usize;
    for command in commands.iter().take(MAX_COMMANDS) {
        match command {
            et::ModelCommand::Push => {
                if depth < 64 {
                    depth += 1;
                    draw.push();
                }
            }
            et::ModelCommand::Pop => {
                if depth > 0 {
                    depth -= 1;
                    draw.pop();
                }
            }
            et::ModelCommand::Translate((x, y, z)) => draw.translate([*x, *y, *z]),
            et::ModelCommand::Scale((x, y, z)) => draw.scale([*x, *y, *z]),
            et::ModelCommand::RotateAbout(rotation) => {
                let (a, p) = (rotation.axis, rotation.pivot);
                draw.rotate_about([a.0, a.1, a.2], rotation.degrees, [p.0, p.1, p.2]);
            }
            et::ModelCommand::SetLight(light) => draw.light = *light,
            et::ModelCommand::Keep => draw.keep(),
            et::ModelCommand::Quad(quad) => {
                let Some(host) = host else { continue };
                if quad.corners.len() == 4 && quad.uv.len() == 4 {
                    let c = |i: usize| [quad.corners[i].0, quad.corners[i].1, quad.corners[i].2];
                    host.draw_quad(
                        draw,
                        &quad.sprite,
                        [c(0), c(1), c(2), c(3)],
                        [quad.uv[0], quad.uv[1], quad.uv[2], quad.uv[3]],
                        quad.tint,
                    );
                }
            }
            et::ModelCommand::Block(block) => {
                let Some(host) = host else { continue };
                let properties: Vec<(&str, &str)> = block
                    .properties
                    .iter()
                    .take(64)
                    .map(|(k, v)| (k.as_str(), v.as_str()))
                    .collect();
                host.draw_block(draw, &block.block, &properties);
            }
            et::ModelCommand::Cube(cube) => {
                let Some(host) = host else { continue };
                if let Some((from, to, faces)) = cube_from(cube) {
                    if host.draw_cube(draw, from, to, faces, cube.tint & 0x00ff_ffff) {
                        note_applied(0, "cube");
                    }
                }
            }
            et::ModelCommand::Item(item) => {
                let Some(host) = host else { continue };
                let Some(stack) = item_from(&item.stack) else {
                    continue;
                };
                let yaw = item.owner_yaw.filter(|yaw| yaw.is_finite());
                if host.draw_item(draw, &stack, display_from(item.display), yaw, item.seed) {
                    note_applied(1, "item");
                }
            }
            // A model by id: a standalone model a mod named in `standalone-models` (`draw_model`, its
            // tint masked to RGB), else, as before, a block (its baked model, default state) or an
            // item (drawn `Fixed`, as an item frame does).
            et::ModelCommand::Model(model) => {
                let Some(host) = host else { continue };
                if !valid_model_id(&model.model) {
                    continue;
                }
                if host.draw_model(draw, &model.model, model.tint & 0x00ff_ffff)
                    || host.draw_block(draw, &model.model, &[])
                    || host.draw_item(
                        draw,
                        &ItemStack::new(&model.model, 1),
                        DisplayContext::Fixed,
                        None,
                        0,
                    )
                {
                    note_applied(2, "model");
                }
            }
        }
    }
    while depth > 0 {
        depth -= 1;
        draw.pop();
    }
}

fn validate_skeleton_layer(layer: &str) -> Result<(), String> {
    if layer.is_empty() || layer.len() > 128 || layer.chars().any(char::is_control) {
        return Err("texture layer is 1 to 128 printable characters".into());
    }
    Ok(())
}

fn finite_script_vec3(v: &yoake_wasm::client_types::ScriptVec3) -> Result<crate::mobs::custom::script::ScriptVec3, String> {
    if !v.x.is_finite() || !v.y.is_finite() || !v.z.is_finite() {
        return Err("numbers must be finite".into());
    }
    Ok(crate::mobs::custom::script::ScriptVec3 { x: v.x, y: v.y, z: v.z })
}

/// Validated fields of a WASM `model-skeleton` payload.
pub(super) struct ParsedSkeleton {
    pub(super) model: String,
    pub(super) pose: Vec<crate::mobs::custom::script::MobBoneWrite>,
    pub(super) texture_layer: Option<String>,
    pub(super) entity: Option<u32>,
}

/// Validates and converts a WASM `model-skeleton` payload into engine bone writes.
pub(super) fn parse_skeleton(
    skeleton: &yoake_wasm::client_types::ModelSkeleton,
) -> Result<ParsedSkeleton, String> {
    if !valid_model_id(&skeleton.model) {
        return Err("model id is a namespace:path id of at most 128 characters".into());
    }
    if let Some(layer) = skeleton.texture_layer.as_deref() {
        validate_skeleton_layer(layer)?;
    }
    if skeleton.pose.len() > crate::mobs::custom::script::MAX_POSE_WRITES {
        return Err(format!(
            "skeleton pose has at most {} bone writes",
            crate::mobs::custom::script::MAX_POSE_WRITES
        ));
    }
    let mut writes = Vec::with_capacity(skeleton.pose.len());
    for w in &skeleton.pose {
        writes.push(crate::mobs::custom::script::MobBoneWrite {
            bone: w.bone,
            additive: w.additive,
            translation: w.translation.as_ref().map(finite_script_vec3).transpose()?,
            rotation_degrees: w.rotation_degrees.as_ref().map(finite_script_vec3).transpose()?,
            scale: w.scale.as_ref().map(finite_script_vec3).transpose()?,
            visible: w.visible,
        });
    }
    Ok(ParsedSkeleton {
        model: skeleton.model.clone(),
        pose: writes,
        texture_layer: skeleton.texture_layer.clone(),
        entity: skeleton.entity,
    })
}

/// Draws a custom mob's skeleton `model` posed by `pose`, optional `texture_layer`, and optional
/// runtime texture `entity` under `draw`.
pub(super) fn apply_skeleton(
    host: Option<&dyn Host>,
    draw: &mut BlockEntityDraw,
    model: &str,
    pose: &[crate::mobs::custom::script::MobBoneWrite],
    texture_layer: Option<&str>,
    entity: Option<u32>,
) -> bool {
    let Some(host) = host else { return false };
    if !valid_model_id(model) {
        return false;
    }
    if let Some(layer) = texture_layer
        && validate_skeleton_layer(layer).is_err()
    {
        return false;
    }
    let finite_vec = |v: Option<crate::mobs::custom::script::ScriptVec3>| {
        v.is_none_or(|p| p.x.is_finite() && p.y.is_finite() && p.z.is_finite())
    };
    if pose.len() > crate::mobs::custom::script::MAX_POSE_WRITES
        || pose
            .iter()
            .any(|w| !finite_vec(w.translation) || !finite_vec(w.rotation_degrees) || !finite_vec(w.scale))
    {
        return false;
    }
    if host.draw_skeleton(draw, model, pose, texture_layer, entity) {
        note_applied(3, "skeleton");
        true
    } else {
        false
    }
}

/// Applies a guest's corner reservations.
pub(super) fn apply_corners(corners: &mut Corners, commands: &[et::CornerCommand]) {
    for command in commands.iter().take(MAX_COMMANDS) {
        let et::CornerCommand::Reserve(reserve) = command;
        let corner = match reserve.corner {
            et::HudCorner::TopLeft => crate::extras::Corner::TopLeft,
            et::HudCorner::TopRight => crate::extras::Corner::TopRight,
            et::HudCorner::BottomLeft => crate::extras::Corner::BottomLeft,
            et::HudCorner::BottomRight => crate::extras::Corner::BottomRight,
        };
        corners.place(
            corner,
            gui_scalar(reserve.width),
            gui_scalar(reserve.height),
        );
    }
}

/// Frame-stage commands (whole-frame fills, tints, vignettes) need the render target, which is only
/// reachable inside the renderer: they are not applied yet.
pub(super) fn apply_frame_stage(commands: &[et::FrameCommand]) {
    static REPORTED: std::sync::atomic::AtomicBool = std::sync::atomic::AtomicBool::new(false);
    if !commands.is_empty() && !REPORTED.swap(true, Ordering::Relaxed) {
        log::warn!(
            "wasm: frame-stage commands are not applied yet (this build has no render-target service)"
        );
    }
}

// ---------------------------------------------------------------------------------------------
// Server services
// ---------------------------------------------------------------------------------------------

/// The server services a full-catalog module may call, forwarded to the game's native services. The
/// ones the game has no counterpart for answer "unsupported" (see `docs/yoake-wasm-dev.md`).
pub(super) struct GameServerHost {
    pub(super) id: String,
    /// The package declared `service.commands`.
    pub(super) commands: bool,
}

thread_local! {
    static SERVER_INIT_OWNER: std::cell::RefCell<Option<String>> = const { std::cell::RefCell::new(None) };
}

pub(super) struct ServerInitScope(Option<String>);

impl ServerInitScope {
    pub(super) fn enter(owner: &str) -> Self {
        Self(SERVER_INIT_OWNER.with(|active| active.replace(Some(owner.to_owned()))))
    }
}

impl Drop for ServerInitScope {
    fn drop(&mut self) {
        SERVER_INIT_OWNER.with(|active| active.replace(self.0.take()));
    }
}

pub(super) fn initializing_server(owner: &str) -> bool {
    SERVER_INIT_OWNER.with(|active| active.borrow().as_deref() == Some(owner))
}

/// A path as the guest may see it: relative to the game folder (the server's working directory), or
/// just its last component when it lies elsewhere. Absolute paths carry the user's account name.
pub(super) fn game_relative(path: &std::path::Path) -> Option<String> {
    let relative = std::env::current_dir()
        .ok()
        .and_then(|game| {
            path.strip_prefix(game)
                .ok()
                .map(std::path::Path::to_path_buf)
        })
        .or_else(|| path.file_name().map(std::path::PathBuf::from))?;
    let text = relative.to_str()?.replace('\\', "/");
    Some(if text.is_empty() {
        ".".to_owned()
    } else {
        text
    })
}

pub(super) fn uuid_text(uuid: u128) -> String {
    let hex = format!("{uuid:032x}");
    format!(
        "{}-{}-{}-{}-{}",
        &hex[0..8],
        &hex[8..12],
        &hex[12..16],
        &hex[16..20],
        &hex[20..32]
    )
}

pub(super) fn parse_uuid(text: &str) -> Result<u128, String> {
    let digits: String = text.chars().filter(|c| *c != '-').collect();
    if digits.len() != 32 {
        return Err(format!("not a player UUID: {text}"));
    }
    u128::from_str_radix(&digits, 16).map_err(|_| format!("not a player UUID: {text}"))
}

pub(super) fn st_player_with(player: &NativeServerPlayer, sneaking: bool) -> st::Player {
    st::Player {
        dimension: player.dimension.clone(),
        game_mode: player.game_mode,
        health: player.health,
        name: player.name.clone(),
        permission_level: player.permission,
        pitch: player.pitch,
        position: st::Vec3 {
            x: player.position[0],
            y: player.position[1],
            z: player.position[2],
        },
        sneaking,
        uuid: uuid_text(player.uuid),
        yaw: player.yaw,
    }
}

pub(super) fn server_service<R>(
    operation: impl FnOnce(&mut dyn Host) -> Result<R, String>,
) -> Result<R, String> {
    service(|invocation| {
        let Some(host) = invocation.host.write() else { return Ok(Err("unsupported: this hook may not change the game (a read-only hook), or this is not inside a server hook".to_owned())) };
        Ok(operation(host))
    })
    .map_err(|error| format!("{error:?}"))?
}

pub(super) fn read_service<R>(
    operation: impl FnOnce(&dyn Host) -> Result<R, String>,
) -> Result<R, String> {
    service(|invocation| {
        let Some(host) = invocation.host.read() else {
            return Ok(Err("unsupported: not inside a server hook".to_owned()));
        };
        Ok(operation(host))
    })
    .map_err(|error| format!("{error:?}"))?
}

pub(super) fn checked(done: bool, what: &str) -> Result<(), String> {
    if done {
        Ok(())
    } else {
        Err(format!("{what} did not apply"))
    }
}

pub(super) fn message_text(message: &str) -> Result<Text, String> {
    if message.len() > abi::MAX_TEXT_BYTES {
        return Err("message too long".into());
    }
    Ok(Text::literal(message))
}

/// The server services that answer "unsupported" in this build (see `docs/yoake-wasm-dev.md`); every other
/// service of `server-services` is forwarded (`server_services.rs`).
pub(super) const UNSUPPORTED_SERVER_SERVICES: &[&str] = &[
    "block_entity_read",
    "block_entity_patch_call",
    "range_override_set",
    "range_override_clear",
];

// ---------------------------------------------------------------------------------------------
// Loading
// ---------------------------------------------------------------------------------------------

/// One module's bytes and identity.
pub(super) struct ModuleSource<'a> {
    pub id: &'a str,
    pub bytes: &'a [u8],
    /// The package declared `service.commands` (server module).
    pub commands: bool,
    pub gpu: bool,
}

#[cfg(test)]
pub(super) fn load_client(source: &ModuleSource<'_>) -> Result<ClientFullMod, String> {
    load_client_with_shaders(source, None)
}

pub(super) fn load_client_with_shaders(
    source: &ModuleSource<'_>,
    shaders: Option<Arc<Mutex<super::dev_shaders::DevShaders>>>,
) -> Result<ClientFullMod, String> {
    let mut host = GameHost::new(source.id).with_gpu(source.gpu);
    host.dev_shaders = shaders;
    load_client_with_host(source, host)
}

fn load_client_with_host(source: &ModuleSource<'_>, host: GameHost) -> Result<ClientFullMod, String> {
    // A full module's `ItemStack` answers are checked against the item table, so it is published.
    horizon_items::items::publish_registry_for_checks();
    let mut sandbox = sandbox().lock().unwrap_or_else(|error| error.into_inner());
    host.gpu_ready.store(false, Ordering::Release);
    let ready = Arc::clone(&host.gpu_ready);
    let module = ClientFullMod::instantiate(
        &mut sandbox,
        source.id,
        source.bytes,
        Box::new(host),
        UNWIRED,
    )
    .map_err(|error| error.to_string())?;
    ready.store(true, Ordering::Release);
    Ok(module)
}

pub(super) fn load_server(source: &ModuleSource<'_>) -> Result<ServerFullMod, String> {
    horizon_items::items::publish_registry_for_checks();
    let mut sandbox = sandbox().lock().unwrap_or_else(|error| error.into_inner());
    let _initialization = ServerInitScope::enter(source.id);
    ServerFullMod::instantiate(&mut sandbox, source.id, source.bytes, Box::new(GameServerHost { id: source.id.to_owned(), commands: source.commands }), UNWIRED)
        .map_err(|error| error.to_string())
}

/// How many hooks have an adapter: all but [`UNWIRED`].
pub(super) fn wired_hooks() -> usize {
    HOOK_COUNT - UNWIRED.len()
}

// Generated by `horizon-mod-api-gen` and committed (`cargo test -p horizon-mod-api-gen --lib
// committed_wasm_adapters` checks it is current; YOAKE_UPDATE_WIT=1 rewrites it).
include!("full_adapters.rs");

#[cfg(test)]
#[path = "full/gpu_tests.rs"]
mod gpu_tests;

#[cfg(test)]
mod tests {
    use super::*;
    use crate::mod_api::AttachmentResult;
    use horizon_world::attachments::{AttachmentError, Attachments, Holder};
    use std::cell::RefCell;
    use yoake_wasm::ServerHost as _;

    struct AttachmentServiceHost {
        attachments: Attachments,
        mutable_reads: usize,
        root: std::path::PathBuf,
    }

    impl AttachmentServiceHost {
        fn new() -> Self {
            let nonce = std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos();
            Self {
                attachments: Attachments::default(),
                mutable_reads: 0,
                root: std::env::temp_dir().join(format!(
                    "yoake-w2bf-services-{}-{nonce}",
                    std::process::id()
                )),
            }
        }
    }

    impl Host for AttachmentServiceHost {
        fn attachments(&mut self, holder: Holder) -> AttachmentResult<&mut Attachments> {
            if holder != Holder::Level {
                return Err(AttachmentError::MissingHolder);
            }
            self.mutable_reads += 1;
            Ok(&mut self.attachments)
        }

        fn attachment_read(&self, holder: Holder) -> AttachmentResult<Option<&Attachments>> {
            if holder != Holder::Level {
                return Err(AttachmentError::MissingHolder);
            }
            Ok(Some(&self.attachments))
        }

        fn world_dir(&self) -> Option<std::path::PathBuf> {
            Some(self.root.join("world"))
        }
        fn config_dir(&self) -> Result<std::path::PathBuf, String> {
            Ok(self.root.join("config"))
        }
    }

    fn with_services<Result>(host: HostAccess<'_>, operation: impl FnOnce() -> Result) -> Result {
        let mut invocation = Invocation {
            host,
            batch: None,
            batch_start: 0,
            client: ClientExtras::default(),
        };
        let _scope = Scope::enter(&mut invocation);
        operation()
    }

    #[test]
    fn attachment_definitions_require_the_owning_server_initialization() {
        let _guard = super::super::server_services::ATTACHMENT_TEST_LOCK
            .lock()
            .unwrap();
        let mut host = GameServerHost {
            id: "cap-init".into(),
            commands: false,
        };
        let define = |host: &mut GameServerHost| {
            host.attachment_define(false, "cap-init:data".into(), "level".into(), true, false)
        };
        assert!(define(&mut host).is_err());
        {
            let _initialization = ServerInitScope::enter("cap-init");
            define(&mut host).unwrap();
            {
                let _other_initialization = ServerInitScope::enter("cap-other");
                assert!(define(&mut host).is_err());
            }
            define(&mut host).unwrap();
        }
        assert!(define(&mut host).is_err());
    }

    #[test]
    fn attachment_calls_enforce_current_hook_access_and_package_ownership() {
        let _guard = super::super::server_services::ATTACHMENT_TEST_LOCK
            .lock()
            .unwrap();
        let mut server = GameServerHost {
            id: "cap-call".into(),
            commands: false,
        };
        {
            let _initialization = ServerInitScope::enter("cap-call");
            server
                .attachment_define(false, "cap-call:data".into(), "level".into(), true, true)
                .unwrap();
        }
        let mut host = AttachmentServiceHost::new();
        let get = |server: &mut GameServerHost| {
            server.attachment_get(String::new(), "cap-call:data".into(), "level".into())
        };
        let set = |server: &mut GameServerHost| {
            server.attachment_set(
                vec![1, 2, 3],
                String::new(),
                "cap-call:data".into(),
                "level".into(),
            )
        };
        let remove = |server: &mut GameServerHost| {
            server.attachment_remove(String::new(), "cap-call:data".into(), "level".into())
        };
        assert!(get(&mut server).is_err());
        assert!(set(&mut server).is_err());
        assert!(remove(&mut server).is_err());
        with_services(HostAccess::None, || {
            assert!(get(&mut server).is_err());
            assert!(set(&mut server).is_err());
            assert!(remove(&mut server).is_err());
        });
        with_services(HostAccess::Read(&host), || {
            assert_eq!(get(&mut server).unwrap(), None);
            assert!(set(&mut server).is_err());
            assert!(remove(&mut server).is_err());
        });
        assert_eq!(host.mutable_reads, 0);
        with_services(HostAccess::Write(&mut host), || {
            set(&mut server).unwrap();
            assert_eq!(get(&mut server).unwrap(), Some(vec![1, 2, 3]));
            assert!(
                server
                    .attachment_set(
                        vec![9],
                        String::new(),
                        "cap-init:data".into(),
                        "level".into()
                    )
                    .is_err()
            );
            assert!(
                server
                    .attachment_get("1,2".into(), "cap-call:data".into(), "chunk".into())
                    .is_err()
            );
        });
        let mutable_reads = host.mutable_reads;
        let before = host.attachments.to_nbt();
        with_services(HostAccess::Read(&host), || {
            assert_eq!(get(&mut server).unwrap(), Some(vec![1, 2, 3]));
            assert!(set(&mut server).is_err());
            assert!(remove(&mut server).is_err());
        });
        assert_eq!(host.mutable_reads, mutable_reads);
        assert_eq!(host.attachments.to_nbt(), before);
        with_services(HostAccess::Write(&mut host), || {
            assert!(remove(&mut server).unwrap());
            assert_eq!(get(&mut server).unwrap(), None);
            assert!(!remove(&mut server).unwrap());
        });
        assert!(get(&mut server).is_err());
    }

    #[test]
    fn storage_reads_allow_listen_but_writes_require_mutable_services() {
        let mut server = GameServerHost {
            id: "cap-storage".into(),
            commands: false,
        };
        let mut other = GameServerHost {
            id: "cap-storage-other".into(),
            commands: false,
        };
        let mut host = AttachmentServiceHost::new();
        let world = host.root.join("world");
        std::fs::create_dir_all(&world).unwrap();
        let handles = ["cap-storage", "cap-storage-other"]
            .into_iter()
            .map(|id| {
                let service = yoake_wasm::package_services::PackageServices::open(
                    id,
                    &[yoake_wasm::package_services::STORAGE_CAPABILITY.into()],
                    &Default::default(),
                    &Default::default(),
                    horizon_mods::yak::ConfigRuntimeSide::Server,
                    &host.root,
                    Some(&world),
                )
                .unwrap();
                (id.into(), std::sync::Arc::new(std::sync::Mutex::new(service)))
            })
            .collect();
        super::super::package::with_test_services(handles, || {
            assert!(server.world_storage_read("state".into()).is_err());
            assert!(server.config_read("state".into()).is_err());
            assert!(server.world_storage_write(vec![1], "state".into()).is_err());
            assert!(server.config_write(vec![2], "state".into()).is_err());
            with_services(HostAccess::Read(&host), || {
                assert_eq!(server.world_storage_read("state".into()).unwrap(), None);
                assert_eq!(server.config_read("state".into()).unwrap(), None);
                assert!(server.world_storage_write(vec![1], "state".into()).is_err());
                assert!(server.config_write(vec![2], "state".into()).is_err());
            });
            with_services(HostAccess::Write(&mut host), || {
                assert_eq!(server.world_storage_read("state".into()).unwrap(), None);
                assert_eq!(server.config_read("state".into()).unwrap(), None);
                server.world_storage_write(vec![1], "state".into()).unwrap();
                server.config_write(vec![2], "state".into()).unwrap();
                assert!(
                    server
                        .world_storage_write(vec![9], "../other".into())
                        .is_err()
                );
            });
            with_services(HostAccess::Read(&host), || {
                assert_eq!(
                    server.world_storage_read("state".into()).unwrap(),
                    Some(vec![1])
                );
                assert_eq!(server.config_read("state".into()).unwrap(), Some(vec![2]));
                assert_eq!(other.world_storage_read("state".into()).unwrap(), None);
                assert_eq!(other.config_read("state".into()).unwrap(), None);
                assert!(server.world_storage_write(vec![3], "state".into()).is_err());
                assert!(server.config_write(vec![4], "state".into()).is_err());
            });
            assert!(server.world_storage_write(vec![3], "state".into()).is_err());
            assert!(server.config_write(vec![4], "state".into()).is_err());
            with_services(HostAccess::Read(&host), || {
                assert_eq!(
                    server.world_storage_read("state".into()).unwrap(),
                    Some(vec![1])
                );
                assert_eq!(server.config_read("state".into()).unwrap(), Some(vec![2]));
            });
        });
        assert!(host.root.starts_with(std::env::temp_dir()));
        crate::test_support::remove_dir_all(&host.root);
    }

    #[test]
    fn gui_size_reads_live_host_without_hud_layout() {
        let host = FakeHost::default();
        host.gui_dimensions.set([320.0, 240.0]);
        let mut service = GameHost::new("gui-size-test");
        assert!(yoake_wasm::ClientServices::gui_size(&mut service).is_err());

        with_services(HostAccess::Read(&host), || {
            let size = yoake_wasm::ClientServices::gui_size(&mut service).unwrap();
            assert_eq!((size.x, size.y), (320.0, 240.0));
            host.gui_dimensions.set([640.0, 360.0]);
            let size = yoake_wasm::ClientServices::gui_size(&mut service).unwrap();
            assert_eq!((size.x, size.y), (640.0, 360.0));
        });
    }

    #[test]
    fn hud_place_v2_shares_top_center_reservations() {
        use yoake_wasm::client_types::{HudCornerV2, Vec2};

        let host = FakeHost::default();
        let mut corners = Corners::new(400.0, 300.0, 26.0);
        corners.reserve_top_center(1);
        let mut invocation = Invocation {
            host: HostAccess::Read(&host),
            batch: None,
            batch_start: 0,
            client: ClientExtras {
                corners: Some(&mut corners),
                ..ClientExtras::default()
            },
        };
        let _scope = Scope::enter(&mut invocation);
        let mut first = GameHost::new("top-center-first");
        let mut second = GameHost::new("top-center-second");
        let size = Vec2 { x: 50.0, y: 12.0 };
        let placed = yoake_wasm::ClientServices::hud_place_v2(
            &mut first,
            HudCornerV2::TopCenter,
            size.clone(),
        )
        .unwrap()
        .unwrap();
        assert_eq!((placed.x, placed.y), (175.0, 19.0));
        let placed = yoake_wasm::ClientServices::hud_place_v2(
            &mut second,
            HudCornerV2::TopCenter,
            size,
        )
        .unwrap()
        .unwrap();
        assert_eq!((placed.x, placed.y), (175.0, 33.0));
    }

    #[test]
    fn hud_place_v2_declines_when_four_bosses_fill_small_screen() {
        use yoake_wasm::client_types::{HudCornerV2, Vec2};

        let host = FakeHost::default();
        let mut batch = HudBatch::new([320, 240]);
        let mut corners = Corners::new(batch.width, batch.height, 0.0);
        corners.reserve_top_center(4);
        {
            let mut invocation = Invocation {
                host: HostAccess::Read(&host),
                batch: Some(&mut batch),
                batch_start: 0,
                client: ClientExtras {
                    corners: Some(&mut corners),
                    ..ClientExtras::default()
                },
            };
            let _scope = Scope::enter(&mut invocation);
            let mut service = GameHost::new("small-screen-boss-test");
            let placed = yoake_wasm::ClientServices::hud_place_v2(
                &mut service,
                HudCornerV2::TopCenter,
                Vec2 { x: 20.0, y: 8.0 },
            )
            .unwrap();
            assert!(placed.is_none());
        }
        assert!(batch.vertices.is_empty());
    }

    #[test]
    fn hud_place_v2_shares_legacy_corner_stacks() {
        use yoake_wasm::client_types::{HudCorner, HudCornerV2, Vec2};

        let cases = [
            (
                HudCorner::TopLeft,
                HudCornerV2::TopLeft,
                [(2.0, 2.0), (2.0, 16.0), (2.0, 30.0)],
            ),
            (
                HudCorner::TopRight,
                HudCornerV2::TopRight,
                [(348.0, 28.0), (348.0, 42.0), (348.0, 56.0)],
            ),
            (
                HudCorner::BottomLeft,
                HudCornerV2::BottomLeft,
                [(2.0, 286.0), (2.0, 272.0), (2.0, 258.0)],
            ),
            (
                HudCorner::BottomRight,
                HudCornerV2::BottomRight,
                [(348.0, 286.0), (348.0, 272.0), (348.0, 258.0)],
            ),
        ];
        let host = FakeHost::default();
        for (legacy_corner, new_corner, expected) in cases {
            let mut corners = Corners::new(400.0, 300.0, 26.0);
            let mut invocation = Invocation {
                host: HostAccess::Read(&host),
                batch: None,
                batch_start: 0,
                client: ClientExtras {
                    corners: Some(&mut corners),
                    ..ClientExtras::default()
                },
            };
            let _scope = Scope::enter(&mut invocation);
            let mut first = GameHost::new("legacy-first");
            let mut second = GameHost::new("v2-second");
            let size = Vec2 { x: 50.0, y: 12.0 };
            let first_placement =
                yoake_wasm::ClientServices::hud_place(&mut first, legacy_corner.clone(), size.clone())
                    .unwrap()
                    .unwrap();
            let second_placement =
                yoake_wasm::ClientServices::hud_place_v2(&mut second, new_corner, size.clone())
                    .unwrap()
                    .unwrap();
            let third_placement =
                yoake_wasm::ClientServices::hud_place(&mut first, legacy_corner, size)
                    .unwrap()
                    .unwrap();
            assert_eq!((first_placement.x, first_placement.y), expected[0]);
            assert_eq!((second_placement.x, second_placement.y), expected[1]);
            assert_eq!((third_placement.x, third_placement.y), expected[2]);
        }
    }

    #[derive(Default)]
    struct FakeHost {
        texts: RefCell<Vec<String>>,
        fills: RefCell<u32>,
        told: RefCell<Vec<String>>,
        gui_dimensions: std::cell::Cell<[f32; 2]>,
        sprite_quads: RefCell<Vec<(String, [[f32; 2]; 4], [[f32; 2]; 4], u32)>>,
        shared_sends: RefCell<Vec<(String, Vec<u8>)>>,
        sends: Vec<(String, Vec<u8>)>,
        carried: Option<ItemStack>,
        stacks: RefCell<Vec<ItemStack>>,
        sounds: RefCell<Vec<(String, Option<[f64; 3]>, f32, f32)>>,
        cubes: RefCell<Vec<RecordedCube>>,
        drawn_items: RefCell<Vec<(String, DisplayContext, Option<f32>, i32)>>,
        drawn_blocks: RefCell<Vec<String>>,
        drawn_models: RefCell<Vec<(String, u32)>>,
        drawn_skeletons: RefCell<
            Vec<(
                String,
                Vec<crate::mobs::custom::script::MobBoneWrite>,
                Option<String>,
                Option<u32>,
            )>,
        >,
        custom_mob_anims: RefCell<Vec<(u32, String)>>,
        fill_rects: RefCell<Vec<([f32; 4], u32)>>,
    }

    /// A `draw_cube` call: corners, faces as (sprite, uv) and the tint.
    type RecordedCube = ([f32; 3], [f32; 3], Vec<Option<(String, Option<[f32; 4]>)>>, u32);

    impl Host for FakeHost {
        fn draw_cube(
            &self,
            _draw: &mut BlockEntityDraw,
            from: [f32; 3],
            to: [f32; 3],
            faces: [Option<CubeFace<'_>>; 6],
            tint: u32,
        ) -> bool {
            let faces = faces
                .iter()
                .map(|f| f.as_ref().map(|f| (f.sprite.to_owned(), f.uv)))
                .collect();
            self.cubes.borrow_mut().push((from, to, faces, tint));
            true
        }

        fn draw_item(
            &self,
            _draw: &mut BlockEntityDraw,
            stack: &ItemStack,
            display: DisplayContext,
            owner_yaw: Option<f32>,
            seed: i32,
        ) -> bool {
            self.drawn_items
                .borrow_mut()
                .push((stack.item.to_string(), display, owner_yaw, seed));
            true
        }

        fn draw_model(&self, _draw: &mut BlockEntityDraw, model: &str, tint: u32) -> bool {
            self.drawn_models.borrow_mut().push((model.to_owned(), tint));
            model.starts_with("mymod:block/")
        }

        fn draw_skeleton(
            &self,
            _draw: &mut BlockEntityDraw,
            model_id: &str,
            writes: &[crate::mobs::custom::script::MobBoneWrite],
            texture_layer: Option<&str>,
            entity: Option<u32>,
        ) -> bool {
            self.drawn_skeletons.borrow_mut().push((
                model_id.to_owned(),
                writes.to_vec(),
                texture_layer.map(str::to_owned),
                entity,
            ));
            model_id == "sample_mobs:cinder_fox" || model_id.starts_with("mymod:")
        }

        fn client_custom_mob_has_anim(&self, entity: u32, name: &str) -> bool {
            self.custom_mob_anims
                .borrow()
                .iter()
                .any(|(id, anim)| *id == entity && anim == name)
        }

        fn draw_block(
            &self,
            _draw: &mut BlockEntityDraw,
            block: &str,
            _properties: &[(&str, &str)],
        ) -> bool {
            self.drawn_blocks.borrow_mut().push(block.to_owned());
            block.starts_with("minecraft:stone")
        }

        fn net_send_to_server(&mut self, channel: &str, data: &[u8]) -> Result<(), String> {
            self.sends.push((channel.to_owned(), data.to_vec()));
            Ok(())
        }

        fn net_send_to_server_shared(&self, channel: &str, data: &[u8]) -> Result<(), String> {
            self.shared_sends
                .borrow_mut()
                .push((channel.to_owned(), data.to_vec()));
            Ok(())
        }

        fn screen_carried_stack(&self) -> Option<&ItemStack> {
            self.carried.as_ref()
        }

        fn local_player_stacks(&self) -> Vec<ItemStack> {
            self.stacks.borrow().clone()
        }

        fn queue_local_sound(
            &self,
            sound: &str,
            at: Option<[f64; 3]>,
            volume: f32,
            pitch: f32,
        ) -> bool {
            if sound.contains(' ') {
                return false;
            }
            self.sounds
                .borrow_mut()
                .push((sound.to_owned(), at, volume, pitch));
            true
        }

        fn sprite_quad(
            &self,
            _batch: &mut HudBatch,
            name: &str,
            corners: [[f32; 2]; 4],
            uv: [[f32; 2]; 4],
            argb: u32,
        ) -> bool {
            self.sprite_quads
                .borrow_mut()
                .push((name.to_owned(), corners, uv, argb));
            true
        }

        fn gui_size(&self) -> [f32; 2] {
            self.gui_dimensions.get()
        }

        fn hud_place(
            &self,
            corners: &mut Corners,
            corner: crate::extras::Corner,
            size: [f32; 2],
        ) -> Option<[f32; 2]> {
            let (position_x, position_y) = corners.place(corner, size[0], size[1]);
            Some([position_x, position_y])
        }

        fn window(&self) -> bool {
            true
        }
        fn text(&self, _batch: &mut HudBatch, text: &str, _at: [f32; 2], _rgb: u32, _shadow: bool) {
            self.texts.borrow_mut().push(text.to_owned());
        }
        fn fill(&self, _batch: &mut HudBatch, rect: [f32; 4], argb: u32) {
            *self.fills.borrow_mut() += 1;
            self.fill_rects.borrow_mut().push((rect, argb));
        }
        fn tell(&mut self, uuid: u128, line: Text) -> bool {
            self.told
                .borrow_mut()
                .push(format!("{uuid:032x}:{}", line.plain()));
            true
        }
    }

    fn module(kind: &str) -> Option<Vec<u8>> {
        use yoake_wasm::test_guests::{FULL_DEMO_CLIENT, FULL_DEMO_CLIENT_PEER, FULL_DEMO_PEER, FULL_DEMO_SERVER, load};
        load(match kind {
            "client-peer" => &FULL_DEMO_CLIENT_PEER,
            "server" => &FULL_DEMO_SERVER,
            "peer" => &FULL_DEMO_PEER,
            _ => &FULL_DEMO_CLIENT,
        })
    }

    fn client() -> Option<FullMod> {
        let bytes = module("client")?;
        let client = load_client(&ModuleSource {
            id: "demo",
            bytes: &bytes,
            commands: false,
            gpu: false,
        })
        .expect("client-full loads");
        Some(FullMod::new("demo".into(), Some(client), None))
    }

    #[test]
    fn items_and_nbt_convert_both_ways() {
        let mut stack = ItemStack::new("minecraft:diamond_sword", 1);
        stack.enchant("minecraft:sharpness", 5);
        let wire = item_w(&stack);
        assert_eq!(wire.id, "minecraft:diamond_sword");
        assert!(!wire.components.is_empty());
        assert_eq!(
            item_from(&wire)
                .unwrap()
                .enchantment_level("minecraft:sharpness"),
            5
        );
        let tag = Tag::compound([("n", Tag::Int(7)), ("s", Tag::String("x".into()))]);
        assert_eq!(nbt_tag_from(&nbt_tag(&tag)), Some(tag));
        assert!(nbt_tag_from(&[99]).is_none());
    }

    #[test]
    fn hud_commands_are_applied_through_the_host_services() {
        let host = FakeHost::default();
        let mut batch = HudBatch::new([854, 480]);
        let commands = vec![
            et::HudCommand::Rect(et::HudRect {
                x: 0.0,
                y: 0.0,
                width: 10.0,
                height: 10.0,
                color: 0xff000000,
            }),
            et::HudCommand::Text(et::HudText {
                text: "{\"text\":\"hi\"}".into(),
                x: 1.0,
                y: 1.0,
                color: 0xffffff,
                shadow: true,
            }),
            et::HudCommand::EndClip,
        ];
        apply_hud(Some(&host), &mut batch, &commands);
        assert_eq!(*host.fills.borrow(), 1);
        assert_eq!(*host.texts.borrow(), ["hi"]);
        apply_hud(None, &mut batch, &commands);
        assert_eq!(*host.fills.borrow(), 1);
    }

    /// REM-12: the HUD commands of a full-access module are drawn by the S route's code: the same wire commands,
    /// so what the S tests above pin (rects, text, strips of a gradient, the highlight outline, clips) holds.
    #[test]
    fn f_hud_commands_become_the_same_wire_commands_and_are_drawn_through_the_host_services() {
        use crate::game::yoake2_f_services::{FBlend, FHud};
        let commands = vec![
            FHud::Rect { x: 0.0, y: 0.0, width: 10.0, height: 10.0, color: 0xff00_0000 },
            FHud::Text { text: "hi".into(), x: 1.0, y: 1.0, color: 0xff_ffff, shadow: true },
            FHud::Gradient { x: 0.0, y: 0.0, width: 4.0, height: 8.0, top: 0xff00_0000, bottom: 0xffff_ffff },
            FHud::Highlight { x: 0.0, y: 0.0, width: 6.0, height: 6.0, invert_text: false },
            FHud::BeginClip { x0: 0.0, y0: 0.0, x1: 5.0, y1: 5.0 },
            FHud::EndClip,
            FHud::BeginScale { center_x: 0.0, center_y: 0.0, scale_x: 2.0, scale_y: 2.0 },
            FHud::EndScale,
            FHud::BeginBlend(FBlend::Additive),
            FHud::EndBlend,
        ];
        let wire = f_hud_commands(&commands);
        assert_eq!(wire.len(), commands.len());
        assert!(matches!(&wire[0], et::HudCommand::Rect(r) if r.width == 10.0 && r.color == 0xff00_0000));
        assert!(matches!(&wire[1], et::HudCommand::Text(t) if text_from(&t.text).map(|t| t.plain()).as_deref() == Some("hi") && t.shadow));
        assert!(matches!(&wire[8], et::HudCommand::BeginBlend(et::HudBlend::Additive)));
        let host = FakeHost::default();
        let mut batch = HudBatch::new([854, 480]);
        apply_f_hud(Some(&host), &mut batch, &commands);
        assert_eq!(*host.texts.borrow(), ["hi"]);
        // one rect, the gradient's strips (4 of 2 px) and the highlight's tint and four edges
        assert_eq!(*host.fills.borrow(), 1 + 4 + 1 + 4);
        apply_f_hud(None, &mut batch, &commands);
        assert_eq!(*host.fills.borrow(), 10, "no host draws nothing");
        // a text longer than the S route's limit draws nothing, as for an S module
        let long = [FHud::Text { text: "x".repeat(abi::MAX_TEXT_BYTES + 1), x: 0.0, y: 0.0, color: 0, shadow: false }];
        apply_f_hud(Some(&host), &mut batch, &long);
        assert_eq!(host.texts.borrow().len(), 1);
    }

    fn plain_draw() -> BlockEntityDraw {
        BlockEntityDraw::for_test(glam::Mat4::IDENTITY, 0xF0F0)
    }

    #[test]
    fn cube_item_and_model_commands_reach_the_draw_services() {
        let host = FakeHost::default();
        let mut draw = plain_draw();
        let face = |sprite: &str, uv: Option<Vec<f32>>| {
            Some(et::ModelCubeFace {
                sprite: sprite.into(),
                uv,
            })
        };
        let commands = vec![
            et::ModelCommand::Translate((1.0, 2.0, 3.0)),
            et::ModelCommand::Cube(et::ModelCube {
                min: (0.0, 0.0, 0.0),
                max: (8.0, 8.0, 8.0),
                faces: vec![
                    face("mymod:entity/a", None),
                    None,
                    face("mymod:entity/b", Some(vec![0.0, 0.0, 4.0, 4.0])),
                ],
                tint: 0xff12_3456,
            }),
            et::ModelCommand::Item(et::ModelItem {
                stack: et::ItemStack {
                    id: "minecraft:apple".into(),
                    count: 1,
                    components: Vec::new(),
                },
                display: et::DisplayContext::Ground,
                owner_yaw: Some(90.0),
                seed: 7,
            }),
            // A standalone model first (never asked as a block or item), then a block, then an item that is no block.
            et::ModelCommand::Model(et::ModelStandalone {
                model: "mymod:block/shaft_half".into(),
                tint: 0xff80_4020,
            }),
            et::ModelCommand::Model(et::ModelStandalone {
                model: "minecraft:stone".into(),
                tint: 0,
            }),
            et::ModelCommand::Model(et::ModelStandalone {
                model: "minecraft:apple".into(),
                tint: 0,
            }),
            et::ModelCommand::Model(et::ModelStandalone {
                model: "bad id\n".into(),
                tint: 0,
            }),
        ];
        apply_model(Some(&host), &mut draw, &commands);
        let cubes = host.cubes.borrow();
        assert_eq!(cubes.len(), 1);
        assert_eq!(cubes[0].0, [0.0; 3]);
        assert_eq!(cubes[0].1, [8.0; 3]);
        assert_eq!(cubes[0].3, 0x0012_3456);
        assert_eq!(cubes[0].2.len(), 6);
        assert_eq!(cubes[0].2[0], Some(("mymod:entity/a".to_owned(), None)));
        assert_eq!(cubes[0].2[1], None);
        assert_eq!(
            cubes[0].2[2],
            Some(("mymod:entity/b".to_owned(), Some([0.0, 0.0, 4.0, 4.0])))
        );
        assert!(cubes[0].2[3..].iter().all(Option::is_none));
        let items = host.drawn_items.borrow();
        assert_eq!(
            *items,
            [
                ("minecraft:apple".to_owned(), DisplayContext::Ground, Some(90.0), 7),
                ("minecraft:apple".to_owned(), DisplayContext::Fixed, None, 0),
            ]
        );
        assert_eq!(*host.drawn_blocks.borrow(), ["minecraft:stone", "minecraft:apple"]);
        // Every valid model id is asked as a standalone model first (tint masked to RGB); only those it
        // does not draw fall back to a block, then an item. A bad id is asked of nobody.
        assert_eq!(
            *host.drawn_models.borrow(),
            [
                ("mymod:block/shaft_half".to_owned(), 0x0080_4020),
                ("minecraft:stone".to_owned(), 0),
                ("minecraft:apple".to_owned(), 0)
            ]
        );
        // The translate moved the draw's pose for the commands after it.
        assert_eq!(draw.pose_matrix().w_axis.truncate(), glam::Vec3::new(1.0, 2.0, 3.0));
    }

    #[test]
    fn malformed_cubes_and_stacks_draw_nothing_and_no_host_is_a_no_op() {
        let host = FakeHost::default();
        let mut draw = plain_draw();
        let cube = |min: f32, uv: Vec<f32>, faces: usize| {
            et::ModelCommand::Cube(et::ModelCube {
                min: (min, 0.0, 0.0),
                max: (1.0, 1.0, 1.0),
                faces: (0..faces)
                    .map(|_| {
                        Some(et::ModelCubeFace {
                            sprite: "a:b".into(),
                            uv: Some(uv.clone()),
                        })
                    })
                    .collect(),
                tint: 0,
            })
        };
        let commands = vec![
            cube(f32::NAN, vec![0.0; 4], 1),
            cube(0.0, vec![0.0; 3], 1),
            cube(0.0, vec![0.0; 4], 7),
            et::ModelCommand::Item(et::ModelItem {
                stack: et::ItemStack {
                    id: "".into(),
                    count: 1,
                    components: Vec::new(),
                },
                display: et::DisplayContext::Fixed,
                owner_yaw: None,
                seed: 0,
            }),
        ];
        apply_model(Some(&host), &mut draw, &commands);
        assert!(host.cubes.borrow().is_empty());
        assert!(host.drawn_items.borrow().is_empty());
        let good = vec![cube(0.0, vec![0.0; 4], 1)];
        apply_model(None, &mut draw, &good);
        assert!(host.cubes.borrow().is_empty());
        apply_model(Some(&host), &mut draw, &good);
        assert_eq!(host.cubes.borrow().len(), 1);
    }

    #[test]
    fn skeleton_commands_validate_and_reach_host_draw_skeleton() {
        use crate::mobs::custom::script::{MAX_POSE_WRITES, MobBoneWrite, ScriptVec3};
        use yoake_wasm::client_types as ct;

        let host = FakeHost::default();
        let mut draw = plain_draw();
        let valid = ct::ModelSkeleton {
            model: "sample_mobs:cinder_fox".into(),
            pose: vec![ct::MobBoneWrite {
                bone: 1,
                additive: true,
                translation: Some(ct::ScriptVec3 { x: 0.0, y: 0.25, z: 0.0 }),
                rotation_degrees: Some(ct::ScriptVec3 { x: 15.0, y: 0.0, z: 0.0 }),
                scale: None,
                visible: Some(true),
            }],
            texture_layer: Some("glow".into()),
            entity: Some(77),
        };
        let parsed = parse_skeleton(&valid).expect("valid skeleton parses");
        assert_eq!(parsed.entity, Some(77));
        assert!(!apply_skeleton(
            None,
            &mut draw,
            &parsed.model,
            &parsed.pose,
            parsed.texture_layer.as_deref(),
            parsed.entity,
        ));
        assert!(host.drawn_skeletons.borrow().is_empty());
        assert!(apply_skeleton(
            Some(&host),
            &mut draw,
            &parsed.model,
            &parsed.pose,
            parsed.texture_layer.as_deref(),
            parsed.entity,
        ));
        assert!(apply_skeleton(
            Some(&host),
            &mut draw,
            &parsed.model,
            &parsed.pose,
            None,
            None,
        ));
        {
            let drawn = host.drawn_skeletons.borrow();
            assert_eq!(drawn.len(), 2);
            assert_eq!(drawn[0].0, "sample_mobs:cinder_fox");
            assert_eq!(drawn[0].1.len(), 1);
            let w = &drawn[0].1[0];
            assert_eq!((w.bone, w.additive, w.visible), (1, true, Some(true)));
            assert_eq!(w.translation.map(|p| (p.x, p.y, p.z)), Some((0.0, 0.25, 0.0)));
            assert_eq!(w.rotation_degrees.map(|p| (p.x, p.y, p.z)), Some((15.0, 0.0, 0.0)));
            assert!(w.scale.is_none());
            assert_eq!(drawn[0].2.as_deref(), Some("glow"));
            assert_eq!(drawn[0].3, Some(77));
            assert_eq!(drawn[1].2.as_deref(), None);
            assert_eq!(drawn[1].3, None);
        }

        // Bad model id, bad texture layer, non-finite bone write, and oversized pose are refused cleanly.
        let mut bad_model = valid.clone();
        bad_model.model = "bad id\n".into();
        assert!(parse_skeleton(&bad_model).is_err());

        let mut bad_layer = valid.clone();
        bad_layer.texture_layer = Some(String::new());
        assert!(parse_skeleton(&bad_layer).is_err());

        let mut bad_float = valid.clone();
        bad_float.pose[0].translation = Some(ct::ScriptVec3 { x: f32::NAN, y: 0.0, z: 0.0 });
        assert!(parse_skeleton(&bad_float).is_err());

        let mut too_many = valid;
        too_many.pose = vec![
            ct::MobBoneWrite {
                bone: 0,
                additive: false,
                translation: None,
                rotation_degrees: None,
                scale: None,
                visible: None,
            };
            MAX_POSE_WRITES + 1
        ];
        assert!(parse_skeleton(&too_many).is_err());
        assert!(!apply_skeleton(
            Some(&host),
            &mut draw,
            "bad id\n",
            &[MobBoneWrite {
                bone: 0,
                additive: false,
                translation: None,
                rotation_degrees: None,
                scale: None,
                visible: None,
            }],
            None,
            None,
        ));
        assert_eq!(host.drawn_skeletons.borrow().len(), 2);
    }

    #[test]
    fn client_services_mob_play_local_anim_and_skeleton_gate_validate_and_dispatch() {
        use yoake_wasm::{ClientServices, client_types as ct};

        static ANIM_LOCK: std::sync::Mutex<()> = std::sync::Mutex::new(());
        let _lock = ANIM_LOCK.lock().unwrap_or_else(|e| e.into_inner());
        let _ = crate::mobs::custom::script::take_local_anims(42);

        let host = FakeHost::default();
        host.custom_mob_anims.borrow_mut().push((42, "pounce".into()));
        let mut service = GameHost::new("custom-mob-client-test");
        let skeleton = ct::ModelSkeleton {
            model: "sample_mobs:cinder_fox".into(),
            pose: Vec::new(),
            texture_layer: None,
            entity: None,
        };
        let with_entity = ct::ModelSkeleton {
            entity: Some(42),
            ..skeleton.clone()
        };

        // Missing service.mobs grant refuses both client services.
        with_services(HostAccess::Read(&host), || {
            assert!(service.mob_play_local_anim(42, "pounce".into(), 1.0).is_err());
            assert!(service.mob_draw_skeleton(skeleton.clone()).is_err());
        });

        let _grant = super::super::client_services::allow_mobs_for_test();
        with_services(HostAccess::Read(&host), || {
            assert!(service.mob_play_local_anim(42, String::new(), 1.0).is_err());
            assert!(service.mob_play_local_anim(42, "pounce".into(), 0.0).is_err());
            assert!(service.mob_play_local_anim(42, "pounce".into(), f32::NAN).is_err());
            assert!(service.mob_play_local_anim(42, "unknown_anim".into(), 1.0).is_err());
            assert!(service.mob_play_local_anim(999, "pounce".into(), 1.0).is_err());
            assert_eq!(service.mob_play_local_anim(42, "pounce".into(), 1.25), Ok(true));
            // Outside a mob render hook, mob_draw_skeleton refuses cleanly.
            assert!(service.mob_draw_skeleton(skeleton.clone()).is_err());
        });

        let queued = crate::mobs::custom::script::take_local_anims(42);
        assert_eq!(queued.len(), 1);
        assert_eq!((queued[0].name.as_str(), queued[0].speed), ("pounce", 1.25));

        let mut draw = plain_draw();
        let mut invocation = Invocation {
            host: HostAccess::Read(&host),
            batch: None,
            batch_start: 0,
            client: ClientExtras {
                mob_draw: Some(&mut draw),
                ..ClientExtras::default()
            },
        };
        let _scope = Scope::enter(&mut invocation);
        assert_eq!(service.mob_draw_skeleton(skeleton), Ok(true));
        assert_eq!(service.mob_draw_skeleton(with_entity), Ok(true));
        let drawn = host.drawn_skeletons.borrow();
        assert_eq!(drawn.len(), 2);
        assert_eq!(drawn[0].3, None);
        assert_eq!(drawn[1].3, Some(42));
    }

    #[test]
    fn a_highlight_is_a_tint_with_a_one_pixel_outline() {
        let host = FakeHost::default();
        let mut batch = HudBatch::new([854, 480]);
        let highlight = |w: f32, invert_text: bool| {
            et::HudCommand::Highlight(et::HudHighlight {
                x: 10.0,
                y: 20.0,
                width: w,
                height: 8.0,
                invert_text,
            })
        };
        apply_hud(Some(&host), &mut batch, &[highlight(30.0, false)]);
        {
            let rects = host.fill_rects.borrow();
            assert_eq!(rects.len(), 5);
            assert_eq!(rects[0], ([10.0, 20.0, 30.0, 8.0], HIGHLIGHT_TINT));
            assert!(rects[1..].iter().all(|(_, color)| *color == HIGHLIGHT_OUTLINE));
            // Top, bottom, left, right edges stay inside the rectangle.
            assert_eq!(rects[1].0, [10.0, 20.0, 30.0, 1.0]);
            assert_eq!(rects[2].0, [10.0, 27.0, 30.0, 1.0]);
            assert_eq!(rects[3].0, [10.0, 21.0, 1.0, 6.0]);
            assert_eq!(rects[4].0, [39.0, 21.0, 1.0, 6.0]);
        }
        host.fill_rects.borrow_mut().clear();
        apply_hud(Some(&host), &mut batch, &[highlight(0.0, false), highlight(-3.0, true)]);
        assert!(host.fill_rects.borrow().is_empty());
        apply_hud(Some(&host), &mut batch, &[highlight(30.0, true)]);
        assert_eq!(host.fill_rects.borrow()[0].1, 0xffff_ffff);
        apply_hud(None, &mut batch, &[highlight(30.0, true)]);
        assert_eq!(host.fill_rects.borrow().len(), 5);
    }

    #[test]
    fn quad_points_follow_a_turned_square() {
        // A 10 x 10 square about its centre, turned a quarter: the top left goes to the top right.
        let corners = [[10.0, 0.0], [10.0, 10.0], [0.0, 10.0], [0.0, 0.0]];
        assert_eq!(quad_point(&corners, 0.0, 0.0), [10.0, 0.0]);
        assert_eq!(quad_point(&corners, 1.0, 0.0), [10.0, 10.0]);
        assert_eq!(quad_point(&corners, 1.0, 1.0), [0.0, 10.0]);
        assert_eq!(quad_point(&corners, 0.0, 1.0), [0.0, 0.0]);
        assert_eq!(quad_point(&corners, 0.5, 0.5), [5.0, 5.0]);
    }

    #[test]
    fn a_sprite_quad_goes_to_the_host_with_its_uvs_and_an_unknown_texture_draws_nothing() {
        let host = FakeHost::default();
        let mut batch = HudBatch::new([854, 480]);
        let quad = |sprite: &str, uv: Option<Vec<(f32, f32)>>| {
            et::HudCommand::Quad(et::HudQuad {
                corners: vec![(0.0, 0.0), (8.0, 0.0), (8.0, 8.0), (0.0, 8.0)],
                sprite: Some(sprite.to_owned()),
                uv,
                color: 0x80ff00ff,
            })
        };
        let commands = vec![
            quad("gui/sprites/hud/heart/full", None),
            quad(
                "gui/sprites/hud/heart/half",
                Some(vec![(0.0, 0.0), (0.5, 0.0), (0.5, 1.0), (0.0, 1.0)]),
            ),
            // A texture id nobody named: nothing is drawn and nothing is asked of the host.
            quad("texture:nobody#1", None),
        ];
        apply_hud(Some(&host), &mut batch, &commands);
        let quads = host.sprite_quads.borrow();
        assert_eq!(quads.len(), 2);
        assert_eq!(quads[0].0, "gui/sprites/hud/heart/full");
        assert_eq!(quads[0].2, [[0.0, 0.0], [1.0, 0.0], [1.0, 1.0], [0.0, 1.0]]);
        assert_eq!(quads[0].3, 0x80ff00ff);
        assert_eq!(quads[1].2[1], [0.5, 0.0]);
    }

    #[test]
    fn net_send_to_server_works_from_read_only_hooks() {
        use yoake_wasm::ClientHost;
        let host = FakeHost::default();
        let mut service = GameHost::new("net-read-test");
        // No host at all: refused as before.
        with_services(HostAccess::None, || {
            assert!(service.net_send_to_server("net:test", &[1]).is_err());
        });
        // A read-only hook (a screen key or mouse hook) goes through the shared path.
        with_services(HostAccess::Read(&host), || {
            service.net_send_to_server("net:test", &[1, 2]).unwrap();
            assert!(service.net_send_to_server(&"c".repeat(300), &[1]).is_err());
        });
        assert_eq!(*host.shared_sends.borrow(), [("net:test".to_owned(), vec![1, 2])]);
        // A hook that may write keeps the ordinary path.
        let mut writer = FakeHost::default();
        with_services(HostAccess::Write(&mut writer), || {
            service.net_send_to_server("net:test", &[3]).unwrap();
        });
        assert!(writer.shared_sends.borrow().is_empty());
        assert_eq!(writer.sends, [("net:test".to_owned(), vec![3])]);
    }

    #[test]
    fn client_inventory_reads_and_sounds_go_through_the_host() {
        use yoake_wasm::ClientServices;
        let mut host = FakeHost::default();
        host.carried = Some(ItemStack::new("minecraft:apple", 3));
        *host.stacks.borrow_mut() = vec![
            ItemStack::new("minecraft:bread", 1),
            ItemStack::new("minecraft:stick", 64),
        ];
        let mut service = GameHost::new("inventory-read-test");
        // Without a client host the services are refused.
        with_services(HostAccess::None, || {
            assert!(service.screen_carried().is_err());
            assert!(service.local_player_stacks().is_err());
            assert!(service.play_local_sound(1.0, "a:b".into(), 1.0).is_err());
        });
        with_services(HostAccess::Read(&host), || {
            let carried = service.screen_carried().unwrap().unwrap();
            assert_eq!((carried.item.as_str(), carried.count), ("minecraft:apple", 3));
            let stacks = service.local_player_stacks().unwrap();
            assert_eq!(stacks.len(), 2);
            assert_eq!(stacks[1].count, 64);
            service.play_local_sound(1.0, "ui:click".into(), 0.5).unwrap();
            let at = yoake_wasm::client_types::Vec3 { x: 1.0, y: 2.0, z: 3.0 };
            service
                .play_local_sound_at(0.8, at, "ui:ding".into(), 1.0)
                .unwrap();
            // The host refuses a bad id or value, and the service says so.
            assert!(service.play_local_sound(1.0, "bad id".into(), 1.0).is_err());
        });
        assert_eq!(
            *host.sounds.borrow(),
            [
                ("ui:click".to_owned(), None, 0.5, 1.0),
                ("ui:ding".to_owned(), Some([1.0, 2.0, 3.0]), 1.0, 0.8),
            ]
        );
        host.carried = None;
        with_services(HostAccess::Read(&host), || {
            assert!(service.screen_carried().unwrap().is_none());
        });
    }

    #[test]
    fn a_guest_cancels_a_layer_draws_commands_and_mutates_values() {
        let _serial = super::super::GUEST_TEST_LOCK.lock().unwrap_or_else(|error| error.into_inner());
        let Some(mod_) = client() else { return };
        let mut host = FakeHost::default();
        let mut batch = HudBatch::new([854, 480]);
        // Cancel: the demo hides `hotbar` only.
        let mut hotbar = GuiLayerPreEvent {
            layer: "hotbar",
            batch: &mut batch,
        };
        assert!(matches!(
            mod_.gui_layer_pre(&mut host, &mut hotbar),
            Flow::Cancel
        ));
        let mut chat = GuiLayerPreEvent {
            layer: "chat",
            batch: &mut batch,
        };
        assert!(matches!(
            mod_.gui_layer_pre(&mut host, &mut chat),
            Flow::Continue
        ));
        // Command list: the hud hook's rect, gradient strips and text.
        let mut corners = Corners::new(854.0, 480.0, 0.0);
        mod_.hud(
            &mut host,
            &mut HudEvent {
                batch: &mut batch,
                corners: &mut corners,
            },
        );
        assert!(*host.fills.borrow() >= 2);
        assert_eq!(host.texts.borrow().len(), 1);
        // Mutate outcome: the demo adds 10 degrees to the field of view.
        let mut fov = ComputeFovEvent {
            partial: 0.0,
            camera: [0.0; 3],
            degrees: 70.0,
        };
        mod_.compute_fov(&host, &mut fov);
        assert_eq!(fov.degrees, 80.0);
        // Collect with an item stack in and text out.
        let stack = ItemStack::new("minecraft:stone", 3);
        let mut lines = vec![(Text::literal("Stone"), 0xffffff)];
        mod_.tooltip(
            &host,
            &mut TooltipEvent {
                stack: &stack,
                lines: &mut lines,
            },
        );
        assert_eq!(lines.len(), 2);
        assert!(lines[1].0.plain().contains("minecraft:stone x3"));
    }

    #[test]
    fn a_server_guest_runs_server_tick_and_perform_command_on_the_server_thread() {
        let _serial = super::super::GUEST_TEST_LOCK.lock().unwrap_or_else(|error| error.into_inner());
        let Some(bytes) = module("server") else { return };
        let server = load_server(&ModuleSource {
            id: "demo",
            bytes: &bytes,
            commands: false,
            gpu: false,
        })
        .expect("server-full loads");
        let mod_ = Arc::new(FullMod::new("demo".into(), None, Some(server)));
        let runner = Arc::clone(&mod_);
        // The server loop's scope, not the thread's name, routes a shared hook to the server module.
        let handle = std::thread::Builder::new()
            .name("not called Server thread".into())
            .spawn(move || {
                let _server = crate::mod_guard::enter_thread(crate::mod_guard::ModThread::Server);
                let mut host = FakeHost::default();
                let player = NativeServerPlayer {
                    uuid: 0x1122_3344_5566_7788_99aa_bbcc_ddee_ff00,
                    name: "Guest".into(),
                    permission: 0,
                    dimension: "minecraft:overworld".into(),
                    position: [0.0; 3],
                    game_mode: 1,
                    health: 20.0,
                    yaw: 0.0,
                    pitch: 0.0,
                };
                let taken = runner.perform_command(
                    &mut host,
                    &mut PerformCommandEvent {
                        player: &player,
                        command: "wasmping",
                    },
                );
                let other = runner.perform_command(
                    &mut host,
                    &mut PerformCommandEvent {
                        player: &player,
                        command: "other",
                    },
                );
                let mut commands = Vec::new();
                runner.register_commands(
                    &mut host,
                    &mut RegisterCommandsEvent {
                        commands: &mut commands,
                    },
                );
                let told = host.told.borrow().clone();
                (
                    matches!(taken, Flow::Cancel),
                    matches!(other, Flow::Continue),
                    commands,
                    told,
                )
            })
            .unwrap();
        let (taken, other_continues, commands, told) = handle.join().unwrap();
        assert!(taken && other_continues);
        assert_eq!(
            commands
                .iter()
                .map(|c| c.usage.as_str())
                .collect::<Vec<_>>(),
            ["wasmping", "wasmsvc"]
        );
        assert_eq!(told.len(), 1);
        assert!(
            told[0].starts_with("112233445566778899aabbccddeeff00:pong #1"),
            "{told:?}"
        );
        // On a thread that did not enter the server scope a hook of the shared code does not reach the server module.
        let mut host = FakeHost::default();
        let player = NativeServerPlayer {
            uuid: 1,
            name: "G".into(),
            permission: 0,
            dimension: "d".into(),
            position: [0.0; 3],
            game_mode: 0,
            health: 1.0,
            yaw: 0.0,
            pitch: 0.0,
        };
        // perform_command is shared code: on this thread the client slot is asked, and there is none.
        assert!(matches!(
            mod_.perform_command(
                &mut host,
                &mut PerformCommandEvent {
                    player: &player,
                    command: "wasmping"
                }
            ),
            Flow::Continue
        ));
    }

    // -----------------------------------------------------------------------------------------
    // Review fixes

    #[test]
    fn a_thread_named_server_thread_is_not_the_server_without_the_scope() {
        let _serial = super::super::GUEST_TEST_LOCK.lock().unwrap_or_else(|error| error.into_inner());
        let Some(bytes) = module("server") else { return };
        let server = load_server(&ModuleSource {
            id: "demo",
            bytes: &bytes,
            commands: false,
            gpu: false,
        })
        .expect("server-full loads");
        let mod_ = Arc::new(FullMod::new("demo".into(), None, Some(server)));
        let runner = Arc::clone(&mod_);
        let flow = std::thread::Builder::new()
            .name("Server thread".into())
            .spawn(move || {
                let mut host = FakeHost::default();
                let player = NativeServerPlayer {
                    uuid: 1,
                    name: "G".into(),
                    permission: 0,
                    dimension: "d".into(),
                    position: [0.0; 3],
                    game_mode: 0,
                    health: 1.0,
                    yaw: 0.0,
                    pitch: 0.0,
                };
                matches!(
                    runner.perform_command(
                        &mut host,
                        &mut PerformCommandEvent {
                            player: &player,
                            command: "wasmping"
                        }
                    ),
                    Flow::Continue
                )
            })
            .unwrap()
            .join()
            .unwrap();
        assert!(
            flow,
            "the thread's name alone must not route a hook to the server module"
        );
    }

    #[test]
    fn a_busy_module_drops_the_hook_with_its_fallback_and_counts_it() {
        let _serial = super::super::GUEST_TEST_LOCK.lock().unwrap_or_else(|error| error.into_inner());
        let Some(mod_) = client() else { return };
        let mut host = FakeHost::default();
        let mut batch = HudBatch::new([854, 480]);
        // Hold the client module's lock, as a re-entry from a guest's own service call would.
        let held = mod_.client.lock().unwrap();
        let mut hotbar = GuiLayerPreEvent {
            layer: "hotbar",
            batch: &mut batch,
        };
        assert!(
            matches!(mod_.gui_layer_pre(&mut host, &mut hotbar), Flow::Continue),
            "the fallback is no answer"
        );
        let mut hotbar = GuiLayerPreEvent {
            layer: "hotbar",
            batch: &mut batch,
        };
        assert!(matches!(
            mod_.gui_layer_pre(&mut host, &mut hotbar),
            Flow::Continue
        ));
        drop(held);
        assert_eq!(mod_.dropped_hooks(Side::Client), 2);
        // Not busy any more: the guest answers again.
        let mut hotbar = GuiLayerPreEvent {
            layer: "hotbar",
            batch: &mut batch,
        };
        assert!(matches!(
            mod_.gui_layer_pre(&mut host, &mut hotbar),
            Flow::Cancel
        ));
        assert_eq!(mod_.dropped_hooks(Side::Client), 2);
    }

    #[test]
    fn a_package_without_a_render_hook_stays_out_of_the_render_pass() {
        let _serial = super::super::GUEST_TEST_LOCK.lock().unwrap_or_else(|error| error.into_inner());
        let Some(server_bytes) = module("server") else { return };
        let server = load_server(&ModuleSource {
            id: "demo",
            bytes: &server_bytes,
            commands: false,
            gpu: false,
        })
        .unwrap();
        let server_only = FullMod::new("demo".into(), None, Some(server));
        assert!(
            server_only.render().is_none(),
            "a server-only package has nothing to draw"
        );
        let Some(mod_) = client() else { return };
        // The demo's client module subscribes to hud and gui layers, not to a render-stage hook.
        let wants_render = RENDER_HOOKS
            .iter()
            .any(|hook| mod_.wants(Side::Client, *hook));
        assert_eq!(mod_.render().is_some(), wants_render);
    }

    #[test]
    fn the_server_module_starts_afresh_for_each_world() {
        let _serial = super::super::GUEST_TEST_LOCK.lock().unwrap_or_else(|error| error.into_inner());
        let Some(bytes) = module("server") else {
            return;
        };
        let source = ServerSource::new("demo", bytes, false);
        let server = load_server(&source.module()).unwrap();
        let mod_ =
            Arc::new(FullMod::new("demo".into(), None, Some(server)).with_server_source(source));
        let runner = Arc::clone(&mod_);
        let told = std::thread::spawn(move || {
            let _server = crate::mod_guard::enter_thread(crate::mod_guard::ModThread::Server);
            let player = NativeServerPlayer {
                uuid: 7,
                name: "G".into(),
                permission: 0,
                dimension: "d".into(),
                position: [0.0; 3],
                game_mode: 0,
                health: 1.0,
                yaw: 0.0,
                pitch: 0.0,
            };
            let mut lines = Vec::new();
            for world in 0..2 {
                // A world's server starts: the guest's counter starts at zero in each.
                runner.reset_server(None);
                let mut host = FakeHost::default();
                runner.perform_command(
                    &mut host,
                    &mut PerformCommandEvent {
                        player: &player,
                        command: "wasmping",
                    },
                );
                runner.perform_command(
                    &mut host,
                    &mut PerformCommandEvent {
                        player: &player,
                        command: "wasmping",
                    },
                );
                lines.push((world, host.told.borrow().clone()));
            }
            lines
        })
        .join()
        .unwrap();
        assert!(
            told[0].1[0].ends_with("pong #1 from the full-demo WASM server module"),
            "{told:?}"
        );
        assert!(
            told[1].1[0].contains("pong #1 "),
            "the second world began with a fresh instance: {told:?}"
        );
        assert!(told[1].1[1].contains("pong #2 "), "{told:?}");
    }

    /// F4M-13 R3: a rebuilt server module is swapped in at a server tick; the guest's counter starts afresh,
    /// a build that does not load keeps the old instance, and the next world starts from the new bytes.
    #[test]
    fn a_rebuilt_server_module_is_swapped_in_at_a_server_tick() {
        let _serial = super::super::GUEST_TEST_LOCK.lock().unwrap_or_else(|error| error.into_inner());
        let Some(bytes) = module("server") else {
            return;
        };
        let source = ServerSource::new("rl-demo", bytes.clone(), false);
        let server = load_server(&source.module()).unwrap();
        let reload = Arc::new(Reload::default());
        let mod_ = Arc::new(
            FullMod::new("rl-demo".into(), None, Some(server))
                .with_server_source(source)
                .with_server_reload(Some(Arc::clone(&reload))),
        );
        let runner = Arc::clone(&mod_);
        let told = std::thread::spawn(move || {
            let _server = crate::mod_guard::enter_thread(crate::mod_guard::ModThread::Server);
            let player = NativeServerPlayer {
                uuid: 7,
                name: "G".into(),
                permission: 0,
                dimension: "d".into(),
                position: [0.0; 3],
                game_mode: 0,
                health: 1.0,
                yaw: 0.0,
                pitch: 0.0,
            };
            let ping = |runner: &FullMod| {
                let mut host = FakeHost::default();
                runner.perform_command(&mut host, &mut PerformCommandEvent { player: &player, command: "wasmping" });
                let told = host.told.borrow().clone();
                told
            };
            let mut lines = vec![ping(&runner), ping(&runner)];
            // The file changed: nothing happens until a server tick asks.
            *reload.bytes.lock().unwrap() = Some(bytes.clone());
            reload.ready.store(true, Ordering::Release);
            lines.push(ping(&runner));
            assert!(reload.ready.load(Ordering::Acquire), "no swap between ticks");
            let _ = runner.wants(Side::Server, H_SERVER_TICK);
            assert!(!reload.ready.load(Ordering::Acquire), "taken at the tick");
            lines.push(ping(&runner));
            // A build that is not a component: the old instance (the new one) keeps counting.
            *reload.bytes.lock().unwrap() = Some(b"not a wasm component".to_vec());
            reload.ready.store(true, Ordering::Release);
            runner.wants(Side::Server, H_SERVER_TICK);
            lines.push(ping(&runner));
            // The next world starts from the swapped-in bytes.
            runner.reset_server(None);
            lines.push(ping(&runner));
            lines
        })
        .join()
        .unwrap();
        assert!(told[0][0].contains("pong #1 "), "{told:?}");
        assert!(told[1][0].contains("pong #2 "), "{told:?}");
        assert!(told[2][0].contains("pong #3 "), "no swap before a tick: {told:?}");
        assert!(told[3][0].contains("pong #1 "), "swapped at the tick, counting afresh: {told:?}");
        assert!(told[4][0].contains("pong #2 "), "a bad build keeps the running instance: {told:?}");
        assert!(told[5][0].contains("pong #1 "), "{told:?}");
    }

    #[test]
    fn mod_call_reaches_the_peers_handle_call_and_never_re_enters_a_waiting_caller() {
        let _serial = super::super::GUEST_TEST_LOCK.lock().unwrap_or_else(|error| error.into_inner());
        let Some(peer_bytes) = module("peer") else { return };
        let Some(demo_bytes) = module("server") else { return };
        let peer = load_server(&ModuleSource {
            id: "w2-peer",
            bytes: &peer_bytes,
            commands: false,
            gpu: false,
        })
        .expect("peer loads");
        let peer = FullMod::new("w2-peer".into(), None, Some(peer));
        let demo = load_server(&ModuleSource {
            id: "w2-caller",
            bytes: &demo_bytes,
            commands: false,
            gpu: false,
        })
        .expect("caller loads");
        let _demo = FullMod::new("w2-caller".into(), None, Some(demo));
        let mut host = FakeHost::default();
        let mut invocation = Invocation {
            host: HostAccess::Write(&mut host),
            batch: None,
            batch_start: 0,
            client: ClientExtras::default(),
        };
        let _scope = Scope::enter(&mut invocation);
        let answer = super::server_services::call_mod("w2-caller", "w2-peer", "echo", b"hi")
            .expect("the peer answers");
        assert_eq!(String::from_utf8(answer).unwrap(), "w2-caller said: hi");
        let counted =
            super::server_services::call_mod("w2-caller", "w2-peer", "count", b"").unwrap();
        assert_eq!(String::from_utf8(counted).unwrap(), "2 call(s) so far");
        // The peer's own error comes back as the caller's error.
        assert!(
            super::server_services::call_mod("w2-caller", "w2-peer", "nope", b"")
                .unwrap_err()
                .contains("unknown method")
        );
        // A callee that is busy (waiting, mid-hook) is refused, not waited for.
        let held = peer.server.state.lock().unwrap();
        assert!(
            super::server_services::call_mod("w2-caller", "w2-peer", "echo", b"x")
                .unwrap_err()
                .contains("busy")
        );
        drop(held);
        assert!(super::server_services::call_mod("w2-caller", "w2-peer", "echo", b"x").is_ok());
    }

    #[test]
    fn client_mod_call_reaches_the_peers_handle_call_with_the_servers_guards() {
        use super::super::client_services::call_client_mod as call;
        let _serial = super::super::GUEST_TEST_LOCK.lock().unwrap_or_else(|error| error.into_inner());
        let Some(peer_bytes) = module("client-peer") else { return };
        let load = |id: &str| {
            let module = load_client(&ModuleSource { id, bytes: &peer_bytes, commands: false, gpu: false }).expect("client peer loads");
            FullMod::new(id.to_owned(), Some(module), None)
        };
        let (a, b, c) = (load("w3-a"), load("w3-b"), load("w3-c"));
        let mut host = FakeHost::default();
        let mut invocation = Invocation { host: HostAccess::Write(&mut host), batch: None, batch_start: 0, client: ClientExtras::default() };
        let _scope = Scope::enter(&mut invocation);
        assert_eq!(String::from_utf8(call("w3-a", "w3-b", "echo", b"hi").expect("b answers")).unwrap(), "w3-a said: hi");
        assert_eq!(String::from_utf8(call("w3-a", "w3-b", "count", b"").unwrap()).unwrap(), "2 call(s) so far");
        // The peer's own error is the caller's error.
        assert!(call("w3-a", "w3-b", "nope", b"").unwrap_err().contains("unknown method"));
        // Guards: ids, method names, payload size, itself, an unknown package.
        assert!(call("w3-a", "b/../c", "m", b"").is_err());
        assert!(call("w3-a", "w3-b", "", b"").is_err());
        assert!(call("w3-a", "w3-b", "echo", &vec![0u8; (1 << 20) + 1]).unwrap_err().contains("1 MiB"));
        assert!(call("w3-a", "w3-a", "echo", b"").unwrap_err().contains("itself"));
        assert!(call("w3-a", "no-such-package", "echo", b"").unwrap_err().contains("no package"));
        // A nested call from inside a handler (b relays to c): the guest calls the service while b runs.
        assert_eq!(String::from_utf8(call("w3-a", "w3-b", "relay", b"w3-c").unwrap()).unwrap(), "w3-b said: relayed");
        // A callee that is busy (waiting, mid-hook) is refused, not waited for; so is a re-entry into a waiting caller.
        let held = b.client.lock().unwrap();
        assert!(call("w3-a", "w3-b", "echo", b"x").unwrap_err().contains("busy"));
        drop(held);
        let held = c.client.lock().unwrap();
        assert!(call("w3-a", "w3-b", "relay", b"w3-c").unwrap_err().contains("busy"));
        drop(held);
        assert!(call("w3-a", "w3-b", "echo", b"x").is_ok());
        let _ = &a;
    }

    #[test]
    fn a_guest_chooses_an_item_models_select_case() {
        let _serial = super::super::GUEST_TEST_LOCK.lock().unwrap_or_else(|error| error.into_inner());
        let Some(bytes) = module("client-peer") else { return };
        let module = load_client(&ModuleSource { id: "w3-select", bytes: &bytes, commands: false, gpu: false }).expect("client peer loads");
        let mod_ = FullMod::new("w3-select".into(), Some(module), None);
        let host = FakeHost::default();
        let context = ItemModelContext { display: DisplayContext::Gui, owner: None, level: None, seed: 0 };
        let components = Components::new();
        let ask = |property: &str, item: &str, mut value: Option<serde_json::Value>| {
            let stack = StackView::new(item, 1);
            mod_.item_model_select_v2(
                &host,
                &mut ItemModelSelectV2Event { property, stack: &stack, components: &components, context: &context, value: &mut value },
            );
            value
        };
        // Its own property: a sword is `worn`, the case reaches the resolver as a JSON string.
        assert_eq!(ask("full-demo-peer:wear", "minecraft:iron_sword", None), Some(serde_json::Value::String("worn".into())));
        // Not a sword, or not its property: what v1 answered is kept, answered or not.
        assert_eq!(ask("full-demo-peer:wear", "minecraft:apple", None), None);
        let kept = Some(serde_json::Value::String("v1".into()));
        assert_eq!(ask("full-demo-peer:wear", "minecraft:apple", kept.clone()), kept);
        assert_eq!(ask("other:wear", "minecraft:iron_sword", kept.clone()), kept);
    }

    #[test]
    fn a_guest_hears_keys_with_state_and_modifiers_and_pad_buttons_and_axes() {
        let _serial = super::super::GUEST_TEST_LOCK.lock().unwrap_or_else(|error| error.into_inner());
        let Some(bytes) = module("client-peer") else { return };
        let module = load_client(&ModuleSource { id: "w3-input", bytes: &bytes, commands: false, gpu: false }).expect("client peer loads");
        let mod_ = FullMod::new("w3-input".into(), Some(module), None);
        let mut host = FakeHost::default();
        // Down, repeat and up of one key with shift + control, and a key with no vanilla name that typed text.
        for (key, text, pressed, repeat, modifiers) in [
            ("key.keyboard.z", "Z", true, false, 3),
            ("key.keyboard.z", "Z", true, true, 3),
            ("key.keyboard.z", "", false, false, 0),
            ("", "q", true, false, 0),
        ] {
            mod_.input_key_v2(&mut host, &mut InputKeyV2Event { key, text, pressed, repeat, modifiers });
        }
        mod_.input_pad_button(&mut host, &mut InputPadButtonEvent { button: "south", pressed: true });
        mod_.input_pad_button(&mut host, &mut InputPadButtonEvent { button: "lb", pressed: false });
        mod_.input_pad_axes(
            &mut host,
            &mut InputPadAxesEvent { left_x: 0.5, left_y: -1.0, right_x: 0.0, right_y: 0.25, left_trigger: 0.0, right_trigger: 1.0 },
        );
        let heard: Vec<String> = host.sounds.borrow().iter().map(|(id, ..)| id.clone()).collect();
        assert_eq!(
            heard,
            [
                "in:key=key.keyboard.z;text=Z;down=true;repeat=false;mods=3",
                "in:key=key.keyboard.z;text=Z;down=true;repeat=true;mods=3",
                "in:key=key.keyboard.z;text=;down=false;repeat=false;mods=0",
                "in:key=;text=q;down=true;repeat=false;mods=0",
                "in:pad=south;down=true",
                "in:pad=lb;down=false",
                "in:axes=0.5,-1,0,0.25,0,1",
            ]
        );
    }

    #[test]
    fn guest_commands_are_gated_by_permission_and_a_denylist() {
        assert!(
            command_gate("say hi", false)
                .unwrap_err()
                .contains("service.commands")
        );
        assert!(command_gate("say hi", true).is_ok());
        assert!(command_gate("/give @a minecraft:stone", true).is_ok());
        for denied in [
            "transfer example.com 25565",
            "fetchprofile name Steve",
            "publish",
            "unpublish",
            "debug start",
            "jfr start",
            "perf start",
            "stop",
            "save-off",
            "op Steve",
            "whitelist add Steve",
            "ban Steve",
            "/stop",
            "STOP",
            "minecraft:stop",
            "execute as @a run stop",
            "execute as @a at @s run execute if entity @e run transfer a 1",
            "return run stop",
        ] {
            assert!(command_gate(denied, true).is_err(), "{denied}");
        }
        // Not a command at all, or no longer a single line.
        assert!(command_gate("say hi\nstop", true).is_err());
        assert!(command_gate("", true).is_err());
        assert!(command_gate(&"a".repeat(abi::MAX_TEXT_BYTES + 1), true).is_err());
        // The host refuses before it touches any service.
        let mut host = GameServerHost {
            id: "m".into(),
            commands: false,
        };
        assert!(
            host.run_command("say hi".into())
                .unwrap_err()
                .contains("service.commands")
        );
        let mut host = GameServerHost {
            id: "m".into(),
            commands: true,
        };
        assert!(
            host.run_command("stop".into())
                .unwrap_err()
                .contains("not available")
        );
    }

    #[test]
    fn a_guest_command_never_reaches_another_mods_command_hook() {
        assert!(!crate::mod_api::in_guest_command());
        {
            let _scope = crate::mod_api::guest_command_scope();
            assert!(crate::mod_api::in_guest_command());
            {
                let _inner = crate::mod_api::guest_command_scope();
            }
            assert!(crate::mod_api::in_guest_command(), "scopes nest");
        }
        assert!(!crate::mod_api::in_guest_command());
    }

    #[test]
    fn guest_visible_paths_are_relative_to_the_game_folder() {
        let game = std::env::current_dir().unwrap();
        assert_eq!(game_relative(&game.join("world")).as_deref(), Some("world"));
        assert_eq!(
            game_relative(&game.join("config").join("mods")).as_deref(),
            Some("config/mods")
        );
        assert_eq!(game_relative(&game).as_deref(), Some("."));
        // Elsewhere: only the last component, never the account's folder.
        let elsewhere = std::path::Path::new("C:/Users/Someone/Documents/server/world");
        let shown = game_relative(elsewhere).unwrap();
        assert!(
            !shown.contains("Someone") && !shown.contains(':'),
            "{shown}"
        );
    }

    #[test]
    fn guest_answers_are_checked_for_meaning() {
        // Item stacks: syntax, count, size.
        let stack = |id: &str, count: u32| et::ItemStack {
            id: id.into(),
            count,
            components: Vec::new(),
        };
        assert!(item_from(&stack("minecraft:stone", 1)).is_some());
        assert!(item_from(&stack("minecraft:stone", 0)).is_none());
        assert!(item_from(&stack("minecraft:stone", MAX_STACK_COUNT + 1)).is_none());
        assert!(item_from(&stack("", 1)).is_none());
        assert!(item_from(&stack("stone", 1)).is_none());
        assert!(item_from(&stack("Minecraft:Stone", 1)).is_none());
        assert!(item_from(&stack(&format!("minecraft:{}", "a".repeat(MAX_ITEM_ID)), 1)).is_none());
        let big = et::ItemStack {
            id: "minecraft:stone".into(),
            count: 1,
            components: vec![0; MAX_ITEM_COMPONENTS + 1],
        };
        assert!(item_from(&big).is_none());
        // Floats and lengths.
        assert_eq!(finite_f32(1.5), Some(1.5));
        assert!(finite_f32(f32::NAN).is_none() && finite_f32(f32::INFINITY).is_none());
        assert!(finite_f64(f64::NEG_INFINITY).is_none());
        assert!(bounded_string("x".repeat(MAX_ANSWER_STRING)).is_some());
        assert!(bounded_string("x".repeat(MAX_ANSWER_STRING + 1)).is_none());
        // Block states: the registry's range.
        assert!(valid_block_state(0));
        assert!(!valid_block_state(u32::MAX));
    }

    #[test]
    fn leaked_strings_are_capped_in_size_count_and_total() {
        assert_eq!(leak_static("wasmping".into()), Some("wasmping"));
        assert!(
            std::ptr::eq(
                leak_static("wasmping".into()).unwrap(),
                leak_static("wasmping".into()).unwrap()
            ),
            "deduplicated"
        );
        assert!(leak_static("x".repeat(LEAK_MAX_LEN + 1)).is_none());
        assert!(leak_static("bad\nline".into()).is_none());
        // A guest cannot make the process keep more than the pool allows.
        let mut accepted = 0;
        for index in 0..(LEAK_MAX_COUNT + 100) {
            if leak_static(format!("guest-string-{index}")).is_some() {
                accepted += 1;
            }
        }
        assert!(accepted <= LEAK_MAX_COUNT, "{accepted}");
        assert!(leak_static("one-more-new-string".into()).is_none());
    }

    #[test]
    fn a_failed_encoding_skips_the_hook_instead_of_sending_an_empty_value() {
        assert!(!take_encode_failure());
        // NBT nested past the encoder's depth limit does not encode.
        let mut deep = Tag::Int(1);
        for _ in 0..600 {
            deep = Tag::List(vec![deep]);
        }
        let bytes = nbt_tag(&deep);
        if bytes.is_empty() {
            assert!(take_encode_failure());
            assert!(!take_encode_failure(), "the mark is cleared once read");
        } else {
            // This build's encoder accepts the depth: the mark must stay clear.
            assert!(!take_encode_failure());
        }
    }
}
