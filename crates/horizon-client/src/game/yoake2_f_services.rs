//! REM-12: the client host services of full-access (route F) modules: a drawable `HUD_RENDER`, sounds and PCM
//! streams, GUI size queries, an IME route and the GPU, the same things a sandboxed `client-full` module (route S)
//! gets from `wasm_mods/client_services.rs` and `wasm_mods/gpu_services.rs`.
//!
//! The functions of this file are the guest-facing bindings: `yoake-bindgen` reads them (target `f_services`) and
//! generates the `engine_call` glue and the guest stubs, `yoake_glue::call` here and
//! `crates/yoake-guest-sdk/src/stubs/f_services.rs`. They are free functions, so they work in every client hook
//! (`CLIENT_TICK`, `HUD_RENDER`, `GPU_FRAME`, the IME hooks) without a `Game` handle. What needs the `Game` is queued
//! in per-thread state here and applied by the game after the hook returned (`yoake2_client.rs`): the HUD commands to
//! the frame's `HudBatch` through the S route's `apply_hud`, the sounds and PCM streams to the audio engine, the IME
//! state to the window. The state belongs to the client thread, as the F runtime does; all client modules of a
//! session share it (the services do not know which module is calling).
//!
//! This file is the bound surface only (wire types and the guest-facing functions): `yoake-bindgen` binds every
//! function of it, so the state, the limits and the game-side flush live in `yoake2_f_state.rs`, which is not a
//! binding target. Nothing here does anything until a module calls it.


// ---------------------------------------------------------------------------------------------------------------
// Wire types: plain data, the shapes of `yoake-full.wit`'s `hud-command` and `pcm-anchor`
// ---------------------------------------------------------------------------------------------------------------

/// One HUD command, 0xAARRGGBB colours. The same set as the S route's `hud-command`
/// (`crates/yoake-wasm/wit/yoake-full.wit`), applied by the same code (`wasm_mods::full::apply_hud`).
#[derive(Clone, Debug, PartialEq)]
pub enum FHud {
    /// Text at a point (GUI pixels); `color` is 0xRRGGBB, the alpha is the default text colour's.
    Text { text: String, x: f32, y: f32, color: u32, shadow: bool },
    /// A flat rectangle.
    Rect { x: f32, y: f32, width: f32, height: f32, color: u32 },
    /// A vertical gradient rectangle.
    Gradient { x: f32, y: f32, width: f32, height: f32, top: u32, bottom: u32 },
    /// Four corners in order round the shape, flat or with a sprite (and optionally its four UV corners).
    Quad { corners: Vec<(f32, f32)>, sprite: Option<String>, uv: Option<Vec<(f32, f32)>>, color: u32 },
    /// A GUI sprite by name; `size` none draws it as large as it is, `fraction` cuts the width, `region` cuts a texture.
    Sprite { name: String, x: f32, y: f32, size: Option<(f32, f32)>, color: u32, fraction: Option<f32>, region: Option<FRegion> },
    /// An item as in an inventory slot (`item` is `namespace:path`).
    Item { item: String, count: u32, x: f32, y: f32, decorations: bool },
    /// A texture a mod registered through the native GUI textures, by id.
    Texture { id: String, x: f32, y: f32, width: f32, height: f32, color: u32 },
    /// A selection highlight; `invert_text` inverts what is under it.
    Highlight { x: f32, y: f32, width: f32, height: f32, invert_text: bool },
    /// Starts a run that `EndClip` cuts to the rectangle.
    BeginClip { x0: f32, y0: f32, x1: f32, y1: f32 },
    EndClip,
    /// Starts a run that `EndScale` scales about a point.
    BeginScale { center_x: f32, center_y: f32, scale_x: f32, scale_y: f32 },
    EndScale,
    /// Starts a run drawn with a blend mode until `EndBlend`.
    BeginBlend(FBlend),
    EndBlend,
}

/// A part of a texture (pixels) of a sprite command.
#[derive(Clone, Debug, PartialEq)]
pub struct FRegion {
    pub x: f32,
    pub y: f32,
    pub width: f32,
    pub height: f32,
    pub texture_width: f32,
    pub texture_height: f32,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum FBlend {
    Alpha,
    Invert,
    Additive,
    Vignette,
}

/// Where a PCM stream is heard from.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum FAnchor {
    /// Relative to the listener, no attenuation.
    Listener,
    /// Follows an entity (its network id).
    Entity(u32),
    /// A place in the world.
    Position { x: f64, y: f64, z: f64 },
}

