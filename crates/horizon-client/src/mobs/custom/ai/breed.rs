//! Breeding of custom mobs and the `breed` and `follow_parent` goals (FR-F4-MOBS.md section 5,
//! F4M-4c).
//!
//! `breedable { food, partner, offspring, cooldown, baby_scale }` is implemented once, here, on the
//! engine's own animal state (`extra.in_love`, `extra.age`: negative is a baby, positive a breeding
//! pause, `love_cause`), the same numbers vanilla's animals use: 600 ticks of love after feeding,
//! 6000 ticks of pause (`cooldown`) after breeding, 24000 ticks as a baby, and feeding a baby
//! brings its growth forward by a tenth.
//!
//! * [`feed`]: food in hand. An adult that can fall in love does; a baby grows up a tenth faster.
//!   A tameable type falls in love only when it is tamed.
//! * [`Breed`] (`breed`, `speed`): walks to a partner in love (the type's own, or `partner`),
//!   after 60 ticks close by it breeds: both parents get the pause and the game's `MobEvent::Breed`
//!   handler spawns the baby (`offspring`, default the same type; see `game/custom_mob_breed.rs`) and
//!   calls [`complete`].
//! * [`FollowParent`] (`follow_parent`, `speed`): a baby follows the nearest adult of its type.
//! * [`tick`]: the growth and pause countdown the engine only runs for vanilla species, and the baby
//!   scale (`baby_scale`, default 0.5: the mob's model scale while it is a baby).
//! * [`complete`]: the part of a breeding that is data: the pause of the partner, the baby's age and
//!   inheritance of the owner, the `mob-bred` notice.
use super::tame;
use crate::mobs::{
    Mob, MobEvent, MobWorld,
    custom::{CustomType, Notice, attrs},
    goals::{Cx, Goal, LOOK, MOVE, goal_clone, reduced},
};
use horizon_mods::custom_mob::{CustomMobFile, Goal as GoalDef};
use std::sync::Arc;

use super::Params;

/// A baby's age when it is born (`AgeableMob.BABY_START_AGE`).
pub const BABY_AGE: i32 = -24000;
/// The breeding pause when the file gives none.
pub const DEFAULT_COOLDOWN: i32 = 6000;
/// `Animal.setInLove`.
pub const LOVE_TICKS: i32 = 600;
/// The model scale of a baby when the file gives none.
pub const DEFAULT_BABY_SCALE: f32 = 0.5;

/// The `breedable` block of a type.
#[derive(Clone, Debug, PartialEq)]
pub struct BreedDef {
    /// Item ids or `#tags`.
    pub food: Vec<String>,
    /// The type it breeds with (default: its own).
    pub partner: Option<String>,
    /// The type of the baby (default: its own).
    pub offspring: Option<String>,
    pub cooldown: i32,
    pub baby_scale: f32,
}

impl BreedDef {
    pub fn from_file(f: &CustomMobFile) -> Option<Self> {
        f.breedable.as_ref().map(|b| Self {
            food: b.food.clone(),
            partner: b.partner.clone(),
            offspring: b.offspring.clone(),
            cooldown: b.cooldown.map_or(DEFAULT_COOLDOWN, |c| c.min(i32::MAX as u32) as i32),
            baby_scale: b.baby_scale.map_or(DEFAULT_BABY_SCALE, |s| s as f32),
        })
    }
}

fn def(m: &Mob) -> Option<&BreedDef> {
    m.kind.custom.as_deref().and_then(|t| t.breed.as_ref())
}

/// The type id of `m`.
fn own_id(m: &Mob) -> Option<&str> {
    m.kind.custom.as_deref().map(|t| t.id.as_str())
}

/// Whether `item` is food of the type (`matches(pattern, item)` resolves `#tags`).
pub fn is_food(m: &Mob, item: Option<&str>, matches: &dyn Fn(&str, &str) -> bool) -> bool {
    match (def(m), item) {
        (Some(d), Some(i)) => d.food.iter().any(|p| matches(p, i)),
        _ => false,
    }
}

/// What feeding did.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Fed {
    /// An adult fell in love.
    Love,
    /// A baby grew up a tenth of what it had left.
    Grew,
    /// Not hungry now: in love already, resting after a breeding, not tamed (a tameable type) or
    /// not breedable. The food is not used.
    No,
}

