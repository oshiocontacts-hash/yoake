//! `ProceduralSet::evaluate` and `apply_bone_writes` must not allocate once the state exists: a counting allocator proves it.

use glam::Vec3;
use horizon_mobmodel::expr::{ExprInputs, Symbols};
use horizon_mobmodel::procedural::*;
use horizon_mobmodel::skeleton::{Bone, BoneIndex, Pose, Skeleton};
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
fn evaluation_does_not_allocate() {
    let b = |n: &str, p: Option<u16>, piv: [f32; 3]| Bone {
        name: n.into(),
        parent: p.map(BoneIndex),
        pivot: Vec3::from(piv),
        rotation: Vec3::ZERO,
    };
    let sk = Skeleton {
        bones: vec![
            b("body", None, [0.0, 10.0, 0.0]),
            b("head", Some(0), [0.0, 16.0, 0.0]),
            b("t1", Some(0), [0.0, 10.0, 4.0]),
            b("t2", Some(2), [0.0, 10.0, 8.0]),
            b("leg", Some(0), [2.0, 8.0, 0.0]),
            b("paw", Some(4), [2.0, 4.0, 0.0]),
        ],
    };
    let json = r#"[
      {"type":"look_at","bone":"head","target":"view"},
      {"type":"aim","bone":"head","target":"target"},
      {"type":"two_bone_ik","root":"leg","mid":"paw","tip":"paw","tip_offset":[0,-4,0],"target":{"input":0},"when":"q.synced('a') == 0"},
      {"type":"spring","bones":["t1","t2"],"gravity":100,"tip_length":4},
      {"type":"follow","bone":"t2","source":"head","delay_ticks":6},
      {"type":"counter_rotate","bone":"head"}
    ]"#;
    let mut syms = Symbols::new(&["a"]);
    let set = parse_procedural_str("t.json", json, &sk, &[], &mut syms).unwrap();
    let mut st = ProceduralState::new(&set);
    let e = ExprInputs::default();
    let mut pose = Pose::rest(&sk);
    let mut vis = BoneVisibility::new(sk.len());
    let mut w = BoneWrite::new(1, WriteMode::Additive);
    w.translation = Some(Vec3::Y);
    w.visible = Some(false);
    let writes = [w; 8];
    let before = ALLOCS.with(|c| c.get());
    for f in 0..500 {
        for b in &mut pose.bones {
            *b = Default::default();
        }
        let inp = ProceduralInputs::new(&e, 0.016)
            .with_target(TargetSource::View, Vec3::new(f as f32 * 0.1, 20.0, -30.0))
            .with_target(TargetSource::Target, Vec3::new(5.0, 10.0, -9.0))
            .with_target(TargetSource::Custom(0), Vec3::new(3.0, 3.0, 2.0));
        let rep = set.evaluate(&mut pose, &mut st, &inp);
        assert!(rep.applied >= 5);
        apply_bone_writes_with_visibility(&mut pose, &mut vis, &writes);
    }
    let after = ALLOCS.with(|c| c.get());
    assert_eq!(after - before, 0, "procedural evaluation allocated");
}
