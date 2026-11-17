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
    server: Arc<ServerSlot>,
    /// One bit per catalog hook, per side: the guest subscribed and has not been disabled.
    masks: Arc<[[AtomicU64; MASK_WORDS]; 2]>,
    /// Hooks dropped because the module was busy, per side.
    dropped: [AtomicU64; 2],
    client_reload: Option<Arc<Reload>>,
    /// What starts the server module afresh at each world start.
    server_source: Option<ServerSource>,
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
        Self {
            id,
            client: Arc::new(Mutex::new(SideState {
                module: client,
                reported: false,
            })),
            gpu_owner: None,
            gpu_version: String::new(),
            server,
            masks: Arc::new([client_mask, server_mask]),
            dropped: [AtomicU64::new(0), AtomicU64::new(0)],
            client_reload: None,
            server_source: None,
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
        match load_server(&source.module()) {
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
        if side == Side::Client {
            self.reload_client();
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

fn nbt_compound_from(bytes: &[u8]) -> Option<Compound> {
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
                if quad.corners.len() == 4 && quad.sprite.is_none() {
                    let p =
                        |i: usize| [gui_scalar(quad.corners[i].0), gui_scalar(quad.corners[i].1)];
                    host.quad(batch, [p(0), p(1), p(2), p(3)], quad.color);
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
            et::HudCommand::Texture(_) | et::HudCommand::Highlight(_) => {}
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
            // Cubes, standalone models and items need engine objects the guest cannot name here.
            et::ModelCommand::Cube(_) | et::ModelCommand::Model(_) | et::ModelCommand::Item(_) => {}
        }
    }
    while depth > 0 {
        depth -= 1;
        draw.pop();
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
    fn enter(owner: &str) -> Self {
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
    // A full module's `ItemStack` answers are checked against the item table, so it is published.
    horizon_items::items::publish_registry_for_checks();
    let mut sandbox = sandbox().lock().unwrap_or_else(|error| error.into_inner());
    let mut host = GameHost::new(source.id).with_gpu(source.gpu);
    host.dev_shaders = shaders;
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
        std::fs::remove_dir_all(&host.root).unwrap();
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
    }

    impl Host for FakeHost {
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
        fn fill(&self, _batch: &mut HudBatch, _rect: [f32; 4], _argb: u32) {
            *self.fills.borrow_mut() += 1;
        }
        fn tell(&mut self, uuid: u128, line: Text) -> bool {
            self.told
                .borrow_mut()
                .push(format!("{uuid:032x}:{}", line.plain()));
            true
        }
    }

    fn module(kind: &str) -> Option<Vec<u8>> {
        use yoake_wasm::test_guests::{FULL_DEMO_CLIENT, FULL_DEMO_PEER, FULL_DEMO_SERVER, load};
        load(match kind {
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
