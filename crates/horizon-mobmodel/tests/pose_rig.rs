//! Golden poses, layers, blending, triggers, sparse evaluation and events on a synthetic skeleton.

use glam::{Mat4, Vec3};
use horizon_mobmodel::anim::{animations_from_bbmodel, parse_animation_file, AnimationLibrary, EventKind};
use horizon_mobmodel::controller::parse_controllers;
use horizon_mobmodel::expr::{ExprInputs, Query, Symbols};
use horizon_mobmodel::pose::{Rig, RigState, Scratch, TriggerPlay};
use horizon_mobmodel::skeleton::{bone_matrices, Bone, BoneIndex, Pose, Skeleton};

fn bone(name: &str, parent: Option<u16>, pivot: [f32; 3]) -> Bone {
    Bone { name: name.into(), parent: parent.map(BoneIndex), pivot: Vec3::from(pivot), rotation: Vec3::ZERO }
}

/// body(0) <- arm(1) <- hand(2); body <- head(3)
fn skeleton() -> Skeleton {
    Skeleton {
        bones: vec![
            bone("body", None, [0.0, 12.0, 0.0]),
            bone("arm", Some(0), [4.0, 12.0, 0.0]),
            bone("hand", Some(1), [4.0, 6.0, 0.0]),
            bone("head", Some(0), [0.0, 24.0, 0.0]),
        ],
    }
}

const ANIMS: &str = r#"{"animations":{
  "idle": {"loop":true,"animation_length":1,"bones":{"head":{"position":[0,0,0]}}},
  "walk": {"loop":true,"animation_length":1,"bones":{"head":{"position":[0,10,0]}}},
  "bonus":{"loop":true,"animation_length":1,"bones":{"head":{"position":[0,1,0]}}},
  "over": {"loop":true,"animation_length":1,"override_previous_animation":true,"bones":{"head":{"position":[0,7,0]}}},
  "wave": {"loop":false,"animation_length":1,"bones":{"arm":{"position":[0,3,0]},"head":{"position":[0,100,0]}},
           "timeline":{"0.5":"hit"}},
  "ticker":{"loop":true,"animation_length":1,"timeline":{"0":"tick","0.5":"tock"}},
  "swing":{"loop":true,"animation_length":2,"bones":{"arm":{"rotation":{"0":[0,0,0],"2":[180,0,0]}}}}
}}"#;

fn rig(ctrl: &str) -> Rig {
    let mut syms = Symbols::new(&[]);
    let mut lib = AnimationLibrary::new();
    for a in parse_animation_file(ANIMS, &mut syms).unwrap().animations {
        lib.add(a).unwrap();
    }
    let c = parse_controllers(ctrl, &mut syms).unwrap().controllers;
    Rig::new(skeleton(), lib, c).unwrap().0
}

fn eval(r: &Rig, st: &RigState, trig: &[TriggerPlay], inp: &mut ExprInputs) -> Pose {
    let mut out = Pose::default();
    r.evaluate(st, trig, inp, &mut Scratch::default(), None, &mut out);
    out
}

fn no_events(_: horizon_mobmodel::pose::RigEvent<'_>) {}

const LOCO: &str = r#"{"controllers":{"loco":{"initial":"idle","states":{
  "idle":{"animations":["idle"],"transitions":[{"to":"walk","when":"q.ground_speed > 0.1"}]},
  "walk":{"animations":["walk"],"blend":1.0,"transitions":[{"to":"idle","when":"q.ground_speed < 0.05"}]}}}}}"#;

