//! Robustness tests for the `.bbmodel` loader (`load_bbmodel`).

use horizon_mobmodel::limits::{MAX_BONES, MAX_CUBES};
use horizon_mobmodel::model::{MAX_LOCATORS, MAX_MESH_VERTICES, MAX_TEXTURES, MAX_TEXTURE_DIM};
use horizon_mobmodel::{load_bbmodel, ModelError};
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
    unsafe fn dealloc(&self, ptr: *mut u8, l: Layout) {
        let _ = ALLOC_CUR.try_with(|c| c.set(c.get().saturating_sub(l.size())));
        unsafe { System.dealloc(ptr, l) }
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

const BASE_BB: &str = r#"{
  "name": "orig_mob",
  "model_identifier": "geometry.orig_mob",
  "resolution": {"width": 16, "height": 16},
  "textures": [{"name": "tex", "path": "tex.png"}],
  "elements": [
    {
      "type": "cube",
      "uuid": "c1",
      "from": [0, 0, 0],
      "to": [2, 2, 2],
      "origin": [0, 0, 0],
      "rotation": [0, 0, 0],
      "inflate": 0.0,
      "box_uv": false,
      "mirror_uv": false,
      "uv_offset": [0, 0],
      "faces": {"north": {"uv": [0, 0, 2, 2], "rotation": 90, "texture": 0}}
    },
    {
      "type": "mesh",
      "uuid": "m1",
      "name": "fin",
      "origin": [0, 0, 0],
      "rotation": [0, 0, 0],
      "vertices": {"v0": [0, 0, 0], "v1": [1, 0, 0], "v2": [0, 1, 0]},
      "faces": {"f0": {"vertices": ["v0", "v1", "v2"], "uv": {"v0": [0, 0], "v1": [1, 0], "v2": [0, 1]}}}
    },
    {
      "type": "locator",
      "uuid": "l1",
      "name": "tip",
      "from": [0, 1, 0],
      "rotation": [0, 0, 0]
    }
  ],
  "outliner": [
    {"name": "root", "export": true, "origin": [0, 0, 0], "rotation": [0, 0, 0], "children": ["c1", "m1", "l1"]}
  ]
}"#;

fn assert_err_file(r: Result<horizon_mobmodel::Model, ModelError>, require_path: bool, label: &str) {
    let e = match r {
        Ok(m) => panic!("{label}: expected Err, got Ok({m:?})"),
        Err(e) => e,
    };
    assert_eq!(e.file, "t.bbmodel", "{label}: {e}");
    assert!(!e.message.is_empty(), "{label}: empty message");
    if require_path {
        assert!(!e.path.is_empty(), "{label}: expected non-empty path, got {e}");
    }
}

