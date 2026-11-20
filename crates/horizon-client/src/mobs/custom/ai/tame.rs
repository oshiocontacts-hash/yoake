//! Taming of custom mobs and the owner goals (FR-F4-MOBS.md section 5, F4M-4c).
//!
//! `tameable { items, chance, sits_on_command, follow_owner }` is implemented once, here, on the
//! engine's own owner state (`Mob.extra.owner`, `tame`, `sitting`, which every other system that
//! reads a tamed mob, `Seen.owner` included, already understands). The vanilla `tame.rs` is the
//! wolf's and is not edited; its owner test ([`crate::mobs::tame::owned_by`]) is reused read-only.
//!
//! * [`try_tame`]: the item was offered (the caller consumed it): a chance roll on the mob's own
//!   random, `chance` (default 0.33). A success sets the owner, keeps the mob from despawning,
//!   sits it down when it `sits_on_command`, marks its `sync: owner` values for the new owner
//!   ([`mark_owner_values`]) and queues the `mob-tamed` notice.
//! * [`set_sitting`] / [`toggle_sit`] (the owner's click): the engine's `extra.sitting`, mirrored
//!   into the type's pose value (a bool called `sitting`/`sit`, or the enum `state` with a `sit`
//!   value) so the animation controllers see it.
//! * Goals: `follow_owner` (`speed`, `start_distance` 10, `stop_distance` 2, teleports to an owner
//!   12 blocks away), `sit`, `owner_hurt_target` (attack what the owner attacked), `owner_hurt_by`
//!   (attack what hurt the owner) and `owner_target` (both). A type with `tameable.follow_owner`
//!   or `sits_on_command` and no such row gets the default goal ([`add_defaults`]).
//!
//! The owner is the player the mob sees (`MobWorld.player`) whose uuid is saved as its owner, as
//! vanilla's `getOwner` is.
use super::Params;
use crate::mobs::{
    Mob, MobWorld, PlayerView, Species, Who,
    custom::{CustomType, Notice, SyncedValue, attrs::Sync},
    goals::{Cx, Goal, JUMP, LOOK, MOVE, TARGET, goal_clone, reduced},
    tame::owned_by,
};
use horizon_core::{
    geometry::{Aabb, Vec3},
    random::RandomSource,
};
use horizon_mods::custom_mob::{CustomMobFile, Goal as GoalDef};

/// The `tameable` block of a type.
#[derive(Clone, Debug, PartialEq)]
pub struct TameDef {
    /// Item ids or `#tags`.
    pub items: Vec<String>,
    pub chance: f64,
    pub sits_on_command: bool,
    pub follow_owner: bool,
}

impl TameDef {
    pub fn from_file(f: &CustomMobFile) -> Option<Self> {
        f.tameable.as_ref().map(|t| Self {
            items: t.items.clone(),
            chance: t.chance,
            sits_on_command: t.sits_on_command,
            follow_owner: t.follow_owner,
        })
    }
}

fn def(m: &Mob) -> Option<&TameDef> {
    m.kind.custom.as_deref().and_then(|t| t.tame.as_ref())
}

/// Whether the mob has an owner.
pub fn is_tamed(m: &Mob) -> bool {
    m.extra.tame && m.extra.owner.is_some()
}

/// The owner as this mob sees it: the player of the world view when its uuid is the saved owner.
pub fn owner_view<'a>(m: &Mob, w: &'a MobWorld) -> Option<&'a PlayerView> {
    if !is_tamed(m) {
        return None;
    }
    let p = w.player.as_ref()?;
    owned_by(m.extra.owner, p.uuid).then_some(p)
}

fn owner_distance_sqr(m: &Mob, w: &MobWorld) -> Option<f64> {
    owner_view(m, w).map(|p| m.distance_sqr(p.position))
}

/// Marks every `sync: owner` value dirty so the (new) owner is told it, whatever it held.
pub fn mark_owner_values(m: &mut Mob) {
    let (Some(t), Some(c)) = (m.kind.custom.clone(), m.extra.custom.as_mut()) else {
        return;
    };
    for (slot, d) in t.synced.iter().enumerate().take(32) {
        if d.sync == Sync::Owner {
            c.synced_dirty |= 1 << slot;
        }
    }
}

