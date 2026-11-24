//! REM-12: the per-thread state, the limits and the game-side flush of the F client services
//! (`yoake2_f_services.rs` has the wire types and the bound functions; this file is not bound).

use std::cell::RefCell;

pub(super) use super::yoake2_f_services::{FAnchor, FHud, FImeEvent};

/// Most HUD commands one frame accepts (the S route's `MAX_COMMANDS`).
pub(super) const MAX_HUD_COMMANDS: usize = 8192;
/// Longest text, sprite or texture name of one HUD command, in bytes.
pub(super) const MAX_NAME_BYTES: usize = 4096;
/// Sounds waiting for the next frame (the game's own quota, `MAX_PENDING_SOUNDS`, is 64 too).
pub(super) const MAX_PENDING_SOUNDS: usize = 64;
/// PCM streams one session may have open through F.
pub(super) const MAX_STREAMS: usize = 16;
/// Interleaved samples one stream may hold that the game has not taken yet.
pub(super) const MAX_PENDING_SAMPLES: usize = 1 << 20;
/// Longest IME text an event carries, in bytes.
pub(super) const MAX_IME_BYTES: usize = 4096;

// ---------------------------------------------------------------------------------------------------------------
// The per-thread state
// ---------------------------------------------------------------------------------------------------------------

/// A sound waiting for the next frame.
#[derive(Clone, Debug, PartialEq)]
pub struct PendingSound {
    pub sound: String,
    pub at: Option<[f64; 3]>,
    pub volume: f32,
    pub pitch: f32,
}

#[derive(Debug)]
struct Stream {
    anchor: FAnchor,
    category: String,
    channels: u16,
    sample_rate: u32,
    /// The anchor to move the stream to at the next flush.
    moved: Option<FAnchor>,
    pending: Vec<f32>,
    /// The game has the stream open.
    open: bool,
    closing: bool,
    /// The game refused to open it (the reason): the stream is dead.
    failed: Option<String>,
    queued: u32,
    dropped: u64,
}

#[derive(Default)]
struct State {
    hud_open: bool,
    hud: Vec<FHud>,
    gui: (f32, f32),
    scale: f32,
    sounds: Vec<PendingSound>,
    streams: Vec<(u32, Stream)>,
    next_stream: u32,
    ime_allowed: Option<bool>,
    ime_area: Option<[f32; 4]>,
    ime_event: Option<FImeEvent>,
}

thread_local! {
    static STATE: RefCell<State> = RefCell::new(State::default());
}

fn with<R>(f: impl FnOnce(&mut State) -> R) -> R {
    STATE.with(|state| f(&mut state.borrow_mut()))
}

fn finite(values: &[f32]) -> bool {
    values.iter().all(|value| value.is_finite())
}

// ---------------------------------------------------------------------------------------------------------------
// HUD
// ---------------------------------------------------------------------------------------------------------------

fn hud_command_ok(command: &FHud) -> Result<(), String> {
    let name_ok = |name: &str| name.len() <= MAX_NAME_BYTES;
    let ok = match command {
        FHud::Text { text, .. } => name_ok(text),
        FHud::Sprite { name, .. } => name_ok(name),
        FHud::Item { item, .. } => name_ok(item),
        FHud::Texture { id, .. } => name_ok(id),
        FHud::Quad { corners, uv, sprite, .. } => {
            corners.len() == 4 && uv.as_ref().is_none_or(|uv| uv.len() == 4) && sprite.as_deref().is_none_or(name_ok)
        }
        _ => true,
    };
    if ok { Ok(()) } else { Err("a HUD command is malformed (text or name over 4096 bytes, or a quad without four corners)".to_owned()) }
}

/// Queues HUD commands for the frame being drawn; they are applied to the frame's batch, in order, after the
/// `HUD_RENDER` hook returned, with the S route's HUD code (non-finite numbers become 0; commands the batch cannot
/// draw are skipped). Only allowed inside the `HUD_RENDER` hook. Returns how many were queued; commands past the
/// per-frame limit of 8192 are refused with an error and none of the call's commands are queued.
pub(super) fn hud_draw(commands: Vec<FHud>) -> Result<u32, String> {
    with(|state| {
        if !state.hud_open {
            return Err("hud_draw is only allowed inside the HUD_RENDER hook".to_owned());
        }
        if state.hud.len() + commands.len() > MAX_HUD_COMMANDS {
            return Err(format!("a frame takes at most {MAX_HUD_COMMANDS} HUD commands"));
        }
        for command in &commands {
            hud_command_ok(command)?;
        }
        let count = commands.len() as u32;
        state.hud.extend(commands);
        Ok(count)
    })
}

