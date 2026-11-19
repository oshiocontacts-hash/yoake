//! The "Cinder Fox" fixture (FR-F4-MOBS.md section 10, scenario 2): both model formats load to the same skeleton and
//! vertices, 30-frame golden pose tables for every animation, the geo.json route and the bbmodel route agree, the
//! controller (locomotion, pounce trigger with its `hit` event, sit), and the AnimClock anchors on a simulated client.
//!
//! The golden pose table `tests/fixtures/cinder_fox/golden_poses.txt` was FIRST PRODUCED BY THIS CODE (`MOBMODEL_UPDATE_GOLDENS=1
//! cargo test -p horizon-mobmodel --test cinder_fox golden`). It pins today's behaviour; it was not checked against Blockbench
//! or GeckoLib. Regenerate it with the same command when a change of the pose maths is intended, and review the diff.

use glam::{Mat4, Vec3};
use horizon_mobmodel::anim::{animations_from_bbmodel, parse_animation_file, AnimationLibrary, Channel, Easing, EventKind, LoopMode};
use horizon_mobmodel::clock::AnimClock;
use horizon_mobmodel::controller::parse_controllers;
use horizon_mobmodel::expr::{ExprInputs, Query, Symbols};
use horizon_mobmodel::pose::{Rig, RigState, Scratch, TriggerPlay};
use horizon_mobmodel::skeleton::{bone_matrices, Pose};
use horizon_mobmodel::{load_bbmodel, load_geo_json, Model};
use std::fmt::Write as _;

const BB: &[u8] = include_bytes!("fixtures/cinder_fox/cinder_fox.bbmodel");
const GEO: &[u8] = include_bytes!("fixtures/cinder_fox/cinder_fox.geo.json");
const ANIM: &str = include_str!("fixtures/cinder_fox/cinder_fox.animation.json");
const CTRL: &str = include_str!("fixtures/cinder_fox/cinder_fox.json");
const TEXTURE: &[u8] = include_bytes!("fixtures/cinder_fox/cinder_fox.png");
const GLOW: &[u8] = include_bytes!("fixtures/cinder_fox/cinder_fox_glowmask.png");
const GOLDEN: &str = "tests/fixtures/cinder_fox/golden_poses.txt";

const ANIMATIONS: [&str; 6] = ["idle", "walk", "run", "sit", "pounce", "death"];
const FRAMES: usize = 30;
const TOL: f32 = 1e-4;
const TICK: f32 = 0.05;

#[derive(Clone, Copy, PartialEq)]
enum Route {
    Geo,
    Bb,
}

struct Fox {
    model: Model,
    rig: Rig,
    warnings: Vec<String>,
}

fn fox(route: Route) -> Fox {
    let model = match route {
        Route::Geo => load_geo_json("cinder_fox.geo.json", GEO).expect("geo loads"),
        Route::Bb => load_bbmodel("cinder_fox.bbmodel", BB).expect("bbmodel loads"),
    };
    let mut warnings: Vec<String> = model.warnings.clone();
    let mut syms = Symbols::new(&["state"]);
    let parsed = match route {
        Route::Geo => parse_animation_file(ANIM, &mut syms).expect("animation.json loads"),
        Route::Bb => animations_from_bbmodel(model.animations.as_ref().expect("bbmodel has animations"), &mut syms)
            .expect("bbmodel animations load"),
    };
    warnings.extend(parsed.warnings.iter().map(|w| w.to_string()));
    let mut lib = AnimationLibrary::new();
    for a in parsed.animations {
        lib.add(a).unwrap();
    }
    let ctrl = parse_controllers(CTRL, &mut syms).expect("controller loads");
    warnings.extend(ctrl.warnings.iter().map(|w| w.to_string()));
    let (rig, rig_warnings) = Rig::new(model.skeleton.clone(), lib, ctrl.controllers).expect("rig binds");
    warnings.extend(rig_warnings.iter().map(|w| w.to_string()));
    Fox { model, rig, warnings }
}

fn ground_speed_for(anim: &str) -> f32 {
    match anim {
        "walk" => 0.12,
        "run" => 0.3,
        _ => 0.0,
    }
}

