//! Goals that deal with blocks, for a custom mob: `open_door` and `eat_block`. The vanilla sheep
//! and raider goals they resemble are untouched (`doors.rs` only gained a door tag the old code
//! does not set).
//!
//! * `open_door` `{ door_tag, break }`: opens the doors (blocks of the block tag `door_tag`, default
//!   `minecraft:wooden_doors`) that are in the mob's way, holds them open while it is near and
//!   closes them behind it. With `"break": true` (off by default) it breaks them down instead
//!   (240 ticks, when mob griefing is on). The mob's navigation is allowed through doors.
//! * `eat_block` `{ blocks, interval, cooldown, effect, duration, trigger }`: now and then (a
//!   1-in-`interval` chance, default 100, never within `cooldown` ticks, default 600, of the last
//!   meal) the mob stops for `duration` ticks (default 40) and eats the block it stands in or
//!   the one below it. `blocks` maps block ids to what is left, `{ "minecraft:grass_block":
//!   "minecraft:dirt" }`, or is a list of ids that are left as air. The blocks change only when
//!   mob griefing is on; the effect always happens: `{ "heal": 4 }` (health), `"grow_up"` (a baby
//!   ages up a minute), `{ "event": "name" }` or just `"name"` (a custom event for the mod, taken
//!   with [`take_events`]), several of them in one object. `trigger` asks for an animation.
use super::Params;
use crate::mobs::{
    Mob, MobEvent,
    custom::build::intern,
    doors::{BreakDoor, OpenTaggedDoor},
    goals::{Cx, Goal, JUMP, LOOK, MOVE, goal_clone, reduced},
};
use horizon_core::random::RandomSource;
use horizon_mods::custom_mob::Goal as GoalDef;
use serde_json::Value;
use std::sync::Arc;

/// The most custom events a mob keeps untaken.
const KEPT: usize = 8;

/// The custom events `m`'s goals raised since the last call (an `eat_block` effect), oldest first.
pub fn take_events(m: &mut Mob) -> Vec<String> {
    m.extra
        .custom
        .as_mut()
        .map(|c| std::mem::take(&mut c.events))
        .unwrap_or_default()
}

/// Adds the goal `g` names if it is `open_door` or `eat_block`; false for any other type.
pub fn add(m: &mut Mob, g: &GoalDef) -> bool {
    let p = Params(g);
    match g.kind.as_str() {
        "open_door" => {
            let tag: Arc<str> = Arc::from(p.text("door_tag").unwrap_or("minecraft:wooden_doors"));
            if crate::mobs::doors::can_navigate_ground(m) {
                m.ai.nav.can_open_doors = true;
            }
            if p.flag("break", false) {
                m.ai.goals.add(p.priority(), BreakDoor::tagged(tag));
            } else {
                m.ai.goals.add(p.priority(), OpenTaggedDoor::new(tag));
            }
            true
        }
        "eat_block" => {
            m.ai.goals.add(p.priority(), EatCustomBlock::from_row(p));
            true
        }
        _ => false,
    }
}

/// What a block turns into when it is eaten.
#[derive(Clone, Copy, Debug, PartialEq)]
enum Left {
    Air,
    Block(&'static str),
}

/// What eating does besides changing the block.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Effect {
    heal: f32,
    grow_up: bool,
    event: Option<String>,
}
impl Effect {
    fn parse(v: Option<&Value>) -> Self {
        let mut e = Self::default();
        match v {
            Some(Value::String(s)) if s == "grow_up" => e.grow_up = true,
            Some(Value::String(s)) if !s.is_empty() => e.event = Some(s.clone()),
            Some(Value::Object(o)) => {
                e.heal = o
                    .get("heal")
                    .and_then(Value::as_f64)
                    .filter(|h| h.is_finite())
                    .unwrap_or(0.0)
                    .clamp(0.0, 1024.0) as f32;
                e.grow_up = o.get("grow_up").and_then(Value::as_bool).unwrap_or(false);
                e.event = o
                    .get("event")
                    .and_then(Value::as_str)
                    .filter(|s| !s.is_empty())
                    .map(str::to_owned);
            }
            _ => {}
        }
        e
    }
}