/// The GUI size of the frame in GUI pixels (the batch's width and height); `(0, 0)` before the first frame.
pub(super) fn hud_gui_size() -> (f32, f32) {
    with(|state| state.gui)
}

/// The GUI scale (window pixels per GUI pixel) of the last frame; `0` before the first frame.
pub(super) fn hud_gui_scale() -> f32 {
    with(|state| state.scale)
}

// ---------------------------------------------------------------------------------------------------------------
// Audio
// ---------------------------------------------------------------------------------------------------------------

fn sound_ok(sound: &str, volume: f32, pitch: f32, at: Option<[f64; 3]>) -> Result<(), String> {
    let id_ok = sound.len() <= 256 && sound.split_once(':').is_some_and(|(namespace, path)| !namespace.is_empty() && !path.is_empty());
    let values_ok = volume.is_finite() && (0.0..=16.0).contains(&volume) && pitch.is_finite() && (0.0..=8.0).contains(&pitch);
    let at_ok = at.is_none_or(|point| point.iter().all(|c| c.is_finite()));
    if id_ok && values_ok && at_ok {
        Ok(())
    } else {
        Err("sound refused: not namespace:path, a volume outside 0 to 16, a pitch outside 0 to 8 or a position that is not finite".to_owned())
    }
}

fn queue_sound(sound: String, at: Option<[f64; 3]>, volume: f32, pitch: f32) -> Result<(), String> {
    sound_ok(&sound, volume, pitch, at)?;
    with(|state| {
        if state.sounds.len() >= MAX_PENDING_SOUNDS {
            return Err("sound refused: too many waiting".to_owned());
        }
        state.sounds.push(PendingSound { sound, at, volume, pitch });
        Ok(())
    })
}

/// Plays a sound event (`namespace:path`) on this client only, as a UI sound (no attenuation); it plays when the
/// next frame starts. Volume 0 to 16, pitch 0 to 8; an event the resources do not have plays nothing.
pub(super) fn audio_play_sound(sound: String, volume: f32, pitch: f32) -> Result<(), String> {
    queue_sound(sound, None, volume, pitch)
}

/// Like [`audio_play_sound`], at a place in the world.
pub(super) fn audio_play_sound_at(sound: String, x: f64, y: f64, z: f64, volume: f32, pitch: f32) -> Result<(), String> {
    queue_sound(sound, Some([x, y, z]), volume, pitch)
}

fn anchor_ok(anchor: &FAnchor) -> Result<(), String> {
    match anchor {
        FAnchor::Position { x, y, z } if ![x, y, z].iter().all(|c| c.is_finite()) => Err("invalid PCM anchor".to_owned()),
        _ => Ok(()),
    }
}

/// The sound categories a stream may be opened in.
pub const CATEGORIES: [&str; 14] = [
    "master", "music", "record", "records", "weather", "block", "blocks", "hostile", "neutral", "player", "players", "ambient", "voice", "ui",
];

/// Opens a PCM stream: interleaved `f32` samples at `sample_rate`, `channels` of them, heard from `anchor` in the
/// sound `category` (`"master"`, `"music"`, `"record"`, `"weather"`, `"block"`, `"hostile"`, `"neutral"`, `"player"`,
/// `"ambient"`, `"voice"`, `"ui"`). The game opens it when the hook returns; a refusal then (the game's quotas)
/// shows as an error of the next call that names the stream. At most 16 streams are open through F at a time.
pub(super) fn audio_stream_open(anchor: FAnchor, category: String, channels: u16, sample_rate: u32) -> Result<u32, String> {
    anchor_ok(&anchor)?;
    if !CATEGORIES.contains(&category.as_str()) {
        return Err(format!("unknown sound category {category:.32}"));
    }
    if channels == 0 || channels > 8 || !(8_000..=192_000).contains(&sample_rate) {
        return Err("a PCM stream takes 1 to 8 channels and 8000 to 192000 Hz".to_owned());
    }
    with(|state| {
        if state.streams.len() >= MAX_STREAMS {
            return Err("too many PCM streams".to_owned());
        }
        state.next_stream = state.next_stream.checked_add(1).ok_or("PCM stream handles exhausted")?;
        let id = state.next_stream;
        state.streams.push((
            id,
            Stream {
                anchor,
                category,
                channels,
                sample_rate,
                moved: None,
                pending: Vec::new(),
                open: false,
                closing: false,
                failed: None,
                queued: 0,
                dropped: 0,
            },
        ));
        Ok(id)
    })
}