/// Bone matrices of every bone at the 30 golden times `length * i / 30` of animation `name` (ground speed per animation
/// as `ground_speed_for`).
fn pose_table(rig: &Rig, name: &str) -> Vec<Vec<Mat4>> {
    let idx = rig.find_animation(name).unwrap_or_else(|| panic!("animation {name}"));
    let length = rig.animation(idx).unwrap().length;
    let mut inp = ExprInputs::default();
    inp.set(Query::GroundSpeed, ground_speed_for(name));
    (0..FRAMES)
        .map(|i| {
            let mut pose = Pose::default();
            rig.sample_animation(idx, length * i as f32 / FRAMES as f32, &mut inp, &mut pose);
            bone_matrices(&rig.skeleton, &pose)
        })
        .collect()
}

fn assert_affine(m: &Mat4, what: &str) {
    let a = m.to_cols_array();
    assert!(a[3].abs() < 1e-5 && a[7].abs() < 1e-5 && a[11].abs() < 1e-5 && (a[15] - 1.0).abs() < 1e-5, "{what}: not affine");
}

fn golden_text(rig: &Rig) -> String {
    let mut s = String::new();
    s.push_str("# Cinder Fox golden pose tables: bone matrices (model space, pixels) at t = length * frame / 30.\n");
    s.push_str("# Produced by horizon-mobmodel itself (geo.json route); they pin behaviour and were not checked against Blockbench.\n");
    s.push_str("# Regenerate: MOBMODEL_UPDATE_GOLDENS=1 cargo test -p horizon-mobmodel --test cinder_fox golden\n");
    s.push_str("# line: <animation> <frame> <bone> <12 numbers: column 0, 1, 2 (x y z each) and the translation column>\n");
    for name in ANIMATIONS {
        let idx = rig.find_animation(name).unwrap();
        let a = rig.animation(idx).unwrap();
        let _ = writeln!(s, "# {name}: length {} loop {:?} ground_speed {}", a.length, a.loop_mode, ground_speed_for(name));
        for (f, mats) in pose_table(rig, name).iter().enumerate() {
            for (b, m) in mats.iter().enumerate() {
                assert_affine(m, &format!("{name} frame {f}"));
                let c = m.to_cols_array();
                let _ = write!(s, "{name} {f:02} {}", rig.skeleton.bones[b].name);
                for k in [0, 1, 2, 4, 5, 6, 8, 9, 10, 12, 13, 14] {
                    let v = format!("{:.5}", c[k]);
                    let v = if v == "-0.00000" { "0.00000".to_string() } else { v };
                    let _ = write!(s, " {v}");
                }
                s.push('\n');
            }
        }
    }
    s
}

fn data_lines(text: &str) -> Vec<&str> {
    text.lines().filter(|l| !l.is_empty() && !l.starts_with('#')).collect()
}

// ---------------------------------------------------------------------------------------------------------------
// (a) both formats load and agree
// ---------------------------------------------------------------------------------------------------------------

#[test]
fn both_formats_load_with_the_same_skeleton_rest_matrices_and_vertices() {
    let (g, b) = (fox(Route::Geo).model, fox(Route::Bb).model);
    assert_eq!(g.identifier, "geometry.cinder_fox");
    assert_eq!(g.identifier, b.identifier);
    assert_eq!(g.texture_size, [64, 64]);
    assert_eq!(b.texture_size, [64, 64]);
    assert_eq!(g.skeleton.len(), 14);
    let names: Vec<&str> = g.skeleton.bones.iter().map(|b| b.name.as_str()).collect();
    assert_eq!(
        names,
        ["root", "body", "head", "ear_l", "ear_r", "snout", "jaw", "leg_fl", "leg_fr", "leg_bl", "leg_br", "tail1", "tail2", "tail3"]
    );
    assert_eq!(g.skeleton, b.skeleton, "same skeleton (names, parents, pivots, rest rotations)");
    assert_eq!(g.element_count(), 14);
    assert_eq!(b.element_count(), 14);

    let rest_g = bone_matrices(&g.skeleton, &Pose::rest(&g.skeleton));
    let rest_b = bone_matrices(&b.skeleton, &Pose::rest(&b.skeleton));
    for (i, (x, y)) in rest_g.iter().zip(&rest_b).enumerate() {
        assert!(x.abs_diff_eq(*y, TOL), "rest matrix of {}", g.skeleton.bones[i].name);
    }
    let tail1 = g.skeleton.find("tail1").unwrap().index();
    assert!(!rest_g[tail1].abs_diff_eq(Mat4::IDENTITY, 1e-3), "a rotated bone really is rotated");
    assert!(g.skeleton.bones.iter().filter(|b| b.rotation != Vec3::ZERO).count() >= 5);

    let (bg, bb) = (g.bake(), b.bake());
    assert_eq!(bg.bones.len(), bb.bones.len());
    assert!(bg.vertex_count() >= 14 * 24, "{}", bg.vertex_count());
    for (i, (x, y)) in bg.bones.iter().zip(&bb.bones).enumerate() {
        let name = &g.skeleton.bones[i].name;
        assert_eq!(x.indices, y.indices, "indices of {name}");
        assert_eq!(x.vertices.len(), y.vertices.len(), "vertex count of {name}");
        for (j, (p, q)) in x.vertices.iter().zip(&y.vertices).enumerate() {
            for k in 0..3 {
                assert!((p.position[k] - q.position[k]).abs() < TOL, "{name} vertex {j} position");
                assert!((p.normal[k] - q.normal[k]).abs() < TOL, "{name} vertex {j} normal");
            }
            for k in 0..2 {
                assert!((p.uv[k] - q.uv[k]).abs() < TOL, "{name} vertex {j} uv");
            }
        }
    }

    // the locator `mouth`
    let loc = |m: &Model| m.locators.iter().map(|l| (l.name.clone(), l.bone, l.offset)).collect::<Vec<_>>();
    assert_eq!(loc(&g).len(), 1);
    assert_eq!(loc(&g), loc(&b));
    assert_eq!(g.locators[0].name, "mouth");
    assert_eq!(g.skeleton.bones[g.locators[0].bone.index()].name, "head");
}

