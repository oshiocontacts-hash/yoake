#![allow(dead_code)]

use std::{path::PathBuf, sync::{Arc, Mutex}, time::{Duration, Instant}};

pub type Mods = Arc<[Box<dyn Hooks>]>;

#[derive(Default)]
pub struct TickEvent;
pub struct KeyEvent<'event> { pub key: &'event str, pub pressed: bool }
pub struct HudEvent<'event> { pub batch: &'event mut HudBatch }
pub struct NetFromServerEvent<'event> { pub channel: &'event str, pub data: &'event [u8] }
pub struct FrameEvent { pub elapsed: Duration }
pub struct LocalPlayerTickEndEvent;
pub struct NetFramesListenedEvent { pub listened: bool }
pub struct NetFrameReceivedEvent<'event> { pub frame: &'event NetFrameReceived<'event> }
pub struct NetSessionEndedEvent<'event> { pub session: u64, pub reason: &'event str }
pub struct GuiLayerPreEvent<'event> { pub layer: &'event str, pub batch: &'event mut HudBatch }
pub struct GuiLayerPostEvent<'event> { pub layer: &'event str, pub batch: &'event mut HudBatch }
pub struct ScreenRenderPostEvent<'event> { pub screen: &'event NativeScreenView<'event>, pub batch: &'event mut HudBatch }
pub struct ScreenKeyPressedPreEvent<'event> { pub screen: &'event NativeScreenView<'event>, pub key: &'event str, pub repeat: bool, pub modifiers: u32 }
pub struct ScreenCharTypedPreEvent<'event> { pub screen: &'event NativeScreenView<'event>, pub c: char, pub modifiers: u32 }
pub struct ScreenMouseButtonPressedPreEvent<'event> { pub screen: &'event NativeScreenView<'event>, pub button: i32, pub modifiers: u32 }
pub struct ScreenMouseButtonReleasedPreEvent<'event> { pub screen: &'event NativeScreenView<'event>, pub button: i32, pub modifiers: u32 }
pub struct ScreenMouseScrolledPreEvent<'event> { pub screen: &'event NativeScreenView<'event>, pub scroll: [f64; 2], pub modifiers: u32 }
pub struct SettingsEvent<'event> { pub mod_id: &'event str, pub buttons: &'event mut Vec<String> }
pub struct SettingsClickEvent<'event> { pub mod_id: &'event str, pub button: usize }
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Flow { Continue, Cancel }
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum NetState { Login, Configuration, Play }
pub struct NetFrameReceived<'frame> { pub state: NetState, pub id: i32, pub body: &'frame [u8], pub at: Instant, pub session: u64 }
pub struct NativeScreenView<'screen> { pub id: &'screen str, pub rect: [f32; 4], pub mouse: [f32; 2], pub partial_tick: f32 }
#[derive(Clone, Copy)]
pub enum Hand { Main, Off }
#[derive(Default)]
pub struct Pose(pub u32);
impl Pose { pub fn id(&self) -> u32 { self.0 } }
#[derive(Default)]
pub struct LocalPlayerSnapshot {
    pub entity_id: i32, pub uuid: u128, pub pos: [f64; 3],
    pub yaw: f32, pub pitch: f32, pub head_yaw: f32, pub body_yaw: f32,
    pub on_ground: bool, pub pose: Pose, pub sprinting: bool, pub swimming: bool, pub fall_flying: bool,
    pub swing: Option<(Hand, u32)>, pub using_item: Option<Hand>, pub use_ticks: i32,
    pub equipment: [Option<horizon_protocol::item::Stack>; 6], pub vehicle: Option<i32>, pub skin_parts: u8,
    pub main_hand_right: bool, pub dimension: String, pub game_mode: u8,
    pub in_water: bool, pub eye_in_water: bool, pub flying: bool,
    pub health: f32, pub max_health: Option<f32>, pub food_level: i32,
}
pub struct SnapshotReport { pub written: usize, pub missing: Vec<String> }
pub struct Account { pub name: String, pub uuid: u128, pub online: bool }
pub struct NativeGuiTexture;
pub enum NativeGuiTextureFilter { Nearest, Linear }
pub struct NativeGuiTextureError;
pub struct NativeGuiNineSlice { pub source_size: [u32; 2], pub border: [u32; 4] }
pub struct HudVertex { pub color: u32 }
#[derive(Default)]
pub struct HudBatch {
    pub text: Vec<String>, pub rects: Vec<[f32; 4]>, pub vertices: Vec<HudVertex>,
    pub scales: Vec<(usize, [f32; 4])>, pub clips: Vec<(usize, [f32; 4])>, pub textures: usize,
}
impl HudBatch {
    pub fn scale_since(&mut self, start: usize, center_x: f32, center_y: f32, scale_x: f32, scale_y: f32) {
        self.scales.push((start, [center_x, center_y, scale_x, scale_y]));
    }
    pub fn clip_since(&mut self, start: usize, left: f32, top: f32, right: f32, bottom: f32) {
        self.clips.push((start, [left, top, right, bottom]));
    }
}
pub struct Text(pub String);
impl Text { pub fn literal(value: &str) -> Self { Self(value.to_owned()) } }