/// What the IME hooks deliver, read with [`ime_event`] inside the hook.
#[derive(Clone, Debug, PartialEq)]
pub enum FImeEvent {
    /// The composition: the text, and the selected UTF-8 byte range of it. Empty text clears the composition.
    Preedit { text: String, cursor: Option<(u32, u32)> },
    /// Text the input method committed.
    Commit { text: String },
}

// ---------------------------------------------------------------------------------------------------------------
// Guest-facing functions (thin: the logic and the per-thread state are in `yoake2_f_state.rs`)
// ---------------------------------------------------------------------------------------------------------------

/// Queues HUD commands for the frame being drawn; they are applied to the frame's batch, in order, after the
/// `HUD_RENDER` hook returned, with the S route's HUD code (non-finite numbers become 0; commands the batch cannot
/// draw are skipped). Only allowed inside the `HUD_RENDER` hook. Returns how many were queued; commands past the
/// per-frame limit of 8192 are refused with an error and none of the call's commands are queued.
pub fn hud_draw(commands: Vec<FHud>) -> Result<u32, String> {
    super::yoake2_f_state::hud_draw(commands)
}

/// The GUI size of the frame in GUI pixels (the batch's width and height); `(0, 0)` before the first frame.
pub fn hud_gui_size() -> (f32, f32) {
    super::yoake2_f_state::hud_gui_size()
}

/// The GUI scale (window pixels per GUI pixel) of the last frame; `0` before the first frame.
pub fn hud_gui_scale() -> f32 {
    super::yoake2_f_state::hud_gui_scale()
}

/// Plays a sound event (`namespace:path`) on this client only, as a UI sound (no attenuation); it plays when the
/// next frame starts. Volume 0 to 16, pitch 0 to 8; an event the resources do not have plays nothing.
pub fn audio_play_sound(sound: String, volume: f32, pitch: f32) -> Result<(), String> {
    super::yoake2_f_state::audio_play_sound(sound, volume, pitch)
}

/// Like [`audio_play_sound`], at a place in the world.
pub fn audio_play_sound_at(sound: String, x: f64, y: f64, z: f64, volume: f32, pitch: f32) -> Result<(), String> {
    super::yoake2_f_state::audio_play_sound_at(sound, x, y, z, volume, pitch)
}

/// Opens a PCM stream: interleaved `f32` samples at `sample_rate`, `channels` of them, heard from `anchor` in the
/// sound `category` (`"master"`, `"music"`, `"record"`, `"weather"`, `"block"`, `"hostile"`, `"neutral"`, `"player"`,
/// `"ambient"`, `"voice"`, `"ui"`). The game opens it when the hook returns; a refusal then (the game's quotas)
/// shows as an error of the next call that names the stream. At most 16 streams are open through F at a time.
pub fn audio_stream_open(anchor: FAnchor, category: String, channels: u16, sample_rate: u32) -> Result<u32, String> {
    super::yoake2_f_state::audio_stream_open(anchor, category, channels, sample_rate)
}

/// Queues interleaved samples (at most about a million waiting per stream). The game takes them when the hook
/// returns. Returns how many were queued; the rest is refused (call again later), never silently dropped here.
pub fn audio_stream_push(stream: u32, samples: Vec<f32>) -> Result<u32, String> {
    super::yoake2_f_state::audio_stream_push(stream, samples)
}

/// How many interleaved samples the game's stream holds (as of the last frame), or none for an unknown stream.
pub fn audio_stream_queued(stream: u32) -> Option<u32> {
    super::yoake2_f_state::audio_stream_queued(stream)
}

/// How many samples the game dropped on overflow since the stream opened (as of the last frame).
pub fn audio_stream_dropped(stream: u32) -> Option<u64> {
    super::yoake2_f_state::audio_stream_dropped(stream)
}

/// Moves the stream's anchor (applied when the hook returns).
pub fn audio_stream_move(stream: u32, anchor: FAnchor) -> Result<(), String> {
    super::yoake2_f_state::audio_stream_move(stream, anchor)
}

