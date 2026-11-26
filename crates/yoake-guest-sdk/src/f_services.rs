//! Ergonomic helpers over `stubs::f_services` (REM-12 client HUD, IME, audio, GPU engine calls).

use alloc::string::String;
use alloc::vec::Vec;
use core::fmt;

use crate::stubs::f_services as raw;
pub use crate::stubs::f_services::types::*;
use crate::Error;

/// Error from an F-route host service call: transport/ABI error or service refusal message.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum CallError {
    Transport(Error),
    Service(String),
}

impl fmt::Display for CallError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            CallError::Transport(e) => write!(f, "transport error: {e:?}"),
            CallError::Service(msg) => f.write_str(msg),
        }
    }
}

fn flatten<T>(res: Result<Result<T, String>, Error>) -> Result<T, CallError> {
    match res {
        Ok(Ok(val)) => Ok(val),
        Ok(Err(msg)) => Err(CallError::Service(msg)),
        Err(err) => Err(CallError::Transport(err)),
    }
}

pub fn hud_draw(commands: Vec<FHud>) -> Result<u32, CallError> {
    flatten(raw::f_services_hud_draw(commands))
}

pub fn hud_gui_size() -> Result<(f32, f32), Error> {
    raw::f_services_hud_gui_size()
}

pub fn hud_gui_scale() -> Result<f32, Error> {
    raw::f_services_hud_gui_scale()
}

pub fn ime_event() -> Result<Option<FImeEvent>, Error> {
    raw::f_services_ime_event()
}

pub fn ime_set_allowed(allowed: bool) -> Result<(), Error> {
    raw::f_services_ime_set_allowed(allowed)
}

pub fn ime_set_cursor_area(x: f32, y: f32, w: f32, h: f32) -> Result<(), CallError> {
    flatten(raw::f_services_ime_set_cursor_area(x, y, w, h))
}

pub fn play_sound(sound_id: String, volume: f32, pitch: f32) -> Result<(), CallError> {
    flatten(raw::f_services_audio_play_sound(sound_id, volume, pitch))
}

pub fn play_sound_at(
    sound_id: String,
    x: f64,
    y: f64,
    z: f64,
    volume: f32,
    pitch: f32,
) -> Result<(), CallError> {
    flatten(raw::f_services_audio_play_sound_at(
        sound_id, x, y, z, volume, pitch,
    ))
}

/// Owned PCM audio stream opened through `f_services::audio_stream_*`. Closes on drop.
#[derive(Debug)]
pub struct AudioStream {
    handle: u32,
}

impl AudioStream {
    pub fn open(
        anchor: FAnchor,
        category: String,
        channels: u16,
        sample_rate: u32,
    ) -> Result<Self, CallError> {
        flatten(raw::f_services_audio_stream_open(
            anchor,
            category,
            channels,
            sample_rate,
        ))
        .map(|handle| Self { handle })
    }

    pub fn push(&self, samples: Vec<f32>) -> Result<u32, CallError> {
        flatten(raw::f_services_audio_stream_push(self.handle, samples))
    }

    pub fn queued(&self) -> Result<Option<u32>, Error> {
        raw::f_services_audio_stream_queued(self.handle)
    }

    pub fn dropped(&self) -> Result<Option<u64>, Error> {
        raw::f_services_audio_stream_dropped(self.handle)
    }

    pub fn move_to(&self, anchor: FAnchor) -> Result<(), CallError> {
        flatten(raw::f_services_audio_stream_move(self.handle, anchor))
    }
}

impl Drop for AudioStream {
    fn drop(&mut self) {
        let _ = raw::f_services_audio_stream_close(self.handle);
    }
}