#[test]
fn swing_golden_pose_uses_blockbench_space() {
    let r = rig(r#"{"controllers":{}}"#);
    let swing = r.find_animation("swing").unwrap();
    let mut inp = ExprInputs::default();
    let mut pose = Pose::default();
    // Bedrock rotation x 90 at t=1 becomes -90 internally; the hand at (4, 6, 0) swings to +Z.
    r.sample_animation(swing, 1.0, &mut inp, &mut pose);
    let m = bone_matrices(&r.skeleton, &pose);
    let p = m[2].transform_point3(Vec3::new(4.0, 6.0, 0.0));
    assert!(p.abs_diff_eq(Vec3::new(4.0, 12.0, 6.0), 1e-4), "{p}");
    // the head and body did not move
    assert!(m[3].abs_diff_eq(Mat4::IDENTITY, 1e-6));
    // Loop: t = 3 is t = 1.
    let mut pose2 = Pose::default();
    r.sample_animation(swing, 3.0, &mut inp, &mut pose2);
    assert!(bone_matrices(&r.skeleton, &pose2)[2].abs_diff_eq(m[2], 1e-4));
    // Sparse single-bone matrix equals the full one.
    assert!(r.bone_matrix(&pose, BoneIndex(2)).abs_diff_eq(m[2], 1e-5));
}

#[test]
fn bbmodel_rotation_needs_no_conversion() {
    let v: serde_json::Value = serde_json::from_str(
        r#"[{"name":"swing","length":2,"loop":"loop","animators":{"u":{"name":"arm","keyframes":[
            {"channel":"rotation","time":0,"data_points":[{"x":"0","y":"0","z":"0"}]},
            {"channel":"rotation","time":2,"data_points":[{"x":"180","y":"0","z":"0"}]}]}}}]"#,
    )
    .unwrap();
    let mut syms = Symbols::new(&[]);
    let mut lib = AnimationLibrary::new();
    for a in animations_from_bbmodel(&v, &mut syms).unwrap().animations {
        lib.add(a).unwrap();
    }
    let (r, _) = Rig::new(skeleton(), lib, vec![]).unwrap();
    let mut pose = Pose::default();
    r.sample_animation(0, 1.0, &mut ExprInputs::default(), &mut pose);
    let p = bone_matrices(&r.skeleton, &pose)[2].transform_point3(Vec3::new(4.0, 6.0, 0.0));
    assert!(p.abs_diff_eq(Vec3::new(4.0, 12.0, -6.0), 1e-4), "{p}");
}

#[test]
fn state_crossfade_golden() {
    let r = rig(LOCO);
    let mut st = r.initial_state();
    let mut inp = ExprInputs::default();
    r.update(&mut st, 0.1, &mut inp, &mut no_events);
    assert_eq!(eval(&r, &st, &[], &mut inp).bones[3].translation, Vec3::ZERO);
    inp.set(Query::GroundSpeed, 1.0);
    r.update(&mut st, 0.0, &mut inp, &mut no_events); // idle -> walk, fade 1 s
    r.update(&mut st, 0.5, &mut inp, &mut no_events);
    let y = eval(&r, &st, &[], &mut inp).bones[3].translation.y;
    assert!((y - 5.0).abs() < 1e-4, "{y}");
    r.update(&mut st, 0.25, &mut inp, &mut no_events);
    let y = eval(&r, &st, &[], &mut inp).bones[3].translation.y;
    assert!((y - 7.5).abs() < 1e-4, "{y}");
    r.update(&mut st, 0.5, &mut inp, &mut no_events);
    assert!((eval(&r, &st, &[], &mut inp).bones[3].translation.y - 10.0).abs() < 1e-4);
}