#[test]
fn textures_are_64x64_and_the_embedded_one_is_the_file() {
    let decode = |bytes: &[u8]| {
        let mut r = png::Decoder::new(std::io::Cursor::new(bytes)).read_info().unwrap();
        let mut buf = vec![0u8; r.output_buffer_size().unwrap()];
        let info = r.next_frame(&mut buf).unwrap();
        assert_eq!(info.color_type, png::ColorType::Rgba);
        ((info.width, info.height), buf[..info.buffer_size()].to_vec())
    };
    let ((w, h), file) = decode(TEXTURE);
    assert_eq!((w, h), (64, 64));
    let b = load_bbmodel("cinder_fox.bbmodel", BB).unwrap();
    assert_eq!(b.textures.len(), 1);
    assert_eq!((b.textures[0].width, b.textures[0].height), (64, 64));
    assert_eq!(b.textures[0].rgba, file, "the embedded texture is cinder_fox.png");

    let ((gw, gh), glow) = decode(GLOW);
    assert_eq!((gw, gh), (64, 64));
    let lit = glow.chunks_exact(4).filter(|p| p[3] != 0).count();
    // two eyes (1 texel each) and the six 3x3 faces of the tail tip
    assert_eq!(lit, 2 + 6 * 9, "emissive texels: eyes and tail tip");
    // every emissive texel is opaque in the colour texture
    for (c, g) in file.chunks_exact(4).zip(glow.chunks_exact(4)) {
        if g[3] != 0 {
            assert_eq!(c[3], 255);
        }
    }
    assert!(file.chunks_exact(4).filter(|p| p[3] != 0).count() > 600, "the texture is painted");
}

#[test]
fn everything_loads_without_warnings() {
    for route in [Route::Geo, Route::Bb] {
        let f = fox(route);
        assert!(f.warnings.is_empty(), "{:?}", f.warnings);
        assert_eq!(f.rig.controllers().len(), 3);
        for a in ANIMATIONS {
            assert!(f.rig.find_animation(a).is_some(), "{a}");
        }
    }
}

// ---------------------------------------------------------------------------------------------------------------
// (b) golden pose tables
// ---------------------------------------------------------------------------------------------------------------

