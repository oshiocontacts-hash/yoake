//! Robustness tests for the `.geo.json` loader (`load_geo_json`, `load_geo_json_with`).

use horizon_mobmodel::limits::{MAX_BONES, MAX_CUBES};
use horizon_mobmodel::model::{MAX_LOCATORS, MAX_MESH_VERTICES, MAX_TEXTURE_DIM};
use horizon_mobmodel::{load_geo_json, load_geo_json_with, ModelError};
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

const BASE_GEO_112: &str = r#"{
  "format_version": "1.12.0",
  "minecraft:geometry": [{
    "description": {
      "identifier": "geometry.orig",
      "texture_width": 16,
      "texture_height": 16,
      "visible_bounds_width": 2,
      "visible_bounds_height": 2,
      "visible_bounds_offset": [0, 1, 0]
    },
    "bones": [{
      "name": "body",
      "pivot": [0, 0, 0],
      "rotation": [0, 0, 0],
      "mirror": false,
      "inflate": 0.0,
      "neverRender": false,
      "cubes": [{
        "origin": [-1, 0, -1],
        "size": [2, 2, 2],
        "pivot": [0, 0, 0],
        "rotation": [0, 0, 0],
        "inflate": 0.0,
        "mirror": false,
        "uv": {"north": {"uv": [0, 0], "uv_size": [2, 2], "uv_rotation": 90}}
      }],
      "poly_mesh": {
        "normalized_uvs": false,
        "positions": [[0, 0, 0], [1, 0, 0], [0, 1, 0]],
        "normals": [[0, 0, 1]],
        "uvs": [[0, 0], [1, 0], [0, 1]],
        "polys": [[[0, 0, 0], [1, 0, 1], [2, 0, 2]]]
      },
      "locators": {
        "nose": {"offset": [0, 1, -1], "rotation": [0, 0, 0]}
      }
    }]
  }]
}"#;

const BASE_GEO_18: &str = r#"{
  "format_version": "1.8.0",
  "geometry.orig": {
    "texturewidth": 16,
    "textureheight": 16,
    "bones": [{"name": "body", "cubes": [{"origin": [0, 0, 0], "size": [1, 1, 1], "uv": [0, 0]}]}]
  }
}"#;

fn assert_err_file(r: Result<horizon_mobmodel::Model, ModelError>, require_path: bool, label: &str) {
    let e = r.unwrap_err();
    assert_eq!(e.file, "t.geo.json", "{label}: {e}");
    assert!(!e.message.is_empty(), "{label}: empty message");
    if require_path {
        assert!(!e.path.is_empty(), "{label}: expected non-empty path, got {e}");
    }
}

#[test]
fn truncated_empty_utf8_and_random_bytes() {
    for src in [BASE_GEO_112.as_bytes(), BASE_GEO_18.as_bytes()] {
        for cut in 0..src.len() {
            let r = within_cap(|| load_geo_json("t.geo.json", &src[..cut]));
            if let Err(e) = r {
                assert_eq!(e.file, "t.geo.json", "cut {cut}");
                assert!(!e.message.is_empty());
            }
        }
    }
    assert_err_file(within_cap(|| load_geo_json("t.geo.json", b"")), false, "empty");
    assert_err_file(within_cap(|| load_geo_json("t.geo.json", b"  \r\n")), false, "ws");
    assert_err_file(within_cap(|| load_geo_json("t.geo.json", b"\xEF\xBB\xBF")), false, "bom");
    assert_err_file(within_cap(|| load_geo_json("t.geo.json", "\u{FEFF}あ🦀\0".as_bytes())), false, "utf8");
    let mut rng = StdRng::seed_from_u64(0x6E01);
    for i in 0..400 {
        let len = rng.random_range(0..1024);
        let buf: Vec<u8> = (0..len).map(|_| rng.random()).collect();
        if let Err(e) = within_cap(|| load_geo_json("t.geo.json", &buf)) {
            assert_eq!(e.file, "t.geo.json", "rand {i}");
            assert!(!e.message.is_empty());
        }
    }
}

fn set_at(v: &mut Value, keys: &[&str], bad: Value) {
    let mut cur = v;
    for &k in &keys[..keys.len() - 1] {
        cur = if let Ok(idx) = k.parse::<usize>() { &mut cur[idx] } else { &mut cur[k] };
    }
    let last = keys[keys.len() - 1];
    if let Ok(idx) = last.parse::<usize>() {
        cur[idx] = bad;
    } else {
        cur[last] = bad;
    }
}

