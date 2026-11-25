//! REM-12 host tests of the F client services: the queues and their limits, with a fake audio engine. The generated
//! bindings and a real guest are in `guest_tests.rs`; the HUD commands drawn on a batch are in
//! `wasm_mods/full.rs` (`f_hud_commands_...`).

use super::*;
use crate::game::yoake2_f_services::{gpu_buffer, gpu_frame, gpu_poll, gpu_shader};
use std::collections::HashMap;

fn rect(x: f32) -> FHud {
    FHud::Rect { x, y: 0.0, width: 10.0, height: 4.0, color: 0xff00_ff00 }
}

#[test]
fn hud_draw_is_refused_outside_the_hud_render_hook_and_queues_inside_it() {
    assert!(hud_draw(vec![rect(1.0)]).unwrap_err().contains("HUD_RENDER"));
    assert_eq!(hud_gui_size(), (0.0, 0.0));
    hud_begin((427.0, 240.0), 2.0);
    assert_eq!(hud_gui_size(), (427.0, 240.0));
    assert_eq!(hud_gui_scale(), 2.0);
    assert_eq!(hud_draw(vec![rect(1.0), rect(2.0)]), Ok(2));
    assert_eq!(hud_draw(vec![FHud::EndClip]), Ok(1));
    let commands = hud_end();
    assert_eq!(commands.len(), 3);
    assert_eq!(commands[1], rect(2.0));
    assert!(hud_draw(vec![rect(3.0)]).is_err(), "closed again");
    assert!(hud_end().is_empty(), "taken once");
    assert_eq!(hud_gui_size(), (427.0, 240.0), "the last frame's size stays for the other hooks");
}

#[test]
fn a_frame_takes_at_most_the_command_limit_and_a_bad_call_queues_nothing() {
    hud_begin((100.0, 100.0), 1.0);
    assert_eq!(hud_draw(vec![rect(0.0); MAX_HUD_COMMANDS - 1]), Ok(MAX_HUD_COMMANDS as u32 - 1));
    assert!(hud_draw(vec![rect(0.0); 2]).unwrap_err().contains("at most"));
    assert_eq!(hud_draw(vec![rect(0.0)]), Ok(1), "the last slot is free: the refused call queued nothing");
    // a quad needs four corners; a name is bounded
    hud_begin((100.0, 100.0), 1.0);
    let quad = FHud::Quad { corners: vec![(0.0, 0.0); 3], sprite: None, uv: None, color: 0 };
    assert!(hud_draw(vec![rect(0.0), quad]).is_err());
    let long = FHud::Text { text: "x".repeat(MAX_NAME_BYTES + 1), x: 0.0, y: 0.0, color: 0, shadow: false };
    assert!(hud_draw(vec![long]).is_err());
    assert!(hud_end().is_empty(), "a call with a bad command queues none of its commands");
}

#[test]
fn a_new_frame_drops_what_an_unfinished_one_queued() {
    hud_begin((10.0, 10.0), 1.0);
    hud_draw(vec![rect(1.0)]).unwrap();
    hud_begin((10.0, 10.0), 1.0);
    assert!(hud_end().is_empty());
}

#[test]
fn sounds_are_checked_and_queued_up_to_the_games_quota() {
    assert!(audio_play_sound("no_namespace".into(), 1.0, 1.0).is_err());
    assert!(audio_play_sound("minecraft:".into(), 1.0, 1.0).is_err());
    assert!(audio_play_sound("minecraft:ui.button.click".into(), 17.0, 1.0).is_err());
    assert!(audio_play_sound("minecraft:ui.button.click".into(), 1.0, f32::NAN).is_err());
    assert!(audio_play_sound_at("minecraft:entity.pig.ambient".into(), f64::INFINITY, 0.0, 0.0, 1.0, 1.0).is_err());
    assert!(take_sounds().is_empty(), "refused sounds queue nothing");
    audio_play_sound("minecraft:ui.button.click".into(), 1.0, 1.5).unwrap();
    audio_play_sound_at("minecraft:entity.pig.ambient".into(), 1.0, 64.0, -3.5, 0.5, 1.0).unwrap();
    let sounds = take_sounds();
    assert_eq!(sounds.len(), 2);
    assert_eq!(sounds[0], PendingSound { sound: "minecraft:ui.button.click".into(), at: None, volume: 1.0, pitch: 1.5 });
    assert_eq!(sounds[1].at, Some([1.0, 64.0, -3.5]));
    assert!(take_sounds().is_empty());
    for _ in 0..MAX_PENDING_SOUNDS {
        audio_play_sound("minecraft:ui.button.click".into(), 1.0, 1.0).unwrap();
    }
    assert!(audio_play_sound("minecraft:ui.button.click".into(), 1.0, 1.0).unwrap_err().contains("too many"));
}