#[test]
fn golden_pose_tables_30_frames_per_animation() {
    let text = golden_text(&fox(Route::Geo).rig);
    let path = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join(GOLDEN);
    if std::env::var("MOBMODEL_UPDATE_GOLDENS").is_ok_and(|v| v == "1") {
        std::fs::write(&path, &text).unwrap();
        eprintln!("wrote {}", path.display());
        return;
    }
    let want = std::fs::read_to_string(&path)
        .unwrap_or_else(|e| panic!("{}: {e} (create it with MOBMODEL_UPDATE_GOLDENS=1)", path.display()));
    let (got, want) = (data_lines(&text), data_lines(&want));
    assert_eq!(want.len(), ANIMATIONS.len() * FRAMES * 14, "golden file size");
    assert_eq!(got.len(), want.len());
    for (g, w) in got.iter().zip(&want) {
        let (gt, wt): (Vec<&str>, Vec<&str>) = (g.split_whitespace().collect(), w.split_whitespace().collect());
        assert_eq!(gt.len(), 15, "{w}");
        assert_eq!(gt[..3], wt[..3], "line key");
        for k in 3..15 {
            let (a, b): (f32, f32) = (gt[k].parse().unwrap(), wt[k].parse().unwrap());
            assert!((a - b).abs() <= TOL, "{} {} {} column {}: {a} vs golden {b}", wt[0], wt[1], wt[2], k - 3);
        }
    }
}

#[test]
fn golden_covers_real_motion() {
    // Guards against a golden of rest poses: every animation moves some bone, the frames are not all equal.
    let f = fox(Route::Geo);
    let rest = bone_matrices(&f.rig.skeleton, &Pose::rest(&f.rig.skeleton));
    for name in ANIMATIONS {
        let t = pose_table(&f.rig, name);
        let moved = t.iter().any(|fr| fr.iter().zip(&rest).any(|(a, b)| !a.abs_diff_eq(*b, 1e-2)));
        let varies = t.iter().any(|fr| fr.iter().zip(&t[0]).any(|(a, b)| !a.abs_diff_eq(*b, 1e-2)));
        assert!(moved && varies, "{name}");
    }
}

#[test]
fn animation_properties_match_the_spec() {
    let f = fox(Route::Geo);
    let get = |n: &str| f.rig.animation(f.rig.find_animation(n).unwrap()).unwrap();
    assert_eq!(get("idle").loop_mode, LoopMode::Loop);
    assert_eq!(get("walk").loop_mode, LoopMode::Loop);
    assert_eq!(get("run").loop_mode, LoopMode::Loop);
    assert_eq!(get("sit").loop_mode, LoopMode::Hold);
    assert_eq!(get("pounce").loop_mode, LoopMode::Once);
    assert_eq!(get("death").loop_mode, LoopMode::Hold);
    assert_eq!(get("pounce").length, 0.8);
    assert!(get("walk").anim_time_update.is_some(), "walk speed comes from an expression");
    // One deterministic order: by time, then kind (timeline, sound, particle).
    let ev: Vec<_> = get("pounce").events.iter().map(|e| (e.time, e.kind, e.name.as_str(), e.locator.as_deref())).collect();
    assert_eq!(
        ev,
        [
            (0.35, EventKind::Timeline, "hit", None),
            (0.35, EventKind::Sound, "cinder_fox_pounce", None),
            (0.35, EventKind::Particle, "cinder_fox_embers", Some("mouth")),
        ]
    );
    // several interpolation modes and easings with an argument are really present
    let (mut lerps, mut eased, mut with_args) = (std::collections::HashSet::new(), 0, 0);
    for i in 0..ANIMATIONS.len() {
        let a = f.rig.animation(i).unwrap();
        for b in &a.bones {
            for ch in [&b.rotation, &b.position, &b.scale].into_iter().flatten() {
                if let Channel::Keys(keys) = ch {
                    for k in keys {
                        lerps.insert(format!("{:?}", k.lerp));
                        eased += usize::from(k.easing != Easing::Linear);
                        with_args += usize::from(k.easing_args.is_some());
                    }
                }
            }
        }
    }
    assert_eq!(lerps.len(), 4, "{lerps:?}");
    assert!(eased >= 8 && with_args >= 3, "{eased} {with_args}");
}

