//! Robustness tests for typed animation events (`src/events.rs`).

use glam::Vec3;
use horizon_mobmodel::anim::{AnimEvent, Animation, AnimationLibrary, EventKind, LoopMode};
use horizon_mobmodel::events::limits::{
    MAX_EVENTS_PER_ANIMATION, MAX_EVENTS_PER_TYPE, MAX_ID_LEN, MAX_NAME_LEN, MAX_PARTICLES,
};
use horizon_mobmodel::events::{parse_particles, parse_server_events, MobEvents, ParticleTable};
use horizon_mobmodel::model::{Locator, ModelError};
use horizon_mobmodel::skeleton::BoneIndex;
use rand::rngs::StdRng;
use rand::{RngExt, SeedableRng};
use serde_json::{json, Map, Value};
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

fn assert_err<T>(r: Result<T, ModelError>, file: &str, label: &str) {
    let e = match r {
        Ok(_) => panic!("{label}: expected Err, got Ok"),
        Err(e) => e,
    };
    assert_eq!(e.file, file, "{label}: {e}");
    assert!(!e.path.is_empty(), "{label}: expected non-empty path, got {e}");
    assert!(!e.message.is_empty(), "{label}: empty message");
}

fn empty_anim(name: &str, events: Vec<AnimEvent>) -> Animation {
    Animation {
        name: name.into(),
        loop_mode: LoopMode::Once,
        length: 1.0,
        override_previous: false,
        anim_time_update: None,
        blend_weight: None,
        bones: Vec::new(),
        events,
    }
}

fn sample_lib() -> AnimationLibrary {
    let mut lib = AnimationLibrary::new();
    lib.add(empty_anim("mob.walk", vec![])).unwrap();
    lib.add(empty_anim("a.idle", vec![])).unwrap();
    lib.add(empty_anim("b.idle", vec![])).unwrap();
    lib
}