pub enum NativeClientWorld {
    Singleplayer { folder: String },
    Server { address: String },
    LanGuest { address: String },
    Friend,
    Replay { title: String },
}

pub trait Host {
    fn window(&self) -> bool;
    fn feedback(&mut self, value: Text);
    fn client_world_key(&self) -> Option<NativeClientWorld>;
    fn text(&self, batch: &mut HudBatch, text: &str, at: [f32; 2], rgb: u32, shadow: bool);
    fn fill(&self, batch: &mut HudBatch, rect: [f32; 4], argb: u32);
    fn net_send_to_server(&mut self, channel: &str, data: &[u8]) -> Result<(), String>;
    fn setting(&self, mod_id: &str, key: &str) -> Option<String>;
    fn set_setting(&self, mod_id: &str, key: &str, value: &str);
    fn text_width(&self, text: &str) -> Option<f32>;
    fn local_player_snapshot(&self) -> Option<LocalPlayerSnapshot>;
    fn snapshot_client_world(&mut self, frame: &mut dyn FnMut(NetState, i32, &[u8])) -> SnapshotReport;
    fn playing(&self) -> bool;
    fn instance_dir(&self) -> PathBuf;
    fn account(&self) -> Option<Account>;
    fn mod_screen_open(&mut self, mod_id: &str) -> bool;
    fn mod_screen_close(&mut self, mod_id: &str);
    fn mod_screen(&self) -> Option<&str>;
    fn mod_screen_open_anywhere(&mut self, mod_id: &str) -> bool;
    fn gui_texture_create(&self, size: [u32; 2], format: crate::TextureFormat, filter: NativeGuiTextureFilter) -> Result<NativeGuiTexture, NativeGuiTextureError>;
    fn gui_texture(&self, batch: &mut HudBatch, texture: &NativeGuiTexture, rect: [f32; 4], uv: [f32; 4], argb: u32, nine: Option<NativeGuiNineSlice>) -> bool;
}

