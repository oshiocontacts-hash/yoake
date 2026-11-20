//! F4M-9: typed animation events (sound, particle, custom), locator binding, client/server sides, exactly-once rules
//! (loops, pause, re-anchor, late join) and the Cinder Fox pounce.

use glam::Vec3;
use horizon_mobmodel::anim::{parse_animation_file, AnimationLibrary};
use horizon_mobmodel::clock::AnimClock;
use horizon_mobmodel::controller::parse_controllers;
use horizon_mobmodel::events::*;
use horizon_mobmodel::expr::{ExprInputs, Symbols};
use horizon_mobmodel::model::Locator;
use horizon_mobmodel::pose::{Rig, RigSource};
use horizon_mobmodel::skeleton::{bone_matrices_into, Bone, BoneIndex, BoneTransform, Pose, Skeleton};
use horizon_mobmodel::{load_geo_json, Model};
use serde_json::Value;

const GEO: &[u8] = include_bytes!("fixtures/cinder_fox/cinder_fox.geo.json");
const ANIM: &str = include_str!("fixtures/cinder_fox/cinder_fox.animation.json");
const CTRL: &str = include_str!("fixtures/cinder_fox/cinder_fox.json");
const TYPE_JSON: &str = include_str!("fixtures/procedural/cinder_fox_type.json");
const TICK: f32 = 0.05;

struct Fox {
    model: Model,
    rig: Rig,
    ty: Value,
}

fn fox() -> Fox {
    let model = load_geo_json("cinder_fox.geo.json", GEO).unwrap();
    let mut syms = Symbols::new(&["state"]);
    let mut lib = AnimationLibrary::new();
    for a in parse_animation_file("cinder_fox.animation.json", ANIM, &mut syms).unwrap().animations {
        lib.add(a).unwrap();
    }
    let ctrl = parse_controllers("cinder_fox.json", CTRL, &mut syms).unwrap();
    let (rig, _) = Rig::new(model.skeleton.clone(), lib, ctrl.controllers).unwrap();
    Fox { model, rig, ty: serde_json::from_str(TYPE_JSON).unwrap() }
}

fn fox_events(f: &Fox) -> MobEvents {
    MobEvents::from_model_json("cinder_fox_type.json", "cinder_fox.animation.json", &f.ty["model"], f.rig.animations(), &f.model.locators)
        .unwrap_or_else(|e| panic!("{e}"))
}

fn action_pounce(f: &Fox) -> (usize, usize) {
    let layer = f.rig.controllers().iter().position(|c| c.name == "action").unwrap();
    let trig = f.rig.controllers()[layer].triggers.iter().position(|t| t.name == "pounce").unwrap();
    (layer, trig)
}

fn label(me: &MobEvents, e: &MobAnimEvent) -> String {
    match e.kind {
        MobEventKind::Custom { name, .. } => format!("custom:{}", me.string(name)),
        MobEventKind::Sound { sound, .. } => format!("sound:{}", me.string(sound)),
        MobEventKind::Particle { particle, .. } => format!("particle:{}", me.string(particle)),
    }
}

/// Server-style run of the pounce trigger: `(tick, label)` for the events `side` gets.
fn run_pounce(f: &Fox, me: &MobEvents, side: EventSide) -> Vec<(u32, String)> {
    let (layer, trig) = action_pounce(f);
    let mut buf = EventBuf::with_capacity(16);
    let mut out = Vec::new();
    let mut elapsed = 0.0f32;
    for tick in 1..=60u32 {
        let e0 = elapsed;
        elapsed += TICK;
        buf.clear();
        me.trigger_window(&f.rig, side, layer, trig, e0, elapsed, &mut buf);
        out.extend(buf.as_slice().iter().map(|e| (tick, label(me, e))));
    }
    out
}

#[test]
fn server_pounce_emits_exactly_one_hit_and_no_cosmetics() {
    let f = fox();
    let me = fox_events(&f);
    let ev = run_pounce(&f, &me, EventSide::Server);
    assert_eq!(ev.len(), 1, "{ev:?}");
    assert_eq!(ev[0].1, "custom:hit");
    assert!((ev[0].0 as i32 - 7).abs() <= 1, "tick {}", ev[0].0);
    // the same run again gives the same events (deterministic)
    assert_eq!(ev, run_pounce(&f, &me, EventSide::Server));
}