/// Makes `owner` the owner of `m` (the taming itself, with no chance roll): persistent, owner
/// values marked for the owner, sitting when the type `sits_on_command`, the `mob-tamed` notice
/// queued. A mob that is already someone's changes hands.
pub fn tame(m: &mut Mob, owner: [i32; 4]) {
    let sits = def(m).is_some_and(|d| d.sits_on_command);
    m.extra.tame = true;
    m.extra.owner = Some(owner);
    m.persistent = true;
    m.ai.target = None;
    mark_owner_values(m);
    set_sitting(m, sits);
    if let Some(c) = m.extra.custom.as_mut() {
        c.notice(Notice::Tamed { owner });
    }
}

/// Forgets the owner (a script or a command un-tames): the owner values go to nobody again.
pub fn untame(m: &mut Mob) {
    m.extra.tame = false;
    m.extra.owner = None;
    set_sitting(m, false);
}

/// The taming roll after `player` offered an item the type accepts: `true` when it was tamed.
/// The roll is the mob's own random, so a seeded mob is deterministic.
pub fn try_tame(m: &mut Mob, player: [i32; 4]) -> bool {
    let Some(chance) = def(m).map(|d| d.chance) else {
        return false;
    };
    if is_tamed(m) || !m.alive() {
        return false;
    }
    if m.random.next_double() < chance {
        tame(m, player);
        true
    } else {
        false
    }
}

/// Whether `item` is one the type tames with (`matches(pattern, item)` resolves `#tags`).
pub fn is_tame_item(m: &Mob, item: Option<&str>, matches: &dyn Fn(&str, &str) -> bool) -> bool {
    match (def(m), item) {
        (Some(d), Some(i)) => d.items.iter().any(|p| matches(p, i)),
        _ => false,
    }
}

/// Sits the mob down or stands it up (the owner's order), and tells the pose value. Does nothing
/// to a mob that is not tamed when asked to sit.
pub fn set_sitting(m: &mut Mob, on: bool) {
    if on && !is_tamed(m) {
        return;
    }
    m.extra.sitting = on;
    if on {
        m.nav_stop();
        m.ai.target = None;
    }
    mirror_pose(m);
}

/// The owner's order: sit or stand, when the type `sits_on_command`. Returns the new state, or
/// `None` when the mob does not take the order.
pub fn toggle_sit(m: &mut Mob) -> Option<bool> {
    if !is_tamed(m) || !def(m).is_some_and(|d| d.sits_on_command) {
        return None;
    }
    let on = !m.extra.sitting;
    set_sitting(m, on);
    Some(on)
}

/// Writes `extra.sitting` into the type's pose value: a `bool` named `sitting` or `sit`, or the
/// enum `state` that has a `sit` value (standing goes back to its default value, and only when it
/// was sitting).
pub fn mirror_pose(m: &mut Mob) {
    let on = m.extra.sitting;
    let (Some(t), Some(c)) = (m.kind.custom.clone(), m.extra.custom.as_mut()) else {
        return;
    };
    for name in ["sitting", "sit"] {
        if let Some(slot) = t.synced_slot(name)
            && t.synced[slot].kind == "bool"
        {
            c.set_synced_slot(&t, slot, SyncedValue::Bool(on));
            return;
        }
    }
    let Some(slot) = t.synced_slot("state") else { return };
    let d = &t.synced[slot];
    let Some(sit) = d.values.iter().position(|v| v == "sit") else {
        return;
    };
    if d.kind != "enum" {
        return;
    }
    if on {
        c.set_synced_slot(&t, slot, SyncedValue::Enum(sit as u8));
    } else if c.synced.get(slot) == Some(&SyncedValue::Enum(sit as u8)) {
        c.set_synced_slot(&t, slot, d.default.clone());
    }
}

// ---- goals --------------------------------------------------------------------------------------

/// Adds the owner goal `g` names; false for any other type.
pub fn add(m: &mut Mob, g: &GoalDef) -> bool {
    let p = Params(g);
    let prio = p.priority();
    match g.kind.as_str() {
        "follow_owner" => m.ai.goals.add(
            prio,
            FollowOwner::new(
                p.speed(1.0),
                p.num("start_distance", 10.0).clamp(1.0, 64.0),
                p.num("stop_distance", 2.0).clamp(0.5, 64.0),
            ),
        ),
        "sit" => m.ai.goals.add(prio, Sit),
        "owner_hurt_target" => m.ai.targets.add(prio, OwnerHurt::new(Which::Target)),
        "owner_hurt_by" => m.ai.targets.add(prio, OwnerHurt::new(Which::HurtBy)),
        "owner_target" => m.ai.targets.add(prio, OwnerHurt::new(Which::Both)),
        _ => return false,
    }
    true
}

