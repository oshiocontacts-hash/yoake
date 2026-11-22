use super::super::{CustomType, persist};
use super::{
    memory::{EntityMemory, MemoryDef, MemoryKind, MemoryValue},
    test_support::*,
};
use crate::mobs::{Damage, DamageKind, Mob, Species, Who, world::test_player};
use horizon_core::geometry::Vec3;
use horizon_mods::custom_mob::CustomMobFile;
use horizon_world::nbt::Tag;
use std::sync::Arc;

fn brain_mob(json_body: &str, at: Vec3) -> Mob {
    let full = format!(
        r#"{{"template":"yoake:mob","category":"creature",
            "size":{{"width":0.6,"height":1.0,"eye_height":0.9}},
            "attributes":{{"max_health":20,"movement_speed":0.25,"attack_damage":3,"follow_range":16}},
            "ai":{json_body}}}"#
    );
    let file = CustomMobFile::parse("demo:brain_mob", full.as_bytes()).unwrap();
    let t = Arc::new(CustomType::new(file, None).unwrap());
    let kind = Arc::new(crate::mobs::custom::build::mob_type(&t));
    let mut m = Mob::new(1, kind, at, 0.0);
    m.on_ground = at.y == 1.0;
    m
}

fn state_ref(m: &Mob) -> &crate::mobs::custom::CustomState {
    m.extra.custom.as_ref().expect("custom mob state")
}

#[test]
fn memory_bank_defaults_coercion_and_ttl_expiry() {
    let ai_json = r#"{
        "memories": {
            "flag": {"type": "bool", "default": true},
            "count": {"type": "i32", "default": 5, "ttl": 10},
            "note": {"type": "string", "ttl": 3},
            "pos": {"type": "vec3"},
            "block": {"type": "block_pos"}
        },
        "goals": []
    }"#;
    let m = brain_mob(ai_json, Vec3::new(0.5, 1.0, 0.5));
    let bank_init = &state_ref(&m).brain.memories;
    assert_eq!(bank_init.get("flag"), Some(&MemoryValue::Bool(true)));
    assert_eq!(bank_init.get("count"), Some(&MemoryValue::I32(5)));
    assert_eq!(bank_init.get("note"), None);

    let mut bank = bank_init.clone();
    bank.set("note".into(), MemoryValue::Str("hi".into()), Some(3));
    assert_eq!(bank.get("note"), Some(&MemoryValue::Str("hi".into())));

    // Stays for N - 1 tick boundaries, expires at tick N
    bank.forget_outdated();
    assert_eq!(bank.entry("note").unwrap().ttl, Some(2));
    bank.forget_outdated();
    assert_eq!(bank.entry("note").unwrap().ttl, Some(1));
    bank.forget_outdated();
    assert_eq!(bank.get("note"), None);
    // Permanent default remains untouched
    assert_eq!(bank.get("flag"), Some(&MemoryValue::Bool(true)));

    // ttl = 0 expires on the very next forget_outdated pass
    bank.set("pos".into(), MemoryValue::Vec3([1.0, 2.0, 3.0]), Some(0));
    bank.forget_outdated();
    assert_eq!(bank.get("pos"), None);

    // Coercion checks finite floats and truncates long strings
    let f_def = MemoryDef {
        name: "f".into(),
        kind: MemoryKind::F32,
        default: None,
        persistent: false,
        ttl: None,
    };
    assert_eq!(f_def.coerce(MemoryValue::F32(f32::NAN)), None);
    assert_eq!(f_def.coerce(MemoryValue::I32(1)), None);
    assert_eq!(f_def.coerce(MemoryValue::F32(2.5)), Some(MemoryValue::F32(2.5)));

    let s_def = MemoryDef {
        name: "s".into(),
        kind: MemoryKind::Str,
        default: None,
        persistent: false,
        ttl: None,
    };
    let huge = "a".repeat(horizon_mods::custom_mob::limits::MAX_MEMORY_STRING + 50);
    let Some(MemoryValue::Str(trimmed)) = s_def.coerce(MemoryValue::Str(huge)) else {
        panic!("expected trimmed string");
    };
    assert_eq!(trimmed.len(), horizon_mods::custom_mob::limits::MAX_MEMORY_STRING);
}

