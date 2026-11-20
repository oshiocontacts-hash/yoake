//! Tests of the script goal and the script side of custom mobs (F4M-10) with a recording stand-in for
//! the WASM runtime (the real guest is run by `wasm_mods::mob_runtime`'s tests).
use super::test_support::*;
use crate::mobs::{
    Mob, MobEvent, Who,
    custom::{
        Notice, SyncedValue,
        script::{self, Command, HurtReply, InteractReply, MobBoneWrite, MobScriptValue, Runtime, ScriptVec3},
    },
    goals::{JUMP, LOOK, MOVE},
};
use horizon_core::geometry::Vec3;
use std::sync::{
    Arc, Mutex,
    atomic::{AtomicBool, Ordering},
};

static SERIAL: Mutex<()> = Mutex::new(());

/// A runtime that records what it was asked and answers as told.
#[derive(Default)]
struct Mock {
    calls: Mutex<Vec<String>>,
    live: AtomicBool,
    can_use: AtomicBool,
    can_continue: AtomicBool,
    /// Commands to queue for the asking mob when `goal-start` is called.
    on_start: Mutex<Vec<Command>>,
}

impl Mock {
    fn new() -> Arc<Self> {
        let m = Arc::new(Self::default());
        m.live.store(true, Ordering::SeqCst);
        m.can_continue.store(true, Ordering::SeqCst);
        m
    }
    fn log(&self, s: String) {
        self.calls.lock().unwrap().push(s);
    }
    fn count(&self, prefix: &str) -> usize {
        self.calls.lock().unwrap().iter().filter(|c| c.starts_with(prefix)).count()
    }
    fn all(&self) -> Vec<String> {
        self.calls.lock().unwrap().clone()
    }
}

impl Runtime for Mock {
    fn live(&self) -> bool {
        self.live.load(Ordering::SeqCst)
    }
    fn goal_can_use(&self, mob: u32, goal: &str) -> Option<bool> {
        self.log(format!("can_use {mob} {goal}"));
        Some(self.can_use.load(Ordering::SeqCst))
    }
    fn goal_can_continue(&self, mob: u32, goal: &str) -> Option<bool> {
        self.log(format!("can_continue {mob} {goal}"));
        Some(self.can_continue.load(Ordering::SeqCst))
    }
    fn goal_start(&self, mob: u32, goal: &str) {
        self.log(format!("start {mob} {goal}"));
        for c in self.on_start.lock().unwrap().drain(..) {
            script::push(mob, c);
        }
    }
    fn goal_tick(&self, mob: u32, goal: &str) {
        self.log(format!("tick {mob} {goal}"));
    }
    fn goal_stop(&self, mob: u32, goal: &str) {
        self.log(format!("stop {mob} {goal}"));
    }
    fn mob_tick(&self, mob: u32) {
        self.log(format!("mob_tick {mob}"));
    }
    fn mob_spawn_check(&self, type_id: &str, _: &str, _: [i32; 3]) -> Option<bool> {
        self.log(format!("spawn_check {type_id}"));
        Some(true)
    }
    fn mob_spawned(&self, mob: u32, _: &str, reason: &str) {
        self.log(format!("spawned {mob} {reason}"));
    }
    fn mob_hurt(&self, mob: u32, part: &str, _: &str, amount: f32) -> Option<HurtReply> {
        self.log(format!("hurt {mob} {part}"));
        Some(HurtReply { amount: amount * 2.0, cancel: false })
    }
    fn mob_died(&self, mob: u32, _: &str, _: Option<u32>) {
        self.log(format!("died {mob}"));
    }
    fn mob_interact(&self, mob: u32, _: &str, _: &str, _: &str) -> InteractReply {
        self.log(format!("interact {mob}"));
        InteractReply::Success
    }
    fn mob_anim_event(&self, mob: u32, name: &str) {
        self.log(format!("anim_event {mob} {name}"));
    }
    fn mob_load(&self, mob: u32, data: &[u8]) {
        self.log(format!("load {mob} {}", data.len()));
    }
    fn mob_save(&self, mob: u32, data: &[u8]) -> Option<Vec<u8>> {
        self.log(format!("save {mob}"));
        Some(data.iter().rev().copied().collect())
    }
    fn mob_tamed(&self, mob: u32, owner: &str) {
        self.log(format!("tamed {mob} {owner}"));
    }
    fn mob_bred(&self, mob: u32, partner: u32, child: u32) {
        self.log(format!("bred {mob} {partner} {child}"));
    }
    fn synced_changed(&self, mob: u32, key: &str, old: &MobScriptValue, new: &MobScriptValue) {
        self.log(format!("changed {mob} {key} {old:?} {new:?}"));
    }
}

