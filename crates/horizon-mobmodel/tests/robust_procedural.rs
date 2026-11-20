//! Robustness tests for the procedural constraint parser (`src/procedural.rs`).

use glam::Vec3;
use horizon_mobmodel::expr::Symbols;
use horizon_mobmodel::model::{Locator, ModelError};
use horizon_mobmodel::procedural::limits::{
    MAX_AIM, MAX_CONSTRAINTS, MAX_COUNTER_ROTATE, MAX_FOLLOW, MAX_IK, MAX_LOOK_AT, MAX_SPRING, MAX_SPRING_BONES,
    MAX_SPRING_CHAIN,
};
use horizon_mobmodel::procedural::{parse_procedural, parse_procedural_str};
use horizon_mobmodel::skeleton::{Bone, BoneIndex, Skeleton};
use rand::rngs::StdRng;
use rand::{RngExt, SeedableRng};
use serde_json::{json, Value};
use std::alloc::{GlobalAlloc, Layout, System};
use std::cell::Cell;

const PEAK_CAP: usize = 64 * 1024 * 1024;

struct PeakAlloc;

thread_local! {
    static ALLOC_CUR: Cell<usize> = const { Cell::new(0) };
    static ALLOC_PEAK: Cell<usize> = const { Cell::new(0) };
}

unsafe impl GlobalAlloc for PeakAlloc {
    unsafe fn alloc(&self, l: Layout) -> *mut u8 {
        let _ = ALLOC_CUR.try_with(|c| {
            let next = c.get().saturating_add(l.size());
            c.set(next);
            let _ = ALLOC_PEAK.try_with(|p| if next > p.get() { p.set(next) });
        });
        unsafe { System.alloc(l) }
    }
    unsafe fn dealloc(&self, _ptr: *mut u8, l: Layout) {
        let _ = ALLOC_CUR.try_with(|c| c.set(c.get().saturating_sub(l.size())));
        unsafe { System.dealloc(_ptr, l) }
    }
    unsafe fn realloc(&self, ptr: *mut u8, l: Layout, new_size: usize) -> *mut u8 {
        let _ = ALLOC_CUR.try_with(|c| {
            let next = c.get().saturating_sub(l.size()).saturating_add(new_size);
            c.set(next);
            let _ = ALLOC_PEAK.try_with(|p| if next > p.get() { p.set(next) });
        });
        unsafe { System.realloc(ptr, l, new_size) }
    }
}

#[global_allocator]
static GLOBAL: PeakAlloc = PeakAlloc;

fn within_cap<T>(f: impl FnOnce() -> T) -> T {
    let base = ALLOC_CUR.with(Cell::get);
    ALLOC_PEAK.with(|p| p.set(base));
    let out = f();
    let peak = ALLOC_PEAK.with(Cell::get).saturating_sub(base);
    assert!(peak <= PEAK_CAP, "peak allocation {peak} exceeded {PEAK_CAP}");
    out
}

fn skel() -> Skeleton {
    let b = |name: &str, p: Option<u16>, piv: [f32; 3]| Bone {
        name: name.into(),
        parent: p.map(BoneIndex),
        pivot: Vec3::from(piv),
        rotation: Vec3::ZERO,
    };
    Skeleton {
        bones: vec![
            b("body", None, [0.0, 0.0, 0.0]),
            b("head", Some(0), [0.0, 10.0, 0.0]),
            b("arm", Some(0), [4.0, 8.0, 0.0]),
            b("hand", Some(2), [4.0, 4.0, 0.0]),
        ],
    }
}

fn locs() -> Vec<Locator> {
    vec![Locator { name: "tip_loc".into(), bone: BoneIndex(3), offset: Vec3::new(4.0, 2.0, 0.0), rotation: Vec3::ZERO, null_object: false }]
}

fn drop_value_iteratively(root: Value) {
    let mut stack = vec![root];
    while let Some(v) = stack.pop() {
        match v {
            Value::Array(arr) => stack.extend(arr),
            Value::Object(map) => {
                for (_, val) in map {
                    stack.push(val);
                }
            }
            _ => {}
        }
    }
}

