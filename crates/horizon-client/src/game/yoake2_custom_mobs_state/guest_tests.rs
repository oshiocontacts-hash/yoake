//! F4M-11 end-to-end guest tests: real wasm32 core modules compiled with `yoake-guest-sdk` run
//! through `yoake_slice` + `yoake2_custom_mobs::yoake_glue::call` + the engine's `ScriptGoal` and
//! `render_state` draw pipeline, without Minecraft resources or a window.

use std::path::Path;
use std::sync::Arc;

use horizon_client::mesher::Vertex;
use horizon_client::mobs::custom::{
    SyncedValue,
    render_state::{DrawCtx, LodMode, MobFacts, MobRender, PoseCtx},
};
use yoake_slice::runtime::{self, ModConfig};

use super::*;

#[path = "../../../../yoake-guest-sdk/tests/support/build.rs"]
mod guest_build;

fn dispatch(handle: u64, id: u32, args: &[u8], out: &mut Vec<u8>) -> Result<(), yoake_slice::codec::CallError> {
    crate::game::yoake2_custom_mobs::yoake_glue::call(handle, id, args, out)
}

fn slot(reports: &[(i32, i64)], n: i32) -> Vec<i64> {
    reports.iter().filter(|(s, _)| *s == n).map(|(_, v)| *v).collect()
}

#[test]
fn draw_stage_hooks_run_in_order_edit_pose_and_cancel_in_before_pose() {
    let _guard = super::BUS_LOCK.lock().unwrap_or_else(|e| e.into_inner());
    let sdk = Path::new(env!("CARGO_MANIFEST_DIR")).join("../yoake-guest-sdk");
    let scratch = std::env::temp_dir().join("yoake2_custom_mob_stage_guest");
    let _ = std::fs::remove_dir_all(&scratch);
    std::fs::create_dir_all(&scratch).unwrap();
    let rlib = guest_build::build_sdk(&sdk, &scratch);
    let wasm = guest_build::build_guest(&rlib, &sdk.join("tests/guests/custom_mob_stage.rs"), &scratch, "custom_mob_stage");

    struct StageGuestReset;
    impl Drop for StageGuestReset {
        fn drop(&mut self) {
            runtime::set_thread_engine_dispatch(None);
            let _ = runtime::uninstall();
        }
    }

    let _ = runtime::uninstall();
    runtime::set_thread_engine_dispatch(Some(dispatch));
    runtime::install(&wasm, ModConfig::new("")).unwrap();
    let _reset = StageGuestReset;

    assert_eq!(crate::yoake2_mods::hook_mode(runtime::HOOK_CUSTOM_MOB_BEFORE_POSE), Some(yoake_slice::bridge::Mode::Full));
    assert_eq!(crate::yoake2_mods::hook_mode(runtime::HOOK_CUSTOM_MOB_AFTER_POSE), Some(yoake_slice::bridge::Mode::Full));
    assert_eq!(crate::yoake2_mods::hook_mode(runtime::HOOK_CUSTOM_MOB_AFTER_VERTICES), Some(yoake_slice::bridge::Mode::ObserveOnly));

    let ty = super::cinder_fox_type();
    let tr = ty.render.as_ref().unwrap();
    let draw_ctx = DrawCtx {
        world_from_model: glam::Mat4::IDENTITY,
        light: 0,
        overlay: 0,
        lod: LodMode::Full,
        atlas: &|_| Some([0.0, 0.0, 1.0, 1.0]),
        texture_overrides: None,
    };

    // Baseline pose without stage edits (compute directly before guest run by checking rest/baseline vertices).
    let mob = super::fox_mob(10);
    let mut base_mr = MobRender::new(tr);
    let ctx = PoseCtx::new(20, 0.25, MobFacts::of(&mob));
    assert!(base_mr.update(&ty, tr, None, &ctx));
    let mut base_verts: Vec<Vertex> = Vec::new();
    tr.draw(&base_mr, &draw_ctx, &mut base_verts);
    assert!(!base_verts.is_empty());

    // Run all 3 stages through the F guest (`entity = 10` edits bone 0 in `after_pose`).
    let mut mr = MobRender::new(tr);
    let mut edited_verts: Vec<Vertex> = Vec::new();
    let cancelled = before_pose_stage(mob.id, &ty.id, LodMode::Full, 0.25, 1.0, 0.0, 0.0, tr.skeleton.len());
    assert!(!cancelled);
    assert!(mr.update(&ty, tr, None, &ctx));
    assert!(!after_pose_stage(mob.id, &ty.id, LodMode::Full, 0.25, 1.0, 0.0, 0.0, tr, &mut mr));
    tr.draw(&mr, &draw_ctx, &mut edited_verts);
    after_vertices_stage(
        mob.id,
        &ty.id,
        LodMode::Full,
        0.25,
        1.0,
        0.0,
        0.0,
        tr.skeleton.len(),
        edited_verts.len(),
    );

    let stats = runtime::stats();
    assert!(!stats.disabled, "guest disabled: {:?}", stats.trap);
    assert!(
        stats.entries >= 3,
        "expected >= 3 stage hook entries, got {} (registered={:?})",
        stats.entries,
        stats.registered
    );
    let reports = runtime::take_reports().unwrap();
    assert!(slot(&reports, 99).is_empty(), "draw_event error in guest: {:?}", slot(&reports, 99));
    assert_eq!(slot(&reports, 1), [11, 22, 33], "stages must fire before_pose -> after_pose -> after_vertices");
    assert_eq!(slot(&reports, 2), [tr.skeleton.len() as i64]);
    assert_eq!(slot(&reports, 3), [edited_verts.len() as i64]);
    assert_eq!(slot(&reports, 4), [1], "pose_set + pose_write must succeed");
    assert_ne!(edited_verts[0].pos, base_verts[0].pos, "overridden pose must shift emitted vertices");

    // Entity 999 cancels in `before_pose`: neither `after_pose` nor `after_vertices` fires and draw is skipped.
    let cancel_id = 999;
    let mut cancelled_verts: Vec<Vertex> = Vec::new();
    if !before_pose_stage(cancel_id, &ty.id, LodMode::Full, 0.25, 1.0, 0.0, 0.0, tr.skeleton.len()) {
        if mr.update(&ty, tr, None, &ctx)
            && !after_pose_stage(cancel_id, &ty.id, LodMode::Full, 0.25, 1.0, 0.0, 0.0, tr, &mut mr)
        {
            tr.draw(&mr, &draw_ctx, &mut cancelled_verts);
            after_vertices_stage(
                cancel_id,
                &ty.id,
                LodMode::Full,
                0.25,
                1.0,
                0.0,
                0.0,
                tr.skeleton.len(),
                cancelled_verts.len(),
            );
        }
    }
    assert!(cancelled_verts.is_empty());
    let reports = runtime::take_reports().unwrap();
    assert!(slot(&reports, 99).is_empty(), "draw_event error in cancel pass: {:?}", slot(&reports, 99));
    assert_eq!(slot(&reports, 1), [41], "only before_pose ran when cancelled");
    assert!(slot(&reports, 3).is_empty());

    let stats = runtime::stats();
    assert!(!stats.disabled, "{:?}", stats.trap);
}

