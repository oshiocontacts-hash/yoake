//! Tests of the server animation runtime on the Cinder Fox fixture (the real loaders over its model,
//! animations and controllers) and the flat resource-free world.
use super::*;
use crate::mobs::{
    MobEvent, Who,
    custom::{CustomType, ai::test_support::*, build},
};
use horizon_core::{geometry::Vec3, random::LegacyRandom};
use horizon_mods::custom_mob::CustomMobFile;
use horizon_protocol::mob_sync::LayerAnchor;
use serde_json::{Value, json};
use std::sync::Arc;

const FOX: &str = include_str!(
    "../../../../../tools/fixtures/cinder_fox/data/sample_mobs/horizon/entity_type/cinder_fox.json"
);

fn root() -> std::path::PathBuf {
    std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../tools/fixtures/cinder_fox")
}

/// The Cinder Fox with `ai` replaced (and edited by `edit`), built as the game builds a type: the
/// real loaders over the fixture's files.
pub(crate) fn fox_type_with(ai: Value, edit: impl FnOnce(&mut Value)) -> Arc<CustomType> {
    let mut v: Value = serde_json::from_str(FOX).unwrap();
    v["ai"] = ai;
    edit(&mut v);
    let file = CustomMobFile::parse("sample_mobs:cinder_fox", v.to_string().as_bytes()).unwrap();
    let model = file
        .load_model(&mut |p| std::fs::read(root().join(p)).map_err(|e| e.to_string()))
        .unwrap();
    Arc::new(CustomType::new(file, Some(&model)).unwrap())
}

pub(crate) fn fox_mob(t: &Arc<CustomType>, at: Vec3) -> Mob {
    let mut m = Mob::new(1, Arc::new(build::mob_type(t)), at, 0.0);
    m.on_ground = true;
    m
}

fn melee(row: Value) -> Value {
    json!({"navigator": "ground", "goals": [row]})
}

fn idle_ai() -> Value {
    json!({"navigator": "ground", "goals": []})
}

/// The pounce attack of the fox: the `pounce` trigger, the `hit` frame of its animation.
fn pounce_row(delay: i64) -> Value {
    json!({"type": "melee_attack", "priority": 3, "trigger": "pounce", "damage_frame": "hit", "damage_delay": delay})
}

struct Run<'a> {
    w0: crate::mobs::MobWorld<'a>,
    lr: LegacyRandom,
    events: Vec<MobEvent>,
    tick: i64,
}
impl<'a> Run<'a> {
    fn new(w0: crate::mobs::MobWorld<'a>) -> Self {
        Self {
            tick: w0.game_time,
            w0,
            lr: LegacyRandom::new(5),
            events: Vec::new(),
        }
    }
    /// One tick through `Mob::tick`; the world clock is the tick.
    fn step(&mut self, m: &mut Mob) {
        let mut w = self.w0.clone();
        w.game_time = self.tick;
        m.tick(&w, &mut self.lr, &mut self.events);
        self.tick += 1;
    }
    fn hits(&self) -> usize {
        self.events
            .iter()
            .filter(|e| matches!(e, MobEvent::AttackPlayer(..)))
            .count()
    }
}

fn anim(m: &mut Mob) -> &mut MobAnim {
    m.extra.custom.as_mut().unwrap().anim.as_mut().unwrap()
}

#[test]
fn the_fixture_fox_has_a_rig_with_its_triggers_and_server_frame() {
    let t = fox_type_with(melee(pounce_row(3)), |_| {});
    let a = t.anim.as_ref().expect("the fox has animations");
    assert_eq!(a.rig.controllers().len(), 3, "locomotion, action, pose");
    let ids: Vec<_> = a.triggers.iter().map(|d| (d.name.as_str(), d.layer)).collect();
    assert!(ids.contains(&("pounce", 1)), "{ids:?}");
    assert_eq!(
        a.trigger_id("pounce").map(usize::from),
        a.triggers.iter().position(|d| d.name == "pounce")
    );
    assert!(a.trigger_id("nothing").is_none());
    // `model.server_events: ["pounce"]` makes the pounce animation's `hit` a server frame.
    assert_eq!(a.frames.len(), 1);
    assert_eq!(a.frames[0].name, "hit");
    assert!((a.frames[0].length - 0.8).abs() < 1e-6);
    // 0.8 s is 16 ticks; the safety net adds the slack.
    assert_eq!(frame_deadline(&t, Some("pounce"), "hit"), Some(16 + FRAME_SLACK_TICKS));
    assert_eq!(frame_deadline(&t, None, "hit"), Some(16 + FRAME_SLACK_TICKS));
    assert_eq!(frame_deadline(&t, Some("pounce"), "nope"), None);
    // A type without the flag has no server frames.
    let off = fox_type_with(melee(pounce_row(3)), |v| {
        v["model"].as_object_mut().unwrap().remove("server_events");
    });
    assert!(off.anim.as_ref().unwrap().frames.is_empty());
    assert_eq!(frame_deadline(&off, Some("pounce"), "hit"), None);
}

