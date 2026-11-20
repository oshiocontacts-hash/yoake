//! The server services of `server-full` (`server-services` of `crates/yoake-wasm/wit/yoake-full.wit`),
//! forwarded to the game's native host services. Every service validates what the guest passes
//! (finite numbers, sizes, registry ids, UUIDs) before the game sees it, and fails closed: an `Err`
//! leaves the game as it was. A mod is trusted inside the game (it may change other players'
//! inventories, break blocks, move items); what it never gets is anything that reaches outside the
//! game folder or the PC: storage is sandboxed per mod, commands are gated and denied lists apply.
//! Not forwarded (they answer "unsupported"): `block-entity-read` and `block-entity-patch-call`
//! (a block-entity hook returns its new state directly), `range-override-*` (voice, deferred).
use std::sync::{Arc, Mutex, Weak};

use super::full::{
    GameServerHost, SideState, checked, game_relative, item_from, item_w, message_text, parse_uuid, read_service,
    server_service, st_player_with, uuid_text, valid_item_id,
};
use super::*;
use super::mob_services as mob;
use crate::mobs::custom::script::Command;
use yoake_wasm::full_bindings::client::yoake::mods::engine_types as et;
use yoake_wasm::{ServerFullMod, ServerHost, server_types as st};

/// Largest value of a storage write, a `mod-call` payload and its answer.
pub(super) const MAX_BLOB: usize = 1 << 20;
/// Most items or slots in a list a guest passes.
const MAX_LIST: usize = 512;
/// Nested `mod-call`s, the outer one included.
const MAX_CALL_DEPTH: usize = 4;

// ---------------------------------------------------------------------------------------------
// Value checks and conversions
// ---------------------------------------------------------------------------------------------

fn finite(values: &[f64]) -> Result<(), String> {
    if values.iter().all(|v| v.is_finite()) { Ok(()) } else { Err("numbers must be finite".into()) }
}

fn vec3(v: &st::Vec3) -> Result<[f64; 3], String> {
    finite(&[v.x, v.y, v.z])?;
    Ok([v.x, v.y, v.z])
}

fn block_pos(p: &st::BlockPos) -> [i32; 3] {
    [p.x, p.y, p.z]
}

/// A `namespace:path` id (dimension, sound, item, advancement...).
fn registry_id(id: &str) -> Result<(), String> {
    if valid_item_id(id) { Ok(()) } else { Err(format!("not a registry id: {id:.64}")) }
}

fn dimension_id(id: &str) -> Result<(), String> {
    registry_id(id)
}

fn st_item(stack: &ItemStack) -> st::ItemStack {
    let wire = item_w(stack);
    st::ItemStack { item: wire.id, count: wire.count, components: wire.components }
}

fn st_slots(slots: &[Option<ItemStack>]) -> Vec<Option<st::ItemStack>> {
    slots.iter().map(|slot| slot.as_ref().map(st_item)).collect()
}

fn game_item(stack: &st::ItemStack) -> Result<ItemStack, String> {
    item_from(&et::ItemStack { id: stack.item.clone(), count: stack.count, components: stack.components.clone() })
        .ok_or_else(|| format!("not a valid stack: {:.64} x{}", stack.item, stack.count))
}

fn game_slots(slots: Vec<Option<st::ItemStack>>) -> Result<Vec<Option<ItemStack>>, String> {
    if slots.len() > MAX_LIST {
        return Err("too many slots".into());
    }
    slots.iter().map(|slot| slot.as_ref().map(game_item).transpose()).collect()
}

pub(super) fn sound_category(name: &str) -> Result<NativeSoundCategory, String> {
    Ok(match name {
        "master" => NativeSoundCategory::Master,
        "music" => NativeSoundCategory::Music,
        "record" | "records" => NativeSoundCategory::Records,
        "weather" => NativeSoundCategory::Weather,
        "block" | "blocks" => NativeSoundCategory::Blocks,
        "hostile" => NativeSoundCategory::Hostile,
        "neutral" => NativeSoundCategory::Neutral,
        "player" | "players" => NativeSoundCategory::Players,
        "ambient" => NativeSoundCategory::Ambient,
        "voice" => NativeSoundCategory::Voice,
        "ui" => NativeSoundCategory::Ui,
        other => return Err(format!("unknown sound category {other:.32}")),
    })
}

/// A command argument word: letters, digits and `_ . - + : /` only, so no argument can start another
/// command or reach a selector.
fn command_word(word: &str, what: &str) -> Result<(), String> {
    let ok = !word.is_empty()
        && word.len() <= 64
        && word.bytes().all(|b| b.is_ascii_alphanumeric() || matches!(b, b'_' | b'.' | b'-' | b'+' | b':' | b'/'));
    if ok { Ok(()) } else { Err(format!("not a valid {what}")) }
}

/// A level or generator id a guest passes: `namespace:path`, usable as a folder name.
fn level_id(id: &str) -> Result<(), String> {
    horizon_client::level_registry::check_level_id(id).map_err(|reason| format!("id `{id:.64}`: {reason}"))
}

/// A JSON object a guest passes (a level stem, a dimension type): at most 64 KiB.
fn level_json(what: &str, text: &str) -> Result<serde_json::Value, String> {
    if text.len() > 1 << 16 {
        return Err(format!("the {what} is larger than 64 KiB"));
    }
    match serde_json::from_str::<serde_json::Value>(text) {
        Ok(v @ serde_json::Value::Object(_)) => Ok(v),
        Ok(_) => Err(format!("the {what} is not a JSON object")),
        Err(e) => Err(format!("the {what}: {e}")),
    }
}

fn vitals_float(value: f32) -> f32 {
    if value.is_finite() { value } else { 0.0 }
}

// ---------------------------------------------------------------------------------------------
// mod-call: the registry of server modules and the call
// ---------------------------------------------------------------------------------------------

/// A package's server module with its dirty flag; shared with the registry of `mod-call`.
pub(super) struct ServerSlot {
    pub(super) state: Mutex<SideState<ServerFullMod>>,
    pub(super) dirty: std::sync::atomic::AtomicBool,
}

static ENDPOINTS: Mutex<Vec<(String, Weak<ServerSlot>)>> = Mutex::new(Vec::new());

thread_local! {
    static CALL_DEPTH: std::cell::Cell<usize> = const { std::cell::Cell::new(0) };
}

/// Makes a package's server module reachable by `mod-call` under its id (replacing an older one).
pub(super) fn register_endpoint(id: &str, slot: &Arc<ServerSlot>) {
    let mut endpoints = ENDPOINTS.lock().unwrap_or_else(|error| error.into_inner());
    endpoints.retain(|(name, weak)| name != id && weak.strong_count() > 0);
    endpoints.push((id.to_owned(), Arc::downgrade(slot)));
    drop(endpoints);
    super::mob_runtime::install();
}

/// Every package's server module slot, in registration order.
pub(super) fn endpoints() -> Vec<Arc<ServerSlot>> {
    let endpoints = ENDPOINTS.lock().unwrap_or_else(|error| error.into_inner());
    endpoints.iter().filter_map(|(_, weak)| weak.upgrade()).collect()
}

pub(super) fn endpoint(id: &str) -> Option<Arc<ServerSlot>> {
    let endpoints = ENDPOINTS.lock().unwrap_or_else(|error| error.into_inner());
    endpoints.iter().find(|(name, _)| name == id).and_then(|(_, weak)| weak.upgrade())
}

/// `caller` calls `target`'s `handle-call`, with the current hook's game access (the callee's own
/// services work as in a hook of its own). The callee that is busy (it is waiting on this very
/// call, directly or through others) is refused: no re-entrancy into a caller.
pub(super) fn call_mod(caller: &str, target: &str, method: &str, payload: &[u8]) -> Result<Vec<u8>, String> {
    command_word(target, "package id")?;
    if method.is_empty() || method.len() > 128 || method.chars().any(char::is_control) {
        return Err("a method name is 1 to 128 characters of plain text".into());
    }
    if payload.len() > MAX_BLOB {
        return Err("a mod-call payload is at most 1 MiB".into());
    }
    let depth = CALL_DEPTH.with(std::cell::Cell::get);
    if depth + 1 >= MAX_CALL_DEPTH {
        return Err("mod-calls are nested at most 4 deep".into());
    }
    if target == caller {
        return Err("a package cannot call itself".into());
    }
    let slot = endpoint(target).ok_or_else(|| format!("no package {target:.64} with a server module"))?;
    super::service(|invocation| {
        let Ok(mut state) = slot.state.try_lock() else {
            return Ok(Err(format!("{target:.64} is busy: it is waiting on this call (no calls back into a waiting caller)")));
        };
        let Some(module) = state.module.as_mut() else {
            return Ok(Err(format!("{target:.64} has no running server module")));
        };
        slot.dirty.store(true, std::sync::atomic::Ordering::Relaxed);
        let host = match &mut invocation.host {
            HostAccess::None => HostAccess::None,
            HostAccess::Read(host) => HostAccess::Read(&**host),
            HostAccess::Write(host) => HostAccess::Write(&mut **host),
        };
        let mut inner = Invocation { host, batch: None, batch_start: 0, client: ClientExtras::default() };
        CALL_DEPTH.with(|d| d.set(depth + 1));
        let answer = {
            let _scope = Scope::enter(&mut inner);
            module.handle_call(caller, method, payload)
        };
        CALL_DEPTH.with(|d| d.set(depth));
        if module.is_disabled() {
            log::error!("wasm mod {target} (server) disabled during a call from {caller}");
        }
        Ok(match answer {
            None => Err(format!("{target:.64} did not answer (trapped or over its limits)")),
            Some(Ok(bytes)) if bytes.len() > MAX_BLOB => Err("the answer is larger than 1 MiB".to_owned()),
            Some(answer) => answer,
        })
    })
    .map_err(|error| format!("{error:?}"))?
}

