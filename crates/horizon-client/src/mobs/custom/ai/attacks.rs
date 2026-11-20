//! Attack goals of a custom mob (FR-F4-MOBS.md section 5): `melee_attack`, `area_attack`,
//! `ranged_attack`, `leap_attack`, every one with an optional trigger animation and damage
//! frame.
//!
//! # Trigger animation and damage frame
//!
//! A goal row may carry
//!
//! * `"trigger": "<name>"`: the animation trigger to play when the attack starts. The server
//!   has no animation runtime in this item, so the goal only *requests* it: the name goes to the
//!   mob's [`AttackState`] outbox, which the animation sync of F4M-5b drains with
//!   [`take_triggers`] and plays on the mob's synced trigger clock.
//! * `"damage_frame": "<name>"`: the name of the server timeline event (a `Custom` event of
//!   an animation flagged `server_events`, FR-F4M-9) at which the blow lands. The engine
//!   part of F4M-9 hands each such event to [`deliver_frame`]; the goal applies the damage (or
//!   fires the projectile) when the event with its name arrives.
//! * `"damage_delay": <ticks>`: the fallback. If the event has not arrived this many ticks after
//!   the attack began, the blow lands anyway. Without `damage_frame` it is the plain delay
//!   (default 0: the blow lands at once, as vanilla's goals do). With `damage_frame` it defaults
//!   to [`DEFAULT_FRAME_DELAY`], and it is also the whole mechanism while the server runs no
//!   animations (nothing calls [`deliver_frame`]): set it to the keyframe's time in ticks. An
//!   author who sets both must make the delay at least the keyframe time, or the fallback wins.
//!
//! A blow that is pending when the goal stops (the target died, a higher-priority goal took over)
//! is dropped. When the blow lands the target must still be alive and in reach.
//!
//! `melee_attack` and `area_attack` chase their target with the engine's `MeleeAttack` (pathing,
//! recalculation, looking), whose own strike is held off; the strike is the goal's. A
//! `melee_attack` with no `trigger`, `damage_frame` or `damage_delay` is vanilla's
//! `MeleeAttackGoal` tick for tick.
use super::{Params, species_list};
use crate::mobs::{
    Damage, Mob, MobEvent, Species, Who,
    goals::{Cx, Goal, JUMP, LOOK, MOVE, MeleeAttack, goal_clone, reduced},
    world::TargetInfo,
};
use crate::projectiles::{Projectile, ProjectileKind};
use horizon_core::{geometry::Vec3, random::RandomSource};
use horizon_mods::custom_mob::Goal as GoalDef;

/// The damage delay of an attack that names a `damage_frame` and no `damage_delay`.
pub const DEFAULT_FRAME_DELAY: i32 = 10;
/// How many undelivered frame names or requested triggers a mob keeps (the oldest are dropped).
const KEPT: usize = 8;
/// What the wrapped `MeleeAttack`'s attack timer is held at so it never strikes on its own.
const HOLD: i32 = 1 << 20;

/// The animation side of a custom mob's attacks (`CustomState.attack`).
#[derive(Clone, Debug, Default, PartialEq)]
pub struct AttackState {
    /// Server timeline events delivered and not yet used.
    frames: Vec<String>,
    /// Trigger animations the goals asked for and the animation sync has not taken.
    triggers: Vec<String>,
}

impl AttackState {
    fn push(list: &mut Vec<String>, name: &str) {
        if list.len() >= KEPT {
            list.remove(0);
        }
        list.push(name.to_owned());
    }
    /// A server timeline event named `name` happened.
    pub fn push_frame(&mut self, name: &str) {
        Self::push(&mut self.frames, name);
    }
    fn take_frame(&mut self, name: &str) -> bool {
        match self.frames.iter().position(|f| f == name) {
            Some(i) => {
                self.frames.remove(i);
                true
            }
            None => false,
        }
    }
    fn forget_frame(&mut self, name: &str) {
        self.frames.retain(|f| f != name);
    }
    pub(super) fn request_trigger(&mut self, name: &str) {
        Self::push(&mut self.triggers, name);
    }
    /// The trigger animations requested since the last call, oldest first.
    pub fn take_triggers(&mut self) -> Vec<String> {
        std::mem::take(&mut self.triggers)
    }
}