/// Closes the stream (applied when the hook returns); false for an unknown stream.
pub fn audio_stream_close(stream: u32) -> bool {
    super::yoake2_f_state::audio_stream_close(stream)
}

/// Takes the input method for this module: while allowed, the engine's own text fields and the overlay it draws for
/// a composition step aside, `IME_PREEDIT` and `IME_COMMIT` deliver the text, and the module draws the composition
/// itself. Set it back to false when the module's text field loses focus (the engine's fields then work again).
pub fn ime_set_allowed(allowed: bool) {
    super::yoake2_f_state::ime_set_allowed(allowed)
}

/// Where the input method's candidate window goes: a rectangle in GUI pixels (finite, with a size of 0 or more).
pub fn ime_set_cursor_area(x: f32, y: f32, w: f32, h: f32) -> Result<(), String> {
    super::yoake2_f_state::ime_set_cursor_area(x, y, w, h)
}

/// The event of the running `IME_PREEDIT` or `IME_COMMIT` hook; none outside them.
pub fn ime_event() -> Option<FImeEvent> {
    super::yoake2_f_state::ime_event()
}

// ---------------------------------------------------------------------------------------------------------------
// GPU (the S route's `gpu-services`, through one GPU identity for all F modules of the session)
// ---------------------------------------------------------------------------------------------------------------

/// `pixel-format`.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum FPixelFormat {
    Rgba8,
    Rgba8Srgb,
    Rgba16f,
    Rgba32f,
    R32f,
    Rg16f,
    Depth32f,
    Bgra8,
    Rgb10a2,
}

/// `stage`: where in the frame a GPU command runs.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum FStage {
    WorldOpaque,
    WorldTranslucent,
    AfterWorld,
    Hud,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum FBindingKind {
    Buffer,
    Texture,
    Sampler,
}

#[derive(Clone, Debug, PartialEq)]
pub struct FTextureDesc {
    pub depth: u32,
    pub format: FPixelFormat,
    pub height: u32,
    pub mips: u32,
    pub usage: u32,
    pub volume: bool,
    pub width: u32,
}

#[derive(Clone, Debug, PartialEq)]
pub struct FAttribute {
    pub components: u8,
    pub location: u32,
    pub offset: u64,
}

#[derive(Clone, Debug, PartialEq)]
pub struct FRenderDesc {
    pub attributes: Vec<FAttribute>,
    pub blend: bool,
    pub cull: bool,
    pub depth: bool,
    pub depth_write: bool,
    pub format: FPixelFormat,
    pub fragment_entry: String,
    pub lines: bool,
    pub shader: u32,
    pub stride: u64,
    pub vertex_entry: String,
}

#[derive(Clone, Debug, PartialEq)]
pub struct FBinding {
    pub binding: u32,
    pub handle: u32,
    pub kind: FBindingKind,
    pub offset: u64,
    pub size: u64,
}

#[derive(Clone, Debug, PartialEq)]
pub struct FBufferRange {
    pub handle: u32,
    pub offset: u64,
    pub size: u64,
}

#[derive(Clone, Debug, PartialEq)]
pub struct FDrawDesc {
    pub count: u32,
    pub groups: Vec<u32>,
    pub index: Option<FBufferRange>,
    pub index32: bool,
    pub instances: u32,
    pub pipeline: u32,
    pub vertex: Option<FBufferRange>,
}

#[derive(Clone, Debug, PartialEq)]
pub struct FGpuRect {
    pub height: u32,
    pub width: u32,
    pub x: u32,
    pub y: u32,
}

#[derive(Clone, Debug, PartialEq)]
pub struct FDrawRegion {
    pub scissor: Option<FGpuRect>,
    pub viewport: Option<FGpuRect>,
}

#[derive(Clone, Debug, PartialEq)]
pub struct FDrawTarget {
    pub clear: Option<(f64, f64, f64, f64)>,
    pub color: Option<u32>,
    pub depth: Option<u32>,
}

#[derive(Clone, Debug, PartialEq)]
pub struct FFrameInfo {
    pub camera: (f64, f64, f64),
    pub delta_seconds: f32,
    pub height: u32,
    pub hud_format: Option<FPixelFormat>,
    pub index: u64,
    pub version: u32,
    pub view_projection: Vec<f32>,
    pub width: u32,
    pub world_format: Option<FPixelFormat>,
}