// ---------------------------------------------------------------------------------------------
// The services
// ---------------------------------------------------------------------------------------------

impl ServerHost for GameServerHost {
    // ---- fully custom mobs (F4M-10, service.mobs) -------------------------------------------

    fn mob_spawn(&mut self, dimension: String, options: st::MobSpawnOptions, position: st::Vec3, type_id: String) -> Result<u32, String> {
        mob::gate(mob::granted(&self.id))?;
        mob::spawn(&dimension, &options, &position, &type_id)
    }
    fn mob_info_call(&mut self, entity: u32) -> Result<Option<st::MobInfo>, String> {
        mob::gate(mob::granted(&self.id))?;
        mob::info(entity)
    }
    fn mob_set_synced(&mut self, entity: u32, key: String, value: st::MobValue) -> Result<(), String> {
        mob::gate(mob::granted(&self.id))?;
        mob::set_synced(entity, &key, &value)
    }
    fn mob_get_synced(&mut self, entity: u32, key: String) -> Result<Option<st::MobValue>, String> {
        mob::gate(mob::granted(&self.id))?;
        mob::get_synced(entity, &key)
    }
    fn mob_trigger_anim(&mut self, controller: String, entity: u32, name: String, speed: f32) -> Result<bool, String> {
        mob::gate(mob::granted(&self.id))?;
        mob::trigger_anim(&controller, entity, &name, speed)
    }
    fn mob_set_controller_state(&mut self, controller: String, entity: u32, state: String) -> Result<bool, String> {
        mob::gate(mob::granted(&self.id))?;
        mob::set_controller_state(&controller, entity, &state)
    }
    fn mob_set_controller_state_hold(&mut self, controller: String, entity: u32, hold_ticks: Option<u32>, state: String) -> Result<bool, String> {
        mob::gate(mob::granted(&self.id))?;
        mob::set_controller_state_hold(&controller, entity, &state, hold_ticks)
    }
    fn mob_release_controller_state(&mut self, controller: String, entity: u32) -> Result<bool, String> {
        mob::gate(mob::granted(&self.id))?;
        mob::release_controller_state(&controller, entity)
    }
    fn mob_navigate_to(&mut self, entity: u32, position: st::Vec3, speed: f64) -> Result<bool, String> {
        mob::gate(mob::granted(&self.id))?;
        mob::navigate_to(entity, &position, speed)
    }
    fn mob_navigate_stop(&mut self, entity: u32) -> Result<(), String> {
        mob::gate(mob::granted(&self.id))?;
        mob::simple(entity, Command::NavigateStop)
    }
    fn mob_look_at(&mut self, entity: u32, pitch_speed: f32, position: st::Vec3, yaw_speed: f32) -> Result<(), String> {
        mob::gate(mob::granted(&self.id))?;
        mob::look_at(entity, pitch_speed, &position, yaw_speed)
    }
    fn mob_jump(&mut self, entity: u32) -> Result<(), String> {
        mob::gate(mob::granted(&self.id))?;
        mob::simple(entity, Command::Jump)
    }
    fn mob_set_target(&mut self, entity: u32, player: Option<String>, target: Option<u32>) -> Result<bool, String> {
        mob::gate(mob::granted(&self.id))?;
        mob::set_target(entity, player.as_deref(), target)
    }
    fn mob_attack(&mut self, damage: f32, entity: u32, knockback: f32, player: Option<String>, target: Option<u32>) -> Result<bool, String> {
        mob::gate(mob::granted(&self.id))?;
        mob::attack(damage, entity, knockback, player.as_deref(), target)
    }
    fn mob_attribute_get(&mut self, attribute: String, entity: u32) -> Result<st::AttributeValue, String> {
        mob::gate(mob::granted(&self.id))?;
        mob::attribute_get(&attribute, entity)
    }
    fn mob_attribute_set_base(&mut self, attribute: String, entity: u32, value: f64) -> Result<(), String> {
        mob::gate(mob::granted(&self.id))?;
        mob::attribute_set_base(&attribute, entity, value)
    }
    fn mob_attribute_add_modifier(&mut self, amount: f64, attribute: String, entity: u32, modifier: String, operation: st::AttributeOperation) -> Result<(), String> {
        mob::gate(mob::granted(&self.id))?;
        mob::attribute_add_modifier(amount, &attribute, entity, &modifier, operation)
    }
    fn mob_attribute_remove_modifier(&mut self, attribute: String, entity: u32, modifier: String) -> Result<bool, String> {
        mob::gate(mob::granted(&self.id))?;
        mob::attribute_remove_modifier(&attribute, entity, &modifier)
    }
    fn mob_set_hitbox_enabled(&mut self, enabled: bool, entity: u32, name: String) -> Result<(), String> {
        mob::gate(mob::granted(&self.id))?;
        mob::set_hitbox_enabled(enabled, entity, &name)
    }
    fn mob_data_get(&mut self, entity: u32) -> Result<Vec<u8>, String> {
        mob::gate(mob::granted(&self.id))?;
        mob::data_get(entity)
    }
    fn mob_data_put(&mut self, data: Vec<u8>, entity: u32) -> Result<(), String> {
        mob::gate(mob::granted(&self.id))?;
        mob::data_put(data, entity)
    }
    fn mob_despawn(&mut self, entity: u32) -> Result<bool, String> {
        mob::gate(mob::granted(&self.id))?;
        mob::simple(entity, Command::Despawn).map(|_| true)
    }
    fn mob_claim_control(&mut self, claim: bool, entity: u32) -> Result<(), String> {
        mob::gate(mob::granted(&self.id))?;
        mob::simple(entity, Command::Claim(claim))
    }
    fn mob_impulse(&mut self, entity: u32, velocity: st::Vec3) -> Result<(), String> {
        mob::gate(mob::granted(&self.id))?;
        mob::impulse(entity, &velocity)
    }
    fn mob_seat_set(&mut self, entity: u32, rider: Option<u32>, seat: u32) -> Result<bool, String> {
        mob::gate(mob::granted(&self.id))?;
        mob::seat_set(entity, rider, seat)
    }

    fn services(&mut self) -> Result<yoake_wasm::package_services::ServiceHandle, String> {
        super::package::services(&self.id, horizon_mods::yak::ConfigRuntimeSide::Server)
    }

    fn log(&mut self, target: &str, message: &str) -> Result<(), w::HostError> {
        let clean = |text: &str| -> String {
            text.chars().take(MAX_LOG_CHARS).map(|c| if c.is_control() { ' ' } else { c }).collect()
        };
        emit(format!("wasm mod {} [{}]: {}", self.id, clean(target), clean(message)));
        Ok(())
    }

    // ---- players ----------------------------------------------------------------------------

    fn acting_player(&mut self) -> Option<st::Player> {
        read_service(|host| Ok(host.acting_player().map(|player| st_player_with(&player, host.player_sneaking(player.uuid).unwrap_or(false)))))
            .ok()
            .flatten()
    }

    fn held(&mut self) -> Option<st::ItemStack> {
        read_service(|host| Ok(host.held_stack().map(|stack| st_item(&stack)))).ok().flatten()
    }

    fn server_players(&mut self) -> Result<Vec<st::Player>, String> {
        read_service(|host| {
            Ok(host.server_players().iter().map(|p| st_player_with(p, host.player_sneaking(p.uuid).unwrap_or(false))).collect())
        })
    }

    fn integrated_owner(&mut self) -> Option<String> {
        read_service(|host| Ok(host.integrated_owner().map(uuid_text))).ok().flatten()
    }

    fn player_sneaking(&mut self, player: String) -> Result<bool, String> {
        let uuid = parse_uuid(&player)?;
        read_service(|host| host.player_sneaking(uuid))
    }

    fn player_set_food(&mut self, food: i32, player: String, saturation: f32) -> Result<(), String> {
        let uuid = parse_uuid(&player)?;
        if !(0..=20).contains(&food) || !saturation.is_finite() || saturation < 0.0 {
            return Err("food is 0 to 20 and saturation is not negative".into());
        }
        server_service(|host| host.player_set_food(uuid, food, saturation))
    }

    // ---- levels and generators (F3-8) ---------------------------------------------------------
    // The registering ones run from `init`, before any world exists: they stage what they are
    // given for every world (`horizon_client::mod_generator`). In a hook, with a world up, they
    // reach the world's registry directly (and are refused once it has ticked). The others need
    // the server's hook.

    fn level_register_type(&mut self, id: String, type_json: String) -> Result<(), String> {
        level_id(&id)?;
        horizon_client::mod_generator::stage_type(&id, level_json("dimension type", &type_json)?)
    }