#[test]
fn sensor_nearest_players_respects_interval_range_visibility_and_creative() {
    let ai = r#"{
        "memories": {"near_player": {"type": "entity"}},
        "sensors": [
            {
                "type": "nearest_players",
                "memory": "near_player",
                "interval": 5,
                "range": 12.0,
                "must_see": true,
                "ignore_creative": true
            }
        ],
        "goals": []
    }"#;
    let mut world = flat();
    let mut m = brain_mob(ai, Vec3::new(0.5, 1.0, 0.5));
    let mut ctx = Ctx::new();

    // Tick 0: creative player is ignored
    let mut w = world.world();
    let mut creative = test_player(Vec3::new(4.5, 1.0, 0.5));
    creative.creative = true;
    creative.uuid = [1, 2, 3, 4];
    w.player = Some(creative);
    run_server(&mut m, &w, &mut ctx, 1);
    assert_eq!(state_ref(&m).brain.memories.get("near_player"), None);

    // Tick 1..4: interval = 5 does NOT re-scan yet even when survival player enters range
    let mut survival = test_player(Vec3::new(4.5, 1.0, 0.5));
    survival.uuid = [9, 8, 7, 6];
    w.player = Some(survival.clone());
    run_server(&mut m, &w, &mut ctx, 4);
    assert_eq!(state_ref(&m).brain.memories.get("near_player"), None, "sensor must wait for interval=5");

    // Tick 5: interval fires and sees survival player
    run_server(&mut m, &w, &mut ctx, 1);
    assert_eq!(
        state_ref(&m).brain.memories.get("near_player"),
        Some(&MemoryValue::Entity(EntityMemory::Player([9, 8, 7, 6])))
    );

    // Place solid stone wall at x = 2 blocking LOS -> on next interval (tick 10), sensor clears memory
    world.set_stone(2, 1, 0);
    world.set_stone(2, 2, 0);
    let mut w2 = world.world();
    w2.player = Some(survival);
    run_server(&mut m, &w2, &mut ctx, 5);
    assert_eq!(state_ref(&m).brain.memories.get("near_player"), None, "wall blocks must_see=true");
}

#[test]
fn sensor_nearest_living_hurt_by_blocks_and_light() {
    let ai = r#"{
        "memories": {
            "threat": {"type": "entity"},
            "hurt": {"type": "bool", "ttl": 4},
            "attacker": {"type": "entity"},
            "shelter": {"type": "block_pos"},
            "sky_light": {"type": "i32"},
            "block_light": {"type": "i32"}
        },
        "sensors": [
            {"type": "nearest_living", "memory": "threat", "types": ["demo:guard", "minecraft:zombie"], "range": 10.0, "must_see": false},
            {"type": "hurt_by", "memory": "hurt", "interval": 1},
            {"type": "hurt_by", "memory": "attacker", "interval": 1, "max_age": 3},
            {"type": "blocks", "memory": "shelter", "tag": "demo:shelters", "radius": 4, "vertical_radius": 0},
            {"type": "light", "memory": "sky_light", "kind": "sky"},
            {"type": "light", "memory": "block_light", "kind": "block"}
        ],
        "goals": []
    }"#;
    let mut world = flat();
    world.set_stone(3, 1, 0);
    world.set_tag("demo:shelters", &["minecraft:stone"]);
    world.set_light(0, 1, 0, 11, 7);

    let mut m = brain_mob(ai, Vec3::new(0.5, 1.0, 0.5));
    m.last_damage = Some((
        Damage {
            kind: DamageKind::MobAttack,
            attacker: Some(Who::Mob(55)),
            from: None,
        },
        0,
    ));

    let mut guard = seen(55, Species::Custom, Vec3::new(5.5, 1.0, 0.5));
    guard.custom = Some(std::sync::Arc::from("demo:guard"));
    let cow = seen(77, Species::Cow, Vec3::new(1.5, 1.0, 0.5));
    let others = [cow, guard];

    let w_base = world.world();
    let w = view(&w_base, &others, None);
    let mut ctx = Ctx::new();
    run_server(&mut m, &w, &mut ctx, 1);

    let bank = &state_ref(&m).brain.memories;
    assert_eq!(bank.get("threat"), Some(&MemoryValue::Entity(EntityMemory::Mob(55))), "ignores closer cow");
    assert_eq!(bank.get("hurt"), Some(&MemoryValue::Bool(true)));
    assert_eq!(bank.get("attacker"), Some(&MemoryValue::Entity(EntityMemory::Mob(55))));
    assert_eq!(bank.get("shelter"), Some(&MemoryValue::BlockPos([3, 1, 0])));
    assert_eq!(bank.get("sky_light"), Some(&MemoryValue::I32(11)));
    assert_eq!(bank.get("block_light"), Some(&MemoryValue::I32(7)));

    // After 4 ticks (at age > 3), both TTL=4 "hurt" and max_age=3 "attacker" are gone
    run_server(&mut m, &w, &mut ctx, 4);
    let bank2 = &state_ref(&m).brain.memories;
    assert_eq!(bank2.get("hurt"), None, "hurt TTL expired after 4 ticks");
    assert_eq!(bank2.get("attacker"), None, "attacker exceeded max_age=3");
}