#[test]
fn particles_wrong_types_invalid_ids_nesting_and_limits() {
    for bad in [json!(null), json!(true), json!(123), json!("smoke"), json!([])] {
        assert_err(within_cap(|| parse_particles("type.json", "model.particles", &bad)), "type.json", "root");
    }

    let over_name = "a".repeat(MAX_NAME_LEN + 1);
    let over_id = "a".repeat(MAX_ID_LEN + 1);
    for (k, v, tag) in [
        ("", json!("smoke"), "empty name"),
        (over_name.as_str(), json!("smoke"), "overlong name"),
        ("bad\nname", json!("smoke"), "ctrl newline"),
        ("bad\0name", json!("smoke"), "ctrl nul"),
        ("dust", json!(null), "val null"),
        ("dust", json!(123), "val num"),
        ("dust", json!([]), "val arr"),
        ("dust", json!({}), "missing id"),
        ("dust", json!({"id": 123}), "non-str id"),
        ("dust", json!({"id": "minecraft:smoke", "extra": 1}), "extra key"),
        ("dust", json!(""), "empty id"),
        ("dust", json!("Upper"), "uppercase id"),
        ("dust", json!("a:b:c"), "two colons"),
        ("dust", json!(":smoke"), "leading colon"),
        ("dust", json!("smoke:"), "trailing colon"),
        ("dust", json!("has space"), "space in id"),
        ("dust", json!(over_id), "overlong id"),
    ] {
        let v = json!({k: v});
        assert_err(within_cap(|| parse_particles("type.json", "model.particles", &v)), "type.json", tag);
    }

    let mut too_many = Map::new();
    for i in 0..=MAX_PARTICLES {
        too_many.insert(format!("p{i}"), json!("smoke"));
    }
    assert_err(within_cap(|| parse_particles("type.json", "model.particles", &Value::Object(too_many))), "type.json", "max_particles");

    let mut deep = json!(1);
    for _ in 0..2000 {
        deep = json!([deep]);
    }
    assert_err(within_cap(|| parse_particles("type.json", "model.particles", &json!({"p": deep}))), "type.json", "deep");

    let dup_serde: Value = serde_json::from_str(r#"{"p":"smoke","p":"cloud"}"#).unwrap();
    let t = within_cap(|| parse_particles("type.json", "model.particles", &dup_serde)).unwrap();
    assert_eq!(t.resolve("p"), Some("minecraft:cloud"));
}

#[test]
fn server_events_wrong_types_and_unknown_anims() {
    let lib = sample_lib();
    for bad in [
        json!(null),
        json!(123),
        json!(1e300),
        json!("walk"),
        json!([123]),
        json!(["missing"]),
        json!(["idle"]), // ambiguous suffix across a.idle and b.idle
        json!({"walk": "true"}),
        json!({"walk": 1}),
        json!({"missing": true}),
    ] {
        assert_err(within_cap(|| parse_server_events("type.json", "model.server_events", &bad, &lib)), "type.json", &bad.to_string());
    }

    let huge = Value::Array(vec![json!("walk"); 5000]);
    let flags = within_cap(|| parse_server_events("type.json", "model.server_events", &huge, &lib)).unwrap();
    assert_eq!(flags, [true, false, false]);
}

#[test]
fn mob_events_build_errors_and_limits() {
    let locs = vec![Locator { name: "foot".into(), bone: BoneIndex(0), offset: Vec3::ZERO, rotation: Vec3::ZERO, null_object: false }];
    let pt = ParticleTable::new();

    let bad_events = [
        AnimEvent::new(0.1, EventKind::Timeline, "hit", Some("missing_loc".into())),
        AnimEvent::new(0.1, EventKind::Timeline, "", None),
        AnimEvent::new(0.1, EventKind::Timeline, "a".repeat(MAX_NAME_LEN + 1), None),
        AnimEvent::new(0.1, EventKind::Timeline, "bad\tname", None),
        AnimEvent::new(0.1, EventKind::Sound, "BAD_SOUND", None),
        AnimEvent::new(0.1, EventKind::Sound, "a:b:c", None),
        AnimEvent::new(0.1, EventKind::Particle, "unmapped_particle", None),
        AnimEvent::new(0.1, EventKind::Particle, "bad:id:2", None),
    ];
    for (i, ev) in bad_events.into_iter().enumerate() {
        let mut lib = AnimationLibrary::new();
        lib.add(empty_anim("a", vec![ev])).unwrap();
        assert_err(within_cap(|| MobEvents::build("anim.json", &lib, &locs, &pt, &[])), "anim.json", &format!("ev_{i}"));
    }

    // Over per-animation event limit.
    let mut lib = AnimationLibrary::new();
    let many_ev: Vec<AnimEvent> = (0..=MAX_EVENTS_PER_ANIMATION).map(|_| AnimEvent::new(0.1, EventKind::Timeline, "hit", None)).collect();
    lib.add(empty_anim("a", many_ev)).unwrap();
    assert_err(within_cap(|| MobEvents::build("anim.json", &lib, &locs, &pt, &[])), "anim.json", "max_per_anim");

    // Over per-type event limit.
    let mut lib2 = AnimationLibrary::new();
    for i in 0..=(MAX_EVENTS_PER_TYPE / 250) {
        let evs = vec![AnimEvent::new(0.1, EventKind::Timeline, "hit", None); 250];
        lib2.add(empty_anim(&format!("a{i}"), evs)).unwrap();
    }
    assert_err(within_cap(|| MobEvents::build("anim.json", &lib2, &locs, &pt, &[])), "anim.json", "max_per_type");

    // from_model_json checks particles and server_events when present, including explicit null.
    let lib3 = sample_lib();
    assert_err(within_cap(|| MobEvents::from_model_json("type.json", "anim.json", &json!({"particles": null}), &lib3, &locs)), "type.json", "null particles");
    assert_err(within_cap(|| MobEvents::from_model_json("type.json", "anim.json", &json!({"server_events": null}), &lib3, &locs)), "type.json", "null server_events");

    // Deterministic seeded invalid random strings for particle ids.
    let mut rng = StdRng::seed_from_u64(0xE7E2);
    for _ in 0..200 {
        let s: String = (0..20).map(|_| rng.random_range(b' '..=b'~') as char).collect();
        let _ = within_cap(|| parse_particles("type.json", "model.particles", &json!({"p": s})));
    }
}

#[test]
#[ignore = "finding: MobEvents::from_model_json silently ignores non-object model Value"]
fn from_model_json_rejects_non_object_model() {
    let lib = sample_lib();
    for bad in [json!(null), json!(123), json!([]), json!("bad")] {
        assert!(within_cap(|| MobEvents::from_model_json("type.json", "anim.json", &bad, &lib, &[])).is_err(), "{bad}");
    }
}
