use super::{C, Game, play};
use horizon_client::{
    lan::{
        chunks::ChunkEncoder,
        data::{BIOME_REGISTRY, SYNCHRONIZED},
    },
    net::{NetState, SnapshotConfiguration, request_snapshot_retention},
    save::SaveTables,
};
use horizon_protocol::{
    buf::{Reader, pack_degrees},
    entity_data::Value,
    item::Stack,
    packets::{Empty, configuration as cfg, login},
    registry::Dynamic,
};
use std::{collections::BTreeMap, sync::Arc};

/// The packet count and limitations of a synchronous client-world snapshot.
#[derive(Debug, Default, PartialEq, Eq)]
pub struct SnapshotReport {
    pub written: usize,
    pub missing: Vec<String>,
}

#[derive(Default)]
struct EntityState {
    spawn: Option<play::AddEntity>,
    data: BTreeMap<u8, (i32, Value)>,
    equipment: BTreeMap<u8, Option<Stack>>,
    attributes: BTreeMap<i32, play::AttributeSnapshot>,
    passengers: Option<Vec<i32>>,
}

#[derive(Default)]
pub(super) struct SnapshotState {
    configuration: SnapshotConfiguration,
    login: Option<play::Login>,
    entities: BTreeMap<i32, EntityState>,
    players: BTreeMap<u128, (u8, play::PlayerInfo)>,
    clocks: BTreeMap<i32, play::ClockState>,
    inventory_state: i32,
}

impl SnapshotState {
    fn from_live(registries: &Dynamic, features: &[String]) -> Self {
        let mut snapshot = Self::default();
        let mut names: Vec<_> = registries.registries.keys().collect();
        let rank = |name: &str| {
            SYNCHRONIZED
                .iter()
                .position(|known| *known == name)
                .unwrap_or(usize::MAX)
        };
        names.sort_by(|left, right| rank(left).cmp(&rank(right)).then_with(|| left.cmp(right)));
        for name in names {
            snapshot
                .configuration
                .packets
                .push(cfg::Clientbound::RegistryData(cfg::RegistryData {
                    registry: name.clone(),
                    entries: registries.registries[name]
                        .iter()
                        .map(|(id, data)| cfg::RegistryEntry {
                            id: id.clone(),
                            data: data.clone(),
                        })
                        .collect(),
                }));
        }
        snapshot.configuration.packets.push(
            cfg::Clientbound::UpdateEnabledFeatures(cfg::UpdateEnabledFeatures {
                features: features.to_vec(),
            }),
        );
        for (registry, tags) in [
            ("minecraft:item", registries.item_tags.as_ref()),
            ("minecraft:dialog", Some(&registries.dialog_tags)),
        ] {
            if let Some(tags) = tags {
                let mut tags: Vec<_> = tags
                    .iter()
                    .map(|(name, ids)| cfg::TagEntry {
                        name: name.clone(),
                        ids: ids.clone(),
                    })
                    .collect();
                tags.sort_by(|left, right| left.name.cmp(&right.name));
                snapshot.remember_tags(&cfg::UpdateTags {
                    registries: vec![cfg::RegistryTags {
                        registry: registry.into(),
                        tags,
                    }],
                });
            }
        }
        snapshot
    }

    fn remember_tags(&mut self, update: &cfg::UpdateTags) {
        if let Some(cfg::Clientbound::UpdateTags(current)) = self
            .configuration
            .packets
            .iter_mut()
            .rev()
            .find(|packet| matches!(packet, cfg::Clientbound::UpdateTags(_)))
        {
            for registry in &update.registries {
                if let Some(previous) = current
                    .registries
                    .iter_mut()
                    .find(|previous| previous.registry == registry.registry)
                {
                    *previous = registry.clone();
                } else {
                    current.registries.push(registry.clone());
                }
            }
        } else {
            self.configuration
                .packets
                .push(cfg::Clientbound::UpdateTags(update.clone()));
        }
    }

    pub(super) fn configure(&mut self, configuration: SnapshotConfiguration) {
        self.configuration = configuration;
    }

    fn spawn_mut(&mut self, id: i32) -> Option<&mut play::AddEntity> {
        self.entities.get_mut(&id)?.spawn.as_mut()
    }

