//! Event emission into a caller-provided buffer must not allocate once the rig, tables and buffer exist.

use glam::Vec3;
use horizon_mobmodel::anim::{parse_animation_file, AnimationLibrary};
use horizon_mobmodel::clock::AnimClock;
use horizon_mobmodel::controller::parse_controllers;
use horizon_mobmodel::events::*;
use horizon_mobmodel::expr::{ExprInputs, Symbols};
use horizon_mobmodel::model::Locator;
use horizon_mobmodel::pose::{Rig, RigSource};
use horizon_mobmodel::skeleton::{bone_matrices_into, Bone, BoneIndex, Pose, Skeleton};
use std::alloc::{GlobalAlloc, Layout, System};
use std::cell::Cell;

struct Counting;

thread_local! {
    static ALLOCS: Cell<usize> = const { Cell::new(0) };
}

unsafe impl GlobalAlloc for Counting {
    unsafe fn alloc(&self, l: Layout) -> *mut u8 {
        let _ = ALLOCS.try_with(|c| c.set(c.get() + 1));
        unsafe { System.alloc(l) }
    }
    unsafe fn dealloc(&self, p: *mut u8, l: Layout) {
        unsafe { System.dealloc(p, l) }
    }
    unsafe fn realloc(&self, p: *mut u8, l: Layout, n: usize) -> *mut u8 {
        let _ = ALLOCS.try_with(|c| c.set(c.get() + 1));
        unsafe { System.realloc(p, l, n) }
    }
}

#[global_allocator]
static A: Counting = Counting;

#[test]
fn emission_does_not_allocate() {
    let sk = Skeleton {
        bones: vec![
            Bone { name: "body".into(), parent: None, pivot: Vec3::new(0.0, 12.0, 0.0), rotation: Vec3::ZERO },
            Bone { name: "head".into(), parent: Some(BoneIndex(0)), pivot: Vec3::new(0.0, 24.0, 0.0), rotation: Vec3::ZERO },
        ],
    };
    let locators = vec![Locator { name: "nose".into(), bone: BoneIndex(1), offset: Vec3::new(0.0, 24.0, -6.0), rotation: Vec3::ZERO, null_object: false }];
    let anims = r#"{"animations":{
      "ticker":{"loop":true,"animation_length":0.4,"timeline":{"0":"tick","0.2":"tock"},
                "sound_effects":{"0.2":{"effect":"mod:step"}},"particle_effects":{"0.2":{"effect":"dust","locator":"nose"}}},
      "wave":{"loop":false,"animation_length":1,"timeline":{"0.5":"hit"}}}}"#;
    let ctrl = r#"{"controllers":{"loco":{"initial":"a","states":{"a":{"animations":["ticker"]}}},"act":{"mode":"add","triggers":{"wave":"wave"}}}}"#;
    let mut syms = Symbols::new(&[]);
    let mut lib = AnimationLibrary::new();
    for a in parse_animation_file(anims, &mut syms).unwrap().animations {
        lib.add(a).unwrap();
    }
    let c = parse_controllers(ctrl, &mut syms).unwrap().controllers;
    let (rig, _) = Rig::new(sk.clone(), lib, c).unwrap();
    let model: serde_json::Value = serde_json::json!({"particles": {"dust": "minecraft:cloud"}, "server_events": true});
    let me = MobEvents::from_model_json("t", "a", &model, rig.animations(), &locators).unwrap();

    let mut st = rig.initial_state();
    let mut inp = ExprInputs::default();
    let mut buf = EventBuf::with_capacity(64);
    let mut mats: Vec<glam::Mat4> = Vec::new();
    let pose = Pose::rest(&sk);
    bone_matrices_into(&sk, &pose, &mut mats); // sizes the scratch
    let clock = AnimClock::new(0, 0, 0.0, 1.0);
    let mut cur = EventCursor::fresh(&clock);
    let mut marks = EventMarks::new(&rig);
    // warm up once so any first-use growth happens before the count
    me.update(&rig, EventSide::Client, &mut st, 0.05, &mut inp, &mut buf);

    let before = ALLOCS.with(|c| c.get());
    let mut seen = 0usize;
    let mut sum = Vec3::ZERO;
    for tick in 1..=400i64 {
        buf.clear();
        me.update(&rig, EventSide::Client, &mut st, 0.05, &mut inp, &mut buf);
        me.update(&rig, EventSide::Server, &mut st, 0.0, &mut inp, &mut buf);
        me.update_marked(&rig, EventSide::Server, &mut st, 0.05, &mut inp, &mut marks, &mut buf);
        me.trigger_window_marked(&rig, EventSide::Server, 1, 0, 0.0, 1.0, &mut marks, &mut buf);
        me.trigger_window(&rig, EventSide::Server, 1, 0, 0.0, 1.0, &mut buf);
        let w = cur.advance(&clock, tick, 0.0);
        me.animation_window(&rig, EventSide::Client, 0, RigSource::State { state: 0, slot: 0 }, 0, w.c0, w.c1, &mut buf);
        bone_matrices_into(&sk, &pose, &mut mats);
        for e in buf.as_slice() {
            seen += 1;
            sum += event_position(&locators, &mats, e);
            let _ = me.string(StrId(0));
        }
    }
    let after = ALLOCS.with(|c| c.get());
    assert!(seen > 400, "events were emitted ({seen})");
    assert!(sum.is_finite());
    assert_eq!(after - before, 0, "event emission allocated");
}
