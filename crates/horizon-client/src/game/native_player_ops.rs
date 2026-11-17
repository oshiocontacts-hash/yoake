//! Authoritative, tick-bounded operations on server players.

use std::{
    borrow::Cow,
    collections::{HashMap, HashSet},
};

use horizon_client::{
    brewing,
    command::args::GameMode,
    effects::EffectInstance,
    mod_api::{
        NativeAttributeOperation, NativeGameMode, NativeTeleportStatus, NativeTeleportTicket,
        NativeTickId, NativeWorldEpoch,
    },
};
use horizon_protocol::packets::play::{self, Clientbound as C};
use horizon_world::attributes::{Modifier, Operation as AttributeOperation};

use super::{
    Dim, Game, commands::Target, native_client_state::NativeSoundCategory,
    native_entity_query::require_server_context, server_travel::LevelTeleport,
};

const MAX_CALLS: u16 = 512;

#[derive(Clone, Copy)]
enum Operation {
    AttributeAdd,
    AttributeRemove,
    EffectGive,
    EffectClear,
    Teleport,
    TeleportStatus,
    GameMode,
    Health,
    Inventory,
    Title,
    ClearTitle,
    BossCreate,
    BossName,
    BossProgress,
    BossPlayers,
    BossRemove,
    Sound,
    BossColor,
    BossOverlay,
}

impl Operation {
    fn index(self) -> usize {
        match self {
            Self::AttributeAdd => 0,
            Self::AttributeRemove => 1,
            Self::EffectGive => 2,
            Self::EffectClear => 3,
            Self::Teleport => 4,
            Self::TeleportStatus => 5,
            Self::GameMode => 6,
            Self::Health => 7,
            Self::Inventory => 8,
            Self::Title => 9,
            Self::ClearTitle => 10,
            Self::BossCreate => 11,
            Self::BossName => 12,
            Self::BossProgress => 13,
            Self::BossPlayers => 14,
            Self::BossRemove => 15,
            Self::Sound => 16,
            Self::BossColor => 17,
            Self::BossOverlay => 18,
        }
    }
}

struct Usage {
    mod_index: usize,
    calls: [u16; 19],
}

pub(super) struct NativeBossBar {
    owner: usize,
    id: String,
    pub(super) uuid: u128,
    pub(super) name: String,
    pub(super) progress: f32,
    pub(super) color: i32,
    pub(super) overlay: i32,
    pub(super) players: Vec<u128>,
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum Phase {
    Queued,
    Processing,
    Arriving,
    Finished,
}

struct Ticket {
    owner: usize,
    conn: u32,
    phase: Phase,
    status: NativeTeleportStatus,
}

#[derive(Default)]
pub(super) struct PlayerOpsState {
    tick: Option<(NativeWorldEpoch, NativeTickId)>,
    usage: Vec<Usage>,
    next_ticket: u64,
    tickets: HashMap<NativeTeleportTicket, Ticket>,
    active: HashMap<u32, NativeTeleportTicket>,
    pub(super) queued: HashMap<usize, NativeTeleportTicket>,
    pub(super) inventory_dirty: HashSet<u32>,
    pub(super) bars: Vec<NativeBossBar>,
    pub(super) sound_packet_indices: Vec<usize>,
}

impl PlayerOpsState {
    fn create_bar(&mut self, owner: usize, id: &str, name: &str) -> bool {
        let Some(id) = canonical_identifier(id) else {
            return false;
        };
        if name.len() > 32767
            || self.bars.iter().filter(|bar| bar.owner == owner).count() >= 16
            || self
                .bars
                .iter()
                .any(|bar| bar.owner == owner && bar.id == id)
        {
            return false;
        }
        let uuid = horizon_client::bossbar::stable_uuid(&format!("horizon:native/{owner}/{id}"));
        if self.bars.iter().any(|bar| bar.uuid == uuid) {
            return false;
        }
        self.bars.push(NativeBossBar {
            owner,
            id,
            uuid,
            name: name.to_owned(),
            progress: 0.0,
            color: 6,
            overlay: 0,
            players: Vec::new(),
        });
        true
    }

    fn bar_mut(&mut self, owner: usize, id: &str) -> Option<&mut NativeBossBar> {
        let id = canonical_identifier(id)?;
        self.bars
            .iter_mut()
            .find(|bar| bar.owner == owner && bar.id == id)
    }

    fn set_bar_name(&mut self, owner: usize, id: &str, name: &str) -> bool {
        if name.len() > 32767 {
            return false;
        }
        let Some(bar) = self.bar_mut(owner, id) else {
            return false;
        };
        bar.name = name.to_owned();
        true
    }

    fn set_bar_progress(&mut self, owner: usize, id: &str, progress: f32) -> bool {
        if !progress.is_finite() {
            return false;
        }
        let Some(bar) = self.bar_mut(owner, id) else {
            return false;
        };
        bar.progress = progress.clamp(0.0, 1.0);
        true
    }

    fn set_bar_color(&mut self, owner: usize, id: &str, color: &str) -> bool {
        let Some(color) = horizon_client::bossbar::COLORS
            .iter()
            .position(|value| *value == color)
        else {
            return false;
        };
        let Some(bar) = self.bar_mut(owner, id) else {
            return false;
        };
        bar.color = color as i32;
        true
    }