/// A shader the GPU refused: the line and column are 1-based, 0 when unknown.
#[derive(Clone, Debug, PartialEq)]
pub struct FShaderError {
    pub column: u32,
    pub line: u32,
    pub message: String,
}

// The GPU functions: arguments and results as in the S route (`gpu-services` of `yoake-full.wit`), a refusal is the
// `Err`. Resources belong to the F identity of the session and are released when the client mods unload.


pub fn gpu_shader(source: String) -> Result<u32, FShaderError> {
    super::yoake2_f_state::gpu(|host| Ok(host.shader(source))).map_err(|message| FShaderError { column: 0, line: 0, message })?
}

pub fn gpu_shader_file(path: String, source: String) -> Result<u32, FShaderError> {
    super::yoake2_f_state::gpu(|host| Ok(host.shader_file(path, source))).map_err(|message| FShaderError { column: 0, line: 0, message })?
}

pub fn gpu_buffer(size: u64, usage: u32) -> Result<u32, String> {
    super::yoake2_f_state::gpu(|host| host.buffer(size, usage))
}

pub fn gpu_texture(desc: FTextureDesc) -> Result<u32, String> {
    super::yoake2_f_state::gpu(|host| host.texture(desc))
}

pub fn gpu_sampler(linear: bool, repeat: bool) -> Result<u32, String> {
    super::yoake2_f_state::gpu(|host| host.sampler(linear, repeat))
}

pub fn gpu_compute(entry: String, shader: u32) -> Result<u32, String> {
    super::yoake2_f_state::gpu(|host| host.compute(entry, shader))
}

pub fn gpu_render(desc: FRenderDesc) -> Result<u32, String> {
    super::yoake2_f_state::gpu(|host| host.render(desc))
}

pub fn gpu_bind_group(bindings: Vec<FBinding>, index: u32, pipeline: u32) -> Result<u32, String> {
    super::yoake2_f_state::gpu(|host| host.bind_group(bindings, index, pipeline))
}

pub fn gpu_destroy(handle: u32) -> Result<(), String> {
    super::yoake2_f_state::gpu(|host| host.destroy(handle))
}

pub fn gpu_write_buffer(bytes: Vec<u8>, handle: u32, offset: u64, stage: FStage) -> Result<(), String> {
    super::yoake2_f_state::gpu(|host| host.write_buffer(bytes, handle, offset, stage))
}

pub fn gpu_copy_buffer(destination: u32, destination_offset: u64, size: u64, source: u32, source_offset: u64, stage: FStage) -> Result<(), String> {
    super::yoake2_f_state::gpu(|host| host.copy_buffer(destination, destination_offset, size, source, source_offset, stage))
}

pub fn gpu_write_texture(bytes: Vec<u8>, extent: (u32, u32, u32), handle: u32, mip: u32, origin: (u32, u32, u32), stage: FStage) -> Result<(), String> {
    super::yoake2_f_state::gpu(|host| host.write_texture(bytes, extent, handle, mip, origin, stage))
}

pub fn gpu_dispatch(groups: Vec<u32>, pipeline: u32, stage: FStage, workgroups: (u32, u32, u32)) -> Result<(), String> {
    super::yoake2_f_state::gpu(|host| host.dispatch(groups, pipeline, stage, workgroups))
}

pub fn gpu_draw(desc: FDrawDesc, stage: FStage, target: FDrawTarget) -> Result<(), String> {
    super::yoake2_f_state::gpu(|host| host.draw(desc, stage, target))
}

pub fn gpu_draw_region(desc: FDrawDesc, region: FDrawRegion, stage: FStage, target: FDrawTarget) -> Result<(), String> {
    super::yoake2_f_state::gpu(|host| host.draw_region(desc, region, stage, target))
}

pub fn gpu_read(handle: u32, offset: u64, size: u64, stage: FStage) -> Result<u32, String> {
    super::yoake2_f_state::gpu(|host| host.read(handle, offset, size, stage))
}

pub fn gpu_poll(ticket: u32) -> Result<Option<Vec<u8>>, String> {
    super::yoake2_f_state::gpu(|host| host.poll(ticket))
}

pub fn gpu_frame() -> Result<FFrameInfo, String> {
    super::yoake2_f_state::gpu(|host| host.frame())
}

#[cfg(feature = "yoake2-slice")]
include!("../yoake_glue/f_services.rs");