/// Hands a server timeline event of `m`'s animation to its attack goals (the F4M-9 engine part
/// calls this for every `Custom` event the server's rig emits). False for a mob that is not a
/// custom one.
pub fn deliver_frame(m: &mut Mob, name: &str) -> bool {
    match m.extra.custom.as_mut() {
        Some(c) => {
            c.attack.push_frame(name);
            true
        }
        None => false,
    }
}

/// The trigger animations `m`'s attack goals asked to play since the last call (F4M-5b plays
/// them on the synced trigger clock).
pub fn take_triggers(m: &mut Mob) -> Vec<String> {
    m.extra
        .custom
        .as_mut()
        .map(|c| c.attack.take_triggers())
        .unwrap_or_default()
}

/// An attack that has begun and waits for its damage frame.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Pending {
    deadline: i64,
}

/// A goal row's `trigger`, `damage_frame` and `damage_delay`.
#[derive(Clone, Debug, Default)]
pub struct Timing {
    trigger: Option<String>,
    frame: Option<String>,
    delay: i32,
}

impl Timing {
    pub fn from_params(p: Params) -> Self {
        let frame = p.text("damage_frame").map(str::to_owned);
        let default = if frame.is_some() { DEFAULT_FRAME_DELAY } else { 0 };
        Self {
            trigger: p.text("trigger").map(str::to_owned),
            delay: p.int("damage_delay", default).clamp(0, 200),
            frame,
        }
    }
    /// Starts an attack at game time `now`: asks for the trigger animation and forgets a stale
    /// frame of the same name.
    pub fn begin(&self, m: &mut Mob, now: i64) -> Pending {
        if let Some(c) = m.extra.custom.as_mut() {
            if let Some(t) = &self.trigger {
                c.attack.request_trigger(t);
            }
            if let Some(f) = &self.frame {
                c.attack.forget_frame(f);
            }
        }
        // With a server animation that has the frame, the keyframe decides and `damage_delay` is
        // not the clock any more: the deadline is only the animation's length as a safety net.
        let delay = match (&self.frame, &m.kind.custom) {
            (Some(f), Some(t)) => super::super::anim_sync::frame_deadline(t, self.trigger.as_deref(), f)
                .map_or(i64::from(self.delay), |safety| safety.max(i64::from(self.delay))),
            _ => i64::from(self.delay),
        };
        Pending {
            deadline: now + delay,
        }
    }
    /// Whether the blow of the attack that began as `p` lands now: the frame arrived, or the
    /// fallback delay ran out.
    pub fn due(&self, m: &mut Mob, p: Pending, now: i64) -> bool {
        if let (Some(f), Some(c)) = (&self.frame, m.extra.custom.as_mut())
            && c.attack.take_frame(f)
        {
            return true;
        }
        now >= p.deadline
    }
}

/// Adds the attack goal `g` names if it is one of this module's; false for any other type.
pub fn add(m: &mut Mob, g: &GoalDef) -> bool {
    let p = Params(g);
    let prio = p.priority();
    let timing = Timing::from_params(p);
    match g.kind.as_str() {
        "melee_attack" => m.ai.goals.add(
            prio,
            ChaseStrike::new(
                Strike::Melee,
                p.speed(1.0),
                p.flag("follow_unseen", false),
                p.int("interval", 20).max(1),
                timing,
            ),
        ),
        "area_attack" => {
            let (species, player, _) = species_list(&p.strings("types"));
            let radius = p.num("radius", 3.0).clamp(0.5, 32.0);
            let area = Area {
                radius,
                range: p.num("range", radius).clamp(0.5, 32.0),
                damage: p.num("damage", f64::from(m.attrs.attack_damage)).max(0.0) as f32,
                knockback: p.num("knockback", 0.5).clamp(0.0, 10.0),
                cone: (p.text("shape") == Some("cone"))
                    .then(|| p.num("angle", 90.0).clamp(1.0, 360.0)),
                species,
                // The default victim is the player.
                player: player || p.0.params.get("types").is_none(),
            };
            m.ai.goals.add(
                prio,
                ChaseStrike::new(
                    Strike::Area(area),
                    p.speed(1.0),
                    p.flag("follow_unseen", false),
                    p.int("interval", 40).max(1),
                    timing,
                ),
            );
        }
        "ranged_attack" => {
            let Some(shot) = Shot::from_params(p) else {
                log::warn!(
                    "Horizon: ranged_attack: unknown projectile {:?}; the goal is left out",
                    p.text("projectile")
                );
                return true;
            };
            m.ai.goals.add(
                prio,
                RangedAttack::new(
                    p.speed(1.0),
                    p.int("interval_min", 20).max(1),
                    p.int("interval_max", 60).max(1),
                    p.num("radius", 15.0).clamp(1.0, 64.0) as f32,
                    shot,
                    timing,
                ),
            );
        }
        "leap_attack" => m.ai.goals.add(
            prio,
            LeapAttack::new(
                LeapSpec {
                    yd: p.num("impulse", 0.4).clamp(0.0, 2.0),
                    forward: p.num("forward", 0.4).clamp(0.0, 4.0),
                    min_sqr: p.num("min_range", 2.0).max(0.0).powi(2),
                    max_sqr: p.num("max_range", 4.0).max(0.5).powi(2),
                    windup: p.int("windup", 0).clamp(0, 100),
                    cooldown: p.int("cooldown", 40).clamp(0, 1200),
                    chance: p.int("chance", 5).max(1),
                },
                timing,
            ),
        ),
        _ => return false,
    }
    true
}