fn assert_err_path<T>(r: Result<T, ModelError>, require_path: bool, label: &str) {
    let e = match r {
        Ok(_) => panic!("{label}: expected Err, got Ok"),
        Err(e) => e,
    };
    assert_eq!(e.file, "t.json", "{label}: {e}");
    assert!(!e.message.is_empty(), "{label}: empty message");
    if require_path {
        assert!(!e.path.is_empty(), "{label}: expected non-empty path, got {e}");
    }
}

const BASE_PROC: &str = r#"[
  {"type": "look_at", "bone": "head", "target": "view", "yaw": 60, "pitch": 40, "speed": 180, "forward": "-z", "up": "y", "offset": [0, 0, 0], "when": "1", "fade": 0.1, "weight": 1.0},
  {"type": "aim", "bone": "arm", "target": "target", "axis": [0, 0, -1], "max_angle": 90, "speed": 180, "offset": [0, 0, 0]},
  {"type": "two_bone_ik", "root": "body", "mid": "arm", "tip": "hand", "tip_offset": [0, -1, 0], "target": {"input": 0}, "pole": {"dir": [0, 0, -1]}},
  {"type": "spring", "bones": ["arm", "hand"], "stiffness": 60, "damping": 4, "gravity": 10, "max_angle": 60, "tip_length": 4, "tip_dir": [0, -1, 0]},
  {"type": "follow", "bone": "head", "source": "arm", "delay_ticks": 2, "channels": ["rotation"]},
  {"type": "counter_rotate", "bone": "head", "axis": [0, 1, 0], "up": [0, 1, 0]}
]"#;

#[test]
fn truncated_empty_utf8_and_random_strings() {
    let s = skel();
    let l = locs();
    for cut in 0..BASE_PROC.len() {
        if !BASE_PROC.is_char_boundary(cut) {
            continue;
        }
        let mut syms = Symbols::new(&["mode"]);
        if let Err(e) = within_cap(|| parse_procedural_str("t.json", &BASE_PROC[..cut], &s, &l, &mut syms)) {
            assert_eq!(e.file, "t.json", "cut {cut}");
            assert!(!e.message.is_empty());
        }
    }
    for bad in ["", "   \n\t", "\u{FEFF}", "\u{FEFF}あ🦀\0", "null", "{}", "123", "\"x\""] {
        let mut syms = Symbols::new(&[]);
        assert_err_path(within_cap(|| parse_procedural_str("t.json", bad, &s, &l, &mut syms)), false, bad);
    }
    let mut rng = StdRng::seed_from_u64(0x940C);
    let alpha: Vec<char> = "{}[]:,\"0123456789.-+abcdefghijklmnopqrstuvwxyz_ \n\t\u{3042}\u{1F98A}\0".chars().collect();
    for _ in 0..400 {
        let len = rng.random_range(0..450);
        let txt: String = (0..len).map(|_| alpha[rng.random_range(0..alpha.len())]).collect();
        let mut syms = Symbols::new(&[]);
        if let Err(e) = within_cap(|| parse_procedural_str("t.json", &txt, &s, &l, &mut syms)) {
            assert_eq!(e.file, "t.json");
            assert!(!e.message.is_empty());
        }
    }
}

