#![allow(dead_code)]

use std::sync::{Arc, Mutex};

pub type Mods = Arc<[Box<dyn Hooks>]>;

#[derive(Default)]
pub struct TickEvent;
pub struct KeyEvent<'event> { pub key: &'event str, pub pressed: bool }
pub struct HudEvent<'event> { pub batch: &'event mut HudBatch }
pub struct NetFromServerEvent<'event> { pub channel: &'event str, pub data: &'event [u8] }
#[derive(Default)]
pub struct HudBatch { pub text: Vec<String>, pub rects: Vec<[f32; 4]> }
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
    fn text(&mut self, batch: &mut HudBatch, text: &str, at: [f32; 2], rgb: u32, shadow: bool);
    fn fill(&mut self, batch: &mut HudBatch, rect: [f32; 4], argb: u32);
    fn net_send_to_server(&mut self, channel: &str, data: &[u8]) -> Result<(), String>;
    fn setting(&self, mod_id: &str, key: &str) -> Option<String>;
    fn set_setting(&self, mod_id: &str, key: &str, value: &str);
}

pub trait Hooks: Send + Sync {
    fn tick(&self, _host: &mut dyn Host, _event: &mut TickEvent) {}
    fn key(&self, _host: &mut dyn Host, _event: &mut KeyEvent<'_>) {}
    fn hud(&self, _host: &mut dyn Host, _event: &mut HudEvent<'_>) {}
    fn net_from_server(&self, _host: &mut dyn Host, _event: &mut NetFromServerEvent<'_>) {}
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
    fn text(&mut self, batch: &mut HudBatch, text: &str, _at: [f32; 2], _rgb: u32, _shadow: bool) {
        batch.text.push(text.to_owned());
    }
    fn fill(&mut self, batch: &mut HudBatch, rect: [f32; 4], _argb: u32) { batch.rects.push(rect); }
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
}