fn live_stream<R>(stream: u32, f: impl FnOnce(&mut Stream) -> Result<R, String>) -> Result<R, String> {
    with(|state| {
        let (_, entry) = state.streams.iter_mut().find(|(id, _)| *id == stream).ok_or("invalid or foreign PCM stream")?;
        if let Some(reason) = &entry.failed {
            return Err(format!("the PCM stream is dead: {reason}"));
        }
        if entry.closing {
            return Err("invalid or foreign PCM stream".to_owned());
        }
        f(entry)
    })
}

/// Queues interleaved samples (at most about a million waiting per stream). The game takes them when the hook
/// returns. Returns how many were queued; the rest is refused (call again later), never silently dropped here.
pub(super) fn audio_stream_push(stream: u32, samples: Vec<f32>) -> Result<u32, String> {
    live_stream(stream, |entry| {
        let room = MAX_PENDING_SAMPLES.saturating_sub(entry.pending.len());
        let take = samples.len().min(room);
        entry.pending.extend(samples[..take].iter().map(|s| if s.is_finite() { s.clamp(-1.0, 1.0) } else { 0.0 }));
        Ok(take as u32)
    })
}

/// How many interleaved samples the game's stream holds (as of the last frame), or none for an unknown stream.
pub(super) fn audio_stream_queued(stream: u32) -> Option<u32> {
    with(|state| state.streams.iter().find(|(id, e)| *id == stream && !e.closing).map(|(_, e)| e.queued))
}

/// How many samples the game dropped on overflow since the stream opened (as of the last frame).
pub(super) fn audio_stream_dropped(stream: u32) -> Option<u64> {
    with(|state| state.streams.iter().find(|(id, e)| *id == stream && !e.closing).map(|(_, e)| e.dropped))
}

/// Moves the stream's anchor (applied when the hook returns).
pub(super) fn audio_stream_move(stream: u32, anchor: FAnchor) -> Result<(), String> {
    anchor_ok(&anchor)?;
    live_stream(stream, |entry| {
        entry.moved = Some(anchor);
        Ok(())
    })
}

/// Closes the stream (applied when the hook returns); false for an unknown stream.
pub(super) fn audio_stream_close(stream: u32) -> bool {
    with(|state| match state.streams.iter_mut().find(|(id, e)| *id == stream && !e.closing) {
        Some((_, entry)) => {
            entry.closing = true;
            entry.pending.clear();
            true
        }
        None => false,
    })
}

// ---------------------------------------------------------------------------------------------------------------
// IME
// ---------------------------------------------------------------------------------------------------------------

/// Takes the input method for this module: while allowed, the engine's own text fields and the overlay it draws for
/// a composition step aside, `IME_PREEDIT` and `IME_COMMIT` deliver the text, and the module draws the composition
/// itself. Set it back to false when the module's text field loses focus (the engine's fields then work again).
pub(super) fn ime_set_allowed(allowed: bool) {
    with(|state| {
        state.ime_allowed = Some(allowed);
        if !allowed {
            state.ime_area = None;
        }
    });
}

/// Where the input method's candidate window goes: a rectangle in GUI pixels (finite, with a size of 0 or more).
pub(super) fn ime_set_cursor_area(x: f32, y: f32, w: f32, h: f32) -> Result<(), String> {
    if !finite(&[x, y, w, h]) || w < 0.0 || h < 0.0 {
        return Err("IME cursor area needs finite coordinates and nonnegative size".to_owned());
    }
    with(|state| state.ime_area = Some([x, y, w, h]));
    Ok(())
}

/// The event of the running `IME_PREEDIT` or `IME_COMMIT` hook; none outside them.
pub(super) fn ime_event() -> Option<FImeEvent> {
    with(|state| state.ime_event.clone())
}

pub(super) fn gpu<R>(f: impl FnOnce(&mut crate::entry::wasm_mods::FGpuHost) -> Result<R, String>) -> Result<R, String> {
    crate::entry::wasm_mods::with_f_gpu(f)
}

// ---------------------------------------------------------------------------------------------------------------
// The game's side: what the flush points of `yoake2_client.rs` and `ime.rs` call
// ---------------------------------------------------------------------------------------------------------------

/// Before the `HUD_RENDER` hook: opens `hud_draw` for this frame.
pub(super) fn hud_begin(gui: (f32, f32), scale: f32) {
    with(|state| {
        state.hud_open = true;
        state.hud.clear();
        state.gui = gui;
        state.scale = scale;
    });
}

/// After the hook: closes `hud_draw` and returns the commands of the frame.
pub(super) fn hud_end() -> Vec<FHud> {
    with(|state| {
        state.hud_open = false;
        std::mem::take(&mut state.hud)
    })
}

/// Sounds queued since the last call.
pub(super) fn take_sounds() -> Vec<PendingSound> {
    with(|state| std::mem::take(&mut state.sounds))
}