/// `Animal.mobInteract` with food in hand, for `player` (who gets the credit).
pub fn feed(m: &mut Mob, player: Option<[i32; 4]>) -> Fed {
    if def(m).is_none() || !m.alive() {
        return Fed::No;
    }
    if m.kind.custom.as_deref().is_some_and(|t| t.tame.is_some()) && !tame::is_tamed(m) {
        return Fed::No;
    }
    if m.extra.age == 0 && m.extra.in_love <= 0 {
        m.extra.in_love = LOVE_TICKS;
        m.extra.love_player = player.is_some();
        m.extra.love_cause = player;
        Fed::Love
    } else if m.extra.age < 0 && !m.extra.age_locked {
        let secs = ((-m.extra.age / 20) as f32 * 0.1) as i32;
        crate::mobs::ai::age_up(m, secs, true);
        Fed::Grew
    } else {
        Fed::No
    }
}

/// The growth of a baby and the pause countdown (only the types that are `breedable`), once per
/// tick. In love is counted down by the engine for every mob. Then the mob's one scale
/// ([`attrs::sync_scale`]: its `scale` attribute times the `baby_scale` while it is a baby) is put on
/// it, for every custom type; with a world `w`, a mob that grew is moved to where its bigger box fits.
pub fn tick(m: &mut Mob, w: Option<&MobWorld>) {
    if def(m).is_some() && m.alive() {
        if m.extra.age < 0 {
            if !m.extra.age_locked {
                let next = m.extra.age + 1;
                crate::mobs::finalize::set_age(m, next);
            }
        } else if m.extra.age > 0 {
            m.extra.age -= 1;
        }
    }
    attrs::sync_scale(m, w);
}

/// The `baby_scale` of a `breedable` type.
pub fn baby_scale(m: &Mob) -> Option<f32> {
    def(m).map(|d| d.baby_scale)
}

/// The type id the baby of `parent` is (`offspring`, else the parent's own).
pub fn offspring_kind(parent: &Mob) -> Option<String> {
    let d = def(parent)?;
    Some(d.offspring.clone().unwrap_or_else(|| own_id(parent).unwrap_or_default().to_owned()))
}

/// The data part of a breeding, once the game has the baby `child`: the pause for the partner (the
/// parent's was set when it bred), the baby's age and scale, the owner a tamed parent hands on, and
/// the `mob-bred` notice for the parent.
pub fn complete(parent: &mut Mob, partner: Option<&mut Mob>, child: &mut Mob) {
    let cooldown = def(parent).map_or(DEFAULT_COOLDOWN, |d| d.cooldown);
    parent.extra.age = cooldown;
    parent.extra.in_love = 0;
    let partner_id = partner.as_ref().map(|p| p.id);
    if let Some(p) = partner {
        p.extra.age = cooldown;
        p.extra.in_love = 0;
    }
    child.extra.age = BABY_AGE;
    child.extra.in_love = 0;
    attrs::sync_scale(child, None);
    // `Wolf.getBreedOffspring`: a tamed parent's baby is tame, with the same owner.
    if tame::is_tamed(parent)
        && let Some(owner) = parent.extra.owner
        && child.kind.custom.as_deref().is_some_and(|t| t.tame.is_some())
    {
        tame::tame(child, owner);
        // A baby does not sit down for the order that tamed it.
        tame::set_sitting(child, false);
    }
    let child_id = child.id;
    if let Some(c) = parent.extra.custom.as_mut() {
        c.notice(Notice::Bred { partner: partner_id, child: Some(child_id) });
    }
}

/// Adds the goal `g` names if it is one of this module's.
pub fn add(m: &mut Mob, t: &CustomType, g: &GoalDef) -> bool {
    let p = Params(g);
    let prio = p.priority();
    match g.kind.as_str() {
        "breed" => {
            let d = t.breed.as_ref();
            m.ai.goals.add(
                prio,
                Breed::new(
                    p.speed(1.0),
                    t.id.clone(),
                    d.and_then(|d| d.partner.clone()),
                    t.tame.is_some(),
                ),
            );
        }
        "follow_parent" => m.ai.goals.add(prio, FollowParent::new(p.speed(1.1), t.id.clone())),
        _ => return false,
    }
    true
}

