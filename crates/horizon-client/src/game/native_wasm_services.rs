//! Server helpers the WebAssembly services of `server-full` need and the native API had none of
//! (fragment `wasm_services`): a player breaking a block with their tool, sneaking, tags and tool
//! requirements, attribute reads, dropped items, advancements, vitals, exact stack giving and
//! clearing, and the player's own 36 slots. Native mods may use them too. Everything acts where
//! the world is simulated, on a server tick, and does nothing unless a mod calls it. Each helper
//! is limited to 1024 calls per mod and server tick.
use std::{cell::RefCell, collections::HashMap};

use horizon_client::{
    items::ItemStack,
    mod_api::{NativeTickId, NativeWorldEpoch},
};
use horizon_core::position::BlockPos;
use horizon_world::attribute_map::ValueKind;

use super::{Game, native_entity_query::require_server_context};

const MAX_CALLS: u16 = 1024;
/// Most entries of a dropped-item list.
const MAX_ITEM_ENTITIES: usize = 256;
/// Farthest a radius query reaches.
const MAX_RADIUS: f64 = 64.0;
/// Slots of a player's main inventory (hotbar first).
pub(crate) const MAIN_SLOTS: usize = 36;
const ARMOR_SLOT_START: u32 = 36;
const OFFHAND_SLOT: u32 = 40;
const CURSOR_SLOT: u32 = 41;

fn player_inventory_slots(
    inventory: &[Option<ItemStack>],
    armor: &[Option<ItemStack>; 4],
    offhand: &Option<ItemStack>,
    carried: &Option<ItemStack>,
) -> Vec<Option<ItemStack>> {
    let mut slots = inventory.to_vec();
    slots.resize(MAIN_SLOTS, None);
    slots.truncate(MAIN_SLOTS);
    slots.extend(armor.iter().cloned());
    slots.push(offhand.clone());
    slots.push(carried.clone());
    slots
}

fn clear_exact_inventory_slot(
    inventory: &mut Vec<Option<ItemStack>>,
    armor: &mut [Option<ItemStack>; 4],
    offhand: &mut Option<ItemStack>,
    carried: &mut Option<ItemStack>,
    slot: u32,
    expected: &ItemStack,
) -> bool {
    let current = match slot {
        0..=35 => inventory.get_mut(slot as usize),
        ARMOR_SLOT_START..=39 => armor.get_mut((slot - ARMOR_SLOT_START) as usize),
        OFFHAND_SLOT => Some(offhand),
        CURSOR_SLOT => Some(carried),
        _ => None,
    };
    let Some(current) = current else {
        return false;
    };
    if current.as_ref() == Some(expected) {
        *current = None;
        true
    } else {
        false
    }
}

type Tick = (NativeWorldEpoch, NativeTickId);