/// The keyframe at 0.35 s lands the blow in the 8th window of the animation (tick 7 or 8),
/// however short the `damage_delay` is: it is not the clock when the animation has the frame.
#[test]
fn the_keyframe_not_the_delay_lands_the_blow() {
    let t = fox_type_with(melee(pounce_row(3)), |_| {});
    let mut m = fox_mob(&t, Vec3::new(0.5, 1.0, 0.5));
    m.ai.target = Some(Who::Player);
    let fx = flat();
    let others = [];
    let mut run = Run::new(view(&fx.world(), &others, Some(Vec3::new(0.5, 1.0, 1.4))));
    let mut hit_tick = None;
    let mut start = None;
    for _ in 0..24 {
        run.step(&mut m);
        if start.is_none() {
            start = m
                .extra
                .custom
                .as_ref()
                .unwrap()
                .anim
                .as_ref()
                .and_then(|a| a.ring.first().copied());
        }
        if hit_tick.is_none() && run.hits() > 0 {
            hit_tick = Some(run.tick - 1);
        }
    }
    let start = start.expect("the goal asked for the pounce and the animation started it");
    let hit = hit_tick.expect("the blow landed");
    // The windows are [0, 0.05), [0.05, 0.1) ...; 0.35 is in the 8th, and the blow is in that tick.
    let n = hit - start.start_tick;
    assert_eq!(n, 8, "ticks from the animation's start to the blow");
    assert!((n - 1) as f32 * TICK_SECONDS <= 0.35 && 0.35 < n as f32 * TICK_SECONDS + 1e-6);
    assert_eq!(run.hits(), 1, "once");
    // The attack began one tick before the animation; the 3 tick fallback would have struck long ago.
    assert!(hit - (start.start_tick + 1) > 3 + 1);
}

/// A frame the type's animations do not have is the fallback's business, as in F4M-4a.
#[test]
fn a_frame_nobody_emits_falls_back_to_the_delay() {
    let row = json!({"type":"melee_attack","priority":3,"trigger":"pounce","damage_frame":"never","damage_delay":5});
    let t = fox_type_with(melee(row), |_| {});
    let mut m = fox_mob(&t, Vec3::new(0.5, 1.0, 0.5));
    m.ai.target = Some(Who::Player);
    let fx = flat();
    let others = [];
    let mut run = Run::new(view(&fx.world(), &others, Some(Vec3::new(0.5, 1.0, 1.4))));
    let mut started = None;
    for _ in 0..30 {
        run.step(&mut m);
        if started.is_none()
            && let Some(a) = m.extra.custom.as_ref().unwrap().anim.as_ref()
            && let Some(tr) = a.ring.first()
        {
            started = Some(tr.start_tick + 1);
        }
        if run.hits() > 0 {
            break;
        }
    }
    assert_eq!(run.hits(), 1);
    // The goal begins one tick before the animation starts; its delay of 5 ticks is counted from the
    // begin.
    assert_eq!(run.tick - 1, started.unwrap() + 4);
}

#[test]
fn a_type_without_animations_runs_nothing() {
    let mut m = mob_with("creature", "{}", "", Vec3::new(0.5, 1.0, 0.5));
    let fx = flat();
    let mut run = Run::new(fx.world());
    for _ in 0..5 {
        run.step(&mut m);
    }
    assert!(m.extra.custom.as_ref().unwrap().anim.is_none());
    assert!(!request_trigger(&mut m, "pounce", 1.0));
    assert!(m.kind.custom.as_ref().unwrap().anim.is_none());
}

