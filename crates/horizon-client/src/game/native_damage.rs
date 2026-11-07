use std::cell::Cell;

use horizon_client::{
    advancements::EntityView,
    items::ItemStack,
    mobs::{Damage, DamageKind, Who},
    mod_api::{NativeTickId, NativeWorldEpoch},
    text::Text,
};
use horizon_core::geometry::Vec3;

use super::{Game, commands::Target, lan::LanHost, native_entity_query::require_server_context};

#[cfg(test)]
mod tests;

const MAX_OPERATIONS: u16 = 256;
const MAX_EXPLOSIONS: u8 = 4;
const MAX_DELIVERY_DEPTH: u8 = 4;

#[non_exhaustive]
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum NativeDamageKind {
    Magic,
    PlayerAttack,
    Explosion,
}

#[non_exhaustive]
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum NativeDamageActor {
    ActingPlayer,
    Player(u128),
    None,
}

#[non_exhaustive]
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct NativeDamage {
    pub kind: NativeDamageKind,
    pub actor: NativeDamageActor,
    pub position: Option<[f64; 3]>,
}

impl NativeDamage {
    pub const fn new(
        kind: NativeDamageKind,
        actor: NativeDamageActor,
        position: Option<[f64; 3]>,
    ) -> Self {
        Self {
            kind,
            actor,
            position,
        }
    }
}

#[derive(Default)]
struct DamageUsage {
    mod_index: usize,
    operations: u16,
    explosions: u8,
}

#[derive(Default)]
pub(super) struct DamageBudgets {
    tick: Option<(NativeWorldEpoch, NativeTickId)>,
    mods: Vec<DamageUsage>,
}

thread_local! {
    static DELIVERY_DEPTH: Cell<u8> = const { Cell::new(0) };
}

pub(super) struct DeliveryGuard;

impl Drop for DeliveryGuard {
    fn drop(&mut self) {
        DELIVERY_DEPTH.with(|depth| depth.set(depth.get() - 1));
    }
}

pub(super) struct PlayerDamageSource {
    pub(super) id: i32,
    pub(super) direct: i32,
    pub(super) entity: Option<EntityView>,
}

struct Actor {
    conn: u32,
    id: i32,
    position: [f64; 3],
    name: Text,
    entity: EntityView,
    held: [Option<ItemStack>; 2],
    item: Option<Text>,
    disable_blocking: f32,
}

#[derive(Clone, Copy)]
enum Victim {
    Mob(u32),
    Player(u32),
}

impl NativeDamageKind {
    fn damage_type(self, has_actor: bool) -> &'static str {
        match self {
            Self::Magic => "minecraft:magic",
            Self::PlayerAttack => "minecraft:player_attack",
            Self::Explosion if has_actor => "minecraft:player_explosion",
            Self::Explosion => "minecraft:explosion",
        }
    }

    fn mob_kind(self) -> DamageKind {
        match self {
            Self::Magic => DamageKind::Magic,
            Self::PlayerAttack => DamageKind::PlayerAttack,
            Self::Explosion => DamageKind::Explosion,
        }
    }
}

impl DamageBudgets {
    fn reserve(
        &mut self,
        tick: (NativeWorldEpoch, NativeTickId),
        mod_index: usize,
        explosion: bool,
    ) -> Result<(), String> {
        if self.tick != Some(tick) {
            self.mods.clear();
            self.tick = Some(tick);
        }
        let usage = if let Some(index) = self
            .mods
            .iter()
            .position(|usage| usage.mod_index == mod_index)
        {
            &mut self.mods[index]
        } else {
            self.mods.push(DamageUsage {
                mod_index,
                ..Default::default()
            });
            self.mods.last_mut().unwrap()
        };
        if explosion {
            if usage.explosions >= MAX_EXPLOSIONS {
                return Err("native explosion limit exceeded (4 per mod per server tick)".into());
            }
            usage.explosions += 1;
        } else {
            if usage.operations >= MAX_OPERATIONS {
                return Err("native hurt/push limit exceeded (256 per mod per server tick)".into());
            }
            usage.operations += 1;
        }
        Ok(())
    }
}