    pub(super) fn observe(&mut self, packet: &C) {
        match packet {
            C::Login(login) => {
                self.login = Some(login.clone());
                self.entities.clear();
                self.clocks.clear();
            }
            C::Respawn(respawn) => {
                if let Some(login) = &mut self.login {
                    login.spawn = respawn.spawn.clone();
                }
                self.entities.clear();
                self.clocks.clear();
            }
            C::AddEntity(spawn) => {
                self.entities.insert(
                    spawn.id,
                    EntityState {
                        spawn: Some(spawn.clone()),
                        ..Default::default()
                    },
                );
            }
            C::RemoveEntities(removed) => {
                for id in &removed.ids {
                    self.entities.remove(id);
                    for entity in self.entities.values_mut() {
                        if let Some(passengers) = &mut entity.passengers {
                            passengers.retain(|passenger| passenger != id);
                        }
                    }
                }
            }
            C::SetEntityData(data) => {
                let entity = self.entities.entry(data.id).or_default();
                for (index, serializer, value) in &data.values {
                    entity.data.insert(*index, (*serializer, value.clone()));
                }
            }
            C::SetEquipment(equipment) => {
                let entity = self.entities.entry(equipment.id).or_default();
                entity.equipment.extend(equipment.slots.iter().cloned());
            }
            C::UpdateAttributes(attributes) => {
                let entity = self.entities.entry(attributes.id).or_default();
                entity.attributes.extend(
                    attributes
                        .values
                        .iter()
                        .map(|value| (value.attribute, value.clone())),
                );
            }
            C::SetPassengers(passengers) => {
                self.entities.entry(passengers.vehicle).or_default().passengers =
                    Some(passengers.passengers.clone());
            }
            C::MoveEntityPos(play::MoveEntityPos(movement))
            | C::MoveEntityPosRot(play::MoveEntityPosRot(movement))
            | C::MoveEntityRot(play::MoveEntityRot(movement)) => {
                if let Some(spawn) = self.spawn_mut(movement.id) {
                    if let Some(delta) = &movement.delta {
                        spawn.position = delta.end(spawn.position);
                    }
                    if let Some((yaw, pitch)) = movement.rotation {
                        spawn.yaw = yaw;
                        spawn.pitch = pitch;
                    }
                }
            }
            C::EntityPositionSync(position) => {
                if let Some(spawn) = self.spawn_mut(position.id) {
                    spawn.position = position.path.end();
                    spawn.yaw = pack_degrees(position.yaw);
                    spawn.pitch = pack_degrees(position.pitch);
                }
            }
            C::TeleportEntity(teleport) => {
                if let Some(spawn) = self.spawn_mut(teleport.id) {
                    for (axis, relative) in [play::relative::X, play::relative::Y, play::relative::Z]
                        .into_iter()
                        .enumerate()
                    {
                        spawn.position[axis] = teleport.position[axis]
                            + if teleport.relatives & relative != 0 {
                                spawn.position[axis]
                            } else {
                                0.0
                            };
                    }
                    spawn.yaw = pack_degrees(teleport.yaw);
                    spawn.pitch = pack_degrees(teleport.pitch);
                }
            }
            C::RotateHead(rotation) => {
                if let Some(spawn) = self.spawn_mut(rotation.id) {
                    spawn.head_yaw = rotation.head_yaw;
                }
            }
            C::SetEntityMotion(motion) => {
                if let Some(spawn) = self.spawn_mut(motion.id) {
                    spawn.velocity = motion.velocity;
                }
            }
            C::MoveMinecartAlongTrack(movement) => {
                if let Some(spawn) = self.spawn_mut(movement.id)
                    && let Some(step) = movement.steps.last()
                {
                    spawn.position = step.position;
                    spawn.velocity = step.movement;
                    spawn.yaw = step.yaw;
                    spawn.pitch = step.pitch;
                }
            }
            C::PlayerInfoRemove(removed) => {
                for uuid in &removed.uuids {
                    self.players.remove(uuid);
                }
            }
            C::PlayerInfoUpdate(update) => {
                for entry in &update.entries {
                    let (actions, player) = self.players.entry(entry.uuid).or_insert_with(|| {
                        (
                            0,
                            play::PlayerInfo {
                                uuid: entry.uuid,
                                ..Default::default()
                            },
                        )
                    });
                    *actions |= update.actions & !2;
                    if update.actions & 1 != 0 {
                        player.name = entry.name.clone();
                        player.properties = entry.properties.clone();
                    }
                    if update.actions & 4 != 0 {
                        player.game_mode = entry.game_mode;
                    }
                    if update.actions & 8 != 0 {
                        player.listed = entry.listed;
                    }
                    if update.actions & 16 != 0 {
                        player.latency = entry.latency;
                    }
                    if update.actions & 32 != 0 {
                        player.display_name = entry.display_name.clone();
                    }
                    if update.actions & 64 != 0 {
                        player.list_order = entry.list_order;
                    }
                    if update.actions & 128 != 0 {
                        player.show_hat = entry.show_hat;
                    }
                }
            }
            C::SetTime(time) => {
                self.clocks
                    .extend(time.clocks.iter().map(|clock| (clock.clock, clock.clone())));
            }
            C::UpdateTags(tags) => {
                self.remember_tags(tags);
            }
            C::ContainerSetContent(content) if content.container == 0 => {
                self.inventory_state = content.state;
            }
            C::ContainerSetSlot(slot) if slot.container == 0 => {
                self.inventory_state = slot.state;
            }
            _ => {}
        }
    }

