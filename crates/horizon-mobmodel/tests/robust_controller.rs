//! Robustness tests for the animation controller parser (`src/controller.rs`).

use horizon_mobmodel::controller::{parse_controllers, MAX_LAYERS, MAX_STATES, MAX_STATE_ANIMS};
use horizon_mobmodel::expr::{Symbols, MAX_VARIABLES};
use rand::rngs::StdRng;
use rand::{RngExt, SeedableRng};
use serde_json::{json, Map};
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

const BASE_CTRL: &str = r#"{
  "controllers": {
    "loco": {
      "initial": "idle",
      "weight": 1.0,
      "bones_mask": ["body"],
      "include_children": true,
      "mode": "override",
      "states": {
        "idle": {
          "animations": ["idle", {"name": "breathe", "speed": 1, "weight": 0.5}],
          "blend": 0.1,
          "speed": "1",
          "inactive": false,
          "on_enter": {"v.x": "0"},
          "transitions": [{"to": "walk", "when": "q.ground_speed > 0.1", "blend": 0.2}]
        },
        "walk": {"animations": ["walk"]}
      },
      "triggers": {"attack": "attack_anim"}
    }
  }
}"#;

#[test]
fn truncated_empty_utf8_and_random_strings() {
    for cut in 0..BASE_CTRL.len() {
        if !BASE_CTRL.is_char_boundary(cut) {
            continue;
        }
        let mut syms = Symbols::new(&[]);
        if let Err(e) = within_cap(|| parse_controllers("c.json", &BASE_CTRL[..cut], &mut syms)) {
            assert_eq!(e.file, "c.json", "cut {cut}");
            assert!(!e.message.is_empty(), "cut {cut}");
        }
    }
    for bad in ["", "  \n\t", "\u{FEFF}", "\u{FEFF}あ🦀\0", "null", "[]", "123", "\"x\"", "{}"] {
        let mut syms = Symbols::new(&[]);
        let e = within_cap(|| parse_controllers("c.json", bad, &mut syms)).unwrap_err();
        assert_eq!(e.file, "c.json", "{bad}");
        assert!(!e.message.is_empty(), "{bad}");
    }
    let syn = within_cap(|| parse_controllers("c.json", "{", &mut Symbols::new(&[]))).unwrap_err();
    assert_eq!(syn.file, "c.json");
    assert!(syn.to_string().contains("line 1"), "{syn}");
    let mut rng = StdRng::seed_from_u64(0xC741);
    let alpha: Vec<char> = "{}[]:,\"0123456789.-+abcdefghijklmnopqrstuvwxyz_ \n\t\u{3042}\u{1F98A}\0".chars().collect();
    for _ in 0..500 {
        let len = rng.random_range(0..500);
        let s: String = (0..len).map(|_| alpha[rng.random_range(0..alpha.len())]).collect();
        let mut syms = Symbols::new(&[]);
        if let Err(e) = within_cap(|| parse_controllers("c.json", &s, &mut syms)) {
            assert_eq!(e.file, "c.json");
            assert!(!e.message.is_empty());
        }
    }
}