pub trait Hooks: Send + Sync {
    fn tick(&self, _host: &mut dyn Host, _event: &mut TickEvent) {}
    fn key(&self, _host: &mut dyn Host, _event: &mut KeyEvent<'_>) {}
    fn hud(&self, _host: &mut dyn Host, _event: &mut HudEvent<'_>) {}
    fn net_from_server(&self, _host: &mut dyn Host, _event: &mut NetFromServerEvent<'_>) {}
    fn frame(&self, _host: &mut dyn Host, _event: &mut FrameEvent) {}
    fn local_player_tick_end(&self, _host: &mut dyn Host, _event: &mut LocalPlayerTickEndEvent) {}
    fn net_frames_listened(&self, _event: &mut NetFramesListenedEvent) {}
    fn net_frame_received(&self, _event: &mut NetFrameReceivedEvent<'_>) {}
    fn net_session_ended(&self, _event: &mut NetSessionEndedEvent<'_>) {}
    fn gui_layer_pre(&self, _host: &mut dyn Host, _event: &mut GuiLayerPreEvent<'_>) -> Flow { Flow::Continue }
    fn gui_layer_post(&self, _host: &mut dyn Host, _event: &mut GuiLayerPostEvent<'_>) {}
    fn screen_render_post(&self, _host: &dyn Host, _event: &mut ScreenRenderPostEvent<'_>) {}
    fn screen_key_pressed_pre(&self, _host: &dyn Host, _event: &mut ScreenKeyPressedPreEvent<'_>) -> Flow { Flow::Continue }
    fn screen_char_typed_pre(&self, _host: &dyn Host, _event: &mut ScreenCharTypedPreEvent<'_>) -> Flow { Flow::Continue }
    fn screen_mouse_button_pressed_pre(&self, _host: &dyn Host, _event: &mut ScreenMouseButtonPressedPreEvent<'_>) -> Flow { Flow::Continue }
    fn screen_mouse_button_released_pre(&self, _host: &dyn Host, _event: &mut ScreenMouseButtonReleasedPreEvent<'_>) -> Flow { Flow::Continue }
    fn screen_mouse_scrolled_pre(&self, _host: &dyn Host, _event: &mut ScreenMouseScrolledPreEvent<'_>) -> Flow { Flow::Continue }
    fn settings(&self, _host: &mut dyn Host, _event: &mut SettingsEvent<'_>) {}
    fn settings_click(&self, _host: &mut dyn Host, _event: &mut SettingsClickEvent<'_>) {}
    fn cancel(&self, _visits: &mut Vec<&'static str>) -> bool { false }
    fn replace(&self) -> Option<&'static str> { None }
    fn collect(&self, _values: &mut Vec<&'static str>) {}
}

#[derive(Default)]
pub struct FakeHost {
    pub feedback: Vec<String>,
    pub sends: Vec<(String, Vec<u8>)>,
    pub settings: Mutex<std::collections::HashMap<(String, String), String>>,
    pub client: bool,
    pub panic_feedback: bool,
    pub screen: Option<String>,
}

impl Host for FakeHost {
    fn window(&self) -> bool { self.client }
    fn feedback(&mut self, value: Text) {
        assert!(!self.panic_feedback, "host feedback panic");
        self.feedback.push(value.0);
    }
    fn client_world_key(&self) -> Option<NativeClientWorld> {
        Some(NativeClientWorld::Server { address: "a".repeat(512) })
    }
    fn text(&self, batch: &mut HudBatch, text: &str, _at: [f32; 2], rgb: u32, _shadow: bool) {
        batch.text.push(text.to_owned());
        batch.vertices.extend((0..6).map(|_| HudVertex { color: rgb | 0xff00_0000 }));
    }
    fn fill(&self, batch: &mut HudBatch, rect: [f32; 4], argb: u32) {
        batch.rects.push(rect);
        batch.vertices.extend((0..6).map(|_| HudVertex { color: argb }));
    }
    fn net_send_to_server(&mut self, channel: &str, data: &[u8]) -> Result<(), String> {
        self.sends.push((channel.to_owned(), data.to_vec()));
        Ok(())
    }
    fn setting(&self, mod_id: &str, key: &str) -> Option<String> {
        self.settings.lock().unwrap().get(&(mod_id.to_owned(), key.to_owned())).cloned()
    }
    fn set_setting(&self, mod_id: &str, key: &str, value: &str) {
        self.settings.lock().unwrap().insert((mod_id.to_owned(), key.to_owned()), value.to_owned());
    }
    fn text_width(&self, text: &str) -> Option<f32> { Some(text.chars().count() as f32 * 6.0) }
    fn local_player_snapshot(&self) -> Option<LocalPlayerSnapshot> {
        Some(LocalPlayerSnapshot {
            entity_id: 42, uuid: 0x112233445566778899aabbccddeeff00, pos: [1.5, 64.0, -2.5],
            yaw: 10.0, pitch: 20.0, head_yaw: 30.0, body_yaw: 40.0,
            on_ground: true, pose: Pose(3), sprinting: true, swimming: true, fall_flying: true,
            swing: Some((Hand::Off, 7)), using_item: Some(Hand::Main), use_ticks: 9,
            vehicle: Some(17), skin_parts: 127, main_hand_right: true, dimension: "minecraft:overworld".into(),
            game_mode: 1, in_water: true, eye_in_water: true, flying: true,
            health: 18.5, max_health: Some(20.0), food_level: 19, ..Default::default()
        })
    }
    fn snapshot_client_world(&mut self, frame: &mut dyn FnMut(NetState, i32, &[u8])) -> SnapshotReport {
        frame(NetState::Configuration, 4, &[0, 1, 255]);
        frame(NetState::Play, 7, &[2, 3]);
        SnapshotReport { written: 2, missing: vec!["optional fixture state".into()] }
    }
    fn playing(&self) -> bool { self.client }
    fn instance_dir(&self) -> PathBuf { PathBuf::from("C:/Yoake/fixture") }
    fn account(&self) -> Option<Account> { Some(Account { name: "fixture".into(), uuid: 123, online: true }) }
    fn mod_screen_open(&mut self, mod_id: &str) -> bool {
        if !self.client { return false; }
        self.mod_screen_open_anywhere(mod_id)
    }
    fn mod_screen_close(&mut self, mod_id: &str) {
        if self.screen.as_deref() == Some(mod_id) { self.screen = None; }
    }
    fn mod_screen(&self) -> Option<&str> { self.screen.as_deref() }
    fn mod_screen_open_anywhere(&mut self, mod_id: &str) -> bool {
        self.screen = Some(mod_id.to_owned());
        true
    }
    fn gui_texture_create(&self, _size: [u32; 2], format: crate::TextureFormat, _filter: NativeGuiTextureFilter) -> Result<NativeGuiTexture, NativeGuiTextureError> {
        assert!(matches!(format, crate::TextureFormat::Rgba8Unorm | crate::TextureFormat::Bgra8Unorm | crate::TextureFormat::Rgba16Float));
        Ok(NativeGuiTexture)
    }
    fn gui_texture(&self, batch: &mut HudBatch, _texture: &NativeGuiTexture, _rect: [f32; 4], _uv: [f32; 4], argb: u32, _nine: Option<NativeGuiNineSlice>) -> bool {
        batch.textures += 1;
        batch.vertices.extend((0..6).map(|_| HudVertex { color: argb }));
        true
    }
}