/// Installs `mock` for a test and removes it after (the runtime is process-wide).
struct Installed<'a>(std::sync::MutexGuard<'a, ()>);
fn install(mock: &Arc<Mock>) -> Installed<'static> {
    let g = SERIAL.lock().unwrap_or_else(|e| e.into_inner());
    script::reset();
    script::set_runtime(Some(mock.clone()));
    Installed(g)
}
impl Drop for Installed<'_> {
    fn drop(&mut self) {
        script::set_runtime(None);
        script::reset();
    }
}

fn run(m: &mut Mob, w: &crate::mobs::MobWorld, events: &mut Vec<MobEvent>, c: Command) -> bool {
    super::super::script_apply::apply_in(m, Some(w), Some(events), c)
}

fn at(x: f64, z: f64) -> Vec3 {
    Vec3::new(x, 1.0, z)
}

const POUNCE: &str = r#"{"goals":[{"type":"script","id":"pounce","priority":2},{"type":"wander","priority":6}],
    "script_goals":[{"id":"pounce","priority":2,"flags":["move","look"],"interval":5}]}"#;

/// A fox-like mob with the pounce goal, ticked at game time `t`.
fn tick(m: &mut Mob, fx: &crate::mobs::mob_hook_bench::BenchWorld, ctx: &mut Ctx, t: i64) {
    let mut w = fx.world();
    w.game_time = t;
    select(m, &w, ctx);
}

#[test]
fn the_script_goal_is_a_goal_with_the_flags_of_its_row() {
    let m = mob(POUNCE, at(0.5, 0.5));
    assert!(m.ai.goals.has("ScriptGoal"));
    assert_eq!(super::script_goal::flags_of(&["move".into(), "look".into(), "jump".into(), "target".into()]), MOVE | LOOK | JUMP | 8);
    assert_eq!(super::script_goal::flags_of(&["bogus".into()]), 0);
}

#[test]
fn can_use_is_asked_every_interval_ticks_only() {
    let mock = Mock::new();
    let _g = install(&mock);
    let fx = flat();
    let mut ctx = Ctx::new();
    let mut m = mob(POUNCE, at(0.5, 0.5));
    for t in 1000..1011 {
        tick(&mut m, &fx, &mut ctx, t);
    }
    // 1000, 1005, 1010.
    assert_eq!(mock.count("can_use"), 3);
    assert!(!m.ai.goals.is_running("ScriptGoal"));
}

#[test]
fn start_tick_and_stop_reach_the_guest_and_the_goal_holds_move_and_look() {
    let mock = Mock::new();
    let _g = install(&mock);
    mock.can_use.store(true, Ordering::SeqCst);
    let fx = flat();
    let mut ctx = Ctx::new();
    let mut m = mob(POUNCE, at(0.5, 0.5));
    tick(&mut m, &fx, &mut ctx, 1000);
    assert!(m.ai.goals.is_running("ScriptGoal"));
    assert!(!m.ai.goals.is_running("Stroll"), "wander needs MOVE, which the script goal holds");
    tick(&mut m, &fx, &mut ctx, 1001);
    assert_eq!(mock.count("start"), 1);
    assert!(mock.count("tick") >= 2);
    assert!(mock.count("can_continue") >= 1);
    mock.can_continue.store(false, Ordering::SeqCst);
    tick(&mut m, &fx, &mut ctx, 1002);
    assert_eq!(mock.count("stop"), 1);
    assert!(!m.ai.goals.is_running("ScriptGoal"));
}

