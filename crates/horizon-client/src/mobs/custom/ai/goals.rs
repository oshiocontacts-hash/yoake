//! Movement, idle and defensive data goals. Each wraps the engine's own goal where one exists
//! (`Float`, `Stroll`, `LookAtPlayer`, `RandomLookAround`, `Panic`, `Tempt`, `AvoidEntity`,
//! `LeapAtTarget`, `MoveToBlock`); the ones the engine had no generic version of are small new
//! goals here: [`LookAtTarget`], [`RetreatWhenHurt`], [`GoToBlock`], [`StayNearHome`].
use super::Params;
use crate::mobs::{
    Mob, MobWorld,
    goals::{
        Avoid, AvoidEntity, Cx, Float, Goal, LOOK, LeapAtTarget, LookAtPlayer, MOVE, MoveToBlock,
        Panic, RandomLookAround, Stroll, Tempt, goal_clone, reduced,
    },
};
use horizon_mods::custom_mob::Goal as GoalDef;
use std::sync::Arc;

/// Adds the goal `g` names if it is one of this module's; false for any other type.
pub fn add(m: &mut Mob, g: &GoalDef) -> bool {
    let p = Params(g);
    let prio = p.priority();
    match g.kind.as_str() {
        "float" | "swim" | "float_in_water" => m.ai.goals.add(prio, Float),
        "wander" | "stroll" => m.ai.goals.add(
            prio,
            Stroll::avoiding_water(p.speed(1.0)).interval(p.int("interval", 120).max(1)),
        ),
        "random_swim" => m.ai.goals.add(
            prio,
            Stroll::swimming(p.speed(1.0), p.int("interval", 40).max(1)),
        ),
        "fly_wander" => m.ai.goals.add(
            prio,
            Stroll::flying(p.speed(1.0)).interval(p.int("interval", 120).max(1)),
        ),
        "look_at_player" => m
            .ai
            .goals
            .add(prio, LookAtPlayer::new(p.num("range", 8.0) as f32)),
        "random_look" => m.ai.goals.add(prio, RandomLookAround::default()),
        "look_at_target" => m
            .ai
            .goals
            .add(prio, LookAtTarget::new(p.num("range", 16.0) as f32)),
        "panic" => m.ai.goals.add(prio, Panic::new(p.speed(1.25))),
        "tempt" => {
            let items: Arc<[String]> = p.strings("items").into();
            m.ai.goals.add(
                prio,
                Tempt::new(p.speed(1.0), items, p.flag("can_scare", false)),
            );
        }
        "flee" | "avoid" => {
            let from = p.strings("from");
            let (list, player, _) = super::species_list(&from);
            let what = if from.is_empty() || player {
                Avoid::Player
            } else {
                Avoid::Species(list)
            };
            m.ai.goals.add(
                prio,
                AvoidEntity::new(
                    what,
                    p.num("distance", 8.0) as f32,
                    p.num("walk_speed", 1.0),
                    p.num("sprint_speed", 1.2),
                ),
            );
        }
        "retreat_when_hurt" => m.ai.goals.add(
            prio,
            RetreatWhenHurt::new(
                p.speed(1.3),
                p.num("health_below", 0.3).clamp(0.0, 1.0) as f32,
                p.num("distance", 16.0).clamp(2.0, 64.0) as i32,
            ),
        ),
        "go_to_block" => {
            m.ai.goals.add(
                prio,
                GoToBlock::new(
                    p.speed(1.0),
                    p.strings("blocks"),
                    p.int("range", 8).clamp(1, 32),
                    p.int("vertical_range", 3).clamp(0, 16),
                    p.int("interval", 200).max(1),
                ),
            );
        }
        "stay_near_home" => {
            let radius = p.int("radius", 16).clamp(1, 128);
            set_home(m, radius);
            m.ai.goals.add(prio, StayNearHome::new(p.speed(1.0)));
        }
        "patrol_around" => {
            let radius = p.int("radius", 12).clamp(1, 128);
            set_home(m, radius);
            m.ai.goals.add(
                prio,
                Stroll::avoiding_water(p.speed(1.0)).interval(p.int("interval", 60).max(1)),
            );
        }
        // Spider-style wall climbing: the type has the engine's climber physics (see
        // `build::mob_type`); the navigator may route up a wall at `cost` extra per block.
        "climb" => m.ai.nav.climb_cost = Some(p.num("cost", 0.5).clamp(0.0, 32.0) as f32),
        "leap_at_target" => m.ai.goals.add(
            prio,
            LeapAtTarget::new(p.num("yd", 0.4).clamp(0.0, 2.0) as f32),
        ),
        _ => return false,
    }
    true
}