#[test]
fn wrong_types_and_invalid_values_at_every_field() {
    let s = skel();
    let l = locs();
    let bad_items = [
        json!(123),
        json!({}),
        json!({"type": 1}),
        json!({"type": "unknown"}),
        json!({"type": "look_at", "bone": "head", "extra": 1}),
        json!({"type": "look_at", "bone": "head", "when": 1}),
        json!({"type": "look_at", "bone": "head", "when": "1 +"}),
        json!({"type": "look_at", "bone": "head", "when": "q.synced('state') == 'bad'"}),
        json!({"type": "look_at", "bone": "head", "fade": "0.1"}),
        json!({"type": "look_at", "bone": "head", "fade": -0.1}),
        json!({"type": "look_at", "bone": "head", "fade": 10.1}),
        json!({"type": "look_at", "bone": "head", "weight": 1.1}),
        json!({"type": "look_at", "bone": 1}),
        json!({"type": "look_at", "bone": "missing"}),
        json!({"type": "look_at", "bone": "head", "target": "bad"}),
        json!({"type": "look_at", "bone": "head", "target": {}}),
        json!({"type": "look_at", "bone": "head", "target": {"input": 5}}),
        json!({"type": "look_at", "bone": "head", "target": {"input": 0.5}}),
        json!({"type": "look_at", "bone": "head", "target": {"point": [1, 2]}}),
        json!({"type": "look_at", "bone": "head", "target": {"locator": "nope"}}),
        json!({"type": "look_at", "bone": "head", "target": {"bone": "nope"}}),
        json!({"type": "look_at", "bone": "head", "target": {"bad": 1}}),
        json!({"type": "look_at", "bone": "head", "yaw": 181}),
        json!({"type": "look_at", "bone": "head", "pitch": -1}),
        json!({"type": "look_at", "bone": "head", "speed": 1e6}),
        json!({"type": "look_at", "bone": "head", "forward": "w"}),
        json!({"type": "look_at", "bone": "head", "forward": "y", "up": "y"}),
        json!({"type": "look_at", "bone": "head", "offset": "bad"}),
        json!({"type": "aim", "bone": "arm", "axis": [0, 0, 0]}),
        json!({"type": "aim", "bone": "arm", "max_angle": 181}),
        json!({"type": "two_bone_ik", "root": "body", "mid": "arm", "target": "view"}),
        json!({"type": "two_bone_ik", "root": "body", "mid": "arm", "tip": "hand", "tip_locator": "tip_loc", "target": "view"}),
        json!({"type": "two_bone_ik", "root": "body", "mid": "arm", "tip_locator": "tip_loc", "tip_offset": [0, 0, 0], "target": "view"}),
        json!({"type": "two_bone_ik", "root": "arm", "mid": "head", "tip": "head", "target": "view"}),
        json!({"type": "two_bone_ik", "root": "body", "mid": "arm", "tip": "hand", "target": "view", "pole": {"dir": [0, 0, 0]}}),
        json!({"type": "two_bone_ik", "root": "body", "mid": "arm", "tip": "hand", "target": "view", "pole": {"bad": 1}}),
        json!({"type": "spring", "bones": "arm"}),
        json!({"type": "spring", "bones": []}),
        json!({"type": "spring", "bones": ["head", "hand"]}),
        json!({"type": "spring", "bones": ["arm"], "stiffness": 10001}),
        json!({"type": "spring", "bones": ["arm"], "damping": 1001}),
        json!({"type": "spring", "bones": ["arm"], "gravity": 200000}),
        json!({"type": "spring", "bones": ["arm"], "tip_length": 0}),
        json!({"type": "spring", "bones": ["arm"], "tip_dir": [0, 0, 0]}),
        json!({"type": "follow", "bone": "head", "source": "head"}),
        json!({"type": "follow", "bone": "head", "source": "arm", "delay_ticks": 1.5}),
        json!({"type": "follow", "bone": "head", "source": "arm", "delay_ticks": 41}),
        json!({"type": "follow", "bone": "head", "source": "arm", "channels": []}),
        json!({"type": "follow", "bone": "head", "source": "arm", "channels": ["bad"]}),
        json!({"type": "follow", "bone": "head", "source": "arm", "channels": ["rotation", "rotation"]}),
        json!({"type": "counter_rotate", "bone": "head", "axis": [0, 0, 0]}),
        json!({"type": "counter_rotate", "bone": "head", "up": [0, 0, 0]}),
    ];
    for item in bad_items {
        let mut syms = Symbols::new(&["state"]);
        syms.set_enum_values("state", &["idle", "sit"]);
        let v = json!([item]);
        assert_err_path(within_cap(|| parse_procedural("t.json", "model.procedural", &v, &s, &l, &mut syms)), true, &item.to_string());
    }
}