#[test]
fn what_the_guest_asked_is_done_before_the_engine_goes_on() {
    let mock = Mock::new();
    let _g = install(&mock);
    mock.can_use.store(true, Ordering::SeqCst);
    *mock.on_start.lock().unwrap() = vec![Command::Jump, Command::Claim(true), Command::Impulse([0.0, 0.5, 0.0])];
    let fx = flat();
    let mut ctx = Ctx::new();
    let mut m = mob(POUNCE, at(0.5, 0.5));
    tick(&mut m, &fx, &mut ctx, 1000);
    assert!(m.ai.jumping, "the jump was asked in `start` and made before select returned");
    assert!(m.velocity.y >= 0.5 - 1e-9);
    assert_eq!(script::queued(), 0);
    assert!(m.extra.custom.as_ref().unwrap().script.claimed);
    // The flags reach the selector at the start of the mob's next tick, and go again when it lets go.
    let w = fx.world();
    script::pre_tick(&mut m, &w, &mut Vec::new());
    assert_eq!(m.ai.goals.disabled & (MOVE | LOOK | JUMP), MOVE | LOOK | JUMP);
    script::apply_pending(&mut m, None, None);
    script::push(m.id, Command::Claim(false));
    script::pre_tick(&mut m, &w, &mut Vec::new());
    assert_eq!(m.ai.goals.disabled & (MOVE | LOOK | JUMP), 0);
}

