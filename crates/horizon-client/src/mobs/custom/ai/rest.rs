//! `sleep_in_bed_like`: a rest goal for a custom mob.
//!
//! At night (the sky is dark and the dimension has one), or whenever the row's `when` expression
//! is true, the mob walks to its rest place and stays there: it stops moving, and a tracked value
//! of the type (`state`, default `sleep`: a bool, or an i32 that is 1 while resting) says so for
//! the animation and the client. It wakes when it is hurt, when day comes, or when the condition
//! turns false; a hurt mob does not go back to rest for 100 ticks.
//!
//! The rest place is the nearest block of the block tag `block_tag` within `range` (default 16)
//! when the row names one, else the mob's home: the home of a `stay_near_home` or `patrol_around`
//! row, or the place it first thought of resting from (where it was made, or loaded).
//!
//! Parameters: `when` (expression), `state` (tracked value, default `sleep`), `block_tag`,
//! `range`, `speed` (default 1.0), `reach` (how near, in blocks, counts as there; default 1.5).
use super::{Params, expr::When};
use crate::mobs::{
    Mob, MobWorld,
    custom::{CustomType, SyncedValue},
    goals::{Cx, Goal, JUMP, LOOK, MOVE, goal_clone},
};
use horizon_mods::custom_mob::Goal as GoalDef;
use std::sync::Arc;

/// Ticks after a hurt in which the mob does not rest.
const HURT_MEMORY: i64 = 100;

/// Adds the goal `g` names if it is `sleep_in_bed_like`; false for any other type.
pub fn add(m: &mut Mob, t: &CustomType, g: &GoalDef) -> bool {
    if g.kind != "sleep_in_bed_like" {
        return false;
    }
    let p = Params(g);
    let when = match p.text("when").map(|s| When::compile(t, s)) {
        None => None,
        Some(Ok(w)) => Some(w),
        Some(Err(e)) => {
            log::warn!("custom mob {}: sleep_in_bed_like `when`: {e}; the goal is left out", t.id);
            return true;
        }
    };
    let state = p.text("state").unwrap_or("sleep").to_owned();
    if t.synced_slot(&state).is_none() {
        log::warn!(
            "custom mob {}: sleep_in_bed_like has no tracked value named \"{state}\"; it rests without showing it",
            t.id
        );
    }
    let tag = p.text("block_tag").map(Arc::<str>::from);
    m.ai.goals.add(
        p.priority(),
        Rest {
            when,
            state,
            tag,
            range: p.int("range", 16).clamp(1, 64),
            speed: p.speed(1.0),
            reach: p.num("reach", 1.5).clamp(0.5, 8.0),
            home: None,
            place: None,
            asleep: false,
            health: 0.0,
        },
    );
    true
}

/// The rest goal.
#[derive(Clone, Debug)]
pub struct Rest {
    when: Option<When>,
    state: String,
    tag: Option<Arc<str>>,
    range: i32,
    speed: f64,
    reach: f64,
    /// Where the mob first thought of resting, when it has no other home.
    home: Option<[i32; 3]>,
    /// Where it is going to rest.
    place: Option<[i32; 3]>,
    asleep: bool,
    /// Its health when it lay down; less later means hurt.
    health: f32,
}

impl Rest {
    fn night(w: &MobWorld) -> bool {
        !w.daylight && !w.nether && !w.end
    }
    fn wants(&self, m: &Mob, w: &MobWorld) -> bool {
        match &self.when {
            Some(e) => e.eval(m, w),
            None => Self::night(w),
        }
    }
    fn recently_hurt(m: &Mob) -> bool {
        m.hurt_time > 0
            || m.last_damage
                .is_some_and(|(_, at)| i64::from(m.age) - at <= HURT_MEMORY)
    }
    /// The place to rest at: the nearest block of the tag, else the home.
    fn find_place(&mut self, m: &Mob, w: &MobWorld) -> Option<[i32; 3]> {
        let here = [
            m.position.x.floor() as i32,
            m.position.y.floor() as i32,
            m.position.z.floor() as i32,
        ];
        if let Some(tag) = &self.tag {
            let range_y = (self.range / 2).max(2);
            return w.nearest_block(here, self.range, range_y, |_, name| w.has_tag(tag, name));
        }
        if let Some((c, _)) = m.ai.home {
            return Some(c);
        }
        Some(*self.home.get_or_insert(here))
    }
    fn near(&self, m: &Mob, p: [i32; 3]) -> bool {
        let dx = m.position.x - (f64::from(p[0]) + 0.5);
        let dz = m.position.z - (f64::from(p[2]) + 0.5);
        let dy = m.position.y - f64::from(p[1]);
        dx * dx + dz * dz <= self.reach * self.reach && dy.abs() <= 2.0
    }
    fn show(&self, m: &mut Mob, on: bool) {
        let (Some(t), Some(c)) = (m.kind.custom.clone(), m.extra.custom.as_mut()) else {
            return;
        };
        let Some(slot) = t.synced_slot(&self.state) else {
            return;
        };
        let value = match t.synced[slot].kind.as_str() {
            "bool" => SyncedValue::Bool(on),
            "i32" => SyncedValue::I32(i32::from(on)),
            _ => return,
        };
        c.set_synced(&t, &self.state, value);
    }
}

impl Goal for Rest {
    goal_clone!();
    fn flags(&self) -> u8 {
        MOVE | LOOK | JUMP
    }
    fn can_use(&mut self, m: &mut Mob, cx: &mut Cx) -> bool {
        if !self.wants(m, cx.w) || Self::recently_hurt(m) {
            return false;
        }
        self.place = self.find_place(m, cx.w);
        self.place.is_some()
    }
    fn can_continue(&mut self, m: &mut Mob, cx: &mut Cx) -> bool {
        if self.asleep && m.health < self.health {
            return false;
        }
        self.wants(m, cx.w) && !Self::recently_hurt(m)
    }
    fn start(&mut self, m: &mut Mob, cx: &mut Cx) {
        self.asleep = false;
        if let Some(p) = self.place
            && !self.near(m, p)
        {
            m.nav_move_to(cx.w, [f64::from(p[0]) + 0.5, f64::from(p[1]), f64::from(p[2]) + 0.5], self.speed);
        }
    }
    fn every_tick(&self) -> bool {
        true
    }
    fn tick(&mut self, m: &mut Mob, cx: &mut Cx) {
        let Some(p) = self.place else { return };
        if self.asleep {
            m.nav_stop();
            return;
        }
        if self.near(m, p) {
            m.nav_stop();
            self.asleep = true;
            self.health = m.health;
            self.show(m, true);
        } else if m.ai.nav.is_done() && cx.w.game_time % 20 == 0 {
            // Not there yet and no path (it was cut off or never found): try again.
            m.nav_move_to(cx.w, [f64::from(p[0]) + 0.5, f64::from(p[1]), f64::from(p[2]) + 0.5], self.speed);
        }
    }
    fn stop(&mut self, m: &mut Mob, _: &mut Cx) {
        if self.asleep {
            self.show(m, false);
        }
        self.asleep = false;
        m.nav_stop();
    }
}