#[test]
fn wrong_types_at_every_field() {
    let cases = [
        r#"{"controllers": []}"#,
        r#"{"controllers": {"c": 1}}"#,
        r#"{"controllers": {"c": {"initial": 1}}}"#,
        r#"{"controllers": {"c": {"initial": "missing", "states": {"s": {}}}}}"#,
        r#"{"controllers": {"c": {"weight": []}}}"#,
        r#"{"controllers": {"c": {"weight": "1 +"}}}"#,
        r#"{"controllers": {"c": {"bones_mask": "body"}}}"#,
        r#"{"controllers": {"c": {"bones_mask": [1]}}}"#,
        r#"{"controllers": {"c": {"include_children": "true"}}}"#,
        r#"{"controllers": {"c": {"mode": 1}}}"#,
        r#"{"controllers": {"c": {"mode": "blend"}}}"#,
        r#"{"controllers": {"c": {"triggers": []}}}"#,
        r#"{"controllers": {"c": {"triggers": {"t": 123}}}}"#,
        r#"{"controllers": {"c": {"states": []}}}"#,
        r#"{"controllers": {"c": {"states": {"s": 1}}}}"#,
        r#"{"controllers": {"c": {"states": {"s": {"animations": {}}}}}}"#,
        r#"{"controllers": {"c": {"states": {"s": {"animations": [1]}}}}}"#,
        r#"{"controllers": {"c": {"states": {"s": {"animations": [{}]}}}}"#,
        r#"{"controllers": {"c": {"states": {"s": {"animations": [{"name": "a", "speed": []}]}}}}}"#,
        r#"{"controllers": {"c": {"states": {"s": {"animations": [{"name": "a", "weight": "1 +"}]}}}}}"#,
        r#"{"controllers": {"c": {"states": {"s": {"transitions": {}}}}}}"#,
        r#"{"controllers": {"c": {"states": {"s": {"transitions": [1]}}}}}"#,
        r#"{"controllers": {"c": {"states": {"s": {"transitions": [{}]}}}}"#,
        r#"{"controllers": {"c": {"states": {"s": {"transitions": [{"to": "missing", "when": "1"}]}}}}}"#,
        r#"{"controllers": {"c": {"states": {"s": {"transitions": [{"to": "s"}]}}}}}"#,
        r#"{"controllers": {"c": {"states": {"s": {"transitions": [{"to": "s", "when": []}]}}}}}"#,
        r#"{"controllers": {"c": {"states": {"s": {"transitions": [{"to": "s", "when": "1 +"}]}}}}}"#,
        r#"{"controllers": {"c": {"states": {"s": {"transitions": [{"to": "s", "when": "1", "blend": -1}]}}}}}"#,
        r#"{"controllers": {"c": {"states": {"s": {"transitions": [{"to": "s", "when": "1", "blend": "0.1"}]}}}}}"#,
        r#"{"controllers": {"c": {"states": {"s": {"inactive": 1}}}}}"#,
        r#"{"controllers": {"c": {"states": {"s": {"blend": -0.1}}}}}"#,
        r#"{"controllers": {"c": {"states": {"s": {"blend_transition": "0.1"}}}}}"#,
        r#"{"controllers": {"c": {"states": {"s": {"speed": {}}}}}}"#,
        r#"{"controllers": {"c": {"states": {"s": {"on_enter": []}}}}}"#,
        r#"{"controllers": {"c": {"states": {"s": {"on_enter": {"v.a + b": "1"}}}}}}"#,
        r#"{"controllers": {"c": {"states": {"s": {"on_enter": {"v.a": []}}}}}}"#,
        r#"{"controllers": {"c": {"states": {"s": {"on_enter": {"v.a": "1 +"}}}}}}"#,
    ];
    for raw in cases {
        let mut syms = Symbols::new(&[]);
        let e = within_cap(|| parse_controllers("c.json", raw, &mut syms)).unwrap_err();
        assert_eq!(e.file, "c.json", "{raw}: {e}");
        assert!(!e.message.is_empty(), "{raw}");
    }
    let mut syms = Symbols::new(&[]);
    let e = within_cap(|| parse_controllers("c.json", r#"{"controllers":{"c":{"weight":"1 +"}}}"#, &mut syms)).unwrap_err();
    assert_eq!(e.file, "c.json");
    assert_eq!(e.path, r#"controllers["c"].weight"#);
    assert!(e.column.is_some());
}

#[test]
fn limits_deep_nesting_huge_numbers_and_duplicates() {
    let mut syms = Symbols::new(&[]);
    let mut layers = Map::new();
    for i in 0..=MAX_LAYERS {
        layers.insert(format!("c{i}"), json!({}));
    }
    let doc = serde_json::to_string(&json!({"controllers": layers})).unwrap();
    assert!(within_cap(|| parse_controllers("c.json", &doc, &mut syms)).is_err());

    let mut states = Map::new();
    for i in 0..=MAX_STATES {
        states.insert(format!("s{i}"), json!({}));
    }
    let doc = serde_json::to_string(&json!({"controllers": {"c": {"states": states}}})).unwrap();
    assert_err_path(within_cap(|| parse_controllers("c.json", &doc, &mut syms)), r#"controllers["c"].states"#);

    let anims = vec![json!("a"); MAX_STATE_ANIMS + 1];
    let doc = serde_json::to_string(&json!({"controllers": {"c": {"states": {"s": {"animations": anims}}}}})).unwrap();
    assert_err_path(
        within_cap(|| parse_controllers("c.json", &doc, &mut syms)),
        r#"controllers["c"].states["s"].animations"#,
    );

    let mut on_enter = Map::new();
    for i in 0..=MAX_VARIABLES {
        on_enter.insert(format!("v.x{i}"), json!(1));
    }
    let doc = serde_json::to_string(&json!({"controllers": {"c": {"states": {"s": {"on_enter": on_enter}}}}})).unwrap();
    assert_err_path(
        within_cap(|| parse_controllers("c.json", &doc, &mut syms)),
        r#"controllers["c"].states["s"].on_enter"#,
    );

    for num in ["1e400", "-1e400", "\"NaN\"", "\"Infinity\""] {
        let raw = format!(r#"{{"controllers":{{"c":{{"states":{{"s":{{"blend":{num}}}}}}}}}}}"#);
        assert!(within_cap(|| parse_controllers("c.json", &raw, &mut syms)).is_err(), "{num}");
    }

    let deep_arr = format!("{}{}", "[".repeat(3000), "]".repeat(3000));
    let e = within_cap(|| parse_controllers("c.json", &deep_arr, &mut syms)).unwrap_err();
    assert_eq!((e.file.as_str(), e.path.as_str()), ("c.json", "json"));
    let deep_obj = format!("{}1{}", "{\"a\":".repeat(3000), "}".repeat(3000));
    let e = within_cap(|| parse_controllers("c.json", &deep_obj, &mut syms)).unwrap_err();
    assert_eq!((e.file.as_str(), e.path.as_str()), ("c.json", "json"));

    let dup = r#"{"controllers":{"c":{"states":{"s":{},"s":{"blend":0.5}}}}}"#;
    let p = within_cap(|| parse_controllers("c.json", dup, &mut syms)).unwrap();
    assert_eq!(p.controllers.len(), 1);
}

fn assert_err_path<T: std::fmt::Debug>(r: Result<T, horizon_mobmodel::anim::LoadError>, prefix: &str) {
    let e = r.unwrap_err();
    assert_eq!(e.file, "c.json");
    assert!(e.path.starts_with(prefix), "expected path starting with {prefix}, got {e}");
    assert!(!e.message.is_empty());
}

#[test]
fn huge_f64_1e300_is_rejected() {
    let mut syms = Symbols::new(&[]);
    assert_err_path(
        within_cap(|| parse_controllers("c.json", r#"{"controllers":{"c":{"weight":1e300}}}"#, &mut syms)),
        r#"controllers["c"].weight"#,
    );
    assert_err_path(
        within_cap(|| parse_controllers("c.json", r#"{"controllers":{"c":{"states":{"s":{"blend":1e300}}}}}"#, &mut syms)),
        r#"controllers["c"].states["s"].blend"#,
    );
    assert_err_path(
        within_cap(|| parse_controllers("c.json", r#"{"controllers":{"c":{"states":{"s":{"animations":[{"name":"a","weight":1e300}]}}}}}"#, &mut syms)),
        r#"controllers["c"].states["s"].animations[0].weight"#,
    );
}