#[derive(Default)]
struct Budget {
    tick: Option<Tick>,
    calls: HashMap<(usize, &'static str), u16>,
}

thread_local! {
    static BUDGET: RefCell<Budget> = RefCell::new(Budget::default());
}

fn spend(tick: Tick, mod_index: usize, operation: &'static str) -> Result<(), String> {
    BUDGET.with(|budget| {
        let mut budget = budget.borrow_mut();
        if budget.tick != Some(tick) {
            budget.tick = Some(tick);
            budget.calls.clear();
        }
        let calls = budget.calls.entry((mod_index, operation)).or_default();
        if *calls >= MAX_CALLS {
            return Err(format!(
                "native {operation} limit exceeded (1024 calls per helper, mod and server tick)"
            ));
        }
        *calls += 1;
        Ok(())
    })
}

/// Whether `a` and `b` are the same item with the same components (counts apart).
pub(crate) fn same_kind(a: &ItemStack, b: &ItemStack) -> bool {
    a.item == b.item && a.damage == b.damage && a.extra == b.extra
}

/// Item totals of some slots, one entry per kind (item and components).
pub(crate) fn totals(slots: &[Option<ItemStack>]) -> Vec<(ItemStack, u64)> {
    let mut out: Vec<(ItemStack, u64)> = Vec::new();
    for stack in slots.iter().flatten().filter(|s| s.count > 0) {
        match out.iter_mut().find(|(kind, _)| same_kind(kind, stack)) {
            Some((_, total)) => *total += u64::from(stack.count),
            None => out.push((stack.clone(), u64::from(stack.count))),
        }
    }
    out
}

/// Whether `new` holds exactly the items `old` held (nothing made, nothing lost), in any
/// arrangement.
pub(crate) fn conserved(old: &[Option<ItemStack>], new: &[Option<ItemStack>]) -> bool {
    let (old, new) = (totals(old), totals(new));
    old.len() == new.len()
        && old
            .iter()
            .all(|(kind, total)| new.iter().any(|(k, t)| same_kind(k, kind) && t == total))
}

/// How many of `stack`'s kind fit into `slots` (each stack up to `max`).
pub(crate) fn room(slots: &[Option<ItemStack>], stack: &ItemStack, max: u32) -> u64 {
    slots
        .iter()
        .map(|slot| match slot {
            None => u64::from(max),
            Some(have) if same_kind(have, stack) => u64::from(max.saturating_sub(have.count)),
            Some(_) => 0,
        })
        .sum()
}

/// Removes exactly `count` items matching `pattern` (any components when `exact` is false) from
/// `slots`, lowest slots first. All or nothing: false with `slots` untouched when fewer match.
pub(crate) fn clear_matching(
    slots: &mut [Option<ItemStack>],
    pattern: &ItemStack,
    exact: bool,
    count: u32,
) -> bool {
    let matches = |s: &ItemStack| {
        if exact {
            same_kind(s, pattern)
        } else {
            s.item == pattern.item
        }
    };
    let have: u64 = slots
        .iter()
        .flatten()
        .filter(|s| matches(s))
        .map(|s| u64::from(s.count))
        .sum();
    if count == 0 || have < u64::from(count) {
        return false;
    }
    let mut left = count;
    for slot in slots.iter_mut() {
        let Some(stack) = slot else { continue };
        if left == 0 || !matches(stack) {
            continue;
        }
        let take = stack.count.min(left);
        stack.count -= take;
        left -= take;
        if stack.count == 0 {
            *slot = None;
        }
    }
    true
}

/// Puts `stack` into `slots` as `Inventory.add` would (onto equal stacks, then empty slots) only
/// when all of it fits; false with `slots` untouched otherwise.
pub(crate) fn add_all(slots: &mut [Option<ItemStack>], stack: &ItemStack, max: u32) -> bool {
    if stack.count == 0 || u64::from(stack.count) > room(slots, stack, max) {
        return false;
    }
    let mut left = stack.count;
    for slot in slots.iter_mut() {
        if left == 0 {
            break;
        }
        if let Some(have) = slot
            && same_kind(have, stack)
        {
            let add = max.saturating_sub(have.count).min(left);
            have.count += add;
            left -= add;
        }
    }
    for slot in slots.iter_mut() {
        if left == 0 {
            break;
        }
        if slot.is_none() {
            let add = max.min(left);
            *slot = Some(ItemStack {
                count: add,
                ..stack.clone()
            });
            left -= add;
        }
    }
    true
}

// Native mods use these; nothing in Horizon does.
#[allow(dead_code)]
impl Game<'_> {
    /// The mod calling now and the server tick it runs in, for a helper that acts on the world.
    fn wasm_service(&self, operation: &'static str) -> Result<(), String> {
        require_server_context(
            self.options.execution_role,
            self.remote.is_some(),
            false,
            self.lan.is_some() || self.lan_acting.is_some(),
        )?;
        let caller = horizon_client::crash::native_mod_call()
            .ok_or("this helper requires an active native mod call")?;
        let tick = self
            .native_lifecycle
            .entity_query_tick()
            .ok_or("this helper requires an active server tick")?;
        spend(tick, caller.index, operation)
    }