#[test]
fn client_pounce_gets_hit_sound_and_particle_at_the_mouth() {
    let f = fox();
    let me = fox_events(&f);
    let (layer, trig) = action_pounce(&f);
    let ev = run_pounce(&f, &me, EventSide::Client);
    let tick = ev[0].0;
    assert_eq!(
        ev,
        [
            (tick, "custom:hit".to_string()),
            (tick, "sound:cinder_fox_pounce".to_string()),
            (tick, "particle:sample_mobs:ember".to_string()),
        ]
    );
    // full detail of the events
    let mut buf = EventBuf::with_capacity(8);
    me.trigger_window(&f.rig, EventSide::Client, layer, trig, 0.3, 0.4, &mut buf);
    let e = buf.as_slice();
    assert_eq!(e.len(), 3);
    for x in e {
        assert!(x.is_cosmetic() && x.side == EventSide::Client);
        assert_eq!((x.time, x.layer as usize, x.loop_index), (0.35, layer, 0));
        assert_eq!(x.source, RigSource::Trigger { trigger: trig as u16 });
        assert_eq!(f.rig.animation(x.animation as usize).unwrap().name, "animation.cinder_fox.pounce");
    }
    let mouth = f.model.locators.iter().position(|l| l.name == "mouth").map(|i| LocatorIndex(i as u16));
    assert!(mouth.is_some());
    assert!(matches!(e[0].kind, MobEventKind::Custom { locator: None, .. }));
    assert!(matches!(e[1].kind, MobEventKind::Sound { volume, pitch, locator: None, .. } if volume == 1.0 && pitch == 1.0));
    assert!(matches!(e[2].kind, MobEventKind::Particle { locator, offset, .. } if locator == mouth && offset == Vec3::ZERO));
}

#[test]
fn unflagged_or_missing_server_events_give_the_server_nothing() {
    let f = fox();
    for model in [r#"{"particles":{"cinder_fox_embers":"x:e"}}"#, r#"{"particles":{"cinder_fox_embers":"x:e"},"server_events":false}"#] {
        let m: Value = serde_json::from_str(model).unwrap();
        let me = MobEvents::from_model_json("t.json", "a.json", &m, f.rig.animations(), &f.model.locators).unwrap();
        assert!(run_pounce(&f, &me, EventSide::Server).is_empty());
        assert_eq!(run_pounce(&f, &me, EventSide::Client).len(), 3, "the client is not affected by the flag");
    }
    // `true` and the object form
    for model in [r#"{"particles":{"cinder_fox_embers":"x:e"},"server_events":true}"#, r#"{"particles":{"cinder_fox_embers":"x:e"},"server_events":{"pounce":true,"idle":false}}"#] {
        let m: Value = serde_json::from_str(model).unwrap();
        let me = MobEvents::from_model_json("t.json", "a.json", &m, f.rig.animations(), &f.model.locators).unwrap();
        assert_eq!(run_pounce(&f, &me, EventSide::Server).len(), 1);
    }
}

#[test]
fn locator_position_follows_the_pose() {
    let f = fox();
    let me = fox_events(&f);
    let (layer, trig) = action_pounce(&f);
    let mut buf = EventBuf::with_capacity(8);
    me.trigger_window(&f.rig, EventSide::Client, layer, trig, 0.3, 0.4, &mut buf);
    let particle = buf.as_slice()[2];
    let loc = &f.model.locators;
    let mut m = Vec::new();
    // rest pose: the locator's rest position
    bone_matrices_into(&f.model.skeleton, &Pose::rest(&f.model.skeleton), &mut m);
    let rest = event_position(loc, &m, &particle);
    let mouth = loc.iter().find(|l| l.name == "mouth").unwrap();
    assert!(rest.abs_diff_eq(mouth.offset, 1e-4), "{rest} vs {}", mouth.offset);
    // head turned 90 degrees about Y: the mouth swings around the head pivot
    let head = f.model.skeleton.find("head").unwrap();
    let mut pose = Pose::rest(&f.model.skeleton);
    pose.bones[head.index()] = BoneTransform {
        rotation: glam::Quat::from_rotation_y(std::f32::consts::FRAC_PI_2),
        ..BoneTransform::IDENTITY
    };
    bone_matrices_into(&f.model.skeleton, &pose, &mut m);
    let turned = event_position(loc, &m, &particle);
    let pivot = f.model.skeleton.bones[head.index()].pivot;
    assert!(!turned.abs_diff_eq(rest, 1.0));
    assert!(((turned - pivot).length() - (rest - pivot).length()).abs() < 1e-3);
    // no locator: the extra offset in model space
    assert_eq!(locator_position(loc, &m, None, Vec3::new(1.0, 2.0, 3.0)), Vec3::new(1.0, 2.0, 3.0));
}