// ---- melee and area: chase, then strike ------------------------------------------------------

/// What the blow of a chase-and-strike goal is.
#[derive(Clone, Debug)]
pub enum Strike {
    /// `Mob.doHurtTarget` on the target, in `within_melee_range`.
    Melee,
    /// Damage to everything in a shape around the mob.
    Area(Area),
}

/// `area_attack`'s shape and numbers.
#[derive(Clone, Debug)]
pub struct Area {
    /// The reach of the blow, blocks from the mob's centre.
    pub radius: f64,
    /// How near the target must be for the attack to begin.
    pub range: f64,
    pub damage: f32,
    pub knockback: f64,
    /// `Some(angle)`: only within this many degrees (in total) in front of the mob.
    pub cone: Option<f64>,
    /// Besides the target itself, these species are hit...
    pub species: Vec<Species>,
    /// ... and the player, when set.
    pub player: bool,
}

#[derive(Clone, Debug)]
pub struct ChaseStrike {
    strike: Strike,
    inner: MeleeAttack,
    timing: Timing,
    interval: i32,
    until: i32,
    pending: Option<Pending>,
}

impl ChaseStrike {
    pub fn new(strike: Strike, speed: f64, follow_unseen: bool, interval: i32, timing: Timing) -> Self {
        Self {
            strike,
            inner: MeleeAttack::new(speed, follow_unseen),
            timing,
            interval,
            until: 0,
            pending: None,
        }
    }
    /// Whether the target is near enough for the blow to begin (or, when it lands, to still hit).
    fn in_reach(&self, m: &Mob, t: &TargetInfo) -> bool {
        match &self.strike {
            Strike::Melee => m.within_melee_range(t),
            Strike::Area(a) => {
                let c = m.center();
                let nearest = nearest_point(&t.bounds, c);
                dist_sqr(c, nearest) <= a.range * a.range
            }
        }
    }
    fn land(&self, m: &mut Mob, who: Who, cx: &mut Cx) {
        match &self.strike {
            Strike::Melee => {
                m.swing();
                crate::mobs::ai::do_hurt_target(m, who, cx);
            }
            Strike::Area(a) => {
                m.swing();
                area_hit(a, m, who, cx);
            }
        }
    }
}

impl Goal for ChaseStrike {
    goal_clone!();
    fn name(&self) -> &'static str {
        match self.strike {
            Strike::Melee => "CustomMelee",
            Strike::Area(_) => "AreaAttack",
        }
    }
    fn flags(&self) -> u8 {
        MOVE | LOOK
    }
    fn can_use(&mut self, m: &mut Mob, cx: &mut Cx) -> bool {
        self.inner.can_use(m, cx)
    }
    fn can_continue(&mut self, m: &mut Mob, cx: &mut Cx) -> bool {
        if self.pending.is_some() {
            // A blow is on its way: stay until it lands, even when the chase has arrived (the
            // wrapped goal ends then), unless the target is gone.
            return m.target_info(cx.w).is_some_and(|t| t.alive);
        }
        self.inner.can_continue(m, cx)
    }
    fn start(&mut self, m: &mut Mob, cx: &mut Cx) {
        self.inner.start(m, cx);
        self.until = 0;
        self.pending = None;
    }
    fn stop(&mut self, m: &mut Mob, cx: &mut Cx) {
        self.pending = None;
        self.inner.stop(m, cx);
    }
    fn every_tick(&self) -> bool {
        true
    }
    fn tick(&mut self, m: &mut Mob, cx: &mut Cx) {
        // The wrapped goal paths and looks; its own strike never comes.
        self.inner.until_attack = HOLD;
        self.inner.tick(m, cx);
        let Some(who) = m.ai.target else { return };
        let Some(t) = cx.w.target_info(who) else {
            return;
        };
        let now = cx.w.game_time;
        self.until = (self.until - 1).max(0);
        if let Some(p) = self.pending {
            if self.timing.due(m, p, now) {
                self.pending = None;
                if t.alive && self.in_reach(m, &t) {
                    self.land(m, who, cx);
                }
            }
            return;
        }
        if self.until <= 0 && self.in_reach(m, &t) && m.sees(cx.w, who) {
            self.until = self.interval;
            let p = self.timing.begin(m, now);
            if self.timing.due(m, p, now) {
                self.land(m, who, cx);
            } else {
                self.pending = Some(p);
            }
        }
    }
}

