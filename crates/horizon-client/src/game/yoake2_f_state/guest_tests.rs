//! REM-12: a real Rust guest (`crates/yoake-guest-sdk/examples/f_client_services.rs`, compiled to wasm32 with the SDK)
//! calls the generated `f_services` bindings through `engine_call`: the engine side is the generated glue
//! (`yoake_glue::call`), the hooks are entered through `yoake2_mods` the way the game does. No game resources and no
//! window: what a real `Game` adds (the batch, the audio engine, the window) is behind the traits the tests above fake.

use super::*;
use std::path::Path;
use yoake_slice::activation::{Accessor, PathRoot, Status};
use yoake_slice::runtime::{self, ModConfig};

#[path = "../../../../yoake-guest-sdk/tests/support/build.rs"]
mod guest_build;

/// A stand-in for the client's `Game` root: no path resolves (the F services need none).
struct TickRoot;

impl PathRoot for TickRoot {
    fn resolve(&mut self, _: &[Accessor], _: &mut dyn FnMut(&mut dyn std::any::Any)) -> Result<(), Status> {
        Err(Status::Stale)
    }
}

fn dispatch(handle: u64, id: u32, args: &[u8], out: &mut Vec<u8>) -> Result<(), yoake_slice::codec::CallError> {
    crate::game::yoake2_f_services::yoake_glue::call(handle, id, args, out)
}

fn slot(reports: &[(i32, i64)], n: i32) -> Vec<i64> {
    reports.iter().filter(|(s, _)| *s == n).map(|(_, v)| *v).collect()
}

#[test]
fn an_f_guest_draws_a_hud_plays_a_sound_feeds_a_stream_and_takes_the_input_method() {
    let sdk = Path::new(env!("CARGO_MANIFEST_DIR")).join("../yoake-guest-sdk");
    let scratch = std::env::temp_dir().join("yoake2_f_services_guest");
    let _ = std::fs::remove_dir_all(&scratch);
    std::fs::create_dir_all(&scratch).unwrap();
    let rlib = guest_build::build_sdk(&sdk, &scratch);
    let wasm = guest_build::build_guest(&rlib, &sdk.join("examples/f_client_services.rs"), &scratch, "f_client_services");

    let _ = runtime::uninstall();
    runtime::set_engine_dispatch(Some(dispatch));
    runtime::install(&wasm, ModConfig::new("")).unwrap();
    assert!(runtime::take_reports().unwrap().is_empty());

    // CLIENT_TICK: the sound, the stream and the IME are queued; hud_draw outside the HUD hook is refused
    runtime::dispatch_root_id(runtime::HOOK_CLIENT_TICK, &mut TickRoot, &mut |_| 0);
    let reports = runtime::take_reports().unwrap();
    assert_eq!(slot(&reports, 4), [1], "{reports:?}");
    assert_eq!(slot(&reports, 6), [4], "four samples were queued");
    assert_eq!(slot(&reports, 9), [1], "hud_draw is refused outside HUD_RENDER");
    let stream = slot(&reports, 5)[0] as u32;
    assert!(ime_claimed());
    assert_eq!(ime_area(), Some([10.0, 20.0, 40.0, 12.0]));
    assert_eq!(take_sounds(), [PendingSound { sound: "minecraft:ui.button.click".into(), at: None, volume: 1.0, pitch: 1.0 }]);
    let mut engine = super::Engine::default();
    flush_streams(&mut engine);
    assert_eq!(engine.pushed[&stream], [0.0, 0.25, -0.25, 0.5]);
    assert_eq!(engine.open[&stream].3, 48_000);
    // a second tick asks for nothing more
    runtime::dispatch_root_id(runtime::HOOK_CLIENT_TICK, &mut TickRoot, &mut |_| 0);
    assert!(runtime::take_reports().unwrap().is_empty());
    assert!(take_sounds().is_empty());

    // HUD_RENDER: the guest sees the frame's GUI size and draws a rect and a text
    hud_begin((427.0, 240.0), 2.0);
    crate::yoake2_mods::dispatch_hud_render();
    let commands = hud_end();
    let reports = runtime::take_reports().unwrap();
    assert_eq!(slot(&reports, 1), [2], "{reports:?}");
    assert_eq!(slot(&reports, 2), [427]);
    assert_eq!(slot(&reports, 3), [200]);
    assert_eq!(slot(&reports, 10), [1]);
    assert_eq!(
        commands,
        [
            FHud::Rect { x: 4.0, y: 4.0, width: 60.0, height: 12.0, color: 0xcc10_2030 },
            FHud::Text { text: "F module HUD".into(), x: 6.0, y: 6.0, color: 0xff_ffff, shadow: true },
        ]
    );

    // the IME hooks: the guest reads the event of the hook that runs
    set_ime_event(Some(commit_event("漢字")));
    crate::yoake2_mods::dispatch_ime_commit();
    set_ime_event(Some(preedit_event("かな", Some((0, 3)))));
    crate::yoake2_mods::dispatch_ime_preedit();
    set_ime_event(None);
    let reports = runtime::take_reports().unwrap();
    assert_eq!(slot(&reports, 7), [6], "the commit's 6 UTF-8 bytes");
    assert_eq!(slot(&reports, 8), [6]);

    // the hook modes the loader reports for the new synthetic ids are observe-only
    assert_eq!(crate::yoake2_mods::hook_mode(runtime::HOOK_GPU_FRAME), Some(yoake_slice::bridge::Mode::ObserveOnly));
    assert_eq!(crate::yoake2_mods::hook_mode(runtime::HOOK_IME_COMMIT), Some(yoake_slice::bridge::Mode::ObserveOnly));
    assert!(crate::yoake2_mods::client_registered(runtime::HOOK_IME_PREEDIT));
    assert!(!crate::yoake2_mods::client_registered(runtime::HOOK_GPU_FRAME), "the example does not register GPU_FRAME");

    let stats = runtime::stats();
    assert!(!stats.disabled, "{:?}", stats.trap);
    runtime::set_engine_dispatch(None);
    let _ = runtime::uninstall();
    shutdown(&mut engine);
}
