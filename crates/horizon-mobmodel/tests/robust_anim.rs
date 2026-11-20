//! Robustness tests for `animation.json` and bbmodel `animations[]` (`src/anim.rs`).

use horizon_mobmodel::anim::{animations_from_bbmodel, parse_animation_file, AnimationLibrary};
use horizon_mobmodel::expr::Symbols;
use horizon_mobmodel::limits::MAX_ANIMATIONS;
use rand::rngs::StdRng;
use rand::{RngExt, SeedableRng};
use serde_json::json;
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

const BASE_ANIM: &str = r#"{
  "format_version": "1.8.0",
  "animations": {
    "a": {
      "loop": true,
      "animation_length": 1.0,
      "override_previous_animation": false,
      "anim_time_update": "query.anim_time",
      "blend_weight": "1",
      "bones": {
        "b": {
          "rotation": {"0": {"pre": [0, 0, 0], "post": [1, 2, 3], "lerp_mode": "bezier", "easing": "easeInSine", "easingArgs": [1], "bezier_left_time": [-0.1, -0.1, -0.1]}},
          "position": [0, 0, 0],
          "scale": 1
        }
      },
      "timeline": {"0.5": "hit"},
      "sound_effects": {"0.2": {"effect": "mod:step", "volume": 1.0, "pitch": 1.0}},
      "particle_effects": {"0.3": {"effect": "dust", "locator": "foot", "offset": [0, 1, 0]}}
    }
  }
}"#;

#[test]
fn truncated_empty_utf8_and_random_strings() {
    for cut in 0..BASE_ANIM.len() {
        if !BASE_ANIM.is_char_boundary(cut) {
            continue;
        }
        let mut syms = Symbols::new(&[]);
        if let Err(e) = within_cap(|| parse_animation_file(&BASE_ANIM[..cut], &mut syms)) {
            assert!(!e.message.is_empty(), "cut {cut}");
        }
    }
    for bad in ["", "   \n\t", "\u{FEFF}", "\u{FEFF}あ🦀\0", "null", "[]", "123", "\"x\"", "{}"] {
        let mut syms = Symbols::new(&[]);
        let e = within_cap(|| parse_animation_file(bad, &mut syms)).unwrap_err();
        assert!(!e.message.is_empty(), "{bad}");
    }
    let mut rng = StdRng::seed_from_u64(0xAA11);
    let alpha: Vec<char> = "{}[]:,\"0123456789.-+eEabcdefghijklmnopqrstuvwxyz_ \n\t\u{3042}\u{1F98A}\0".chars().collect();
    for _ in 0..500 {
        let len = rng.random_range(0..500);
        let s: String = (0..len).map(|_| alpha[rng.random_range(0..alpha.len())]).collect();
        let mut syms = Symbols::new(&[]);
        if let Err(e) = within_cap(|| parse_animation_file(&s, &mut syms)) {
            assert!(!e.message.is_empty());
        }
    }
}