    /// Breaks the block at `pos` as player `uuid` does with the tool in their hand: the game's
    /// own player break (drops with Fortune and Silk Touch, tool damage, experience, statistics,
    /// the mods' destroy hooks), in their level `dimension`. Ok(false) when the game refused (a
    /// protected spawn, adventure mode, a canceled break) or nothing was there; Err for a player
    /// who is not on the server, another dimension or an unloaded position.
    pub(super) fn native_break_block_as(
        &mut self,
        dimension: &str,
        uuid: u128,
        pos: [i32; 3],
    ) -> Result<bool, String> {
        self.wasm_service("break_block_as")?;
        let (target, conn, theirs) = self.native_player_target(uuid)?;
        if theirs.id() != dimension {
            return Err(format!("the player is in {}, not {dimension}", theirs.id()));
        }
        self.for_native_player(target, theirs, |game| {
            let at = BlockPos::new(pos[0], pos[1], pos[2]);
            if pos[1] < game.world.bottom_y() || pos[1] >= game.world.top_y() {
                return Err("the position is outside the world".to_owned());
            }
            if !game.world.loaded(at.x, at.z) {
                return Err("the position is not loaded".to_owned());
            }
            let before = game.world.block(pos[0], pos[1], pos[2]);
            if game.res.facts().get(before).name.as_ref() == "minecraft:air" {
                return Ok(false);
            }
            game.break_block(pos).map_err(|error| error.to_string())?;
            game.native_player_ops.inventory_dirty.insert(conn);
            Ok(game.world.block(pos[0], pos[1], pos[2]) != before)
        })?
    }

    /// The stack in the main hand of the acting player, whole (components included).
    pub(super) fn native_held_stack(&self) -> Option<ItemStack> {
        self.inventory.get(self.selected)?.clone()
    }

    /// Whether the player is sneaking, from their server-side movement state or guest record.
    pub(super) fn native_player_sneaking(&self, uuid: u128) -> Result<bool, String> {
        let host = self.lan.as_ref();
        if let Some(guest) = host
            .into_iter()
            .flat_map(|host| host.hub.guests.iter())
            .find(|guest| guest.placed && guest.uuid == uuid)
        {
            return Ok(guest.sneaking);
        }
        if self.lan_acting.is_none() && self.lan_acting_uuid == Some(uuid) {
            if let Some(player) = self.player.as_ref() {
                return Ok(player.shift_key_down);
            }
        }
        Err("the player is not on the server".into())
    }

    /// The tags (without `#`) a block belongs to, sorted.
    pub(super) fn native_block_tags(&self, block: &str) -> Vec<String> {
        let mut tags: Vec<String> = self
            .res
            .items()
            .block_tags
            .iter()
            .filter(|(_, members)| members.contains(block))
            .map(|(tag, _)| tag.clone())
            .collect();
        tags.sort();
        tags
    }

    /// The tags (without `#`) an item belongs to, sorted.
    pub(super) fn native_item_tags(&self, item: &str) -> Vec<String> {
        let mut tags: Vec<String> = self
            .res
            .items()
            .item_tags
            .iter()
            .filter(|(_, members)| members.contains(item))
            .map(|(tag, _)| tag.clone())
            .collect();
        tags.sort();
        tags
    }

    /// What mining a block takes, from its default state: `(hardness, requires_tool, mineable
    /// tags such as minecraft:mineable/pickaxe, the tier tag minecraft:needs_<tier>_tool)`.
    /// None for an unknown block.
    pub(super) fn native_block_tool_requirement(
        &self,
        block: &str,
    ) -> Option<(f32, bool, Vec<String>, Option<String>)> {
        let state = self.pack.assets.registry.default_state(block)?;
        let facts = self.res.facts().get(state);
        let tags = self.native_block_tags(block);
        let mineable = tags
            .iter()
            .filter(|tag| tag.starts_with("minecraft:mineable/"))
            .cloned()
            .collect();
        let tier = tags
            .iter()
            .find(|tag| tag.starts_with("minecraft:needs_") && tag.ends_with("_tool"))
            .cloned();
        Some((facts.hardness, facts.requires_tool, mineable, tier))
    }