#[cfg(feature = "yoake-hook-bench")]
#[test]
fn fox_ai_full_scripts_pounce_through_script_goal_and_f_runtime() {
    use horizon_client::mobs::{
        MobEvent, Who,
        custom::{ai::attacks, script},
        goals::Cx,
        mob_hook_bench::BenchWorld,
    };
    use horizon_core::{geometry::Vec3, random::LegacyRandom};

    let _guard = super::BUS_LOCK.lock().unwrap_or_else(|e| e.into_inner());
    let sdk = Path::new(env!("CARGO_MANIFEST_DIR")).join("../yoake-guest-sdk");
    let scratch = std::env::temp_dir().join("yoake2_fox_ai_full_guest");
    let _ = std::fs::remove_dir_all(&scratch);
    std::fs::create_dir_all(&scratch).unwrap();
    let rlib = guest_build::build_sdk(&sdk, &scratch);
    let wasm = guest_build::build_guest(&rlib, &sdk.join("examples/fox_ai_full.rs"), &scratch, "fox_ai_full");

    struct SessionReset;
    impl Drop for SessionReset {
        fn drop(&mut self) {
            script::reset();
            runtime::set_thread_engine_dispatch(None);
            let _ = runtime::uninstall();
        }
    }

    let _ = runtime::uninstall();
    script::reset();
    runtime::set_thread_engine_dispatch(Some(dispatch));
    runtime::install(&wasm, ModConfig::new("")).unwrap();
    let _reset = SessionReset;

    assert!(f_world_runtime().live());
    let _session = script::open_session(f_world_runtime()).expect("open F script session");

    let fx = BenchWorld::new();
    let mut lr = LegacyRandom::new(1);
    let mut events = Vec::new();
    let mut fox = super::fox_mob(1);
    let mut prey = super::fox_mob(2);
    prey.position = Vec3::new(4.5, 1.0, 0.5);
    Arc::make_mut(&mut prey.kind).species = horizon_client::mobs::Species::Chicken;
    Arc::make_mut(&mut prey.kind).custom = None;
    let seen_prey = prey.seen(None);
    fox.ai.target = Some(Who::Mob(2));

    let mut w = fx.world();
    w.game_time = 1000;
    w.others = std::slice::from_ref(&seen_prey);
    let mut goals = std::mem::take(&mut fox.ai.goals);
    {
        let mut cx = Cx { w: &w, lr: &mut lr, events: &mut events };
        goals.tick(&mut fox, &mut cx);
    }
    fox.ai.goals = goals;
    assert!(fox.ai.goals.is_running("ScriptGoal"), "fox_ai_full must start pounce ScriptGoal at 4 blocks");

    // Server timeline "hit" frame leaps and bites through `MOB_ANIM_EVENT` -> `cm.attack(...)`
    attacks::deliver_frame(&mut fox, "hit");
    w.game_time = 1001;
    let y0 = fox.velocity.y;
    events.clear();
    assert!(script::pre_tick(&mut fox, &w, &mut events));
    assert!(fox.velocity.y > y0 + 0.3, "fox_ai_full hit frame leaps: {:?}", fox.velocity);
    assert!(events.iter().any(|e| matches!(e, MobEvent::AttackMob { target: 2, .. })));

    // Hurt event reaches `fox_ai_full` via `MOB_HURT` and sets `warmth`.
    let _ = script::hurt(&mut fox, "", "Generic", 4.0);
    w.game_time = 1002;
    fox.tick(&w, &mut lr, &mut events);
    let ty = fox.kind.custom.clone().unwrap();
    let warmth = fox.extra.custom.as_ref().unwrap().synced_value(&ty, "warmth").cloned();
    assert!(matches!(warmth, Some(SyncedValue::I32(v)) if v > 0), "warmth updated: {warmth:?}");

    let reports = runtime::take_reports().unwrap();
    assert_eq!(slot(&reports, 1), [1]);
    assert_eq!(slot(&reports, 2), [1]);
    assert_eq!(slot(&reports, 3), [1]);
    assert!(!slot(&reports, 4).is_empty());
    assert!(super::goal_registered("pounce"), "fox_ai_full registered pounce goal");
    assert!(!runtime::stats().disabled, "{:?}", runtime::stats().trap);
}