    fn level_register(&mut self, id: String, stem_json: String) -> Result<(), String> {
        level_id(&id)?;
        let stem = level_json("level stem", &stem_json)?;
        if super::full::initializing_server(&self.id) {
            horizon_client::mod_generator::stage_level_replacing(&id, stem)
        } else {
            server_service(|host| host.levels_register(&id, &stem_json))
        }
    }

    fn generator_register(&mut self, id: String) -> Result<(), String> {
        level_id(&id)?;
        if super::full::initializing_server(&self.id) {
            horizon_client::mod_generator::stage_generator_replacing(&id, super::level_services::generator_factory(&self.id))
        } else {
            server_service(|host| host.levels_generator(&id, &self.id))
        }
    }

    fn level_create(&mut self, id: String, persistent: bool, stem_json: String) -> Result<(), String> {
        level_id(&id)?;
        level_json("level stem", &stem_json)?;
        server_service(|host| host.levels_create(&id, persistent, &stem_json))
    }

    fn level_delete(&mut self, id: String) -> Result<(), String> {
        level_id(&id)?;
        server_service(|host| host.levels_delete(&id))
    }

    fn level_unload(&mut self, id: String) -> Result<(), String> {
        level_id(&id)?;
        server_service(|host| host.levels_unload(&id))
    }

    fn level_regenerate(&mut self, id: String, seed: Option<i64>) -> Result<(), String> {
        level_id(&id)?;
        server_service(|host| host.levels_regenerate(&id, seed))
    }

    fn current_level(&mut self, player: String) -> Result<String, String> {
        let uuid = parse_uuid(&player)?;
        read_service(|host| {
            host.server_players()
                .into_iter()
                .find(|p| p.uuid == uuid)
                .map(|p| p.dimension)
                .ok_or_else(|| "the player is not here".to_owned())
        })
    }

    fn level_list(&mut self) -> Result<Vec<String>, String> {
        read_service(|host| Ok(host.levels_list()))
    }

    fn player_stacks(&mut self, player: String) -> Result<Vec<st::PlayerInventorySlot>, String> {
        let uuid = parse_uuid(&player)?;
        let slots = read_service(|host| {
            host.player_inventory_slots(uuid)
                .ok_or_else(|| "the player is not here".to_owned())
        })?;
        Ok(slots
            .iter()
            .enumerate()
            .map(|(id, stack)| st::PlayerInventorySlot {
                id: id as u32,
                stack: stack.as_ref().map(st_item),
            })
            .collect())
    }

    fn player_remove_slot(&mut self, expected: st::ItemStack, player: String, slot: u32) -> Result<bool, String> {
        let uuid = parse_uuid(&player)?;
        let expected = game_item(&expected)?;
        server_service(|host| host.player_remove_slot(uuid, slot, expected))
    }

    fn player_teleport(&mut self, dimension: String, pitch: f32, player: String, position: st::Vec3, yaw: f32) -> Result<(), String> {
        let uuid = parse_uuid(&player)?;
        dimension_id(&dimension)?;
        let position = vec3(&position)?;
        finite(&[f64::from(pitch), f64::from(yaw)])?;
        server_service(|host| host.player_teleport(uuid, &dimension, position, yaw, pitch).map(|_| ()))
    }

    fn player_set_health(&mut self, health: f32, player: String) -> Result<(), String> {
        let uuid = parse_uuid(&player)?;
        finite(&[f64::from(health)])?;
        server_service(|host| checked(host.player_set_health(uuid, health), "set health"))
    }

    fn player_set_gamemode(&mut self, game_mode: i32, player: String) -> Result<(), String> {
        let uuid = parse_uuid(&player)?;
        let mode = match game_mode {
            0 => NativeGameMode::Survival,
            1 => NativeGameMode::Creative,
            2 => NativeGameMode::Adventure,
            3 => NativeGameMode::Spectator,
            _ => return Err("a game mode is 0 to 3".into()),
        };
        server_service(|host| host.player_set_gamemode(uuid, mode).and_then(|done| checked(done, "set game mode")))
    }

    fn player_clear_inventory(&mut self, player: String) -> Result<(), String> {
        let uuid = parse_uuid(&player)?;
        server_service(|host| checked(host.player_clear_inventory(uuid), "clear inventory"))
    }

    fn player_effect_give(&mut self, ambient: bool, amplifier: u8, duration_ticks: u32, effect: String, icon: bool, particles: bool, player: String) -> Result<(), String> {
        let uuid = parse_uuid(&player)?;
        registry_id(&effect)?;
        let duration = i32::try_from(duration_ticks).map_err(|_| "duration too long".to_owned())?;
        server_service(|host| host.player_effect_give(uuid, &effect, duration, amplifier, ambient, particles, icon).and_then(|done| checked(done, "give effect")))
    }

    fn player_effect_clear(&mut self, effect: Option<String>, player: String) -> Result<(), String> {
        let uuid = parse_uuid(&player)?;
        if let Some(effect) = &effect { registry_id(effect)?; }
        server_service(|host| host.player_effect_clear(uuid, effect.as_deref()).map(|_| ()))
    }

    fn player_attribute_modifier_add(&mut self, amount: f64, attribute: String, modifier: String, operation: st::AttributeOperation, player: String) -> Result<(), String> {
        let uuid = parse_uuid(&player)?;
        finite(&[amount])?;
        command_word(&attribute, "attribute id")?;
        command_word(&modifier, "modifier id")?;
        let operation = match operation {
            st::AttributeOperation::AddValue => NativeAttributeOperation::AddValue,
            st::AttributeOperation::AddMultipliedBase => NativeAttributeOperation::AddMultipliedBase,
            st::AttributeOperation::AddMultipliedTotal => NativeAttributeOperation::AddMultipliedTotal,
        };
        server_service(|host| host.player_attribute_modifier_add(uuid, &attribute, &modifier, amount, operation))
    }

    fn player_attribute_modifier_remove(&mut self, attribute: String, modifier: String, player: String) -> Result<(), String> {
        let uuid = parse_uuid(&player)?;
        command_word(&attribute, "attribute id")?;
        command_word(&modifier, "modifier id")?;
        server_service(|host| host.player_attribute_modifier_remove(uuid, &attribute, &modifier).map(|_| ()))
    }

    fn attribute_get(&mut self, attribute: String, player: String) -> Result<st::AttributeValue, String> {
        let uuid = parse_uuid(&player)?;
        command_word(&attribute, "attribute id")?;
        server_service(|host| host.attribute_get(uuid, &attribute).map(|(base, value)| st::AttributeValue { base, value }))
    }

    fn tell(&mut self, message: String, player: String) -> Result<(), String> {
        let (uuid, text) = (parse_uuid(&player)?, message_text(&message)?);
        server_service(|host| checked(host.tell(uuid, text), "tell"))
    }

    fn action_bar(&mut self, message: String, player: String) -> Result<(), String> {
        let (uuid, text) = (parse_uuid(&player)?, message_text(&message)?);
        server_service(|host| checked(host.action_bar(uuid, text), "action bar"))
    }

    fn player_title(&mut self, fade_in: i32, fade_out: i32, player: String, stay: i32, subtitle: Option<String>, title: Option<String>) -> Result<(), String> {
        let uuid = parse_uuid(&player)?;
        server_service(|host| checked(host.player_title(uuid, title.as_deref(), subtitle.as_deref(), fade_in, stay, fade_out), "title"))
    }

    fn player_clear_title(&mut self, player: String) -> Result<(), String> {
        let uuid = parse_uuid(&player)?;
        server_service(|host| checked(host.player_clear_title(uuid), "clear title"))
    }

    fn player_play_sound(&mut self, category: String, pitch: f32, player: String, position: st::Vec3, sound: String, volume: f32) -> Result<(), String> {
        let uuid = parse_uuid(&player)?;
        registry_id(&sound)?;
        let category = sound_category(&category)?;
        let at = vec3(&position)?;
        finite(&[f64::from(pitch), f64::from(volume)])?;
        server_service(|host| checked(host.player_play_sound(uuid, &sound, category, at[0], at[1], at[2], volume, pitch), "play sound"))
    }

    fn play_sound_at(&mut self, category: String, dimension: String, pitch: f32, position: st::Vec3, sound: String, volume: f32) -> Result<u32, String> {
        registry_id(&sound)?;
        dimension_id(&dimension)?;
        let category = sound_category(&category)?;
        let at = vec3(&position)?;
        finite(&[f64::from(pitch), f64::from(volume)])?;
        let range = 16.0 * f64::from(volume.clamp(1.0, 16.0));
        server_service(|host| {
            let mut reached = 0u32;
            for player in host.server_players() {
                let near = (0..3).map(|i| (player.position[i] - at[i]).powi(2)).sum::<f64>() <= range * range;
                if player.dimension == dimension && near
                    && host.player_play_sound(player.uuid, &sound, category, at[0], at[1], at[2], volume, pitch)
                {
                    reached += 1;
                }
            }
            Ok(reached)
        })
    }

    fn server_player_motion(&mut self, player: String) -> Result<st::PlayerMotion, String> {
        let uuid = parse_uuid(&player)?;
        server_service(|host| {
            host.server_player_motion(uuid).map(|m| st::PlayerMotion {
                position: st::Vec3 { x: m.position[0], y: m.position[1], z: m.position[2] },
                velocity: st::Vec3 { x: m.velocity[0], y: m.velocity[1], z: m.velocity[2] },
                on_ground: m.on_ground,
                horizontal_collision: m.horizontal_collision,
                fall_distance: m.fall_distance,
            })
        })
    }

