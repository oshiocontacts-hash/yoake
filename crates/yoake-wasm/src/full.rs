//! Runtime of the full-catalog worlds: [`ClientFullMod`] (`client-full`) and [`ServerFullMod`]
//! (`server-full`). Like `ClientMod` they are one sandboxed instance each (same memory, fuel and
//! time limits, WASI subset); a trap, a limit or a bad answer disables only that module. The typed
//! hook calls are made by the host's generated adapters through [`ClientFullMod::dispatch`] and
//! [`ServerFullMod::dispatch`], which only run a hook the module subscribed to.
use wasmtime::Store;
use wasmtime::component::HasSelf;

use crate::bindings::yoake::mods::types::HostError;
use crate::full_bindings::{client as client_world, server as server_world};
use crate::host::{ClientContext, ClientHost};
use crate::subscriptions::FullWorld;
use crate::{HostState, LoadError, ModInstance, OutputLine, Sandbox, Subscriptions};

pub use crate::full_bindings::server::yoake::mods::server_types;

fn service_call<T>(
    handle: Result<crate::package_services::ServiceHandle, String>,
    call: impl FnOnce(&mut crate::package_services::PackageServices) -> Result<T, String>,
) -> Result<T, String> {
    let began = std::time::Instant::now();
    let answer = handle.and_then(|handle| {
        let mut services = handle.lock().map_err(|_| "package service lock poisoned")?;
        call(&mut services)
    });
    crate::add_storage_time(began.elapsed());
    answer
}

struct TicketGuard(Option<(crate::package_services::ServiceHandle, u64)>);

impl TicketGuard {
    fn new(handle: Result<crate::package_services::ServiceHandle, String>) -> Self {
        Self(handle.ok().and_then(|handle| {
            let owner = handle.lock().ok()?.claim_tickets();
            Some((handle, owner))
        }))
    }

    fn abort(&self) {
        if let Some((handle, owner)) = &self.0 {
            if let Ok(mut services) = handle.lock() {
                services.release_tickets(*owner);
            }
        }
    }
}

impl Drop for TicketGuard {
    fn drop(&mut self) {
        self.abort();
    }
}

macro_rules! package_imports {
    () => {
        fn config_get(&mut self, key: String) -> Result<server_types::ConfigValue, String> {
            service_call(self.host.services(), |services| services.config_get(&key))
        }
        fn config_set(
            &mut self,
            key: String,
            value: server_types::ConfigValue,
        ) -> Result<(), String> {
            service_call(self.host.services(), |services| {
                services.config_set(&key, value)
            })
        }
        fn storage_get(
            &mut self,
            key: String,
            scope: server_types::StorageScope,
        ) -> Result<Option<Vec<u8>>, String> {
            service_call(self.host.services(), |services| {
                services.storage_get(&key, scope)
            })
        }
        fn storage_put(
            &mut self,
            key: String,
            scope: server_types::StorageScope,
            value: Vec<u8>,
        ) -> Result<(), String> {
            service_call(self.host.services(), |services| {
                services.storage_put(&key, scope, &value)
            })
        }
        fn storage_delete(
            &mut self,
            key: String,
            scope: server_types::StorageScope,
        ) -> Result<bool, String> {
            service_call(self.host.services(), |services| {
                services.storage_delete(&key, scope)
            })
        }
        fn storage_blob_begin(
            &mut self,
            key: String,
            scope: server_types::StorageScope,
            total: u64,
        ) -> Result<u64, String> {
            service_call(self.host.services(), |services| {
                services.storage_blob_begin(&key, scope, total)
            })
        }
        fn storage_blob_write(
            &mut self,
            data: Vec<u8>,
            id: u64,
            offset: u64,
        ) -> Result<(), String> {
            service_call(self.host.services(), |services| {
                services.storage_blob_write(&data, id, offset)
            })
        }
        fn storage_blob_commit(&mut self, id: u64) -> Result<(), String> {
            service_call(self.host.services(), |services| {
                services.storage_blob_commit(id)
            })
        }
        fn storage_blob_abort(&mut self, id: u64) -> Result<bool, String> {
            service_call(self.host.services(), |services| {
                services.storage_blob_abort(id)
            })
        }
        fn storage_blob_read(
            &mut self,
            key: String,
            length: u32,
            offset: u64,
            scope: server_types::StorageScope,
        ) -> Result<Vec<u8>, String> {
            service_call(self.host.services(), |services| {
                services.storage_blob_read(&key, length, offset, scope)
            })
        }
    };
}

