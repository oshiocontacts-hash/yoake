//! Resource-free unit tests for F4M-11 custom-mob services, validation, command bus queuing,
//! texture management, and renderer stage no-op guards.

use std::sync::{Arc, Mutex};

#[cfg(feature = "yoake2-slice")]
use horizon_client::mesher::Vertex;
#[cfg(feature = "yoake2-slice")]
use horizon_client::mobs::custom::render_state::{DrawCtx, LodMode, MobFacts, MobRender, PoseCtx};
use horizon_client::mobs::{
    Mob,
    custom::{
        CustomType, SyncedValue, build,
        script::{self, Command, MobScriptValue, MobView},
        textures,
    },
};
use horizon_core::geometry::Vec3;
use horizon_mods::custom_mob::CustomMobFile;

use super::*;

static BUS_LOCK: Mutex<()> = Mutex::new(());

fn cinder_fox_type() -> Arc<CustomType> {
    let json = include_str!("../../../../../tools/fixtures/cinder_fox/content/data/sample_mobs/horizon/entity_type/cinder_fox.json");
    let file = CustomMobFile::parse("sample_mobs:cinder_fox", json.as_bytes()).unwrap();
    let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../tools/fixtures/cinder_fox/content");
    let model = file.load_model(&mut |p| std::fs::read(root.join(p)).map_err(|e| e.to_string())).unwrap();
    Arc::new(CustomType::new(file, Some(&model)).unwrap())
}

fn fox_mob(id: u32) -> Mob {
    let ty = cinder_fox_type();
    let kind = Arc::new(build::mob_type(&ty));
    let mut m = Mob::new(id, kind, Vec3::new(0.5, 1.0, 0.5), 0.0);
    build::register(&mut m);
    m.on_ground = true;
    m
}

struct Reset;
impl Drop for Reset {
    fn drop(&mut self) {
        script::set_runtime(None);
        script::reset();
        let _ = take_queued_spawns();
    }
}