    fn set_server_player_velocity(&mut self, dimension: String, player: String, velocity: st::Vec3) -> Result<bool, String> {
        let uuid = parse_uuid(&player)?;
        dimension_id(&dimension)?;
        let velocity = vec3(&velocity)?;
        server_service(|host| host.set_server_player_velocity(&dimension, uuid, velocity))
    }

    fn set_player_attack_hitbox(&mut self, height: f32, player: String, width: f32) -> Result<bool, String> {
        let uuid = parse_uuid(&player)?;
        finite(&[f64::from(height), f64::from(width)])?;
        server_service(|host| host.set_player_attack_hitbox(uuid, width, height))
    }

    fn clear_player_attack_hitbox(&mut self, player: String) -> Result<bool, String> {
        let uuid = parse_uuid(&player)?;
        server_service(|host| host.clear_player_attack_hitbox(uuid))
    }

    // ---- inventories, containers, items -------------------------------------------------------

    fn player_set_slots(&mut self, player: String, slots: Vec<Option<st::ItemStack>>) -> Result<(), String> {
        let uuid = parse_uuid(&player)?;
        let slots = game_slots(slots)?;
        server_service(|host| host.player_set_slots(uuid, slots))
    }

    fn give_items(&mut self, items: Vec<st::ItemStack>, player: String) -> Result<Vec<st::ItemStack>, String> {
        let uuid = parse_uuid(&player)?;
        if items.len() > MAX_LIST {
            return Err("too many stacks".into());
        }
        let stacks = items.iter().map(game_item).collect::<Result<Vec<_>, _>>()?;
        server_service(|host| {
            let mut rest = Vec::new();
            for stack in stacks {
                if let Some(left) = host.give_stack(uuid, stack) {
                    rest.push(st_item(&left));
                }
            }
            Ok(rest)
        })
    }

    fn inventory_apply(&mut self, change: st::InventoryChange, player: String) -> Result<bool, String> {
        let uuid = parse_uuid(&player)?;
        let exact = change.components.is_some();
        let stack = game_item(&st::ItemStack { item: change.item, count: change.count, components: change.components.unwrap_or_default() })?;
        let give = matches!(change.operation, st::InventoryOperation::Give);
        server_service(|host| host.inventory_apply(uuid, give, stack, exact))
    }

    fn container_slots(&mut self, dimension: String, position: st::BlockPos) -> Result<Vec<Option<st::ItemStack>>, String> {
        dimension_id(&dimension)?;
        read_service(|host| host.container_all_slots(&dimension, block_pos(&position)).map(|slots| st_slots(&slots)))
    }

    fn container_set_slots(&mut self, dimension: String, position: st::BlockPos, slots: Vec<Option<st::ItemStack>>) -> Result<(), String> {
        dimension_id(&dimension)?;
        let slots = game_slots(slots)?;
        server_service(|host| host.container_replace_slots(&dimension, block_pos(&position), slots))
    }

    fn containers_in(&mut self, center: st::Vec3, dimension: String, radius: f64) -> Result<Vec<st::ContainerRef>, String> {
        dimension_id(&dimension)?;
        let center = vec3(&center)?;
        finite(&[radius])?;
        read_service(|host| {
            host.containers_near(&dimension, center, radius).map(|found| {
                found
                    .into_iter()
                    .map(|(at, kind, slots, block)| st::ContainerRef { position: st::BlockPos { x: at[0], y: at[1], z: at[2] }, kind, slots, block })
                    .collect()
            })
        })
    }

    fn open_menu(&mut self, extra: Vec<u8>, menu_type: String, player: String, title: String) -> Result<bool, String> {
        let uuid = parse_uuid(&player)?;
        registry_id(&menu_type)?;
        if extra.len() > abi::MAX_DATA_BYTES {
            return Err("menu data too large".into());
        }
        let title = message_text(&title)?;
        server_service(|host| Ok(host.open_menu(uuid, &menu_type, title, &extra)))
    }

    fn menu_set_items(&mut self, items: Vec<Option<st::ItemStack>>, player: String) -> Result<(), String> {
        let uuid = parse_uuid(&player)?;
        let items = game_slots(items)?;
        server_service(|host| {
            host.set_menu_items(uuid, items);
            Ok(())
        })
    }

    fn menu_close(&mut self, player: String) -> Result<(), String> {
        let uuid = parse_uuid(&player)?;
        server_service(|host| {
            host.close_menu(uuid);
            Ok(())
        })
    }

    fn item_entities_in(&mut self, center: st::Vec3, dimension: String, radius: f64) -> Result<Vec<st::ItemEntity>, String> {
        dimension_id(&dimension)?;
        let center = vec3(&center)?;
        finite(&[radius])?;
        read_service(|host| {
            host.item_entities_in(&dimension, center, radius).map(|found| {
                found
                    .into_iter()
                    .map(|(handle, at, velocity, stack)| st::ItemEntity {
                        handle,
                        position: st::Vec3 { x: at[0], y: at[1], z: at[2] },
                        velocity: st::Vec3 { x: velocity[0], y: velocity[1], z: velocity[2] },
                        stack: st_item(&stack),
                    })
                    .collect()
            })
        })
    }

    fn item_entity_move(&mut self, dimension: String, entity: u32, position: st::Vec3, velocity: st::Vec3) -> Result<bool, String> {
        dimension_id(&dimension)?;
        let (position, velocity) = (vec3(&position)?, vec3(&velocity)?);
        server_service(|host| host.item_entity_move(&dimension, entity, position, velocity))
    }

    fn item_entity_pickup(&mut self, dimension: String, entity: u32, player: String) -> Result<bool, String> {
        let uuid = parse_uuid(&player)?;
        dimension_id(&dimension)?;
        server_service(|host| host.item_entity_pickup(&dimension, entity, uuid))
    }

    // ---- registries ----------------------------------------------------------------------------

    fn block_tags(&mut self, block: String) -> Result<Vec<String>, String> {
        registry_id(&block)?;
        read_service(|host| Ok(host.block_tags(&block)))
    }

    fn item_tags(&mut self, item: String) -> Result<Vec<String>, String> {
        registry_id(&item)?;
        read_service(|host| Ok(host.item_tags(&item)))
    }

    fn block_tool_requirement(&mut self, block: String) -> Result<st::ToolRequirement, String> {
        registry_id(&block)?;
        read_service(|host| {
            host.block_tool_requirement(&block)
                .map(|(hardness, requires_tool, mineable, tier)| st::ToolRequirement { hardness: vitals_float(hardness), requires_tool, mineable, tier })
                .ok_or_else(|| "unknown block".to_owned())
        })
    }

    fn can_harvest(&mut self, block: String, tool: Option<st::ItemStack>) -> Result<bool, String> {
        registry_id(&block)?;
        let tool = tool.as_ref().map(game_item).transpose()?;
        read_service(|host| host.can_harvest(&block, tool.as_ref()).ok_or_else(|| "unknown block".to_owned()))
    }

    // ---- advancements, vitals, damage ----------------------------------------------------------

    fn advancement_progress(&mut self, advancement: String, player: String) -> Result<st::AdvancementState, String> {
        let uuid = parse_uuid(&player)?;
        registry_id(&advancement)?;
        server_service(|host| {
            host.advancement_progress(uuid, &advancement)
                .map(|(known, done, completed, remaining)| st::AdvancementState { known, done, completed, remaining })
        })
    }

    fn advancement_grant(&mut self, advancement: String, player: String) -> Result<bool, String> {
        let uuid = parse_uuid(&player)?;
        registry_id(&advancement)?;
        server_service(|host| host.advancement_grant(uuid, &advancement))
    }

    fn entity_vitals_of(&mut self, dimension: String, entity: u32) -> Result<st::EntityVitals, String> {
        dimension_id(&dimension)?;
        server_service(|host| {
            host.entity_vitals(&dimension, entity).map(|(health, max, armor, absorption)| st::EntityVitals {
                health: vitals_float(health),
                max_health: vitals_float(max),
                armor: vitals_float(armor),
                absorption: vitals_float(absorption),
            })
        })
    }

    fn hurt_entity(&mut self, amount: f32, attacker: Option<String>, damage: st::DamageKind, dimension: String, entity: u32) -> Result<bool, String> {
        dimension_id(&dimension)?;
        if !amount.is_finite() || amount < 0.0 {
            return Err("damage must be finite and not negative".into());
        }
        let actor = match attacker {
            Some(uuid) => NativeDamageActor::Player(parse_uuid(&uuid)?),
            None => NativeDamageActor::None,
        };
        let kind = match damage {
            st::DamageKind::Magic => NativeDamageKind::Magic,
            st::DamageKind::PlayerAttack => NativeDamageKind::PlayerAttack,
            st::DamageKind::Explosion => NativeDamageKind::Explosion,
        };
        let damage = NativeDamage::new(kind, actor, None);
        server_service(|host| host.hurt_entity(&dimension, entity, amount, &damage))
    }