/// The goals `tameable` implies when the type's rows do not list them: `follow_owner` (priority 6)
/// and `sit` (priority 1, as vanilla's `SitWhenOrderedToGoal`), and the owner's target selectors
/// (`owner_hurt_by` 1, `owner_hurt_target` 2) for a tameable type that already attacks (it has an
/// attack row).
pub fn add_defaults(m: &mut Mob, t: &CustomType) {
    let Some(d) = &t.tame else { return };
    let has = |k: &str| t.file.ai.goals.iter().chain(&t.file.ai.targets).any(|g| g.kind == k);
    if d.follow_owner && !has("follow_owner") && !has("owner_target") {
        m.ai.goals.add(6, FollowOwner::new(1.0, 10.0, 2.0));
    }
    if d.sits_on_command && !has("sit") {
        m.ai.goals.add(1, Sit);
    }
    let attacks = t.file.ai.goals.iter().any(|g| {
        matches!(
            g.kind.as_str(),
            "melee_attack" | "ranged_attack" | "leap_attack" | "area_attack"
        )
    });
    if attacks && !has("owner_hurt_by") && !has("owner_target") {
        m.ai.targets.add(1, OwnerHurt::new(Which::HurtBy));
    }
    if attacks && !has("owner_hurt_target") && !has("owner_target") {
        m.ai.targets.add(2, OwnerHurt::new(Which::Target));
    }
}

/// `TamableAnimal.tryToTeleportToOwner`: ten tries around the owner, on a walkable block.
fn teleport_to_owner(m: &mut Mob, w: &MobWorld) {
    let Some(p) = owner_view(m, w) else { return };
    let base = [p.position.x.floor() as i32, p.position.y.floor() as i32, p.position.z.floor() as i32];
    for _ in 0..10 {
        let xd = m.random.next_int_bound(7).unwrap_or(0) - 3;
        let zd = m.random.next_int_bound(7).unwrap_or(0) - 3;
        if xd.abs() < 2 && zd.abs() < 2 {
            continue;
        }
        let yd = m.random.next_int_bound(3).unwrap_or(0) - 1;
        let at = [base[0] + xd, base[1] + yd, base[2] + zd];
        if can_stand_at(m, w, at) {
            m.position = Vec3::new(f64::from(at[0]) + 0.5, f64::from(at[1]), f64::from(at[2]) + 0.5);
            m.previous = m.position;
            m.nav_stop();
            return;
        }
    }
}

fn can_stand_at(m: &Mob, w: &MobWorld, p: [i32; 3]) -> bool {
    let below = w.facts.get(w.world.block(p[0], p[1] - 1, p[2]));
    if !below.full_collision || below.name.ends_with("_leaves") {
        return false;
    }
    let here = w.facts.get(w.world.block(p[0], p[1], p[2]));
    if here.fluid != crate::blocks::Fluid::None {
        return false;
    }
    let half = f64::from(m.width()) / 2.0;
    let (x, y, z) = (f64::from(p[0]) + 0.5, f64::from(p[1]), f64::from(p[2]) + 0.5);
    w.no_collision(Aabb::new(
        Vec3::new(x - half, y, z - half),
        Vec3::new(x + half, y + f64::from(m.height()), z + half),
    ))
}

/// `FollowOwnerGoal(speed, startDistance, stopDistance)`.
#[derive(Clone, Debug)]
pub struct FollowOwner {
    speed: f64,
    start: f64,
    stop: f64,
    recalc: i32,
}
impl FollowOwner {
    pub fn new(speed: f64, start: f64, stop: f64) -> Self {
        Self { speed, start, stop, recalc: 0 }
    }
}
impl Goal for FollowOwner {
    goal_clone!();
    fn name(&self) -> &'static str {
        "FollowOwner"
    }
    fn flags(&self) -> u8 {
        MOVE | LOOK
    }
    fn can_use(&mut self, m: &mut Mob, cx: &mut Cx) -> bool {
        let Some(d) = owner_distance_sqr(m, cx.w) else { return false };
        !m.extra.sitting && d >= self.start * self.start
    }
    fn can_continue(&mut self, m: &mut Mob, cx: &mut Cx) -> bool {
        if m.ai.nav.is_done() || m.extra.sitting {
            return false;
        }
        owner_distance_sqr(m, cx.w).is_some_and(|d| d > self.stop * self.stop)
    }
    fn start(&mut self, _: &mut Mob, _: &mut Cx) {
        self.recalc = 0;
    }
    fn stop(&mut self, m: &mut Mob, _: &mut Cx) {
        m.nav_stop();
    }
    fn every_tick(&self) -> bool {
        true
    }
    fn tick(&mut self, m: &mut Mob, cx: &mut Cx) {
        let Some((at, eye)) = owner_view(m, cx.w).map(|p| (p.position, p.eye[1])) else {
            return;
        };
        let far = m.distance_sqr(at) >= 144.0;
        if !far {
            let max = m.max_head_x_rot();
            m.look_at_speed([at.x, eye, at.z], 10.0, max);
        }
        self.recalc -= 1;
        if self.recalc <= 0 {
            self.recalc = reduced(10);
            if far {
                teleport_to_owner(m, cx.w);
            } else {
                m.nav_move_to_entity(cx.w, [at.x, at.y, at.z], 0, self.speed);
            }
        }
    }
}