/// A fake audio engine: records what it was asked, refuses what it is told to.
#[derive(Default)]
struct Engine {
    open: HashMap<u32, (FAnchor, String, u16, u32)>,
    pushed: HashMap<u32, Vec<f32>>,
    queued: HashMap<u32, usize>,
    take_per_push: Option<usize>,
    refuse_open: bool,
    closed: Vec<u32>,
    moves: Vec<(u32, FAnchor)>,
}

impl StreamBackend for Engine {
    fn open(&mut self, local: u32, anchor: &FAnchor, category: &str, channels: u16, sample_rate: u32) -> Result<(), String> {
        if self.refuse_open {
            return Err("too many streams".into());
        }
        self.open.insert(local, (*anchor, category.to_owned(), channels, sample_rate));
        Ok(())
    }
    fn push(&mut self, local: u32, samples: &[f32]) -> Result<usize, String> {
        let take = self.take_per_push.map_or(samples.len(), |n| n.min(samples.len()));
        self.pushed.entry(local).or_default().extend_from_slice(&samples[..take]);
        *self.queued.entry(local).or_default() += take;
        Ok(take)
    }
    fn queued(&mut self, local: u32) -> Option<usize> {
        self.queued.get(&local).copied()
    }
    fn dropped(&mut self, _: u32) -> Option<u64> {
        Some(7)
    }
    fn move_to(&mut self, local: u32, anchor: &FAnchor) -> Result<(), String> {
        self.moves.push((local, *anchor));
        Ok(())
    }
    fn close(&mut self, local: u32) -> bool {
        self.closed.push(local);
        self.open.remove(&local).is_some()
    }
}

#[test]
fn a_pcm_stream_opens_takes_samples_reports_and_closes_when_the_game_flushes() {
    let mut engine = Engine::default();
    let id = audio_stream_open(FAnchor::Listener, "music".into(), 2, 48_000).unwrap();
    assert_eq!(audio_stream_queued(id), Some(0));
    assert_eq!(audio_stream_push(id, vec![0.25, f32::NAN, 4.0, -4.0]), Ok(4), "non-finite samples are silence, the rest is clamped");
    audio_stream_move(id, FAnchor::Position { x: 1.0, y: 2.0, z: 3.0 }).unwrap();
    assert!(engine.open.is_empty(), "nothing reaches the engine until the game flushes");
    flush_streams(&mut engine);
    assert_eq!(engine.open[&id], (FAnchor::Listener, "music".to_owned(), 2, 48_000));
    assert_eq!(engine.pushed[&id], [0.25, 0.0, 1.0, -1.0]);
    assert_eq!(engine.moves, [(id, FAnchor::Position { x: 1.0, y: 2.0, z: 3.0 })]);
    assert_eq!(audio_stream_queued(id), Some(4));
    assert_eq!(audio_stream_dropped(id), Some(7));
    // a second flush with nothing pending changes nothing and refreshes the counts
    engine.queued.insert(id, 1);
    flush_streams(&mut engine);
    assert_eq!(audio_stream_queued(id), Some(1));
    assert!(audio_stream_close(id));
    assert!(!audio_stream_close(id), "closing twice");
    assert!(audio_stream_push(id, vec![0.0]).is_err());
    assert_eq!(audio_stream_queued(id), None);
    flush_streams(&mut engine);
    assert_eq!(engine.closed, [id]);
    assert_eq!(audio_stream_dropped(id), None, "gone");
}

#[test]
fn a_stream_the_engine_does_not_take_all_of_keeps_the_rest_for_the_next_flush() {
    let mut engine = Engine { take_per_push: Some(3), ..Engine::default() };
    let id = audio_stream_open(FAnchor::Entity(5), "voice".into(), 1, 16_000).unwrap();
    audio_stream_push(id, (0..8).map(|i| i as f32 / 10.0).collect()).unwrap();
    flush_streams(&mut engine);
    flush_streams(&mut engine);
    flush_streams(&mut engine);
    assert_eq!(engine.pushed[&id].len(), 8);
    assert_eq!(engine.pushed[&id][7], 0.7);
}

#[test]
fn a_stream_the_engine_refuses_is_dead_with_the_reason_and_is_not_retried() {
    let mut engine = Engine { refuse_open: true, ..Engine::default() };
    let id = audio_stream_open(FAnchor::Listener, "ui".into(), 1, 44_100).unwrap();
    flush_streams(&mut engine);
    let error = audio_stream_push(id, vec![0.0]).unwrap_err();
    assert!(error.contains("dead") && error.contains("too many streams"), "{error}");
    engine.refuse_open = false;
    flush_streams(&mut engine);
    assert!(engine.open.is_empty(), "not retried");
    assert!(audio_stream_close(id));
    flush_streams(&mut engine);
    assert!(audio_stream_queued(id).is_none());
}