fn dist_sqr(a: [f64; 3], b: [f64; 3]) -> f64 {
    (0..3).map(|i| (a[i] - b[i]).powi(2)).sum()
}

/// The point of `b` nearest to `p`.
fn nearest_point(b: &horizon_core::geometry::Aabb, p: [f64; 3]) -> [f64; 3] {
    let (lo, hi) = (b.min_position(), b.max_position());
    [
        p[0].clamp(lo.x, hi.x),
        p[1].clamp(lo.y, hi.y),
        p[2].clamp(lo.z, hi.z),
    ]
}

/// The horizontal direction the mob faces (`-sin(yaw)`, `cos(yaw)`).
fn facing(yaw: f32) -> [f64; 2] {
    let a = f64::from(yaw).to_radians();
    [-a.sin(), a.cos()]
}

/// Whether the point `at` is in the area's shape around `m`.
fn in_shape(a: &Area, m: &Mob, bounds: &horizon_core::geometry::Aabb) -> bool {
    let c = m.center();
    let n = nearest_point(bounds, c);
    if dist_sqr(c, n) > a.radius * a.radius {
        return false;
    }
    let Some(angle) = a.cone else { return true };
    let (dx, dz) = (n[0] - c[0], n[2] - c[2]);
    let len = (dx * dx + dz * dz).sqrt();
    if len < 1.0e-6 {
        return true;
    }
    let f = facing(m.yaw);
    let cos = (dx * f[0] + dz * f[1]) / len;
    cos >= (angle / 2.0).to_radians().cos()
}

/// The blow of an `area_attack`: the player, the listed species and the goal's own target, when
/// they are in the shape; each takes `damage` and is pushed away from the mob.
fn area_hit(a: &Area, m: &mut Mob, target: Who, cx: &mut Cx) {
    let from = [m.position.x, m.position.y, m.position.z];
    let source = Damage::mob(m.id, from);
    let mut hits: Vec<Who> = Vec::new();
    if a.player
        && let Some(p) = cx.w.player.as_ref()
        && p.alive
        && !p.creative
        && !p.spectator
        && in_shape(a, m, &p.bounds)
    {
        hits.push(Who::Player);
    }
    let id = m.id;
    let reach = a.radius + 4.0;
    let center = Vec3::new(from[0], from[1], from[2]);
    let list = a.species.clone();
    let mobs: Vec<(u32, horizon_core::geometry::Aabb)> = cx
        .w
        .nearby(center, reach, move |s| s.id != id && list.contains(&s.species))
        .map(|s| (s.id, s.bounds))
        .collect();
    for (mid, bounds) in mobs {
        if in_shape(a, m, &bounds) {
            hits.push(Who::Mob(mid));
        }
    }
    // The goal's own target is hit if it is in the shape, whatever its type.
    if !hits.contains(&target)
        && let Some(t) = cx.w.target_info(target)
        && t.alive
        && in_shape(a, m, &t.bounds)
    {
        hits.push(target);
    }
    let yaw = m.yaw.to_radians();
    for who in hits {
        match who {
            Who::Player => cx.events.push(MobEvent::AttackPlayer(a.damage, from)),
            Who::Mob(target) => cx.events.push(MobEvent::AttackMob {
                target,
                amount: a.damage,
                source,
            }),
        }
        if a.knockback > 0.0 {
            cx.events.push(MobEvent::Knockback {
                target: who,
                power: a.knockback,
                x: f64::from(yaw.sin()),
                z: f64::from(-yaw.cos()),
                source,
            });
        }
    }
}