impl DeliveryGuard {
    fn enter() -> Result<Self, String> {
        DELIVERY_DEPTH.with(|depth| {
            if depth.get() >= MAX_DELIVERY_DEPTH {
                return Err("native damage delivery depth exceeded (4)".into());
            }
            depth.set(depth.get() + 1);
            Ok(Self)
        })
    }
}

fn player_ready(host: &LanHost, conn: u32, dimension: &str) -> bool {
    host.hub.guest(conn).is_some_and(|guest| {
        guest.dimension == dimension
            && guest.placed
            && guest.motion.is_some()
            && host.play.contains_key(&conn)
    })
}

impl Game<'_> {
    fn native_damage_context(&self, dimension: &str) -> Result<(), String> {
        require_server_context(
            self.options.execution_role,
            self.remote.is_some(),
            self.lan_acting.is_some(),
            self.lan.is_some(),
        )?;
        if dimension != self.dim.id() {
            return Err("native damage requires the current server dimension".into());
        }
        Ok(())
    }

    pub(super) fn native_damage_reserve(
        &mut self,
        dimension: &str,
        explosion: bool,
    ) -> Result<DeliveryGuard, String> {
        self.native_damage_context(dimension)?;
        let caller = horizon_client::crash::native_mod_call()
            .ok_or("native damage requires an active native mod call")?;
        let tick = self
            .native_lifecycle
            .entity_query_tick()
            .ok_or("native damage requires a ready server world")?;
        let guard = DeliveryGuard::enter()?;
        self.native_damage_budgets
            .reserve(tick, caller.index, explosion)?;
        Ok(guard)
    }

    fn native_damage_player_conn(&self, dimension: &str, player: u128) -> Result<u32, String> {
        let host = self
            .lan
            .as_ref()
            .ok_or("native damage requires the server host")?;
        let guest = host
            .hub
            .guests
            .iter()
            .find(|guest| guest.uuid == player)
            .ok_or("native damage player is disconnected or unknown")?;
        if !player_ready(host, guest.conn, dimension) {
            return Err("native damage player is not ready in the current dimension".into());
        }
        Ok(guest.conn)
    }

    fn native_damage_victim(&self, dimension: &str, id: u32) -> Result<Victim, String> {
        let host = self
            .lan
            .as_ref()
            .ok_or("native damage requires the server host")?;
        if let Some(guest) = host
            .hub
            .guests
            .iter()
            .find(|guest| u32::try_from(guest.entity_id) == Ok(id))
        {
            return if player_ready(host, guest.conn, dimension) {
                Ok(Victim::Player(guest.conn))
            } else {
                Err("native damage target player is not ready in the current dimension".into())
            };
        }
        let target = i32::try_from(id)
            .ok()
            .and_then(|id| self.level_entity(id))
            .ok_or("native damage entity is unknown in the current dimension")?;
        match target {
            Target::Mob(id) => Ok(Victim::Mob(id)),
            target => Err(format!(
                "unsupported entity kind {}",
                self.target_type(target)
            )),
        }
    }

    fn native_damage_actor(&mut self, host: &mut LanHost, conn: u32) -> Result<Actor, String> {
        let guest = host
            .hub
            .guest(conn)
            .ok_or("native damage actor disconnected")?;
        let name = self.player_text(&guest.name, guest.uuid);
        self.as_guest(host, conn, |game| {
            let player = game.player.as_ref()?;
            Some(Actor {
                conn,
                id: player.entity_id,
                position: [player.position.x, player.position.y, player.position.z],
                name,
                entity: game.player_entity(),
                held: [
                    game.inventory.get(game.selected).cloned().flatten(),
                    game.offhand.clone(),
                ],
                item: game.main_hand_custom_name(),
                disable_blocking: if game.using.is_some() {
                    0.0
                } else {
                    game.attack_item_info()
                        .map_or(0.0, |item| item.disable_blocking_seconds)
                },
            })
        })
        .flatten()
        .ok_or_else(|| "native damage actor cannot be entered".into())
    }

    /// @API:entity_damage@
    pub(super) fn native_hurt_entity(
        &mut self,
        dimension: &str,
        id: u32,
        amount: f32,
        damage: &NativeDamage,
    ) -> Result<bool, String> {
        self.native_damage_context(dimension)?;
        if !amount.is_finite() || amount < 0.0 {
            return Err("native damage amount must be finite and nonnegative".into());
        }
        if damage
            .position
            .is_some_and(|position| !position.into_iter().all(f64::is_finite))
        {
            return Err("native damage position must be finite".into());
        }
        let victim = self.native_damage_victim(dimension, id)?;
        let actor = match damage.actor {
            NativeDamageActor::None => None,
            NativeDamageActor::Player(player) => {
                Some(self.native_damage_player_conn(dimension, player)?)
            }
            NativeDamageActor::ActingPlayer => {
                let player = self
                    .lan
                    .as_ref()
                    .and_then(|host| host.owner.as_ref())
                    .ok_or("native damage has no acting server player; use Player(uuid)")?
                    .uuid;
                Some(self.native_damage_player_conn(dimension, player)?)
            }
        };
        if self
            .motion
            .health
            .damage
            .id(damage.kind.damage_type(actor.is_some()))
            .is_none()
        {
            return Err("native damage type is not registered".into());
        }
        let _delivery = self.native_damage_reserve(dimension, false)?;
        let conn = match victim {
            Victim::Mob(id) => {
                return if let Some(conn) = actor {
                    self.for_player(Target::Guest(conn), |game| {
                        game.native_damage_mob(id, amount, damage, true)
                    })
                    .ok_or("native damage actor cannot be entered")?
                } else {
                    self.native_damage_mob(id, amount, damage, false)
                };
            }
            Victim::Player(conn) => conn,
        };
        let mut host = self
            .lan
            .take()
            .ok_or("native damage requires the server host")?;
        let result = (|| {
            let actor = actor
                .map(|conn| self.native_damage_actor(&mut host, conn))
                .transpose()?;
            self.native_damage_player(&mut host, conn, amount, damage, actor.as_ref())
        })();
        self.lan = Some(host);
        result
    }

    fn native_damage_mob(
        &mut self,
        id: u32,
        amount: f32,
        damage: &NativeDamage,
        by_player: bool,
    ) -> Result<bool, String> {
        let (causing, position, held) = if by_player {
            let player = self
                .player
                .as_ref()
                .ok_or("native damage actor cannot be entered")?;
            (
                player.entity_id,
                Some([player.position.x, player.position.y, player.position.z]),
                [
                    self.inventory.get(self.selected).cloned().flatten(),
                    self.offhand.clone(),
                ],
            )
        } else {
            (0, None, [None, None])
        };
        let _holding = horizon_client::health::knockback_from_holding(causing, held);
        let hurt = self.command_damage_mob(
            id,
            amount,
            Damage {
                kind: damage.kind.mob_kind(),
                attacker: by_player.then_some(Who::Player),
                from: damage.position.or(position),
            },
        );
        if hurt {
            if by_player && let Some(host) = self.lan.as_mut() {
                if let Some(conn) = self.lan_acting {
                    host.mob_players.insert(id, conn);
                } else {
                    host.mob_players.remove(&id);
                }
            }
            self.finish_native_mob_damage(id, by_player);
        }
        Ok(hurt)
    }

    fn native_damage_player(
        &mut self,
        host: &mut LanHost,
        conn: u32,
        amount: f32,
        damage: &NativeDamage,
        actor: Option<&Actor>,
    ) -> Result<bool, String> {
        if let Some(actor) = actor {
            let victim = host
                .hub
                .guest(conn)
                .ok_or("native damage victim disconnected")?;
            let attacker = host
                .hub
                .guest(actor.conn)
                .ok_or("native damage actor disconnected")?;
            let victim_team = self.score.board.players_team(&victim.name);
            let actor_team = self.score.board.players_team(&attacker.name);
            if !self.ui.rules.bool("pvp")
                || !horizon_client::scoreboard::teams_allow_damage(victim_team, actor_team)
            {
                return Ok(false);
            }
        }
        let damage_type = damage.kind.damage_type(actor.is_some());
        let actor_id = actor.map_or(0, |actor| actor.id);
        let source = PlayerDamageSource {
            id: actor_id,
            direct: if damage.kind == NativeDamageKind::Explosion {
                0
            } else {
                actor_id
            },
            entity: actor.map(|actor| actor.entity.clone()),
        };
        let from = damage
            .position
            .or_else(|| actor.map(|actor| actor.position));
        let incoming = self.guest_combat.entry(conn).or_default().take_incoming();
        let land = |game: &mut Self| {
            if game.spectator() {
                return (false, Vec::new());
            }
            let _holding = horizon_client::health::knockback_from_holding(
                actor_id,
                actor.map_or([None, None], |actor| actor.held.clone()),
            );
            if let Some(actor) = actor {
                game.note_player_attacker(
                    damage_type,
                    actor.id as u32,
                    actor.name.clone(),
                    actor.item.clone(),
                );
                if damage.kind == NativeDamageKind::PlayerAttack {
                    game.combat.player_attacker = Some((actor.position, actor.disable_blocking));
                }
            }
            let hurt = game.hurt_player_from_entities(
                amount,
                from,
                damage_type,
                Some(None),
                Some(&source),
            );
            game.combat.player_attacker = None;
            let thorns = if hurt && actor.is_some() {
                game.thorns_damage()
            } else {
                Vec::new()
            };
            (hurt, thorns)
        };
        let result = if damage.kind == NativeDamageKind::PlayerAttack {
            self.guest_struck_by(host, conn, land)
        } else {
            self.guest_hit_by(host, conn, land)
        };
        self.guest_combat
            .entry(conn)
            .or_default()
            .restore_incoming(incoming);
        let (hurt, thorns) = result.ok_or("native damage victim cannot be entered")?;
        if let Some(actor) = actor {
            if hurt
                && host
                    .hub
                    .guest(conn)
                    .and_then(|guest| guest.motion.as_deref())
                    .is_some_and(|motion| {
                        motion.health.dead_or_dying() && !motion.health.totem_used
                    })
            {
                self.as_guest(host, actor.conn, |game| {
                    game.award_custom_stat("player_kills", 1);
                });
            }
            let cause = self.player_attacker(host, Some(conn));
            for amount in thorns {
                let incoming = self
                    .guest_combat
                    .entry(actor.conn)
                    .or_default()
                    .take_incoming();
                self.guest_hit_by(host, actor.conn, |game| {
                    if let Some((id, name)) = &cause {
                        game.note_named_attacker("minecraft:thorns", *id, name.clone());
                    }
                    game.hurt_player_by("minecraft:thorns", amount)
                });
                self.guest_combat
                    .entry(actor.conn)
                    .or_default()
                    .restore_incoming(incoming);
            }
        }
        Ok(hurt)
    }

    /// @API:server_player_impulse@
    pub(super) fn native_push_server_player(
        &mut self,
        dimension: &str,
        player: u128,
        impulse: [f64; 3],
    ) -> Result<bool, String> {
        self.native_damage_context(dimension)?;
        if !impulse.into_iter().all(f64::is_finite) {
            return Err("native player impulse must be finite".into());
        }
        let conn = self.native_damage_player_conn(dimension, player)?;
        let velocity = self
            .lan
            .as_ref()
            .and_then(|host| host.hub.guest(conn))
            .and_then(|guest| guest.motion.as_deref())
            .ok_or("native player motion is unavailable")?
            .velocity;
        let pushed = velocity.add_vector(Vec3::new(impulse[0], impulse[1], impulse[2]));
        if ![pushed.x, pushed.y, pushed.z]
            .into_iter()
            .all(f64::is_finite)
        {
            return Err("native player impulse would overflow velocity".into());
        }
        let _delivery = self.native_damage_reserve(dimension, false)?;
        let mut host = self
            .lan
            .take()
            .ok_or("native push requires the server host")?;
        let result = self.guest_hit_by(&mut host, conn, |game| {
            if game.spectator() {
                return false;
            }
            let Some(player) = game.player.as_mut() else {
                return false;
            };
            if player.health.dead_or_dying() {
                return false;
            }
            player.velocity = pushed;
            true
        });
        self.lan = Some(host);
        result.ok_or_else(|| "native push player cannot be entered".into())
    }
}