#[test]
fn wrong_types_at_every_field_of_animation_json() {
    let cases = [
        r#"{"animations": 1}"#,
        r#"{"animations": {"a": 1}}"#,
        r#"{"animations": {"a": {"loop": "forever"}}}"#,
        r#"{"animations": {"a": {"loop": 1}}}"#,
        r#"{"animations": {"a": {"animation_length": -1}}}"#,
        r#"{"animations": {"a": {"animation_length": "1"}}}"#,
        r#"{"animations": {"a": {"override_previous_animation": "true"}}}"#,
        r#"{"animations": {"a": {"anim_time_update": []}}}"#,
        r#"{"animations": {"a": {"anim_time_update": "1 +"}}}"#,
        r#"{"animations": {"a": {"blend_weight": {}}}}"#,
        r#"{"animations": {"a": {"blend_weight": "query.synced('nope')"}}}"#,
        r#"{"animations": {"a": {"bones": []}}}"#,
        r#"{"animations": {"a": {"bones": {"b": []}}}}"#,
        r#"{"animations": {"a": {"bones": {"b": {"rotation": [1, 2]}}}}}"#,
        r#"{"animations": {"a": {"bones": {"b": {"rotation": {}}}}}}"#,
        r#"{"animations": {"a": {"bones": {"b": {"rotation": {"bad": [0, 0, 0]}}}}}}"#,
        r#"{"animations": {"a": {"bones": {"b": {"rotation": {"-1": [0, 0, 0]}}}}}}"#,
        r#"{"animations": {"a": {"bones": {"b": {"rotation": {"0": {}}}}}}}"#,
        r#"{"animations": {"a": {"bones": {"b": {"rotation": {"0": {"pre": [1, 2]}}}}}}}"#,
        r#"{"animations": {"a": {"bones": {"b": {"rotation": {"0": {"post": "1 +"}}}}}}}"#,
        r#"{"animations": {"a": {"bones": {"b": {"rotation": {"0": {"post": [0, 0, 0], "bezier_left_time": "x"}}}}}}}"#,
        r#"{"animations": {"a": {"bones": {"b": {"position": {"0": {"post": [0, 0, 0], "bezier_right_value": [1, "x", 0]}}}}}}}"#,
        r#"{"animations": {"a": {"timeline": []}}}"#,
        r#"{"animations": {"a": {"timeline": {"bad": "hit"}}}}"#,
        r#"{"animations": {"a": {"timeline": {"0": 123}}}}"#,
        r#"{"animations": {"a": {"sound_effects": []}}}"#,
        r#"{"animations": {"a": {"sound_effects": {"0": 123}}}}"#,
        r#"{"animations": {"a": {"sound_effects": {"0": {}}}}}"#,
        r#"{"animations": {"a": {"sound_effects": {"0": {"effect": "s", "volume": 5}}}}}"#,
        r#"{"animations": {"a": {"sound_effects": {"0": {"effect": "s", "pitch": 0.1}}}}}"#,
        r#"{"animations": {"a": {"particle_effects": []}}}"#,
        r#"{"animations": {"a": {"particle_effects": {"0": {"effect": "p", "offset": [1, 2]}}}}}"#,
        r#"{"animations": {"a": {"particle_effects": {"0": {"effect": "p", "offset": [1e5, 0, 0]}}}}}"#,
    ];
    for raw in cases {
        let mut syms = Symbols::new(&[]);
        let e = within_cap(|| parse_animation_file(raw, &mut syms)).unwrap_err();
        assert!(!e.message.is_empty(), "{raw}");
        if raw.contains(r#""a":"#) {
            assert_eq!(e.context, "a", "{raw}: {e}");
        }
    }
    let mut syms = Symbols::new(&[]);
    let e = within_cap(|| parse_animation_file(r#"{"animations":{"a":{"anim_time_update":"1 +"}}}"#, &mut syms)).unwrap_err();
    assert!(e.column.is_some());
}

#[test]
fn bbmodel_animations_errors_and_limits() {
    for v in [
        json!(null),
        json!({}),
        json!(123),
        json!([{"name": "a", "loop": "forever"}]),
        json!([{"name": "a", "anim_time_update": "1 +"}]),
        json!([{"name": "a", "blend_weight": "query.synced('missing')"}]),
        json!([{"name": "a", "animators": {"u": {"name": "b", "keyframes": [{"channel": "rotation", "data_points": [{}]}]}}}]),
        json!([{"name": "a", "animators": {"u": {"name": "b", "keyframes": [{"channel": "rotation", "time": -1, "data_points": [{}]}]}}}]),
        json!([{"name": "a", "animators": {"u": {"name": "b", "keyframes": [{"channel": "rotation", "time": 0, "data_points": []}]}}}]),
        json!([{"name": "a", "animators": {"u": {"name": "b", "keyframes": [{"channel": "rotation", "time": 0, "data_points": [{"x": "1 +"}]}]}}}]),
        json!([{"name": "a", "animators": {"u": {"name": "b", "keyframes": [{"channel": "rotation", "time": 0, "data_points": [{}], "bezier_left_time": [1, 2]}]}}}]),
        json!([{"name": "a", "animators": {"u": {"type": "effect", "keyframes": [{"channel": "sound", "data_points": [{"effect": "s"}]}]}}}]),
        json!([{"name": "a", "animators": {"u": {"type": "effect", "keyframes": [{"channel": "sound", "time": 0, "data_points": [{"effect": "s", "volume": 9}]}]}}}]),
        json!([{"name": "a", "animators": {"u": {"type": "effect", "keyframes": [{"channel": "particle", "time": 0, "data_points": [{"effect": "p", "offset": ["bad", 0, 0]}]}]}}}]),
    ] {
        let mut syms = Symbols::new(&[]);
        let e = within_cap(|| animations_from_bbmodel(&v, &mut syms)).unwrap_err();
        assert!(!e.message.is_empty(), "{v}");
    }

    // Large arrays & AnimationLibrary limit.
    let mut map = serde_json::Map::new();
    for i in 0..(MAX_ANIMATIONS + 20) {
        map.insert(format!("a{i}"), json!({}));
    }
    let big = serde_json::to_string(&json!({"animations": map})).unwrap();
    let mut syms = Symbols::new(&[]);
    let parsed = within_cap(|| parse_animation_file(&big, &mut syms)).unwrap();
    let mut lib = AnimationLibrary::new();
    for (i, a) in parsed.animations.into_iter().enumerate() {
        let res = lib.add(a);
        if i >= MAX_ANIMATIONS {
            assert!(res.is_err());
        } else {
            assert!(res.is_ok());
        }
    }
}

#[test]
fn deep_nesting_huge_numbers_and_duplicate_keys() {
    let deep_arr = format!("{}{}", "[".repeat(3000), "]".repeat(3000));
    let mut syms = Symbols::new(&[]);
    assert!(within_cap(|| parse_animation_file(&deep_arr, &mut syms)).is_err());
    let deep_obj = format!("{}1{}", "{\"a\":".repeat(3000), "}".repeat(3000));
    assert!(within_cap(|| parse_animation_file(&deep_obj, &mut syms)).is_err());

    for num in ["1e400", "-1e400", "\"NaN\"", "\"Infinity\"", "\"-Infinity\""] {
        let raw = format!(r#"{{"animations":{{"a":{{"animation_length":{num}}}}}}}"#);
        assert!(within_cap(|| parse_animation_file(&raw, &mut syms)).is_err(), "{num}");
    }
    let dup = r#"{"animations":{"a":{"bones":{"b":{"rotation":{"1":[0,0,0],"1.0":[1,0,0]}}}},"a":{"loop":true}}}"#;
    let p = within_cap(|| parse_animation_file(dup, &mut syms)).unwrap();
    assert_eq!(p.animations.len(), 2);
}

#[test]
#[ignore = "finding: 1e300 in anim.rs scalar/animation_length/bezier/bb_time turns into f32::INFINITY"]
fn huge_f64_1e300_is_rejected_not_turned_into_infinity() {
    let mut syms = Symbols::new(&[]);
    let raw = r#"{"animations":{"a":{"loop":true,"animation_length":1e300,"bones":{"b":{"scale":1e300}}}}}"#;
    assert!(within_cap(|| parse_animation_file(raw, &mut syms)).is_err());
    let bb = json!([{"name": "a", "animators": {"u": {"name": "b", "keyframes": [{"channel": "scale", "time": 1e300, "data_points": [{}]}]}}}]);
    assert!(within_cap(|| animations_from_bbmodel(&bb, &mut syms)).is_err());
}

#[test]
#[ignore = "finding: animations_from_bbmodel silently accepts non-object elements, animators, keyframes, data_points"]
fn bbmodel_animations_reject_wrong_nested_container_types() {
    let mut syms = Symbols::new(&[]);
    for v in [
        json!([123]),
        json!([null]),
        json!([{"name": "a", "animators": 123}]),
        json!([{"name": "a", "animators": {"u": {"name": "b", "keyframes": 123}}}]),
        json!([{"name": "a", "animators": {"u": {"name": "b", "keyframes": [{"channel": "rotation", "time": 0, "data_points": [123]}]}}}]),
    ] {
        assert!(within_cap(|| animations_from_bbmodel(&v, &mut syms)).is_err(), "{v}");
    }
}