    fn set_bar_overlay(&mut self, owner: usize, id: &str, overlay: &str) -> bool {
        let Some(overlay) = horizon_client::bossbar::STYLES
            .iter()
            .position(|value| *value == overlay)
        else {
            return false;
        };
        let Some(bar) = self.bar_mut(owner, id) else {
            return false;
        };
        bar.overlay = overlay as i32;
        true
    }

    fn set_bar_players(&mut self, owner: usize, id: &str, players: &[u128]) -> bool {
        if players.len() > 1024 {
            return false;
        }
        let Some(bar) = self.bar_mut(owner, id) else {
            return false;
        };
        bar.players = players.to_vec();
        true
    }

    fn remove_bar(&mut self, owner: usize, id: &str) -> bool {
        let Some(id) = canonical_identifier(id) else {
            return false;
        };
        let Some(index) = self
            .bars
            .iter()
            .position(|bar| bar.owner == owner && bar.id == id)
        else {
            return false;
        };
        self.bars.remove(index);
        true
    }

    fn reserve(
        &mut self,
        tick: (NativeWorldEpoch, NativeTickId),
        mod_index: usize,
        operation: Operation,
    ) -> Result<(), String> {
        if self.tick != Some(tick) {
            if self.tick.is_some_and(|previous| previous.0 != tick.0) {
                self.tickets.clear();
                self.active.clear();
                self.queued.clear();
                self.bars.clear();
                self.inventory_dirty.clear();
            }
            self.tick = Some(tick);
            self.usage.clear();
        }
        let index = match self
            .usage
            .iter()
            .position(|usage| usage.mod_index == mod_index)
        {
            Some(index) => index,
            None => {
                self.usage.push(Usage {
                    mod_index,
                    calls: [0; 19],
                });
                self.usage.len() - 1
            }
        };
        let calls = &mut self.usage[index].calls[operation.index()];
        if *calls >= MAX_CALLS {
            return Err(
                "native player operation limit exceeded (512 calls per helper, mod and server tick)"
                    .into(),
            );
        }
        *calls += 1;
        Ok(())
    }

    fn ticket(&mut self, owner: usize, conn: u32) -> Result<NativeTeleportTicket, String> {
        if self.active.contains_key(&conn) {
            return Err("the player already has a pending native teleport".into());
        }
        self.next_ticket = self
            .next_ticket
            .checked_add(1)
            .ok_or("native teleport ticket space exhausted")?;
        let ticket = NativeTeleportTicket(self.next_ticket);
        self.tickets.insert(
            ticket,
            Ticket {
                owner,
                conn,
                phase: Phase::Queued,
                status: NativeTeleportStatus::Pending,
            },
        );
        self.active.insert(conn, ticket);
        Ok(ticket)
    }

    pub(super) fn processing(&mut self, ticket: Option<NativeTeleportTicket>) {
        if let Some(record) = ticket.and_then(|ticket| self.tickets.get_mut(&ticket)) {
            record.phase = Phase::Processing;
        }
    }

    pub(super) fn arriving(&mut self, ticket: Option<NativeTeleportTicket>) {
        if let Some(record) = ticket.and_then(|ticket| self.tickets.get_mut(&ticket)) {
            record.phase = Phase::Arriving;
        }
    }

    pub(super) fn finish(&mut self, ticket: Option<NativeTeleportTicket>, error: Option<String>) {
        if let Some(record) = ticket.and_then(|ticket| self.tickets.get_mut(&ticket)) {
            if record.phase == Phase::Finished {
                return;
            }
            record.phase = Phase::Finished;
            record.status = match error {
                Some(error) => NativeTeleportStatus::Failed(error),
                None => NativeTeleportStatus::Done,
            };
            self.active.remove(&record.conn);
        }
    }
}

fn operation_name(operation: NativeAttributeOperation) -> &'static str {
    match operation {
        NativeAttributeOperation::AddValue => "add_value",
        NativeAttributeOperation::AddMultipliedBase => "add_multiplied_base",
        NativeAttributeOperation::AddMultipliedTotal => "add_multiplied_total",
    }
}

pub(super) fn attribute_name(attribute: &str) -> Cow<'_, str> {
    if attribute.contains(':') {
        Cow::Borrowed(attribute)
    } else {
        Cow::Owned(format!("minecraft:{attribute}"))
    }
}

fn valid_identifier(id: &str) -> bool {
    horizon_world::identifier::Identifier::parse(id)
        .is_ok_and(|identifier| !identifier.path().is_empty())
}

pub(super) fn canonical_identifier(id: &str) -> Option<String> {
    if id.len() > 256 {
        return None;
    }
    let identifier = horizon_world::identifier::Identifier::parse(id).ok()?;
    (!identifier.path().is_empty()).then(|| identifier.to_string())
}

fn clear_player_slots<T>(
    inventory: &mut [Option<T>],
    armor: &mut [Option<T>],
    offhand: &mut Option<T>,
    carried: &mut Option<T>,
) {
    for slot in inventory.iter_mut().chain(armor.iter_mut()) {
        *slot = None;
    }
    *offhand = None;
    *carried = None;
}