#[test]
fn hold_animations_hold_and_the_one_shot_ends() {
    let f = fox(Route::Geo);
    let at = |name: &str, t: f32| {
        let idx = f.rig.find_animation(name).unwrap();
        let mut pose = Pose::default();
        f.rig.sample_animation(idx, t, &mut ExprInputs::default(), &mut pose);
        bone_matrices(&f.rig.skeleton, &pose)
    };
    for (name, len) in [("sit", 0.6), ("death", 1.2)] {
        let (a, b) = (at(name, len), at(name, len + 7.0));
        assert!(a.iter().zip(&b).all(|(x, y)| x.abs_diff_eq(*y, 1e-5)), "{name} holds its last frame");
    }
    let rest = bone_matrices(&f.rig.skeleton, &Pose::rest(&f.rig.skeleton));
    assert!(at("pounce", 0.8).iter().zip(&rest).all(|(x, y)| x.abs_diff_eq(*y, 1e-5)), "pounce is over after 0.8 s");
    assert!(!at("pounce", 0.79).iter().zip(&rest).all(|(x, y)| x.abs_diff_eq(*y, 1e-5)));
    // walk: the clock speed comes from the ground speed expression
    let gait = |gs: f32| {
        let idx = f.rig.find_animation("walk").unwrap();
        let mut inp = ExprInputs::default();
        inp.set(Query::GroundSpeed, gs);
        let mut pose = Pose::default();
        f.rig.sample_animation(idx, 0.3, &mut inp, &mut pose);
        bone_matrices(&f.rig.skeleton, &pose)
    };
    let (slow, fast) = (gait(0.0), gait(0.4));
    assert!(slow.iter().zip(&fast).any(|(x, y)| !x.abs_diff_eq(*y, 1e-2)), "ground speed changes the walk");
}

// ---------------------------------------------------------------------------------------------------------------
// (c) geo route == bbmodel route
// ---------------------------------------------------------------------------------------------------------------

#[test]
fn geo_route_and_bbmodel_route_give_the_same_pose_tables() {
    let (g, b) = (fox(Route::Geo), fox(Route::Bb));
    let mut compared = 0;
    for name in ANIMATIONS {
        let (tg, tb) = (pose_table(&g.rig, name), pose_table(&b.rig, name));
        for (f, (fg, fb)) in tg.iter().zip(&tb).enumerate() {
            for (i, (x, y)) in fg.iter().zip(fb).enumerate() {
                compared += 1;
                assert!(x.abs_diff_eq(*y, TOL), "{name} frame {f} bone {}:
{x:?}
{y:?}", g.rig.skeleton.bones[i].name);
            }
        }
        // events (same order, no sorting), length, loop mode
        let (ag, ab) = (
            g.rig.animation(g.rig.find_animation(name).unwrap()).unwrap(),
            b.rig.animation(b.rig.find_animation(name).unwrap()).unwrap(),
        );
        assert_eq!(ag.events, ab.events, "{name} events");
        assert_eq!((ag.length, ag.loop_mode), (ab.length, ab.loop_mode), "{name} length and loop");
        assert_eq!(ag.anim_time_update.is_some(), ab.anim_time_update.is_some());
    }
    assert_eq!(compared, ANIMATIONS.len() * FRAMES * 14);
}

#[test]
fn the_bbmodel_route_really_carries_the_easing() {
    // Equality above would also hold if neither route had easing: count the eased keys (with arguments) on both.
    let count = |rig: &Rig| {
        let (mut eased, mut args) = (0, 0);
        for i in 0..ANIMATIONS.len() {
            for bone in &rig.animation(i).unwrap().bones {
                for ch in [&bone.rotation, &bone.position, &bone.scale].into_iter().flatten() {
                    if let Channel::Keys(keys) = ch {
                        for k in keys {
                            eased += usize::from(k.easing != Easing::Linear);
                            args += usize::from(k.easing_args.is_some());
                        }
                    }
                }
            }
        }
        (eased, args)
    };
    let (g, b) = (count(&fox(Route::Geo).rig), count(&fox(Route::Bb).rig));
    assert_eq!(g, b);
    assert!(g.0 >= 12 && g.1 >= 5, "{g:?}");
}

// ---------------------------------------------------------------------------------------------------------------
// (d) controller
// ---------------------------------------------------------------------------------------------------------------

fn layer(rig: &Rig, name: &str) -> usize {
    rig.controllers().iter().position(|c| c.name == name).unwrap_or_else(|| panic!("layer {name}"))
}

fn state_name<'a>(rig: &'a Rig, st: &RigState, layer: usize) -> &'a str {
    &rig.controllers()[layer].states[st.layers[layer].cur.state].name
}

fn eval(rig: &Rig, st: &RigState, trig: &[TriggerPlay], inp: &mut ExprInputs) -> Pose {
    let mut out = Pose::default();
    rig.evaluate(st, trig, inp, &mut Scratch::default(), None, &mut out);
    out
}