#[test]
fn server_services_validate_arguments_and_queue_engine_commands() {
    let _guard = BUS_LOCK.lock().unwrap_or_else(|e| e.into_inner());
    let _reset = Reset;
    script::reset();

    let mob = fox_mob(41);
    script::publish(MobView::of(&mob).unwrap());

    let fetched = info(41).unwrap().unwrap();
    assert_eq!(fetched.handle, 41);
    assert_eq!(fetched.type_id, "sample_mobs:cinder_fox");
    assert!(fetched.alive);
    assert!(info(999).unwrap().is_none());

    // Synced get/set validation
    assert!(matches!(get_synced(41, "warmth").unwrap(), Some(FMobValue::Int(_))));
    assert!(set_synced(41, "no_such_key", &FMobValue::Bool(true)).is_err());
    assert!(set_synced(41, "warmth", &FMobValue::Float(f32::NAN)).is_err());
    set_synced(41, "warmth", &FMobValue::Int(7)).unwrap();

    // Controllers & anim triggers
    assert!(trigger_anim(41, "", "pounce", 1.0).unwrap());
    assert!(trigger_anim(41, "", "pounce", f32::INFINITY).is_err());
    assert!(set_controller_state(41, "base", "sit").unwrap());
    assert!(set_controller_state_hold(41, "base", "sit", Some(20)).unwrap());
    assert!(release_controller_state(41, "base").unwrap());

    // Navigate, look, jump, target, attack validation
    assert!(navigate_to(41, FVec3 { x: 4.0, y: 64.0, z: 0.5 }, 0.0).is_err());
    assert!(navigate_to(41, FVec3 { x: f64::NAN, y: 64.0, z: 0.5 }, 1.0).is_err());
    assert!(navigate_to(41, FVec3 { x: 4.0, y: 64.0, z: 0.5 }, 1.2).unwrap());
    navigate_stop(41).unwrap();
    look_at(41, FVec3 { x: 5.0, y: 65.0, z: 0.5 }, 30.0, 30.0).unwrap();
    jump(41).unwrap();
    assert!(set_target(41, Some(99), None).unwrap());
    assert!(attack(41, None, None, 2.0, 0.2).is_err());
    assert!(attack(41, Some(99), None, -1.0, 0.2).is_err());
    assert!(attack(41, Some(99), None, 3.0, 0.4).unwrap());

    // Attributes, hitboxes, persistent script data, impulse, despawn, claim
    let _ = attribute_get(41, "sample_mobs:warmth").unwrap();
    attribute_set_base(41, "sample_mobs:warmth", 5.0).unwrap();
    attribute_add_modifier(41, "sample_mobs:warmth", "bonus", 2.0, FAttributeOperation::AddValue).unwrap();
    assert!(attribute_remove_modifier(41, "sample_mobs:warmth", "bonus").unwrap());
    set_hitbox_enabled(41, "head", false).unwrap();
    assert!(data_get(41).unwrap().is_empty());
    assert!(data_put(41, vec![0; MAX_DATA_BYTES + 1]).is_err());
    data_put(41, vec![1, 2, 3]).unwrap();
    claim_control(41, true).unwrap();
    impulse(41, FVec3 { x: 0.1, y: 0.2, z: 0.0 }).unwrap();
    assert!(seat_set(41, 0, None).is_err());
    assert!(despawn(41).unwrap());

    // Brain memory validation
    assert!(memory_set(41, "missing", None, None).is_err());
    assert!(memory_set(41, "pounced", Some(&FMobMemoryValue::I32(1)), None).is_err());
    memory_set(41, "pounced", Some(&FMobMemoryValue::Bool(true)), Some(40)).unwrap();

    // Queued spawn validation
    assert!(
        queue_spawn(
            "bad".into(),
            "sample_mobs:cinder_fox".into(),
            FVec3 { x: 0.0, y: 64.0, z: 0.0 },
            FMobSpawnOptions { yaw: 0.0, baby: false, persistent: true },
        )
        .is_err()
    );
    queue_spawn(
        "minecraft:overworld".into(),
        "sample_mobs:cinder_fox".into(),
        FVec3 { x: 1.5, y: 64.0, z: 2.5 },
        FMobSpawnOptions { yaw: 45.0, baby: true, persistent: true },
    )
    .unwrap();
    assert_eq!(take_queued_spawns().len(), 1);
    assert!(take_queued_spawns().is_empty());

    let queued = script::take_for(41);
    assert!(queued.iter().any(|c| matches!(c, Command::SetSynced { key, .. } if key == "warmth")));
    assert!(queued.iter().any(|c| matches!(c, Command::TriggerAnim { name, .. } if name == "pounce")));
    assert!(queued.iter().any(|c| matches!(c, Command::HoldControllerState { .. })));
    assert!(queued.iter().any(|c| matches!(c, Command::Attack { damage, .. } if (*damage - 3.0).abs() < 1e-6)));
    assert!(queued.iter().any(|c| matches!(c, Command::SetMemory { key, .. } if key == "pounced")));
    assert!(queued.contains(&Command::Despawn));
}

#[test]
fn client_texture_store_and_bone_helpers_round_trip() {
    let _guard = BUS_LOCK.lock().unwrap_or_else(|e| e.into_inner());
    let _ty = cinder_fox_type();
    textures::store().lock().unwrap_or_else(|e| e.into_inner()).clear();

    // Registers the custom type in registry so script::bone_index finds its bones.
    horizon_client::mobs::custom::registry::registry()
        .write()
        .unwrap()
        .replace_one(_ty.clone());
    assert!(bone_index("sample_mobs:cinder_fox", "body").unwrap().is_some());
    assert!(bone_index("sample_mobs:cinder_fox", "missing_bone").unwrap().is_none());

    let id = texture_create(32, 32).unwrap();
    texture_write(id, (0, 0), (2, 2), &[255; 16]).unwrap();
    texture_bind(88, "default", Some(id)).unwrap();
    assert!(textures::overrides_for(88, [0, 0, 1024, 1024], [4096, 4096]).is_some());
    release_f_textures();
    assert!(textures::overrides_for(88, [0, 0, 1024, 1024], [4096, 4096]).is_none());
}

