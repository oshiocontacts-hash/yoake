//! Runtime of the full-catalog worlds: [`ClientFullMod`] (`client-full`) and [`ServerFullMod`]
//! (`server-full`). Like `ClientMod` they are one sandboxed instance each (same memory, fuel and
//! time limits, WASI subset); a trap, a limit or a bad answer disables only that module. The typed
//! hook calls are made by the host's generated adapters through [`ClientFullMod::dispatch`] and
//! [`ServerFullMod::dispatch`], which only run a hook the module subscribed to.
use wasmtime::component::HasSelf;
use wasmtime::Store;

use crate::bindings::yoake::mods::types::HostError;
use crate::full_bindings::{client as client_world, server as server_world};
use crate::host::{ClientContext, ClientHost};
use crate::subscriptions::FullWorld;
use crate::{HostState, LoadError, ModInstance, OutputLine, Sandbox, Subscriptions};

pub use crate::full_bindings::server::yoake::mods::server_types;

fn flush_output<T: 'static>(raw: &mut ModInstance<T>, mut log: impl FnMut(&mut T, &OutputLine)) {
    for line in raw.drain_output() {
        log(&mut raw.store.data_mut().host, &line);
    }
}

fn stream_name(line: &OutputLine) -> &'static str {
    match line.stream {
        crate::OutputStream::Stdout => "stdout",
        crate::OutputStream::Stderr => "stderr",
    }
}

/// A `client-full` module.
pub struct ClientFullMod {
    raw: ModInstance<ClientContext>,
    exports: client_world::ClientFull,
    subscriptions: Subscriptions,
}

impl ClientFullMod {
    /// Instantiates the component and reads its subscriptions. `unwired` lists the catalog hooks the
    /// host has no adapter for: subscribing to one is an error.
    pub fn instantiate(
        runtime: &mut Sandbox,
        id: impl Into<String>,
        component_bytes: &[u8],
        host: Box<dyn ClientHost>,
        unwired: &[&str],
    ) -> Result<Self, LoadError> {
        let id = id.into();
        let texture_count = runtime.texture_count(&id);
        let mut raw = runtime
            .compile(component_bytes)
            .map_err(LoadError::new)?
            .instantiate(id.clone(), ClientContext::new(id, host, texture_count), |linker| {
                client_world::ClientFull::add_to_linker::<HostState<ClientContext>, HasSelf<ClientContext>>(
                    linker,
                    |state| &mut state.host,
                )
                .map_err(|error| error.to_string())
            })
            .map_err(LoadError::new)?;
        let exports = client_world::ClientFull::new(&mut raw.store, &raw.instance)
            .map_err(|error| LoadError::new(error.to_string()))?;
        let names = raw.call("init", |store, _| exports.call_init(store).map_err(|error| error.to_string()));
        flush_output(&mut raw, |host, line| host.log_output(line));
        let names = names.ok_or_else(|| LoadError::new("mod init trapped or timed out"))?;
        let subscriptions = Subscriptions::for_full_world(FullWorld::Client, names, unwired).map_err(LoadError::new)?;
        Ok(Self { raw, exports, subscriptions })
    }

    pub fn subscribed(&self, hook: &str) -> bool {
        self.subscriptions.subscribed(hook)
    }

    pub fn subscriptions(&self) -> &Subscriptions {
        &self.subscriptions
    }

    pub fn is_disabled(&self) -> bool {
        self.raw.is_disabled()
    }

    /// Runs `call` against the guest's exports if it subscribed to `hook`; `None` when it did not
    /// subscribe, trapped, or ran out of fuel or time (the module is then disabled).
    pub fn dispatch<R>(
        &mut self,
        hook: &str,
        call: impl FnOnce(&client_world::ClientFull, &mut Store<HostState<ClientContext>>) -> wasmtime::Result<R>,
    ) -> Option<R> {
        if !self.subscribed(hook) {
            return None;
        }
        let exports = &self.exports;
        let result = self.raw.call(hook, |store, _| call(exports, store).map_err(|error| error.to_string()));
        flush_output(&mut self.raw, |host, line| host.log_output(line));
        result
    }
}

const UNSUPPORTED: &str = "unsupported";