    /// Whether `tool` (none for a bare hand) harvests the drops of `block` (`Tool.isCorrectForDrops`
    /// when the block needs a tool). None for an unknown block.
    pub(super) fn native_can_harvest(&self, block: &str, tool: Option<&ItemStack>) -> Option<bool> {
        let state = self.pack.assets.registry.default_state(block)?;
        let facts = self.res.facts().get(state);
        Some(!facts.requires_tool || self.res.items().correct_for_drops(tool, block))
    }

    /// The base and the modified value of a player's attribute (`minecraft:attack_damage` or
    /// `attack_damage`).
    pub(super) fn native_attribute_get(
        &mut self,
        uuid: u128,
        attribute: &str,
    ) -> Result<(f64, f64), String> {
        self.wasm_service("attribute_get")?;
        let attribute = super::native_player_ops::attribute_name(attribute);
        let (target, _, dimension) = self.native_player_target(uuid)?;
        self.for_native_player(target, dimension, |game| {
            let player = game.player.as_mut().ok_or("the player is not here")?;
            let key = player
                .attributes
                .key(attribute.as_ref())
                .ok_or("unknown player attribute")?;
            if player
                .attributes
                .get_instance(Some(key.clone()))
                .map_err(|error| format!("{error:?}"))?
                .is_none()
            {
                return Err("attribute is not available on this player".to_owned());
            }
            let base = player
                .attributes
                .value(Some(key.clone()), ValueKind::Base)
                .map_err(|error| format!("{error:?}"))?;
            let value = player
                .attributes
                .value(Some(key), ValueKind::Computed)
                .map_err(|error| format!("{error:?}"))?;
            Ok((base, value))
        })?
    }

    /// Dropped items within `radius` of `center` in the level in place: `(entity id, position,
    /// velocity, stack)`, at most 256, nearest first. The radius is at most 64.
    pub(super) fn native_item_entities_in(
        &self,
        dimension: &str,
        center: [f64; 3],
        radius: f64,
    ) -> Result<Vec<(u32, [f64; 3], [f64; 3], ItemStack)>, String> {
        if !self.native_level_open(dimension) {
            return Err(format!("{dimension} is not the level being ticked"));
        }
        if !center.iter().all(|c| c.is_finite()) || !radius.is_finite() || radius < 0.0 {
            return Err("the centre and the radius must be finite".into());
        }
        let radius = radius.min(MAX_RADIUS);
        let mut found: Vec<(f64, (u32, [f64; 3], [f64; 3], ItemStack))> = self
            .dropped
            .iter()
            .filter_map(|item| {
                let at = [item.position.x, item.position.y, item.position.z];
                let distance = (0..3).map(|i| (at[i] - center[i]).powi(2)).sum::<f64>().sqrt();
                (distance <= radius).then(|| {
                    (
                        distance,
                        (
                            item.id,
                            at,
                            [item.velocity.x, item.velocity.y, item.velocity.z],
                            item.stack.clone(),
                        ),
                    )
                })
            })
            .collect();
        found.sort_by(|a, b| a.0.total_cmp(&b.0));
        found.truncate(MAX_ITEM_ENTITIES);
        Ok(found.into_iter().map(|(_, entry)| entry).collect())
    }

    /// Moves dropped item `id` to `position` with `velocity` (a magnet). False for no such item.
    pub(super) fn native_item_entity_move(
        &mut self,
        dimension: &str,
        id: u32,
        position: [f64; 3],
        velocity: [f64; 3],
    ) -> Result<bool, String> {
        self.wasm_service("item_entity_move")?;
        if !self.native_level_open(dimension) {
            return Err(format!("{dimension} is not the level being ticked"));
        }
        if !position.iter().chain(&velocity).all(|c| c.is_finite() && c.abs() < 3.0e7) {
            return Err("the position and the velocity must be finite and within the world".into());
        }
        let Some(item) = self.dropped.iter_mut().find(|item| item.id == id) else {
            return Ok(false);
        };
        item.position = horizon_core::geometry::Vec3::new(position[0], position[1], position[2]);
        item.velocity = horizon_core::geometry::Vec3::new(velocity[0], velocity[1], velocity[2]);
        item.on_ground = false;
        item.needs_sync = true;
        Ok(true)
    }