    fn prefix(&self, profile: login::GameProfile, output: &mut Output<'_>) {
        output.frame(
            NetState::Login,
            login::Clientbound::LoginFinished(login::LoginFinished {
                profile,
                session: 0,
            })
            .encode(),
        );
        if !self.configuration.complete {
            output.missing("configuration before retention was enabled; prime RH-9 before joining");
        }
        for packet in &self.configuration.packets {
            output.frame(NetState::Configuration, packet.encode());
        }
        output.frame(
            NetState::Configuration,
            cfg::Clientbound::FinishConfiguration(Empty).encode(),
        );
    }

    fn write_entities(&self, own_id: i32, output: &mut Output<'_>) {
        for (&id, entity) in &self.entities {
            if let Some(spawn) = &entity.spawn {
                output.play(C::AddEntity(spawn.clone()));
            } else if id != own_id {
                output.missing(format!("entity {id}: spawn predates retention"));
            }
        }
        for (&id, entity) in &self.entities {
            if entity.spawn.is_none() && id != own_id {
                continue;
            }
            if !entity.data.is_empty() {
                output.play(C::SetEntityData(play::SetEntityData {
                    id,
                    values: entity
                        .data
                        .iter()
                        .map(|(&index, (serializer, value))| (index, *serializer, value.clone()))
                        .collect(),
                }));
            }
            if !entity.equipment.is_empty() {
                output.play(C::SetEquipment(play::SetEquipment {
                    id,
                    slots: entity
                        .equipment
                        .iter()
                        .map(|(&slot, stack)| (slot, stack.clone()))
                        .collect(),
                }));
            }
            if !entity.attributes.is_empty() {
                output.play(C::UpdateAttributes(play::UpdateAttributes {
                    id,
                    values: entity.attributes.values().cloned().collect(),
                }));
            }
        }
        for (&vehicle, entity) in &self.entities {
            if entity.spawn.is_none() && vehicle != own_id {
                continue;
            }
            if let Some(passengers) = &entity.passengers {
                let passengers = passengers
                    .iter()
                    .copied()
                    .filter(|passenger| {
                        let known = *passenger == own_id
                            || self
                                .entities
                                .get(passenger)
                                .is_some_and(|entity| entity.spawn.is_some());
                        if !known {
                            output.missing(format!(
                                "entity {vehicle}: passenger {passenger} spawn predates retention"
                            ));
                        }
                        known
                    })
                    .collect();
                output.play(C::SetPassengers(play::SetPassengers {
                    vehicle,
                    passengers,
                }));
            }
        }
    }
}

struct Output<'a> {
    out: &'a mut dyn FnMut(NetState, i32, &[u8]),
    report: SnapshotReport,
}

impl Output<'_> {
    fn frame(&mut self, state: NetState, bytes: Vec<u8>) {
        let mut reader = Reader::new(&bytes);
        match reader.varint() {
            Ok(id) => {
                (self.out)(state, id, reader.rest());
                self.report.written += 1;
            }
            Err(error) => self.missing(format!("packet header: {error}")),
        }
    }

    fn play(&mut self, packet: C) {
        self.frame(NetState::Play, packet.encode());
    }

    fn missing(&mut self, reason: impl Into<String>) {
        self.report.missing.push(reason.into());
    }

    fn limitations(&mut self) {
        for reason in [
            "entity effects, transient animations and interpolation history are not written",
            "scoreboard, teams and boss bars are not written",
            "maps, recipes, advancements and commands are not written",
            "open non-inventory menus and custom payload state are not written",
        ] {
            self.missing(reason);
        }
    }
}

