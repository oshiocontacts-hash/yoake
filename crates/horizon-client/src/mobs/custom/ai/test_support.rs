//! Test fixtures for the custom AI: a flat world that needs no game resources (the synthetic one
//! the mob hook bench uses: a stone floor at `y = 0` over 81 by 81 blocks, air above) and custom
//! mobs built from a JSON `ai` block.
use crate::mobs::{
    Mob, MobEvent, MobWorld, Seen, TargetInfo, Who,
    custom::{CustomType, build},
    goals::Cx,
    mob_hook_bench::BenchWorld,
};
use horizon_core::{geometry::{Aabb, Vec3}, random::LegacyRandom};
use horizon_mods::custom_mob::CustomMobFile;
use std::sync::Arc;

/// The flat world. Mobs stand on `y = 1`.
pub fn flat() -> BenchWorld {
    BenchWorld::new()
}

/// A custom mob of `category` at `at` whose file has the given `ai` object (JSON text) and the
/// given extra top-level members (`,"hitboxes": [...]`).
pub fn mob_with(category: &str, ai: &str, extra: &str, at: Vec3) -> Mob {
    let json = format!(
        r#"{{"template":"yoake:mob","category":"{category}",
            "size":{{"width":0.6,"height":1.0,"eye_height":0.9}},
            "attributes":{{"max_health":20,"movement_speed":0.25,"attack_damage":3,"follow_range":16}},
            "ai":{ai}{extra}}}"#
    );
    let file = CustomMobFile::parse("demo:thing", json.as_bytes()).unwrap();
    let t = Arc::new(CustomType::new(file, None).unwrap());
    let kind = Arc::new(build::mob_type(&t));
    let mut m = Mob::new(1, kind, at, 0.0);
    // Standing on the floor, as the physics would have settled it.
    m.on_ground = at.y == 1.0;
    m
}

/// A mob of the default creature category.
pub fn mob(ai: &str, at: Vec3) -> Mob {
    mob_with("creature", ai, "", at)
}

/// The snapshot of a mob standing at `at` (what the goals of another mob see of it).
pub fn seen(id: u32, species: crate::mobs::Species, at: Vec3) -> Seen {
    let mut kind = (*crate::mobs::sensor_scan_tests::kind()).clone();
    kind.species = species;
    Mob::new(id, Arc::new(kind), at, 0.0).seen(None)
}

/// A world view with the given player and snapshots.
pub fn view<'a>(w: &MobWorld<'a>, others: &'a [Seen], player: Option<Vec3>) -> MobWorld<'a> {
    let mut v = w.clone();
    v.others = others;
    v.player = player.map(crate::mobs::world::test_player);
    v
}

/// A context over `w` for calling a goal directly.
pub struct Ctx {
    pub lr: LegacyRandom,
    pub events: Vec<MobEvent>,
    /// Ticks [`run`] has made with this context: the world clock is `game_time` plus this.
    pub elapsed: i64,
}
impl Ctx {
    pub fn new() -> Self {
        Self {
            lr: LegacyRandom::new(7),
            events: Vec::new(),
            elapsed: 0,
        }
    }
    pub fn cx<'a, 'w>(&'a mut self, w: &'a MobWorld<'w>) -> Cx<'a, 'w> {
        Cx {
            w,
            lr: &mut self.lr,
            events: &mut self.events,
        }
    }
}

/// Ticks `m` `n` times through the engine's whole mob tick. The world's clock advances too.
pub fn run(m: &mut Mob, w: &MobWorld, ctx: &mut Ctx, n: usize) {
    let mut w = w.clone();
    for _ in 0..n {
        w.game_time += ctx.elapsed;
        crate::mobs::ai::tick(m, &w, &mut ctx.lr, &mut ctx.events);
        w.game_time -= ctx.elapsed;
        ctx.elapsed += 1;
    }
}

/// A target's `TargetInfo` of a plain 0.6 by 1.8 box at `at`.
pub fn target_info(at: Vec3) -> TargetInfo {
    TargetInfo {
        position: at,
        eye_y: at.y + 1.6,
        bounds: Aabb::new(
            Vec3::new(at.x - 0.3, at.y, at.z - 0.3),
            Vec3::new(at.x + 0.3, at.y + 1.8, at.z + 0.3),
        ),
        alive: true,
        can_be_seen_as_enemy: true,
        is_player: false,
        creative: false,
        passenger: false,
    }
}

/// Whether any event satisfies `f`.
pub fn any(events: &[MobEvent], f: impl Fn(&MobEvent) -> bool) -> bool {
    events.iter().any(f)
}

pub fn count(events: &[MobEvent], f: impl Fn(&MobEvent) -> bool) -> usize {
    events.iter().filter(|e| f(e)).count()
}

/// `Who` of the player.
pub const PLAYER: Who = Who::Player;

/// One evaluation of the mob's target and goal selectors (no movement or physics): what
/// `server_ai_step` does between the two.
pub fn select(m: &mut Mob, w: &MobWorld, ctx: &mut Ctx) {
    let mut targets = std::mem::take(&mut m.ai.targets);
    let mut goals = std::mem::take(&mut m.ai.goals);
    {
        let mut cx = ctx.cx(w);
        targets.tick(m, &mut cx);
        goals.tick(m, &mut cx);
    }
    m.ai.targets = targets;
    m.ai.goals = goals;
}

/// Runs [`select`] until `done` holds (at most `limit` times); the number of rounds, or `None`.
pub fn select_until(
    m: &mut Mob,
    w: &MobWorld,
    ctx: &mut Ctx,
    limit: usize,
    done: impl Fn(&Mob) -> bool,
) -> Option<usize> {
    for i in 0..limit {
        select(m, w, ctx);
        if done(m) {
            return Some(i + 1);
        }
    }
    None
}