/// The anchors follow the controller: one per layer to begin with, a new one when the state
/// changes, none while it runs at a steady speed, one when the speed changes enough.
#[test]
fn anchors_are_sent_on_state_and_speed_changes_only() {
    let t = fox_type_with(idle_ai(), |_| {});
    let mut m = fox_mob(&t, Vec3::new(0.5, 1.0, 0.5));
    let fx = flat();
    let mut run = Run::new(fx.world());
    run.step(&mut m);
    let first = anim(&mut m).take_anchor_changes(ANCHOR_TOLERANCE);
    assert_eq!(first.len(), 3, "every layer on the first look");
    assert!(anim(&mut m).take_anchor_changes(ANCHOR_TOLERANCE).is_empty());
    let idle_state = first[0].state;
    // Idle for a second: nothing to send (the idle clock runs at speed 1).
    for _ in 0..20 {
        run.step(&mut m);
    }
    assert!(
        anim(&mut m).take_anchor_changes(ANCHOR_TOLERANCE).is_empty(),
        "a steady clock needs no new anchor"
    );
    // The fox walks at 0.12 blocks a tick: idle goes to walk (ground_speed > 0.05).
    for _ in 0..3 {
        m.position.x += 0.12;
        run.step(&mut m);
    }
    let walking = anim(&mut m).take_anchor_changes(ANCHOR_TOLERANCE);
    assert!(
        walking.iter().any(|l| l.layer == 0 && l.state != idle_state),
        "{walking:?}"
    );
    let state = walking.iter().find(|l| l.layer == 0).unwrap().clone();
    // The walk animation runs at `q.ground_speed * 4`: 0.48.
    assert!((state.slots[0].1 - 0.48).abs() < 0.01, "speed {:?}", state.slots);
    // The same pace for two seconds: the speed is steady, so no anchor.
    for _ in 0..40 {
        m.position.x += 0.12;
        run.step(&mut m);
    }
    assert!(anim(&mut m).take_anchor_changes(ANCHOR_TOLERANCE).is_empty());
    // A client that extrapolates from the anchor is where the server's clock is.
    let cur = anim(&mut m).anchors()[0].clone();
    let clock = horizon_mobmodel::clock::AnimClock::new(
        state.state,
        state.tick,
        f64::from(state.slots[0].0),
        state.slots[0].1,
    );
    assert!((clock.time_at(cur.tick, 0.0) - f64::from(cur.slots[0].0)).abs() < 1e-3);
    // A faster pace changes the speed: a new anchor at the new speed (run starts above 0.2).
    let mut sent = None;
    for _ in 0..6 {
        m.position.x += 0.17;
        run.step(&mut m);
        let c = anim(&mut m).take_anchor_changes(ANCHOR_TOLERANCE);
        if let Some(l) = c.into_iter().find(|l| l.layer == 0) {
            sent = Some(l);
            break;
        }
    }
    let faster = sent.expect("a speed change is re-anchored");
    assert!(faster.slots[0].1 > 0.6, "{faster:?}");
    // Back to a stand: idle again.
    for _ in 0..4 {
        run.step(&mut m);
    }
    assert!(anim(&mut m)
        .take_anchor_changes(ANCHOR_TOLERANCE)
        .iter()
        .any(|l| l.layer == 0 && l.state == idle_state));
}

#[test]
fn a_zero_tolerance_re_anchors_on_every_change_of_speed() {
    let a = LayerAnchor { layer: 0, state: 1, tick: 100, slots: vec![(2.0, 1.0)] };
    let same = LayerAnchor { tick: 120, slots: vec![(3.0, 1.0)], ..a.clone() };
    assert!(!needs_anchor(&a, &same, 0.0));
    let nudged = LayerAnchor { tick: 120, slots: vec![(3.0, 1.001)], ..a.clone() };
    assert!(needs_anchor(&a, &nudged, 0.0));
    assert!(
        !needs_anchor(&a, &nudged, ANCHOR_TOLERANCE),
        "within the tolerance it is not worth a message"
    );
    let drifted = LayerAnchor { tick: 120, slots: vec![(3.05, 1.0)], ..a.clone() };
    assert!(needs_anchor(&a, &drifted, ANCHOR_TOLERANCE), "50 ms off");
    let stopped = LayerAnchor { tick: 101, slots: vec![(2.05, 0.0)], ..a.clone() };
    assert!(needs_anchor(&a, &stopped, 1.0), "stopping is always sent");
    let other_state = LayerAnchor { state: 2, ..same.clone() };
    assert!(needs_anchor(&a, &other_state, 1.0));
    let other_slots = LayerAnchor { slots: vec![(3.0, 1.0), (0.0, 1.0)], ..same };
    assert!(needs_anchor(&a, &other_slots, 1.0));
}

