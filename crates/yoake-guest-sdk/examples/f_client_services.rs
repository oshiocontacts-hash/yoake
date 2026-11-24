//! Example full-access CLIENT mod (REM-12): draws from `HUD_RENDER`, plays a sound and feeds a PCM stream from
//! `CLIENT_TICK`, and takes the input method (`IME_PREEDIT` / `IME_COMMIT`). These are the client host services a
//! sandboxed client-full module has, as the engine-call bindings `stubs::f_services`.
//!
//! Compiled as a `#![no_std]` wasm32 core module (see `tests/support/build.rs`), then loaded as a `[wasm] client` module.
#![no_std]
extern crate alloc;

use alloc::string::String;
use alloc::vec;
use alloc::vec::Vec;
use core::sync::atomic::{AtomicU32, Ordering};

use yoake_guest_sdk::stubs::f_services::types::{FAnchor, FHud, FImeEvent};
use yoake_guest_sdk::stubs::f_services::{
    f_services_audio_play_sound, f_services_audio_stream_open, f_services_audio_stream_push, f_services_hud_draw,
    f_services_hud_gui_scale, f_services_hud_gui_size, f_services_ime_event, f_services_ime_set_allowed,
    f_services_ime_set_cursor_area,
};
use yoake_guest_sdk::{Handle, Registrar, export_guest, hooks, report};

// Report slots read by the test (`game/yoake2_f_state/guest_tests.rs`).
const SLOT_HUD_DRAWN: i32 = 1;
const SLOT_GUI_WIDTH: i32 = 2;
const SLOT_GUI_SCALE_X100: i32 = 3;
const SLOT_SOUND_OK: i32 = 4;
const SLOT_STREAM: i32 = 5;
const SLOT_STREAM_PUSHED: i32 = 6;
const SLOT_COMMIT_BYTES: i32 = 7;
const SLOT_PREEDIT_BYTES: i32 = 8;
const SLOT_DRAW_OUTSIDE_HUD_REFUSED: i32 = 9;
const SLOT_FRAMES: i32 = 10;

static TICKS: AtomicU32 = AtomicU32::new(0);
static FRAMES: AtomicU32 = AtomicU32::new(0);

fn init(r: &mut Registrar) {
    r.hook(hooks::CLIENT_TICK, 0).expect("register CLIENT_TICK");
    r.hook(hooks::HUD_RENDER, 0).expect("register HUD_RENDER");
    r.hook(hooks::IME_PREEDIT, 0).expect("register IME_PREEDIT");
    r.hook(hooks::IME_COMMIT, 0).expect("register IME_COMMIT");
}

fn hook(id: u32, _arg0: Handle) -> i64 {
    match id {
        hooks::CLIENT_TICK => {
            if TICKS.fetch_add(1, Ordering::Relaxed) == 0 {
                // the first tick: ask for everything once
                let _ = f_services_ime_set_allowed(true);
                let _ = f_services_ime_set_cursor_area(10.0, 20.0, 40.0, 12.0);
                let sound = f_services_audio_play_sound(String::from("minecraft:ui.button.click"), 1.0, 1.0);
                report(SLOT_SOUND_OK, i64::from(matches!(sound, Ok(Ok(())))));
                if let Ok(Ok(stream)) = f_services_audio_stream_open(FAnchor::Listener, String::from("ui"), 1, 48_000) {
                    report(SLOT_STREAM, i64::from(stream));
                    if let Ok(Ok(count)) = f_services_audio_stream_push(stream, vec![0.0, 0.25, -0.25, 0.5]) {
                        report(SLOT_STREAM_PUSHED, i64::from(count));
                    }
                }
                // drawing is only allowed inside HUD_RENDER
                let refused = !matches!(f_services_hud_draw(vec![FHud::EndClip]), Ok(Ok(_)));
                report(SLOT_DRAW_OUTSIDE_HUD_REFUSED, i64::from(refused));
            }
            0
        }
        hooks::HUD_RENDER => {
            let frames = FRAMES.fetch_add(1, Ordering::Relaxed) + 1;
            report(SLOT_FRAMES, i64::from(frames));
            if let Ok((width, _height)) = f_services_hud_gui_size() {
                report(SLOT_GUI_WIDTH, width as i64);
            }
            if let Ok(scale) = f_services_hud_gui_scale() {
                report(SLOT_GUI_SCALE_X100, (scale * 100.0) as i64);
            }
            let commands: Vec<FHud> = vec![
                FHud::Rect { x: 4.0, y: 4.0, width: 60.0, height: 12.0, color: 0xcc10_2030 },
                FHud::Text { text: String::from("F module HUD"), x: 6.0, y: 6.0, color: 0xff_ffff, shadow: true },
            ];
            if let Ok(Ok(count)) = f_services_hud_draw(commands) {
                report(SLOT_HUD_DRAWN, i64::from(count));
            }
            0
        }
        hooks::IME_PREEDIT => {
            if let Ok(Some(FImeEvent::Preedit { text, .. })) = f_services_ime_event() {
                report(SLOT_PREEDIT_BYTES, text.len() as i64);
            }
            0
        }
        hooks::IME_COMMIT => {
            if let Ok(Some(FImeEvent::Commit { text })) = f_services_ime_event() {
                report(SLOT_COMMIT_BYTES, text.len() as i64);
            }
            0
        }
        _ => 0,
    }
}

export_guest!(init, hook);