    /// The player picks dropped item `id` up now: what fits goes into their inventory, the rest
    /// stays dropped. True when anything was taken.
    pub(super) fn native_item_entity_pickup(
        &mut self,
        dimension: &str,
        id: u32,
        uuid: u128,
    ) -> Result<bool, String> {
        self.wasm_service("item_entity_pickup")?;
        if !self.native_level_open(dimension) {
            return Err(format!("{dimension} is not the level being ticked"));
        }
        let Some(index) = self.dropped.iter().position(|item| item.id == id) else {
            return Ok(false);
        };
        let stack = self.dropped[index].stack.clone();
        let count = stack.count;
        let rest = self.native_give_stack(uuid, stack);
        let taken = rest.as_ref().map_or(count, |rest| count - rest.count);
        match rest {
            None => {
                self.remove_dropped(index);
            }
            Some(rest) => self.dropped[index].stack = rest,
        }
        Ok(taken > 0)
    }

    /// An advancement's state for a player: `(known, done, completed criteria, remaining
    /// criteria)`.
    pub(super) fn native_advancement_progress(
        &mut self,
        uuid: u128,
        advancement: &str,
    ) -> Result<(bool, bool, Vec<String>, Vec<String>), String> {
        self.wasm_service("advancement_progress")?;
        let id = super::native_player_ops::canonical_identifier(advancement)
            .ok_or("not an advancement id")?;
        let (target, _, dimension) = self.native_player_target(uuid)?;
        self.for_native_player(target, dimension, |game| {
            let tree = game.advancement_tree();
            let Some(&index) = tree.by_id.get(&id) else {
                return (false, false, Vec::new(), Vec::new());
            };
            let a = &tree.list[index];
            let (mut done, mut left) = (Vec::new(), Vec::new());
            for criterion in &a.criteria {
                if game.adv.progress.has(&a.id, &criterion.name) {
                    done.push(criterion.name.clone());
                } else {
                    left.push(criterion.name.clone());
                }
            }
            (true, game.adv.progress.done(a), done, left)
        })
    }

    /// Completes every remaining criterion of an advancement for a player (the rewards and the
    /// toast follow as for `/advancement grant`). False when it was done already.
    pub(super) fn native_advancement_grant(
        &mut self,
        uuid: u128,
        advancement: &str,
    ) -> Result<bool, String> {
        self.wasm_service("advancement_grant")?;
        let id = super::native_player_ops::canonical_identifier(advancement)
            .ok_or("not an advancement id")?;
        let (target, _, dimension) = self.native_player_target(uuid)?;
        self.for_native_player(target, dimension, |game| {
            let tree = game.advancement_tree();
            let index = *tree.by_id.get(&id).ok_or("no such advancement")?;
            if game.adv.progress.done(&tree.list[index]) {
                return Ok(false);
            }
            Ok(game.grant_advancement(index, None) && game.adv.progress.done(&tree.list[index]))
        })?
    }