#[test]
fn locomotion_goes_idle_walk_run_with_ground_speed_and_back() {
    let rig = fox(Route::Geo).rig;
    let mut st = rig.initial_state();
    let mut inp = ExprInputs::default();
    let loco = layer(&rig, "locomotion");
    let mut seen: Vec<String> = vec![state_name(&rig, &st, loco).to_string()];
    let plan = [
        (0.0, "idle"),
        (0.04, "idle"), // inside the hysteresis band: stays
        (0.10, "walk"),
        (0.19, "walk"), // below the run threshold 0.2
        (0.30, "run"),
        (0.18, "run"), // above 0.17: stays running
        (0.10, "walk"),
        (0.04, "walk"), // above 0.03: stays walking
        (0.0, "idle"),
    ];
    for (gs, want) in plan {
        inp.set(Query::GroundSpeed, gs);
        for _ in 0..10 {
            rig.update(&mut st, TICK, &mut inp, &mut |_| {});
            let n = state_name(&rig, &st, loco);
            if seen.last().map(String::as_str) != Some(n) {
                seen.push(n.to_string());
            }
        }
        assert_eq!(state_name(&rig, &st, loco), want, "ground speed {gs}");
    }
    assert_eq!(seen, ["idle", "walk", "run", "walk", "idle"]);

    // the run animation's clock speed is the expression clamp(gs * 3, 0.8, 2)
    let c = &rig.controllers()[loco];
    let mut st = rig.initial_state();
    inp.set(Query::GroundSpeed, 0.3);
    for _ in 0..5 {
        rig.update(&mut st, TICK, &mut inp, &mut |_| {});
    }
    assert_eq!(c.states[st.layers[loco].cur.state].name, "run");
    assert!((c.slot_speed(&st.layers[loco].cur, 0, &mut inp) - 0.9).abs() < 1e-5);
    inp.set(Query::GroundSpeed, 0.9);
    assert_eq!(c.slot_speed(&st.layers[loco].cur, 0, &mut inp), 2.0);
    inp.set(Query::GroundSpeed, 0.18);
    assert_eq!(c.slot_speed(&st.layers[loco].cur, 0, &mut inp), 0.8);

    // the cross-fade is a real blend: halfway through idle -> walk the pose is neither
    let mut st = rig.initial_state();
    let mut inp = ExprInputs::default();
    rig.update(&mut st, 0.5, &mut inp, &mut |_| {});
    let idle_pose = eval(&rig, &st, &[], &mut inp);
    inp.set(Query::GroundSpeed, 0.1);
    rig.update(&mut st, 0.0, &mut inp, &mut |_| {});
    assert_eq!(state_name(&rig, &st, loco), "walk");
    rig.update(&mut st, 0.05, &mut inp, &mut |_| {}); // half of the 0.1 s fade
    let mid = eval(&rig, &st, &[], &mut inp);
    assert!(mid.bones.iter().zip(&idle_pose.bones).any(|(a, b)| a != b));
}

#[test]
fn pounce_trigger_fires_hit_exactly_once_at_0_35_s() {
    let rig = fox(Route::Geo).rig;
    let action = layer(&rig, "action");
    let trig = rig.controllers()[action].triggers.iter().position(|t| t.name == "pounce").unwrap();
    let (mut elapsed, mut hits, mut sounds, mut particles) = (0.0f32, Vec::new(), Vec::new(), Vec::new());
    for tick in 1..=60u32 {
        let e0 = elapsed;
        elapsed += TICK;
        rig.trigger_events(action, trig, e0, elapsed, &mut |ev| match ev.event.kind {
            EventKind::Timeline => hits.push((tick, ev.event.name.clone())),
            EventKind::Sound => sounds.push((tick, ev.event.name.clone())),
            EventKind::Particle => particles.push((tick, ev.event.name.clone(), ev.event.locator.clone())),
        });
    }
    // 0.35 s is tick 7; one tick of tolerance
    assert_eq!(hits.len(), 1, "{hits:?}");
    assert_eq!(hits[0].1, "hit");
    assert!((hits[0].0 as i32 - 7).abs() <= 1, "hit at tick {}", hits[0].0);
    assert_eq!(sounds, [(hits[0].0, "cinder_fox_pounce".to_string())]);
    assert_eq!(particles, [(hits[0].0, "cinder_fox_embers".to_string(), Some("mouth".to_string()))]);

    // the pose the trigger adds on top of the locomotion pose at the hit frame: body up 1.6 and forward 3.5 pixels
    let mut inp = ExprInputs::default();
    let st = rig.initial_state();
    let base = eval(&rig, &st, &[], &mut inp);
    let hit = eval(&rig, &st, &[TriggerPlay { layer: action as u16, trigger: trig as u16, elapsed: 0.35 }], &mut inp);
    let body = rig.skeleton.find("body").unwrap().index();
    let d = hit.bones[body].translation - base.bones[body].translation;
    assert!(d.abs_diff_eq(Vec3::new(0.0, 1.6, -3.5), 1e-4), "{d}");
    // and after 0.8 s the animation has ended and the pose is the locomotion pose again
    let after = eval(&rig, &st, &[TriggerPlay { layer: action as u16, trigger: trig as u16, elapsed: 0.9 }], &mut inp);
    assert_eq!(after, base);
}