#[test]
fn activity_switching_core_goals_and_schedule_work_end_to_end() {
    let ai = r#"{
        "memories": {
            "threat": {"type": "entity"},
            "food_nearby": {"type": "bool", "ttl": 3},
            "focus": {"type": "i32", "default": 5}
        },
        "sensors": [
            {"type": "nearest_players", "memory": "threat", "interval": 1, "range": 8.0, "must_see": false, "ignore_creative": true}
        ],
        "default_activity": "idle",
        "activities": {
            "vital": {
                "core": true
            },
            "alert": {
                "priority": 1,
                "when": [{"memory": "threat", "present": true}]
            },
            "forage": {
                "priority": 3,
                "when": [
                    {"memory": "food_nearby", "eq": true},
                    {"memory": "focus", "gte": 4, "lt": 10}
                ]
            },
            "work": {
                "priority": 5
            },
            "idle": {
                "priority": 8,
                "goals": [
                    {"type": "random_look", "priority": 5}
                ]
            }
        },
        "schedule": [
            {"time": 0, "activity": "work"},
            {"time": 13000, "activity": "idle"}
        ],
        "goals": [
            {"type": "flee", "priority": 0, "activity": "vital", "from": ["minecraft:zombie"], "distance": 12.0},
            {"type": "panic", "priority": 1, "activity": "alert", "speed": 1.3},
            {"type": "look_at_player", "priority": 2, "activity": "alert"},
            {"type": "wander", "priority": 3, "activity": "forage"},
            {"type": "wander", "priority": 4, "activity": "work"}
        ]
    }"#;
    let world = flat();
    let mut m = brain_mob(ai, Vec3::new(0.5, 1.0, 0.5));
    let mut ctx = Ctx::new();

    // Verify core goal runs whenever vital is active (always), while schedule picks work at day.
    // Run 2 server ticks so gravity settles `on_ground = true` after spawn before pathfinding.
    let mut w_day = world.world();
    w_day.day_time = 1000;
    run_server(&mut m, &w_day, &mut ctx, 2);
    assert_eq!(state_ref(&m).brain.active_activity.as_deref(), Some("work"), "day schedule picks work");
    assert!(state_ref(&m).brain.active_cores.iter().any(|s| s == "vital"));

    // With a zombie nearby while active_activity == "work", core goal `flee` (priority 0) starts.
    let zombie = seen(99, Species::Zombie, Vec3::new(3.5, 1.0, 0.5));
    let others = [zombie];
    let w_core = view(&w_day, &others, None);
    assert!(super::brain::is_active(&m, "vital"));
    assert!(
        select_until(&mut m, &w_core, &mut ctx, 30, |m| m.ai.goals.is_running("AvoidEntity")).is_some(),
        "core goal runs regardless of active activity"
    );

    // Setting `food_nearby` (ttl=3) switches to `forage` (priority 3 < schedule's 5)
    m.extra.custom.as_mut().unwrap().brain.memories.set(
        "food_nearby".into(),
        MemoryValue::Bool(true),
        Some(3),
    );
    // Re-evaluate directly on next tick (tick_pre ticks TTLs down 3 -> 2 -> kept)
    run_server(&mut m, &w_day, &mut ctx, 1);
    assert_eq!(state_ref(&m).brain.active_activity.as_deref(), Some("forage"));
    // Player enters within range 8 -> `alert` (priority 1) overrides `forage` and starts `LookAtPlayer`
    let mut p = test_player(Vec3::new(4.5, 1.0, 0.5));
    p.uuid = [1, 1, 1, 1];
    let mut w_alert = w_day.clone();
    w_alert.player = Some(p);
    run_server(&mut m, &w_alert, &mut ctx, 1);
    assert_eq!(state_ref(&m).brain.active_activity.as_deref(), Some("alert"));
    assert!(
        select_until(&mut m, &w_alert, &mut ctx, 60, |m| m.ai.goals.is_running("LookAtPlayer")).is_some(),
        "alert's LookAtPlayer should start"
    );

    // Night time with no player and expired food_nearby falls back to `idle` per schedule at 13000,
    // and LookAtPlayer from `alert` stops because `alert` is no longer active.
    let mut w_night = world.world();
    w_night.day_time = 14000;
    run_server(&mut m, &w_night, &mut ctx, 5);
    assert_eq!(state_ref(&m).brain.active_activity.as_deref(), Some("idle"));
    assert!(!m.ai.goals.is_running("LookAtPlayer"));

    // Wrap-around schedule check: day_time = 24500 behaves as 500 -> "work"
    let mut w_wrap = world.world();
    w_wrap.day_time = 24500;
    run_server(&mut m, &w_wrap, &mut ctx, 1);
    assert_eq!(state_ref(&m).brain.active_activity.as_deref(), Some("work"));
}