#[test]
fn wrong_types_at_every_field_carry_path() {
    let base: Value = serde_json::from_str(BASE_GEO_112).unwrap();
    let cases: &[(&[&str], Value)] = &[
        (&["minecraft:geometry"], json!(5)),
        (&["minecraft:geometry"], json!([])),
        (&["minecraft:geometry", "0"], json!(null)),
        (&["minecraft:geometry", "0", "description"], json!(1)),
        (&["minecraft:geometry", "0", "description", "identifier"], json!(1)),
        (&["minecraft:geometry", "0", "description", "texture_width"], json!("16")),
        (&["minecraft:geometry", "0", "description", "texture_height"], json!(0)),
        (&["minecraft:geometry", "0", "description", "visible_bounds_width"], json!("w")),
        (&["minecraft:geometry", "0", "description", "visible_bounds_height"], json!(false)),
        (&["minecraft:geometry", "0", "description", "visible_bounds_offset"], json!([1, 2])),
        (&["minecraft:geometry", "0", "bones"], json!({})),
        (&["minecraft:geometry", "0", "bones", "0"], json!(1)),
        (&["minecraft:geometry", "0", "bones", "0", "name"], json!(1)),
        (&["minecraft:geometry", "0", "bones", "0", "name"], json!("")),
        (&["minecraft:geometry", "0", "bones", "0", "parent"], json!(1)),
        (&["minecraft:geometry", "0", "bones", "0", "pivot"], json!("p")),
        (&["minecraft:geometry", "0", "bones", "0", "rotation"], json!([1])),
        (&["minecraft:geometry", "0", "bones", "0", "mirror"], json!(1)),
        (&["minecraft:geometry", "0", "bones", "0", "inflate"], json!("0")),
        (&["minecraft:geometry", "0", "bones", "0", "neverRender"], json!("false")),
        (&["minecraft:geometry", "0", "bones", "0", "cubes"], json!({})),
        (&["minecraft:geometry", "0", "bones", "0", "cubes", "0"], json!(1)),
        (&["minecraft:geometry", "0", "bones", "0", "cubes", "0", "origin"], json!(1)),
        (&["minecraft:geometry", "0", "bones", "0", "cubes", "0", "size"], json!([1, 2])),
        (&["minecraft:geometry", "0", "bones", "0", "cubes", "0", "pivot"], json!("x")),
        (&["minecraft:geometry", "0", "bones", "0", "cubes", "0", "rotation"], json!(true)),
        (&["minecraft:geometry", "0", "bones", "0", "cubes", "0", "inflate"], json!([])),
        (&["minecraft:geometry", "0", "bones", "0", "cubes", "0", "mirror"], json!("y")),
        (&["minecraft:geometry", "0", "bones", "0", "cubes", "0", "uv"], json!(5)),
        (&["minecraft:geometry", "0", "bones", "0", "cubes", "0", "uv", "north"], json!(1)),
        (&["minecraft:geometry", "0", "bones", "0", "cubes", "0", "uv", "north", "uv"], json!([1])),
        (&["minecraft:geometry", "0", "bones", "0", "cubes", "0", "uv", "north", "uv_size"], json!(1)),
        (&["minecraft:geometry", "0", "bones", "0", "cubes", "0", "uv", "north", "uv_rotation"], json!(45)),
        (&["minecraft:geometry", "0", "bones", "0", "poly_mesh"], json!([])),
        (&["minecraft:geometry", "0", "bones", "0", "poly_mesh", "normalized_uvs"], json!(1)),
        (&["minecraft:geometry", "0", "bones", "0", "poly_mesh", "positions"], json!({})),
        (&["minecraft:geometry", "0", "bones", "0", "poly_mesh", "positions", "0"], json!(1)),
        (&["minecraft:geometry", "0", "bones", "0", "poly_mesh", "normals"], json!(1)),
        (&["minecraft:geometry", "0", "bones", "0", "poly_mesh", "normals", "0"], json!("n")),
        (&["minecraft:geometry", "0", "bones", "0", "poly_mesh", "uvs"], json!(1)),
        (&["minecraft:geometry", "0", "bones", "0", "poly_mesh", "uvs", "0"], json!([0])),
        (&["minecraft:geometry", "0", "bones", "0", "poly_mesh", "polys"], json!("triangulated")),
        (&["minecraft:geometry", "0", "bones", "0", "poly_mesh", "polys", "0"], json!([[0, 0, 0]])),
        (&["minecraft:geometry", "0", "bones", "0", "poly_mesh", "polys", "0", "0"], json!([0, 0])),
        (&["minecraft:geometry", "0", "bones", "0", "poly_mesh", "polys", "0", "0", "0"], json!(99)),
        (&["minecraft:geometry", "0", "bones", "0", "locators"], json!([])),
        (&["minecraft:geometry", "0", "bones", "0", "locators", "nose"], json!(1)),
        (&["minecraft:geometry", "0", "bones", "0", "locators", "nose", "offset"], json!([0])),
        (&["minecraft:geometry", "0", "bones", "0", "locators", "nose", "rotation"], json!("r")),
    ];
    for (path, bad) in cases {
        let mut doc = base.clone();
        set_at(&mut doc, path, bad.clone());
        let raw = serde_json::to_vec(&doc).unwrap();
        assert_err_file(within_cap(|| load_geo_json("t.geo.json", &raw)), true, &path.join("."));
    }
    let old_bad = br#"{"geometry.orig":{"texturewidth":"bad","bones":[]}}"#;
    assert_err_file(within_cap(|| load_geo_json("t.geo.json", old_bad)), true, "1.8 texturewidth");
    assert_err_file(
        within_cap(|| load_geo_json_with("t.geo.json", BASE_GEO_112.as_bytes(), Some("missing"))),
        true,
        "missing id 1.12",
    );
    assert_err_file(
        within_cap(|| load_geo_json_with("t.geo.json", BASE_GEO_18.as_bytes(), Some("geometry.missing"))),
        false,
        "missing id 1.8",
    );
}