/// `BreedGoal`: walk to a partner in love and breed after 60 ticks.
#[derive(Clone, Debug)]
pub struct Breed {
    speed: f64,
    partner_type: Arc<str>,
    tamable: bool,
    partner: Option<u32>,
    love_time: i32,
}
impl Breed {
    pub fn new(speed: f64, own: String, partner: Option<String>, tamable: bool) -> Self {
        let partner_type: Arc<str> = Arc::from(partner.unwrap_or_else(|| own.clone()).as_str());
        Self {
            speed,
            partner_type,
            tamable,
            partner: None,
            love_time: 0,
        }
    }
}
impl Goal for Breed {
    goal_clone!();
    fn name(&self) -> &'static str {
        "Breed"
    }
    fn flags(&self) -> u8 {
        MOVE | LOOK
    }
    fn can_use(&mut self, m: &mut Mob, cx: &mut Cx) -> bool {
        if m.extra.in_love <= 0 || m.is_baby() || m.extra.sitting {
            return false;
        }
        if self.tamable && !tame::is_tamed(m) {
            return false;
        }
        let (id, tamable) = (m.id, self.tamable);
        let want = self.partner_type.clone();
        self.partner = cx
            .w
            .nearby(m.position, 8.0, move |s| {
                s.id != id
                    && s.in_love
                    && !s.baby
                    && !s.sitting
                    && s.custom.as_deref() == Some(&*want)
                    && (!tamable || s.tame)
            })
            .map(|s| s.id)
            .next();
        self.partner.is_some()
    }
    fn can_continue(&mut self, _: &mut Mob, cx: &mut Cx) -> bool {
        self.partner.and_then(|p| cx.w.seen(p)).is_some_and(|s| s.alive && s.in_love) && self.love_time < 60
    }
    fn stop(&mut self, _: &mut Mob, _: &mut Cx) {
        self.partner = None;
        self.love_time = 0;
    }
    fn tick(&mut self, m: &mut Mob, cx: &mut Cx) {
        let Some(p) = self.partner.and_then(|p| cx.w.seen(p)).cloned() else {
            return;
        };
        let x = m.max_head_x_rot();
        m.look_at_speed([p.position.x, p.eye_y, p.position.z], 10.0, x);
        m.nav_move_to_entity(cx.w, [p.position.x, p.position.y, p.position.z], 1, self.speed);
        self.love_time += 1;
        if self.love_time >= reduced(60) && m.distance_sqr(p.position) < 9.0 {
            m.extra.in_love = 0;
            m.extra.age = def(m).map_or(DEFAULT_COOLDOWN, |d| d.cooldown);
            cx.events.push(MobEvent::Breed { parent: m.id, partner: p.id, llama: None });
        }
    }
}

/// `FollowParentGoal`: a baby walks after the nearest adult of its type that is further than 3
/// blocks away.
#[derive(Clone, Debug)]
pub struct FollowParent {
    speed: f64,
    own: Arc<str>,
    parent: Option<u32>,
    recalc: i32,
}
impl FollowParent {
    pub fn new(speed: f64, own: String) -> Self {
        Self { speed, own: Arc::from(own.as_str()), parent: None, recalc: 0 }
    }
}
impl Goal for FollowParent {
    goal_clone!();
    fn name(&self) -> &'static str {
        "FollowParent"
    }
    fn flags(&self) -> u8 {
        MOVE | LOOK
    }
    fn can_use(&mut self, m: &mut Mob, cx: &mut Cx) -> bool {
        if !m.is_baby() {
            return false;
        }
        let (id, own) = (m.id, self.own.clone());
        let at = m.position;
        let best = cx
            .w
            .nearby(at, 8.0, move |s| s.id != id && !s.baby && s.alive && s.custom.as_deref() == Some(&*own))
            .map(|s| (s.id, s.position.distance_squared(at)))
            .min_by(|a, b| a.1.total_cmp(&b.1));
        match best {
            Some((p, d)) if d >= 9.0 => {
                self.parent = Some(p);
                true
            }
            _ => false,
        }
    }
    fn can_continue(&mut self, m: &mut Mob, cx: &mut Cx) -> bool {
        if !m.is_baby() {
            return false;
        }
        let Some(p) = self.parent.and_then(|p| cx.w.seen(p)) else {
            return false;
        };
        let d = m.distance_sqr(p.position);
        p.alive && (9.0..=256.0).contains(&d)
    }
    fn start(&mut self, _: &mut Mob, _: &mut Cx) {
        self.recalc = 0;
    }
    fn stop(&mut self, _: &mut Mob, _: &mut Cx) {
        self.parent = None;
    }
    fn tick(&mut self, m: &mut Mob, cx: &mut Cx) {
        self.recalc -= 1;
        if self.recalc <= 0
            && let Some(p) = self.parent.and_then(|p| cx.w.seen(p)).map(|s| s.position)
        {
            self.recalc = reduced(10);
            m.nav_move_to_entity(cx.w, [p.x, p.y, p.z], 0, self.speed);
        }
    }
}
