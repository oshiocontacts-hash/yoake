//! Fuzz-style tests: random mutations of the fixtures must never panic (errors are fine), and anything that loads must
//! bake into finite, in-range triangles. Seeds are fixed so a failure reproduces.

use horizon_mobmodel::skeleton::{bone_matrices, Pose};
use horizon_mobmodel::*;
use rand::rngs::StdRng;
use rand::{RngExt, SeedableRng};
use serde_json::{json, Value};

const FOX_BB: &[u8] = include_bytes!("fixtures/test_fox.bbmodel");
const FOX_GEO: &[u8] = include_bytes!("fixtures/test_fox.geo.json");
const OLD_GEO: &[u8] = include_bytes!("fixtures/cube_old.geo.json");

fn random_leaf(rng: &mut StdRng) -> Value {
    match rng.random_range(0..22) {
        0 => Value::Null,
        1 => json!(true),
        2 => json!(false),
        3 => json!(-1),
        4 => json!(0),
        5 => json!(1),
        6 => json!(0.5),
        7 => json!(90),
        8 => json!(1e30),
        9 => json!(-1e300),
        10 => json!(4097),
        11 => json!(u64::MAX),
        12 => json!("x"),
        13 => json!(""),
        14 => json!([]),
        15 => json!({}),
        16 => json!([1, 2]),
        17 => json!([1, 2, 3]),
        18 => json!([1, 2, 3, 4]),
        19 => json!(vec![0; 50]),
        20 => json!({"north": {"uv": [0, 0], "uv_size": [1, 1]}}),
        _ => json!([[0, 0, 0], [1, 1, 1], [2, 2, 2]]),
    }
}

fn mutate_string(s: &str, rng: &mut StdRng) -> String {
    let mut chars: Vec<char> = s.chars().collect();
    if chars.is_empty() {
        return "a".to_string();
    }
    match rng.random_range(0..4) {
        0 => {
            let i = rng.random_range(0..chars.len());
            chars[i] = char::from(rng.random_range(32u8..127));
        }
        1 => chars.truncate(rng.random_range(0..chars.len())),
        2 => {
            let i = rng.random_range(0..chars.len());
            chars.remove(i);
        }
        _ => chars.extend(std::iter::repeat_n('A', rng.random_range(1..8))),
    }
    chars.into_iter().collect()
}

/// Mutates one random node somewhere in the tree.
fn mutate(v: &mut Value, rng: &mut StdRng, depth: usize) {
    match v {
        Value::Object(map) if !map.is_empty() && depth < 40 => {
            let keys: Vec<String> = map.keys().cloned().collect();
            let k = keys[rng.random_range(0..keys.len())].clone();
            match rng.random_range(0..8) {
                0 => {
                    map.remove(&k);
                }
                1 => {
                    let leaf = random_leaf(rng);
                    map.insert(k, leaf);
                }
                2 => {
                    let copy = map[&k].clone();
                    map.insert(format!("{k}_2"), copy);
                }
                _ => mutate(map.get_mut(&k).unwrap(), rng, depth + 1),
            }
        }
        Value::Array(arr) if !arr.is_empty() && depth < 40 => {
            let i = rng.random_range(0..arr.len());
            match rng.random_range(0..8) {
                0 => {
                    arr.remove(i);
                }
                1 => arr[i] = random_leaf(rng),
                2 => {
                    let copy = arr[i].clone();
                    arr.push(copy);
                }
                3 => {
                    let j = rng.random_range(0..arr.len());
                    arr.swap(i, j);
                }
                _ => mutate(&mut arr[i], rng, depth + 1),
            }
        }
        Value::String(s) => *s = mutate_string(s, rng),
        Value::Number(n) => {
            let x = n.as_f64().unwrap_or(0.0);
            *v = match rng.random_range(0..5) {
                0 => json!(-x),
                1 => json!(x + 1.0),
                2 => json!(x * 1000.0),
                3 => json!(0),
                _ => json!(x / 3.0),
            };
        }
        Value::Bool(b) => *b = !*b,
        _ => *v = random_leaf(rng),
    }
}

fn check_loaded(m: &Model) {
    let baked = m.bake();
    assert_eq!(baked.bones.len(), m.skeleton.len());
    assert_eq!(m.parts.len(), m.skeleton.len());
    for b in &baked.bones {
        assert_eq!(b.indices.len() % 3, 0);
        assert!(b.indices.iter().all(|&i| (i as usize) < b.vertices.len()));
        for v in &b.vertices {
            assert!(v.position.iter().chain(&v.normal).chain(&v.uv).all(|x| x.is_finite()), "non finite vertex {v:?}");
        }
    }
    for (i, bone) in m.skeleton.bones.iter().enumerate() {
        if let Some(p) = bone.parent {
            assert!(p.index() < i, "parents come first");
        }
    }
    let mats = bone_matrices(&m.skeleton, &Pose::rest(&m.skeleton));
    assert_eq!(mats.len(), m.skeleton.len());
    assert!(m.skeleton.len() <= limits::MAX_BONES && m.element_count() <= limits::MAX_CUBES);
    assert!(m.locators.iter().all(|l| l.bone.index() < m.skeleton.len()));
}

fn load_both(name: &str, bytes: &[u8]) -> usize {
    let mut ok = 0;
    if let Ok(m) = load_geo_json(name, bytes) {
        check_loaded(&m);
        ok += 1;
    }
    if let Ok(m) = load_bbmodel(name, bytes) {
        check_loaded(&m);
        ok += 1;
    }
    ok
}

