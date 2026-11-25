//! F4M-11 renderer stage hooks (`before_pose`, `after_pose`, `after_vertices`) on `game/custom_mob_draw.rs`.
//!
//! When no F client module is loaded or none registered the corresponding synthetic hook, each stage costs a
//! single thread-local check and skips all pose cloning or matrix recomputation.

use std::cell::RefCell;

use horizon_client::mobs::custom::{
    render_state::{LodMode, MobRender, StagePose, TypeRender},
    script::{MobBoneWrite, ScriptVec3},
};
use yoake_slice::runtime as yr;

use super::{
    FBoneTransform, FMobBoneWrite, FMobDrawEvent, FMobDrawStage, FMobLodMode, FQuat, FVec3f,
};

#[derive(Default)]
struct StageState {
    event: Option<FMobDrawEvent>,
    bone_names: Vec<String>,
    pose: Option<StagePose>,
    pose_dirty: bool,
    cancelled: bool,
}

thread_local! {
    static STAGE: RefCell<StageState> = RefCell::new(StageState::default());
}

struct StageReset;

impl Drop for StageReset {
    fn drop(&mut self) {
        STAGE.with(|s| {
            let mut s = s.borrow_mut();
            s.event = None;
            s.bone_names.clear();
            s.pose = None;
            s.pose_dirty = false;
            s.cancelled = false;
        });
    }
}

fn f_lod(lod: LodMode) -> FMobLodMode {
    match lod {
        LodMode::Full => FMobLodMode::Full,
        LodMode::HalfRate => FMobLodMode::HalfRate,
        LodMode::QuarterRate => FMobLodMode::QuarterRate,
        LodMode::LowBones => FMobLodMode::LowBones,
        LodMode::Static => FMobLodMode::Static,
        LodMode::Hidden => FMobLodMode::Hidden,
    }
}

fn stage_event(
    entity: u32,
    type_id: &str,
    stage: FMobDrawStage,
    lod: LodMode,
    partial: f32,
    scale: f32,
    yaw: f32,
    roll: f32,
    bone_count: usize,
    vertex_count: usize,
) -> FMobDrawEvent {
    FMobDrawEvent {
        entity,
        type_id: type_id.to_owned(),
        stage,
        lod: f_lod(lod),
        partial,
        scale,
        yaw,
        roll,
        bone_count: bone_count as u32,
        vertex_count: vertex_count as u32,
    }
}

/// Runs `HOOK_CUSTOM_MOB_BEFORE_POSE`. Returns `true` when the draw should be cancelled.
#[allow(clippy::too_many_arguments)]
pub(crate) fn before_pose_stage(
    entity: u32,
    type_id: &str,
    lod: LodMode,
    partial: f32,
    scale: f32,
    yaw: f32,
    roll: f32,
    bone_count: usize,
) -> bool {
    if !crate::yoake2_mods::mob_hook_registered(yr::HOOK_CUSTOM_MOB_BEFORE_POSE) {
        return false;
    }
    let event = stage_event(entity, type_id, FMobDrawStage::BeforePose, lod, partial, scale, yaw, roll, bone_count, 0);
    STAGE.with(|s| {
        let mut s = s.borrow_mut();
        s.event = Some(event);
        s.bone_names.clear();
        s.pose = None;
        s.pose_dirty = false;
        s.cancelled = false;
    });
    let _reset = StageReset;
    let raw = crate::yoake2_mods::dispatch_custom_mob_hook(yr::HOOK_CUSTOM_MOB_BEFORE_POSE);
    STAGE.with(|s| s.borrow().cancelled || raw != 0)
}

/// Runs `HOOK_CUSTOM_MOB_AFTER_POSE`. Applies any pose overrides back onto `(tr, state)` and returns `true`
/// if cancelled.
#[allow(clippy::too_many_arguments)]
pub(crate) fn after_pose_stage(
    entity: u32,
    type_id: &str,
    lod: LodMode,
    partial: f32,
    scale: f32,
    yaw: f32,
    roll: f32,
    tr: &TypeRender,
    state: &mut MobRender,
) -> bool {
    if !crate::yoake2_mods::mob_hook_registered(yr::HOOK_CUSTOM_MOB_AFTER_POSE) {
        return false;
    }
    let bone_count = tr.skeleton.len();
    let names: Vec<String> = tr.skeleton.bones.iter().map(|b| b.name.clone()).collect();
    let pose = state.take_stage_pose(tr);
    let event = stage_event(entity, type_id, FMobDrawStage::AfterPose, lod, partial, scale, yaw, roll, bone_count, 0);
    STAGE.with(|s| {
        let mut s = s.borrow_mut();
        s.event = Some(event);
        s.bone_names = names;
        s.pose = Some(pose);
        s.pose_dirty = false;
        s.cancelled = false;
    });
    let _reset = StageReset;
    let raw = crate::yoake2_mods::dispatch_custom_mob_hook(yr::HOOK_CUSTOM_MOB_AFTER_POSE);
    let (cancelled, dirty, updated) = STAGE.with(|s| {
        let mut s = s.borrow_mut();
        (s.cancelled || raw != 0, s.pose_dirty, s.pose.take())
    });
    if dirty && let Some(pose) = updated {
        state.commit_stage_pose(tr, pose);
    }
    cancelled
}

