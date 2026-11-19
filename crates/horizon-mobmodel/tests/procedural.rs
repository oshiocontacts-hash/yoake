//! F4M-8: two-bone IK, look-at, aim, spring, follow, counter-rotate, JSON parsing, script bone writes, and the Cinder
//! Fox with its real fixture files.

use glam::{Mat4, Quat, Vec3};
use horizon_mobmodel::anim::{parse_animation_file, AnimationLibrary};
use horizon_mobmodel::controller::parse_controllers;
use horizon_mobmodel::expr::{ExprInputs, Symbols};
use horizon_mobmodel::ik::{angle_between_deg, euler_deg_to_quat};
use horizon_mobmodel::model::Locator;
use horizon_mobmodel::pose::{Rig, Scratch};
use horizon_mobmodel::procedural::*;
use horizon_mobmodel::skeleton::{bone_matrices, Bone, BoneIndex, Pose, Skeleton};
use horizon_mobmodel::{load_bbmodel, load_geo_json};

fn bone(name: &str, parent: Option<u16>, pivot: [f32; 3]) -> Bone {
    Bone { name: name.into(), parent: parent.map(BoneIndex), pivot: Vec3::from(pivot), rotation: Vec3::ZERO }
}

/// Joint position of bone `i` in the posed model.
/// Rotation angle of a quaternion in degrees, accurate for tiny angles.
fn qangle(q: Quat) -> f32 {
    2.0 * q.xyz().length().atan2(q.w.abs()).to_degrees()
}

fn joint(sk: &Skeleton, pose: &Pose, i: usize) -> Vec3 {
    bone_matrices(sk, pose)[i].transform_point3(sk.bones[i].pivot)
}

/// A direction of the bone's geometry frame in model space.
fn axis_world(sk: &Skeleton, pose: &Pose, i: usize, axis: Vec3) -> Vec3 {
    bone_matrices(sk, pose)[i].transform_vector3(axis).normalize()
}

fn set(sk: &Skeleton, json: &str) -> ProceduralSet {
    let mut syms = Symbols::new(&["a"]);
    parse_procedural_str("test.json", json, sk, &[], &mut syms).unwrap_or_else(|e| panic!("{e}"))
}

fn run(set: &ProceduralSet, st: &mut ProceduralState, pose: &mut Pose, inp: &ProceduralInputs) -> EvalReport {
    set.evaluate(pose, st, inp)
}

// ---------------------------------------------------------------------------------------------------------------
// Two-bone IK
// ---------------------------------------------------------------------------------------------------------------

/// body(0) <- thigh(1) <- shin(2) <- foot(3); thigh 8 above shin, shin 8 above foot.
fn leg() -> Skeleton {
    Skeleton {
        bones: vec![
            bone("body", None, [0.0, 20.0, 0.0]),
            bone("thigh", Some(0), [0.0, 16.0, 0.0]),
            bone("shin", Some(1), [0.0, 8.0, 0.0]),
            bone("foot", Some(2), [0.0, 0.0, 0.0]),
        ],
    }
}

fn ik_set(sk: &Skeleton, extra: &str) -> ProceduralSet {
    set(
        sk,
        &format!(
            r#"[{{"type":"two_bone_ik","root":"thigh","mid":"shin","tip":"foot","target":{{"input":0}},"fade":0 {extra}}}]"#
        ),
    )
}

fn solve_ik(sk: &Skeleton, set: &ProceduralSet, target: Vec3) -> Pose {
    let mut pose = Pose::rest(sk);
    let mut st = ProceduralState::new(set);
    let e = ExprInputs::default();
    let inp = ProceduralInputs::new(&e, 0.05).with_target(TargetSource::Custom(0), target);
    let rep = run(set, &mut st, &mut pose, &inp);
    assert_eq!(rep.applied, 1);
    pose
}

fn assert_lengths(sk: &Skeleton, pose: &Pose) {
    let (a, b, c) = (joint(sk, pose, 1), joint(sk, pose, 2), joint(sk, pose, 3));
    assert!(((b - a).length() - 8.0).abs() < 1e-3, "upper bone length {}", (b - a).length());
    assert!(((c - b).length() - 8.0).abs() < 1e-3, "lower bone length {}", (c - b).length());
}

#[test]
fn ik_reachable_target_is_hit() {
    let sk = leg();
    let s = ik_set(&sk, "");
    for t in [Vec3::new(3.0, 6.0, 4.0), Vec3::new(-5.0, 10.0, 2.0), Vec3::new(0.5, 4.0, -8.0), Vec3::new(0.0, 5.0, 0.0)] {
        let pose = solve_ik(&sk, &s, t);
        let foot = joint(&sk, &pose, 3);
        assert!((foot - t).length() < 1e-3, "target {t} reached {foot}");
        assert_lengths(&sk, &pose);
    }
}

#[test]
fn ik_out_of_reach_extends_fully_toward_target() {
    let sk = leg();
    let s = ik_set(&sk, "");
    let root = Vec3::new(0.0, 16.0, 0.0);
    for t in [Vec3::new(40.0, -10.0, 20.0), Vec3::new(0.0, -100.0, 0.0), Vec3::new(-3.0, 40.0, 0.0)] {
        let pose = solve_ik(&sk, &s, t);
        let foot = joint(&sk, &pose, 3);
        let dir = (t - root).normalize();
        assert!((foot - (root + dir * 16.0)).length() < 1e-3, "foot {foot}");
        // fully extended: all three joints on the line toward the target
        let mid = joint(&sk, &pose, 2);
        assert!((mid - (root + dir * 8.0)).length() < 1e-3, "mid {mid}");
        assert_lengths(&sk, &pose);
    }
}

#[test]
fn ik_too_close_target_folds_and_keeps_lengths() {
    let sk = leg();
    let s = ik_set(&sk, "");
    let pose = solve_ik(&sk, &s, Vec3::new(0.0, 16.0, 0.0)); // on the root: |l1 - l2| = 0 is reachable
    assert!((joint(&sk, &pose, 3) - Vec3::new(0.0, 16.0, 0.0)).length() < 1e-2);
    assert_lengths(&sk, &pose);
}