    // ---- bossbars ------------------------------------------------------------------------------

    fn bossbar_create(&mut self, bar: String, name: String) -> Result<(), String> {
        server_service(|host| checked(host.bossbar_create(&bar, &name), "boss bar"))
    }

    fn bossbar_remove(&mut self, bar: String) -> Result<(), String> {
        server_service(|host| checked(host.bossbar_remove(&bar), "remove boss bar"))
    }

    fn bossbar_set_color(&mut self, bar: String, color: String) -> Result<(), String> {
        server_service(|host| checked(host.bossbar_set_color(&bar, &color), "boss bar colour"))
    }

    fn bossbar_set_name(&mut self, bar: String, name: String) -> Result<(), String> {
        server_service(|host| checked(host.bossbar_set_name(&bar, &name), "boss bar name"))
    }

    fn bossbar_set_overlay(&mut self, bar: String, overlay: String) -> Result<(), String> {
        server_service(|host| checked(host.bossbar_set_overlay(&bar, &overlay), "boss bar overlay"))
    }

    fn bossbar_set_players(&mut self, bar: String, players: Vec<String>) -> Result<(), String> {
        if players.len() > MAX_LIST {
            return Err("too many players".into());
        }
        let uuids = players.iter().map(|p| parse_uuid(p)).collect::<Result<Vec<_>, _>>()?;
        server_service(|host| checked(host.bossbar_set_players(&bar, &uuids), "boss bar players"))
    }

    fn bossbar_set_progress(&mut self, bar: String, progress: f32) -> Result<(), String> {
        finite(&[f64::from(progress)])?;
        server_service(|host| checked(host.bossbar_set_progress(&bar, progress), "boss bar progress"))
    }

    // ---- commands, world ---------------------------------------------------------------------

    fn run_command(&mut self, command: String) -> Result<bool, String> {
        super::full::command_gate(&command, self.commands)?;
        // The guest's line goes to the game's own commands, never to another mod's command hook.
        let _guest = crate::mod_api::guest_command_scope();
        server_service(|host| Ok(host.run_command(&command)))
    }

    fn block(&mut self, position: st::BlockPos) -> Result<String, String> {
        read_service(|host| Ok(host.block(block_pos(&position))))
    }

    fn block_property(&mut self, position: st::BlockPos, property: String) -> Result<Option<String>, String> {
        read_service(|host| Ok(host.block_property(block_pos(&position), &property)))
    }

    fn set_block_property(&mut self, flags: i32, position: st::BlockPos, property: String, value: String) -> Result<bool, String> {
        if property.len() > 64 || value.len() > 64 {
            return Err("property names and values are at most 64 characters".into());
        }
        server_service(|host| Ok(host.set_block_property_with(block_pos(&position), &property, &value, flags)))
    }

    fn block_collision(&mut self, position: st::BlockPos) -> Result<Vec<st::Bounds>, String> {
        read_service(|host| {
            Ok(host
                .block_collision(block_pos(&position))
                .into_iter()
                .map(|b| st::Bounds {
                    min: st::Vec3 { x: b[0], y: b[1], z: b[2] },
                    max: st::Vec3 { x: b[3], y: b[4], z: b[5] },
                })
                .collect())
        })
    }

    fn entities_in(&mut self, center: st::Vec3, dimension: String, radius: f64) -> Result<Vec<st::EntitySnapshot>, String> {
        dimension_id(&dimension)?;
        let center = vec3(&center)?;
        finite(&[radius])?;
        server_service(|host| {
            host.entities_in(&dimension, center, radius).map(|found| {
                found
                    .into_iter()
                    .map(|e| st::EntitySnapshot {
                        handle: e.id,
                        kind: e.kind,
                        dimension: e.dimension,
                        position: st::Vec3 { x: e.position[0], y: e.position[1], z: e.position[2] },
                        bounds: st::Bounds {
                            min: st::Vec3 { x: e.bounding_box[0][0], y: e.bounding_box[0][1], z: e.bounding_box[0][2] },
                            max: st::Vec3 { x: e.bounding_box[1][0], y: e.bounding_box[1][1], z: e.bounding_box[1][2] },
                        },
                        alive: e.alive,
                        player_uuid: e.player_uuid.map(uuid_text),
                    })
                    .collect()
            })
        })
    }

    fn clip(&mut self, dimension: String, end: st::Vec3, start: st::Vec3) -> Result<Option<st::RayHit>, String> {
        dimension_id(&dimension)?;
        let (from, to) = (vec3(&start)?, vec3(&end)?);
        server_service(|host| {
            host.clip(&dimension, from, to).map(|hit| {
                hit.map(|h| st::RayHit {
                    position: st::Vec3 { x: h.location[0], y: h.location[1], z: h.location[2] },
                    block: st::BlockPos { x: h.pos[0], y: h.pos[1], z: h.pos[2] },
                    face: format!("{:?}", h.face).to_ascii_lowercase(),
                })
            })
        })
    }

    fn break_block_as(&mut self, dimension: String, player: String, position: st::BlockPos) -> Result<bool, String> {
        let uuid = parse_uuid(&player)?;
        dimension_id(&dimension)?;
        server_service(|host| host.break_block_as(&dimension, uuid, block_pos(&position)))
    }

    fn edit_blocks(&mut self, dimension: String, edit: st::BlockEdit) -> Result<u64, String> {
        dimension_id(&dimension)?;
        if edit.runs.len() > 65_536 || edit.palette.len() > 4096 || edit.filters.len() > 65_536 {
            return Err("the edit is too large".into());
        }
        let mut runs = Vec::with_capacity(edit.runs.len());
        for run in &edit.runs {
            let surface = match run.surface.as_deref() {
                None => None,
                Some("world_surface") => Some(NativeHeightmap::WorldSurface),
                Some("motion_blocking") => Some(NativeHeightmap::MotionBlocking),
                Some("motion_blocking_no_leaves") => Some(NativeHeightmap::MotionBlockingNoLeaves),
                Some("ocean_floor") => Some(NativeHeightmap::OceanFloor),
                Some(other) => return Err(format!("unknown heightmap {other:.32}")),
            };
            runs.push(NativeBlockRun { x: run.x, z: run.z, y: run.y, height: run.height, surface, block: run.block, only: run.only });
        }
        let edit = NativeBlockEdit { palette: edit.palette, filters: edit.filters, runs };
        server_service(|host| host.edit_blocks(&dimension, &edit))
    }

    fn block_entity_sync(&mut self, position: st::BlockPos) -> Result<(), String> {
        server_service(|host| {
            host.block_entity_sync(block_pos(&position));
            Ok(())
        })
    }

    fn spawn_mannequin(&mut self, player: String, pose: String, position: st::Vec3, yaw: f32) -> Result<Option<u32>, String> {
        let uuid = parse_uuid(&player)?;
        let at = vec3(&position)?;
        finite(&[f64::from(yaw)])?;
        let pose = match pose.as_str() {
            "standing" => NativeMannequinPose::Standing,
            "sleeping" => NativeMannequinPose::Sleeping,
            _ => return Err("a pose is standing or sleeping".into()),
        };
        server_service(|host| Ok(host.spawn_mannequin(at[0], at[1], at[2], yaw, uuid, pose)))
    }

    fn remove_spawned_entity(&mut self, entity: u32) -> Result<bool, String> {
        server_service(|host| Ok(host.remove_spawned_entity(entity)))
    }

    fn world_border(&mut self, dimension: String) -> Result<st::WorldBorderState, String> {
        dimension_id(&dimension)?;
        server_service(|host| {
            host.world_border(&dimension).map(|b| st::WorldBorderState {
                center_x: b.center[0],
                center_z: b.center[1],
                size: b.size,
                target_size: b.target_size,
                remaining_ticks: b.remaining_ticks,
            })
        })
    }

    fn set_world_border(&mut self, center_x: f64, center_z: f64, dimension: String, lerp_ticks: Option<u64>, size: f64, target_size: Option<f64>) -> Result<(), String> {
        dimension_id(&dimension)?;
        finite(&[center_x, center_z, size])?;
        let lerp = match (target_size, lerp_ticks) {
            (Some(target), Some(ticks)) => {
                finite(&[target])?;
                Some((target, ticks))
            }
            (None, None) => None,
            _ => return Err("target-size and lerp-ticks go together".into()),
        };
        server_service(|host| host.set_world_border(&dimension, [center_x, center_z], size, lerp))
    }

    fn world_control(&mut self, change: st::WorldControlChange) -> Result<bool, String> {
        let line = world_control_line(&change)?;
        server_service(|host| Ok(host.run_command(&line)))
    }

    fn team_apply(&mut self, change: st::TeamChange) -> Result<bool, String> {
        let line = team_line(&change)?;
        server_service(|host| Ok(host.run_command(&line)))
    }

    fn particle_spawn(&mut self, request: st::ParticleRequest) -> Result<(), String> {
        let lines = particle_lines(&request)?;
        server_service(|host| {
            let mut sent = true;
            for line in &lines {
                sent &= host.run_command(line);
            }
            checked(sent, "particle")
        })
    }