#[test]
fn stream_arguments_and_quotas_are_checked() {
    assert!(audio_stream_open(FAnchor::Listener, "nonsense".into(), 1, 44_100).is_err());
    assert!(audio_stream_open(FAnchor::Listener, "ui".into(), 0, 44_100).is_err());
    assert!(audio_stream_open(FAnchor::Listener, "ui".into(), 9, 44_100).is_err());
    assert!(audio_stream_open(FAnchor::Listener, "ui".into(), 1, 100).is_err());
    assert!(audio_stream_open(FAnchor::Position { x: f64::NAN, y: 0.0, z: 0.0 }, "ui".into(), 1, 44_100).is_err());
    let ids: Vec<u32> = (0..MAX_STREAMS).map(|_| audio_stream_open(FAnchor::Listener, "ui".into(), 1, 44_100).unwrap()).collect();
    assert!(audio_stream_open(FAnchor::Listener, "ui".into(), 1, 44_100).unwrap_err().contains("too many"));
    assert!(ids.windows(2).all(|w| w[0] < w[1]), "ids are never reused");
    let id = ids[0];
    assert_eq!(audio_stream_push(id, vec![0.0; MAX_PENDING_SAMPLES + 5]), Ok(MAX_PENDING_SAMPLES as u32), "the rest is refused, not dropped silently");
    assert_eq!(audio_stream_push(id, vec![0.0]), Ok(0));
    assert!(audio_stream_push(9999, vec![0.0]).is_err());
    assert!(audio_stream_move(9999, FAnchor::Listener).is_err());
}

#[test]
fn shutdown_closes_the_games_streams_and_forgets_everything_but_never_reuses_an_id() {
    let mut engine = Engine::default();
    let id = audio_stream_open(FAnchor::Listener, "ui".into(), 1, 44_100).unwrap();
    flush_streams(&mut engine);
    ime_set_allowed(true);
    audio_play_sound("minecraft:ui.button.click".into(), 1.0, 1.0).unwrap();
    shutdown(&mut engine);
    assert_eq!(engine.closed, [id]);
    assert!(take_sounds().is_empty());
    assert_eq!(ime_allowed(), None);
    let next = audio_stream_open(FAnchor::Listener, "ui".into(), 1, 44_100).unwrap();
    assert!(next > id, "a stale handle of the old session never names a new stream");
}

#[test]
fn ime_allowed_area_and_events() {
    assert!(!ime_claimed());
    assert_eq!(ime_event(), None);
    assert!(ime_set_cursor_area(f32::NAN, 0.0, 1.0, 1.0).is_err());
    assert!(ime_set_cursor_area(0.0, 0.0, -1.0, 1.0).is_err());
    assert_eq!(ime_area(), None);
    ime_set_cursor_area(10.0, 20.0, 30.0, 12.0).unwrap();
    assert_eq!(ime_area(), Some([10.0, 20.0, 30.0, 12.0]));
    ime_set_allowed(true);
    assert!(ime_claimed());
    ime_set_allowed(false);
    assert!(!ime_claimed());
    assert_eq!(ime_allowed(), Some(false), "a module that handed the input method back is still 'used'");
    assert_eq!(ime_area(), None, "handing it back drops the area");

    // events: a byte range in UTF-8, capped text
    set_ime_event(Some(preedit_event("かな", Some((0, 3)))));
    assert_eq!(ime_event(), Some(FImeEvent::Preedit { text: "かな".into(), cursor: Some((0, 3)) }));
    set_ime_event(Some(commit_event("漢字")));
    assert_eq!(ime_event(), Some(FImeEvent::Commit { text: "漢字".into() }));
    set_ime_event(None);
    assert_eq!(ime_event(), None);
    let long = "あ".repeat(MAX_IME_BYTES);
    let FImeEvent::Commit { text } = commit_event(&long) else { unreachable!() };
    assert!(text.len() <= MAX_IME_BYTES && text.len() > MAX_IME_BYTES - 3 && text.chars().all(|c| c == 'あ'), "cut at a character");
    assert_eq!(preedit_event("", None), FImeEvent::Preedit { text: String::new(), cursor: None });
}

#[test]
fn the_gpu_services_refuse_without_a_gpu_or_a_renderer_and_never_panic() {
    // no device in a unit test: the service reports it as an error (the S route does the same). The renderer's GPU
    // runtime is process-wide and `wasm_mods::full::gpu_tests` configures a real device when the machine has one
    // (and leaves it), so take its test lock and expect the shader refusal only when no device is configured.
    let _gpu = horizon_client_render::gfx::mod_gpu_runtime::test_guard();
    let error = gpu_buffer(16, 1).unwrap_err();
    assert!(!error.is_empty());
    if error.contains("not ready") {
        assert!(gpu_shader("fn main() {}".into()).is_err());
        assert!(gpu_frame().is_err());
        assert!(gpu_poll(1).is_err());
    } else {
        // a device is configured: the calls must still answer without panicking
        let _ = (gpu_shader("fn main() {}".into()), gpu_frame(), gpu_poll(1));
    }
}

#[cfg(feature = "yoake2-slice")]
#[path = "guest_tests.rs"]
mod guest_tests;
