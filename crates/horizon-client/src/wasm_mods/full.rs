//! Full-catalog WebAssembly mods (`client-full` and `server-full`, `crates/yoake-wasm/wit/yoake-full.wit`).
//!
//! One [`FullMod`] per `.yak` holds the module instances of that package (a client one and a server one,
//! each optional) and implements every hook of the catalog the generator could wire. The hook methods
//! are generated into `OUT_DIR/wasm_full_adapters.rs` and included below: each converts the native
//! event to the WIT values, calls the guest only if it subscribed (one atomic load otherwise), and
//! applies the answer per the hook's contract. Nothing here exists, and no hook of the game is wrapped,
//! unless a `.yak` with such a module is installed.
//!
//! A hook of the game's shared code runs on the client thread or the server thread; the thread decides
//! which module gets it. The two modules have their own locks, so a client hook never waits for the
//! server thread.

use super::*;
use crate::gfx::native_render::*;
use horizon_client_core::text::Text;
use horizon_items::items::ItemStack;
use horizon_world::nbt::{Compound, Tag};
use yoake_wasm::full_bindings::client::yoake::mods::engine_types as et;
use yoake_wasm::{ClientFullMod, ServerFullMod, ServerHost, convert, server_types as st};

/// Commands a guest may return for one object in one call.
const MAX_COMMANDS: usize = 8192;

#[derive(Clone, Copy, PartialEq, Eq)]
pub(super) enum Side {
    Client = 0,
    Server = 1,
}

thread_local! {
    static SERVER_THREAD: std::cell::Cell<Option<bool>> = const { std::cell::Cell::new(None) };
}

/// Whether the current thread is a (dedicated or integrated) server thread.
fn on_server_thread() -> bool {
    SERVER_THREAD.with(|cached| match cached.get() {
        Some(answer) => answer,
        None => {
            let answer = std::thread::current().name() == Some("Server thread");
            cached.set(Some(answer));
            answer
        }
    })
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
        Self {
            id,
            client: Mutex::new(SideState { module: client, reported: false }),
            server: Mutex::new(SideState { module: server, reported: false }),
            masks: [client_mask, server_mask],
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
            Err(TryLockError::WouldBlock) => return None,
            Err(TryLockError::Poisoned(_)) => {
                self.disable(Side::Client);
                log::error!("wasm mod {} disabled: poisoned client instance", self.id);
                return None;
            }
        };
        let module = state.module.as_mut()?;
        let _ = hook;
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
            Err(TryLockError::WouldBlock) => return None,
            Err(TryLockError::Poisoned(_)) => {
                self.disable(Side::Server);
                log::error!("wasm mod {} disabled: poisoned server instance", self.id);
                return None;
            }
        };
        let module = state.module.as_mut()?;
        let _ = hook;
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

fn nbt_tag(tag: &Tag) -> Vec<u8> {
    convert::tag_to_nbt(tag).unwrap_or_default()
}

fn nbt_compound(compound: &Compound) -> Vec<u8> {
    convert::compound_to_nbt(compound).unwrap_or_default()
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

fn item_from(stack: &et::ItemStack) -> Option<ItemStack> {
    convert::item_from_wire(&convert::WireItemStack {
        id: stack.id.clone(),
        count: stack.count,
        components: stack.components.clone(),
    })
    .ok()
}

/// A string that lives for the rest of the process, deduplicated (for answers the engine keeps as
/// `&'static str`, such as command names).
fn leak_static(text: String) -> &'static str {
    static POOL: Mutex<Option<HashSet<&'static str>>> = Mutex::new(None);
    let mut pool = POOL.lock().unwrap_or_else(|error| error.into_inner());
    let pool = pool.get_or_insert_with(HashSet::new);
    if let Some(existing) = pool.get(text.as_str()) {
        return existing;
    }
    let leaked: &'static str = Box::leak(text.into_boxed_str());
    pool.insert(leaked);
    leaked
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
        if command.len() > abi::MAX_TEXT_BYTES {
            return Err("command too long".into());
        }
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
        read_service(|host| Ok(host.world_dir())).ok().flatten().and_then(|path| path.to_str().map(str::to_owned))
    }

    fn config_dir(&mut self) -> Result<String, String> {
        read_service(|host| host.config_dir())
            .and_then(|path| path.to_str().map(str::to_owned).ok_or_else(|| "path is not text".to_owned()))
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
}

pub(super) fn load_client(source: &ModuleSource<'_>) -> Result<ClientFullMod, String> {
    let mut sandbox = sandbox().lock().unwrap_or_else(|error| error.into_inner());
    ClientFullMod::instantiate(&mut sandbox, source.id, source.bytes, Box::new(GameHost::new(source.id)), UNWIRED)
        .map_err(|error| error.to_string())
}

pub(super) fn load_server(source: &ModuleSource<'_>) -> Result<ServerFullMod, String> {
    let mut sandbox = sandbox().lock().unwrap_or_else(|error| error.into_inner());
    ServerFullMod::instantiate(&mut sandbox, source.id, source.bytes, Box::new(GameServerHost { id: source.id.to_owned() }), UNWIRED)
        .map_err(|error| error.to_string())
}

/// How many hooks have an adapter: all but [`UNWIRED`].
pub(super) fn wired_hooks() -> usize {
    HOOK_COUNT - UNWIRED.len()
}

include!(concat!(env!("OUT_DIR"), "/wasm_full_adapters.rs"));

#[cfg(test)]
mod tests {
    use super::*;
    use std::cell::RefCell;

    const DIR: &str = "C:/Users/USER/NestgateHorizon/target-wasm/wasm32-wasip2/release/";

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
        let bytes = std::fs::read(format!("{DIR}full_demo_{kind}.wasm")).ok();
        if bytes.is_none() { eprintln!("skipped: full_demo_{kind}.wasm is not built"); }
        bytes
    }

    fn client() -> Option<FullMod> {
        let bytes = module("client")?;
        let client = load_client(&ModuleSource { id: "demo", bytes: &bytes }).expect("client-full loads");
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
        let server = load_server(&ModuleSource { id: "demo", bytes: &bytes }).expect("server-full loads");
        let mod_ = Arc::new(FullMod::new("demo".into(), None, Some(server)));
        let runner = Arc::clone(&mod_);
        let handle = std::thread::Builder::new().name("Server thread".into()).spawn(move || {
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
        // On a non-server thread a hook of the shared code does not reach the server module.
        let mut host = FakeHost::default();
        let player = NativeServerPlayer {
            uuid: 1, name: "G".into(), permission: 0, dimension: "d".into(), position: [0.0; 3], game_mode: 0, health: 1.0, yaw: 0.0, pitch: 0.0,
        };
        // perform_command is shared code: on this thread the client slot is asked, and there is none.
        assert!(matches!(mod_.perform_command(&mut host, &mut PerformCommandEvent { player: &player, command: "wasmping" }), Flow::Continue));
    }
}