#[test]
fn sit_follows_the_synced_state_and_holds() {
    let rig = fox(Route::Geo).rig;
    let pose_layer = layer(&rig, "pose");
    let mut st = rig.initial_state();
    let mut inp = ExprInputs::default();
    let slot = Symbols::new(&["state"]).synced_slot("state").unwrap();
    rig.update(&mut st, TICK, &mut inp, &mut |_| {});
    assert_eq!(state_name(&rig, &st, pose_layer), "stand");

    inp.synced[slot] = 1.0; // state = sit
    rig.update(&mut st, TICK, &mut inp, &mut |_| {});
    assert_eq!(state_name(&rig, &st, pose_layer), "sit");
    for _ in 0..40 {
        rig.update(&mut st, TICK, &mut inp, &mut |_| {});
    }
    let sitting = eval(&rig, &st, &[], &mut inp);
    for _ in 0..200 {
        rig.update(&mut st, TICK, &mut inp, &mut |_| {});
    }
    let later = eval(&rig, &st, &[], &mut inp);
    let idx = rig.find_animation("sit").unwrap();
    let mut last = Pose::default();
    rig.sample_animation(idx, 0.6, &mut inp, &mut last);
    let (body, leg) = (rig.skeleton.find("body").unwrap().index(), rig.skeleton.find("leg_bl").unwrap().index());
    // held: nothing moves between 2 s and 12 s in the state, and the sitting channels equal the animation's last frame
    for i in [body, leg] {
        assert!(sitting.bones[i].translation.abs_diff_eq(later.bones[i].translation, 1e-5));
        assert!(sitting.bones[i].rotation.abs_diff_eq(later.bones[i].rotation, 1e-5));
        assert!(later.bones[i].rotation.abs_diff_eq(last.bones[i].rotation, 1e-4));
    }
    assert!(later.bones[body].translation.abs_diff_eq(last.bones[body].translation, 1e-4), "body stays at the sit height");
    assert!((later.bones[body].translation.y + 2.0).abs() < 1e-4);

    // standing up again fades the layer out over 0.2 s (no pop): halfway the body is between sit and locomotion
    let loco_only = {
        let mut st2 = rig.initial_state();
        let mut inp2 = ExprInputs::default();
        for _ in 0..(2 + 40 + 200 + 1) {
            rig.update(&mut st2, TICK, &mut inp2, &mut |_| {});
        }
        eval(&rig, &st2, &[], &mut inp2)
    };
    let sit_y = later.bones[body].translation.y;
    inp.synced[slot] = 0.0;
    rig.update(&mut st, TICK, &mut inp, &mut |_| {});
    assert_eq!(state_name(&rig, &st, pose_layer), "stand");
    let start = eval(&rig, &st, &[], &mut inp).bones[body].translation.y;
    assert!((start - sit_y).abs() < 1e-3, "no jump at the start of the fade: {start} vs {sit_y}");
    rig.update(&mut st, TICK * 2.0, &mut inp, &mut |_| {}); // half of 0.2 s
    let mid = eval(&rig, &st, &[], &mut inp).bones[body].translation.y;
    let end_y = loco_only.bones[body].translation.y;
    assert!(mid < end_y - 0.5 && mid > sit_y + 0.5, "mid {mid} between {sit_y} and {end_y}");
    for _ in 0..4 {
        rig.update(&mut st, TICK, &mut inp, &mut |_| {});
    }
    let up = eval(&rig, &st, &[], &mut inp);
    assert!(up.bones[body].translation.abs_diff_eq(loco_only.bones[body].translation, 0.35), "back to locomotion");
    assert!(st.layers[pose_layer].prev.is_none());
}