/// `eat_block`.
#[derive(Clone, Debug)]
pub struct EatCustomBlock {
    blocks: Vec<(String, Left)>,
    interval: i32,
    cooldown: i64,
    duration: i32,
    effect: Effect,
    trigger: Option<String>,
    ready_at: i64,
    tick: i32,
}

impl EatCustomBlock {
    fn from_row(p: Params) -> Self {
        let mut blocks = Vec::new();
        match p.0.params.get("blocks") {
            Some(Value::Object(o)) => {
                for (from, to) in o {
                    let left = match to.as_str() {
                        None | Some("minecraft:air") | Some("") => Left::Air,
                        Some(name) => Left::Block(intern(name)),
                    };
                    blocks.push((from.clone(), left));
                }
            }
            _ => blocks.extend(p.strings("blocks").into_iter().map(|b| (b, Left::Air))),
        }
        Self {
            blocks,
            interval: p.int("interval", 100).max(1),
            cooldown: i64::from(p.int("cooldown", 600).max(0)),
            duration: p.int("duration", 40).clamp(5, 1200),
            effect: Effect::parse(p.0.params.get("effect")),
            trigger: p.text("trigger").map(str::to_owned),
            ready_at: 0,
            tick: 0,
        }
    }
    /// The block the mob would eat: where it stands, else the one below.
    fn find(&self, m: &Mob, cx: &Cx) -> Option<([i32; 3], Left)> {
        let at = [
            m.position.x.floor() as i32,
            m.position.y.floor() as i32,
            m.position.z.floor() as i32,
        ];
        for p in [at, [at[0], at[1] - 1, at[2]]] {
            let name = cx.w.block_name(p[0], p[1], p[2]);
            if let Some((_, left)) = self.blocks.iter().find(|(b, _)| b == name) {
                return Some((p, *left));
            }
        }
        None
    }
}

impl Goal for EatCustomBlock {
    goal_clone!();
    fn flags(&self) -> u8 {
        MOVE | LOOK | JUMP
    }
    fn can_use(&mut self, m: &mut Mob, cx: &mut Cx) -> bool {
        if cx.w.game_time < self.ready_at || self.blocks.is_empty() {
            return false;
        }
        if m.random.next_int_bound(reduced(self.interval)).unwrap_or(1) != 0 {
            return false;
        }
        self.find(m, cx).is_some()
    }
    fn can_continue(&mut self, _: &mut Mob, _: &mut Cx) -> bool {
        self.tick > 0
    }
    fn start(&mut self, m: &mut Mob, _: &mut Cx) {
        self.tick = reduced(self.duration);
        m.nav_stop();
        if let (Some(t), Some(c)) = (&self.trigger, m.extra.custom.as_mut()) {
            c.attack.request_trigger(t);
        }
    }
    fn stop(&mut self, _: &mut Mob, _: &mut Cx) {
        self.tick = 0;
    }
    fn every_tick(&self) -> bool {
        true
    }
    fn tick(&mut self, m: &mut Mob, cx: &mut Cx) {
        self.tick = (self.tick - 1).max(0);
        if self.tick != reduced(4) {
            return;
        }
        let Some((p, left)) = self.find(m, cx) else {
            return;
        };
        if cx.w.mob_griefing {
            match left {
                Left::Air => cx.events.push(MobEvent::BreakBlock { pos: p, drop: false }),
                Left::Block(name) => cx.events.push(MobEvent::SetBlockName {
                    mob: Some(m.id),
                    pos: p,
                    name,
                    effect: true,
                    flags: horizon_world::level::flags::UPDATE_CLIENTS,
                }),
            }
        }
        self.ready_at = cx.w.game_time + self.cooldown;
        if self.effect.heal > 0.0 {
            m.health = (m.health + self.effect.heal).min(m.attrs.max_health);
        }
        if self.effect.grow_up && crate::mobs::ai::can_age_up(m) {
            crate::mobs::ai::age_up(m, 60, false);
        }
        if let (Some(name), Some(c)) = (&self.effect.event, m.extra.custom.as_mut()) {
            if c.events.len() >= KEPT {
                c.events.remove(0);
            }
            c.events.push(name.clone());
        }
    }
}