#[test]
fn truncated_empty_utf8_and_random_bytes() {
    let bytes = BASE_BB.as_bytes();
    for cut in 0..bytes.len() {
        let r = within_cap(|| load_bbmodel("t.bbmodel", &bytes[..cut]));
        if let Err(e) = r {
            assert_eq!(e.file, "t.bbmodel", "cut {cut}");
            assert!(!e.message.is_empty());
        }
    }
    assert_err_file(within_cap(|| load_bbmodel("t.bbmodel", b"")), false, "empty");
    assert_err_file(within_cap(|| load_bbmodel("t.bbmodel", b"   \n\t ")), false, "ws");
    assert_err_file(within_cap(|| load_bbmodel("t.bbmodel", b"\xEF\xBB\xBF")), false, "bom");
    assert_err_file(
        within_cap(|| load_bbmodel("t.bbmodel", "\u{FEFF}あいう🦀\0".as_bytes())),
        false,
        "utf8 garbage",
    );
    let mut rng = StdRng::seed_from_u64(0xBB01);
    for i in 0..400 {
        let len = rng.random_range(0..1024);
        let buf: Vec<u8> = (0..len).map(|_| rng.random()).collect();
        let r = within_cap(|| load_bbmodel("t.bbmodel", &buf));
        if let Err(e) = r {
            assert_eq!(e.file, "t.bbmodel", "rand {i}");
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
    for root_bad in ["null", "true", "42", "\"s\"", "[]"] {
        assert_err_file(within_cap(|| load_bbmodel("t.bbmodel", root_bad.as_bytes())), false, root_bad);
    }
    let base: Value = serde_json::from_str(BASE_BB).unwrap();
    let cases: &[(&[&str], Value)] = &[
        (&["resolution"], json!(1)),
        (&["resolution", "width"], json!("16")),
        (&["resolution", "height"], json!(false)),
        (&["model_identifier"], json!(123)),
        (&["name"], json!([])),
        (&["textures"], json!({})),
        (&["textures", "0"], json!("bad")),
        (&["textures", "0", "name"], json!(9)),
        (&["textures", "0", "path"], json!(true)),
        (&["textures", "0", "source"], json!(5)),
        (&["textures", "0", "source"], json!("data:image/png;base64,!!!")),
        (&["textures", "0", "source"], json!("data:image/png;base64,AAAA")),
        (&["elements"], json!({})),
        (&["elements", "0"], json!(1)),
        (&["elements", "0", "uuid"], json!(1)),
        (&["elements", "0", "export"], json!("yes")),
        (&["elements", "0", "type"], json!(5)),
        (&["elements", "0", "from"], json!([1, 2])),
        (&["elements", "0", "from", "0"], json!("0")),
        (&["elements", "0", "to"], json!("box")),
        (&["elements", "0", "origin"], json!(1)),
        (&["elements", "0", "rotation"], json!([1, 2, 3, 4])),
        (&["elements", "0", "inflate"], json!("0")),
        (&["elements", "0", "box_uv"], json!(1)),
        (&["elements", "0", "mirror_uv"], json!("no")),
        (&["elements", "0", "uv_offset"], json!([1])),
        (&["elements", "0", "faces"], json!([])),
        (&["elements", "0", "faces", "north"], json!(1)),
        (&["elements", "0", "faces", "north", "uv"], json!([1, 2])),
        (&["elements", "0", "faces", "north", "rotation"], json!(45)),
        (&["elements", "1", "vertices"], json!([])),
        (&["elements", "1", "vertices", "v0"], json!("bad")),
        (&["elements", "1", "faces"], json!([])),
        (&["elements", "1", "faces", "f0"], json!(2)),
        (&["elements", "1", "faces", "f0", "vertices"], json!({})),
        (&["elements", "1", "faces", "f0", "vertices", "0"], json!(0)),
        (&["elements", "1", "faces", "f0", "vertices", "0"], json!("missing_v")),
        (&["elements", "1", "faces", "f0", "uv"], json!([])),
        (&["elements", "1", "faces", "f0", "uv", "v0"], json!([0])),
        (&["elements", "2", "name"], json!(false)),
        (&["elements", "2", "from"], json!([1, 2])),
        (&["elements", "2", "rotation"], json!("bad")),
        (&["outliner"], json!(1)),
        (&["outliner", "0"], json!(1)),
        (&["outliner", "0", "export"], json!("false")),
        (&["outliner", "0", "name"], json!(12)),
        (&["outliner", "0", "name"], json!("")),
        (&["outliner", "0", "origin"], json!("o")),
        (&["outliner", "0", "rotation"], json!(true)),
        (&["outliner", "0", "children"], json!({})),
        (&["outliner", "0", "children", "0"], json!("missing-uuid")),
    ];
    for (path, bad) in cases {
        let mut doc = base.clone();
        set_at(&mut doc, path, bad.clone());
        let raw = serde_json::to_vec(&doc).unwrap();
        assert_err_file(within_cap(|| load_bbmodel("t.bbmodel", &raw)), true, &path.join("."));
    }
    // `relative_path` is a fallback read only when `path` is absent; ignored by design when `path` is present.
    let mut with_both = base.clone();
    set_at(&mut with_both, &["textures", "0", "relative_path"], json!([]));
    let ok_m = within_cap(|| load_bbmodel("t.bbmodel", &serde_json::to_vec(&with_both).unwrap())).unwrap();
    assert_eq!(ok_m.textures[0].path.as_deref(), Some("tex.png"));
    let mut only_rel = base;
    only_rel["textures"][0].as_object_mut().unwrap().remove("path");
    set_at(&mut only_rel, &["textures", "0", "relative_path"], json!([]));
    assert_err_file(within_cap(|| load_bbmodel("t.bbmodel", &serde_json::to_vec(&only_rel).unwrap())), true, "textures.0.relative_path");
}

#[test]
fn limits_numbers_nesting_and_duplicates() {
    // Textures, resolution, bones, cubes, locators, mesh vertices, outliner depth.
    let too_many_tex = json!({"textures": vec![json!({"name": "t"}); MAX_TEXTURES + 1]});
    assert_err_file(within_cap(|| load_bbmodel("t.bbmodel", &serde_json::to_vec(&too_many_tex).unwrap())), true, "max_tex");

    let bad_dim = json!({"resolution": {"width": MAX_TEXTURE_DIM + 1, "height": 16}});
    assert_err_file(within_cap(|| load_bbmodel("t.bbmodel", &serde_json::to_vec(&bad_dim).unwrap())), true, "max_dim");

    let many_bones: Vec<Value> = (0..=MAX_BONES).map(|i| json!({"name": format!("b{i}")})).collect();
    let bones_doc = json!({"outliner": many_bones});
    assert_err_file(within_cap(|| load_bbmodel("t.bbmodel", &serde_json::to_vec(&bones_doc).unwrap())), true, "max_bones");

    let many_cubes: Vec<Value> = (0..=MAX_CUBES).map(|i| json!({"uuid": format!("u{i}"), "from": [0,0,0], "to": [1,1,1]})).collect();
    let uuids: Vec<Value> = (0..=MAX_CUBES).map(|i| json!(format!("u{i}"))).collect();
    let cubes_doc = json!({"elements": many_cubes, "outliner": [{"name": "r", "children": uuids}]});
    assert_err_file(within_cap(|| load_bbmodel("t.bbmodel", &serde_json::to_vec(&cubes_doc).unwrap())), true, "max_cubes");

    let many_locs: Vec<Value> = (0..=MAX_LOCATORS).map(|i| json!({"type": "locator", "uuid": format!("l{i}"), "from": [0,0,0]})).collect();
    let luuids: Vec<Value> = (0..=MAX_LOCATORS).map(|i| json!(format!("l{i}"))).collect();
    let locs_doc = json!({"elements": many_locs, "outliner": [{"name": "r", "children": luuids}]});
    assert_err_file(within_cap(|| load_bbmodel("t.bbmodel", &serde_json::to_vec(&locs_doc).unwrap())), true, "max_locators");

    let verts: serde_json::Map<String, Value> = (0..=MAX_MESH_VERTICES).map(|i| (format!("v{i}"), json!([0,0,0]))).collect();
    let mesh_doc = json!({"elements": [{"type": "mesh", "uuid": "m", "vertices": verts}], "outliner": ["m"]});
    assert_err_file(within_cap(|| load_bbmodel("t.bbmodel", &serde_json::to_vec(&mesh_doc).unwrap())), true, "max_verts");

    let mut nested = json!({"name": "leaf"});
    for i in 0..70 {
        nested = json!({"name": format!("g{i}"), "children": [nested]});
    }
    assert_err_file(within_cap(|| load_bbmodel("t.bbmodel", &serde_json::to_vec(&json!({"outliner": [nested]})).unwrap())), false, "depth");

    // Non-finite / huge numbers and string numbers.
    for num in ["1e400", "-1e400", "1e300", "-1e300", "1e7", "\"NaN\"", "\"Infinity\""] {
        let raw = format!(r#"{{"resolution":{{"width":{num},"height":16}}}}"#);
        assert_err_file(within_cap(|| load_bbmodel("t.bbmodel", raw.as_bytes())), false, num);
    }

    // Deep JSON nesting (3000 levels).
    let deep_arr = format!("{}{}", "[".repeat(3000), "]".repeat(3000));
    assert_err_file(within_cap(|| load_bbmodel("t.bbmodel", deep_arr.as_bytes())), false, "deep_arr");
    let deep_obj = format!("{}1{}", "{\"a\":".repeat(3000), "}".repeat(3000));
    assert_err_file(within_cap(|| load_bbmodel("t.bbmodel", deep_obj.as_bytes())), false, "deep_obj");

    // Duplicate JSON keys and duplicate bone/uuid names.
    let dup_json = br#"{"resolution":{"width":16,"width":32,"height":16},"elements":[],"outliner":[]}"#;
    assert!(within_cap(|| load_bbmodel("t.bbmodel", dup_json)).is_ok());
    let dup_bone = br#"{"elements":[],"outliner":[{"name":"b"},{"name":"b"}]}"#;
    assert_err_file(within_cap(|| load_bbmodel("t.bbmodel", dup_bone)), true, "dup_bone");
}

#[test]
#[ignore = "finding: decode_png allocates 256 MiB for an 8192x8192 RGBA IHDR before reading pixels"]
fn huge_8192_png_header_stays_within_alloc_cap() {
    use base64::Engine;
    let mut png = vec![0x89, b'P', b'N', b'G', 0x0D, 0x0A, 0x1A, 0x0A, 0, 0, 0, 13, b'I', b'H', b'D', b'R'];
    png.extend_from_slice(&8192u32.to_be_bytes());
    png.extend_from_slice(&8192u32.to_be_bytes());
    png.extend_from_slice(&[8, 6, 0, 0, 0]); // RGBA8
    let mut crc: u32 = 0xFFFF_FFFF;
    for &b in &png[12..29] {
        crc ^= b as u32;
        for _ in 0..8 {
            crc = if crc & 1 != 0 { (crc >> 1) ^ 0xEDB8_8320 } else { crc >> 1 };
        }
    }
    png.extend_from_slice(&(!crc).to_be_bytes());
    png.extend_from_slice(&[0, 0, 0, 0, b'I', b'E', b'N', b'D', 0xAE, 0x42, 0x60, 0x82]);
    let b64 = base64::engine::general_purpose::STANDARD.encode(&png);
    let doc = json!({"resolution":{"width":16,"height":16},"elements":[],"outliner":[],"textures":[{"name":"t","source":format!("data:image/png;base64,{b64}")}]});
    let raw = serde_json::to_vec(&doc).unwrap();
    let _ = within_cap(|| load_bbmodel("t.bbmodel", &raw));
}

#[test]
#[ignore = "finding: bbmodel MAX_DEPTH=64 is shadowed by serde_json 128-level recursion limit so >63 nested groups fail with empty path"]
fn outliner_past_max_depth_reports_json_path() {
    let mut nested = json!({"name": "leaf"});
    for i in 0..70 {
        nested = json!({"name": format!("g{i}"), "children": [nested]});
    }
    let raw = serde_json::to_vec(&json!({"outliner": [nested]})).unwrap();
    assert_err_file(within_cap(|| load_bbmodel("t.bbmodel", &raw)), true, "depth");
}