    // ---- network, storage ----------------------------------------------------------------------

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
        let handle = self.services()?;
        handle
            .lock()
            .map_err(|_| "package services unavailable")?
            .require(yoake_wasm::package_services::CONFIG_CAPABILITY)?;
        let root = crate::yak_mods::game_root().ok_or("package root unavailable")?;
        game_relative(&root.join("config")).ok_or_else(|| "path is not text".to_owned())
    }

    fn world_storage_read(&mut self, key: String) -> Result<Option<Vec<u8>>, String> {
        read_service(|_| {
            self.services()?
                .lock()
                .map_err(|_| "package services unavailable")?
                .storage_get(&key, st::StorageScope::World)
        })
    }

    fn world_storage_write(&mut self, data: Vec<u8>, key: String) -> Result<(), String> {
        server_service(|_| {
            self.services()?
                .lock()
                .map_err(|_| "package services unavailable")?
                .storage_put(&key, st::StorageScope::World, &data)
        })
    }

    fn config_read(&mut self, key: String) -> Result<Option<Vec<u8>>, String> {
        read_service(|_| {
            self.services()?
                .lock()
                .map_err(|_| "package services unavailable")?
                .storage_get(&key, st::StorageScope::Global)
        })
    }

    fn config_write(&mut self, data: Vec<u8>, key: String) -> Result<(), String> {
        server_service(|_| {
            self.services()?
                .lock()
                .map_err(|_| "package services unavailable")?
                .storage_put(&key, st::StorageScope::Global, &data)
        })
    }

    fn asset_read(&mut self, key: String) -> Result<Option<Vec<u8>>, String> {
        let plain = |p: &str| !p.is_empty() && p != "." && p != ".." && !p.contains('\\') && !p.contains(':');
        if key.len() > 256 || !key.split('/').all(plain) {
            return Err("an asset key is a relative path of plain names".into());
        }
        let path = format!("data/{key}");
        read_service(|host| Ok(host.data(&path).filter(|bytes| bytes.len() <= MAX_BLOB)))
    }

    fn mod_call(&mut self, method: String, payload: Vec<u8>, target_mod: String) -> Result<Vec<u8>, String> {
        call_mod(&self.id, &target_mod, &method, &payload)
    }

    fn attachment_define(&mut self, copy_on_death: bool, id: String, kind: String, persistent: bool, synced: bool) -> Result<(), String> {
        self.attachment_define_impl(&id, &kind, persistent, synced, copy_on_death)
    }

    fn attachment_get(&mut self, holder: String, id: String, kind: String) -> Result<Option<Vec<u8>>, String> {
        self.attachment_get_impl(&id, &kind, &holder)
    }

    fn attachment_set(&mut self, data: Vec<u8>, holder: String, id: String, kind: String) -> Result<(), String> {
        self.attachment_set_impl(&id, &kind, &holder, &data)
    }

    fn attachment_remove(&mut self, holder: String, id: String, kind: String) -> Result<bool, String> {
        self.attachment_remove_impl(&id, &kind, &holder)
    }

    // ---- diagnostics and what is not forwarded ------------------------------------------------

    fn qa_mode_enabled(&mut self) -> bool {
        false
    }

    fn qa_record(&mut self, event: String, json: String) -> Result<(), String> {
        // No QA recorder in the game: the record goes to the log, scoped to the package and bounded.
        let clean = |text: &str, limit: usize| -> String {
            text.chars().take(limit).map(|c| if c.is_control() { ' ' } else { c }).collect()
        };
        log::debug!("wasm mod {} qa {}: {}", self.id, clean(&event, 64), clean(&json, 1024));
        Ok(())
    }
}

// ---------------------------------------------------------------------------------------------
// Typed changes written as the game's own commands (only validated words reach the line)
// ---------------------------------------------------------------------------------------------

const GAME_RULE_VALUE_MAX: usize = 16;

fn world_control_line(change: &st::WorldControlChange) -> Result<String, String> {
    match change.operation {
        st::WorldControlOperation::SetTime => {
            let value = change.value.as_str();
            let named = matches!(value, "day" | "night" | "noon" | "midnight");
            if !named && value.parse::<i64>().map_or(true, |t| !(0..=24_000_000).contains(&t)) {
                return Err("a time is day, night, noon, midnight or ticks".into());
            }
            Ok(format!("time set {value}"))
        }
        st::WorldControlOperation::SetGameRule => {
            command_word(&change.key, "game rule")?;
            if change.key.contains(':') || change.value.len() > GAME_RULE_VALUE_MAX {
                return Err("not a valid game rule".into());
            }
            let value = change.value.as_str();
            if value != "true" && value != "false" && value.parse::<i32>().is_err() {
                return Err("a game rule value is true, false or a number".into());
            }
            Ok(format!("gamerule {} {}", change.key, value))
        }
        st::WorldControlOperation::AddForceLoad | st::WorldControlOperation::RemoveForceLoad => {
            dimension_id(&change.dimension)?;
            let from = change.from.as_ref().ok_or("force-load needs a from position")?;
            let to = change.to.as_ref().unwrap_or(from);
            let chunks = (i64::from((to.x >> 4) - (from.x >> 4)).abs() + 1) * (i64::from((to.z >> 4) - (from.z >> 4)).abs() + 1);
            if chunks > 256 {
                return Err("a force-load area is at most 256 chunks".into());
            }
            let verb = if matches!(change.operation, st::WorldControlOperation::AddForceLoad) { "add" } else { "remove" };
            Ok(format!("execute in {} run forceload {verb} {} {} {} {}", change.dimension, from.x, from.z, to.x, to.z))
        }
    }
}

fn team_line(change: &st::TeamChange) -> Result<String, String> {
    let name_ok = !change.team.is_empty()
        && change.team.len() <= 16
        && change.team.bytes().all(|b| b.is_ascii_alphanumeric() || matches!(b, b'_' | b'.' | b'-' | b'+'));
    if !name_ok {
        return Err("a team name is 1 to 16 letters, digits and _ . - +".into());
    }
    if change.players.len() > MAX_LIST {
        return Err("too many players".into());
    }
    let players = change.players.iter().map(|p| parse_uuid(p).map(uuid_text)).collect::<Result<Vec<_>, _>>()?;
    Ok(match change.operation {
        st::TeamOperation::Add => format!("team add {}", change.team),
        st::TeamOperation::Remove => format!("team remove {}", change.team),
        st::TeamOperation::Join | st::TeamOperation::Leave if players.is_empty() => return Err("name the players".into()),
        st::TeamOperation::Join => format!("team join {} {}", change.team, players.join(" ")),
        st::TeamOperation::Leave => format!("team leave {}", players.join(" ")),
        st::TeamOperation::Modify => {
            let (property, value) = match (&change.property, &change.value) {
                (Some(p), Some(v)) => (p, v),
                _ => return Err("modify needs a property and a value".into()),
            };
            if !matches!(
                property.as_str(),
                "color" | "friendlyFire" | "seeFriendlyInvisibles" | "nametagVisibility" | "deathMessageVisibility" | "collisionRule"
            ) {
                return Err("modifiable team properties: color, friendlyFire, seeFriendlyInvisibles, nametagVisibility, deathMessageVisibility, collisionRule".into());
            }
            command_word(value, "value")?;
            format!("team modify {} {property} {value}", change.team)
        }
    })
}