fn title_packets(
    title: Option<&str>,
    subtitle: Option<&str>,
    fade_in: i32,
    stay: i32,
    fade_out: i32,
) -> Option<Vec<C>> {
    if title.is_some_and(|text| text.len() > 32767)
        || subtitle.is_some_and(|text| text.len() > 32767)
    {
        return None;
    }
    let mut packets = vec![C::SetTitlesAnimation(play::TitlesAnimation {
        fade_in,
        stay,
        fade_out,
    })];
    for (text, is_title) in [(subtitle, false), (title, true)] {
        if let Some(text) = text {
            let text = play::Text {
                text: horizon_client::lan::data::component_nbt(
                    &horizon_client::text::Text::literal(text.to_owned()).component(),
                ),
            };
            packets.push(if is_title {
                C::SetTitleText(text)
            } else {
                C::SetSubtitleText(text)
            });
        }
    }
    Some(packets)
}

fn clear_title_packet() -> C {
    C::ClearTitles(play::Bool1 { value: false })
}

fn sound_packet(
    sound: &str,
    category: NativeSoundCategory,
    position: [f64; 3],
    volume: f32,
    pitch: f32,
) -> Option<C> {
    if sound.len() > 256
        || !valid_identifier(sound)
        || !position.iter().all(|value| {
            value.is_finite()
                && (*value * 8.0) >= f64::from(i32::MIN)
                && (*value * 8.0) <= f64::from(i32::MAX)
        })
        || !volume.is_finite()
        || volume < 0.0
        || !pitch.is_finite()
        || !(0.0..=2.0).contains(&pitch)
    {
        return None;
    }
    let source = match category {
        NativeSoundCategory::Master => 0,
        NativeSoundCategory::Music => 1,
        NativeSoundCategory::Records => 2,
        NativeSoundCategory::Weather => 3,
        NativeSoundCategory::Blocks => 4,
        NativeSoundCategory::Hostile => 5,
        NativeSoundCategory::Neutral => 6,
        NativeSoundCategory::Players => 7,
        NativeSoundCategory::Ambient => 8,
        NativeSoundCategory::Voice => 9,
        NativeSoundCategory::Ui => 10,
    };
    Some(C::Sound(play::Sound {
        sound: (
            -1,
            Some((
                horizon_world::identifier::Identifier::parse(sound)
                    .ok()?
                    .to_string(),
                None,
            )),
        ),
        source,
        x: (position[0] * 8.0) as i32,
        y: (position[1] * 8.0) as i32,
        z: (position[2] * 8.0) as i32,
        volume,
        pitch,
        seed: 0,
    }))
}

fn resolve_sound_packet(packet: &mut C, lookup: impl FnOnce(&str) -> Option<i32>) {
    if let C::Sound(sound) = packet
        && let (_, Some((name, _))) = &sound.sound
        && let Some(id) = lookup(name)
    {
        sound.sound = (id, None);
    }
}

pub(super) fn resolve_native_sound(hub: &horizon_client::lan::hub::Hub, packet: &mut C) {
    resolve_sound_packet(packet, |name| {
        hub.registry_id("minecraft:sound_event", name)
    });
}

fn dimension(name: &str) -> Result<Dim, String> {
    match name {
        "minecraft:overworld" => Ok(Dim::Overworld),
        "minecraft:the_nether" => Ok(Dim::Nether),
        "minecraft:the_end" => Ok(Dim::End),
        _ => Err("unknown native player dimension".into()),
    }
}