/// Runs `HOOK_CUSTOM_MOB_AFTER_VERTICES` (observe-only).
#[allow(clippy::too_many_arguments)]
pub(crate) fn after_vertices_stage(
    entity: u32,
    type_id: &str,
    lod: LodMode,
    partial: f32,
    scale: f32,
    yaw: f32,
    roll: f32,
    bone_count: usize,
    vertex_count: usize,
) {
    if !crate::yoake2_mods::mob_hook_registered(yr::HOOK_CUSTOM_MOB_AFTER_VERTICES) {
        return;
    }
    let event = stage_event(
        entity,
        type_id,
        FMobDrawStage::AfterVertices,
        lod,
        partial,
        scale,
        yaw,
        roll,
        bone_count,
        vertex_count,
    );
    STAGE.with(|s| {
        let mut s = s.borrow_mut();
        s.event = Some(event);
        s.bone_names.clear();
        s.pose = None;
        s.pose_dirty = false;
        s.cancelled = false;
    });
    let _reset = StageReset;
    let _ = crate::yoake2_mods::dispatch_custom_mob_hook(yr::HOOK_CUSTOM_MOB_AFTER_VERTICES);
}

pub(crate) fn draw_event() -> Option<FMobDrawEvent> {
    STAGE.with(|s| s.borrow().event.clone())
}

pub(crate) fn draw_cancel() -> Result<(), String> {
    STAGE.with(|s| {
        let mut s = s.borrow_mut();
        let can_cancel = matches!(
            s.event.as_ref().map(|e| e.stage),
            Some(FMobDrawStage::BeforePose | FMobDrawStage::AfterPose)
        );
        if !can_cancel {
            return Err("mob_draw_cancel is only valid inside before_pose or after_pose".into());
        }
        s.cancelled = true;
        Ok(())
    })
}

fn require_after_pose<R>(f: impl FnOnce(&mut StageState, &mut StagePose) -> Result<R, String>) -> Result<R, String> {
    STAGE.with(|s| {
        let mut s = s.borrow_mut();
        if s.event.as_ref().map(|e| e.stage) != Some(FMobDrawStage::AfterPose) {
            return Err("pose read/edit services are only available inside after_pose".into());
        }
        let mut pose = s.pose.take().ok_or_else(|| "pose state unavailable".to_owned())?;
        let res = f(&mut s, &mut pose);
        s.pose = Some(pose);
        res
    })
}

fn to_f_bone((t, q, sc, vis): ([f32; 3], [f32; 4], [f32; 3], bool)) -> FBoneTransform {
    FBoneTransform {
        translation: FVec3f { x: t[0], y: t[1], z: t[2] },
        rotation: FQuat { x: q[0], y: q[1], z: q[2], w: q[3] },
        scale: FVec3f { x: sc[0], y: sc[1], z: sc[2] },
        visible: vis,
    }
}

pub(crate) fn pose_bone_count() -> Result<u32, String> {
    require_after_pose(|_, pose| Ok(pose.len() as u32))
}

pub(crate) fn pose_bone_name(bone: u32) -> Result<Option<String>, String> {
    require_after_pose(|s, _| Ok(s.bone_names.get(bone as usize).cloned()))
}

pub(crate) fn pose_bone_index(name: &str) -> Result<Option<u32>, String> {
    require_after_pose(|s, _| Ok(s.bone_names.iter().position(|n| n == name).map(|i| i as u32)))
}

pub(crate) fn pose_get(bone: u32) -> Result<FBoneTransform, String> {
    require_after_pose(|_, pose| {
        pose.get(bone)
            .map(to_f_bone)
            .ok_or_else(|| format!("bone index {bone} out of range (0..{})", pose.len()))
    })
}

pub(crate) fn pose_get_all() -> Result<Vec<FBoneTransform>, String> {
    require_after_pose(|_, pose| Ok(pose.get_all().into_iter().map(to_f_bone).collect()))
}

pub(crate) fn pose_set(bone: u32, transform: FBoneTransform) -> Result<(), String> {
    require_after_pose(|s, pose| {
        pose.set(
            bone,
            [transform.translation.x, transform.translation.y, transform.translation.z],
            [transform.rotation.x, transform.rotation.y, transform.rotation.z, transform.rotation.w],
            [transform.scale.x, transform.scale.y, transform.scale.z],
            Some(transform.visible),
        )?;
        s.pose_dirty = true;
        Ok(())
    })
}

pub(crate) fn pose_write(write: FMobBoneWrite) -> Result<(), String> {
    require_after_pose(|s, pose| {
        let v = |p: FVec3f| ScriptVec3 { x: p.x, y: p.y, z: p.z };
        pose.apply_write(&MobBoneWrite {
            bone: write.bone,
            additive: write.additive,
            translation: write.translation.map(v),
            rotation_degrees: write.rotation_degrees.map(v),
            scale: write.scale.map(v),
            visible: write.visible,
        })?;
        s.pose_dirty = true;
        Ok(())
    })
}
