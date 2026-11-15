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

use super::*;
use crate::gfx::native_render::*;
use horizon_client_core::text::Text;
use horizon_items::items::ItemStack;
use horizon_world::nbt::{Compound, Tag};
use yoake_inject_rt::command_gate;
use yoake_wasm::full_bindings::client::yoake::mods::engine_types as et;
use yoake_wasm::{ClientFullMod, ServerFullMod, ServerHost, convert, server_types as st};

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

struct SideState<M> {
    module: Option<M>,
    reported: bool,
}

pub(super) struct FullMod {
    id: String,
    client: Mutex<SideState<ClientFullMod>>,
    server: Mutex<SideState<ServerFullMod>>,
    /// One bit per catalog hook, per side: the guest subscribed and has not been disabled.
    masks: [[AtomicU64; MASK_WORDS]; 2],
    /// Hooks dropped because the module was busy, per side.
    dropped: [AtomicU64; 2],
    /// The client module subscribed to a render hook: only then does this mod take part in the
    /// render pass (`Hooks::render`), so a package without one costs the frame nothing.
    render_wanted: bool,
    /// What starts the server module afresh at each world start.
    server_source: Option<ServerSource>,
    /// The server module ran a hook since it was created.
    server_dirty: std::sync::atomic::AtomicBool,
}

/// The bytes of a server module and its grants, kept so every world starts with a fresh instance.
pub(super) struct ServerSource {
    id: String,
    bytes: Arc<Vec<u8>>,
    commands: bool,
}

impl ServerSource {
    pub(super) fn new(id: &str, bytes: Vec<u8>, commands: bool) -> Self {
        Self { id: id.to_owned(), bytes: Arc::new(bytes), commands }
    }

    pub(super) fn module(&self) -> ModuleSource<'_> {
        ModuleSource { id: &self.id, bytes: &self.bytes, commands: self.commands }
    }
}

use std::sync::atomic::AtomicU64;

