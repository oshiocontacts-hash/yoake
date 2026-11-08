//! Authoritative, tick-bounded operations on server players.

use std::{borrow::Cow, collections::HashMap};

use horizon_client::{
    brewing,
    command::args::GameMode,
    effects::EffectInstance,
    mod_api::{
        NativeAttributeOperation, NativeGameMode, NativeTeleportStatus, NativeTeleportTicket,
        NativeTickId, NativeWorldEpoch,
    },
};
use horizon_world::attributes::{Modifier, Operation as AttributeOperation};

use super::{
    Dim, Game, commands::Target, native_entity_query::require_server_context,
    server_travel::LevelTeleport,
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
        }
    }
}

struct Usage {
    mod_index: usize,
    calls: [u16; 7],
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
}

impl PlayerOpsState {
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
                    calls: [0; 7],
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

fn attribute_name(attribute: &str) -> Cow<'_, str> {
    if attribute.contains(':') {
        Cow::Borrowed(attribute)
    } else {
        Cow::Owned(format!("minecraft:{attribute}"))
    }
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

    fn native_player_target(&self, uuid: u128) -> Result<(Target, u32, Dim), String> {
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

    fn for_native_player<T>(
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
            if let Err(error) = self.for_native_player(target, source, |game| {
                game.native_player_teleport_here(pos, yaw, pitch);
            }) {
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
}

#[cfg(test)]
mod tests {
    use super::*;

    fn tick(epoch: u64, tick: u64) -> (NativeWorldEpoch, NativeTickId) {
        (NativeWorldEpoch::new(epoch), NativeTickId::new(tick))
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