fn particle_lines(request: &st::ParticleRequest) -> Result<Vec<String>, String> {
    registry_id(&request.particle)?;
    let at = vec3(&request.position)?;
    let spread = vec3(&request.spread)?;
    if !request.speed.is_finite() || request.speed < 0.0 || request.count > 1000 || spread.iter().any(|s| s.abs() > 64.0) {
        return Err("a particle has a finite speed, at most 1000 copies and a spread within 64".into());
    }
    if request.players.len() > MAX_LIST {
        return Err("too many players".into());
    }
    let base = format!(
        "particle {} {} {} {} {} {} {} {} {} normal",
        request.particle, at[0], at[1], at[2], spread[0], spread[1], spread[2], request.speed, request.count
    );
    if request.players.is_empty() {
        return Ok(vec![base]);
    }
    request.players.iter().map(|p| parse_uuid(p).map(|uuid| format!("{base} {}", uuid_text(uuid)))).collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn world_control_lines_only_hold_checked_words() {
        let change = |operation, key: &str, value: &str| st::WorldControlChange {
            operation,
            dimension: "minecraft:overworld".into(),
            key: key.into(),
            value: value.into(),
            from: Some(st::BlockPos { x: 0, y: 0, z: 0 }),
            to: None,
        };
        assert_eq!(world_control_line(&change(st::WorldControlOperation::SetTime, "", "day")).unwrap(), "time set day");
        assert!(world_control_line(&change(st::WorldControlOperation::SetTime, "", "day; stop")).is_err());
        assert!(world_control_line(&change(st::WorldControlOperation::SetGameRule, "doDaylightCycle", "false")).unwrap().starts_with("gamerule"));
        assert!(world_control_line(&change(st::WorldControlOperation::SetGameRule, "x y", "false")).is_err());
        assert!(world_control_line(&change(st::WorldControlOperation::SetGameRule, "keepInventory", "run stop")).is_err());
        let line = world_control_line(&change(st::WorldControlOperation::AddForceLoad, "", "")).unwrap();
        assert_eq!(line, "execute in minecraft:overworld run forceload add 0 0 0 0");
        let mut big = change(st::WorldControlOperation::AddForceLoad, "", "");
        big.to = Some(st::BlockPos { x: 1600, y: 0, z: 1600 });
        assert!(world_control_line(&big).is_err());
    }

    #[test]
    fn team_and_particle_lines_reject_injection() {
        let team = |operation, name: &str| st::TeamChange { operation, team: name.into(), players: Vec::new(), property: None, value: None };
        assert_eq!(team_line(&team(st::TeamOperation::Add, "red")).unwrap(), "team add red");
        assert!(team_line(&team(st::TeamOperation::Add, "red run stop")).is_err());
        let mut modify = team(st::TeamOperation::Modify, "red");
        modify.property = Some("color".into());
        modify.value = Some("red; stop".into());
        assert!(team_line(&modify).is_err());
        modify.property = Some("prefix".into());
        modify.value = Some("x".into());
        assert!(team_line(&modify).is_err());
        let request = st::ParticleRequest {
            particle: "minecraft:flame".into(),
            position: st::Vec3 { x: 1.0, y: 2.0, z: 3.0 },
            spread: st::Vec3 { x: 0.0, y: 0.0, z: 0.0 },
            speed: 0.1,
            count: 5,
            players: Vec::new(),
        };
        assert_eq!(particle_lines(&request).unwrap().len(), 1);
        let mut bad = request.clone();
        bad.count = 100_000;
        assert!(particle_lines(&bad).is_err());
        bad = request;
        bad.position.x = f64::NAN;
        assert!(particle_lines(&bad).is_err());
    }

    #[test]
    fn calls_are_bounded_and_refuse_unknown_targets() {
        assert!(call_mod("a", "b/../c", "m", b"").is_err());
        assert!(call_mod("a", "b", "", b"").is_err());
        assert!(call_mod("a", "b", "m", &vec![0u8; MAX_BLOB + 1]).is_err());
        assert!(call_mod("a", "a", "m", b"").is_err());
        assert!(call_mod("a", "no-such-package", "m", b"").unwrap_err().contains("no package"));
    }

    #[test]
    fn guest_numbers_must_be_finite() {
        assert!(vec3(&st::Vec3 { x: 1.0, y: f64::INFINITY, z: 0.0 }).is_err());
        assert!(registry_id("minecraft:stone").is_ok());
        assert!(registry_id("Stone;stop").is_err());
        assert!(sound_category("hostile").is_ok());
        assert!(sound_category("loud").is_err());
    }
}

// ---------------------------------------------------------------------------------------------
// Data attachments (W2-B): opaque bytes a package keeps on players, entities, block entities,
// chunks and the level. Persistence, sync to clients and copy-on-death are the engine's own
// (`Attachments`); the definition's policy decides which apply.
// ---------------------------------------------------------------------------------------------

/// Largest value of an attachment.
const MAX_ATTACHMENT_BYTES: usize = 64 << 10;
/// Most definitions one package may declare.
const MAX_ATTACHMENT_TYPES: usize = 256;

static ATTACHMENT_TYPES: Mutex<Option<horizon_world::attachments::DynamicAttachmentRegistry>> = Mutex::new(None);
static ATTACHMENT_COUNTS: Mutex<Vec<(String, usize)>> = Mutex::new(Vec::new());

#[cfg(test)]
pub(super) static ATTACHMENT_TEST_LOCK: Mutex<()> = Mutex::new(());

pub(crate) fn attachment_registry() -> horizon_world::attachments::DynamicAttachmentRegistry {
    ATTACHMENT_TYPES
        .lock()
        .unwrap_or_else(|error| error.into_inner())
        .get_or_insert_with(Default::default)
        .clone()
}

pub(crate) fn prepare_attachments(
    holder: horizon_world::attachments::Holder,
    map: &mut horizon_world::attachments::Attachments,
    key: String,
) -> Result<(), horizon_world::attachments::AttachmentError> {
    let registry = attachment_registry();
    registry.bind(map, key);
    registry.reconcile_map(holder, map)
}

pub(crate) fn reset_attachments() {
    let mut registry = ATTACHMENT_TYPES
        .lock()
        .unwrap_or_else(|error| error.into_inner());
    *registry = None;
    ATTACHMENT_COUNTS
        .lock()
        .unwrap_or_else(|error| error.into_inner())
        .clear();
}

fn attachment_scope(kind: &str) -> Result<horizon_world::attachments::AttachmentScope, String> {
    use horizon_world::attachments::AttachmentScope as S;
    Ok(match kind {
        "player" => S::Player,
        "entity" => S::Entity,
        "block-entity" => S::BlockEntity,
        "chunk" => S::Chunk,
        "level" => S::Level,
        _ => return Err("an attachment kind is player, entity, block-entity, chunk or level".into()),
    })
}

fn attachment_holder(kind: &str, holder: &str) -> Result<horizon_world::attachments::Holder, String> {
    use horizon_world::attachments::Holder;
    let numbers = |count: usize| -> Result<Vec<i32>, String> {
        let parts: Vec<i32> = holder.split(',').map(|p| p.trim().parse::<i32>()).collect::<Result<_, _>>().map_err(|_| "not a coordinate list".to_owned())?;
        if parts.len() == count { Ok(parts) } else { Err(format!("a {kind} holder is {count} comma-separated numbers")) }
    };
    Ok(match kind {
        "player" => Holder::Player(parse_uuid(holder)?),
        "entity" => Holder::Entity(holder.trim().parse::<u32>().map_err(|_| "an entity holder is its decimal id".to_owned())?),
        "block-entity" => {
            let n = numbers(3)?;
            Holder::BlockEntity([n[0], n[1], n[2]])
        }
        "chunk" => {
            let n = numbers(2)?;
            Holder::Chunk([n[0], n[1]])
        }
        "level" => Holder::Level,
        _ => return Err("an attachment kind is player, entity, block-entity, chunk or level".into()),
    })
}

fn attachment_error(error: horizon_world::attachments::AttachmentError) -> String {
    use horizon_world::attachments::AttachmentError;
    match error {
        AttachmentError::ConflictingType => {
            "attachment: another definition of this id (native or from another package) declares a different copy_on_death".to_owned()
        }
        other => format!("attachment: {other:?}"),
    }
}

fn attachment_type(owner: &str, id: &str) -> Result<horizon_world::attachments::DynamicAttachmentType, String> {
    let registry = ATTACHMENT_TYPES.lock().map_err(|_| "attachment registry poisoned".to_owned())?;
    registry
        .as_ref()
        .and_then(|r| r.get(owner, id))
        .cloned()
        .ok_or_else(|| format!("attachment {id:.64} is not defined by this package (call attachment-define first)"))
}

/// Registers a definition for `owner`; the same definition twice is accepted (both sides define it).
pub(super) fn define_attachment(
    owner: &str,
    id: &str,
    kind: &str,
    persistent: bool,
    synced: bool,
    copy_on_death: bool,
) -> Result<(), String> {
    use horizon_world::attachments::{AttachmentError, DynamicAttachmentPolicy, DynamicAttachmentRegistry, MAX_ATTACHMENT_ID_BYTES};
    if id.len() > MAX_ATTACHMENT_ID_BYTES {
        return Err("an attachment id is at most 128 bytes".into());
    }
    let policy = DynamicAttachmentPolicy { scope: attachment_scope(kind)?, persistent, synced, copy_on_death };
    let mut registry = ATTACHMENT_TYPES.lock().map_err(|_| "attachment registry poisoned".to_owned())?;
    let registry = registry.get_or_insert_with(DynamicAttachmentRegistry::default);
    if let Some(existing) = registry.get(owner, id) {
        return if existing.policy() == policy { Ok(()) } else { Err("attachment already defined with another policy".into()) };
    }
    let mut counts = ATTACHMENT_COUNTS.lock().map_err(|_| "attachment registry poisoned".to_owned())?;
    let slot = match counts.iter().position(|(o, _)| o == owner) {
        Some(i) => i,
        None => {
            counts.push((owner.to_owned(), 0));
            counts.len() - 1
        }
    };
    if counts[slot].1 >= MAX_ATTACHMENT_TYPES {
        return Err("a package defines at most 256 attachments".into());
    }
    registry.define(owner, id, policy).map_err(|e| match e {
        AttachmentError::WrongOwner => "an attachment id starts with the package id and a colon".to_owned(),
        other => attachment_error(other),
    })?;
    counts[slot].1 += 1;
    Ok(())
}

fn attachment_compound(data: &[u8]) -> Result<horizon_world::nbt::Compound, String> {
    use horizon_world::nbt::{Compound, Tag};
    if data.len() > MAX_ATTACHMENT_BYTES {
        return Err("an attachment value is at most 64 KiB".into());
    }
    let mut compound = Compound::new();
    compound.insert("b".into(), Tag::ByteArray(data.to_vec()));
    Ok(compound)
}

fn attachment_bytes(compound: &horizon_world::nbt::Compound) -> Option<Vec<u8>> {
    match compound.get(&"b".into()) {
        Some(horizon_world::nbt::Tag::ByteArray(bytes)) => Some(bytes.clone()),
        _ => None,
    }
}