#[test]
fn a_mob_makes_at_most_32_goal_calls_in_a_tick() {
    // The budget counts per mob and tick.
    let mut st = script::ScriptState::default();
    assert!((0..32).all(|_| st.charge(1000)));
    assert!(!st.charge(1000));
    assert!(st.charge(1001), "a new tick, a new budget");
    // A mob that has spent 30 of 32 asks two of its ten goals (interval 1) and no more.
    let mock = Mock::new();
    let _g = install(&mock);
    let goals: Vec<String> = (0..10).map(|i| format!(r#"{{"type":"script","id":"g{i}","priority":{i}}}"#)).collect();
    let defs: Vec<String> = (0..10).map(|i| format!(r#"{{"id":"g{i}","priority":{i},"flags":[],"interval":1}}"#)).collect();
    let json = format!(r#"{{"goals":[{}],"script_goals":[{}]}}"#, goals.join(","), defs.join(","));
    let fx = flat();
    let mut ctx = Ctx::new();
    let mut m = mob(&json, at(0.5, 0.5));
    let s = script::state(&mut m).unwrap();
    s.eval_tick = 1000;
    s.evaluations = 30;
    tick(&mut m, &fx, &mut ctx, 1000);
    assert_eq!(mock.count("can_use"), 2);
    tick(&mut m, &fx, &mut ctx, 1001);
    assert_eq!(mock.count("can_use"), 12, "the next tick: all ten");
}

#[test]
fn a_running_goal_is_not_cut_off_by_the_budget() {
    let mock = Mock::new();
    let _g = install(&mock);
    mock.can_use.store(true, Ordering::SeqCst);
    let fx = flat();
    let mut ctx = Ctx::new();
    let mut m = mob(POUNCE, at(0.5, 0.5));
    tick(&mut m, &fx, &mut ctx, 1000);
    assert!(m.ai.goals.is_running("ScriptGoal"));
    // Spend the whole budget of the next tick, then let the selector ask `can-continue`.
    let spent = |m: &mut Mob| {
        let s = script::state(m).unwrap();
        s.eval_tick = 1001;
        s.evaluations = script::MAX_EVALUATIONS_PER_MOB_TICK;
    };
    spent(&mut m);
    let before = mock.count("can_continue");
    mock.can_continue.store(false, Ordering::SeqCst);
    tick(&mut m, &fx, &mut ctx, 1001);
    assert_eq!(mock.count("can_continue"), before, "not asked: the budget is spent");
    assert!(m.ai.goals.is_running("ScriptGoal"), "and the goal goes on");
}

#[test]
fn without_a_runtime_the_goal_never_starts_and_nothing_breaks() {
    let _g = SERIAL.lock().unwrap_or_else(|e| e.into_inner());
    script::set_runtime(None);
    let fx = flat();
    let mut ctx = Ctx::new();
    let mut m = mob(POUNCE, at(0.5, 0.5));
    for t in 1000..1020 {
        tick(&mut m, &fx, &mut ctx, t);
    }
    assert!(!m.ai.goals.is_running("ScriptGoal"));
    assert!(m.ai.goals.is_running("Stroll") || !m.ai.goals.is_running("ScriptGoal"));
}

#[test]
fn a_guest_that_dies_stops_its_goal_safely() {
    let mock = Mock::new();
    let _g = install(&mock);
    mock.can_use.store(true, Ordering::SeqCst);
    let fx = flat();
    let mut ctx = Ctx::new();
    let mut m = mob(POUNCE, at(0.5, 0.5));
    tick(&mut m, &fx, &mut ctx, 1000);
    assert!(m.ai.goals.is_running("ScriptGoal"));
    // The module trapped or timed out: the runtime is not live any more.
    mock.live.store(false, Ordering::SeqCst);
    tick(&mut m, &fx, &mut ctx, 1001);
    assert!(!m.ai.goals.is_running("ScriptGoal"), "can-continue answers no without a live guest");
    for t in 1002..1030 {
        tick(&mut m, &fx, &mut ctx, t);
    }
    assert!(!m.ai.goals.is_running("ScriptGoal"), "and it does not start again");
    assert_eq!(mock.count("start"), 1);
}

#[test]
fn the_tick_hooks_run_in_order_and_tell_each_thing_once() {
    let mock = Mock::new();
    let _g = install(&mock);
    let extra = r#","synced":{"state":{"type":"i32","default":0}}"#;
    let mut m = mob_with("creature", POUNCE, extra, at(0.5, 0.5));
    let fx = flat();
    let w = fx.world();
    {
        let c = m.extra.custom.as_mut().unwrap();
        c.script.data = vec![1, 2, 3];
        c.notice(Notice::Tamed { owner: [1, 2, 3, 4] });
        c.notice(Notice::Bred { partner: Some(9), child: None });
        c.script.push_event("hit");
        let t = m.kind.custom.clone().unwrap();
        assert!(c.set_synced(&t, "state", SyncedValue::I32(4)));
    }
    let mut events = Vec::new();
    assert!(script::pre_tick(&mut m, &w, &mut events));
    let calls = mock.all();
    assert_eq!(
        calls,
        [
            "load 1 3",
            "anim_event 1 hit",
            "changed 1 state Int(0) Int(4)",
            "tamed 1 00000001-0000-0002-0000-000300000004",
            "bred 1 9 0",
            "mob_tick 1",
        ]
    );
    // The second tick has only `mob-tick` to say.
    script::pre_tick(&mut m, &w, &mut events);
    assert_eq!(mock.all().len(), 7);
}

#[test]
fn a_value_that_does_not_change_is_not_a_change() {
    let t = {
        let extra = r#","synced":{"state":{"type":"i32","default":3}}"#;
        mob_with("creature", POUNCE, extra, at(0.5, 0.5))
    };
    let mut m = t;
    let ty = m.kind.custom.clone().unwrap();
    let c = m.extra.custom.as_mut().unwrap();
    assert!(c.set_synced(&ty, "state", SyncedValue::I32(3)));
    assert!(c.script.changes.is_empty());
    assert!(c.set_synced(&ty, "state", SyncedValue::I32(5)));
    assert_eq!(c.script.changes.len(), 1);
}

#[test]
fn commands_are_checked_and_applied() {
    let fx = flat();
    let w = fx.world();
    let extra = r#","synced":{"state":{"type":"i32","default":0}},"hitboxes":[{"name":"head","size":[0.4,0.4]}]"#;
    let mut m = mob_with("creature", POUNCE, extra, at(0.5, 0.5));
    let mut events = Vec::new();
    // A value of the wrong type, an unknown name: ignored, no panic.
    assert!(run(&mut m, &w, &mut events, Command::SetSynced { key: "state".into(), value: MobScriptValue::Bool(true) }));
    assert!(run(&mut m, &w, &mut events, Command::SetSynced { key: "nope".into(), value: MobScriptValue::Int(1) }));
    assert!(run(&mut m, &w, &mut events, Command::SetSynced { key: "state".into(), value: MobScriptValue::Int(7) }));
    assert_eq!(super::super::script_apply::get_synced(&m, "state"), Some(MobScriptValue::Int(7)));
    assert!(run(&mut m, &w, &mut events, Command::SetHitbox { name: "head".into(), enabled: false }));
    assert_eq!(m.extra.custom.as_ref().unwrap().hitbox_mask & 1, 0);
    assert!(run(&mut m, &w, &mut events, Command::PutData(vec![0; script::MAX_DATA_BYTES + 1])));
    assert!(m.extra.custom.as_ref().unwrap().script.data.is_empty(), "too much data is refused");
    assert!(run(&mut m, &w, &mut events, Command::PutData(vec![5, 6])));
    assert_eq!(m.extra.custom.as_ref().unwrap().script.data, [5, 6]);
    assert!(run(&mut m, &w, &mut events, Command::SetTarget { mob: Some(9), player: false }));
    assert_eq!(m.ai.target, Some(Who::Mob(9)));
    assert!(run(&mut m, &w, &mut events, Command::Attack { mob: None, player: true, damage: 3.0, knockback: 0.4 }));
    assert!(events.iter().any(|e| matches!(e, MobEvent::AttackPlayer(d, _) if (*d - 3.0).abs() < 1e-6)));
    assert!(events.iter().any(|e| matches!(e, MobEvent::Knockback { .. })));
    assert!(run(&mut m, &w, &mut events, Command::Impulse([f64::INFINITY, 0.0, 0.0])));
    assert!(m.velocity.x.is_finite());
    assert!(run(&mut m, &w, &mut events, Command::Despawn));
    assert!(m.removed);
}

#[test]
fn a_command_that_needs_a_world_waits_for_one() {
    let fx = flat();
    let w = fx.world();
    let mut m = mob(POUNCE, at(0.5, 0.5));
    // No world: put back; with a world: made.
    script::reset();
    script::push(m.id, Command::NavigateTo { position: [5.5, 1.0, 5.5], speed: 1.0 });
    script::apply_pending(&mut m, None, None);
    assert_eq!(script::queued(), 1);
    script::apply_pending(&mut m, Some(&w), None);
    assert_eq!(script::queued(), 0);
    script::reset();
}

#[test]
fn hurt_died_interact_and_save_go_through_the_runtime() {
    let mock = Mock::new();
    let _g = install(&mock);
    let mut m = mob(POUNCE, at(0.5, 0.5));
    let r = script::hurt(&mut m, "head", "PlayerAttack", 2.0).unwrap();
    assert_eq!(r, HurtReply { amount: 4.0, cancel: false });
    script::died(&mut m, Some(3));
    assert_eq!(script::interact(&mut m, "p", "main", "minecraft:stick"), InteractReply::Success);
    m.extra.custom.as_mut().unwrap().script.data = vec![1, 2, 3];
    assert_eq!(script::data_to_save(&m), [3, 2, 1]);
    script::spawned(&mut m, "egg");
    script::spawned(&mut m, "egg");
    assert_eq!(mock.count("spawned"), 1, "once per mob");
    assert!(mock.all().contains(&"hurt 1 head".to_string()));
}

#[test]
fn bone_writes_drop_the_invalid_and_keep_the_cap() {
    let v = |x: f32| Some(ScriptVec3 { x, y: 0.0, z: 0.0 });
    let list = vec![
        MobBoneWrite { bone: 0, additive: true, rotation_degrees: v(10.0), ..Default::default() },
        MobBoneWrite { bone: 1, translation: v(f32::NAN), ..Default::default() },
        MobBoneWrite { bone: 99, scale: v(1.0), ..Default::default() },
        MobBoneWrite { bone: 2, visible: Some(false), ..Default::default() },
    ];
    let out = script::bone_writes(&list, 3);
    assert_eq!(out.iter().map(|w| w.bone).collect::<Vec<_>>(), [0, 2]);
    assert_eq!(out[1].visible, Some(false));
    let many = vec![MobBoneWrite { bone: 0, visible: Some(true), ..Default::default() }; script::MAX_POSE_WRITES + 50];
    assert_eq!(script::bone_writes(&many, 1).len(), script::MAX_POSE_WRITES);
}

#[test]
fn uuid_text_is_the_canonical_form() {
    assert_eq!(script::uuid_text([-1, 0, 1, 0x1234_5678]), "ffffffff-0000-0000-0000-000112345678");
}