impl Game<'_> {
    fn native_player_ops_reserve(&mut self, operation: Operation) -> Result<usize, String> {
        require_server_context(
            self.options.execution_role,
            self.remote.is_some(),
            false,
            self.lan.is_some() || self.lan_acting.is_some(),
        )?;
        let caller = horizon_client::crash::native_mod_call()
            .ok_or("native player operations require an active native mod call")?;
        let tick = self
            .native_lifecycle
            .entity_query_tick()
            .ok_or("native player operations require an active server tick")?;
        self.native_player_ops
            .reserve(tick, caller.index, operation)?;
        Ok(caller.index)
    }

    pub(super) fn native_player_target(&self, uuid: u128) -> Result<(Target, u32, Dim), String> {
        if self.lan_acting_uuid == Some(uuid)
            && let Some(conn) = self.lan_acting
            && self.player.is_some()
        {
            return Ok((Target::Player, conn, self.dim));
        }
        let host = self
            .lan
            .as_ref()
            .ok_or("native player server host is unavailable")?;
        let guest = host
            .hub
            .guests
            .iter()
            .find(|guest| guest.uuid == uuid && guest.placed && host.play.contains_key(&guest.conn))
            .ok_or("native player is disconnected or not ready")?;
        Ok((
            Target::Guest(guest.conn),
            guest.conn,
            Dim::from_id(&guest.dimension),
        ))
    }

    pub(super) fn for_native_player<T>(
        &mut self,
        target: Target,
        dimension: Dim,
        action: impl FnOnce(&mut Self) -> T,
    ) -> Result<T, String> {
        let previous = self.dim;
        if dimension != previous {
            let mut host = self
                .lan
                .take()
                .ok_or("native player server host is unavailable")?;
            let opened = self
                .enter_level(Some(&mut host), dimension)
                .map_err(|error| format!("could not open player level: {error}"));
            self.lan = Some(host);
            opened?;
        }
        let result = self
            .for_player(target, action)
            .ok_or_else(|| "could not enter the target player's state".to_owned());
        if self.dim != previous {
            let mut host = self
                .lan
                .take()
                .ok_or("native player server host is unavailable")?;
            let restored = self
                .enter_level(Some(&mut host), previous)
                .map_err(|error| format!("could not restore the previous level: {error}"));
            self.lan = Some(host);
            restored?;
        }
        result
    }

    pub(super) fn native_player_attribute_modifier_add_impl(
        &mut self,
        uuid: u128,
        attribute: &str,
        id: &str,
        amount: f64,
        operation: NativeAttributeOperation,
    ) -> Result<(), String> {
        self.native_player_ops_reserve(Operation::AttributeAdd)?;
        if !amount.is_finite() {
            return Err("native attribute modifier amount must be finite".into());
        }
        let modifier = Modifier::new(id, amount, AttributeOperation::AddValue)
            .map_err(|error| error.to_string())?;
        let attribute = attribute_name(attribute);
        let (target, _, dimension) = self.native_player_target(uuid)?;
        self.for_native_player(target, dimension, |game| {
            let player = game
                .player
                .as_mut()
                .ok_or("native player motion is unavailable")?;
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
                return Err("attribute is not available on this player".into());
            }
            if player
                .attributes
                .has_modifier(Some(key), modifier.id())
                .map_err(|error| format!("{error:?}"))?
            {
                return Err("attribute modifier already exists".into());
            }
            player
                .set_modifier(
                    attribute.as_ref(),
                    modifier.id(),
                    amount,
                    operation_name(operation),
                    true,
                )
                .map_err(|error| error.to_string())
        })?
    }

    pub(super) fn native_player_attribute_modifier_remove_impl(
        &mut self,
        uuid: u128,
        attribute: &str,
        id: &str,
    ) -> Result<bool, String> {
        self.native_player_ops_reserve(Operation::AttributeRemove)?;
        let modifier = Modifier::new(id, 0.0, AttributeOperation::AddValue)
            .map_err(|error| error.to_string())?;
        let attribute = attribute_name(attribute);
        let (target, _, dimension) = self.native_player_target(uuid)?;
        self.for_native_player(target, dimension, |game| {
            let player = game
                .player
                .as_mut()
                .ok_or("native player motion is unavailable")?;
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
                return Err("attribute is not available on this player".into());
            }
            if !player
                .attributes
                .has_modifier(Some(key), modifier.id())
                .map_err(|error| format!("{error:?}"))?
            {
                return Ok(false);
            }
            player
                .set_modifier(attribute.as_ref(), modifier.id(), 0.0, "add_value", false)
                .map_err(|error| error.to_string())?;
            Ok(true)
        })?
    }

    pub(super) fn native_player_effect_give_impl(
        &mut self,
        uuid: u128,
        effect: &str,
        duration_ticks: i32,
        amplifier: u8,
        ambient: bool,
        particles: bool,
        icon: bool,
    ) -> Result<bool, String> {
        self.native_player_ops_reserve(Operation::EffectGive)?;
        if !icon {
            return Err("independent effect icon visibility is not supported by Horizon".into());
        }
        if duration_ticks != -1 && duration_ticks <= 0 {
            return Err("effect duration must be positive or -1 for infinite".into());
        }
        let effect = brewing::effect(effect).ok_or("unknown player effect")?;
        let (target, _, dimension) = self.native_player_target(uuid)?;
        let mut instance = EffectInstance::new(effect.id, duration_ticks, i32::from(amplifier));
        instance.ambient = ambient;
        instance.visible = particles;
        self.for_native_player(target, dimension, |game| game.add_player_effect(instance))
    }

    pub(super) fn native_player_effect_clear_impl(
        &mut self,
        uuid: u128,
        effect: Option<&str>,
    ) -> Result<bool, String> {
        self.native_player_ops_reserve(Operation::EffectClear)?;
        let effect = effect
            .map(|id| brewing::effect(id).ok_or("unknown player effect"))
            .transpose()?;
        let (target, _, dimension) = self.native_player_target(uuid)?;
        self.for_native_player(target, dimension, |game| {
            let before = game.potions.effects.list.len();
            game.potions
                .effects
                .list
                .retain(|item| effect.is_some_and(|effect| item.id != effect.id));
            let changed = game.potions.effects.list.len() != before;
            if changed {
                game.apply_effect_status();
            }
            changed
        })
    }

    pub(super) fn native_player_teleport_impl(
        &mut self,
        uuid: u128,
        dimension_name: &str,
        pos: [f64; 3],
        yaw: f32,
        pitch: f32,
    ) -> Result<NativeTeleportTicket, String> {
        let owner = self.native_player_ops_reserve(Operation::Teleport)?;
        let destination = dimension(dimension_name)?;
        if !pos
            .into_iter()
            .all(|value| value.is_finite() && value.abs() <= 30_000_000.0)
            || !yaw.is_finite()
            || !pitch.is_finite()
        {
            return Err("invalid native player teleport position or rotation".into());
        }
        let (target, conn, source) = self.native_player_target(uuid)?;
        if let Some(previous) = self.native_player_ops.active.get(&conn).copied()
            && self
                .native_player_ops
                .tickets
                .get(&previous)
                .is_some_and(|record| record.phase == Phase::Arriving)
            && !self
                .server_trips
                .iter()
                .any(|trip| trip.native_ticket == Some(previous))
        {
            self.native_player_ops.finish(
                Some(previous),
                Some("native teleport was interrupted".into()),
            );
        }
        if self.level_teleports.iter().any(|trip| trip.conn == conn)
            || self.server_trips.iter().any(|trip| trip.is(conn))
        {
            return Err("the player already has a pending teleport".into());
        }
        let ticket = self.native_player_ops.ticket(owner, conn)?;
        if source == destination {
            self.native_player_ops.processing(Some(ticket));
            let result = self
                .for_native_player(target, source, |game| {
                    if !game.native_chunk_loaded(pos) {
                        return Err("the destination chunk is not loaded".into());
                    }
                    game.native_player_teleport_here(pos, yaw, pitch);
                    Ok(())
                })
                .and_then(|result| result);
            if let Err(error) = result {
                self.native_player_ops
                    .finish(Some(ticket), Some(error.clone()));
                return Err(error);
            }
            self.native_player_ops.finish(Some(ticket), None);
        } else {
            let index = self.level_teleports.len();
            self.level_teleports.push(LevelTeleport {
                conn,
                dim: destination,
                at: pos,
                rotation: Some([yaw, pitch.clamp(-90.0, 90.0)]),
            });
            self.native_player_ops.queued.insert(index, ticket);
        }
        Ok(ticket)
    }

    pub(super) fn native_player_teleport_status_impl(
        &mut self,
        ticket: NativeTeleportTicket,
    ) -> NativeTeleportStatus {
        let Ok(owner) = self.native_player_ops_reserve(Operation::TeleportStatus) else {
            return NativeTeleportStatus::Unknown;
        };
        let Some(record) = self.native_player_ops.tickets.get(&ticket) else {
            return NativeTeleportStatus::Unknown;
        };
        if record.owner != owner {
            return NativeTeleportStatus::Unknown;
        }
        if record.phase == Phase::Arriving
            && !self
                .server_trips
                .iter()
                .any(|trip| trip.native_ticket == Some(ticket))
        {
            self.native_player_ops
                .finish(Some(ticket), Some("native teleport was interrupted".into()));
        }
        self.native_player_ops
            .tickets
            .get(&ticket)
            .map_or(NativeTeleportStatus::Unknown, |record| {
                record.status.clone()
            })
    }

    pub(super) fn native_player_set_gamemode_impl(
        &mut self,
        uuid: u128,
        mode: NativeGameMode,
    ) -> Result<bool, String> {
        self.native_player_ops_reserve(Operation::GameMode)?;
        let mode = match mode {
            NativeGameMode::Survival => GameMode::Survival,
            NativeGameMode::Creative => GameMode::Creative,
            NativeGameMode::Adventure => GameMode::Adventure,
            NativeGameMode::Spectator => GameMode::Spectator,
        };
        let (target, _, dimension) = self.native_player_target(uuid)?;
        self.for_native_player(target, dimension, |game| game.set_game_mode(mode))
    }

    pub(super) fn native_player_set_health_impl(&mut self, player: u128, health: f32) -> bool {
        if self.native_player_ops_reserve(Operation::Health).is_err() || !health.is_finite() {
            return false;
        }
        let Ok((target, _, dimension)) = self.native_player_target(player) else {
            return false;
        };
        self.for_native_player(target, dimension, |game| {
            game.player
                .as_mut()
                .is_some_and(|motion| motion.set_health(health, false).is_ok())
        })
        .unwrap_or(false)
    }

    pub(super) fn native_player_clear_inventory_impl(&mut self, player: u128) -> bool {
        if self
            .native_player_ops_reserve(Operation::Inventory)
            .is_err()
        {
            return false;
        }
        let Ok((target, conn, dimension)) = self.native_player_target(player) else {
            return false;
        };
        self.for_native_player(target, dimension, |game| {
            clear_player_slots(
                &mut game.inventory,
                &mut game.armor,
                &mut game.offhand,
                &mut game.carried,
            );
            game.native_player_ops.inventory_dirty.insert(conn);
            true
        })
        .unwrap_or(false)
    }

    fn native_player_packet(&mut self, target: Target, packet: C) -> bool {
        match target {
            Target::Player if self.lan_acting.is_some() => {
                self.guest_packets.push(packet);
                true
            }
            Target::Guest(conn) => {
                if let Some(host) = self.lan.as_ref() {
                    host.hub.send_to(conn, &packet);
                    true
                } else {
                    false
                }
            }
            _ => false,
        }
    }

    pub(super) fn native_player_title_impl(
        &mut self,
        player: u128,
        title: Option<&str>,
        subtitle: Option<&str>,
        fade_in: i32,
        stay: i32,
        fade_out: i32,
    ) -> bool {
        if self.native_player_ops_reserve(Operation::Title).is_err() {
            return false;
        }
        let Ok((target, _, _)) = self.native_player_target(player) else {
            return false;
        };
        let Some(packets) = title_packets(title, subtitle, fade_in, stay, fade_out) else {
            return false;
        };
        for packet in packets {
            if !self.native_player_packet(target, packet) {
                return false;
            }
        }
        true
    }

    pub(super) fn native_player_clear_title_impl(&mut self, player: u128) -> bool {
        if self
            .native_player_ops_reserve(Operation::ClearTitle)
            .is_err()
        {
            return false;
        }
        let Ok((target, _, _)) = self.native_player_target(player) else {
            return false;
        };
        self.native_player_packet(target, clear_title_packet())
    }

    pub(super) fn native_bossbar_create_impl(&mut self, id: &str, name: &str) -> bool {
        let Ok(owner) = self.native_player_ops_reserve(Operation::BossCreate) else {
            return false;
        };
        self.native_player_ops.create_bar(owner, id, name)
    }

    pub(super) fn native_bossbar_set_name_impl(&mut self, id: &str, name: &str) -> bool {
        let Ok(owner) = self.native_player_ops_reserve(Operation::BossName) else {
            return false;
        };
        self.native_player_ops.set_bar_name(owner, id, name)
    }

    pub(super) fn native_bossbar_set_progress_impl(&mut self, id: &str, progress: f32) -> bool {
        let Ok(owner) = self.native_player_ops_reserve(Operation::BossProgress) else {
            return false;
        };
        self.native_player_ops.set_bar_progress(owner, id, progress)
    }

    pub(super) fn native_bossbar_set_color_impl(&mut self, id: &str, color: &str) -> bool {
        let Ok(owner) = self.native_player_ops_reserve(Operation::BossColor) else {
            return false;
        };
        self.native_player_ops.set_bar_color(owner, id, color)
    }

    pub(super) fn native_bossbar_set_overlay_impl(&mut self, id: &str, overlay: &str) -> bool {
        let Ok(owner) = self.native_player_ops_reserve(Operation::BossOverlay) else {
            return false;
        };
        self.native_player_ops.set_bar_overlay(owner, id, overlay)
    }

    pub(super) fn native_bossbar_set_players_impl(&mut self, id: &str, players: &[u128]) -> bool {
        let Ok(owner) = self.native_player_ops_reserve(Operation::BossPlayers) else {
            return false;
        };
        self.native_player_ops.set_bar_players(owner, id, players)
    }

    pub(super) fn native_bossbar_remove_impl(&mut self, id: &str) -> bool {
        let Ok(owner) = self.native_player_ops_reserve(Operation::BossRemove) else {
            return false;
        };
        self.native_player_ops.remove_bar(owner, id)
    }

    pub(super) fn native_player_play_sound_impl(
        &mut self,
        player: u128,
        sound: &str,
        category: NativeSoundCategory,
        x: f64,
        y: f64,
        z: f64,
        volume: f32,
        pitch: f32,
    ) -> bool {
        if self.native_player_ops_reserve(Operation::Sound).is_err() {
            return false;
        }
        let Ok((target, _, _)) = self.native_player_target(player) else {
            return false;
        };
        let Some(mut packet) = sound_packet(sound, category, [x, y, z], volume, pitch) else {
            return false;
        };
        match target {
            Target::Player if self.lan_acting.is_some() => {
                self.native_player_ops
                    .sound_packet_indices
                    .push(self.guest_packets.len());
                self.guest_packets.push(packet);
                true
            }
            Target::Guest(conn) => {
                if let Some(host) = self.lan.as_ref() {
                    resolve_native_sound(&host.hub, &mut packet);
                    host.hub.send_to(conn, &packet);
                    true
                } else {
                    false
                }
            }
            _ => false,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn tick(epoch: u64, tick: u64) -> (NativeWorldEpoch, NativeTickId) {
        (NativeWorldEpoch::new(epoch), NativeTickId::new(tick))
    }

    struct TestHealth {
        value: f32,
        maximum: f64,
    }

    impl horizon_world::living_health::HealthWorld for TestHealth {
        type Error = ();
        type Data = ();

        fn health(&mut self) -> Result<f32, Self::Error> {
            Ok(self.value)
        }

        fn health_data(&self) -> Self::Data {}

        fn max_health_attribute(&mut self) -> Result<f64, Self::Error> {
            Ok(self.maximum)
        }

        fn max_absorption_attribute(&mut self) -> Result<f64, Self::Error> {
            Ok(0.0)
        }

        fn store_health(&mut self, _: Self::Data, value: f32) -> Result<(), Self::Error> {
            self.value = value;
            Ok(())
        }

        fn store_absorption(&mut self, _: f32) -> Result<(), Self::Error> {
            Ok(())
        }

        fn dead_flag(&self) -> bool {
            false
        }

        fn dispatch_set_health(&mut self, value: f32) -> Result<(), Self::Error> {
            self.value = value;
            Ok(())
        }
    }

    #[test]
    fn health_is_unchanged_without_a_call_and_clamped_with_one() {
        let mut health = TestHealth {
            value: 12.0,
            maximum: 20.0,
        };
        assert_eq!(health.value, 12.0);
        horizon_world::living_health::set_health(&mut health, 40.0).unwrap();
        assert_eq!(health.value, 20.0);
        horizon_world::living_health::set_health(&mut health, -1.0).unwrap();
        assert_eq!(health.value, 0.0);
        assert!(horizon_world::living_health::is_dead_or_dying(&mut health).unwrap());
    }

    #[test]
    fn inventory_is_unchanged_without_a_call_and_cleared_with_one() {
        let mut inventory = [Some(1), Some(2)];
        let mut armor = [Some(3), None];
        let mut offhand = Some(4);
        let mut carried = Some(5);
        assert_eq!(inventory, [Some(1), Some(2)]);
        assert_eq!(
            (armor, offhand, carried),
            ([Some(3), None], Some(4), Some(5))
        );
        clear_player_slots(&mut inventory, &mut armor, &mut offhand, &mut carried);
        assert_eq!(inventory, [None, None]);
        assert_eq!(armor, [None, None]);
        assert_eq!((offhand, carried), (None, None));
    }

    #[test]
    fn title_and_clear_packets_are_only_created_on_request() {
        let untouched = Vec::<C>::new();
        assert!(untouched.is_empty());
        let packets = title_packets(Some("Title"), Some("Subtitle"), 10, 70, 20).unwrap();
        assert_eq!(packets.len(), 3);
        let C::SetTitlesAnimation(animation) = &packets[0] else {
            panic!("expected title animation packet");
        };
        assert_eq!(
            (animation.fade_in, animation.stay, animation.fade_out),
            (10, 70, 20)
        );
        assert!(matches!(&packets[1], C::SetSubtitleText(_)));
        assert!(matches!(&packets[2], C::SetTitleText(_)));
        assert_eq!(title_packets(None, None, 0, 0, 0).unwrap().len(), 1);
        assert!(title_packets(Some(&"a".repeat(32768)), None, 0, 0, 0).is_none());
        assert!(matches!(
            clear_title_packet(),
            C::ClearTitles(play::Bool1 { value: false })
        ));
    }

    #[test]
    fn boss_bars_are_namespaced_and_change_only_on_request() {
        let mut state = PlayerOpsState::default();
        assert!(state.bars.is_empty());
        assert!(state.reserve(tick(1, 1), 0, Operation::BossCreate).is_ok());
        assert!(state.create_bar(0, "test:bar", "First"));
        assert!(state.bars[0].players.is_empty());
        assert_eq!(state.bars[0].progress, 0.0);
        assert_eq!((state.bars[0].color, state.bars[0].overlay), (6, 0));
        assert!(!state.create_bar(0, "test:bar", "Duplicate"));
        assert!(state.create_bar(1, "test:bar", "Second"));
        assert_ne!(state.bars[0].uuid, state.bars[1].uuid);
        assert!(!state.set_bar_name(2, "test:bar", "No access"));
        assert!(state.set_bar_name(0, "test:bar", "Changed"));
        assert_eq!(state.bars[0].name, "Changed");
        assert!(!state.set_bar_progress(0, "test:bar", f32::NAN));
        assert!(state.set_bar_progress(0, "test:bar", 2.0));
        assert_eq!(state.bars[0].progress, 1.0);
        assert!(state.set_bar_progress(0, "test:bar", -1.0));
        assert_eq!(state.bars[0].progress, 0.0);
        assert!(!state.set_bar_color(0, "test:bar", "invalid"));
        assert!(!state.set_bar_overlay(0, "test:bar", "invalid"));
        assert!(!state.set_bar_color(2, "test:bar", "red"));
        assert!(!state.set_bar_overlay(2, "test:bar", "notched_10"));
        assert!(state.set_bar_color(0, "test:bar", "red"));
        assert!(state.set_bar_overlay(0, "test:bar", "notched_10"));
        assert_eq!((state.bars[0].color, state.bars[0].overlay), (2, 2));
        assert!(state.set_bar_players(0, "test:bar", &[11, 22]));
        assert_eq!(state.bars[0].players, vec![11, 22]);
        assert!(!state.set_bar_players(0, "test:bar", &vec![1; 1025]));
        assert!(!state.remove_bar(2, "test:bar"));
        assert!(state.remove_bar(0, "test:bar"));
        assert_eq!(state.bars.len(), 1);
        assert!(state.reserve(tick(2, 1), 0, Operation::BossCreate).is_ok());
        assert!(state.bars.is_empty());
    }

    #[test]
    fn boss_bar_count_is_bounded_per_mod() {
        let mut state = PlayerOpsState::default();
        for index in 0..16 {
            assert!(state.create_bar(0, &format!("test:bar_{index}"), ""));
        }
        assert!(!state.create_bar(0, "test:overflow", ""));
        assert!(state.create_bar(1, "test:overflow", ""));
    }

    #[test]
    fn boss_bar_identifiers_use_the_default_namespace() {
        let mut state = PlayerOpsState::default();
        assert!(state.create_bar(0, "local", "First"));
        assert!(!state.create_bar(0, "minecraft:local", "Duplicate"));
        assert!(state.set_bar_name(0, "minecraft:local", "Changed"));
        assert!(state.remove_bar(0, "local"));
    }

    #[test]
    fn sound_packet_is_fixed_seed_and_only_created_on_request() {
        let untouched = Vec::<C>::new();
        assert!(untouched.is_empty());
        let Some(C::Sound(packet)) = sound_packet(
            "minecraft:entity.player.levelup",
            NativeSoundCategory::Players,
            [1.0, -2.0, 0.5],
            0.5,
            1.25,
        ) else {
            panic!("expected sound packet");
        };
        assert_eq!(packet.sound.0, -1);
        assert_eq!(packet.source, 7);
        assert_eq!((packet.x, packet.y, packet.z), (8, -16, 4));
        assert_eq!((packet.volume, packet.pitch, packet.seed), (0.5, 1.25, 0));
        assert!(sound_packet("bad:", NativeSoundCategory::Master, [0.0; 3], 1.0, 1.0).is_none());
        assert!(
            sound_packet(
                "test:sound",
                NativeSoundCategory::Master,
                [f64::NAN, 0.0, 0.0],
                1.0,
                1.0
            )
            .is_none()
        );
        let mut registered = sound_packet(
            "entity.player.levelup",
            NativeSoundCategory::Players,
            [0.0; 3],
            1.0,
            1.0,
        )
        .unwrap();
        resolve_sound_packet(&mut registered, |name| {
            assert_eq!(name, "minecraft:entity.player.levelup");
            Some(42)
        });
        let C::Sound(registered) = registered else {
            panic!("expected sound packet");
        };
        assert_eq!(registered.sound, (42, None));
    }

    #[test]
    fn budgets_are_per_helper_mod_and_tick() {
        let mut state = PlayerOpsState::default();
        for _ in 0..MAX_CALLS {
            assert!(state.reserve(tick(1, 1), 0, Operation::Teleport).is_ok());
        }
        assert!(state.reserve(tick(1, 1), 0, Operation::Teleport).is_err());
        assert!(state.reserve(tick(1, 1), 0, Operation::EffectGive).is_ok());
        assert!(state.reserve(tick(1, 1), 1, Operation::Teleport).is_ok());
        assert!(state.reserve(tick(1, 2), 0, Operation::Teleport).is_ok());
    }

    #[test]
    fn new_helpers_each_have_a_separate_budget() {
        let operations = [
            Operation::Health,
            Operation::Inventory,
            Operation::Title,
            Operation::ClearTitle,
            Operation::BossCreate,
            Operation::BossName,
            Operation::BossProgress,
            Operation::BossPlayers,
            Operation::BossRemove,
            Operation::Sound,
            Operation::BossColor,
            Operation::BossOverlay,
        ];
        let mut state = PlayerOpsState::default();
        for operation in operations {
            for _ in 0..MAX_CALLS {
                assert!(state.reserve(tick(1, 1), 0, operation).is_ok());
            }
            assert!(state.reserve(tick(1, 1), 0, operation).is_err());
        }
    }

    #[test]
    fn attribute_names_default_to_minecraft_namespace() {
        assert_eq!(attribute_name("movement_speed"), "minecraft:movement_speed");
        assert_eq!(attribute_name("other:gravity"), "other:gravity");
    }

    #[test]
    fn teleport_ticket_tracks_completion_and_failure() {
        let mut state = PlayerOpsState::default();
        let ticket = state.ticket(3, 7).unwrap();
        assert!(state.ticket(3, 7).is_err());
        assert_eq!(state.tickets[&ticket].owner, 3);
        assert!(matches!(
            &state.tickets[&ticket].status,
            NativeTeleportStatus::Pending
        ));
        state.processing(Some(ticket));
        assert!(state.tickets[&ticket].phase == Phase::Processing);
        state.arriving(Some(ticket));
        assert!(state.tickets[&ticket].phase == Phase::Arriving);
        state.finish(Some(ticket), Some("failed".into()));
        state.finish(Some(ticket), None);
        assert!(matches!(
            &state.tickets[&ticket].status,
            NativeTeleportStatus::Failed(message) if message == "failed"
        ));
        let completed = state.ticket(3, 7).unwrap();
        state.finish(Some(completed), None);
        assert!(matches!(
            &state.tickets[&completed].status,
            NativeTeleportStatus::Done
        ));
    }

    #[test]
    fn new_world_invalidates_old_tickets_without_reusing_ids() {
        let mut state = PlayerOpsState::default();
        state.reserve(tick(1, 1), 0, Operation::Teleport).unwrap();
        let old = state.ticket(0, 7).unwrap();
        state.queued.insert(0, old);
        state.reserve(tick(2, 1), 0, Operation::Teleport).unwrap();
        assert!(!state.tickets.contains_key(&old));
        assert!(state.queued.is_empty());
        assert_ne!(old, state.ticket(0, 7).unwrap());
    }
}