impl GameServerHost {
    fn attachment_define_impl(&mut self, id: &str, kind: &str, persistent: bool, synced: bool, copy_on_death: bool) -> Result<(), String> {
        if !super::full::initializing_server(&self.id) {
            return Err("attachment types may only be defined during server initialization".into());
        }
        define_attachment(&self.id, id, kind, persistent, synced, copy_on_death)
    }

    fn attachment_get_impl(&mut self, id: &str, kind: &str, holder: &str) -> Result<Option<Vec<u8>>, String> {
        let ty = attachment_type(&self.id, id)?;
        let holder = attachment_holder(kind, holder)?;
        read_service(|host| {
            let attachments = host.attachment_read(holder).map_err(attachment_error)?;
            ty.read(holder, attachments).map_err(attachment_error)
        })
        .map(|v| v.and_then(|c| attachment_bytes(&c)))
    }

    fn attachment_set_impl(&mut self, id: &str, kind: &str, holder: &str, data: &[u8]) -> Result<(), String> {
        let ty = attachment_type(&self.id, id)?;
        let holder = attachment_holder(kind, holder)?;
        let value = attachment_compound(data)?;
        server_service(|host| ty.set(host, holder, value).map_err(attachment_error))
    }

    fn attachment_remove_impl(&mut self, id: &str, kind: &str, holder: &str) -> Result<bool, String> {
        let ty = attachment_type(&self.id, id)?;
        let holder = attachment_holder(kind, holder)?;
        server_service(|host| ty.remove(host, holder).map_err(attachment_error)).map(|v| v.is_some())
    }
}

#[cfg(test)]
mod attachment_tests {
    use super::*;
    use horizon_world::attachments::{AttachmentError, AttachmentHost, Attachments, Holder};

    struct One(Attachments);
    impl AttachmentHost for One {
        fn attachments(&mut self, _: Holder) -> Result<&mut Attachments, AttachmentError> {
            Ok(&mut self.0)
        }
    }

    #[test]
    fn holders_parse_and_bad_ones_are_refused() {
        assert_eq!(attachment_holder("entity", "42").unwrap(), Holder::Entity(42));
        assert_eq!(attachment_holder("block-entity", "1, -2,3").unwrap(), Holder::BlockEntity([1, -2, 3]));
        assert_eq!(attachment_holder("chunk", "4,5").unwrap(), Holder::Chunk([4, 5]));
        assert_eq!(attachment_holder("level", "").unwrap(), Holder::Level);
        assert!(attachment_holder("entity", "x").is_err());
        assert!(attachment_holder("chunk", "1,2,3").is_err());
        assert!(attachment_holder("planet", "").is_err());
    }

    #[test]
    fn a_policy_conflict_with_a_static_type_is_an_error_not_a_poisoned_lock() {
        use horizon_world::attachments::{AttachmentCodec, AttachmentType, register_static};
        let _guard = ATTACHMENT_TEST_LOCK.lock().unwrap();
        reset_attachments();
        let native = AttachmentType::<i32> {
            namespace: "wcf1",
            name: "coins",
            default: || 0,
            codec: Some(AttachmentCodec {
                encode: |n| Ok([("n".into(), horizon_world::nbt::Tag::Int(*n))].into()),
                decode: |_| Ok(0),
            }),
            copy_on_death: true,
            sync: false,
        };
        register_static(&native).unwrap();
        let error = define_attachment("wcf1", "wcf1:coins", "player", true, false, false).unwrap_err();
        assert!(error.contains("different copy_on_death"), "{error}");
        // Neither lock is poisoned: this and other packages still define types.
        define_attachment("wcf1", "wcf1:other", "player", true, false, false).unwrap();
        define_attachment("wcf2", "wcf2:x", "player", true, false, true).unwrap();
        define_attachment("wcf1", "wcf1:coins", "player", true, false, true).unwrap();
        reset_attachments();
    }

    #[test]
    fn respawn_copy_through_the_registry_judges_unread_values() {
        let _guard = ATTACHMENT_TEST_LOCK.lock().unwrap();
        reset_attachments();
        define_attachment("wcf3", "wcf3:kept", "player", true, false, true).unwrap();
        define_attachment("wcf3", "wcf3:dropped", "player", true, false, false).unwrap();
        let (kept, dropped) = (attachment_type("wcf3", "wcf3:kept").unwrap(), attachment_type("wcf3", "wcf3:dropped").unwrap());
        let holder = Holder::Player(9);
        let mut host = One(Attachments::default());
        kept.set(&mut host, holder, attachment_compound(&[1]).unwrap()).unwrap();
        dropped.set(&mut host, holder, attachment_compound(&[2]).unwrap()).unwrap();
        // Saved and loaded again: nothing has read either value.
        let mut loaded = Attachments::from_nbt(host.0.to_nbt());
        let mut copy = One(attachment_registry().copy_on_death(holder, &mut loaded).unwrap());
        assert!(kept.get(&mut copy, holder).unwrap().is_some());
        assert!(dropped.get(&mut copy, holder).unwrap().is_none());
        reset_attachments();
    }

    #[test]
    fn definitions_are_owned_idempotent_and_policy_checked() {
        let _guard = ATTACHMENT_TEST_LOCK.lock().unwrap();
        assert!(define_attachment("wa1", "wa1:coins", "player", true, true, true).is_ok());
        assert!(define_attachment("wa1", "wa1:coins", "player", true, true, true).is_ok());
        assert!(define_attachment("wa1", "wa1:coins", "player", false, true, true).is_err());
        assert!(define_attachment("wa1", "other:coins", "player", true, false, false).is_err());
        assert!(attachment_type("wa2", "wa1:coins").is_err());
        assert!(
            define_attachment(
                "wa1",
                &format!("wa1:{}", "x".repeat(125)),
                "player",
                true,
                false,
                false
            )
            .is_err()
        );
    }

    #[test]
    fn values_survive_save_and_load_and_respawn_copy() {
        let _guard = ATTACHMENT_TEST_LOCK.lock().unwrap();
        define_attachment("wa3", "wa3:kept", "player", true, false, true).unwrap();
        define_attachment("wa3", "wa3:temp", "player", false, false, false).unwrap();
        let (kept, temp) = (attachment_type("wa3", "wa3:kept").unwrap(), attachment_type("wa3", "wa3:temp").unwrap());
        let mut host = One(Attachments::default());
        let holder = Holder::Player(7);
        kept.set(&mut host, holder, attachment_compound(&[1, 2, 3]).unwrap()).unwrap();
        temp.set(&mut host, holder, attachment_compound(&[9]).unwrap()).unwrap();
        // save, then load into a fresh holder
        let mut loaded = One(Attachments::from_nbt(host.0.to_nbt()));
        let value = kept.get(&mut loaded, holder).unwrap().unwrap();
        assert_eq!(attachment_bytes(&value), Some(vec![1, 2, 3]));
        assert!(temp.get(&mut loaded, holder).unwrap().is_none(), "non-persistent data is not saved");
        // copy on death keeps only opted-in values
        let mut copy = One(host.0.copy_on_death().unwrap());
        assert!(kept.get(&mut copy, holder).unwrap().is_some());
        assert!(temp.get(&mut copy, holder).unwrap().is_none());
        assert!(attachment_compound(&vec![0u8; MAX_ATTACHMENT_BYTES + 1]).is_err());
        assert!(kept.remove(&mut loaded, holder).unwrap().is_some());
    }

    #[test]
    fn synced_changes_are_queued_for_clients() {
        let _guard = ATTACHMENT_TEST_LOCK.lock().unwrap();
        define_attachment("wa4", "wa4:shown", "entity", true, true, false).unwrap();
        let shown = attachment_type("wa4", "wa4:shown").unwrap();
        let mut host = One(Attachments::default());
        shown.set(&mut host, Holder::Entity(1), attachment_compound(&[5]).unwrap()).unwrap();
        let changes = host.0.take_sync_changes();
        assert_eq!(changes.len(), 1);
        assert!(changes[0].value.is_some());
        let mut client = Attachments::default();
        client.apply_sync(changes.into_iter().next().unwrap());
        let mut client = One(client);
        assert_eq!(attachment_bytes(&shown.get(&mut client, Holder::Entity(1)).unwrap().unwrap()), Some(vec![5]));
    }

    #[test]
    fn changing_worlds_clears_definitions_and_package_counts() {
        let _guard = ATTACHMENT_TEST_LOCK.lock().unwrap();
        reset_attachments();
        for index in 0..MAX_ATTACHMENT_TYPES {
            define_attachment(
                "reset",
                &format!("reset:data{index}"),
                "player",
                true,
                true,
                true,
            )
            .unwrap();
        }
        assert!(define_attachment("reset", "reset:extra", "player", true, true, true).is_err());
        reset_attachments();
        assert!(attachment_type("reset", "reset:data0").is_err());
        assert!(ATTACHMENT_COUNTS.lock().unwrap().is_empty());
        for index in 0..MAX_ATTACHMENT_TYPES {
            define_attachment(
                "reset",
                &format!("reset:data{index}"),
                "player",
                false,
                false,
                false,
            )
            .unwrap();
        }
        let policy = attachment_type("reset", "reset:data0").unwrap().policy();
        assert!(!policy.persistent && !policy.synced && !policy.copy_on_death);
        assert!(define_attachment("reset", "reset:extra", "player", false, false, false).is_err());
        reset_attachments();
    }
}