    /// An entity's `(health, max health, armor, absorption)`: a player's, or a mob's in the level
    /// in place (`entity` is the id events carry).
    pub(super) fn native_entity_vitals(
        &mut self,
        dimension: &str,
        id: u32,
    ) -> Result<(f32, f32, f32, f32), String> {
        use horizon_client::mobs::finalize::{Attr, max_health, value};
        self.wasm_service("entity_vitals")?;
        let guest = self
            .lan
            .iter()
            .flat_map(|host| host.hub.guests.iter())
            .find(|guest| guest.placed && i64::from(guest.entity_id) == i64::from(id))
            .map(|guest| guest.uuid);
        if let Some(uuid) = guest {
            let (target, _, theirs) = self.native_player_target(uuid)?;
            return self.for_native_player(target, theirs, |game| {
                let player = game.player.as_mut().ok_or("the player is not here")?;
                let mut read = |name: &str| -> f64 {
                    player
                        .attributes
                        .key(name)
                        .and_then(|key| player.attributes.value(Some(key), ValueKind::Computed).ok())
                        .unwrap_or(0.0)
                };
                let (max, armor) = (read("minecraft:max_health"), read("minecraft:armor"));
                Ok((player.health.health(), max as f32, armor as f32, 0.0))
            })?;
        }
        if !self.native_level_open(dimension) {
            return Err(format!("{dimension} is not the level being ticked"));
        }
        let mob = self
            .mobs
            .iter()
            .find(|m| m.id == id && !m.removed)
            .ok_or("no such living entity")?;
        Ok((mob.health, max_health(mob), value(mob, Attr::Armor) as f32, 0.0))
    }

    /// The player's 36 main slots, hotbar first.
    pub(super) fn native_player_main_slots(
        &mut self,
        uuid: u128,
    ) -> Result<Vec<Option<ItemStack>>, String> {
        let (target, _, dimension) = self.native_player_target(uuid)?;
        self.for_native_player(target, dimension, |game| {
            let mut slots = game.inventory.clone();
            slots.resize(MAIN_SLOTS, None);
            slots.truncate(MAIN_SLOTS);
            slots
        })
    }

    /// The main inventory, armor (head to feet), off hand and cursor, addressed by slot ids
    /// 0..35, 36..39, 40 and 41. The active guest is read from the swapped-in player state.
    pub(super) fn native_player_inventory_slots(
        &self,
        uuid: u128,
    ) -> Option<Vec<Option<ItemStack>>> {
        if self.lan_acting_uuid == Some(uuid)
            && self.player.is_some()
        {
            return Some(player_inventory_slots(
                &self.inventory,
                &self.armor,
                &self.offhand,
                &self.carried,
            ));
        }
        let host = self.lan.as_ref()?;
        let guest = host
            .hub
            .guests
            .iter()
            .find(|guest| guest.placed && guest.uuid == uuid)?;
        (!guest.dead && self.lan_acting != Some(guest.conn)).then(|| {
            player_inventory_slots(
                &guest.inventory,
                &guest.armor,
                &guest.offhand,
                &guest.carried,
            )
        })
    }

    /// Clears one player slot only if its complete stack still equals `expected`.
    /// Main slots are 0..35, armor is 36..39 (head to feet), off hand is 40 and cursor is 41.
    pub(super) fn native_player_remove_slot(
        &mut self,
        uuid: u128,
        slot: u32,
        expected: ItemStack,
    ) -> Result<bool, String> {
        self.wasm_service("player_remove_slot")?;
        if slot > CURSOR_SLOT {
            return Err("a player slot id is 0 through 41".into());
        }
        let (target, conn, dimension) = self.native_player_target(uuid)?;
        self.for_native_player(target, dimension, |game| {
            let removed = clear_exact_inventory_slot(
                &mut game.inventory,
                &mut game.armor,
                &mut game.offhand,
                &mut game.carried,
                slot,
                &expected,
            );
            if removed {
                game.native_player_ops.inventory_dirty.insert(conn);
                if slot == game.selected as u32 || (ARMOR_SLOT_START..=OFFHAND_SLOT).contains(&slot) {
                    game.apply_equipment();
                }
            }
            removed
        })
    }