// ---- ranged ----------------------------------------------------------------------------------

/// What a `ranged_attack` shoots.
#[derive(Clone, Debug)]
pub struct Shot {
    kind: ProjectileKind,
    /// Fireballs fly straight at their target; the rest are lobbed.
    fireball: bool,
    speed: f32,
    spread: f32,
    /// How much of the horizontal distance is added to the aim's height (vanilla's skeletons
    /// and snow golems use 0.2).
    arc: f64,
}

impl Shot {
    /// The shot of a `ranged_attack` row: `projectile` (`arrow`, `snowball`, `egg`,
    /// `llama_spit`, `small_fireball`, `large_fireball`, `wind_charge`; default `arrow`),
    /// `projectile_speed`, `spread` (degrees of uncertainty), `arc` and, for an arrow, `damage`
    /// (its base damage, default 2).
    pub fn from_params(p: Params) -> Option<Self> {
        let name = p.text("projectile").unwrap_or("arrow");
        let name = name.strip_prefix("minecraft:").unwrap_or(name);
        let (kind, fireball, speed, spread, arc) = match name {
            "arrow" => (
                ProjectileKind::Arrow {
                    base_damage: p.num("damage", 2.0).clamp(0.0, 100.0),
                    critical: false,
                    effect: None,
                },
                false,
                1.6,
                10.0,
                0.2,
            ),
            "snowball" => (ProjectileKind::Snowball, false, 1.6, 12.0, 0.2),
            "egg" => (ProjectileKind::Egg, false, 1.6, 12.0, 0.2),
            "llama_spit" => (ProjectileKind::LlamaSpit, false, 1.5, 10.0, 0.2),
            "small_fireball" => (ProjectileKind::SmallFireball, true, 1.0, 0.0, 0.0),
            "large_fireball" => (ProjectileKind::LargeFireball { power: 1 }, true, 1.0, 0.0, 0.0),
            "wind_charge" => (ProjectileKind::BreezeWindCharge, false, 1.0, 5.0, 0.0),
            _ => return None,
        };
        Some(Self {
            kind,
            fireball,
            speed: p.num("projectile_speed", f64::from(speed)).clamp(0.1, 8.0) as f32,
            spread: p.num("spread", f64::from(spread)).clamp(0.0, 90.0) as f32,
            arc: p.num("arc", arc).clamp(0.0, 2.0),
        })
    }

    /// Shoots at `who` from the mob's eyes.
    fn fire(&self, m: &mut Mob, who: Who, cx: &mut Cx) {
        let Some(t) = cx.w.target_info(who) else {
            return;
        };
        let start = Vec3::new(m.position.x, m.eye_y() - 0.1, m.position.z);
        let (lo, hi) = (t.bounds.min_position(), t.bounds.max_position());
        let xd = t.position.x - m.position.x;
        let yd = lo.y + (hi.y - lo.y) / 3.0 - start.y;
        let zd = t.position.z - m.position.z;
        let dist = (xd * xd + zd * zd).sqrt();
        let owner = Some(Who::Mob(m.id));
        let p = if self.fireball {
            Projectile::fireball(self.kind.clone(), start, owner, [xd, yd, zd])
        } else {
            let mut p = Projectile::new(self.kind.clone(), start, owner);
            let mut random = crate::projectiles::own_random(m);
            p.shoot(
                &mut random,
                [xd, yd + dist * self.arc, zd],
                self.speed,
                self.spread,
            );
            p
        };
        cx.events.push(MobEvent::Projectile(p));
    }
}

/// `RangedAttackGoal(speed, intervalMin, intervalMax, radius)` with a data projectile. The
/// engine's own `RangedAttack` takes a plain function for its shot, which cannot carry a row's
/// numbers, so its timing is written out here (the same formula and the same movement); the
/// shot goes through [`Timing`] like every attack.
#[derive(Clone, Debug)]
pub struct RangedAttack {
    speed: f64,
    min: i32,
    max: i32,
    radius: f32,
    shot: Shot,
    timing: Timing,
    attack_time: i32,
    see_time: i32,
    pending: Option<Pending>,
}