/// A mob that stays near where it was made (or loaded): the engine's `home` restriction, which
/// the stroll goals and `DefaultRandomPos` respect.
fn set_home(m: &mut Mob, radius: i32) {
    let c = [
        m.position.x.floor() as i32,
        m.position.y.floor() as i32,
        m.position.z.floor() as i32,
    ];
    m.ai.home = Some((c, radius));
}

/// Looks at the mob's attack target while it has one within `range`.
#[derive(Clone, Debug)]
pub struct LookAtTarget {
    range: f32,
}
impl LookAtTarget {
    pub fn new(range: f32) -> Self {
        Self { range }
    }
    fn near(&self, m: &Mob, cx: &Cx) -> bool {
        m.target_info(cx.w).is_some_and(|t| {
            t.alive && m.distance_sqr(t.position) <= f64::from(self.range * self.range)
        })
    }
}
impl Goal for LookAtTarget {
    goal_clone!();
    fn flags(&self) -> u8 {
        LOOK
    }
    fn can_use(&mut self, m: &mut Mob, cx: &mut Cx) -> bool {
        self.near(m, cx)
    }
    fn every_tick(&self) -> bool {
        true
    }
    fn tick(&mut self, m: &mut Mob, cx: &mut Cx) {
        if let Some(t) = m.target_info(cx.w) {
            m.look_at_speed([t.position.x, t.eye_y, t.position.z], 30.0, 30.0);
        }
    }
}

/// Runs away from the mob's target (or whatever hurt it last) once its health is at or below
/// `health_below` of the maximum. Takes `MOVE`, so it replaces a chase or an attack of a higher
/// priority number.
#[derive(Clone, Debug)]
pub struct RetreatWhenHurt {
    speed: f64,
    below: f32,
    distance: i32,
}
impl RetreatWhenHurt {
    pub fn new(speed: f64, below: f32, distance: i32) -> Self {
        Self {
            speed,
            below,
            distance,
        }
    }
    /// Where the threat is: the target, else where the last hit came from (within 100 ticks).
    fn threat(m: &Mob, w: &MobWorld) -> Option<[f64; 3]> {
        if let Some(t) = m.target_info(w).filter(|t| t.alive) {
            return Some([t.position.x, t.position.y, t.position.z]);
        }
        m.last_damage
            .filter(|(_, at)| i64::from(m.age) - at <= 100)
            .and_then(|(d, _)| d.from)
    }
    fn hurt(&self, m: &Mob) -> bool {
        m.attrs.max_health > 0.0 && m.health / m.attrs.max_health <= self.below
    }
}
impl Goal for RetreatWhenHurt {
    goal_clone!();
    fn flags(&self) -> u8 {
        MOVE
    }
    fn can_use(&mut self, m: &mut Mob, cx: &mut Cx) -> bool {
        if !self.hurt(m) {
            return false;
        }
        let Some(from) = Self::threat(m, cx.w) else {
            return false;
        };
        let Some(pos) = m.default_random_pos_away(cx.w, self.distance, 7, from) else {
            return false;
        };
        let path = m.create_path_to(
            cx.w,
            [
                pos[0].floor() as i32,
                pos[1].floor() as i32,
                pos[2].floor() as i32,
            ],
            0,
        );
        m.ai.pending_path = path;
        m.ai.pending_path.is_some()
    }
    fn can_continue(&mut self, m: &mut Mob, _: &mut Cx) -> bool {
        !m.ai.nav.is_done()
    }
    fn start(&mut self, m: &mut Mob, cx: &mut Cx) {
        let path = m.ai.pending_path.take();
        m.nav_move_to_path(cx.w, path, self.speed);
        m.ai.panicking = true;
    }
    fn stop(&mut self, m: &mut Mob, _: &mut Cx) {
        m.ai.panicking = false;
    }
}