macro_rules! server_services {
    ($($name:ident($($arg:ident: $ty:ty),*) -> $ret:ty = $default:expr;)*) => {
        /// The host side of the `server-services` interface. Every method has a default that
        /// answers "unsupported" (an `Err` for services that can fail, none or false otherwise), so
        /// a host implements what it can forward and leaves the rest.
        pub trait ServerHost: Send {
            fn log(&mut self, target: &str, message: &str) -> Result<(), HostError> {
                let _ = (target, message);
                Err(HostError::Unsupported)
            }
            $(
                fn $name(&mut self, $($arg: $ty),*) -> $ret {
                    $(let _ = &$arg;)*
                    $default
                }
            )*
        }

        impl server_world::yoake::mods::server_services::Host for ServerContext {
            $(
                fn $name(&mut self, $($arg: $ty),*) -> $ret {
                    self.host.$name($($arg),*)
                }
            )*
        }

        /// The names of every server service of the interface.
        pub const SERVER_SERVICES: &[&str] = &[$(stringify!($name)),*];
    };
}

fn unsupported<T>() -> Result<T, String> {
    Err(UNSUPPORTED.to_owned())
}

use server_types as st;

server_services! {
    acting_player() -> Option<st::Player> = None;
    held() -> Option<st::ItemStack> = None;
    server_players() -> Result<Vec<st::Player>, String> = unsupported();
    integrated_owner() -> Option<String> = None;
    entities_in(center: st::Vec3, dimension: String, radius: f64) -> Result<Vec<st::EntitySnapshot>, String> = unsupported();
    clip(dimension: String, end: st::Vec3, start: st::Vec3) -> Result<Option<st::RayHit>, String> = unsupported();
    player_stacks(player: String) -> Result<Vec<Option<st::ItemStack>>, String> = unsupported();
    player_teleport(dimension: String, pitch: f32, player: String, position: st::Vec3, yaw: f32) -> Result<(), String> = unsupported();
    player_set_health(health: f32, player: String) -> Result<(), String> = unsupported();
    player_set_gamemode(game_mode: i32, player: String) -> Result<(), String> = unsupported();
    player_clear_inventory(player: String) -> Result<(), String> = unsupported();
    player_effect_give(ambient: bool, amplifier: u8, duration_ticks: u32, effect: String, icon: bool, particles: bool, player: String) -> Result<(), String> = unsupported();
    player_effect_clear(effect: Option<String>, player: String) -> Result<(), String> = unsupported();
    player_attribute_modifier_remove(attribute: String, modifier: String, player: String) -> Result<(), String> = unsupported();
    tell(message: String, player: String) -> Result<(), String> = unsupported();
    action_bar(message: String, player: String) -> Result<(), String> = unsupported();
    player_title(fade_in: i32, fade_out: i32, player: String, stay: i32, subtitle: Option<String>, title: Option<String>) -> Result<(), String> = unsupported();
    player_clear_title(player: String) -> Result<(), String> = unsupported();
    player_play_sound(category: String, pitch: f32, player: String, position: st::Vec3, sound: String, volume: f32) -> Result<(), String> = unsupported();
    bossbar_create(bar: String, name: String) -> Result<(), String> = unsupported();
    bossbar_remove(bar: String) -> Result<(), String> = unsupported();
    bossbar_set_color(bar: String, color: String) -> Result<(), String> = unsupported();
    bossbar_set_name(bar: String, name: String) -> Result<(), String> = unsupported();
    bossbar_set_overlay(bar: String, overlay: String) -> Result<(), String> = unsupported();
    bossbar_set_players(bar: String, players: Vec<String>) -> Result<(), String> = unsupported();
    bossbar_set_progress(bar: String, progress: f32) -> Result<(), String> = unsupported();
    run_command(command: String) -> Result<bool, String> = unsupported();
    block(position: st::BlockPos) -> Result<String, String> = unsupported();
    block_property(position: st::BlockPos, property: String) -> Result<Option<String>, String> = unsupported();
    set_block_property(flags: i32, position: st::BlockPos, property: String, value: String) -> Result<bool, String> = unsupported();
    block_collision(position: st::BlockPos) -> Result<Vec<st::Bounds>, String> = unsupported();
    edit_blocks(dimension: String, edit: st::BlockEdit) -> Result<u64, String> = unsupported();
    block_entity_sync(position: st::BlockPos) -> Result<(), String> = unsupported();
    block_entity_read(block_entity: u32) -> Result<st::BlockEntityState, String> = unsupported();
    block_entity_patch_call(block_entity: u32, patch: st::BlockEntityPatch) -> Result<(), String> = unsupported();
    spawn_mannequin(player: String, pose: String, position: st::Vec3, yaw: f32) -> Result<Option<u32>, String> = unsupported();
    remove_spawned_entity(entity: u32) -> Result<bool, String> = unsupported();
    net_send_to_player(channel: String, payload: Vec<u8>, player: String) -> Result<(), String> = unsupported();
    world_dir() -> Option<String> = None;
    config_dir() -> Result<String, String> = unsupported();
    world_storage_read(key: String) -> Result<Option<Vec<u8>>, String> = unsupported();
    world_storage_write(data: Vec<u8>, key: String) -> Result<(), String> = unsupported();
    config_read(key: String) -> Result<Option<Vec<u8>>, String> = unsupported();
    config_write(data: Vec<u8>, key: String) -> Result<(), String> = unsupported();
    asset_read(key: String) -> Result<Option<Vec<u8>>, String> = unsupported();
    inventory_apply(change: st::InventoryChange, player: String) -> Result<bool, String> = unsupported();
    team_apply(change: st::TeamChange) -> Result<bool, String> = unsupported();
    world_control(change: st::WorldControlChange) -> Result<bool, String> = unsupported();
    particle_spawn(request: st::ParticleRequest) -> Result<(), String> = unsupported();
    range_override_set(blocks: u32, owner: String) -> Result<(), String> = unsupported();
    range_override_clear(owner: String) -> Result<(), String> = unsupported();
    qa_mode_enabled() -> bool = false;
    qa_record(event: String, json: String) -> Result<(), String> = unsupported();
}