/// `SitWhenOrderedToGoal`: while ordered to sit the mob stands still (it holds `MOVE` and `JUMP`).
#[derive(Clone, Debug)]
pub struct Sit;
impl Goal for Sit {
    goal_clone!();
    fn name(&self) -> &'static str {
        "Sit"
    }
    fn flags(&self) -> u8 {
        JUMP | MOVE
    }
    fn can_use(&mut self, m: &mut Mob, _: &mut Cx) -> bool {
        is_tamed(m) && m.extra.sitting && !m.fluids.in_water && m.on_ground
    }
    fn can_continue(&mut self, m: &mut Mob, _: &mut Cx) -> bool {
        m.extra.sitting
    }
    fn start(&mut self, m: &mut Mob, _: &mut Cx) {
        m.nav_stop();
        mirror_pose(m);
    }
}

/// What an owner target goal reacts to.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Which {
    /// What the owner attacked (`OwnerHurtTargetGoal`).
    Target,
    /// What hurt the owner (`OwnerHurtByTargetGoal`).
    HurtBy,
    /// Either, the more recent.
    Both,
}

/// `OwnerHurtTargetGoal` / `OwnerHurtByTargetGoal`.
#[derive(Clone, Debug)]
pub struct OwnerHurt {
    which: Which,
    /// The owner event (its timestamp) already answered.
    last: i64,
}
impl OwnerHurt {
    pub fn new(which: Which) -> Self {
        Self { which, last: i64::MIN }
    }
    fn pick(&self, m: &Mob, w: &MobWorld) -> Option<(Who, i64)> {
        let p = owner_view(m, w)?;
        let by = p.hurt_by.filter(|_| self.which != Which::Target);
        let hit = p.last_hurt.filter(|_| self.which != Which::HurtBy);
        match (by, hit) {
            (Some(a), Some(b)) => Some(if a.1 >= b.1 { a } else { b }),
            (a, b) => a.or(b),
        }
    }
}
impl Goal for OwnerHurt {
    goal_clone!();
    fn name(&self) -> &'static str {
        "OwnerHurt"
    }
    fn flags(&self) -> u8 {
        TARGET
    }
    fn can_use(&mut self, m: &mut Mob, cx: &mut Cx) -> bool {
        if m.extra.sitting {
            return false;
        }
        let Some((who, at)) = self.pick(m, cx.w) else { return false };
        at != self.last && wants_to_attack(m, cx.w, who)
    }
    fn can_continue(&mut self, m: &mut Mob, cx: &mut Cx) -> bool {
        !m.extra.sitting && m.ai.target.and_then(|t| cx.w.target_info(t)).is_some_and(|t| t.alive)
    }
    fn start(&mut self, m: &mut Mob, cx: &mut Cx) {
        if let Some((who, at)) = self.pick(m, cx.w) {
            m.ai.target = Some(who);
            self.last = at;
        }
    }
    fn stop(&mut self, m: &mut Mob, _: &mut Cx) {
        m.ai.target = None;
    }
}

/// What a tamed mob will not turn on: itself, another mob of the same owner, and a player.
fn wants_to_attack(m: &Mob, w: &MobWorld, who: Who) -> bool {
    let Who::Mob(id) = who else { return false };
    if id == m.id {
        return false;
    }
    let Some(s) = w.seen(id) else { return false };
    if s.tame && s.owner.is_some() && s.owner == m.extra.owner {
        return false;
    }
    // Creepers and ghasts are not worth it, as for a wolf.
    !matches!(s.species, Species::Creeper | Species::Ghast)
}