#[test]
fn ik_pole_selects_the_bend_side() {
    let sk = leg();
    let target = Vec3::new(0.0, 6.0, 0.0);
    for (pole, sign) in [(r#""pole":{"dir":[0,0,1]}"#, 1.0), (r#""pole":{"dir":[0,0,-1]}"#, -1.0), (r#""pole":{"dir":[1,0,0]}"#, 0.0)] {
        let s = ik_set(&sk, &format!(",{pole}"));
        let pose = solve_ik(&sk, &s, target);
        let mid = joint(&sk, &pose, 2);
        assert!((joint(&sk, &pose, 3) - target).length() < 1e-3);
        assert_lengths(&sk, &pose);
        if sign != 0.0 {
            assert!(mid.z * sign > 3.0, "pole {pole}: mid {mid}");
            assert!(mid.x.abs() < 1e-3);
        } else {
            assert!(mid.x > 3.0 && mid.z.abs() < 1e-3, "pole {pole}: mid {mid}");
        }
    }
    // a pole point and a pole bone work too
    let s = ik_set(&sk, r#","pole":{"point":[0,16,-30]}"#);
    assert!(joint(&sk, &solve_ik(&sk, &s, target), 2).z < -3.0);
    let s = ik_set(&sk, r#","pole":{"bone":"body"}"#);
    let _ = solve_ik(&sk, &s, target);
}

#[test]
fn ik_keeps_lengths_for_random_targets() {
    let sk = leg();
    let s = ik_set(&sk, r#","pole":{"dir":[0.3,0.2,1]}"#);
    let mut seed = 12345u32;
    let mut rnd = || {
        seed = seed.wrapping_mul(1664525).wrapping_add(1013904223);
        (seed >> 8) as f32 / (1u32 << 24) as f32 * 40.0 - 20.0
    };
    for _ in 0..200 {
        let t = Vec3::new(rnd(), rnd() + 10.0, rnd());
        let pose = solve_ik(&sk, &s, t);
        assert_lengths(&sk, &pose);
        let foot = joint(&sk, &pose, 3);
        let root = Vec3::new(0.0, 16.0, 0.0);
        let d = (t - root).length();
        let expect = if d <= 16.0 { t } else { root + (t - root) / d * 16.0 };
        assert!((foot - expect).length() < 2e-3, "target {t}");
    }
}

#[test]
fn ik_on_animated_and_rotated_parents_still_reaches() {
    let sk = leg();
    let s = ik_set(&sk, r#","pole":{"dir":[0,0,1]}"#);
    let mut pose = Pose::rest(&sk);
    pose.bones[0].rotation = euler_deg_to_quat(Vec3::new(20.0, 70.0, -15.0));
    pose.bones[0].translation = Vec3::new(1.0, -2.0, 3.0);
    pose.bones[1].rotation = euler_deg_to_quat(Vec3::new(-30.0, 10.0, 5.0));
    pose.bones[2].rotation = euler_deg_to_quat(Vec3::new(40.0, 0.0, 0.0));
    let root = joint(&sk, &pose, 1);
    let target = root + Vec3::new(2.0, -9.0, 5.0);
    let mut st = ProceduralState::new(&s);
    let e = ExprInputs::default();
    run(&s, &mut st, &mut pose, &ProceduralInputs::new(&e, 0.05).with_target(TargetSource::Custom(0), target));
    assert!((joint(&sk, &pose, 3) - target).length() < 1e-3);
    assert!(((joint(&sk, &pose, 2) - joint(&sk, &pose, 1)).length() - 8.0).abs() < 1e-3);
    assert!(((joint(&sk, &pose, 3) - joint(&sk, &pose, 2)).length() - 8.0).abs() < 1e-3);
}

#[test]
fn ik_weight_blends_and_missing_target_fades_out() {
    let sk = leg();
    let target = Vec3::new(5.0, 8.0, 0.0);
    let full = solve_ik(&sk, &ik_set(&sk, ""), target);
    let half = solve_ik(&sk, &ik_set(&sk, r#","weight":0.5"#), target);
    let rest_q = Quat::IDENTITY;
    for i in [1usize, 2] {
        let want = rest_q.slerp(full.bones[i].rotation, 0.5);
        assert!(half.bones[i].rotation.abs_diff_eq(want, 1e-4) || half.bones[i].rotation.abs_diff_eq(-want, 1e-4));
    }
    // activation: target present then gone; fade 0.2 s, dt 0.05 -> weight 1, .75, .5, ..
    let s = set(&sk, r#"[{"type":"two_bone_ik","root":"thigh","mid":"shin","tip":"foot","target":{"input":0},"fade":0.2}]"#);
    let mut st = ProceduralState::new(&s);
    let e = ExprInputs::default();
    let mut pose = Pose::rest(&sk);
    run(&s, &mut st, &mut pose, &ProceduralInputs::new(&e, 0.05).with_target(TargetSource::Custom(0), target));
    let mut angles = vec![];
    for _ in 0..6 {
        let mut p = Pose::rest(&sk);
        let rep = run(&s, &mut st, &mut p, &ProceduralInputs::new(&e, 0.05));
        angles.push(p.bones[1].rotation.angle_between(Quat::IDENTITY));
        assert_eq!(rep.missing_target, 0);
    }
    assert!(angles[0] < full.bones[1].rotation.angle_between(Quat::IDENTITY));
    assert!(angles.windows(2).all(|w| w[1] <= w[0] + 1e-6), "{angles:?}");
    assert!(angles[5] < 1e-6, "fully faded out: {angles:?}");
}

#[test]
fn ik_condition_uses_the_synced_enum_index() {
    let sk = leg();
    let s = set(
        &sk,
        r#"[{"type":"two_bone_ik","root":"thigh","mid":"shin","tip":"foot","target":{"point":[4,6,0]},"fade":0,"when":"q.synced('a') == 1"}]"#,
    );
    let mut st = ProceduralState::new(&s);
    let mut e = ExprInputs::default();
    let mut p = Pose::rest(&sk);
    let rep = run(&s, &mut st, &mut p, &ProceduralInputs::new(&e, 0.05));
    assert_eq!((rep.applied, rep.inactive), (0, 1));
    assert_eq!(p, Pose::rest(&sk));
    e.synced[0] = 1.0;
    let rep = run(&s, &mut st, &mut p, &ProceduralInputs::new(&e, 0.05));
    assert_eq!(rep.applied, 1);
    assert!((joint(&sk, &p, 3) - Vec3::new(4.0, 6.0, 0.0)).length() < 1e-3);
}

#[test]
fn ik_with_a_locator_target_and_tip_locator() {
    let sk = leg();
    let locators = vec![
        Locator { name: "mark".into(), bone: BoneIndex(0), offset: Vec3::new(3.0, 8.0, 2.0), rotation: Vec3::ZERO, null_object: false },
        Locator { name: "toe".into(), bone: BoneIndex(3), offset: Vec3::new(0.0, 0.0, -3.0), rotation: Vec3::ZERO, null_object: false },
    ];
    let mut syms = Symbols::new(&[]);
    let s = parse_procedural_str(
        "t.json",
        r#"[{"type":"two_bone_ik","root":"thigh","mid":"shin","tip_locator":"toe","target":{"locator":"mark"},"fade":0}]"#,
        &sk,
        &locators,
        &mut syms,
    )
    .unwrap();
    let mut pose = Pose::rest(&sk);
    pose.bones[0].translation = Vec3::new(2.0, 0.0, 0.0); // the body (and the locator) moves
    let mut st = ProceduralState::new(&s);
    let e = ExprInputs::default();
    run(&s, &mut st, &mut pose, &ProceduralInputs::new(&e, 0.05));
    let m = bone_matrices(&sk, &pose);
    let toe = m[3].transform_point3(Vec3::new(0.0, 0.0, -3.0));
    assert!((toe - Vec3::new(5.0, 8.0, 2.0)).length() < 1e-3, "{toe}");
}

// ---------------------------------------------------------------------------------------------------------------
// look_at and aim
// ---------------------------------------------------------------------------------------------------------------

/// body(0) <- head(1); the head looks along -Z.
fn head() -> Skeleton {
    Skeleton { bones: vec![bone("body", None, [0.0, 10.0, 0.0]), bone("head", Some(0), [0.0, 16.0, 0.0])] }
}

fn look_once(sk: &Skeleton, json: &str, pose: &mut Pose, target: Vec3, dt: f32, st: &mut ProceduralState, s: &ProceduralSet) {
    let _ = json;
    let e = ExprInputs::default();
    run(s, st, pose, &ProceduralInputs::new(&e, dt).with_target(TargetSource::View, target));
    let _ = sk;
}

#[test]
fn look_at_aims_exactly_within_limits_even_under_a_rotated_parent() {
    let sk = head();
    let json = r#"[{"type":"look_at","bone":"head","target":"view","yaw":180,"pitch":90,"speed":0,"fade":0}]"#;
    let s = set(&sk, json);
    let mut st = ProceduralState::new(&s);
    let mut pose = Pose::rest(&sk);
    pose.bones[0].rotation = euler_deg_to_quat(Vec3::new(10.0, 90.0, 0.0));
    let target = Vec3::new(30.0, 25.0, -20.0);
    look_once(&sk, json, &mut pose, target, 0.05, &mut st, &s);
    let eye = joint(&sk, &pose, 1);
    let fwd = axis_world(&sk, &pose, 1, Vec3::NEG_Z);
    assert!(angle_between_deg(fwd, target - eye) < 0.01, "forward {fwd}");
}

#[test]
fn look_at_clamps_yaw_and_pitch() {
    let sk = head();
    let json = r#"[{"type":"look_at","bone":"head","target":"view","yaw":60,"pitch":40,"speed":0,"fade":0}]"#;
    let s = set(&sk, json);
    let eye = Vec3::new(0.0, 16.0, 0.0);
    let cases: [(Vec3, f32, f32); 5] = [
        (Vec3::new(100.0, 0.0, 0.0), 60.0, 0.0),   // 90 deg right -> 60
        (Vec3::new(-100.0, 0.0, 0.0), -60.0, 0.0), // left
        (Vec3::new(0.0, 100.0, -1.0), 0.0, 40.0),  // straight up -> 40
        (Vec3::new(0.0, -100.0, -1.0), 0.0, -40.0),
        (Vec3::new(10.0, 0.0, -10.0), 45.0, 0.0), // inside the limit
    ];
    for (offset, yaw, pitch) in cases {
        let mut st = ProceduralState::new(&s);
        let mut pose = Pose::rest(&sk);
        look_once(&sk, json, &mut pose, eye + offset, 0.05, &mut st, &s);
        let fwd = axis_world(&sk, &pose, 1, Vec3::NEG_Z);
        let want = Quat::from_axis_angle(Vec3::X, pitch.to_radians()) * Vec3::NEG_Z;
        let want = Quat::from_axis_angle(Vec3::Y, (-yaw).to_radians()) * want;
        assert!(angle_between_deg(fwd, want) < 0.01, "offset {offset}: forward {fwd} want {want}");
    }
}

#[test]
fn look_at_speed_limit_and_relax() {
    let sk = head();
    let json = r#"[{"type":"look_at","bone":"head","target":"view","yaw":180,"pitch":90,"speed":90,"fade":0}]"#;
    let s = set(&sk, json);
    let mut st = ProceduralState::new(&s);
    let target = Vec3::new(100.0, 16.0, 0.0); // 90 deg to the right
    let yaw_of = |pose: &Pose| angle_between_deg(axis_world(&sk, pose, 1, Vec3::NEG_Z), Vec3::NEG_Z);
    let mut last = 0.0;
    for f in 1..=30 {
        let mut pose = Pose::rest(&sk);
        look_once(&sk, json, &mut pose, target, 0.05, &mut st, &s);
        let a = yaw_of(&pose);
        if f <= 20 {
            assert!((a - 4.5 * f as f32).abs() < 1e-2, "frame {f}: {a}");
        }
        assert!(a <= 90.0 + 1e-3 && a >= last - 1e-3);
        last = a;
    }
    assert!((last - 90.0).abs() < 1e-2);
    // no target: relaxes back at the same speed
    let e = ExprInputs::default();
    for f in 1..=10 {
        let mut pose = Pose::rest(&sk);
        run(&s, &mut st, &mut pose, &ProceduralInputs::new(&e, 0.05));
        assert!((yaw_of(&pose) - (90.0 - 4.5 * f as f32)).abs() < 1e-2, "relax frame {f}");
    }
}

#[test]
fn look_at_activation_fades_in() {
    let sk = head();
    let s = set(
        &sk,
        r#"[{"type":"look_at","bone":"head","target":"view","yaw":90,"pitch":90,"speed":0,"fade":0.5,"when":"q.synced('a') == 1"}]"#,
    );
    let mut st = ProceduralState::new(&s);
    let mut e = ExprInputs::default();
    let target = Vec3::new(100.0, 16.0, 0.0);
    let mut pose = Pose::rest(&sk);
    run(&s, &mut st, &mut pose, &ProceduralInputs::new(&e, 0.1).with_target(TargetSource::View, target));
    assert_eq!(pose, Pose::rest(&sk));
    e.synced[0] = 1.0;
    let mut got = vec![];
    for _ in 0..3 {
        let mut pose = Pose::rest(&sk);
        run(&s, &mut st, &mut pose, &ProceduralInputs::new(&e, 0.1).with_target(TargetSource::View, target));
        got.push(angle_between_deg(axis_world(&sk, &pose, 1, Vec3::NEG_Z), Vec3::NEG_Z));
    }
    for (i, a) in got.iter().enumerate() {
        assert!((a - 90.0 * 0.2 * (i + 1) as f32).abs() < 0.05, "{got:?}");
    }
}

#[test]
fn aim_turns_an_arbitrary_axis_with_cone_and_speed() {
    // arm points along +X in its geometry frame
    let sk = Skeleton { bones: vec![bone("arm", None, [0.0, 10.0, 0.0])] };
    let json = r#"[{"type":"aim","bone":"arm","axis":[1,0,0],"target":"target","max_angle":50,"speed":0,"fade":0}]"#;
    let s = set(&sk, json);
    let mut st = ProceduralState::new(&s);
    let e = ExprInputs::default();
    let mut pose = Pose::rest(&sk);
    // target straight up: 90 deg from +X, clamped to the 50 deg cone, in the plane X-Y
    run(&s, &mut st, &mut pose, &ProceduralInputs::new(&e, 0.05).with_target(TargetSource::Target, Vec3::new(0.0, 110.0, 0.0)));
    let dir = axis_world(&sk, &pose, 0, Vec3::X);
    assert!((angle_between_deg(dir, Vec3::X) - 50.0).abs() < 0.01);
    assert!(dir.y > 0.7 && dir.z.abs() < 1e-4);
    // inside the cone: exact
    let mut st = ProceduralState::new(&s);
    let mut pose = Pose::rest(&sk);
    let t = Vec3::new(20.0, 10.0, 10.0);
    run(&s, &mut st, &mut pose, &ProceduralInputs::new(&e, 0.05).with_target(TargetSource::Target, t));
    assert!(angle_between_deg(axis_world(&sk, &pose, 0, Vec3::X), t - Vec3::new(0.0, 10.0, 0.0)) < 0.01);
    // speed
    let s2 = set(&sk, r#"[{"type":"aim","bone":"arm","axis":"x","target":"target","max_angle":180,"speed":100,"fade":0}]"#);
    let mut st = ProceduralState::new(&s2);
    for f in 1..=4 {
        let mut pose = Pose::rest(&sk);
        run(&s2, &mut st, &mut pose, &ProceduralInputs::new(&e, 0.05).with_target(TargetSource::Target, Vec3::new(0.0, 100.0, 0.0)));
        let a = angle_between_deg(axis_world(&sk, &pose, 0, Vec3::X), Vec3::X);
        assert!((a - 5.0 * f as f32).abs() < 1e-2, "frame {f}: {a}");
    }
}

#[test]
fn nan_targets_and_pose_do_not_poison_anything() {
    let sk = head();
    let s = set(&sk, r#"[{"type":"look_at","bone":"head","target":"view","fade":0},{"type":"counter_rotate","bone":"head"}]"#);
    let mut st = ProceduralState::new(&s);
    let e = ExprInputs::default();
    let mut pose = Pose::rest(&sk);
    let inp = ProceduralInputs::new(&e, f32::NAN).with_target(TargetSource::View, Vec3::new(f32::NAN, 0.0, 1.0));
    let rep = run(&s, &mut st, &mut pose, &inp);
    assert_eq!(rep.missing_target, 1);
    assert!(pose.bones.iter().all(|b| b.rotation.is_finite() && b.translation.is_finite()));
    let mut bad = Pose::rest(&sk);
    bad.bones[0].rotation = Quat::from_xyzw(f32::NAN, 0.0, 0.0, 1.0);
    run(&s, &mut st, &mut bad, &ProceduralInputs::new(&e, 0.05).with_target(TargetSource::View, Vec3::X));
    assert!(bad.bones[1].rotation.is_finite());
    // pose too short / state of another set
    let mut short = Pose { bones: vec![] };
    assert!(run(&s, &mut st, &mut short, &inp).pose_mismatch);
    let other = set(&leg(), r#"[{"type":"counter_rotate","bone":"foot"}]"#);
    let mut other_state = ProceduralState::new(&other);
    assert!(run(&s, &mut other_state, &mut Pose::rest(&sk), &inp).state_mismatch);
}

// ---------------------------------------------------------------------------------------------------------------
// spring
// ---------------------------------------------------------------------------------------------------------------

/// body(0) <- tail1(1) <- tail2(2) <- tail3(3), along +Z at height 10.
fn tail() -> Skeleton {
    Skeleton {
        bones: vec![
            bone("body", None, [0.0, 10.0, 0.0]),
            bone("tail1", Some(0), [0.0, 10.0, 4.0]),
            bone("tail2", Some(1), [0.0, 10.0, 8.0]),
            bone("tail3", Some(2), [0.0, 10.0, 12.0]),
        ],
    }
}

fn spring_json(stiffness: f32, damping: f32, gravity: f32, max_angle: f32) -> String {
    format!(
        r#"[{{"type":"spring","bones":["tail1","tail2","tail3"],"stiffness":{stiffness},"damping":{damping},"gravity":{gravity},"max_angle":{max_angle},"tip_length":4,"fade":0}}]"#
    )
}

/// Tip point of the whole tail (tail3's pivot plus 4 along its geometry +Z) in the posed model.
fn tail_tip(sk: &Skeleton, pose: &Pose) -> Vec3 {
    bone_matrices(sk, pose)[3].transform_point3(Vec3::new(0.0, 10.0, 16.0))
}

#[test]
fn spring_settles_under_gravity() {
    let sk = tail();
    let s = set(&sk, &spring_json(60.0, 6.0, 160.0, 90.0));
    let mut st = ProceduralState::new(&s);
    let e = ExprInputs::default();
    let mut prev = Pose::rest(&sk);
    let mut tip_y = vec![];
    for f in 0..900 {
        let mut pose = Pose::rest(&sk);
        run(&s, &mut st, &mut pose, &ProceduralInputs::new(&e, 1.0 / 60.0));
        assert!(pose.bones.iter().all(|b| b.rotation.is_finite()));
        // bone lengths: the joints stay where the rotations put them, distances between pivots are unchanged
        for i in 2..4 {
            let d = (joint(&sk, &pose, i) - joint(&sk, &pose, i - 1)).length();
            assert!((d - 4.0).abs() < 1e-3, "frame {f}: joint distance {d}");
        }
        if f == 899 {
            for i in 1..4 {
                assert!(pose.bones[i].rotation.angle_between(prev.bones[i].rotation) < 1e-4, "still moving at the end");
            }
        }
        tip_y.push(tail_tip(&sk, &pose).y);
        prev = pose;
    }
    assert!(*tip_y.last().unwrap() < 10.0 - 0.5, "the tail sags: {}", tip_y.last().unwrap());
    // overdamped enough: no wild swing after the first second
    assert!(tip_y[60..].iter().all(|y| *y < 10.0 + 1e-3));
}

#[test]
fn spring_without_forces_stays_on_the_animation() {
    let sk = tail();
    let s = set(&sk, &spring_json(60.0, 4.0, 0.0, 90.0));
    let mut st = ProceduralState::new(&s);
    let e = ExprInputs::default();
    for _ in 0..120 {
        let mut pose = Pose::rest(&sk);
        run(&s, &mut st, &mut pose, &ProceduralInputs::new(&e, 1.0 / 60.0));
        for b in &pose.bones {
            assert!(qangle(b.rotation) < 0.05, "{}", qangle(b.rotation));
        }
    }
}

#[test]
fn spring_is_stable_for_any_dt_and_nan_input() {
    let sk = tail();
    let s = set(&sk, &spring_json(10_000.0, 0.0, 600.0, 70.0));
    let e = ExprInputs::default();
    for dt in [0.0, 1.0e-9, 0.016, 0.1, 0.5, 10.0, 1.0e30, -1.0, f32::NAN, f32::INFINITY, f32::NEG_INFINITY] {
        let mut st = ProceduralState::new(&s);
        for f in 0..400 {
            let mut pose = Pose::rest(&sk);
            let mut inp = ProceduralInputs::new(&e, dt);
            if f % 50 == 49 {
                inp.model_to_world = Mat4::from_translation(Vec3::new(f as f32, 0.0, 0.0)); // jumps
            }
            if f % 97 == 96 {
                inp.model_to_world = Mat4::from_cols_array(&[f32::NAN; 16]);
            }
            run(&s, &mut st, &mut pose, &inp);
            for (i, b) in pose.bones.iter().enumerate() {
                assert!(b.rotation.is_finite() && (b.rotation.length() - 1.0).abs() < 1e-3, "dt {dt} frame {f} bone {i}");
            }
            let tip = tail_tip(&sk, &pose);
            assert!(tip.is_finite() && (tip - Vec3::new(0.0, 10.0, 0.0)).length() < 17.0, "dt {dt}: tip {tip}");
            // max_angle 70 from the animated direction per bone: the first bone is easy to check
            assert!(pose.bones[1].rotation.angle_between(Quat::IDENTITY).to_degrees() <= 70.5, "dt {dt} frame {f}");
        }
    }
}

#[test]
fn spring_lags_behind_a_moving_model_and_returns() {
    let sk = tail();
    let s = set(&sk, &spring_json(60.0, 4.0, 0.0, 90.0));
    let mut st = ProceduralState::new(&s);
    let e = ExprInputs::default();
    let mut x = 0.0;
    let mut min_tip_x = 0.0f32;
    for f in 0..600 {
        if f < 90 {
            x += 1.0; // 60 px/s
        }
        let mut pose = Pose::rest(&sk);
        let mut inp = ProceduralInputs::new(&e, 1.0 / 60.0);
        inp.model_to_world = Mat4::from_translation(Vec3::new(x, 0.0, 0.0));
        run(&s, &mut st, &mut pose, &inp);
        min_tip_x = min_tip_x.min(tail_tip(&sk, &pose).x);
    }
    assert!(min_tip_x < -1.0, "the tail trails behind the motion: {min_tip_x}");
    let mut pose = Pose::rest(&sk);
    let mut inp = ProceduralInputs::new(&e, 1.0 / 60.0);
    inp.model_to_world = Mat4::from_translation(Vec3::new(x, 0.0, 0.0));
    run(&s, &mut st, &mut pose, &inp);
    assert!(tail_tip(&sk, &pose).x.abs() < 0.05, "back at rest: {}", tail_tip(&sk, &pose));
}

#[test]
fn spring_restarts_after_deactivation() {
    let sk = tail();
    let s = set(
        &sk,
        r#"[{"type":"spring","bones":["tail1","tail2"],"gravity":300,"tip_length":4,"fade":0,"when":"q.synced('a') == 1"}]"#,
    );
    let mut st = ProceduralState::new(&s);
    let mut e = ExprInputs::default();
    e.synced[0] = 1.0;
    for _ in 0..120 {
        let mut p = Pose::rest(&sk);
        run(&s, &mut st, &mut p, &ProceduralInputs::new(&e, 1.0 / 60.0));
    }
    e.synced[0] = 0.0;
    let mut p = Pose::rest(&sk);
    assert_eq!(run(&s, &mut st, &mut p, &ProceduralInputs::new(&e, 1.0 / 60.0)).inactive, 1);
    assert_eq!(p, Pose::rest(&sk));
    e.synced[0] = 1.0;
    let mut p = Pose::rest(&sk);
    run(&s, &mut st, &mut p, &ProceduralInputs::new(&e, 1.0 / 60.0));
    // starts again from the animated pose: only one frame of sag, tiny
    assert!(qangle(p.bones[1].rotation) < 1.0, "{}", qangle(p.bones[1].rotation));
}

// ---------------------------------------------------------------------------------------------------------------
// follow and counter_rotate
// ---------------------------------------------------------------------------------------------------------------

/// body(0) <- src(1), dst(2)
fn pair() -> Skeleton {
    Skeleton { bones: vec![bone("body", None, [0.0, 0.0, 0.0]), bone("src", Some(0), [4.0, 0.0, 0.0]), bone("dst", Some(0), [-4.0, 0.0, 0.0])] }
}

#[test]
fn follow_copies_with_a_delay_in_ticks() {
    let sk = pair();
    let s = set(&sk, r#"[{"type":"follow","bone":"dst","source":"src","delay_ticks":4,"fade":0}]"#);
    let mut st = ProceduralState::new(&s);
    let e = ExprInputs::default();
    let step = euler_deg_to_quat(Vec3::new(0.0, 0.0, 90.0));
    let mut dst_angle = vec![];
    for f in 0..40 {
        let mut pose = Pose::rest(&sk);
        if f >= 20 {
            pose.bones[1].rotation = step;
        }
        run(&s, &mut st, &mut pose, &ProceduralInputs::new(&e, 0.05));
        dst_angle.push(pose.bones[2].rotation.angle_between(Quat::IDENTITY).to_degrees());
    }
    assert!(dst_angle[..24].iter().all(|a| *a < 1.0), "{dst_angle:?}");
    assert!(dst_angle[24..].iter().all(|a| (*a - 90.0).abs() < 1.0), "{dst_angle:?}");
    // delay 0 copies at once, weight 0.5 halves
    let s0 = set(&sk, r#"[{"type":"follow","bone":"dst","source":"src","fade":0,"weight":0.5}]"#);
    let mut st0 = ProceduralState::new(&s0);
    let mut pose = Pose::rest(&sk);
    pose.bones[1].rotation = step;
    run(&s0, &mut st0, &mut pose, &ProceduralInputs::new(&e, 0.05));
    assert!((pose.bones[2].rotation.angle_between(Quat::IDENTITY).to_degrees() - 45.0).abs() < 0.01);
}

#[test]
fn follow_interpolates_between_frames_of_different_length() {
    let sk = pair();
    let s = set(&sk, r#"[{"type":"follow","bone":"dst","source":"src","delay_ticks":2,"fade":0}]"#);
    let mut st = ProceduralState::new(&s);
    let e = ExprInputs::default();
    // the source rotates linearly in time: 90 deg per second; the follower must trail it by exactly 0.1 s
    let mut t = 0.0f32;
    let dt = 0.013;
    for _ in 0..400 {
        t += dt;
        let mut pose = Pose::rest(&sk);
        pose.bones[1].rotation = euler_deg_to_quat(Vec3::new(0.0, 0.0, 90.0 * t));
        run(&s, &mut st, &mut pose, &ProceduralInputs::new(&e, dt));
        if t > 0.5 && t < 1.9 {
            let a = pose.bones[2].rotation.angle_between(Quat::IDENTITY).to_degrees();
            let want = 90.0 * (t - 0.1);
            assert!((a - want).abs() < 1.0, "t {t}: {a} vs {want}"); // slerp chords and tick quantisation
        }
    }
}

#[test]
fn counter_rotate_keeps_the_bone_upright() {
    let sk = head();
    let s = set(&sk, r#"[{"type":"counter_rotate","bone":"head","axis":"y","up":[0,1,0],"fade":0}]"#);
    let e = ExprInputs::default();
    let mut st = ProceduralState::new(&s);
    let mut pose = Pose::rest(&sk);
    pose.bones[0].rotation = euler_deg_to_quat(Vec3::new(40.0, 25.0, 30.0));
    run(&s, &mut st, &mut pose, &ProceduralInputs::new(&e, 0.05));
    let up = axis_world(&sk, &pose, 1, Vec3::Y);
    assert!(angle_between_deg(up, Vec3::Y) < 0.01, "{up}");
    // weight 0.5: halfway
    let s = set(&sk, r#"[{"type":"counter_rotate","bone":"head","fade":0,"weight":0.5}]"#);
    let mut st = ProceduralState::new(&s);
    let mut pose = Pose::rest(&sk);
    pose.bones[0].rotation = euler_deg_to_quat(Vec3::new(40.0, 0.0, 0.0));
    run(&s, &mut st, &mut pose, &ProceduralInputs::new(&e, 0.05));
    let a = angle_between_deg(axis_world(&sk, &pose, 1, Vec3::Y), Vec3::Y);
    assert!((a - 20.0).abs() < 0.01, "{a}");
    // the bone's own rotation about the up axis survives
    let s = set(&sk, r#"[{"type":"counter_rotate","bone":"head","fade":0}]"#);
    let mut st = ProceduralState::new(&s);
    let mut pose = Pose::rest(&sk);
    pose.bones[0].rotation = euler_deg_to_quat(Vec3::new(0.0, 0.0, 35.0));
    pose.bones[1].rotation = euler_deg_to_quat(Vec3::new(0.0, 50.0, 0.0));
    run(&s, &mut st, &mut pose, &ProceduralInputs::new(&e, 0.05));
    assert!(angle_between_deg(axis_world(&sk, &pose, 1, Vec3::Y), Vec3::Y) < 0.01);
    let fwd = axis_world(&sk, &pose, 1, Vec3::NEG_Z);
    assert!(fwd.y.abs() < 1e-3, "still level: {fwd}");
}

// ---------------------------------------------------------------------------------------------------------------
// JSON
// ---------------------------------------------------------------------------------------------------------------

fn parse_err(json: &str) -> String {
    let sk = tail();
    let mut syms = Symbols::new(&["state"]);
    let loc = [Locator { name: "mark".into(), bone: BoneIndex(0), offset: Vec3::ZERO, rotation: Vec3::ZERO, null_object: false }];
    match parse_procedural_str("fox.json", json, &sk, &loc, &mut syms) {
        Ok(_) => panic!("expected an error for {json}"),
        Err(e) => e.to_string(),
    }
}

#[test]
fn json_errors_name_file_and_path() {
    let cases: &[(&str, &[&str])] = &[
        ("{}", &["fox.json: procedural: expected an array"]),
        ("[1]", &["fox.json: procedural[0]: expected an object"]),
        (r#"[{"type":"wag"}]"#, &["procedural[0].type", "unknown constraint type 'wag'"]),
        (r#"[{"type":"look_at","bone":"nose"}]"#, &["procedural[0].bone", "unknown bone 'nose'", "tail1"]),
        (r#"[{"type":"look_at"}]"#, &["procedural[0]", "missing field 'bone'"]),
        (r#"[{"type":"look_at","bone":"tail1","yaw":400}]"#, &["procedural[0].yaw", "out of range"]),
        (r#"[{"type":"look_at","bone":"tail1","yaw":"x"}]"#, &["procedural[0].yaw", "expected a number"]),
        (r#"[{"type":"look_at","bone":"tail1","turbo":1}]"#, &["procedural[0].turbo", "unknown field"]),
        (r#"[{"type":"look_at","bone":"tail1","target":"moon"}]"#, &["procedural[0].target", "unknown target 'moon'"]),
        (r#"[{"type":"look_at","bone":"tail1","target":{"locator":"nope"}}]"#, &["target.locator", "unknown locator 'nope'", "mark"]),
        (r#"[{"type":"look_at","bone":"tail1","when":"q.synced('mode') == 1"}]"#, &["procedural[0].when", "unknown synced value 'mode'"]),
        (r#"[{"type":"look_at","bone":"tail1","when":"1 +"}]"#, &["procedural[0].when", "expression", "column"]),
        (r#"[{"type":"look_at","bone":"tail1","forward":"z","up":"-z"}]"#, &["procedural[0]", "parallel"]),
        (r#"[{"type":"two_bone_ik","root":"tail1","mid":"tail2","target":"owner"}]"#, &["procedural[0]", "missing field 'tip'"]),
        (r#"[{"type":"two_bone_ik","root":"tail2","mid":"tail1","tip":"tail3","target":"owner"}]"#, &["procedural[0]", "mid must be a descendant of root"]),
        (r#"[{"type":"two_bone_ik","root":"tail1","mid":"tail2","tip":"body","target":"owner"}]"#, &["tip must be mid or a descendant"]),
        (r#"[{"type":"two_bone_ik","root":"tail1","mid":"tail2","tip":"tail3","tip_locator":"mark","target":"owner"}]"#, &["either 'tip'"]),
        (r#"[{"type":"two_bone_ik","root":"tail1","mid":"tail2","tip":"tail3","target":{"input":9}}]"#, &["target.input", "out of range"]),
        (r#"[{"type":"two_bone_ik","root":"tail1","mid":"tail2","tip":"tail3","target":"owner","pole":{"dir":[0,0,0]}}]"#, &["pole.dir", "must not be zero"]),
        (r#"[{"type":"spring","bones":[]}]"#, &["procedural[0].bones", "0 bones"]),
        (r#"[{"type":"spring","bones":["tail2","tail1"]}]"#, &["procedural[0]", "not a descendant"]),
        (r#"[{"type":"spring","bones":["tail1"],"stiffness":1e9}]"#, &["procedural[0].stiffness", "out of range"]),
        (r#"[{"type":"follow","bone":"tail1","source":"tail1"}]"#, &["procedural[0]", "same"]),
        (r#"[{"type":"follow","bone":"tail1","source":"tail2","delay_ticks":2.5}]"#, &["delay_ticks", "whole number"]),
        (r#"[{"type":"follow","bone":"tail1","source":"tail2","delay_ticks":41}]"#, &["delay_ticks", "out of range"]),
        (r#"[{"type":"counter_rotate","bone":"tail1","axis":[0,0,0]}]"#, &["procedural[0].axis", "must not be zero"]),
        (r#"[{"type":"look_at","bone":"tail1","weight":2}]"#, &["procedural[0].weight"]),
    ];
    for (json, parts) in cases {
        let msg = parse_err(json);
        assert!(msg.starts_with("fox.json: "), "{msg}");
        for p in *parts {
            assert!(msg.contains(p), "case {json}\n  message: {msg}\n  missing: {p}");
        }
    }
    let msg = parse_err("[1,");
    assert!(msg.contains("invalid JSON"), "{msg}");
}

#[test]
fn json_limits() {
    let one = r#"{"type":"counter_rotate","bone":"tail1"}"#;
    let many = format!("[{}]", vec![one; 17].join(","));
    let msg = parse_err(&many);
    assert!(msg.contains("17 counter_rotate constraints (at most 16)"), "{msg}");
    let lots = format!("[{}]", vec![one; 65].join(","));
    assert!(parse_err(&lots).contains("65 constraints (at most 64)"));
    let long_chain = format!(
        r#"[{{"type":"spring","bones":[{}]}}]"#,
        vec![r#""tail1""#; 17].join(",")
    );
    assert!(parse_err(&long_chain).contains("17 bones in the chain (1 to 16)"));
    // total spring bones over the cap
    let mut sk = Skeleton { bones: vec![bone("b0", None, [0.0, 0.0, 0.0])] };
    for i in 1..20 {
        sk.bones.push(bone(&format!("b{i}"), Some(i as u16 - 1), [0.0, 0.0, i as f32]));
    }
    let chain: Vec<String> = (0..16).map(|i| format!("\"b{i}\"")).collect();
    let spring = format!(r#"{{"type":"spring","bones":[{}]}}"#, chain.join(","));
    let ok = format!("[{}]", [spring.as_str(); 4].join(","));
    let mut syms = Symbols::new(&[]);
    assert!(parse_procedural_str("t.json", &ok, &sk, &[], &mut syms).is_ok());
    let over = format!("[{}]", [spring.as_str(); 5].join(","));
    let e = parse_procedural_str("t.json", &over, &sk, &[], &mut syms).unwrap_err().to_string();
    assert!(e.contains("80 spring bones in all (at most 64)"), "{e}");
}

#[test]
fn json_full_example_parses() {
    let sk = tail();
    let mut syms = Symbols::new(&["state"]);
    let loc = [Locator { name: "mark".into(), bone: BoneIndex(0), offset: Vec3::new(0.0, 20.0, 0.0), rotation: Vec3::ZERO, null_object: false }];
    let v: serde_json::Value = serde_json::from_str(
        r#"[
        {"type":"look_at","bone":"tail1","target":{"bone":"tail3"},"yaw":30,"pitch":20,"speed":120,"forward":"z","up":"y","offset":[0,1,0],"weight":0.8,"fade":0.3,"comment":"x"},
        {"type":"aim","bone":"tail2","axis":[0,0,1],"target":{"locator":"mark"},"max_angle":45},
        {"type":"two_bone_ik","root":"tail1","mid":"tail2","tip":"tail3","tip_offset":[0,0,4],"target":{"point":[0,5,5]},"pole":{"bone":"body"},"when":"q.synced('state') == 1 && q.is_on_ground"},
        {"type":"spring","bones":["tail1","tail2","tail3"],"tip_dir":[0,0,1]},
        {"type":"follow","bone":"tail3","source":"tail1","delay_ticks":10},
        {"type":"counter_rotate","bone":"tail2","axis":"-z","up":[0,1,0]}
    ]"#,
    )
    .unwrap();
    let s = parse_procedural("fox.json", "model.procedural", &v, &sk, &loc, &mut syms).unwrap();
    assert_eq!(s.constraints().len(), 6);
    let names: Vec<&str> = s.constraints().iter().map(|c| c.kind.name()).collect();
    assert_eq!(names, ["look_at", "aim", "two_bone_ik", "spring", "follow", "counter_rotate"]);
    assert_eq!(s.constraints()[0].weight, 0.8);
    assert!(s.constraints()[2].when.is_some());
    // evaluating all of them on a rest pose does not blow up
    let mut st = ProceduralState::new(&s);
    let e = ExprInputs::default();
    for _ in 0..60 {
        let mut p = Pose::rest(&sk);
        run(&s, &mut st, &mut p, &ProceduralInputs::new(&e, 0.05));
        assert!(p.bones.iter().all(|b| b.rotation.is_finite()));
    }
    let bad_path = parse_procedural("fox.json", "model.procedural", &serde_json::json!([{"type":"aim","bone":"nope"}]), &sk, &[], &mut syms).unwrap_err();
    assert_eq!(bad_path.path, "model.procedural[0].bone");
    assert_eq!(bad_path.file, "fox.json");
}

// ---------------------------------------------------------------------------------------------------------------
// Script bone writes
// ---------------------------------------------------------------------------------------------------------------

#[test]
fn bone_writes_additive_and_absolute() {
    let sk = tail();
    let mut pose = Pose::rest(&sk);
    pose.bones[1].translation = Vec3::new(1.0, 0.0, 0.0);
    pose.bones[1].scale = Vec3::splat(2.0);
    pose.bones[1].rotation = euler_deg_to_quat(Vec3::new(0.0, 0.0, 30.0));
    let mut w = BoneWrite::new(1, WriteMode::Additive);
    w.translation = Some(Vec3::new(0.0, 2.0, 0.0));
    w.rotation = Some(WriteRotation::EulerDegrees(Vec3::new(0.0, 0.0, 15.0)));
    w.scale = Some(Vec3::new(1.0, 0.5, 1.0));
    let rep = apply_bone_writes(&mut pose, &[w]);
    assert_eq!(rep, WriteReport { applied: 1, ..Default::default() });
    assert_eq!(pose.bones[1].translation, Vec3::new(1.0, 2.0, 0.0));
    assert_eq!(pose.bones[1].scale, Vec3::new(2.0, 1.0, 2.0));
    assert!(pose.bones[1].rotation.abs_diff_eq(euler_deg_to_quat(Vec3::new(0.0, 0.0, 45.0)), 1e-5));

    let mut a = BoneWrite::new(2, WriteMode::Absolute);
    a.translation = Some(Vec3::new(5.0, 5.0, 5.0));
    a.rotation = Some(WriteRotation::Quat(Quat::from_xyzw(0.0, 0.0, 2.0, 2.0))); // not unit: normalised
    a.scale = Some(Vec3::splat(3.0));
    apply_bone_writes(&mut pose, &[a]);
    assert_eq!(pose.bones[2].translation, Vec3::splat(5.0));
    assert_eq!(pose.bones[2].scale, Vec3::splat(3.0));
    assert!((pose.bones[2].rotation.length() - 1.0).abs() < 1e-6);
    assert!(pose.bones[2].rotation.abs_diff_eq(euler_deg_to_quat(Vec3::new(0.0, 0.0, 90.0)), 1e-5));

    // unset channels stay
    let mut only_t = BoneWrite::new(2, WriteMode::Absolute);
    only_t.translation = Some(Vec3::ZERO);
    apply_bone_writes(&mut pose, &[only_t]);
    assert_eq!(pose.bones[2].scale, Vec3::splat(3.0));
}

#[test]
fn bone_writes_validate() {
    let sk = tail();
    let mut pose = Pose::rest(&sk);
    let before = pose.clone();
    let mut unknown = BoneWrite::new(99, WriteMode::Absolute);
    unknown.translation = Some(Vec3::ONE);
    let mut nan_t = BoneWrite::new(1, WriteMode::Additive);
    nan_t.translation = Some(Vec3::new(f32::NAN, 0.0, 0.0));
    let mut inf_s = BoneWrite::new(1, WriteMode::Absolute);
    inf_s.scale = Some(Vec3::new(1.0, f32::INFINITY, 1.0));
    let mut zero_q = BoneWrite::new(1, WriteMode::Absolute);
    zero_q.rotation = Some(WriteRotation::Quat(Quat::from_xyzw(0.0, 0.0, 0.0, 0.0)));
    let mut nan_e = BoneWrite::new(1, WriteMode::Absolute);
    nan_e.rotation = Some(WriteRotation::EulerDegrees(Vec3::new(0.0, f32::NAN, 0.0)));
    let mut huge = BoneWrite::new(1, WriteMode::Absolute);
    huge.translation = Some(Vec3::splat(1.0e9));
    // one write mixing a valid and an invalid channel is rejected whole
    let mut mixed = BoneWrite::new(3, WriteMode::Absolute);
    mixed.translation = Some(Vec3::ONE);
    mixed.scale = Some(Vec3::new(f32::NAN, 1.0, 1.0));
    let rep = apply_bone_writes(&mut pose, &[unknown, nan_t, inf_s, zero_q, nan_e, huge, mixed]);
    assert_eq!(rep.ignored_unknown, 1);
    assert_eq!(rep.rejected_invalid, 6);
    assert_eq!(rep.applied, 0);
    assert_eq!(pose, before);
    // limit
    let ok = {
        let mut w = BoneWrite::new(1, WriteMode::Additive);
        w.translation = Some(Vec3::X);
        w
    };
    let many = vec![ok; limits::MAX_BONE_WRITES + 10];
    let rep = apply_bone_writes(&mut pose, &many);
    assert_eq!(rep.applied as usize, limits::MAX_BONE_WRITES);
    assert_eq!(rep.dropped_over_limit, 10);
    assert_eq!(pose.bones[1].translation.x, limits::MAX_BONE_WRITES as f32);
}

#[test]
fn bone_writes_visibility() {
    let sk = tail();
    let mut pose = Pose::rest(&sk);
    let mut vis = BoneVisibility::new(sk.len());
    let mut hide = BoneWrite::new(2, WriteMode::Additive);
    hide.visible = Some(false);
    let rep = apply_bone_writes_with_visibility(&mut pose, &mut vis, &[hide]);
    assert_eq!(rep.applied, 1);
    assert!(!vis.is_visible(2) && vis.is_visible(1));
    let mut show = hide;
    show.visible = Some(true);
    apply_bone_writes_with_visibility(&mut pose, &mut vis, &[show]);
    assert!(vis.is_visible(2));
    // without a visibility sink the value is counted, not lost silently
    let rep = apply_bone_writes(&mut pose, &[hide]);
    assert_eq!(rep.visibility_ignored, 1);
}

#[test]
fn script_writes_apply_after_constraints() {
    let sk = head();
    let s = set(&sk, r#"[{"type":"look_at","bone":"head","target":"view","yaw":90,"pitch":90,"speed":0,"fade":0}]"#);
    let mut st = ProceduralState::new(&s);
    let e = ExprInputs::default();
    let mut pose = Pose::rest(&sk);
    run(&s, &mut st, &mut pose, &ProceduralInputs::new(&e, 0.05).with_target(TargetSource::View, Vec3::new(100.0, 16.0, 0.0)));
    let mut w = BoneWrite::new(1, WriteMode::Absolute);
    w.rotation = Some(WriteRotation::EulerDegrees(Vec3::ZERO));
    apply_bone_writes(&mut pose, &[w]);
    assert!(pose.bones[1].rotation.abs_diff_eq(Quat::IDENTITY, 1e-6), "the script has the last word");
}

// ---------------------------------------------------------------------------------------------------------------
// The Cinder Fox with its real fixture files
// ---------------------------------------------------------------------------------------------------------------

const BB: &[u8] = include_bytes!("fixtures/cinder_fox/cinder_fox.bbmodel");
const GEO: &[u8] = include_bytes!("fixtures/cinder_fox/cinder_fox.geo.json");
const ANIM: &str = include_str!("fixtures/cinder_fox/cinder_fox.animation.json");
const CTRL: &str = include_str!("fixtures/cinder_fox/cinder_fox.json");
const TYPE_JSON: &str = include_str!("fixtures/procedural/cinder_fox_type.json");

fn fox_rig(model: &horizon_mobmodel::Model) -> Rig {
    let mut syms = Symbols::new(&["state"]);
    let parsed = parse_animation_file(ANIM, &mut syms).expect("animations");
    let mut lib = AnimationLibrary::new();
    for a in parsed.animations {
        lib.add(a).unwrap();
    }
    let ctrl = parse_controllers(CTRL, &mut syms).expect("controller");
    Rig::new(model.skeleton.clone(), lib, ctrl.controllers).expect("rig").0
}

fn fox_set(model: &horizon_mobmodel::Model) -> ProceduralSet {
    let ty: serde_json::Value = serde_json::from_str(TYPE_JSON).unwrap();
    let mut syms = Symbols::new(&["state"]);
    parse_procedural("cinder_fox_type.json", "model.procedural", &ty["model"]["procedural"], &model.skeleton, &model.locators, &mut syms)
        .unwrap_or_else(|e| panic!("{e}"))
}

#[test]
fn cinder_fox_look_at_head_and_front_leg_ik() {
    for (route, model) in [
        ("geo", load_geo_json("cinder_fox.geo.json", GEO).expect("geo")),
        ("bbmodel", load_bbmodel("cinder_fox.bbmodel", BB).expect("bbmodel")),
    ] {
        let sk = &model.skeleton;
        let rig = fox_rig(&model);
        let set = fox_set(&model);
        assert_eq!(set.constraints().len(), 4);
        let idx = |n: &str| sk.find(n).unwrap().index();
        let (head, body, leg_fr) = (idx("head"), idx("body"), idx("leg_fr"));

        // --- standing, idle animation: the head follows a viewer to the right and above the fox
        let mut inputs = ExprInputs::default();
        let mut rs = rig.initial_state();
        rig.update(&mut rs, 0.05, &mut inputs, &mut |_| {});
        let mut animated = Pose::default();
        rig.evaluate(&rs, &[], &mut inputs, &mut Scratch::default(), None, &mut animated);
        let mut pst = ProceduralState::new(&set);
        let eye0 = joint(sk, &animated, head);
        let viewer = eye0 + Vec3::new(40.0, 25.0, -60.0);
        let mut pose = animated.clone();
        let rep = set.evaluate(&mut pose, &mut pst, &ProceduralInputs::new(&inputs, 0.05).with_target(TargetSource::View, viewer));
        assert_eq!((rep.applied, rep.inactive), (2, 2), "look_at and spring run, both IKs wait for the sit state");
        assert_eq!(pose.bones[idx("leg_fl")], animated.bones[idx("leg_fl")]);
        // converge (speed 360 deg/s)
        for _ in 0..10 {
            let mut p = animated.clone();
            set.evaluate(&mut p, &mut pst, &ProceduralInputs::new(&inputs, 0.05).with_target(TargetSource::View, viewer));
            pose = p;
        }
        let m = bone_matrices(sk, &pose);
        let fwd = m[head].transform_vector3(Vec3::NEG_Z).normalize();
        let m0 = bone_matrices(sk, &animated);
        let rest_fwd = m0[head].transform_vector3(Vec3::NEG_Z).normalize();
        let eye = joint(sk, &pose, head);
        let want = (viewer - eye).normalize();
        let turned = angle_between_deg(fwd, rest_fwd);
        assert!(turned > 20.0, "the head turned toward the viewer: {turned}");
        assert!(angle_between_deg(fwd, want) < angle_between_deg(rest_fwd, want), "closer to the viewer than before");
        // the limits hold: yaw 60 and pitch 40 relative to the animated head orientation
        let q0 = Quat::from_mat4(&m0[head]);
        let rel = q0.inverse() * (fwd * 1.0);
        let yaw = rel.x.atan2(-rel.z).to_degrees();
        let pitch = rel.y.atan2((rel.x * rel.x + rel.z * rel.z).sqrt()).to_degrees();
        assert!(yaw.abs() <= 60.5 && pitch.abs() <= 40.5, "yaw {yaw} pitch {pitch}");
        // the ears (children of the head) came along; the springs left the tail finite
        assert!(pose.bones.iter().all(|b| b.rotation.is_finite()));
        // tail bones sag: with gravity the tail tip is lower than animated
        let tip = |p: &Pose| bone_matrices(sk, p)[idx("tail3")].transform_point3(sk.bones[idx("tail3")].pivot + Vec3::new(0.0, 0.0, 6.0));
        let mut sag = animated.clone();
        let mut pst2 = ProceduralState::new(&set);
        for _ in 0..200 {
            sag = animated.clone();
            set.evaluate(&mut sag, &mut pst2, &ProceduralInputs::new(&inputs, 0.05));
        }
        assert!(tip(&sag).y < tip(&animated).y - 0.2, "tail sags {} vs {}", tip(&sag).y, tip(&animated).y);

        // --- sitting: synced state 1 activates the front-leg IK
        inputs.synced[0] = 1.0;
        let mut sit_state = rig.initial_state();
        rig.update(&mut sit_state, 0.05, &mut inputs, &mut |_| {});
        for _ in 0..40 {
            rig.update(&mut sit_state, 0.05, &mut inputs, &mut |_| {});
        }
        let mut sitting = Pose::default();
        rig.evaluate(&sit_state, &[], &mut inputs, &mut Scratch::default(), None, &mut sitting);
        let a = joint(sk, &sitting, body);
        // a reachable paw target for each leg, 6 px from the body joint (limb reach is 2.1 .. 8.1)
        let t_fr = a + Vec3::new(2.0, -3.0, -5.0).normalize() * 6.0;
        let t_fl = a + Vec3::new(-2.0, -3.0, -5.0).normalize() * 6.0;
        let mut pst3 = ProceduralState::new(&set);
        let mut pose = sitting.clone();
        let inp = ProceduralInputs::new(&inputs, 0.05)
            .with_target(TargetSource::Custom(0), t_fl)
            .with_target(TargetSource::Custom(1), t_fr)
            .with_target(TargetSource::View, eye0 + Vec3::new(0.0, 0.0, -50.0));
        let rep = set.evaluate(&mut pose, &mut pst3, &inp);
        assert_eq!(rep.applied, 4, "everything is active while sitting: {rep:?}");
        // both legs share `body` as the IK root, so the second solve moves the first leg again: the last one is exact
        let paw_fr = bone_matrices(sk, &pose)[leg_fr].transform_point3(sk.bones[leg_fr].pivot + Vec3::new(0.0, -3.0, 0.0));
        assert!((paw_fr - t_fr).length() < 1e-3, "{route}: front-right paw {paw_fr} target {t_fr}; rep {rep:?}");
        // Bone lengths: the sitting fox's body is squashed a little (non-uniform scale on `body`), which makes the limb
        // lengths depend slightly on direction, so compare with the lengths before the IK, within 0.15 px.
        let j = bone_matrices(sk, &pose);
        let j0 = bone_matrices(sk, &sitting);
        let end = |m: &[Mat4]| {
            let a = m[body].transform_point3(sk.bones[body].pivot);
            let b = m[leg_fr].transform_point3(sk.bones[leg_fr].pivot);
            let c = m[leg_fr].transform_point3(sk.bones[leg_fr].pivot + Vec3::new(0.0, -3.0, 0.0));
            (a, b, c)
        };
        let (a2, b2, _) = end(&j);
        let (a0, b0, c0) = end(&j0);
        assert!(((b2 - a2).length() - (b0 - a0).length()).abs() < 0.15);
        assert!(((paw_fr - b2).length() - (c0 - b0).length()).abs() < 0.15);
        let line = (paw_fr - a2).normalize();
        let bend = (b2 - a2) - line * (b2 - a2).dot(line);
        assert!(bend.dot(Vec3::NEG_Z) > 0.0, "bend {bend}");
    }
}