    /// Replaces the player's 36 main slots (hotbar first) with `slots`, which must hold exactly
    /// the items the slots hold now (in any arrangement) with no stack past its item's limit.
    pub(super) fn native_player_set_slots(
        &mut self,
        uuid: u128,
        slots: Vec<Option<ItemStack>>,
    ) -> Result<(), String> {
        self.wasm_service("player_set_slots")?;
        if slots.len() != MAIN_SLOTS {
            return Err(format!("a player has {MAIN_SLOTS} main slots, got {}", slots.len()));
        }
        let (target, conn, dimension) = self.native_player_target(uuid)?;
        let items = self.res.items();
        for stack in slots.iter().flatten() {
            let max = items.get(&stack.item).map(|info| info.max_stack.max(1));
            if stack.count == 0 || max.is_none_or(|max| stack.count > max) {
                return Err(format!("{} x{} is not a valid stack", stack.item, stack.count));
            }
        }
        self.for_native_player(target, dimension, |game| {
            let mut old = game.inventory.clone();
            old.resize(MAIN_SLOTS, None);
            old.truncate(MAIN_SLOTS);
            if !conserved(&old, &slots) {
                return Err("the new slots do not hold exactly the same items".to_owned());
            }
            for (slot, stack) in game.inventory.iter_mut().zip(slots) {
                *slot = stack;
            }
            game.native_player_ops.inventory_dirty.insert(conn);
            Ok(())
        })?
    }