/// The game's audio engine as the stream flush sees it; streams are named by the local ids [`audio_stream_open`]
/// returned.
pub(super) trait StreamBackend {
    fn open(&mut self, local: u32, anchor: &FAnchor, category: &str, channels: u16, sample_rate: u32) -> Result<(), String>;
    /// Pushes samples; returns how many the engine took.
    fn push(&mut self, local: u32, samples: &[f32]) -> Result<usize, String>;
    fn queued(&mut self, local: u32) -> Option<usize>;
    fn dropped(&mut self, local: u32) -> Option<u64>;
    fn move_to(&mut self, local: u32, anchor: &FAnchor) -> Result<(), String>;
    fn close(&mut self, local: u32) -> bool;
}

/// Applies the streams' pending work: open, move, push, close, then refreshes `queued` and `dropped`. A stream the
/// engine refuses to open or to feed is dead (its error shows in the next call naming it) and is closed.
pub(super) fn flush_streams(backend: &mut dyn StreamBackend) {
    let mut streams = with(|state| std::mem::take(&mut state.streams));
    for (local, stream) in &mut streams {
        if stream.failed.is_none() && !stream.open && !stream.closing {
            match backend.open(*local, &stream.anchor, &stream.category, stream.channels, stream.sample_rate) {
                Ok(()) => stream.open = true,
                Err(reason) => stream.failed = Some(reason),
            }
        }
        if stream.open && !stream.closing {
            if let Some(anchor) = stream.moved.take() {
                if let Err(reason) = backend.move_to(*local, &anchor) {
                    stream.failed = Some(reason);
                } else {
                    stream.anchor = anchor;
                }
            }
            if stream.failed.is_none() && !stream.pending.is_empty() {
                match backend.push(*local, &stream.pending) {
                    Ok(taken) => {
                        stream.pending.drain(..taken.min(stream.pending.len()));
                    }
                    Err(reason) => stream.failed = Some(reason),
                }
            }
            stream.queued = backend.queued(*local).unwrap_or(0).min(u32::MAX as usize) as u32;
            stream.dropped = backend.dropped(*local).unwrap_or(stream.dropped);
        }
        if stream.failed.is_some() && stream.open {
            backend.close(*local);
            stream.open = false;
            stream.pending.clear();
        }
        if stream.closing && stream.open {
            backend.close(*local);
            stream.open = false;
        }
    }
    streams.retain(|(_, stream)| !(stream.closing && !stream.open));
    with(|state| {
        // streams opened by a callback of the backend (none) would be in `state.streams`: keep them
        streams.append(&mut state.streams);
        state.streams = streams;
    });
}

/// Session end: closes every stream the game has open and forgets all queued work and IME state.
pub(super) fn shutdown(backend: &mut dyn StreamBackend) {
    for (local, stream) in with(|state| std::mem::take(&mut state.streams)) {
        if stream.open {
            backend.close(local);
        }
    }
    reset();
}

/// Forgets everything (a test, or a session that ends).
pub(super) fn reset() {
    with(|state| *state = State { next_stream: state.next_stream, ..State::default() });
}

/// The IME state for the window: `Some(true)` while a module took the input method.
pub(super) fn ime_allowed() -> Option<bool> {
    with(|state| state.ime_allowed)
}

/// The candidate window's rectangle in GUI pixels, if a module set one.
pub(super) fn ime_area() -> Option<[f32; 4]> {
    with(|state| state.ime_area)
}

/// Whether a module owns the text input right now.
pub(super) fn ime_claimed() -> bool {
    ime_allowed() == Some(true)
}

/// Publishes the event of the IME hook about to run; `None` ends it.
pub(super) fn set_ime_event(event: Option<FImeEvent>) {
    with(|state| state.ime_event = event);
}

/// A preedit as an event (the text capped, the cursor in `u32`).
pub(super) fn preedit_event(text: &str, cursor: Option<(usize, usize)>) -> FImeEvent {
    let cursor = cursor.and_then(|(a, b)| Some((u32::try_from(a).ok()?, u32::try_from(b).ok()?)));
    FImeEvent::Preedit { text: cap_text(text), cursor }
}

pub(super) fn commit_event(text: &str) -> FImeEvent {
    FImeEvent::Commit { text: cap_text(text) }
}

fn cap_text(text: &str) -> String {
    if text.len() <= MAX_IME_BYTES {
        return text.to_owned();
    }
    let mut end = MAX_IME_BYTES;
    while !text.is_char_boundary(end) {
        end -= 1;
    }
    text[..end].to_owned()
}

#[cfg(test)]
mod tests;