impl Game<'_> {
    pub(crate) fn snapshot_client_world(
        &mut self,
        out: &mut dyn FnMut(NetState, i32, &[u8]),
    ) -> SnapshotReport {
        request_snapshot_retention();
        let mut output = Output {
            out,
            report: SnapshotReport::default(),
        };
        if !self.options.execution_role.is_client() || self.native_client_world_key().is_none() {
            output.missing(
                "no loaded client world; configuration retention is armed for the next join",
            );
            return output.report;
        }
        let Some(remote) = &mut self.remote else {
            output.missing("legacy non-pipe world: no configured protocol registry or login");
            return output.report;
        };
        if remote.snapshot.is_none() {
            remote.snapshot = Some(SnapshotState::from_live(
                &remote.registries,
                &remote.features,
            ));
        }
        let remote = self.remote.as_ref().unwrap();
        let snapshot = remote.snapshot.as_ref().unwrap();
        let properties = snapshot
            .players
            .get(&remote.profile)
            .map_or_else(Vec::new, |(_, player)| player.properties.clone());
        let login = if let Some(login) = &snapshot.login {
            login.clone()
        } else {
            output.missing("login-only fields before retention: level list, max players, reduced debug, previous game mode, debug-world flag and sea level use replay defaults");
            play::Login {
                player_id: remote.entity_id,
                hardcore: self.level.hardcore,
                levels: vec![remote.level.clone()],
                max_players: 1,
                chunk_radius: remote.view_distance,
                simulation_distance: remote.simulation_distance,
                reduced_debug_info: false,
                show_death_screen: remote.show_death_screen,
                limited_crafting: remote.do_limited_crafting,
                spawn: play::SpawnInfo {
                    dimension_type: match remote
                        .registries
                        .id("minecraft:dimension_type", &remote.dimension.name)
                    {
                        Some(id) => id,
                        None => {
                            output.missing(
                                "current dimension type is absent from the synchronized registry",
                            );
                            return output.report;
                        }
                    },
                    dimension: remote.level.clone(),
                    seed: remote.hashed_seed,
                    game_type: remote.game_mode,
                    previous_game_type: None,
                    debug: false,
                    flat: remote.flat,
                    last_death: self.level.last_death.as_ref().map(|(dimension, pos)| {
                        play::GlobalPos {
                            dimension: dimension.clone(),
                            pos: *pos,
                        }
                    }),
                    portal_cooldown: self.portals.player.cooldown,
                    sea_level: 63,
                },
                online_mode: false,
                enforces_secure_chat: false,
            }
        };
        let mut login = login;
        login.online_mode = false;
        login.enforces_secure_chat = false;
        login.spawn.game_type = remote.game_mode;
        login.spawn.portal_cooldown = self.portals.player.cooldown;
        login.show_death_screen = remote.show_death_screen;
        login.limited_crafting = remote.do_limited_crafting;
        login.chunk_radius = remote.view_distance;
        login.simulation_distance = remote.simulation_distance;
        snapshot.prefix(
            login::GameProfile {
                uuid: remote.profile,
                name: remote.profile_name.clone(),
                properties,
            },
            &mut output,
        );
        output.play(C::Login(login));
        let mut players = snapshot.players.clone();
        for (&uuid, name) in &remote.players {
            let (actions, player) = players.entry(uuid).or_insert_with(|| {
                (
                    0,
                    play::PlayerInfo {
                        uuid,
                        ..Default::default()
                    },
                )
            });
            if *actions & 1 == 0 {
                output.missing(format!("player info {uuid}: profile properties predate retention"));
                player.name = Some(name.clone());
                *actions |= 1;
            }
            if let Some(tab) = remote.tab_players.get(&uuid) {
                *actions |= 4 | 8 | 16 | 32 | 64 | 128;
                player.game_mode = Some(tab.game_mode);
                player.listed = Some(remote.listed.contains(&uuid));
                player.latency = Some(tab.latency);
                player.display_name = Some(
                    tab.display_name
                        .as_ref()
                        .map(horizon_client::scoreboard::json_to_tag),
                );
                player.list_order = Some(tab.list_order);
                player.show_hat = Some(tab.show_hat);
            }
        }
        for (actions, player) in players.values() {
            if actions & 1 != 0 {
                output.play(C::PlayerInfoUpdate(play::PlayerInfoUpdate {
                    actions: *actions,
                    entries: vec![player.clone()],
                }));
            } else {
                output.missing(format!(
                    "player info {}: add-player predates retention",
                    player.uuid
                ));
            }
        }
        output.play(C::SetChunkCacheCenter(play::SetChunkCacheCenter {
            x: remote.chunk_center.0,
            z: remote.chunk_center.1,
        }));
        let biome_names: Vec<String> = world_biomes!(self)
            .list
            .iter()
            .map(|biome| biome.name.clone())
            .collect();
        let registry: Vec<String> = remote
            .registries
            .registries
            .get(BIOME_REGISTRY)
            .map(|entries| entries.iter().map(|(name, _)| name.clone()).collect())
            .unwrap_or_default();
        let mut encoder = ChunkEncoder::with_signs(
            self.res.facts().states.iter().map(|state| {
                (
                    state.name.clone(),
                    state.fluid_level > 0 || state.waterlogged,
                    state.sign_form(),
                )
            }),
            &remote.builtin,
            &biome_names,
            &registry,
        );
        encoder.set_dimension(self.world.shape);
        encoder.tables = Some(Arc::new(SaveTables::new(&self.pack.assets)));
        encoder.mod_update_tag = Some(|position, entity| {
            let mut tag = None;
            Game::native_block_entity_update_tag(position, entity, &mut tag);
            tag
        });
        let mut chunks: Vec<_> = self.world.chunks.values().collect();
        chunks.sort_by_key(|chunk| (chunk.x, chunk.z));
        for chunk in chunks {
            output.play(C::LevelChunkWithLight(
                encoder.encode(chunk, &self.world.block_entities),
            ));
        }
        snapshot.write_entities(remote.entity_id, &mut output);
        for id in remote.entities.keys() {
            if !snapshot
                .entities
                .get(id)
                .is_some_and(|entity| entity.spawn.is_some())
            {
                output.missing(format!("entity {id}: spawn predates retention"));
            }
        }
        for (event, param) in [
            (if self.world.weather.raining { 1 } else { 2 }, 0.0),
            (7, self.world.weather.rain_level.1),
            (8, self.world.weather.thunder_level.1),
        ] {
            output.play(C::GameEvent(play::GameEvent { event, param }));
        }
        let mut clocks = snapshot.clocks.clone();
        if let Some(clock) = remote.clock {
            clocks.insert(
                clock,
                play::ClockState {
                    clock,
                    total_ticks: self.world.day_time,
                    partial_tick: remote.clock_partial,
                    rate: remote.clock_rate,
                },
            );
        } else {
            output.missing("world clock: no clock id has been received");
        }
        output.play(C::SetTime(play::SetTime {
            game_time: self.world.game_time,
            clocks: clocks.into_values().collect(),
        }));
        let center = self.border.center();
        output.play(C::InitializeBorder(play::InitializeBorder {
            center_x: center[0],
            center_z: center[1],
            old_size: self.border.size(),
            new_size: self.border.target_size(),
            lerp_time: self.border.remaining_ticks(),
            absolute_max_size: self.border.absolute_max_size(),
            warning_blocks: self.border.warning_distance,
            warning_time: self.border.warning_time,
        }));
        self.snapshot_own_player(&mut output);
        output.limitations();
        if snapshot
            .clocks
            .keys()
            .any(|clock| Some(*clock) != remote.clock)
        {
            output.missing(
                "inactive world clocks retain their last received values, not unobserved server ticks",
            );
        }
        output.report
    }

    fn snapshot_own_player(&self, output: &mut Output<'_>) {
        let Some(remote) = &self.remote else {
            return;
        };
        let Some(player) = &self.player else {
            output.missing("local player: the first position packet has not been applied");
            return;
        };
        output.play(C::PlayerPosition(play::PlayerPosition {
            teleport: 0,
            position: [player.position.x, player.position.y, player.position.z],
            delta: [player.velocity.x, player.velocity.y, player.velocity.z],
            yaw: self.yaw,
            pitch: self.pitch,
            relatives: 0,
        }));
        output.play(C::SetHealth(play::SetHealth {
            health: player.health.health(),
            food: player.health.food.food,
            saturation: player.health.food.saturation,
        }));
        output.play(C::SetExperience(play::SetExperience {
            progress: self.experience.progress,
            level: self.experience.level,
            total: self.experience.total,
        }));
        if let Some(abilities) = &remote.abilities {
            let mut abilities = abilities.clone();
            abilities.flags = (abilities.flags & !2) | if player.flying { 2 } else { 0 };
            output.play(C::PlayerAbilities(abilities));
        } else {
            output.missing("player abilities: no abilities packet has been received");
        }
        let mut items = vec![None; 46];
        if remote.menu.is_none() {
            items[0] = self.to_network(&remote.craft_result);
        } else {
            output.missing("inventory crafting result while a non-inventory menu is open");
        }
        for (index, item) in self.craft_grid.iter().take(4).enumerate() {
            items[1 + index] = self.to_network(item);
        }
        for (index, item) in self.armor.iter().enumerate() {
            items[5 + index] = self.to_network(item);
        }
        for (index, item) in self.inventory.iter().take(36).enumerate() {
            items[if index < 9 { 36 + index } else { index }] = self.to_network(item);
        }
        items[45] = self.to_network(&self.offhand);
        output.play(C::ContainerSetContent(play::ContainerSetContent {
            container: 0,
            state: remote
                .snapshot
                .as_ref()
                .map_or(0, |snapshot| snapshot.inventory_state),
            items,
            carried: self.to_network(&self.carried),
        }));
        output.play(C::SetHeldSlot(play::SetHeldSlot {
            slot: self.selected as i32,
        }));
        let equipment = [
            self.inventory
                .get(self.selected)
                .and_then(|stack| self.to_network(stack)),
            self.to_network(&self.offhand),
            self.to_network(&self.armor[3]),
            self.to_network(&self.armor[2]),
            self.to_network(&self.armor[1]),
            self.to_network(&self.armor[0]),
        ];
        output.play(C::SetEquipment(play::SetEquipment {
            id: remote.entity_id,
            slots: equipment
                .into_iter()
                .enumerate()
                .map(|(slot, stack)| (slot as u8, stack))
                .collect(),
        }));
        if let Some(vehicle) = self.riding {
            let vehicle = vehicle as i32;
            if !remote.snapshot.as_ref().is_some_and(|snapshot| {
                snapshot
                    .entities
                    .get(&vehicle)
                    .and_then(|entity| entity.passengers.as_ref())
                    .is_some_and(|passengers| passengers.contains(&remote.entity_id))
            }) {
                output.missing("local vehicle passenger list predates retention");
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::chunk::{Chunk, Light};
    use horizon_protocol::registry::Builtin;
    use std::collections::HashMap;

    fn profile() -> login::GameProfile {
        login::GameProfile {
            uuid: 17,
            name: "ReplayViewer".into(),
            properties: Vec::new(),
        }
    }

    fn login_packet() -> play::Login {
        play::Login {
            player_id: 7,
            hardcore: false,
            levels: vec!["minecraft:overworld".into()],
            max_players: 1,
            chunk_radius: 3,
            simulation_distance: 3,
            reduced_debug_info: false,
            show_death_screen: true,
            limited_crafting: false,
            spawn: play::SpawnInfo {
                dimension_type: 0,
                dimension: "minecraft:overworld".into(),
                seed: 0,
                game_type: 0,
                previous_game_type: None,
                debug: false,
                flat: false,
                last_death: None,
                portal_cooldown: 0,
                sea_level: 63,
            },
            online_mode: false,
            enforces_secure_chat: false,
        }
    }

    fn entity(id: i32) -> play::AddEntity {
        play::AddEntity {
            id,
            uuid: id as u128,
            kind: 0,
            position: [1.0, 72.0, 2.0],
            velocity: [0.0; 3],
            pitch: 0,
            yaw: 0,
            head_yaw: 0,
            data: 0,
        }
    }

    #[test]
    fn synthetic_checkpoint_decodes_with_client_chunk_light_and_ordered_entities() {
        let configuration = vec![
            cfg::Clientbound::RegistryData(cfg::RegistryData {
                registry: BIOME_REGISTRY.into(),
                entries: vec![cfg::RegistryEntry {
                    id: "minecraft:plains".into(),
                    data: None,
                }],
            }),
            cfg::Clientbound::UpdateEnabledFeatures(cfg::UpdateEnabledFeatures {
                features: vec!["minecraft:vanilla".into()],
            }),
        ];
        let mut snapshot = SnapshotState::default();
        snapshot.configure(SnapshotConfiguration {
            packets: configuration.clone(),
            complete: true,
        });
        snapshot.observe(&C::Login(login_packet()));
        snapshot.observe(&C::AddEntity(entity(10)));
        snapshot.observe(&C::AddEntity(entity(11)));
        snapshot.observe(&C::SetEntityData(play::SetEntityData {
            id: 10,
            values: vec![(0, 0, Value::Byte(8))],
        }));
        snapshot.observe(&C::SetEquipment(play::SetEquipment {
            id: 11,
            slots: vec![(0, Some(Stack::new(1, 1)))],
        }));
        snapshot.observe(&C::UpdateAttributes(play::UpdateAttributes {
            id: 11,
            values: vec![play::AttributeSnapshot {
                attribute: 0,
                base: 20.0,
                modifiers: Vec::new(),
            }],
        }));
        snapshot.observe(&C::SetPassengers(play::SetPassengers {
            vehicle: 10,
            passengers: vec![11, 7],
        }));
        let builtin = Builtin::parse("{}").unwrap();
        let encoder = ChunkEncoder::new(
            ["minecraft:air", "minecraft:stone"]
                .map(|name| (Arc::<str>::from(name), false)),
            &builtin,
            &[],
            &[],
        );
        let mut chunk = Chunk::empty(-7, 12);
        chunk.set(3, 70, 9, 1);
        Arc::make_mut(&mut chunk.sections[5]).light = Light::Uniform(7);
        let chunk_packet = encoder.encode(&chunk, &HashMap::new());
        let mut frames = Vec::new();
        let report = {
            let mut receive = |state, id, body: &[u8]| {
                frames.push((state, id, body.to_vec()));
            };
            let mut output = Output {
                out: &mut receive,
                report: SnapshotReport::default(),
            };
            snapshot.prefix(profile(), &mut output);
            output.play(C::Login(snapshot.login.clone().unwrap()));
            output.play(C::LevelChunkWithLight(chunk_packet.clone()));
            snapshot.write_entities(7, &mut output);
            output.limitations();
            output.report
        };
        assert_eq!(report.written, frames.len());
        assert_eq!(report.missing.len(), 4);
        assert!(report.missing.iter().any(|reason| reason.contains("scoreboard")));
        assert!(report.missing.iter().any(|reason| reason.contains("entity effects")));
        let mut configuration_packets = Vec::new();
        let mut play_packets = Vec::new();
        for (state, id, body) in &frames {
            match state {
                NetState::Login => assert_eq!(
                    login::Clientbound::decode(*id, body).unwrap(),
                    login::Clientbound::LoginFinished(login::LoginFinished {
                        profile: profile(),
                        session: 0,
                    }),
                ),
                NetState::Configuration => {
                    configuration_packets.push(cfg::Clientbound::decode(*id, body).unwrap());
                }
                NetState::Play => play_packets.push(C::decode(*id, body).unwrap()),
            }
        }
        assert_eq!(configuration_packets[..configuration.len()], configuration);
        assert_eq!(
            configuration_packets.last(),
            Some(&cfg::Clientbound::FinishConfiguration(Empty)),
        );
        assert_eq!(play_packets[0], C::Login(login_packet()));
        assert_eq!(play_packets[1], C::LevelChunkWithLight(chunk_packet));
        let C::LevelChunkWithLight(decoded_chunk) = &play_packets[1] else {
            panic!("expected a chunk");
        };
        assert_eq!((decoded_chunk.x, decoded_chunk.z), (-7, 12));
        assert!(!decoded_chunk.chunk.buffer.is_empty());
        assert!(!decoded_chunk.light.block.is_empty());
        let passengers = play_packets.iter().position(|packet| {
            matches!(packet, C::SetPassengers(passengers) if passengers.vehicle == 10)
        }).unwrap();
        for id in [10, 11] {
            assert!(play_packets.iter().position(|packet| {
                matches!(packet, C::AddEntity(spawn) if spawn.id == id)
            }).unwrap() < passengers);
        }
        assert!(play_packets.contains(&C::SetEntityData(play::SetEntityData {
            id: 10,
            values: vec![(0, 0, Value::Byte(8))],
        })));
        assert!(play_packets.contains(&C::SetEquipment(play::SetEquipment {
            id: 11,
            slots: vec![(0, Some(Stack::new(1, 1)))],
        })));
    }

    #[test]
    fn retained_deltas_merge_replacements_and_remove_stale_entities_and_chat_keys() {
        let mut snapshot = SnapshotState::default();
        snapshot.observe(&C::AddEntity(entity(10)));
        snapshot.observe(&C::AddEntity(entity(11)));
        for value in [8, 16] {
            snapshot.observe(&C::SetEntityData(play::SetEntityData {
                id: 10,
                values: vec![(0, 0, Value::Byte(value))],
            }));
        }
        for item in [Some(Stack::new(1, 1)), None] {
            snapshot.observe(&C::SetEquipment(play::SetEquipment {
                id: 10,
                slots: vec![(0, item)],
            }));
        }
        snapshot.observe(&C::MoveEntityPosRot(play::MoveEntityPosRot(play::MoveEntity {
            id: 10,
            delta: Some(play::VecDelta::Linear([4096, 0, -4096])),
            rotation: Some((12, 6)),
            on_ground: true,
        })));
        snapshot.observe(&C::SetPassengers(play::SetPassengers {
            vehicle: 10,
            passengers: vec![11, 7],
        }));
        snapshot.observe(&C::PlayerInfoUpdate(play::PlayerInfoUpdate {
            actions: 1 | 2 | 4,
            entries: vec![play::PlayerInfo {
                uuid: 17,
                name: Some("ReplayViewer".into()),
                chat_session: Some((3, 4, vec![5], vec![6])),
                game_mode: Some(1),
                ..Default::default()
            }],
        }));
        snapshot.observe(&C::PlayerInfoUpdate(play::PlayerInfoUpdate {
            actions: 16,
            entries: vec![play::PlayerInfo {
                uuid: 17,
                latency: Some(42),
                ..Default::default()
            }],
        }));
        snapshot.observe(&C::RemoveEntities(play::RemoveEntities { ids: vec![11] }));
        let retained = &snapshot.entities[&10];
        assert_eq!(retained.data[&0], (0, Value::Byte(16)));
        assert_eq!(retained.equipment[&0], None);
        let spawn = retained.spawn.as_ref().unwrap();
        assert_eq!(spawn.position, [2.0, 72.0, 1.0]);
        assert_eq!((spawn.yaw, spawn.pitch), (12, 6));
        assert_eq!(retained.passengers.as_ref().unwrap(), &[7]);
        assert!(!snapshot.entities.contains_key(&11));
        let (actions, player) = &snapshot.players[&17];
        assert_eq!(*actions, 1 | 4 | 16);
        assert_eq!(player.name.as_deref(), Some("ReplayViewer"));
        assert_eq!(player.game_mode, Some(1));
        assert_eq!(player.latency, Some(42));
        assert_eq!(player.chat_session, None);
    }

    #[test]
    fn cold_checkpoint_reports_gaps_and_omits_unspawned_passengers() {
        let registries = Dynamic::default();
        let mut snapshot = SnapshotState::from_live(&registries, &["minecraft:vanilla".into()]);
        snapshot.observe(&C::AddEntity(entity(10)));
        snapshot.observe(&C::SetEntityData(play::SetEntityData {
            id: 30,
            values: vec![(0, 0, Value::Byte(8))],
        }));
        snapshot.observe(&C::SetPassengers(play::SetPassengers {
            vehicle: 10,
            passengers: vec![7, 99],
        }));
        let mut frames = Vec::new();
        let report = {
            let mut receive = |state, id, body: &[u8]| {
                frames.push((state, id, body.to_vec()));
            };
            let mut output = Output {
                out: &mut receive,
                report: SnapshotReport::default(),
            };
            snapshot.prefix(profile(), &mut output);
            snapshot.write_entities(7, &mut output);
            output.report
        };
        assert!(report.missing.iter().any(|reason| reason.contains("configuration before")));
        assert!(report.missing.iter().any(|reason| reason == "entity 30: spawn predates retention"));
        assert!(report.missing.iter().any(|reason| reason.contains("passenger 99")));
        let packets: Vec<_> = frames.iter().filter_map(|(state, id, body)| {
            (*state == NetState::Play).then(|| C::decode(*id, body).unwrap())
        }).collect();
        assert!(packets.contains(&C::SetPassengers(play::SetPassengers {
            vehicle: 10,
            passengers: vec![7],
        })));
        assert!(!packets.iter().any(|packet| matches!(packet, C::SetEntityData(_))));
    }

    #[test]
    fn tag_updates_preserve_other_registries_and_replace_the_updated_registry() {
        let mut snapshot = SnapshotState::default();
        for (registry, value) in [("minecraft:item", 1), ("minecraft:block", 2), ("minecraft:item", 3)] {
            snapshot.remember_tags(&cfg::UpdateTags {
                registries: vec![cfg::RegistryTags {
                    registry: registry.into(),
                    tags: vec![cfg::TagEntry { name: "test:tag".into(), ids: vec![value] }],
                }],
            });
        }
        let [cfg::Clientbound::UpdateTags(tags)] = snapshot.configuration.packets.as_slice() else {
            panic!("expected one merged tag packet");
        };
        assert_eq!(tags.registries.len(), 2);
        assert_eq!(tags.registries[0].tags[0].ids, [3]);
        assert_eq!(tags.registries[1].tags[0].ids, [2]);
    }
}