    /// Gives or clears exactly `stack.count` of `stack` for a player, all or nothing. A give needs
    /// room for all of it (main slots, as `Inventory.add` places it); a clear takes from the main
    /// slots and the off hand, matching the item and, when `exact`, its components too (damage,
    /// name, enchantments...). Ok(false) with nothing changed when it does not all fit or are not
    /// all there.
    pub(super) fn native_inventory_apply(
        &mut self,
        uuid: u128,
        give: bool,
        stack: ItemStack,
        exact: bool,
    ) -> Result<bool, String> {
        self.wasm_service("inventory_apply")?;
        let max = self
            .res
            .items()
            .get(&stack.item)
            .map(|info| info.max_stack.max(1))
            .ok_or("unknown item")?;
        if stack.count == 0 || (give && stack.count > max * MAIN_SLOTS as u32) {
            return Err("the count is 0 or more than the inventory holds".into());
        }
        let (target, conn, dimension) = self.native_player_target(uuid)?;
        self.for_native_player(target, dimension, |game| {
            game.inventory.resize(MAIN_SLOTS, None);
            let done = if give {
                add_all(&mut game.inventory[..MAIN_SLOTS], &stack, max)
            } else {
                let mut all: Vec<Option<ItemStack>> = game.inventory.clone();
                all.push(game.offhand.take());
                let done = clear_matching(&mut all, &stack, exact, stack.count);
                // Untouched when it failed, so the slots go back as they were.
                game.offhand = all.pop().flatten();
                game.inventory = all;
                done
            };
            if done {
                game.native_player_ops.inventory_dirty.insert(conn);
            }
            done
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn stack(item: &str, count: u32) -> ItemStack {
        ItemStack {
            item: item.into(),
            count,
            damage: 0,
            extra: None,
        }
    }

    #[test]
    fn totals_conserved_in_any_arrangement() {
        let old = vec![Some(stack("a", 3)), None, Some(stack("b", 5)), Some(stack("a", 4))];
        let new = vec![Some(stack("b", 5)), Some(stack("a", 7)), None, None];
        assert!(conserved(&old, &new));
        let more = vec![Some(stack("b", 5)), Some(stack("a", 8)), None, None];
        assert!(!conserved(&old, &more));
        let other = vec![Some(stack("b", 5)), Some(stack("a", 7)), Some(stack("c", 1)), None];
        assert!(!conserved(&old, &other));
        let mut damaged = stack("a", 7);
        damaged.damage = 1;
        assert!(!conserved(&old, &[Some(stack("b", 5)), Some(damaged)]));
    }

    #[test]
    fn clearing_is_all_or_nothing_and_exact() {
        let mut slots = vec![Some(stack("a", 3)), Some(stack("b", 9)), Some(stack("a", 4))];
        assert!(!clear_matching(&mut slots, &stack("a", 8), true, 8));
        assert_eq!(slots[0].as_ref().unwrap().count, 3, "nothing changed on failure");
        assert!(clear_matching(&mut slots, &stack("a", 5), true, 5));
        assert!(slots[0].is_none());
        assert_eq!(slots[2].as_ref().unwrap().count, 2);
        // A different damage value is another kind when exact, the same item when not.
        let mut hurt = stack("a", 2);
        hurt.damage = 7;
        let mut slots = vec![Some(hurt.clone())];
        assert!(!clear_matching(&mut slots, &stack("a", 1), true, 1));
        assert!(clear_matching(&mut slots, &stack("a", 1), false, 1));
        assert_eq!(slots[0].as_ref().unwrap().count, 1);
    }

    #[test]
    fn giving_needs_room_for_all_of_it() {
        let mut slots = vec![Some(stack("a", 60)), Some(stack("b", 1))];
        assert!(!add_all(&mut slots, &stack("a", 10), 64));
        assert_eq!(slots[0].as_ref().unwrap().count, 60);
        let mut slots = vec![Some(stack("a", 60)), None];
        assert!(add_all(&mut slots, &stack("a", 10), 64));
        assert_eq!(slots[0].as_ref().unwrap().count, 64);
        assert_eq!(slots[1].as_ref().unwrap().count, 6);
    }

    #[test]
    fn player_slots_cover_main_armor_offhand_and_cursor_in_stable_order() {
        let inventory = (0..MAIN_SLOTS)
            .map(|slot| Some(stack(&format!("main_{slot}"), 1)))
            .collect::<Vec<_>>();
        let armor = std::array::from_fn(|slot| Some(stack(&format!("armor_{slot}"), 1)));
        let offhand = Some(stack("offhand", 1));
        let carried = Some(stack("cursor", 1));
        let slots = player_inventory_slots(&inventory, &armor, &offhand, &carried);
        assert_eq!(slots.len(), 42);
        assert_eq!(&*slots[0].as_ref().unwrap().item, "main_0");
        assert_eq!(&*slots[35].as_ref().unwrap().item, "main_35");
        assert_eq!(&*slots[36].as_ref().unwrap().item, "armor_0");
        assert_eq!(&*slots[39].as_ref().unwrap().item, "armor_3");
        assert_eq!(&*slots[40].as_ref().unwrap().item, "offhand");
        assert_eq!(&*slots[41].as_ref().unwrap().item, "cursor");
    }

    #[test]
    fn player_slot_removal_checks_the_exact_stack_and_changes_one_slot() {
        let mut inventory = vec![Some(stack("main", 3))];
        let mut armor = std::array::from_fn(|_| None);
        armor[0] = Some(stack("helmet", 1));
        let mut offhand = Some(stack("shield", 1));
        let mut carried = Some(stack("cursor", 2));

        assert!(!clear_exact_inventory_slot(
            &mut inventory,
            &mut armor,
            &mut offhand,
            &mut carried,
            41,
            &stack("cursor", 1),
        ));
        assert!(carried.is_some(), "a mismatch leaves the cursor unchanged");
        assert!(clear_exact_inventory_slot(
            &mut inventory,
            &mut armor,
            &mut offhand,
            &mut carried,
            36,
            &stack("helmet", 1),
        ));
        assert!(armor[0].is_none());
        assert!(inventory[0].is_some());
        assert!(!clear_exact_inventory_slot(
            &mut inventory,
            &mut armor,
            &mut offhand,
            &mut carried,
            42,
            &stack("x", 1),
        ));
    }

    #[test]
    fn the_budget_is_per_mod_helper_and_tick() {
        let tick = (NativeWorldEpoch::new(1), NativeTickId::new(1));
        for _ in 0..MAX_CALLS {
            spend(tick, 7, "budget_test").unwrap();
        }
        assert!(spend(tick, 7, "budget_test").is_err());
        assert!(spend(tick, 8, "budget_test").is_ok());
        assert!(spend(tick, 7, "budget_test_other").is_ok());
    }
}