#[test]
fn triggers_play_once_keep_a_ring_and_end() {
    let t = fox_type_with(idle_ai(), |_| {});
    let ta = t.anim.as_ref().unwrap();
    let mut m = fox_mob(&t, Vec3::new(0.5, 1.0, 0.5));
    let fx = flat();
    let mut run = Run::new(fx.world());
    run.step(&mut m);
    assert!(request_trigger(&mut m, "pounce", 1.0));
    assert!(!request_trigger(&mut m, "no_such_trigger", 1.0));
    run.step(&mut m);
    let started = anim(&mut m).take_started();
    assert_eq!(started.len(), 1);
    assert_eq!(started[0].id, ta.trigger_id("pounce").unwrap());
    assert_eq!(started[0].start_tick, run.tick - 2, "the tick before the one it started in");
    assert_eq!(started[0].speed, 16);
    assert!(anim(&mut m).take_started().is_empty());
    assert_eq!(anim(&mut m).recent_triggers(ta).len(), 1);
    // 0.8 s is 16 ticks: afterwards it is not offered to a late joiner any more.
    for _ in 0..20 {
        run.step(&mut m);
    }
    assert!(anim(&mut m).runs.is_empty(), "finished");
    assert!(anim(&mut m).recent_triggers(ta).is_empty());
    assert_eq!(anim(&mut m).ring.len(), 1, "the ring keeps it; the snapshot filter drops it");
    // Playing it again replaces its entry; a ring keeps at most four.
    for i in 0..6 {
        request_trigger(&mut m, "pounce", 1.0 + i as f32);
        run.step(&mut m);
    }
    assert_eq!(anim(&mut m).ring.len(), 1, "the same trigger is one entry");
    assert_eq!(anim(&mut m).runs.len(), 1);
}

#[test]
fn the_wire_version_follows_the_layout() {
    let a = fox_type_with(idle_ai(), |_| {});
    let b = fox_type_with(idle_ai(), |_| {});
    assert_eq!(a.sync_version, b.sync_version, "the same file, the same version");
    let c = fox_type_with(idle_ai(), |v| {
        v["synced"]["warmth"]["max"] = json!(20);
    });
    assert_ne!(a.sync_version, c.sync_version);
    let d = fox_type_with(idle_ai(), |v| {
        v["synced"]["state"]["values"] = json!(["idle", "sit", "pounce", "leap"]);
    });
    assert_ne!(a.sync_version, d.sync_version);
}

/// The byte budget of a mob's values is the schema's: no type can be built whose values do not fit
/// the 256 bytes of one `horizon:mob_sync` message.
#[test]
fn the_values_of_a_type_fit_256_bytes() {
    let parse = |n: usize| {
        let values = (0..n)
            .map(|i| format!(r#""s{i}":{{"type":"string"}}"#))
            .collect::<Vec<_>>()
            .join(",");
        let json = format!(
            r#"{{"template":"yoake:mob","size":{{"width":1,"height":1}},"synced":{{{values}}}}}"#
        );
        CustomMobFile::parse("demo:thing", json.as_bytes())
    };
    assert!(parse(3).is_ok(), "3 x 65 = 195");
    let err = parse(4).unwrap_err();
    assert!(err.contains("260 bytes"), "{err}");
    // What a message holds at the most is what the schema allows, tags aside.
    let t = horizon_protocol::mob_sync::MAX_VALUE_BYTES;
    assert_eq!(t, 256);
    assert_eq!(horizon_protocol::mob_sync::max_value_len("string"), Some(65));
}