#[test]
fn limits_cycles_numbers_deep_nesting_and_duplicates() {
    let many_bones: Vec<Value> = (0..=MAX_BONES).map(|i| json!({"name": format!("b{i}")})).collect();
    let doc = json!({"format_version":"1.12.0","minecraft:geometry":[{"description":{"identifier":"g"},"bones":many_bones}]});
    assert_err_file(within_cap(|| load_geo_json("t.geo.json", &serde_json::to_vec(&doc).unwrap())), true, "max_bones");

    let many_cubes: Vec<Value> = (0..=MAX_CUBES).map(|_| json!({"origin":[0,0,0],"size":[1,1,1],"uv":[0,0]})).collect();
    let doc = json!({"format_version":"1.12.0","minecraft:geometry":[{"description":{"identifier":"g"},"bones":[{"name":"b","cubes":many_cubes}]}]});
    assert_err_file(within_cap(|| load_geo_json("t.geo.json", &serde_json::to_vec(&doc).unwrap())), true, "max_cubes");

    let locs: serde_json::Map<String, Value> = (0..=MAX_LOCATORS).map(|i| (format!("l{i}"), json!([0,0,0]))).collect();
    let doc = json!({"format_version":"1.12.0","minecraft:geometry":[{"description":{"identifier":"g"},"bones":[{"name":"b","locators":locs}]}]});
    assert_err_file(within_cap(|| load_geo_json("t.geo.json", &serde_json::to_vec(&doc).unwrap())), true, "max_locs");

    let bad_uvs = vec![json!([0,0]); MAX_MESH_VERTICES + 1];
    let doc = json!({"format_version":"1.12.0","minecraft:geometry":[{"description":{"identifier":"g"},"bones":[{"name":"b","poly_mesh":{"positions":[],"uvs":bad_uvs,"polys":[]}}]}]});
    assert_err_file(within_cap(|| load_geo_json("t.geo.json", &serde_json::to_vec(&doc).unwrap())), true, "max_uvs");

    let bad_dim = json!({"format_version":"1.12.0","minecraft:geometry":[{"description":{"identifier":"g","texture_width":MAX_TEXTURE_DIM+1,"texture_height":16},"bones":[]}]});
    assert_err_file(within_cap(|| load_geo_json("t.geo.json", &serde_json::to_vec(&bad_dim).unwrap())), true, "max_dim");

    let cycle = json!({"format_version":"1.12.0","minecraft:geometry":[{"description":{"identifier":"g"},"bones":[{"name":"a","parent":"b"},{"name":"b","parent":"a"}]}]});
    assert_err_file(within_cap(|| load_geo_json("t.geo.json", &serde_json::to_vec(&cycle).unwrap())), true, "cycle");

    for num in ["1e400", "-1e400", "1e300", "-1e300", "1e7", "\"NaN\"", "\"Infinity\""] {
        let raw = format!(r#"{{"format_version":"1.12.0","minecraft:geometry":[{{"description":{{"identifier":"g","texture_width":{num}}},"bones":[]}}]}}"#);
        assert_err_file(within_cap(|| load_geo_json("t.geo.json", raw.as_bytes())), false, num);
    }

    let deep_arr = format!("{}{}", "[".repeat(3000), "]".repeat(3000));
    assert_err_file(within_cap(|| load_geo_json("t.geo.json", deep_arr.as_bytes())), false, "deep_arr");
    let deep_obj = format!("{}1{}", "{\"a\":".repeat(3000), "}".repeat(3000));
    assert_err_file(within_cap(|| load_geo_json("t.geo.json", deep_obj.as_bytes())), false, "deep_obj");

    let dup_bone = br#"{"format_version":"1.12.0","minecraft:geometry":[{"description":{"identifier":"g"},"bones":[{"name":"b"},{"name":"b"}]}]}"#;
    assert_err_file(within_cap(|| load_geo_json("t.geo.json", dup_bone)), true, "dup_bone");
    let dup_key = br#"{"format_version":"1.12.0","minecraft:geometry":[{"description":{"identifier":"g","identifier":"g2"},"bones":[]}]}"#;
    assert!(within_cap(|| load_geo_json("t.geo.json", dup_key)).is_ok());
}