impl RangedAttack {
    pub fn new(speed: f64, min: i32, max: i32, radius: f32, shot: Shot, timing: Timing) -> Self {
        Self {
            speed,
            min,
            max: max.max(min),
            radius,
            shot,
            timing,
            attack_time: -1,
            see_time: 0,
            pending: None,
        }
    }
}

impl Goal for RangedAttack {
    goal_clone!();
    fn name(&self) -> &'static str {
        "CustomRanged"
    }
    fn flags(&self) -> u8 {
        MOVE | LOOK
    }
    fn can_use(&mut self, m: &mut Mob, cx: &mut Cx) -> bool {
        m.target_info(cx.w).is_some_and(|t| t.alive)
    }
    fn can_continue(&mut self, m: &mut Mob, cx: &mut Cx) -> bool {
        self.can_use(m, cx) || m.target_info(cx.w).is_some_and(|t| t.alive) && !m.ai.nav.is_done()
    }
    fn stop(&mut self, _: &mut Mob, _: &mut Cx) {
        self.see_time = 0;
        self.attack_time = -1;
        self.pending = None;
    }
    fn every_tick(&self) -> bool {
        true
    }
    fn tick(&mut self, m: &mut Mob, cx: &mut Cx) {
        let Some(who) = m.ai.target else { return };
        let Some(t) = cx.w.target_info(who) else {
            return;
        };
        let now = cx.w.game_time;
        let d2 = m.distance_sqr(t.position);
        let sees = m.sees(cx.w, who);
        if sees {
            self.see_time += 1;
        } else {
            self.see_time = 0;
        }
        if d2 <= f64::from(self.radius * self.radius) && self.see_time >= 5 {
            m.nav_stop();
        } else {
            m.nav_move_to_entity(
                cx.w,
                [t.position.x, t.position.y, t.position.z],
                1,
                self.speed,
            );
        }
        m.look_at_speed([t.position.x, t.eye_y, t.position.z], 30.0, 30.0);
        if let Some(p) = self.pending {
            if self.timing.due(m, p, now) {
                self.pending = None;
                if t.alive {
                    self.shot.fire(m, who, cx);
                }
            }
        }
        self.attack_time -= 1;
        if self.attack_time == 0 {
            if !sees {
                return;
            }
            let dist = (d2.sqrt() as f32) / self.radius;
            if self.pending.is_none() {
                let p = self.timing.begin(m, now);
                if self.timing.due(m, p, now) {
                    self.shot.fire(m, who, cx);
                } else {
                    self.pending = Some(p);
                }
            }
            self.attack_time =
                (dist.clamp(0.1, 1.0) * (self.max - self.min) as f32 + self.min as f32).floor()
                    as i32;
        } else if self.attack_time < 0 {
            let f = d2.sqrt() / f64::from(self.radius);
            self.attack_time =
                (f64::from(self.min) + f * f64::from(self.max - self.min)).floor() as i32;
        }
    }
}

// ---- leap ------------------------------------------------------------------------------------

/// `leap_attack`'s numbers.
#[derive(Clone, Debug)]
pub struct LeapSpec {
    /// The upward impulse (`LeapAtTargetGoal`'s `yd`).
    pub yd: f64,
    /// The horizontal speed of the leap.
    pub forward: f64,
    /// The leap starts when the target is within these distances (squared).
    pub min_sqr: f64,
    pub max_sqr: f64,
    /// Ticks the mob stands facing the target before it jumps.
    pub windup: i32,
    /// Ticks between two leaps.
    pub cooldown: i32,
    /// One in this many chances a tick the leap starts (vanilla's goal is 5).
    pub chance: i32,
}

#[derive(Clone, Copy, Debug, PartialEq)]
enum Phase {
    Idle,
    /// Facing the target, this many ticks to the jump.
    Windup(i32),
    /// In the air for this many ticks; the blow still to come, or done.
    Air { ticks: i32, struck: bool },
}

/// `LeapAtTargetGoal` with a windup, a trigger animation and a damage frame: the mob faces its
/// target for `windup` ticks, jumps, and hurts the target (`Mob.doHurtTarget`) once, at the
/// damage frame (or, with none, the first tick it is in reach while in the air). Takes `MOVE`
/// and `JUMP`.
#[derive(Clone, Debug)]
pub struct LeapAttack {
    spec: LeapSpec,
    timing: Timing,
    phase: Phase,
    pending: Option<Pending>,
    aim: [f64; 3],
    ready_at: i64,
}