pub struct ServerContext {
    id: String,
    host: Box<dyn ServerHost>,
}

impl ServerContext {
    fn new(id: String, host: Box<dyn ServerHost>) -> Self {
        Self { id, host }
    }

    fn log_output(&mut self, line: &OutputLine) {
        let _ = self.host.log(stream_name(line), &line.text);
        let _ = &self.id;
    }
}

impl crate::bindings::yoake::mods::types::Host for ServerContext {}
impl crate::bindings::yoake::mods::logging::Host for ServerContext {
    fn log(&mut self, target: String, message: String) -> Result<(), HostError> {
        self.host.log(&target, &message)
    }
}
impl client_world::yoake::mods::engine_types::Host for ServerContext {}
impl client_world::yoake::mods::engine_types::Host for ClientContext {}
impl server_world::yoake::mods::server_types::Host for ServerContext {}

/// A `server-full` module.
pub struct ServerFullMod {
    raw: ModInstance<ServerContext>,
    exports: server_world::ServerFull,
    subscriptions: Subscriptions,
}

impl ServerFullMod {
    pub fn instantiate(
        runtime: &mut Sandbox,
        id: impl Into<String>,
        component_bytes: &[u8],
        host: Box<dyn ServerHost>,
        unwired: &[&str],
    ) -> Result<Self, LoadError> {
        let id = id.into();
        let mut raw = runtime
            .compile(component_bytes)
            .map_err(LoadError::new)?
            .instantiate(id.clone(), ServerContext::new(id, host), |linker| {
                server_world::ServerFull::add_to_linker::<HostState<ServerContext>, HasSelf<ServerContext>>(
                    linker,
                    |state| &mut state.host,
                )
                .map_err(|error| error.to_string())
            })
            .map_err(LoadError::new)?;
        let exports = server_world::ServerFull::new(&mut raw.store, &raw.instance)
            .map_err(|error| LoadError::new(error.to_string()))?;
        let names = raw.call("init", |store, _| exports.call_init(store).map_err(|error| error.to_string()));
        flush_output(&mut raw, |host, line| host.log_output(line));
        let names = names.ok_or_else(|| LoadError::new("mod init trapped or timed out"))?;
        let subscriptions = Subscriptions::for_full_world(FullWorld::Server, names, unwired).map_err(LoadError::new)?;
        Ok(Self { raw, exports, subscriptions })
    }

    pub fn subscribed(&self, hook: &str) -> bool {
        self.subscriptions.subscribed(hook)
    }

    pub fn subscriptions(&self) -> &Subscriptions {
        &self.subscriptions
    }

    pub fn is_disabled(&self) -> bool {
        self.raw.is_disabled()
    }

    /// Like [`ClientFullMod::dispatch`], for a server-side hook.
    pub fn dispatch<R>(
        &mut self,
        hook: &str,
        call: impl FnOnce(&server_world::ServerFull, &mut Store<HostState<ServerContext>>) -> wasmtime::Result<R>,
    ) -> Option<R> {
        if !self.subscribed(hook) {
            return None;
        }
        let exports = &self.exports;
        let result = self.raw.call(hook, |store, _| call(exports, store).map_err(|error| error.to_string()));
        flush_output(&mut self.raw, |host, line| host.log_output(line));
        result
    }
}