fn mask_of(subscribed: impl Fn(&str) -> bool) -> [AtomicU64; MASK_WORDS] {
    let words: [AtomicU64; MASK_WORDS] = std::array::from_fn(|_| AtomicU64::new(0));
    for (ordinal, (name, _)) in yoake_wasm::FULL_HOOK_LIST.iter().enumerate() {
        if subscribed(name) {
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
    pub(super) fn new(id: String, client: Option<ClientFullMod>, server: Option<ServerFullMod>) -> Self {
        let client_mask = match &client {
            Some(module) => mask_of(|name| module.subscribed(name)),
            None => mask_of(|_| false),
        };
        let server_mask = match &server {
            Some(module) => mask_of(|name| module.subscribed(name)),
            None => mask_of(|_| false),
        };
        let render_wanted = RENDER_HOOKS.iter().any(|hook| client_mask[hook / 64].load(Ordering::Relaxed) >> (hook % 64) & 1 != 0);
        Self {
            id,
            client: Mutex::new(SideState { module: client, reported: false }),
            server: Mutex::new(SideState { module: server, reported: false }),
            masks: [client_mask, server_mask],
            dropped: [AtomicU64::new(0), AtomicU64::new(0)],
            render_wanted,
            server_source: None,
            server_dirty: std::sync::atomic::AtomicBool::new(false),
        }
    }

    pub(super) fn with_server_source(mut self, source: ServerSource) -> Self {
        self.server_source = Some(source);
        self
    }

    /// Whether this package takes part in the render pass.
    fn render_wanted(&self) -> bool {
        self.render_wanted
    }

    /// How many hooks of `side` were dropped because the module was busy.
    #[allow(dead_code)]
    pub(super) fn dropped_hooks(&self, side: Side) -> u64 {
        self.dropped[side as usize].load(Ordering::Relaxed)
    }

    /// A hook found the module busy: its answer is the hook's fallback, and the drop is counted.
    fn busy(&self, side: Side, hook: usize) {
        if self.dropped[side as usize].fetch_add(1, Ordering::Relaxed) == 0 {
            let name = yoake_wasm::FULL_HOOK_LIST.get(hook).map_or("?", |(name, _)| *name);
            log::warn!(
                "wasm mod {} ({}): hook {name} dropped, the module was busy or re-entered; the hook's fallback applies (further drops are only counted)",
                self.id, side_name(side)
            );
        }
    }

    /// Starts the server module afresh when it already served a world (the server runs one world at
    /// a time and a guest keeps its state in its instance): called as a world's server starts.
    fn reset_server(&self) {
        let Some(source) = &self.server_source else { return };
        if !self.server_dirty.swap(false, Ordering::Relaxed) { return; }
        let Ok(mut state) = self.server.try_lock() else { return };
        match load_server(&source.module()) {
            Ok(module) => {
                for (word, fresh) in self.masks[Side::Server as usize].iter().zip(mask_of(|name| module.subscribed(name))) {
                    word.store(fresh.load(Ordering::Relaxed), Ordering::Relaxed);
                }
                state.module = Some(module);
                state.reported = false;
                log::info!("wasm mod {} (server): started afresh for the new world", self.id);
            }
            Err(error) => {
                state.module = None;
                self.disable(Side::Server);
                log::error!("wasm mod {} (server) could not start afresh and is off: {error}", self.id);
            }
        }
    }

    /// Whether this module's `side` instance subscribed to the hook with this catalog ordinal.
    #[inline]
    fn wants(&self, side: Side, hook: usize) -> bool {
        self.masks[side as usize][hook / 64].load(Ordering::Relaxed) >> (hook % 64) & 1 != 0
    }

    fn disable(&self, side: Side) {
        for word in &self.masks[side as usize] {
            word.store(0, Ordering::Relaxed);
        }
    }

    fn with_client<'call, R>(
        &self,
        hook: usize,
        host: HostAccess<'call>,
        batch: Option<&'call mut HudBatch>,
        call: impl FnOnce(&mut ClientFullMod) -> Option<R>,
    ) -> Option<R> {
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
        let mut invocation = Invocation { host, batch, batch_start };
        let result = {
            let _scope = Scope::enter(&mut invocation);
            call(module)
        };
        if module.is_disabled() {
            self.disable(Side::Client);
            if !state.reported {
                state.reported = true;
                log::error!("wasm mod {} (client) disabled: trapped or exceeded its limits (see the line above)", self.id);
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
        let mut state = match self.server.try_lock() {
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
        self.server_dirty.store(true, Ordering::Relaxed);
        let batch_start = batch.as_deref().map_or(0, |batch| batch.vertices.len());
        let mut invocation = Invocation { host, batch, batch_start };
        let result = {
            let _scope = Scope::enter(&mut invocation);
            call(module)
        };
        if module.is_disabled() {
            self.disable(Side::Server);
            if !state.reported {
                state.reported = true;
                log::error!("wasm mod {} (server) disabled: trapped or exceeded its limits (see the line above)", self.id);
            }
        }
        result
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
val!(bool, char, i8, i16, i32, i64, i128, isize, u8, u16, u32, u64, u128, usize, f32, f64);

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
        let name = yoake_wasm::FULL_HOOK_LIST.get(hook).map_or("?", |(name, _)| *name);
        log::warn!("wasm: hook {name} skipped, an NBT argument did not encode (further skips are only counted)");
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

fn item_w(stack: &ItemStack) -> et::ItemStack {
    match convert::item_to_wire(stack) {
        Ok(wire) => et::ItemStack { id: wire.id, count: wire.count, components: wire.components },
        Err(_) => et::ItemStack { id: String::new(), count: 0, components: Vec::new() },
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
fn valid_item_id(id: &str) -> bool {
    let Some((namespace, path)) = id.split_once(':') else { return false };
    id.len() <= MAX_ITEM_ID
        && !namespace.is_empty()
        && !path.is_empty()
        && namespace.bytes().all(|b| b.is_ascii_lowercase() || b.is_ascii_digit() || matches!(b, b'_' | b'-' | b'.'))
        && path.bytes().all(|b| b.is_ascii_lowercase() || b.is_ascii_digit() || matches!(b, b'_' | b'-' | b'.' | b'/'))
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

fn item_from(stack: &et::ItemStack) -> Option<ItemStack> {
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
pub(super) fn apply_hud(host: Option<&dyn Host>, batch: &mut HudBatch, commands: &[et::HudCommand]) {
    let Some(host) = host else { return };
    let mut pending: Vec<(Pending, usize)> = Vec::new();
    for command in commands.iter().take(MAX_COMMANDS) {
        match command {
            et::HudCommand::Text(text) => {
                let line = text_from(&text.text).map(|text| text.plain()).unwrap_or_default();
                if line.len() <= abi::MAX_TEXT_BYTES {
                    host.text(batch, &line, [gui_scalar(text.x), gui_scalar(text.y)], text.color & 0x00ff_ffff, text.shadow);
                }
            }
            et::HudCommand::Rect(rect) => {
                host.fill(batch, [gui_scalar(rect.x), gui_scalar(rect.y), gui_scalar(rect.width), gui_scalar(rect.height)], rect.color);
            }
            et::HudCommand::Gradient(gradient) => {
                let (x, y, w, h) = (gui_scalar(gradient.x), gui_scalar(gradient.y), gui_scalar(gradient.width), gui_scalar(gradient.height));
                // The game's fill has no gradient service: strips of at most 2 pixels.
                let strips = (h / 2.0).ceil().clamp(1.0, 128.0) as u32;
                let step = h / strips as f32;
                for index in 0..strips {
                    let t = if strips == 1 { 0.0 } else { index as f32 / (strips - 1) as f32 };
                    host.fill(batch, [x, y + step * index as f32, w, step.max(1.0)], mix(gradient.top, gradient.bottom, t));
                }
            }
            et::HudCommand::Quad(quad) => {
                if quad.corners.len() == 4 && quad.sprite.is_none() {
                    let p = |i: usize| [gui_scalar(quad.corners[i].0), gui_scalar(quad.corners[i].1)];
                    host.quad(batch, [p(0), p(1), p(2), p(3)], quad.color);
                }
            }
            et::HudCommand::Sprite(sprite) => {
                let (width, height) = sprite.size.unwrap_or((16.0, 16.0));
                let src = match &sprite.region {
                    Some(region) => [region.x, region.y, region.width, region.height],
                    None => [0.0, 0.0, gui_scalar(width), gui_scalar(height)],
                };
                host.sprite(batch, &sprite.name, src, [gui_scalar(sprite.x), gui_scalar(sprite.y), gui_scalar(width), gui_scalar(height)]);
            }
            et::HudCommand::Item(item) => {
                if let Some(stack) = item_from(&item.stack) {
                    host.item_stack(batch, &stack, [gui_scalar(item.x), gui_scalar(item.y)], 16.0);
                }
            }
            et::HudCommand::Texture(_) | et::HudCommand::Highlight(_) => {}
            et::HudCommand::BeginClip(bounds) => {
                pending.push((Pending::Clip([bounds.x0, bounds.y0, bounds.x1, bounds.y1]), batch.vertices.len()));
            }
            et::HudCommand::BeginScale(scale) => {
                pending.push((Pending::Scale([scale.center_x, scale.center_y, scale.scale_x, scale.scale_y]), batch.vertices.len()));
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
                let Some((run, start)) = pending.pop() else { continue };
                match (command, run) {
                    (et::HudCommand::EndClip, Pending::Clip(b)) => batch.clip_since(start, b[0], b[1], b[2], b[3]),
                    (et::HudCommand::EndScale, Pending::Scale(s)) => batch.scale_since(start, s[0], s[1], s[2], s[3]),
                    (et::HudCommand::EndBlend, Pending::Blend(blend)) => batch.blend_since(start, blend),
                    // A mismatched end closes nothing.
                    _ => {}
                }
            }
        }
    }
}

/// Applies a guest's pose-stack commands to a block entity, mob, player or item draw.
pub(super) fn apply_model(host: Option<&dyn Host>, draw: &mut BlockEntityDraw, commands: &[et::ModelCommand]) {
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
                    host.draw_quad(draw, &quad.sprite, [c(0), c(1), c(2), c(3)], [quad.uv[0], quad.uv[1], quad.uv[2], quad.uv[3]], quad.tint);
                }
            }
            et::ModelCommand::Block(block) => {
                let Some(host) = host else { continue };
                let properties: Vec<(&str, &str)> = block.properties.iter().take(64).map(|(k, v)| (k.as_str(), v.as_str())).collect();
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
        corners.place(corner, gui_scalar(reserve.width), gui_scalar(reserve.height));
    }
}

/// Frame-stage commands (whole-frame fills, tints, vignettes) need the render target, which is only
/// reachable inside the renderer: they are not applied yet.
pub(super) fn apply_frame_stage(commands: &[et::FrameCommand]) {
    static REPORTED: std::sync::atomic::AtomicBool = std::sync::atomic::AtomicBool::new(false);
    if !commands.is_empty() && !REPORTED.swap(true, Ordering::Relaxed) {
        log::warn!("wasm: frame-stage commands are not applied yet (this build has no render-target service)");
    }
}

// ---------------------------------------------------------------------------------------------
// Server services
// ---------------------------------------------------------------------------------------------

/// The server services a full-catalog module may call, forwarded to the game's native services. The
/// ones the game has no counterpart for answer "unsupported" (see `docs/yoake-wasm-dev.md`).
struct GameServerHost {
    id: String,
    /// The package declared `service.commands`.
    commands: bool,
}

/// A path as the guest may see it: relative to the game folder (the server's working directory), or
/// just its last component when it lies elsewhere. Absolute paths carry the user's account name.
fn game_relative(path: &std::path::Path) -> Option<String> {
    let relative = std::env::current_dir()
        .ok()
        .and_then(|game| path.strip_prefix(game).ok().map(std::path::Path::to_path_buf))
        .or_else(|| path.file_name().map(std::path::PathBuf::from))?;
    let text = relative.to_str()?.replace('\\', "/");
    Some(if text.is_empty() { ".".to_owned() } else { text })
}

fn uuid_text(uuid: u128) -> String {
    let hex = format!("{uuid:032x}");
    format!("{}-{}-{}-{}-{}", &hex[0..8], &hex[8..12], &hex[12..16], &hex[16..20], &hex[20..32])
}

fn parse_uuid(text: &str) -> Result<u128, String> {
    let digits: String = text.chars().filter(|c| *c != '-').collect();
    if digits.len() != 32 {
        return Err(format!("not a player UUID: {text}"));
    }
    u128::from_str_radix(&digits, 16).map_err(|_| format!("not a player UUID: {text}"))
}

fn st_player(player: &NativeServerPlayer) -> st::Player {
    st::Player {
        dimension: player.dimension.clone(),
        game_mode: player.game_mode,
        health: player.health,
        name: player.name.clone(),
        permission_level: player.permission,
        pitch: player.pitch,
        position: st::Vec3 { x: player.position[0], y: player.position[1], z: player.position[2] },
        uuid: uuid_text(player.uuid),
        yaw: player.yaw,
    }
}

fn server_service<R>(operation: impl FnOnce(&mut dyn Host) -> Result<R, String>) -> Result<R, String> {
    service(|invocation| {
        let Some(host) = invocation.host.write() else { return Ok(Err("unsupported: not inside a server hook".to_owned())) };
        Ok(operation(host))
    })
    .map_err(|error| format!("{error:?}"))?
}

fn read_service<R>(operation: impl FnOnce(&dyn Host) -> Result<R, String>) -> Result<R, String> {
    service(|invocation| {
        let Some(host) = invocation.host.read() else { return Ok(Err("unsupported: not inside a server hook".to_owned())) };
        Ok(operation(host))
    })
    .map_err(|error| format!("{error:?}"))?
}

fn checked(done: bool, what: &str) -> Result<(), String> {
    if done { Ok(()) } else { Err(format!("{what} did not apply")) }
}

fn message_text(message: &str) -> Result<Text, String> {
    if message.len() > abi::MAX_TEXT_BYTES {
        return Err("message too long".into());
    }
    Ok(Text::literal(message))
}

impl ServerHost for GameServerHost {
    fn log(&mut self, target: &str, message: &str) -> Result<(), w::HostError> {
        let clean = |text: &str| -> String {
            text.chars().take(MAX_LOG_CHARS).map(|c| if c.is_control() { ' ' } else { c }).collect()
        };
        emit(format!("wasm mod {} [{}]: {}", self.id, clean(target), clean(message)));
        Ok(())
    }

    fn acting_player(&mut self) -> Option<st::Player> {
        read_service(|host| Ok(host.acting_player())).ok().flatten().map(|player| st_player(&player))
    }

    fn server_players(&mut self) -> Result<Vec<st::Player>, String> {
        read_service(|host| Ok(host.server_players().iter().map(st_player).collect()))
    }

    fn tell(&mut self, message: String, player: String) -> Result<(), String> {
        let (uuid, text) = (parse_uuid(&player)?, message_text(&message)?);
        server_service(|host| checked(host.tell(uuid, text), "tell"))
    }

    fn action_bar(&mut self, message: String, player: String) -> Result<(), String> {
        let (uuid, text) = (parse_uuid(&player)?, message_text(&message)?);
        server_service(|host| checked(host.action_bar(uuid, text), "action bar"))
    }

    fn player_set_health(&mut self, health: f32, player: String) -> Result<(), String> {
        let uuid = parse_uuid(&player)?;
        server_service(|host| checked(host.player_set_health(uuid, health), "set health"))
    }

    fn player_title(&mut self, fade_in: i32, fade_out: i32, player: String, stay: i32, subtitle: Option<String>, title: Option<String>) -> Result<(), String> {
        let uuid = parse_uuid(&player)?;
        server_service(|host| {
            checked(host.player_title(uuid, title.as_deref(), subtitle.as_deref(), fade_in, stay, fade_out), "title")
        })
    }

    fn bossbar_create(&mut self, bar: String, name: String) -> Result<(), String> {
        server_service(|host| checked(host.bossbar_create(&bar, &name), "boss bar"))
    }

    fn run_command(&mut self, command: String) -> Result<bool, String> {
        command_gate(&command, self.commands)?;
        // The guest's line goes to the game's own commands, never to another mod's command hook.
        let _guest = crate::mod_api::guest_command_scope();
        server_service(|host| Ok(host.run_command(&command)))
    }

    fn block(&mut self, position: st::BlockPos) -> Result<String, String> {
        read_service(|host| Ok(host.block([position.x, position.y, position.z])))
    }

    fn block_property(&mut self, position: st::BlockPos, property: String) -> Result<Option<String>, String> {
        read_service(|host| Ok(host.block_property([position.x, position.y, position.z], &property)))
    }

    fn net_send_to_player(&mut self, channel: String, payload: Vec<u8>, player: String) -> Result<(), String> {
        let uuid = parse_uuid(&player)?;
        if channel.len() > 256 || payload.len() > abi::MAX_DATA_BYTES {
            return Err("packet too large".into());
        }
        server_service(|host| host.net_send_to_player(uuid, &channel, &payload))
    }

    fn world_dir(&mut self) -> Option<String> {
        read_service(|host| Ok(host.world_dir())).ok().flatten().and_then(|path| game_relative(&path))
    }

    fn config_dir(&mut self) -> Result<String, String> {
        read_service(|host| host.config_dir()).and_then(|path| game_relative(&path).ok_or_else(|| "path is not text".to_owned()))
    }

    fn qa_mode_enabled(&mut self) -> bool {
        false
    }
}

/// The server services this build forwards (the rest answer "unsupported").
pub(super) const FORWARDED_SERVER_SERVICES: &[&str] = &[
    "acting_player", "server_players", "tell", "action_bar", "player_set_health", "player_title",
    "bossbar_create", "run_command", "block", "block_property", "net_send_to_player", "world_dir", "config_dir",
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
}

pub(super) fn load_client(source: &ModuleSource<'_>) -> Result<ClientFullMod, String> {
    // A full module's `ItemStack` answers are checked against the item table, so it is published.
    horizon_items::items::publish_registry_for_checks();
    let mut sandbox = sandbox().lock().unwrap_or_else(|error| error.into_inner());
    ClientFullMod::instantiate(&mut sandbox, source.id, source.bytes, Box::new(GameHost::new(source.id)), UNWIRED)
        .map_err(|error| error.to_string())
}

pub(super) fn load_server(source: &ModuleSource<'_>) -> Result<ServerFullMod, String> {
    horizon_items::items::publish_registry_for_checks();
    let mut sandbox = sandbox().lock().unwrap_or_else(|error| error.into_inner());
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
mod tests {
    use super::*;
    use std::cell::RefCell;

    #[derive(Default)]
    struct FakeHost {
        texts: RefCell<Vec<String>>,
        fills: RefCell<u32>,
        told: RefCell<Vec<String>>,
    }

    impl Host for FakeHost {
        fn window(&self) -> bool { true }
        fn text(&self, _batch: &mut HudBatch, text: &str, _at: [f32; 2], _rgb: u32, _shadow: bool) {
            self.texts.borrow_mut().push(text.to_owned());
        }
        fn fill(&self, _batch: &mut HudBatch, _rect: [f32; 4], _argb: u32) {
            *self.fills.borrow_mut() += 1;
        }
        fn tell(&mut self, uuid: u128, line: Text) -> bool {
            self.told.borrow_mut().push(format!("{uuid:032x}:{}", line.plain()));
            true
        }
    }

    fn module(kind: &str) -> Option<Vec<u8>> {
        use yoake_wasm::test_guests::{FULL_DEMO_CLIENT, FULL_DEMO_SERVER, load};
        load(if kind == "server" { &FULL_DEMO_SERVER } else { &FULL_DEMO_CLIENT })
    }

    fn client() -> Option<FullMod> {
        let bytes = module("client")?;
        let client = load_client(&ModuleSource { id: "demo", bytes: &bytes, commands: false }).expect("client-full loads");
        Some(FullMod::new("demo".into(), Some(client), None))
    }

    #[test]
    fn items_and_nbt_convert_both_ways() {
        let mut stack = ItemStack::new("minecraft:diamond_sword", 1);
        stack.enchant("minecraft:sharpness", 5);
        let wire = item_w(&stack);
        assert_eq!(wire.id, "minecraft:diamond_sword");
        assert!(!wire.components.is_empty());
        assert_eq!(item_from(&wire).unwrap().enchantment_level("minecraft:sharpness"), 5);
        let tag = Tag::compound([("n", Tag::Int(7)), ("s", Tag::String("x".into()))]);
        assert_eq!(nbt_tag_from(&nbt_tag(&tag)), Some(tag));
        assert!(nbt_tag_from(&[99]).is_none());
    }

    #[test]
    fn hud_commands_are_applied_through_the_host_services() {
        let host = FakeHost::default();
        let mut batch = HudBatch::new([854, 480]);
        let commands = vec![
            et::HudCommand::Rect(et::HudRect { x: 0.0, y: 0.0, width: 10.0, height: 10.0, color: 0xff000000 }),
            et::HudCommand::Text(et::HudText { text: "{\"text\":\"hi\"}".into(), x: 1.0, y: 1.0, color: 0xffffff, shadow: true }),
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
        let Some(mod_) = client() else { return };
        let mut host = FakeHost::default();
        let mut batch = HudBatch::new([854, 480]);
        // Cancel: the demo hides `hotbar` only.
        let mut hotbar = GuiLayerPreEvent { layer: "hotbar", batch: &mut batch };
        assert!(matches!(mod_.gui_layer_pre(&mut host, &mut hotbar), Flow::Cancel));
        let mut chat = GuiLayerPreEvent { layer: "chat", batch: &mut batch };
        assert!(matches!(mod_.gui_layer_pre(&mut host, &mut chat), Flow::Continue));
        // Command list: the hud hook's rect, gradient strips and text.
        let mut corners = Corners::new(854.0, 480.0, 0.0);
        mod_.hud(&mut host, &mut HudEvent { batch: &mut batch, corners: &mut corners });
        assert!(*host.fills.borrow() >= 2);
        assert_eq!(host.texts.borrow().len(), 1);
        // Mutate outcome: the demo adds 10 degrees to the field of view.
        let mut fov = ComputeFovEvent { partial: 0.0, camera: [0.0; 3], degrees: 70.0 };
        mod_.compute_fov(&host, &mut fov);
        assert_eq!(fov.degrees, 80.0);
        // Collect with an item stack in and text out.
        let stack = ItemStack::new("minecraft:stone", 3);
        let mut lines = vec![(Text::literal("Stone"), 0xffffff)];
        mod_.tooltip(&host, &mut TooltipEvent { stack: &stack, lines: &mut lines });
        assert_eq!(lines.len(), 2);
        assert!(lines[1].0.plain().contains("minecraft:stone x3"));
    }

    #[test]
    fn a_server_guest_runs_server_tick_and_perform_command_on_the_server_thread() {
        let Some(bytes) = module("server") else { return };
        let server = load_server(&ModuleSource { id: "demo", bytes: &bytes, commands: false }).expect("server-full loads");
        let mod_ = Arc::new(FullMod::new("demo".into(), None, Some(server)));
        let runner = Arc::clone(&mod_);
        // The server loop's scope, not the thread's name, routes a shared hook to the server module.
        let handle = std::thread::Builder::new().name("not called Server thread".into()).spawn(move || {
            let _server = crate::mod_guard::enter_thread(crate::mod_guard::ModThread::Server);
            let mut host = FakeHost::default();
            let player = NativeServerPlayer {
                uuid: 0x1122_3344_5566_7788_99aa_bbcc_ddee_ff00, name: "Guest".into(), permission: 0,
                dimension: "minecraft:overworld".into(), position: [0.0; 3], game_mode: 1, health: 20.0, yaw: 0.0, pitch: 0.0,
            };
            let taken = runner.perform_command(&mut host, &mut PerformCommandEvent { player: &player, command: "wasmping" });
            let other = runner.perform_command(&mut host, &mut PerformCommandEvent { player: &player, command: "other" });
            let mut commands = Vec::new();
            runner.register_commands(&mut host, &mut RegisterCommandsEvent { commands: &mut commands });
            let told = host.told.borrow().clone();
            (matches!(taken, Flow::Cancel), matches!(other, Flow::Continue), commands, told)
        }).unwrap();
        let (taken, other_continues, commands, told) = handle.join().unwrap();
        assert!(taken && other_continues);
        assert_eq!(commands.iter().map(|c| c.usage.as_str()).collect::<Vec<_>>(), ["wasmping"]);
        assert_eq!(told.len(), 1);
        assert!(told[0].starts_with("112233445566778899aabbccddeeff00:pong #1"), "{told:?}");
        // On a thread that did not enter the server scope a hook of the shared code does not reach the server module.
        let mut host = FakeHost::default();
        let player = NativeServerPlayer {
            uuid: 1, name: "G".into(), permission: 0, dimension: "d".into(), position: [0.0; 3], game_mode: 0, health: 1.0, yaw: 0.0, pitch: 0.0,
        };
        // perform_command is shared code: on this thread the client slot is asked, and there is none.
        assert!(matches!(mod_.perform_command(&mut host, &mut PerformCommandEvent { player: &player, command: "wasmping" }), Flow::Continue));
    }

    // -----------------------------------------------------------------------------------------
    // Review fixes

    #[test]
    fn a_thread_named_server_thread_is_not_the_server_without_the_scope() {
        let Some(bytes) = module("server") else { return };
        let server = load_server(&ModuleSource { id: "demo", bytes: &bytes, commands: false }).expect("server-full loads");
        let mod_ = Arc::new(FullMod::new("demo".into(), None, Some(server)));
        let runner = Arc::clone(&mod_);
        let flow = std::thread::Builder::new().name("Server thread".into()).spawn(move || {
            let mut host = FakeHost::default();
            let player = NativeServerPlayer {
                uuid: 1, name: "G".into(), permission: 0, dimension: "d".into(), position: [0.0; 3], game_mode: 0, health: 1.0, yaw: 0.0, pitch: 0.0,
            };
            matches!(runner.perform_command(&mut host, &mut PerformCommandEvent { player: &player, command: "wasmping" }), Flow::Continue)
        }).unwrap().join().unwrap();
        assert!(flow, "the thread's name alone must not route a hook to the server module");
    }

    #[test]
    fn a_busy_module_drops_the_hook_with_its_fallback_and_counts_it() {
        let Some(mod_) = client() else { return };
        let mut host = FakeHost::default();
        let mut batch = HudBatch::new([854, 480]);
        // Hold the client module's lock, as a re-entry from a guest's own service call would.
        let held = mod_.client.lock().unwrap();
        let mut hotbar = GuiLayerPreEvent { layer: "hotbar", batch: &mut batch };
        assert!(matches!(mod_.gui_layer_pre(&mut host, &mut hotbar), Flow::Continue), "the fallback is no answer");
        let mut hotbar = GuiLayerPreEvent { layer: "hotbar", batch: &mut batch };
        assert!(matches!(mod_.gui_layer_pre(&mut host, &mut hotbar), Flow::Continue));
        drop(held);
        assert_eq!(mod_.dropped_hooks(Side::Client), 2);
        // Not busy any more: the guest answers again.
        let mut hotbar = GuiLayerPreEvent { layer: "hotbar", batch: &mut batch };
        assert!(matches!(mod_.gui_layer_pre(&mut host, &mut hotbar), Flow::Cancel));
        assert_eq!(mod_.dropped_hooks(Side::Client), 2);
    }

    #[test]
    fn a_package_without_a_render_hook_stays_out_of_the_render_pass() {
        let Some(server_bytes) = module("server") else { return };
        let server = load_server(&ModuleSource { id: "demo", bytes: &server_bytes, commands: false }).unwrap();
        let server_only = FullMod::new("demo".into(), None, Some(server));
        assert!(server_only.render().is_none(), "a server-only package has nothing to draw");
        let Some(mod_) = client() else { return };
        // The demo's client module subscribes to hud and gui layers, not to a render-stage hook.
        let wants_render = RENDER_HOOKS.iter().any(|hook| mod_.wants(Side::Client, *hook));
        assert_eq!(mod_.render().is_some(), wants_render);
    }

    #[test]
    fn the_server_module_starts_afresh_for_each_world() {
        let Some(bytes) = module("server") else { return };
        let source = ServerSource::new("demo", bytes, false);
        let server = load_server(&source.module()).unwrap();
        let mod_ = Arc::new(FullMod::new("demo".into(), None, Some(server)).with_server_source(source));
        let runner = Arc::clone(&mod_);
        let told = std::thread::spawn(move || {
            let _server = crate::mod_guard::enter_thread(crate::mod_guard::ModThread::Server);
            let player = NativeServerPlayer {
                uuid: 7, name: "G".into(), permission: 0, dimension: "d".into(), position: [0.0; 3], game_mode: 0, health: 1.0, yaw: 0.0, pitch: 0.0,
            };
            let mut lines = Vec::new();
            for world in 0..2 {
                // A world's server starts: the guest's counter starts at zero in each.
                runner.reset_server();
                let mut host = FakeHost::default();
                runner.perform_command(&mut host, &mut PerformCommandEvent { player: &player, command: "wasmping" });
                runner.perform_command(&mut host, &mut PerformCommandEvent { player: &player, command: "wasmping" });
                lines.push((world, host.told.borrow().clone()));
            }
            lines
        }).join().unwrap();
        assert!(told[0].1[0].ends_with("pong #1 from the full-demo WASM server module"), "{told:?}");
        assert!(told[1].1[0].contains("pong #1 "), "the second world began with a fresh instance: {told:?}");
        assert!(told[1].1[1].contains("pong #2 "), "{told:?}");
    }

    #[test]
    fn guest_commands_are_gated_by_permission_and_a_denylist() {
        assert!(command_gate("say hi", false).unwrap_err().contains("service.commands"));
        assert!(command_gate("say hi", true).is_ok());
        assert!(command_gate("/give @a minecraft:stone", true).is_ok());
        for denied in [
            "transfer example.com 25565", "fetchprofile name Steve", "publish", "unpublish", "debug start", "jfr start", "perf start",
            "stop", "save-off", "op Steve", "whitelist add Steve", "ban Steve", "/stop", "STOP", "minecraft:stop",
            "execute as @a run stop", "execute as @a at @s run execute if entity @e run transfer a 1", "return run stop",
        ] {
            assert!(command_gate(denied, true).is_err(), "{denied}");
        }
        // Not a command at all, or no longer a single line.
        assert!(command_gate("say hi\nstop", true).is_err());
        assert!(command_gate("", true).is_err());
        assert!(command_gate(&"a".repeat(abi::MAX_TEXT_BYTES + 1), true).is_err());
        // The host refuses before it touches any service.
        let mut host = GameServerHost { id: "m".into(), commands: false };
        assert!(host.run_command("say hi".into()).unwrap_err().contains("service.commands"));
        let mut host = GameServerHost { id: "m".into(), commands: true };
        assert!(host.run_command("stop".into()).unwrap_err().contains("not available"));
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
        assert_eq!(game_relative(&game.join("config").join("mods")).as_deref(), Some("config/mods"));
        assert_eq!(game_relative(&game).as_deref(), Some("."));
        // Elsewhere: only the last component, never the account's folder.
        let elsewhere = std::path::Path::new("C:/Users/Someone/Documents/server/world");
        let shown = game_relative(elsewhere).unwrap();
        assert!(!shown.contains("Someone") && !shown.contains(':'), "{shown}");
    }

    #[test]
    fn guest_answers_are_checked_for_meaning() {
        // Item stacks: syntax, count, size.
        let stack = |id: &str, count: u32| et::ItemStack { id: id.into(), count, components: Vec::new() };
        assert!(item_from(&stack("minecraft:stone", 1)).is_some());
        assert!(item_from(&stack("minecraft:stone", 0)).is_none());
        assert!(item_from(&stack("minecraft:stone", MAX_STACK_COUNT + 1)).is_none());
        assert!(item_from(&stack("", 1)).is_none());
        assert!(item_from(&stack("stone", 1)).is_none());
        assert!(item_from(&stack("Minecraft:Stone", 1)).is_none());
        assert!(item_from(&stack(&format!("minecraft:{}", "a".repeat(MAX_ITEM_ID)), 1)).is_none());
        let big = et::ItemStack { id: "minecraft:stone".into(), count: 1, components: vec![0; MAX_ITEM_COMPONENTS + 1] };
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
        assert!(std::ptr::eq(leak_static("wasmping".into()).unwrap(), leak_static("wasmping".into()).unwrap()), "deduplicated");
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