fn run_mutations(fixture: &[u8], seed: u64, rounds: usize) -> (usize, usize) {
    let base: Value = serde_json::from_slice(fixture).unwrap();
    let mut rng = StdRng::seed_from_u64(seed);
    let (mut loaded, mut total) = (0, 0);
    for _ in 0..rounds {
        let mut v = base.clone();
        for _ in 0..rng.random_range(1..4) {
            mutate(&mut v, &mut rng, 0);
        }
        let bytes = serde_json::to_vec(&v).unwrap();
        loaded += load_both("fuzz.json", &bytes);
        total += 2;
    }
    (loaded, total)
}

#[test]
fn mutated_fox_bbmodel_never_panics() {
    let (loaded, total) = run_mutations(FOX_BB, 0xB0B, 3000);
    assert!(loaded > 100 && loaded < total, "loaded {loaded} of {total}: the mutations should hit both outcomes");
}

#[test]
fn mutated_fox_geo_never_panics() {
    let (loaded, total) = run_mutations(FOX_GEO, 0x6E0, 3000);
    assert!(loaded > 100 && loaded < total, "loaded {loaded} of {total}");
}

#[test]
fn mutated_old_geo_never_panics() {
    let (loaded, total) = run_mutations(OLD_GEO, 0x01D, 2000);
    assert!(loaded > 50 && loaded < total, "loaded {loaded} of {total}");
}

#[test]
fn byte_level_corruption_never_panics() {
    let mut rng = StdRng::seed_from_u64(0xFEED);
    for fixture in [FOX_BB, FOX_GEO, OLD_GEO] {
        for _ in 0..600 {
            let mut bytes = fixture.to_vec();
            match rng.random_range(0..4) {
                0 => bytes.truncate(rng.random_range(0..bytes.len())),
                1 => {
                    for _ in 0..rng.random_range(1..8) {
                        let i = rng.random_range(0..bytes.len());
                        bytes[i] = rng.random();
                    }
                }
                2 => {
                    let i = rng.random_range(0..bytes.len());
                    let j = (i + rng.random_range(0..200)).min(bytes.len());
                    bytes.drain(i..j);
                }
                _ => {
                    let i = rng.random_range(0..bytes.len());
                    bytes.insert(i, rng.random());
                }
            }
            load_both("bytes.json", &bytes);
        }
    }
}

#[test]
fn random_garbage_and_odd_documents_never_panic() {
    let mut rng = StdRng::seed_from_u64(0xDEAD);
    for _ in 0..500 {
        let len = rng.random_range(0..400);
        let bytes: Vec<u8> = (0..len).map(|_| rng.random()).collect();
        load_both("garbage.json", &bytes);
    }
    for text in [
        "",
        "null",
        "true",
        "0",
        "\"x\"",
        "[]",
        "{}",
        "[[]]",
        "{\"minecraft:geometry\": 5}",
        "{\"minecraft:geometry\": []}",
        "{\"minecraft:geometry\": [null]}",
        "{\"minecraft:geometry\": [{}]}",
        "{\"minecraft:geometry\": [{\"description\": {}}]}",
        "{\"geometry.x\": null}",
        "{\"geometry.x\": {\"bones\": 3}}",
        "{\"outliner\": 1}",
        "{\"outliner\": [null]}",
        "{\"elements\": {}}",
        "{\"textures\": [1]}",
        "{\"textures\": [{\"source\": 5}]}",
        "{\"textures\": [{\"source\": \"data:,\"}]}",
        "{\"resolution\": []}",
        "{\"resolution\": {\"width\": 1e300, \"height\": -3}}",
    ] {
        load_both("odd.json", text.as_bytes());
    }
}

#[test]
fn hostile_sizes_are_rejected_not_allocated() {
    // Huge mesh arrays and a gigantic PNG header must fail cleanly and quickly.
    let positions: Vec<Value> = vec![json!([0, 0, 0]); 70_000];
    let doc = json!({"format_version": "1.12.0", "minecraft:geometry": [{
        "description": {"identifier": "g", "texture_width": 16, "texture_height": 16},
        "bones": [{"name": "b", "poly_mesh": {"positions": positions, "uvs": [], "polys": []}}]}]});
    assert!(load_geo_json("big.geo.json", &serde_json::to_vec(&doc).unwrap()).is_err());

    // A PNG whose IHDR claims 60000 x 60000.
    let mut png = vec![0x89, b'P', b'N', b'G', 0x0D, 0x0A, 0x1A, 0x0A, 0, 0, 0, 13, b'I', b'H', b'D', b'R'];
    png.extend_from_slice(&60000u32.to_be_bytes());
    png.extend_from_slice(&60000u32.to_be_bytes());
    png.extend_from_slice(&[8, 6, 0, 0, 0, 0, 0, 0, 0]);
    use base64::Engine;
    let b64 = base64::engine::general_purpose::STANDARD.encode(&png);
    let doc = json!({"resolution": {"width": 16, "height": 16}, "elements": [], "outliner": [],
                     "textures": [{"name": "t", "source": format!("data:image/png;base64,{b64}")}]});
    let e = load_bbmodel("png.bbmodel", &serde_json::to_vec(&doc).unwrap()).unwrap_err();
    assert!(e.path == "textures[0].source", "{e}");
}