// ----- synthetic rig --------------------------------------------------------------------------------------------

fn skeleton() -> Skeleton {
    let b = |n: &str, p: Option<u16>, piv: [f32; 3]| Bone { name: n.into(), parent: p.map(BoneIndex), pivot: Vec3::from(piv), rotation: Vec3::ZERO };
    Skeleton { bones: vec![b("body", None, [0.0, 12.0, 0.0]), b("head", Some(0), [0.0, 24.0, 0.0])] }
}

fn locators() -> Vec<Locator> {
    vec![
        Locator { name: "nose".into(), bone: BoneIndex(1), offset: Vec3::new(0.0, 24.0, -6.0), rotation: Vec3::ZERO, null_object: false },
        Locator { name: "tail".into(), bone: BoneIndex(0), offset: Vec3::new(0.0, 12.0, 8.0), rotation: Vec3::ZERO, null_object: false },
    ]
}

const ANIMS: &str = r#"{"animations":{
  "ticker":{"loop":true,"animation_length":1,"timeline":{"0":"tick","0.5":"tock"},
            "sound_effects":{"0.5":{"effect":"mod:step","volume":0.5,"pitch":1.5,"locator":"tail"}},
            "particle_effects":{"0.5":{"effect":"dust","locator":"nose","offset":[0,1,2]}}},
  "wave":{"loop":false,"animation_length":1,"timeline":{"0.5":"hit"}},
  "quiet":{"loop":true,"animation_length":1,"timeline":{"0.25":"hum"}}
}}"#;

const LOCO: &str = r#"{"controllers":{
  "loco":{"initial":"a","states":{"a":{"animations":["ticker","quiet"]}}},
  "act":{"mode":"add","triggers":{"wave":"wave"}}}}"#;

fn rig() -> Rig {
    let mut syms = Symbols::new(&[]);
    let mut lib = AnimationLibrary::new();
    for a in parse_animation_file("anims.json", ANIMS, &mut syms).unwrap().animations {
        lib.add(a).unwrap();
    }
    let c = parse_controllers("ctrl.json", LOCO, &mut syms).unwrap().controllers;
    Rig::new(skeleton(), lib, c).unwrap().0
}

