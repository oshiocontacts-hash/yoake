//! Guest fixture for F4M-11 renderer stage hook tests (`before_pose`, `after_pose`, `after_vertices`).
//! Compiled as a `#![no_std]` wasm32 core module (see `tests/support/build.rs`).
#![no_std]
extern crate alloc;

use alloc::string::String;
use core::sync::atomic::{AtomicU32, Ordering};

use yoake_guest_sdk::custom_mobs::{
    self, FBoneTransform, FMobBoneWrite, FMobDrawStage, FQuat, FVec3f, register_draw_stage_hooks,
};
use yoake_guest_sdk::{Error, Handle, Registrar, export_guest, hooks, report};

// Modes controlled via `entity`:
// - entity == 999: cancel inside `before_pose`.
// - otherwise: record stage order, edit bone 0 via `pose_set` + `pose_write` inside `after_pose`.
const CANCEL_ENTITY: u32 = 999;

const SLOT_ORDER: i32 = 1;
const SLOT_BONE_COUNT: i32 = 2;
const SLOT_VERTEX_COUNT: i32 = 3;
const SLOT_SET_OK: i32 = 4;
const SLOT_ERROR: i32 = 99;

static STEP: AtomicU32 = AtomicU32::new(0);

fn init(r: &mut Registrar) {
    register_draw_stage_hooks(r, 0).expect("register draw stage hooks");
}

fn hook(id: u32, _arg0: Handle) -> i64 {
    let ev = match custom_mobs::draw_event() {
        Ok(Some(ev)) => ev,
        Ok(None) => {
            report(SLOT_ERROR, -1);
            return 0;
        }
        Err(Error::Status(s)) => {
            report(SLOT_ERROR, i64::from(s));
            return 0;
        }
        Err(Error::Handle(h)) => {
            report(SLOT_ERROR, -100 - i64::from(h));
            return 0;
        }
        Err(Error::Decode(_)) => {
            report(SLOT_ERROR, -200);
            return 0;
        }
    };
    let seq = STEP.fetch_add(1, Ordering::Relaxed) + 1;
    match (id, ev.stage) {
        (hooks::CUSTOM_MOB_BEFORE_POSE, FMobDrawStage::BeforePose) => {
            report(SLOT_ORDER, i64::from(seq) * 10 + 1);
            if ev.entity == CANCEL_ENTITY {
                let _ = custom_mobs::draw_cancel();
                return 1;
            }
            0
        }
        (hooks::CUSTOM_MOB_AFTER_POSE, FMobDrawStage::AfterPose) => {
            report(SLOT_ORDER, i64::from(seq) * 10 + 2);
            if let Ok(n) = custom_mobs::pose_bone_count() {
                report(SLOT_BONE_COUNT, i64::from(n));
            }
            if let Ok(Some(name)) = custom_mobs::pose_bone_name(0)
                && custom_mobs::pose_bone_index(name).ok().flatten() == Some(0)
            {
                if let Ok(mut cur) = custom_mobs::pose_get(0) {
                    cur.translation.y += 16.0;
                    cur.rotation = FQuat { x: 0.0, y: 0.0, z: 0.0, w: 1.0 };
                    let set_ok = custom_mobs::pose_set(0, FBoneTransform {
                        translation: cur.translation,
                        rotation: cur.rotation,
                        scale: cur.scale,
                        visible: true,
                    })
                    .is_ok();
                    let write_ok = custom_mobs::pose_write(FMobBoneWrite {
                        bone: 0,
                        additive: true,
                        translation: Some(FVec3f { x: 4.0, y: 0.0, z: 0.0 }),
                        rotation_degrees: None,
                        scale: None,
                        visible: None,
                    })
                    .is_ok();
                    report(SLOT_SET_OK, i64::from(set_ok && write_ok));
                }
            }
            0
        }
        (hooks::CUSTOM_MOB_AFTER_VERTICES, FMobDrawStage::AfterVertices) => {
            report(SLOT_ORDER, i64::from(seq) * 10 + 3);
            report(SLOT_VERTEX_COUNT, i64::from(ev.vertex_count));
            0
        }
        _ => 0,
    }
}

export_guest!(init, hook);