fn single_state(anims: &str) -> Rig {
    rig(&format!(r#"{{"controllers":{{"c":{{"states":{{"s":{{"animations":{anims}}}}}}}}}}}"#))
}

#[test]
fn stacking_adds_and_override_replaces() {
    let y = |r: &Rig| eval(r, &r.initial_state(), &[], &mut ExprInputs::default()).bones[3].translation.y;
    assert_eq!(y(&single_state(r#"["walk","bonus"]"#)), 11.0);
    assert_eq!(y(&single_state(r#"["walk","over"]"#)), 7.0);
    assert_eq!(y(&single_state(r#"["walk","over","bonus"]"#)), 8.0);
    // weights: entry weight scales an additive animation, and caps an override's lerp.
    assert_eq!(y(&single_state(r#"["walk",{"name":"bonus","weight":0.5}]"#)), 10.5);
    assert_eq!(y(&single_state(r#"["walk",{"name":"over","weight":0.5}]"#)), 8.5);
}

#[test]
fn layers_mask_weight_and_triggers() {
    let r = rig(
        r#"{"controllers":{
          "base":{"states":{"s":{"animations":["walk"]}}},
          "action":{"bones_mask":["arm"],"triggers":{"wave":"wave"}}}}"#,
    );
    let st = r.initial_state();
    let mut inp = ExprInputs::default();
    let play = |e: f32| [TriggerPlay { layer: 1, trigger: 0, elapsed: e }];
    let p = eval(&r, &st, &play(0.5), &mut inp);
    assert_eq!(p.bones[1].translation, Vec3::new(0.0, 3.0, 0.0)); // arm: driven by the trigger
    assert_eq!(p.bones[3].translation, Vec3::new(0.0, 10.0, 0.0)); // head: masked out of the action layer
    let p = eval(&r, &st, &play(1.5), &mut inp); // finished: no effect
    assert_eq!(p.bones[1].translation, Vec3::ZERO);
    let p = eval(&r, &st, &[], &mut inp);
    assert_eq!(p.bones[1].translation, Vec3::ZERO);
    // A trigger of another layer does nothing here.
    let p = eval(&r, &st, &[TriggerPlay { layer: 0, trigger: 0, elapsed: 0.5 }], &mut inp);
    assert_eq!(p.bones[1].translation, Vec3::ZERO);
}

#[test]
fn layer_modes_and_child_masks() {
    let r = rig(
        r#"{"controllers":{
          "base":{"states":{"s":{"animations":["walk"]}}},
          "top":{"weight":0.5,"bones_mask":["head"],"states":{"s":{"animations":["over"]}}}}}"#,
    );
    let p = eval(&r, &r.initial_state(), &[], &mut ExprInputs::default());
    assert_eq!(p.bones[3].translation.y, 8.5); // lerp(10, 7, 0.5)
    let r = rig(
        r#"{"controllers":{
          "base":{"states":{"s":{"animations":["walk"]}}},
          "top":{"weight":0.5,"mode":"add","bones_mask":["head"],"states":{"s":{"animations":["bonus"]}}}}}"#,
    );
    let p = eval(&r, &r.initial_state(), &[], &mut ExprInputs::default());
    assert_eq!(p.bones[3].translation.y, 10.5);
    // include_children (default): a swing layer masked to "body" drives the arm below it; masked to "head" it does not.
    let swing = |mask: &str| {
        let r = rig(&format!(
            r#"{{"controllers":{{"c":{{"bones_mask":["{mask}"],"states":{{"s":{{"animations":["swing"]}}}}}}}}}}"#
        ));
        let mut st = r.initial_state();
        let mut inp = ExprInputs::default();
        r.update(&mut st, 1.0, &mut inp, &mut no_events);
        eval(&r, &st, &[], &mut inp).bones[1].rotation
    };
    assert_ne!(swing("body"), glam::Quat::IDENTITY);
    assert_eq!(swing("head"), glam::Quat::IDENTITY);
}

#[test]
fn layer_weight_expression() {
    let r = rig(r#"{"controllers":{"c":{"weight":"q.health / 10","states":{"s":{"animations":["walk"]}}}}}"#);
    let mut inp = ExprInputs::default();
    inp.set(Query::Health, 5.0);
    assert_eq!(eval(&r, &r.initial_state(), &[], &mut inp).bones[3].translation.y, 5.0);
    inp.set(Query::Health, 0.0);
    assert_eq!(eval(&r, &r.initial_state(), &[], &mut inp).bones[3].translation.y, 0.0);
}

#[test]
fn sparse_evaluation_matches_the_full_pose() {
    let r = rig(r#"{"controllers":{"c":{"states":{"s":{"animations":["walk","swing"]}}}}}"#);
    let mut st = r.initial_state();
    let mut inp = ExprInputs::default();
    r.update(&mut st, 0.7, &mut inp, &mut no_events);
    let full = eval(&r, &st, &[], &mut inp);
    let needed = r.bone_closure(&[BoneIndex(2)]);
    assert_eq!(needed, [true, true, true, false]);
    let mut sparse = Pose::default();
    r.evaluate(&st, &[], &mut inp, &mut Scratch::default(), Some(&needed), &mut sparse);
    for i in 0..3 {
        assert_eq!(sparse.bones[i], full.bones[i]);
    }
    assert_eq!(sparse.bones[3], Default::default()); // head not asked for
    assert_ne!(full.bones[3], Default::default());
    assert!(r.bone_matrix(&sparse, BoneIndex(2)).abs_diff_eq(bone_matrices(&r.skeleton, &full)[2], 1e-5));
}

#[test]
fn timeline_events_once_per_loop_and_not_while_paused() {
    let r = rig(r#"{"controllers":{"c":{"states":{"s":{"animations":["ticker"]}}}}}"#);
    let mut st = r.initial_state();
    let mut inp = ExprInputs::default();
    let mut log: Vec<String> = Vec::new();
    let mut step = |st: &mut RigState, dt: f32, log: &mut Vec<String>| {
        r.update(st, dt, &mut inp, &mut |e| {
            assert_eq!(e.event.kind, EventKind::Timeline);
            log.push(format!("{}#{}", e.event.name, e.loop_index))
        });
    };
    step(&mut st, 0.0, &mut log); // paused at the very start: nothing, not even "tick"
    assert!(log.is_empty());
    step(&mut st, 0.25, &mut log);
    assert_eq!(log, ["tick#0"]);
    step(&mut st, 0.0, &mut log); // paused
    step(&mut st, 0.0, &mut log);
    assert_eq!(log, ["tick#0"]);
    step(&mut st, 0.5, &mut log); // 0.25 -> 0.75 crosses tock
    assert_eq!(log, ["tick#0", "tock#0"]);
    step(&mut st, 0.5, &mut log); // 0.75 -> 1.25 crosses the loop: tick again
    assert_eq!(log, ["tick#0", "tock#0", "tick#1"]);
    step(&mut st, 1.0, &mut log); // 1.25 -> 2.25: tock#1, tick#2
    assert_eq!(&log[3..], ["tock#1", "tick#2"]);
}

#[test]
fn trigger_events() {
    let r = rig(r#"{"controllers":{"a":{"triggers":{"wave":"wave"}}}}"#);
    let mut n = Vec::new();
    r.trigger_events(0, 0, 0.0, 0.4, &mut |e| n.push(e.event.name.clone()));
    assert!(n.is_empty());
    r.trigger_events(0, 0, 0.4, 0.6, &mut |e| n.push(e.event.name.clone()));
    assert_eq!(n, ["hit"]);
    r.trigger_events(0, 0, 0.6, 5.0, &mut |e| n.push(e.event.name.clone()));
    assert_eq!(n, ["hit"]);
    r.trigger_events(3, 0, 0.0, 5.0, &mut |_| panic!());
}

#[test]
fn binding_errors_and_warnings() {
    let mut syms = Symbols::new(&[]);
    let mut lib = AnimationLibrary::new();
    for a in parse_animation_file(
        r#"{"animations":{"a":{"bones":{"tail":{"rotation":[1,2,3]},"arm":{"rotation":[1,2,3]}}}}}"#,
        &mut syms,
    )
    .unwrap()
    .animations
    {
        lib.add(a).unwrap();
    }
    let bad = parse_controllers(r#"{"controllers":{"c":{"states":{"s":{"animations":["nope"]}}}}}"#, &mut syms).unwrap();
    assert!(Rig::new(skeleton(), lib.clone(), bad.controllers).unwrap_err().message.contains("nope"));
    let bad = parse_controllers(r#"{"controllers":{"c":{"triggers":{"t":"nope"}}}}"#, &mut syms).unwrap();
    assert!(Rig::new(skeleton(), lib.clone(), bad.controllers).is_err());
    let ok = parse_controllers(r#"{"controllers":{"c":{"bones_mask":["ghost"]}}}"#, &mut syms).unwrap();
    let (_, w) = Rig::new(skeleton(), lib, ok.controllers).unwrap();
    let w: Vec<_> = w.iter().map(|w| w.to_string()).collect();
    assert!(w.iter().any(|m| m.contains("tail")), "{w:?}");
    assert!(w.iter().any(|m| m.contains("ghost")), "{w:?}");
}

#[test]
fn evaluation_is_deterministic_and_restores_anim_time() {
    let r = rig(LOCO);
    let mut st = r.initial_state();
    let mut inp = ExprInputs::default();
    inp.set(Query::GroundSpeed, 1.0);
    r.update(&mut st, 0.3, &mut inp, &mut no_events);
    r.update(&mut st, 0.3, &mut inp, &mut no_events);
    inp.set(Query::AnimTime, 42.0);
    let a = eval(&r, &st, &[], &mut inp);
    assert_eq!(inp.get(Query::AnimTime), 42.0);
    let b = eval(&r, &st, &[], &mut inp);
    assert_eq!(a, b);
}

#[test]
fn degenerate_inputs_never_panic() {
    let r = rig(LOCO);
    let mut st = r.initial_state();
    let mut inp = ExprInputs::default();
    for dt in [f32::NAN, f32::INFINITY, -1.0, 1e30, 0.0] {
        r.update(&mut st, dt, &mut inp, &mut no_events);
        let p = eval(&r, &st, &[TriggerPlay { layer: 9, trigger: 9, elapsed: f32::NAN }], &mut inp);
        assert!(p.bones.iter().all(|b| b.translation.is_finite() && b.rotation.is_finite() && b.scale.is_finite()));
    }
    let (e, _) = Rig::new(Skeleton::default(), AnimationLibrary::new(), vec![]).unwrap();
    let p = eval(&e, &e.initial_state(), &[], &mut inp);
    assert!(p.bones.is_empty());
}

// ---------------------------------------------------------------------------------------------------------------
// Fix round: layer fade to "no state", bbmodel easing, deterministic event order
// ---------------------------------------------------------------------------------------------------------------

fn fade_rig(blend: f32) -> Rig {
    rig(&format!(
        r#"{{"controllers":{{"l":{{"initial":"off","states":{{
  "off":{{"inactive":true,"blend":{blend},"transitions":[{{"to":"on","when":"q.ground_speed > 0.5"}}]}},
  "on":{{"animations":["walk"],"blend":{blend},"transitions":[{{"to":"off","when":"q.ground_speed < 0.5"}}]}}}}}}}}}}"#
    ))
}

#[test]
fn layer_fades_in_and_out_through_an_inactive_state() {
    let r = fade_rig(1.0);
    let mut st = r.initial_state();
    let mut inp = ExprInputs::default();
    let head_y = |st: &RigState, inp: &mut ExprInputs| eval(&r, st, &[], inp).bones[3].translation.y;
    r.update(&mut st, 0.1, &mut inp, &mut no_events);
    assert_eq!(head_y(&st, &mut inp), 0.0, "an inactive layer contributes nothing");
    inp.set(Query::GroundSpeed, 1.0);
    r.update(&mut st, 0.0, &mut inp, &mut no_events); // off -> on, 1 s fade-in of the weight
    assert!((head_y(&st, &mut inp) - 0.0).abs() < 1e-4);
    r.update(&mut st, 0.5, &mut inp, &mut no_events);
    assert!((head_y(&st, &mut inp) - 5.0).abs() < 1e-4);
    r.update(&mut st, 0.5, &mut inp, &mut no_events);
    assert!((head_y(&st, &mut inp) - 10.0).abs() < 1e-4);
    inp.set(Query::GroundSpeed, 0.0);
    r.update(&mut st, 0.0, &mut inp, &mut no_events); // on -> off: fade out, the old state keeps playing
    assert!((head_y(&st, &mut inp) - 10.0).abs() < 1e-4, "no jump at the start");
    r.update(&mut st, 0.25, &mut inp, &mut no_events);
    assert!((head_y(&st, &mut inp) - 7.5).abs() < 1e-4);
    r.update(&mut st, 0.5, &mut inp, &mut no_events);
    assert!((head_y(&st, &mut inp) - 2.5).abs() < 1e-4);
    r.update(&mut st, 0.5, &mut inp, &mut no_events);
    assert!(st.layers[0].prev.is_none());
    assert_eq!(head_y(&st, &mut inp), 0.0, "layer inactive after the fade");
    assert_eq!(eval(&r, &st, &[], &mut inp), Pose::rest(&r.skeleton));
}

#[test]
fn inactive_switch_with_blend_zero_is_instant() {
    let r = fade_rig(0.0);
    let mut st = r.initial_state();
    let mut inp = ExprInputs::default();
    inp.set(Query::GroundSpeed, 1.0);
    r.update(&mut st, 0.05, &mut inp, &mut no_events);
    assert!((eval(&r, &st, &[], &mut inp).bones[3].translation.y - 10.0).abs() < 1e-4);
    inp.set(Query::GroundSpeed, 0.0);
    r.update(&mut st, 0.05, &mut inp, &mut no_events);
    assert_eq!(eval(&r, &st, &[], &mut inp).bones[3].translation.y, 0.0);
}

fn bb_anim(v: &str) -> horizon_mobmodel::anim::Animation {
    let v: serde_json::Value = serde_json::from_str(v).unwrap();
    animations_from_bbmodel(&v, &mut Symbols::new(&[])).unwrap().animations.remove(0)
}

fn rot_x(a: &horizon_mobmodel::anim::Animation, t: f32) -> f32 {
    a.bones[0].rotation.as_ref().unwrap().sample(t, &ExprInputs::default()).x
}

#[test]
fn bbmodel_keyframe_easing_shapes_the_segment_ending_at_the_key() {
    let kf = |t: f32, x: &str, extra: &str| {
        format!(r#"{{"channel":"rotation","time":{t},"data_points":[{{"x":"{x}","y":"0","z":"0"}}]{extra}}}"#)
    };
    let mk = |k1: String| {
        bb_anim(&format!(
            r#"[{{"name":"a","length":1,"loop":"once","animators":{{"u":{{"name":"b","keyframes":[{},{}]}}}}}}]"#,
            kf(0.0, "0", ""),
            k1
        ))
    };
    let plain = mk(kf(1.0, "100", ""));
    assert!((rot_x(&plain, 0.5) - 50.0).abs() < 1e-4);
    let quad = mk(kf(1.0, "100", r#","easing":"easeInQuad""#));
    assert!((rot_x(&quad, 0.5) - 25.0).abs() < 1e-4);
    let back_a = mk(kf(1.0, "100", r#","easing":"easeOutBack","easingArgs":[3.0]"#));
    let back_b = mk(kf(1.0, "100", r#","easing":{"type":"easeOutBack","args":[3.0]}"#));
    let back_def = mk(kf(1.0, "100", r#","easing":"easeOutBack""#));
    assert!((rot_x(&back_a, 0.5) - rot_x(&back_b, 0.5)).abs() < 1e-5, "nested form equals the flat form");
    assert!((rot_x(&back_a, 0.5) - rot_x(&back_def, 0.5)).abs() > 1.0, "easingArgs is used");
    // the easing of the START key does not apply to the segment that starts there
    let start = bb_anim(
        r#"[{"name":"a","length":1,"animators":{"u":{"name":"b","keyframes":[
        {"channel":"rotation","time":0,"easing":"easeInQuad","data_points":[{"x":"0","y":"0","z":"0"}]},
        {"channel":"rotation","time":1,"data_points":[{"x":"100","y":"0","z":"0"}]}]}}}]"#,
    );
    assert!((rot_x(&start, 0.5) - 50.0).abs() < 1e-4);
}

#[test]
fn same_time_events_have_one_order_in_both_formats() {
    use horizon_mobmodel::anim::EventKind::*;
    let bed = parse_animation_file(
        r#"{"animations":{"a":{"animation_length":1,
        "particle_effects":{"0.5":[{"effect":"p1"},{"effect":"p2"}]},
        "sound_effects":{"0.5":{"effect":"s"},"0.2":{"effect":"s0"}},
        "timeline":{"0.5":["t1","t2"]}}}}"#,
        &mut Symbols::new(&[]),
    )
    .unwrap()
    .animations
    .remove(0);
    let bb = bb_anim(
        r#"[{"name":"a","length":1,"animators":{"e":{"type":"effect","name":"E","keyframes":[
        {"channel":"particle","time":0.5,"data_points":[{"effect":"p1"},{"effect":"p2"}]},
        {"channel":"sound","time":0.2,"data_points":[{"effect":"s0"}]},
        {"channel":"sound","time":0.5,"data_points":[{"effect":"s"}]},
        {"channel":"timeline","time":0.5,"data_points":[{"script":"t1"},{"script":"t2"}]}]}}}]"#,
    );
    let key = |a: &horizon_mobmodel::anim::Animation| {
        a.events.iter().map(|e| (e.kind, e.name.clone())).collect::<Vec<_>>()
    };
    let want: Vec<_> = [(Sound, "s0"), (Timeline, "t1"), (Timeline, "t2"), (Sound, "s"), (Particle, "p1"), (Particle, "p2")]
        .iter()
        .map(|(k, n)| (*k, n.to_string()))
        .collect();
    assert_eq!(key(&bed), want);
    assert_eq!(key(&bb), want);
}