fn model_json(extra: &str) -> Value {
    serde_json::from_str(&format!(r#"{{"particles":{{"dust":"minecraft:cloud"}}{extra}}}"#)).unwrap()
}

fn events(rig: &Rig, extra: &str) -> MobEvents {
    MobEvents::from_model_json("type.json", "anim.json", &model_json(extra), rig.animations(), &locators()).unwrap_or_else(|e| panic!("{e}"))
}

fn run_update(rig: &Rig, me: &MobEvents, side: EventSide, dts: &[f32]) -> Vec<(usize, String, u32)> {
    let mut st = rig.initial_state();
    let mut inp = ExprInputs::default();
    let mut buf = EventBuf::with_capacity(64);
    let mut out = Vec::new();
    for (i, dt) in dts.iter().enumerate() {
        buf.clear();
        me.update(rig, side, &mut st, *dt, &mut inp, &mut buf);
        out.extend(buf.as_slice().iter().map(|e| (i, label(me, e), e.loop_index)));
    }
    out
}

#[test]
fn once_per_loop_never_while_paused_and_deterministic() {
    let rig = rig();
    let me = events(&rig, r#","server_events":["ticker"]"#);
    // 2.5 s in 50 ticks, a pause of 10 ticks in the middle
    let mut dts = vec![0.05f32; 25];
    dts.extend([0.0f32; 10]);
    dts.extend([0.05f32; 25]);
    let ev = run_update(&rig, &me, EventSide::Server, &dts);
    let hits: Vec<_> = ev.iter().map(|e| (e.1.as_str(), e.2)).collect();
    // ticker runs 1.25 s... the pause freezes the clock: total 2.5 s of animation time -> loops 0,1 and the start of 2
    assert_eq!(hits.iter().filter(|h| h.0 == "custom:tick").count(), 3, "{hits:?}"); // t = 0, 1, 2
    assert_eq!(hits.iter().filter(|h| h.0 == "custom:tock").count(), 2, "{hits:?}"); // t = 0.5, 1.5
    assert!(ev.iter().all(|e| e.1.starts_with("custom:")), "server: custom only");
    assert!(!ev.iter().any(|e| e.1 == "custom:hum"), "quiet is not flagged");
    // nothing at all on the paused ticks (indices 25..35)
    assert!(ev.iter().all(|e| !(25..35).contains(&e.0)), "{ev:?}");
    // same input, same events
    assert_eq!(ev, run_update(&rig, &me, EventSide::Server, &dts));
    // the client sees hum too, plus sound and particle, once per loop
    let c = run_update(&rig, &me, EventSide::Client, &dts);
    assert_eq!(c.iter().filter(|e| e.1 == "custom:hum").count(), 3); // 0.25, 1.25, 2.25
    assert_eq!(c.iter().filter(|e| e.1 == "sound:mod:step").count(), 2);
    assert_eq!(c.iter().filter(|e| e.1 == "particle:minecraft:cloud").count(), 2);
}

#[test]
fn sound_and_particle_parameters_and_locators_are_typed() {
    let rig = rig();
    let me = events(&rig, "");
    let mut buf = EventBuf::with_capacity(16);
    let mut st = rig.initial_state();
    me.update(&rig, EventSide::Client, &mut st, 0.6, &mut ExprInputs::default(), &mut buf);
    let e = buf.as_slice();
    let find = |f: fn(&MobEventKind) -> bool| e.iter().find(|x| f(&x.kind)).unwrap();
    let s = find(|k| matches!(k, MobEventKind::Sound { .. }));
    let MobEventKind::Sound { sound, volume, pitch, locator } = s.kind else { unreachable!() };
    assert_eq!((me.string(sound), volume, pitch, locator), ("mod:step", 0.5, 1.5, Some(LocatorIndex(1))));
    assert_eq!(s.source, RigSource::State { state: 0, slot: 0 });
    assert_eq!(s.time, 0.5);
    let p = find(|k| matches!(k, MobEventKind::Particle { .. }));
    let MobEventKind::Particle { particle, locator, offset } = p.kind else { unreachable!() };
    assert_eq!((me.string(particle), locator, offset), ("minecraft:cloud", Some(LocatorIndex(0)), Vec3::new(0.0, 1.0, 2.0)));
    // the quiet animation is the second slot of the state
    let h = e.iter().find(|x| label(&me, x) == "custom:hum").unwrap();
    assert_eq!(h.source, RigSource::State { state: 0, slot: 1 });
    // positions: nose at rest plus the offset in the bone frame
    let mut m = Vec::new();
    bone_matrices_into(&skeleton(), &Pose::rest(&skeleton()), &mut m);
    assert_eq!(event_position(&locators(), &m, p), Vec3::new(0.0, 25.0, -4.0));
    assert_eq!(event_position(&locators(), &m, s), Vec3::new(0.0, 12.0, 8.0));
    // an unknown id string
    assert_eq!(me.find_string("nope"), None);
}

#[test]
fn trigger_events_report_the_trigger_slot() {
    let rig = rig();
    let me = events(&rig, r#","server_events":{"wave":true}"#);
    let layer = rig.controllers().iter().position(|c| c.name == "act").unwrap();
    let mut buf = EventBuf::with_capacity(4);
    me.trigger_window(&rig, EventSide::Server, layer, 0, 0.4, 0.6, &mut buf);
    assert_eq!(buf.len(), 1);
    assert_eq!(buf.as_slice()[0].source, RigSource::Trigger { trigger: 0 });
    assert_eq!(buf.as_slice()[0].layer as usize, layer);
    // window past the event, and an empty or reversed window
    buf.clear();
    me.trigger_window(&rig, EventSide::Server, layer, 0, 0.6, 0.9, &mut buf);
    me.trigger_window(&rig, EventSide::Server, layer, 0, 0.6, 0.4, &mut buf);
    assert!(buf.is_empty());
}

#[test]
fn full_buffer_drops_and_counts() {
    let rig = rig();
    let me = events(&rig, "");
    let mut buf = EventBuf::with_capacity(2);
    let mut st = rig.initial_state();
    me.update(&rig, EventSide::Client, &mut st, 0.6, &mut ExprInputs::default(), &mut buf);
    assert_eq!(buf.len(), 2);
    assert!(buf.dropped() >= 1);
    buf.clear();
    assert_eq!((buf.len(), buf.dropped()), (0, 0));
}

// ----- load errors ----------------------------------------------------------------------------------------------

fn build_err(anims: &str, model: &str) -> String {
    let mut syms = Symbols::new(&[]);
    let mut lib = AnimationLibrary::new();
    for a in parse_animation_file("x.animation.json", anims, &mut syms).unwrap().animations {
        lib.add(a).unwrap();
    }
    let m: Value = serde_json::from_str(model).unwrap();
    MobEvents::from_model_json("type.json", "x.animation.json", &m, &lib, &locators()).unwrap_err().to_string()
}

#[test]
fn load_errors_name_file_and_path() {
    let bad_loc = r#"{"animations":{"a":{"loop":true,"animation_length":1,"timeline":{"0":"x"},"sound_effects":{"0.5":{"effect":"s","locator":"ghost"}}}}}"#;
    let e = build_err(bad_loc, "{}");
    assert!(e.starts_with("x.animation.json: animations[\"a\"].events[1].locator: unknown locator 'ghost'"), "{e}");
    assert!(e.contains("nose, tail"), "{e}");

    let part = r#"{"animations":{"a":{"loop":true,"animation_length":1,"particle_effects":{"0":"dust"}}}}"#;
    let e = build_err(part, "{}");
    assert!(e.contains("x.animation.json: animations[\"a\"].events[0]") && e.contains("particles table"), "{e}");
    // a namespaced id is accepted without the table
    let direct = r#"{"animations":{"a":{"loop":true,"animation_length":1,"particle_effects":{"0":"minecraft:flame"}}}}"#;
    assert!(MobEvents::from_model_json("t", "a", &serde_json::json!({}), &{
        let mut s = Symbols::new(&[]);
        let mut l = AnimationLibrary::new();
        for a in parse_animation_file("a", direct, &mut s).unwrap().animations {
            l.add(a).unwrap();
        }
        l
    }, &locators()).is_ok());

    let ok = r#"{"animations":{"a":{"loop":true,"animation_length":1,"timeline":{"0":"x"}}}}"#;
    let e = build_err(ok, r#"{"server_events":["a","b"]}"#);
    assert_eq!(e, "type.json: model.server_events[1]: unknown animation 'b'");
    let e = build_err(ok, r#"{"server_events":{"zz":true}}"#);
    assert!(e.starts_with("type.json: model.server_events[\"zz\"]"), "{e}");
    let e = build_err(ok, r#"{"server_events":3}"#);
    assert!(e.starts_with("type.json: model.server_events: expected true"), "{e}");
    let e = build_err(ok, r#"{"particles":{"p":"Bad Id"}}"#);
    assert!(e.starts_with("type.json: model.particles[\"p\"]: invalid particle id"), "{e}");
    let e = build_err(ok, r#"{"particles":{"p":{"id":"a:b","extra":1}}}"#);
    assert!(e.contains("unknown field 'extra'"), "{e}");
    let e = build_err(ok, r#"{"particles":[]}"#);
    assert!(e.starts_with("type.json: model.particles: expected an object"), "{e}");
    let many: String = (0..257).map(|i| format!("\"p{i}\":\"a:b\"")).collect::<Vec<_>>().join(",");
    let e = build_err(ok, &format!("{{\"particles\":{{{many}}}}}"));
    assert!(e.contains("257 particles (at most 256)"), "{e}");
}

#[test]
fn bad_sound_parameters_are_load_errors() {
    for (frag, what) in [(r#""volume":9"#, "volume"), (r#""pitch":0.1"#, "pitch")] {
        let src = format!(r#"{{"animations":{{"a":{{"animation_length":1,"sound_effects":{{"0":{{"effect":"s",{frag}}}}}}}}}}}"#);
        let e = parse_animation_file("a.json", &src, &mut Symbols::new(&[])).unwrap_err().to_string();
        assert!(e.contains(what), "{e}");
    }
    let src = r#"{"animations":{"a":{"animation_length":1,"particle_effects":{"0":{"effect":"s","offset":[1,2]}}}}}"#;
    assert!(parse_animation_file("a.json", src, &mut Symbols::new(&[])).unwrap_err().to_string().contains("offset"));
}

// ----- the client cursor ----------------------------------------------------------------------------------------

/// Run a cursor over `ticks` (inclusive range) of `clock` with the fox's pounce trigger; the ticks where events fired.
fn cursor_run(f: &Fox, me: &MobEvents, cur: &mut EventCursor, clock: &AnimClock, ticks: std::ops::RangeInclusive<i64>) -> Vec<(i64, String)> {
    let (layer, trig) = action_pounce(f);
    let mut buf = EventBuf::with_capacity(16);
    let mut out = Vec::new();
    for tick in ticks {
        let w = cur.advance(clock, tick, 0.0);
        buf.clear();
        me.trigger_window(&f.rig, EventSide::Client, layer, trig, w.c0, w.c1, &mut buf);
        out.extend(buf.as_slice().iter().map(|e| (tick, label(me, e))));
    }
    out
}

#[test]
fn a_client_that_sees_the_start_gets_each_event_once() {
    let f = fox();
    let me = fox_events(&f);
    let clock = AnimClock::new(2, 100, 0.0, 1.0);
    let mut cur = EventCursor::fresh(&clock);
    let ev = cursor_run(&f, &me, &mut cur, &clock, 100..=160);
    assert_eq!(ev.len(), 3, "{ev:?}");
    assert!(ev.iter().all(|e| e.0 == ev[0].0 && (e.0 - 107).abs() <= 1));
    // looking again at the same tick changes nothing
    assert!(cursor_run(&f, &me, &mut cur, &clock, 160..=160).is_empty());
}

#[test]
fn a_late_joiner_never_replays_the_past() {
    let f = fox();
    let me = fox_events(&f);
    let clock = AnimClock::new(2, 100, 0.0, 1.0);
    // joins before the hit: sees it once
    let mut cur = EventCursor::joined(&clock, 105, 0.0);
    assert_eq!(cursor_run(&f, &me, &mut cur, &clock, 105..=160).len(), 3);
    // joins exactly at the keyframe time: the event is not strictly after, so nothing fires
    let mut cur = EventCursor::joined(&clock, 107, 0.0);
    assert!(cursor_run(&f, &me, &mut cur, &clock, 107..=160).is_empty());
    // joins after it
    let mut cur = EventCursor::joined(&clock, 120, 0.0);
    assert!(cursor_run(&f, &me, &mut cur, &clock, 120..=200).is_empty());
    // a join late in a looping animation only fires what comes next, once per loop
    let rig = rig();
    let me = events(&rig, "");
    let clock = AnimClock::new(0, 0, 0.0, 1.0);
    let mut cur = EventCursor::joined(&clock, 1000, 0.0); // 50 s in, exactly on a loop start
    let mut buf = EventBuf::with_capacity(32);
    let mut ticks = Vec::new();
    for tick in 1000..=1060 {
        let w = cur.advance(&clock, tick, 0.0);
        buf.clear();
        me.animation_window(&rig, EventSide::Client, 0, RigSource::State { state: 0, slot: 0 }, 0, w.c0, w.c1, &mut buf);
        ticks.extend(buf.as_slice().iter().map(|e| (tick, label(&me, e))));
    }
    // 3 s: the tick at t=50 is not replayed; tock at 50.5, tick at 51, tock at 51.5, tick at 52, tock 52.5, tick at 53
    let names: Vec<_> = ticks.iter().filter(|t| t.1.starts_with("custom:")).map(|t| t.1.as_str()).collect();
    assert_eq!(names, ["custom:tock", "custom:tick", "custom:tock", "custom:tick", "custom:tock"], "{ticks:?}");
}

#[test]
fn reanchors_backwards_or_forwards_fire_nothing_and_continuous_ones_do_not_matter() {
    let f = fox();
    let me = fox_events(&f);
    // continuous re-anchor (speed change at tick 103) keeps the hit exactly once
    let c0 = AnimClock::new(2, 100, 0.0, 1.0);
    let mut cur = EventCursor::fresh(&c0);
    let mut ev = cursor_run(&f, &me, &mut cur, &c0, 100..=102);
    let c1 = c0.reanchor(103, 2, 2.0);
    ev.extend(cursor_run(&f, &me, &mut cur, &c1, 103..=140));
    assert_eq!(ev.len(), 3, "{ev:?}");
    // a hold (speed 0) and release does not repeat it either
    let c2 = c1.reanchor(120, 2, 0.0);
    assert!(cursor_run(&f, &me, &mut cur, &c2, 121..=130).is_empty());

    // anchor moved BACKWARDS to 0.2 s after the hit played: no replay, and a later pass strictly after 0.2 fires once
    let mut cur = EventCursor::fresh(&c0);
    assert_eq!(cursor_run(&f, &me, &mut cur, &c0, 100..=112).len(), 3);
    let back = AnimClock::new(2, 113, 0.2, 1.0);
    assert!(cursor_run(&f, &me, &mut cur, &back, 113..=114).is_empty(), "nothing fires at the seek");
    assert_eq!(cursor_run(&f, &me, &mut cur, &back, 115..=130).len(), 3, "the hit is strictly after the seek point");

    // anchor jumped FORWARD over the hit: the skipped event is not fired in a burst
    let mut cur = EventCursor::fresh(&c0);
    let ev = cursor_run(&f, &me, &mut cur, &c0, 100..=103);
    assert!(ev.is_empty());
    let fwd = AnimClock::new(2, 104, 0.6, 1.0);
    assert!(cursor_run(&f, &me, &mut cur, &fwd, 104..=140).is_empty());

    // a restart (anchor at time 0 at a later tick) fires from time 0 again
    let again = AnimClock::new(2, 150, 0.0, 1.0);
    assert_eq!(cursor_run(&f, &me, &mut cur, &again, 150..=170).len(), 3);
    // negative speed fires nothing
    let mut cur = EventCursor::fresh(&AnimClock::new(2, 100, 0.8, -1.0));
    assert!(cursor_run(&f, &me, &mut cur, &AnimClock::new(2, 100, 0.8, -1.0), 100..=116).is_empty());
}

#[test]
fn a_long_stall_is_a_seek() {
    let clock = AnimClock::new(0, 0, 0.0, 1.0);
    let mut cur = EventCursor::fresh(&clock);
    assert!(!cur.advance(&clock, 10, 0.0).is_empty());
    // 10 s later in one call: no burst
    assert!(cur.advance(&clock, 210, 0.0).is_empty());
    assert!(!cur.advance(&clock, 211, 0.0).is_empty());
}

// ----- high-water marks -----------------------------------------------------------------------------------------

#[test]
fn rewind_then_forward_does_not_emit_the_hit_again_on_the_client_cursor() {
    let f = fox();
    let me = fox_events(&f);
    let c0 = AnimClock::new(2, 100, 0.0, 1.0);
    let mut cur = EventCursor::fresh(&c0);
    assert_eq!(cursor_run(&f, &me, &mut cur, &c0, 100..=112).len(), 3, "the hit plays once");
    // run backwards (continuous re-anchors) to about 0.05 s, then forward again over 0.35 s
    let back = c0.reanchor(113, 2, -1.0);
    assert!(cursor_run(&f, &me, &mut cur, &back, 113..=125).is_empty());
    let fwd = back.reanchor(126, 2, 1.0);
    assert!(cursor_run(&f, &me, &mut cur, &fwd, 126..=160).is_empty(), "no second hit");
}

#[test]
fn a_new_loop_after_the_mark_fires_again_and_the_rewound_part_does_not() {
    let rig = rig();
    let me = events(&rig, "");
    let run = |cur: &mut EventCursor, clock: &AnimClock, ticks: std::ops::RangeInclusive<i64>| {
        let mut buf = EventBuf::with_capacity(32);
        let mut out = Vec::new();
        for tick in ticks {
            let w = cur.advance(clock, tick, 0.0);
            buf.clear();
            me.animation_window(&rig, EventSide::Client, 0, RigSource::State { state: 0, slot: 0 }, 0, w.c0, w.c1, &mut buf);
            out.extend(buf.as_slice().iter().filter(|e| matches!(e.kind, MobEventKind::Custom { .. })).map(|e| label(&me, e)));
        }
        out
    };
    let c0 = AnimClock::new(0, 0, 0.0, 1.0);
    let mut cur = EventCursor::fresh(&c0);
    assert_eq!(run(&mut cur, &c0, 0..=24), ["custom:tick", "custom:tock", "custom:tick"]); // 0, 0.5, 1.0
    let back = c0.reanchor(24, 0, -1.0); // time 1.2 down to 0.2
    assert!(run(&mut cur, &back, 25..=44).is_empty());
    let fwd = back.reanchor(44, 0, 1.0);
    assert!(run(&mut cur, &fwd, 45..=64).is_empty(), "back up to the mark (1.2 s): nothing");
    // past the mark: the tock at 1.5 is new, and so are the next loop's tick at 2.0 and tock at 2.5
    assert_eq!(run(&mut cur, &fwd, 65..=91), ["custom:tock", "custom:tick", "custom:tock"]);
}

#[test]
fn server_marks_stop_repeated_and_rewound_windows_from_double_hitting() {
    let f = fox();
    let me = fox_events(&f);
    let (layer, trig) = action_pounce(&f);
    let mut marks = EventMarks::new(&f.rig);
    let mut buf = EventBuf::with_capacity(16);
    let w = |c0: f32, c1: f32, marks: &mut EventMarks, buf: &mut EventBuf, side| {
        buf.clear();
        me.trigger_window_marked(&f.rig, side, layer, trig, c0, c1, marks, buf);
        buf.len()
    };
    assert_eq!(w(0.30, 0.40, &mut marks, &mut buf, EventSide::Server), 1);
    assert_eq!(w(0.30, 0.40, &mut marks, &mut buf, EventSide::Server), 0, "the same window again");
    assert_eq!(w(0.00, 0.40, &mut marks, &mut buf, EventSide::Server), 0, "a rewound, wider window");
    assert_eq!(w(0.40, 0.80, &mut marks, &mut buf, EventSide::Server), 0);
    // the trigger is played again: reset, and it fires once more
    marks.reset_trigger(layer, trig);
    assert_eq!(w(0.30, 0.40, &mut marks, &mut buf, EventSide::Server), 1);
    // the client's hit, sound and particle share a keyframe time and all fire together
    marks.reset_all();
    assert_eq!(w(0.30, 0.40, &mut marks, &mut buf, EventSide::Client), 3);
    assert_eq!(w(0.30, 0.40, &mut marks, &mut buf, EventSide::Client), 0);
}

#[test]
fn update_marked_gives_the_same_events_as_update_and_never_repeats() {
    let rig = rig();
    let me = events(&rig, r#","server_events":["ticker"]"#);
    let mut dts = vec![0.05f32; 25];
    dts.extend([0.0f32; 10]);
    dts.extend([0.05f32; 25]);
    let plain = run_update(&rig, &me, EventSide::Server, &dts);
    let mut st = rig.initial_state();
    let mut inp = ExprInputs::default();
    let mut marks = EventMarks::new(&rig);
    let mut buf = EventBuf::with_capacity(64);
    let mut marked = Vec::new();
    for (i, dt) in dts.iter().enumerate() {
        buf.clear();
        me.update_marked(&rig, EventSide::Server, &mut st, *dt, &mut inp, &mut marks, &mut buf);
        marked.extend(buf.as_slice().iter().map(|e| (i, label(&me, e), e.loop_index)));
    }
    assert_eq!(marked, plain);
}