#[test]
fn limits_numbers_deep_nesting_and_duplicates() {
    let s = skel();
    let l = locs();
    let mut syms = Symbols::new(&[]);
    let over_total = json!(vec![json!({"type": "counter_rotate", "bone": "head"}); MAX_CONSTRAINTS + 1]);
    assert_err_path(within_cap(|| parse_procedural("t.json", "p", &over_total, &s, &l, &mut syms)), true, "max_constraints");

    for (item, cap, tag) in [
        (json!({"type": "look_at", "bone": "head"}), MAX_LOOK_AT, "look"),
        (json!({"type": "aim", "bone": "arm"}), MAX_AIM, "aim"),
        (json!({"type": "two_bone_ik", "root": "body", "mid": "arm", "tip": "hand", "target": "view"}), MAX_IK, "ik"),
        (json!({"type": "spring", "bones": ["arm"]}), MAX_SPRING, "spring"),
        (json!({"type": "follow", "bone": "head", "source": "arm"}), MAX_FOLLOW, "follow"),
        (json!({"type": "counter_rotate", "bone": "head"}), MAX_COUNTER_ROTATE, "cr"),
    ] {
        let v = json!(vec![item; cap + 1]);
        assert_err_path(within_cap(|| parse_procedural("t.json", "p", &v, &s, &l, &mut syms)), true, tag);
    }

    let chain_skel = Skeleton {
        bones: (0..=MAX_SPRING_CHAIN).map(|i| Bone {
            name: format!("b{i}"),
            parent: if i == 0 { None } else { Some(BoneIndex((i - 1) as u16)) },
            pivot: Vec3::new(0.0, i as f32, 0.0),
            rotation: Vec3::ZERO,
        }).collect(),
    };
    let chain_names: Vec<String> = (0..=MAX_SPRING_CHAIN).map(|i| format!("b{i}")).collect();
    assert_err_path(within_cap(|| parse_procedural("t.json", "p", &json!([{"type": "spring", "bones": chain_names}]), &chain_skel, &[], &mut syms)), true, "spring_chain");

    let sixteen: Vec<String> = (0..MAX_SPRING_CHAIN).map(|i| format!("b{i}")).collect();
    let n_springs = (MAX_SPRING_BONES / MAX_SPRING_CHAIN) + 1;
    let over_spring_bones = json!(vec![json!({"type": "spring", "bones": sixteen}); n_springs]);
    assert_err_path(within_cap(|| parse_procedural("t.json", "p", &over_spring_bones, &chain_skel, &[], &mut syms)), true, "spring_bones");

    for num in ["1e400", "-1e400", "1e300", "-1e300", "\"NaN\"", "\"Infinity\""] {
        let raw = format!(r#"[{{"type":"look_at","bone":"head","yaw":{num}}}]"#);
        assert_err_path(within_cap(|| parse_procedural_str("t.json", &raw, &s, &l, &mut syms)), false, num);
    }

    let deep_json = format!("{}{}", "[".repeat(3000), "]".repeat(3000));
    assert_err_path(within_cap(|| parse_procedural_str("t.json", &deep_json, &s, &l, &mut syms)), false, "deep_json");
    let mut deep_val = Value::from(1);
    for _ in 0..2000 {
        deep_val = Value::Array(vec![deep_val]);
    }
    assert_err_path(within_cap(|| parse_procedural("t.json", "p", &deep_val, &s, &l, &mut syms)), true, "deep_val");
    drop_value_iteratively(deep_val);

    let dup = r#"[{"type":"look_at","bone":"head","yaw":10,"yaw":20}]"#;
    assert!(within_cap(|| parse_procedural_str("t.json", dup, &s, &l, &mut syms)).is_ok());
}