#[test]
fn brainless_custom_mob_keeps_existing_goals_untouched() {
    let ai = r#"{
        "goals": [
            {"type": "wander", "priority": 1},
            {"type": "random_look", "priority": 2}
        ]
    }"#;
    let world = flat();
    let mut m = mob(ai, Vec3::new(0.5, 1.0, 0.5));
    assert!(m.kind.custom.as_ref().unwrap().brain.is_none());
    let w = world.world();
    let mut ctx = Ctx::new();
    run_server(&mut m, &w, &mut ctx, 5);
    assert!(state_ref(&m).brain.active_activity.is_none());
}

#[test]
fn persistent_memories_round_trip_through_nbt_and_preserve_unknown_and_drop_expired() {
    let ai = r#"{
        "memories": {
            "b": {"type": "bool", "persistent": true},
            "n": {"type": "i32", "persistent": true},
            "f": {"type": "f32", "persistent": true},
            "s": {"type": "string", "persistent": true},
            "v": {"type": "vec3", "persistent": true},
            "bp": {"type": "block_pos", "persistent": true},
            "emob": {"type": "entity", "persistent": true},
            "eplayer": {"type": "entity", "persistent": true},
            "ephemeral": {"type": "bool", "persistent": false},
            "default_on": {"type": "i32", "default": 9, "persistent": true}
        },
        "goals": []
    }"#;
    let mut m = brain_mob(ai, Vec3::new(0.5, 1.0, 0.5));
    let bank = &mut m.extra.custom.as_mut().unwrap().brain.memories;
    bank.set("b".into(), MemoryValue::Bool(true), None);
    bank.set("n".into(), MemoryValue::I32(-42), Some(120));
    bank.set("f".into(), MemoryValue::F32(3.25), Some(40));
    bank.set("s".into(), MemoryValue::Str("home".into()), None);
    bank.set("v".into(), MemoryValue::Vec3([1.5, 64.0, -2.5]), Some(15));
    bank.set("bp".into(), MemoryValue::BlockPos([10, 65, -3]), None);
    bank.set("emob".into(), MemoryValue::Entity(EntityMemory::Mob(7)), Some(10));
    bank.set(
        "eplayer".into(),
        MemoryValue::Entity(EntityMemory::Player([1, 2, 3, 4])),
        None,
    );
    bank.set("ephemeral".into(), MemoryValue::Bool(true), None);

    let mut tag = Tag::Compound(Default::default());
    persist::write(&mut tag, &m);

    // Inject an unknown key and an expired key in yoake.memories before reading back
    let Tag::Compound(root) = &mut tag else { unreachable!() };
    if let Some(Tag::Compound(yf)) = root.get_mut(&"yoake".into())
        && let Some(Tag::Compound(mems)) = yf.get_mut(&"memories".into())
    {
        mems.insert(
            "future_mod_key".into(),
            Tag::compound([("t", Tag::String("i32".into())), ("v", Tag::Int(99))]),
        );
        mems.insert(
            "b_expired_like".into(),
            Tag::compound([
                ("t", Tag::String("bool".into())),
                ("v", Tag::Byte(1)),
                ("ttl", Tag::Int(0)),
            ]),
        );
    }

    let mut loaded = brain_mob(ai, Vec3::new(0.5, 1.0, 0.5));
    persist::read(&mut loaded, &tag);
    let lb = &state_ref(&loaded).brain.memories;
    assert_eq!(lb.get("b"), Some(&MemoryValue::Bool(true)));
    assert_eq!(lb.get("n"), Some(&MemoryValue::I32(-42)));
    assert_eq!(lb.entry("n").unwrap().ttl, Some(120));
    assert_eq!(lb.get("f"), Some(&MemoryValue::F32(3.25)));
    assert_eq!(lb.get("s"), Some(&MemoryValue::Str("home".into())));
    assert_eq!(lb.get("v"), Some(&MemoryValue::Vec3([1.5, 64.0, -2.5])));
    assert_eq!(lb.get("bp"), Some(&MemoryValue::BlockPos([10, 65, -3])));
    assert_eq!(lb.get("emob"), Some(&MemoryValue::Entity(EntityMemory::Mob(7))));
    assert_eq!(
        lb.get("eplayer"),
        Some(&MemoryValue::Entity(EntityMemory::Player([1, 2, 3, 4])))
    );
    assert_eq!(lb.get("ephemeral"), None, "non-persistent memory is not saved");
    assert_eq!(lb.get("default_on"), Some(&MemoryValue::I32(9)));

    // Re-writing preserves unknown keys while writing current entries
    let mut tag2 = tag.clone();
    persist::write(&mut tag2, &loaded);
    let kept_unknown = tag2
        .get("yoake")
        .and_then(|y| y.get("memories"))
        .and_then(|mems| mems.get("future_mod_key"))
        .is_some();
    assert!(kept_unknown, "unknown memory keys must be preserved across saves");
}