/// `MoveToBlockGoal` for a list of block ids: walks to the nearest of them within `range`
/// (every `interval` ticks it looks again) and stays by it. The block is the one below the spot
/// the mob stands on, as the engine's `MoveToBlock` has it.
#[derive(Clone, Debug)]
pub struct GoToBlock {
    inner: MoveToBlock,
    blocks: Vec<String>,
    interval: i32,
}
impl GoToBlock {
    pub fn new(speed: f64, blocks: Vec<String>, range: i32, vertical: i32, interval: i32) -> Self {
        Self {
            inner: MoveToBlock::new(speed, range, vertical),
            blocks,
            interval,
        }
    }
    fn valid(blocks: &[String], w: &MobWorld, p: [i32; 3]) -> bool {
        let name = w.block_name(p[0], p[1], p[2]);
        blocks.iter().any(|b| b == name)
    }
    /// Whether the mob has reached its block.
    pub fn reached(&self) -> bool {
        self.inner.reached
    }
}
impl Goal for GoToBlock {
    goal_clone!();
    fn flags(&self) -> u8 {
        MOVE
    }
    fn can_use(&mut self, m: &mut Mob, cx: &mut Cx) -> bool {
        let blocks = &self.blocks;
        let next = reduced(self.interval);
        self.inner
            .can_use(m, cx.w, next, |w, p| Self::valid(blocks, w, p))
    }
    fn can_continue(&mut self, _: &mut Mob, cx: &mut Cx) -> bool {
        let blocks = &self.blocks;
        self.inner
            .can_continue(cx.w, |w, p| Self::valid(blocks, w, p))
    }
    fn start(&mut self, m: &mut Mob, cx: &mut Cx) {
        self.inner.start(m, cx.w);
    }
    fn every_tick(&self) -> bool {
        true
    }
    fn tick(&mut self, m: &mut Mob, cx: &mut Cx) {
        self.inner.tick(m, cx.w);
    }
    fn stop(&mut self, m: &mut Mob, _: &mut Cx) {
        m.nav_stop();
    }
}

/// `MoveTowardsRestrictionGoal`: when the mob is outside its home area, walks back toward it.
#[derive(Clone, Debug)]
pub struct StayNearHome {
    speed: f64,
    wanted: [f64; 3],
}
impl StayNearHome {
    pub fn new(speed: f64) -> Self {
        Self {
            speed,
            wanted: [0.0; 3],
        }
    }
    fn inside(m: &Mob) -> bool {
        let Some((c, r)) = m.ai.home else {
            return true;
        };
        let p = [
            m.position.x.floor() as i32,
            m.position.y.floor() as i32,
            m.position.z.floor() as i32,
        ];
        crate::mobs::goals::within_home(c, r, p)
    }
}
impl Goal for StayNearHome {
    goal_clone!();
    fn flags(&self) -> u8 {
        MOVE
    }
    fn can_use(&mut self, m: &mut Mob, cx: &mut Cx) -> bool {
        if Self::inside(m) {
            return false;
        }
        let Some((c, _)) = m.ai.home else {
            return false;
        };
        let to = [
            f64::from(c[0]) + 0.5,
            f64::from(c[1]),
            f64::from(c[2]) + 0.5,
        ];
        match m.default_random_pos_towards(cx.w, 16, 7, to, std::f64::consts::FRAC_PI_2) {
            Some(p) => {
                self.wanted = p;
                true
            }
            None => false,
        }
    }
    fn can_continue(&mut self, m: &mut Mob, _: &mut Cx) -> bool {
        !m.ai.nav.is_done()
    }
    fn start(&mut self, m: &mut Mob, cx: &mut Cx) {
        m.nav_move_to(cx.w, self.wanted, self.speed);
    }
}