impl LeapAttack {
    pub fn new(spec: LeapSpec, timing: Timing) -> Self {
        Self {
            spec,
            timing,
            phase: Phase::Idle,
            pending: None,
            aim: [0.0; 3],
            ready_at: 0,
        }
    }
    fn jump(&self, m: &mut Mob) {
        let mut d = [self.aim[0] - m.position.x, self.aim[2] - m.position.z];
        let len_sq = d[0] * d[0] + d[1] * d[1];
        if len_sq > 1.0e-7 {
            let l = len_sq.sqrt();
            d = [
                d[0] / l * self.spec.forward + m.velocity.x * 0.2,
                d[1] / l * self.spec.forward + m.velocity.z * 0.2,
            ];
        }
        m.velocity = Vec3::new(d[0], self.spec.yd, d[1]);
    }
}

impl Goal for LeapAttack {
    goal_clone!();
    fn name(&self) -> &'static str {
        "LeapAttack"
    }
    fn flags(&self) -> u8 {
        MOVE | JUMP
    }
    fn can_use(&mut self, m: &mut Mob, cx: &mut Cx) -> bool {
        if cx.w.game_time < self.ready_at || !m.on_ground {
            return false;
        }
        let Some(who) = m.ai.target else {
            return false;
        };
        let Some(t) = cx.w.target_info(who) else {
            return false;
        };
        let d = m.distance_sqr(t.position);
        if !t.alive || d < self.spec.min_sqr || d > self.spec.max_sqr || !m.sees(cx.w, who) {
            return false;
        }
        self.aim = [t.position.x, t.position.y, t.position.z];
        m.random
            .next_int_bound(reduced(self.spec.chance))
            .unwrap_or(1)
            == 0
    }
    fn can_continue(&mut self, m: &mut Mob, _: &mut Cx) -> bool {
        match self.phase {
            Phase::Idle => false,
            Phase::Windup(_) => true,
            // Off the ground (the jump is a tick old before the mob has left it), and not forever.
            Phase::Air { ticks, .. } => ticks < 2 || (!m.on_ground && ticks < 100),
        }
    }
    fn start(&mut self, m: &mut Mob, cx: &mut Cx) {
        m.nav_stop();
        self.pending = Some(self.timing.begin(m, cx.w.game_time));
        if self.spec.windup > 0 {
            self.phase = Phase::Windup(self.spec.windup);
        } else {
            self.jump(m);
            self.phase = Phase::Air {
                ticks: 0,
                struck: false,
            };
        }
    }
    fn stop(&mut self, _: &mut Mob, cx: &mut Cx) {
        self.phase = Phase::Idle;
        self.pending = None;
        self.ready_at = cx.w.game_time + i64::from(self.spec.cooldown);
    }
    fn every_tick(&self) -> bool {
        true
    }
    fn tick(&mut self, m: &mut Mob, cx: &mut Cx) {
        let target = m.ai.target.and_then(|w| Some((w, cx.w.target_info(w)?)));
        match self.phase {
            Phase::Idle => {}
            Phase::Windup(left) => {
                if let Some((_, t)) = &target {
                    self.aim = [t.position.x, t.position.y, t.position.z];
                    m.look_at_speed([t.position.x, t.eye_y, t.position.z], 30.0, 30.0);
                }
                if left <= 1 {
                    self.jump(m);
                    self.phase = Phase::Air {
                        ticks: 0,
                        struck: false,
                    };
                } else {
                    self.phase = Phase::Windup(left - 1);
                }
            }
            Phase::Air { ticks, struck } => {
                let mut struck = struck;
                if !struck && let Some((who, t)) = target {
                    let now = cx.w.game_time;
                    let strike = match self.pending {
                        // A damage frame (or its fallback): the blow lands now or not at all.
                        Some(p) if self.timing.frame.is_some() || self.timing.delay > 0 => {
                            self.timing.due(m, p, now).then_some(true)
                        }
                        // No frame: the first tick it is in reach.
                        _ => m.within_melee_range(&t).then_some(true),
                    };
                    if strike.is_some() {
                        struck = true;
                        self.pending = None;
                        if t.alive && m.within_melee_range(&t) {
                            m.swing();
                            crate::mobs::ai::do_hurt_target(m, who, cx);
                        }
                    }
                }
                self.phase = Phase::Air {
                    ticks: ticks + 1,
                    struck,
                };
            }
        }
    }
}