#[test]
fn stale_hurt_by_damage_beyond_max_age_is_ignored_and_equal_priority_when_beats_schedule() {
    let ai = r#"{
        "memories": {
            "flag": {"type": "bool"},
            "attacker": {"type": "entity"}
        },
        "sensors": [
            {"type": "hurt_by", "memory": "attacker", "interval": 1, "max_age": 3}
        ],
        "activities": {
            "react": {"priority": 5, "when": [{"memory": "flag", "eq": true}]},
            "work": {"priority": 5}
        },
        "schedule": [
            {"time": 0, "activity": "work"}
        ],
        "goals": []
    }"#;
    let world = flat();
    let w = world.world();
    let mut ctx = Ctx::new();
    let mut m = brain_mob(ai, Vec3::new(0.5, 1.0, 0.5));
    m.age = 20;
    m.last_damage = Some((
        Damage {
            kind: DamageKind::MobAttack,
            attacker: Some(Who::Mob(55)),
            from: None,
        },
        10,
    ));
    run_server(&mut m, &w, &mut ctx, 1);
    assert_eq!(state_ref(&m).brain.memories.get("attacker"), None, "damage older than max_age=3 must not populate memory");
    assert_eq!(state_ref(&m).brain.active_activity.as_deref(), Some("work"));

    m.extra.custom.as_mut().unwrap().brain.memories.set("flag".into(), MemoryValue::Bool(true), None);
    run_server(&mut m, &w, &mut ctx, 1);
    assert_eq!(state_ref(&m).brain.active_activity.as_deref(), Some("react"), "predicate-gated activity beats scheduled-only at equal priority");
}