#[test]
fn reply_and_stage_pose_helpers_refuse_outside_their_hook() {
    assert!(hook_event().is_none());
    assert!(reply_bool(true).is_err());
    assert!(reply_hurt(1.0, false).is_err());
    assert!(reply_interact(FInteractReply::Success).is_err());
    assert!(reply_save(vec![1]).is_err());
    assert!(reply_pose(vec![]).is_err());
    assert!(draw_event().is_none());
    assert!(draw_cancel().is_err());
    assert!(pose_bone_count().is_err());
    assert!(pose_bone_name(0).is_err());
    assert!(pose_bone_index("body").is_err());
    assert!(pose_get(0).is_err());
    assert!(pose_get_all().is_err());
    assert!(
        pose_set(
            0,
            FBoneTransform {
                translation: FVec3f { x: 0.0, y: 0.0, z: 0.0 },
                rotation: FQuat { x: 0.0, y: 0.0, z: 0.0, w: 1.0 },
                scale: FVec3f { x: 1.0, y: 1.0, z: 1.0 },
                visible: true,
            }
        )
        .is_err()
    );
    assert!(pose_write(FMobBoneWrite::default()).is_err());
    let _ = to_f_value(&SyncedValue::I32(1).into_script_value_for_test());
}

trait SyncedToScriptTest {
    fn into_script_value_for_test(self) -> MobScriptValue;
}

impl SyncedToScriptTest for SyncedValue {
    fn into_script_value_for_test(self) -> MobScriptValue {
        match self {
            SyncedValue::Bool(b) => MobScriptValue::Bool(b),
            SyncedValue::I32(n) => MobScriptValue::Int(n),
            SyncedValue::F32(f) => MobScriptValue::Float(f),
            SyncedValue::Str(s) => MobScriptValue::Text(s),
            SyncedValue::Vec3(p) => MobScriptValue::Vec3(horizon_client::mobs::custom::script::ScriptVec3 {
                x: p[0],
                y: p[1],
                z: p[2],
            }),
            SyncedValue::Enum(_) => MobScriptValue::Choice(String::new()),
        }
    }
}

#[cfg(feature = "yoake2-slice")]
#[test]
fn draw_stage_hooks_are_noop_when_no_f_module_is_loaded() {
    let _guard = BUS_LOCK.lock().unwrap_or_else(|e| e.into_inner());
    let _ = yoake_slice::runtime::uninstall();
    assert!(!crate::yoake2_mods::client_mods_loaded());
    assert!(!f_client_runtime().live());
    assert!(!f_world_runtime().live());

    let ty = cinder_fox_type();
    let tr = ty.render.as_ref().unwrap();
    let mob = fox_mob(5);
    let mut mr = MobRender::new(tr);
    let ctx = PoseCtx::new(10, 0.5, MobFacts::of(&mob));
    assert!(!before_pose_stage(mob.id, &ty.id, LodMode::Full, 0.5, 1.0, 0.0, 0.0, tr.skeleton.len()));
    assert!(mr.update(&ty, tr, None, &ctx));
    assert!(!after_pose_stage(mob.id, &ty.id, LodMode::Full, 0.5, 1.0, 0.0, 0.0, tr, &mut mr));
    let draw = DrawCtx {
        world_from_model: glam::Mat4::IDENTITY,
        light: 0,
        overlay: 0,
        lod: LodMode::Full,
        atlas: &|_| Some([0.0, 0.0, 1.0, 1.0]),
        texture_overrides: None,
    };
    let mut verts: Vec<Vertex> = Vec::new();
    let stats = tr.draw(&mr, &draw, &mut verts);
    assert!(stats.total() > 0 && !verts.is_empty());
    after_vertices_stage(mob.id, &ty.id, LodMode::Full, 0.5, 1.0, 0.0, 0.0, tr.skeleton.len(), verts.len());
}

#[cfg(feature = "yoake2-slice")]
#[path = "guest_tests.rs"]
mod guest_tests;