impl client_world::yoake::mods::server_types::Host for ClientContext {}
impl client_world::yoake::mods::client_services::Host for ClientContext {
    package_imports!();
}

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
    tickets: TicketGuard,
}

impl ClientFullMod {
    /// Instantiates the component and reads its subscriptions. `unwired` lists the catalog hooks the
    /// host has no adapter for: subscribing to one is an error.
    pub fn instantiate(
        runtime: &mut Sandbox,
        id: impl Into<String>,
        component_bytes: &[u8],
        mut host: Box<dyn ClientHost>,
        unwired: &[&str],
    ) -> Result<Self, LoadError> {
        let id = id.into();
        let tickets = TicketGuard::new(host.services());
        let texture_count = runtime.texture_count(&id);
        let mut raw = runtime
            .compile(component_bytes)
            .map_err(LoadError::new)?
            .instantiate(
                id.clone(),
                ClientContext::new(id, host, texture_count),
                |linker| {
                    client_world::ClientFull::add_to_linker::<
                        HostState<ClientContext>,
                        HasSelf<ClientContext>,
                    >(linker, |state| &mut state.host)
                    .map_err(|error| error.to_string())
                },
            )
            .map_err(LoadError::new)?;
        let exports = client_world::ClientFull::new(&mut raw.store, &raw.instance)
            .map_err(|error| LoadError::new(error.to_string()))?;
        let names = raw.call_init(crate::TimeLimits::CLIENT, |store, _| {
            exports
                .call_init(store)
                .map_err(|error| format!("{error:#}"))
        });
        flush_output(&mut raw, |host, line| host.log_output(line));
        let names = names.ok_or_else(|| LoadError::new("mod init trapped or timed out"))?;
        let subscriptions = Subscriptions::for_full_world(FullWorld::Client, names, unwired)
            .map_err(LoadError::new)?;
        let mut module = Self {
            raw,
            exports,
            subscriptions,
            tickets,
        };
        module.notify_config_changes();
        Ok(module)
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

    pub fn notify_config_changes(&mut self) {
        if self.raw.is_disabled() {
            self.tickets.abort();
            return;
        }
        let Ok(services) = self.raw.store.data_mut().host.host.services() else {
            return;
        };
        let changes = match services.lock() {
            Ok(mut services) => services.take_changes(),
            Err(_) => return,
        };
        if !self.subscribed("config_changed") {
            return;
        }
        for (key, value) in changes {
            let exports = &self.exports;
            self.raw.call("config_changed", |store, _| {
                exports
                    .yoake_mods_client_hooks()
                    .call_config_changed(store, &key, &value)
                    .map_err(|error| format!("{error:#}"))
            });
            flush_output(&mut self.raw, |host, line| host.log_output(line));
            if self.raw.is_disabled() {
                self.tickets.abort();
                return;
            }
        }
    }

    /// Runs `call` against the guest's exports if it subscribed to `hook`; `None` when it did not
    /// subscribe, trapped, or ran out of fuel or time (the module is then disabled).
    pub fn dispatch<R>(
        &mut self,
        hook: &str,
        call: impl FnOnce(&client_world::ClientFull, &mut Store<HostState<ClientContext>>) -> wasmtime::Result<R>,
    ) -> Option<R> {
        self.notify_config_changes();
        if !self.subscribed(hook) {
            return None;
        }
        let exports = &self.exports;
        let result = self.raw.call(hook, |store, _| call(exports, store).map_err(|error| format!("{error:#}")));
        flush_output(&mut self.raw, |host, line| host.log_output(line));
        self.notify_config_changes();
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
            fn services(&mut self) -> Result<crate::package_services::ServiceHandle, String> {
                Err("package services unavailable".into())
            }

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
            package_imports!();

            fn has_permission(&mut self, node: String, player: String) -> Result<bool, String> {
                let player = horizon_mods::yak::normalize_uuid(&player)
                    .ok_or("invalid player UUID")?;
                let handle = self.host.services()?;
                service_call(Ok(handle.clone()), |services| services.require(crate::package_services::PERMISSIONS_CAPABILITY))?;
                let level = self.host.server_players()?.into_iter().find(|entry| {
                    horizon_mods::yak::normalize_uuid(&entry.uuid).as_ref() == Some(&player)
                })
                    .ok_or("player is not connected")?.permission_level;
                service_call(Ok(handle), |services| services.has_permission(&node, &player, level))
            }

            $(
                fn $name(&mut self, $($arg: $ty),*) -> $ret {
                    let began = std::time::Instant::now();
                    let answer = self.host.$name($($arg),*);
                    if !matches!(stringify!($name), "world_storage_read" | "world_storage_write" | "config_read" | "config_write" | "config_dir") {
                        crate::add_host_time(began.elapsed());
                    } else {
                        crate::add_storage_time(began.elapsed());
                    }
                    answer
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
    player_stacks(player: String) -> Result<Vec<st::PlayerInventorySlot>, String> = unsupported();
    player_remove_slot(expected: st::ItemStack, player: String, slot: u32) -> Result<bool, String> = unsupported();
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
    break_block_as(dimension: String, player: String, position: st::BlockPos) -> Result<bool, String> = unsupported();
    player_sneaking(player: String) -> Result<bool, String> = unsupported();
    container_slots(dimension: String, position: st::BlockPos) -> Result<Vec<Option<st::ItemStack>>, String> = unsupported();
    container_set_slots(dimension: String, position: st::BlockPos, slots: Vec<Option<st::ItemStack>>) -> Result<(), String> = unsupported();
    containers_in(center: st::Vec3, dimension: String, radius: f64) -> Result<Vec<st::ContainerRef>, String> = unsupported();
    player_set_slots(player: String, slots: Vec<Option<st::ItemStack>>) -> Result<(), String> = unsupported();
    block_tags(block: String) -> Result<Vec<String>, String> = unsupported();
    item_tags(item: String) -> Result<Vec<String>, String> = unsupported();
    block_tool_requirement(block: String) -> Result<st::ToolRequirement, String> = unsupported();
    can_harvest(block: String, tool: Option<st::ItemStack>) -> Result<bool, String> = unsupported();
    player_attribute_modifier_add(amount: f64, attribute: String, modifier: String, operation: st::AttributeOperation, player: String) -> Result<(), String> = unsupported();
    attribute_get(attribute: String, player: String) -> Result<st::AttributeValue, String> = unsupported();
    open_menu(extra: Vec<u8>, menu_type: String, player: String, title: String) -> Result<bool, String> = unsupported();
    menu_set_items(items: Vec<Option<st::ItemStack>>, player: String) -> Result<(), String> = unsupported();
    menu_close(player: String) -> Result<(), String> = unsupported();
    item_entities_in(center: st::Vec3, dimension: String, radius: f64) -> Result<Vec<st::ItemEntity>, String> = unsupported();
    item_entity_move(dimension: String, entity: u32, position: st::Vec3, velocity: st::Vec3) -> Result<bool, String> = unsupported();
    item_entity_pickup(dimension: String, entity: u32, player: String) -> Result<bool, String> = unsupported();
    give_items(items: Vec<st::ItemStack>, player: String) -> Result<Vec<st::ItemStack>, String> = unsupported();
    advancement_progress(advancement: String, player: String) -> Result<st::AdvancementState, String> = unsupported();
    advancement_grant(advancement: String, player: String) -> Result<bool, String> = unsupported();
    entity_vitals_of(dimension: String, entity: u32) -> Result<st::EntityVitals, String> = unsupported();
    hurt_entity(amount: f32, attacker: Option<String>, damage: st::DamageKind, dimension: String, entity: u32) -> Result<bool, String> = unsupported();
    server_player_motion(player: String) -> Result<st::PlayerMotion, String> = unsupported();
    set_server_player_velocity(dimension: String, player: String, velocity: st::Vec3) -> Result<bool, String> = unsupported();
    play_sound_at(category: String, dimension: String, pitch: f32, position: st::Vec3, sound: String, volume: f32) -> Result<u32, String> = unsupported();
    set_player_attack_hitbox(height: f32, player: String, width: f32) -> Result<bool, String> = unsupported();
    clear_player_attack_hitbox(player: String) -> Result<bool, String> = unsupported();
    world_border(dimension: String) -> Result<st::WorldBorderState, String> = unsupported();
    set_world_border(center_x: f64, center_z: f64, dimension: String, lerp_ticks: Option<u64>, size: f64, target_size: Option<f64>) -> Result<(), String> = unsupported();
    mod_call(method: String, payload: Vec<u8>, target_mod: String) -> Result<Vec<u8>, String> = unsupported();
    attachment_define(copy_on_death: bool, id: String, kind: String, persistent: bool, synced: bool) -> Result<(), String> = unsupported();
    attachment_get(holder: String, id: String, kind: String) -> Result<Option<Vec<u8>>, String> = unsupported();
    attachment_set(data: Vec<u8>, holder: String, id: String, kind: String) -> Result<(), String> = unsupported();
    attachment_remove(holder: String, id: String, kind: String) -> Result<bool, String> = unsupported();
}

pub struct ServerContext {
    id: String,
    pub(crate) host: Box<dyn ServerHost>,
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
    tickets: TicketGuard,
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::package_services::{PackageServices, STORAGE_CAPABILITY, ServiceHandle};
    use std::collections::BTreeMap;
    use std::sync::{Arc, Mutex};
    use std::time::Duration;

    struct StorageHost(ServiceHandle);

    impl ServerHost for StorageHost {
        fn services(&mut self) -> Result<ServiceHandle, String> {
            Ok(self.0.clone())
        }

        fn world_storage_write(&mut self, data: Vec<u8>, key: String) -> Result<(), String> {
            self.0
                .lock()
                .unwrap()
                .storage_put(&key, server_types::StorageScope::World, &data)
        }

        fn server_players(&mut self) -> Result<Vec<server_types::Player>, String> {
            std::thread::sleep(Duration::from_millis(2));
            Ok(Vec::new())
        }
    }

    #[test]
    fn storage_io_remains_charged_to_the_guest_time_budget() {
        use server_world::yoake::mods::server_services::Host;

        let time = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        let root = std::fs::canonicalize(std::env::temp_dir())
            .unwrap()
            .join(format!("yoake-storage-time-{}-{time}", std::process::id()));
        std::fs::create_dir(&root).unwrap();
        let services = Arc::new(Mutex::new(
            PackageServices::open(
                "example",
                &[STORAGE_CAPABILITY.into()],
                &BTreeMap::new(),
                &BTreeMap::new(),
                horizon_mods::yak::ConfigRuntimeSide::Server,
                &root,
                Some(&root),
            )
            .unwrap(),
        ));
        let mut context =
            ServerContext::new("example".into(), Box::new(StorageHost(services.clone())));
        let previous = crate::HOST_TIME.with(|time| time.replace(Duration::ZERO));
        let previous_storage = crate::STORAGE_TIME.with(|time| time.replace(Duration::ZERO));
        context
            .storage_put("typed".into(), server_types::StorageScope::World, vec![1])
            .unwrap();
        assert_eq!(crate::HOST_TIME.with(|time| time.get()), Duration::ZERO);
        let typed_storage_time = crate::STORAGE_TIME.with(|time| time.get());
        assert!(typed_storage_time > Duration::ZERO);
        context
            .world_storage_write(vec![2], "legacy".into())
            .unwrap();
        assert_eq!(crate::HOST_TIME.with(|time| time.get()), Duration::ZERO);
        assert!(crate::STORAGE_TIME.with(|time| time.get()) > typed_storage_time);
        context.server_players().unwrap();
        assert!(crate::HOST_TIME.with(|time| time.get()) >= Duration::from_millis(2));
        crate::HOST_TIME.with(|time| time.set(previous));
        crate::STORAGE_TIME.with(|time| time.set(previous_storage));
        drop(context);
        drop(services);
        std::fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn hot_reload_does_not_abort_the_replacement_instances_blob_tickets() {
        let time = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        let root = std::fs::canonicalize(std::env::temp_dir())
            .unwrap()
            .join(format!("yoake-ticket-owner-{}-{time}", std::process::id()));
        std::fs::create_dir(&root).unwrap();
        let services = Arc::new(Mutex::new(
            PackageServices::open(
                "example",
                &[STORAGE_CAPABILITY.into()],
                &BTreeMap::new(),
                &BTreeMap::new(),
                horizon_mods::yak::ConfigRuntimeSide::Server,
                &root,
                Some(&root),
            )
            .unwrap(),
        ));
        let old = TicketGuard::new(Ok(services.clone()));
        let old_ticket = services
            .lock()
            .unwrap()
            .storage_blob_begin("old", server_types::StorageScope::World, 4)
            .unwrap();
        let replacement = TicketGuard::new(Ok(services.clone()));
        assert!(
            services
                .lock()
                .unwrap()
                .storage_blob_write(b"data", old_ticket, 0)
                .is_err()
        );
        let new_ticket = services
            .lock()
            .unwrap()
            .storage_blob_begin("new", server_types::StorageScope::World, 4)
            .unwrap();
        old.abort();
        drop(old);
        services
            .lock()
            .unwrap()
            .storage_blob_write(b"data", new_ticket, 0)
            .unwrap();
        services
            .lock()
            .unwrap()
            .storage_blob_commit(new_ticket)
            .unwrap();
        assert_eq!(
            services
                .lock()
                .unwrap()
                .storage_get("new", server_types::StorageScope::World)
                .unwrap()
                .unwrap(),
            b"data"
        );
        let pending = services
            .lock()
            .unwrap()
            .storage_blob_begin("pending", server_types::StorageScope::World, 4)
            .unwrap();
        drop(replacement);
        assert!(
            services
                .lock()
                .unwrap()
                .storage_blob_write(b"data", pending, 0)
                .is_err()
        );
        drop(services);
        std::fs::remove_dir_all(root).unwrap();
    }
}

impl ServerFullMod {
    pub fn instantiate(
        runtime: &mut Sandbox,
        id: impl Into<String>,
        component_bytes: &[u8],
        mut host: Box<dyn ServerHost>,
        unwired: &[&str],
    ) -> Result<Self, LoadError> {
        let id = id.into();
        let tickets = TicketGuard::new(host.services());
        let mut raw = runtime
            .compile(component_bytes)
            .map_err(LoadError::new)?
            .instantiate(id.clone(), ServerContext::new(id, host), |linker| {
                server_world::ServerFull::add_to_linker::<
                    HostState<ServerContext>,
                    HasSelf<ServerContext>,
                >(linker, |state| &mut state.host)
                .map_err(|error| error.to_string())
            })
            .map_err(LoadError::new)?;
        let exports = server_world::ServerFull::new(&mut raw.store, &raw.instance)
            .map_err(|error| LoadError::new(error.to_string()))?;
        let names = raw.call_init(crate::TimeLimits::SERVER, |store, _| {
            exports
                .call_init(store)
                .map_err(|error| format!("{error:#}"))
        });
        flush_output(&mut raw, |host, line| host.log_output(line));
        let names = names.ok_or_else(|| LoadError::new("mod init trapped or timed out"))?;
        let subscriptions = Subscriptions::for_full_world(FullWorld::Server, names, unwired)
            .map_err(LoadError::new)?;
        let mut module = Self {
            raw,
            exports,
            subscriptions,
            tickets,
        };
        module.notify_config_changes();
        Ok(module)
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

    /// Calls the module's `handle-call` export (another package's `mod-call`). `None` when the
    /// module is disabled, trapped or ran out of its limits on this call (it is then disabled).
    pub fn handle_call(
        &mut self,
        caller: &str,
        method: &str,
        payload: &[u8],
    ) -> Option<Result<Vec<u8>, String>> {
        self.notify_config_changes();
        if self.raw.is_disabled() {
            return None;
        }
        let exports = &self.exports;
        let result = self.raw.call("handle_call", |store, _| {
            exports.call_handle_call(store, caller, method, payload).map_err(|error| format!("{error:#}"))
        });
        flush_output(&mut self.raw, |host, line| host.log_output(line));
        self.notify_config_changes();
        result
    }

    pub fn notify_config_changes(&mut self) {
        if self.raw.is_disabled() {
            self.tickets.abort();
            return;
        }
        let Ok(services) = self.raw.store.data_mut().host.host.services() else {
            return;
        };
        let changes = match services.lock() {
            Ok(mut services) => services.take_changes(),
            Err(_) => return,
        };
        if !self.subscribed("config_changed") {
            return;
        }
        for (key, value) in changes {
            let exports = &self.exports;
            self.raw.call("config_changed", |store, _| {
                exports
                    .yoake_mods_server_hooks()
                    .call_config_changed(store, &key, &value)
                    .map_err(|error| format!("{error:#}"))
            });
            flush_output(&mut self.raw, |host, line| host.log_output(line));
            if self.raw.is_disabled() {
                self.tickets.abort();
                return;
            }
        }
    }

    /// Like [`ClientFullMod::dispatch`], for a server-side hook.
    pub fn dispatch<R>(
        &mut self,
        hook: &str,
        call: impl FnOnce(&server_world::ServerFull, &mut Store<HostState<ServerContext>>) -> wasmtime::Result<R>,
    ) -> Option<R> {
        self.notify_config_changes();
        if !self.subscribed(hook) {
            return None;
        }
        let exports = &self.exports;
        let result = self.raw.call(hook, |store, _| call(exports, store).map_err(|error| format!("{error:#}")));
        flush_output(&mut self.raw, |host, line| host.log_output(line));
        self.notify_config_changes();
        result
    }
}