// ---------------------------------------------------------------------------------------------------------------
// (e) AnimClock anchors on a simulated client
// ---------------------------------------------------------------------------------------------------------------

fn same_pose(a: &Pose, b: &Pose, rig: &Rig, what: &str) {
    for (i, (x, y)) in bone_matrices(&rig.skeleton, a).iter().zip(bone_matrices(&rig.skeleton, b)).enumerate() {
        assert!(x.abs_diff_eq(y, TOL), "{what}: bone {}:\n{x:?}\n{y:?}", rig.skeleton.bones[i].name);
    }
}

#[test]
fn anim_clock_anchors_reproduce_the_server_pose_on_a_client() {
    let rig = fox(Route::Geo).rig;
    let loco = layer(&rig, "locomotion");
    let c = &rig.controllers()[loco];
    let mut inp = ExprInputs::default();
    inp.set(Query::GroundSpeed, 0.3);

    // server: 60 ticks, running at 0.9 clock speed
    let mut server = rig.initial_state();
    for _ in 0..60 {
        rig.update(&mut server, TICK, &mut inp, &mut |_| {});
    }
    assert_eq!(state_name(&rig, &server, loco), "run");
    let anchor = c.anchor(&server.layers[loco], 0, 60, &mut inp);
    assert_eq!(anchor.state as usize, server.layers[loco].cur.state);
    assert!((anchor.speed - 0.9).abs() < 1e-5);
    assert!(!c.anchor_stale(&server.layers[loco], 0, &anchor, &mut inp));
    inp.set(Query::GroundSpeed, 0.2);
    assert!(c.anchor_stale(&server.layers[loco], 0, &anchor, &mut inp), "a speed change needs a new anchor");
    inp.set(Query::GroundSpeed, 0.3);

    // a late joiner enters the state without a fade and sets the clock from the anchor
    let mut client = rig.initial_state();
    c.set_state(&mut client.layers[loco], anchor.state as usize, false, &mut inp);
    c.apply_anchor(&mut client.layers[loco], 0, &anchor, 60, 0.0, &mut inp);
    same_pose(&eval(&rig, &server, &[], &mut inp), &eval(&rig, &client, &[], &mut inp), &rig, "tick 60");

    // 15 more ticks on the server; the client only re-evaluates the anchor at the later tick
    for _ in 0..15 {
        rig.update(&mut server, TICK, &mut inp, &mut |_| {});
    }
    c.apply_anchor(&mut client.layers[loco], 0, &anchor, 75, 0.0, &mut inp);
    same_pose(&eval(&rig, &server, &[], &mut inp), &eval(&rig, &client, &[], &mut inp), &rig, "tick 75");

    // between ticks: partial 0.5 of a tick is half a server step
    rig.update(&mut server, TICK / 2.0, &mut inp, &mut |_| {});
    c.apply_anchor(&mut client.layers[loco], 0, &anchor, 75, 0.5, &mut inp);
    same_pose(&eval(&rig, &server, &[], &mut inp), &eval(&rig, &client, &[], &mut inp), &rig, "tick 75.5");

    // a joiner that is simply handed the anchor (with the usual fade) has settled after the fade time
    let mut late = rig.initial_state();
    c.apply_anchor(&mut late.layers[loco], 0, &anchor, 60, 0.0, &mut inp);
    assert_eq!(state_name(&rig, &late, loco), "run");
    assert!(late.layers[loco].prev.is_some(), "entering with a fade");

    // the pounce trigger: the client derives the trigger's clock from (start tick, speed 1)
    let action = layer(&rig, "action");
    let trig = TriggerPlay { layer: action as u16, trigger: 0, elapsed: 0.0 };
    let start = AnimClock::new(0, 100, 0.0, 1.0);
    let mut srv_elapsed = 0.0f32;
    for _ in 0..7 {
        srv_elapsed += TICK;
    }
    srv_elapsed += TICK / 2.0;
    let client_elapsed = start.time_at(107, 0.5) as f32;
    assert!((client_elapsed - srv_elapsed).abs() < 1e-5, "{client_elapsed} vs {srv_elapsed}");
    let st = rig.initial_state();
    let a = eval(&rig, &st, &[TriggerPlay { elapsed: srv_elapsed, ..trig }], &mut inp);
    let b = eval(&rig, &st, &[TriggerPlay { elapsed: client_elapsed, ..trig }], &mut inp);
    same_pose(&a, &b, &rig, "pounce at 0.375 s");
}
