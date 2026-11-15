use std::collections::HashMap;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::Arc;

use wasmtime::component::Resource;

use crate::bindings::yoake::mods::{
    accounts, client_world, feedback, hud, instance, logging, network, player, screen, settings,
    types,
};
use types::{
    AccountSnapshot, GuiNineSlice, HostError, PlayerSnapshot, Point, Rect, SendError,
    TextureFilter, TextureFormat, WorldKey, WorldSnapshot,
};

const MAX_TEXTURES: usize = 64;

macro_rules! timed_host_call {
    ($call:expr) => {{
        let started = std::time::Instant::now();
        let result = $call;
        crate::add_host_time(started.elapsed());
        result
    }};
}

macro_rules! host_methods {
    ($(fn $name:ident(&mut self $(, $arg:ident: $arg_type:ty)*) -> $result:ty;)*) => {
        $(
            fn $name(&mut self, $($arg: $arg_type),*) -> Result<$result, HostError> {
                $(let _ = $arg;)*
                Err(HostError::Unsupported)
            }
        )*
    };
}

pub trait ClientHost: Send {
    host_methods! {
        fn log(&mut self, target: &str, message: &str) -> ();
        fn feedback(&mut self, text: &str) -> ();
        fn text(&mut self, text: &str, at: Point, rgb: u32, shadow: bool) -> ();
        fn fill(&mut self, bounds: Rect, argb: u32) -> ();
        fn text_width(&mut self, text: &str) -> Option<f32>;
        fn batch_len(&mut self) -> u64;
        fn batch_scale_since(&mut self, start: u64, origin: Point, scale: Point) -> ();
        fn batch_clip_since(&mut self, start: u64, bounds: Rect) -> ();
        fn batch_tint_since(&mut self, start: u64, alpha: f32) -> ();
        fn gui_texture_create(&mut self, width: u32, height: u32, format: TextureFormat, filter: TextureFilter) -> u64;
        fn gui_texture_draw(&mut self, texture: u64, bounds: Rect, uv: Rect, argb: u32, nine_slice: Option<GuiNineSlice>) -> bool;
        fn setting(&mut self, mod_id: &str, key: &str) -> Option<String>;
        fn set_setting(&mut self, mod_id: &str, key: &str, value: &str) -> ();
        fn client_world_key(&mut self) -> Option<WorldKey>;
        fn snapshot_client_world(&mut self) -> WorldSnapshot;
        fn playing(&mut self) -> bool;
        fn local_player_snapshot(&mut self) -> Option<PlayerSnapshot>;
        fn instance_dir(&mut self) -> String;
        fn account(&mut self) -> Option<AccountSnapshot>;
        fn mod_screen_open(&mut self, mod_id: &str) -> bool;
        fn mod_screen_close(&mut self, mod_id: &str) -> ();
        fn mod_screen(&mut self) -> Option<String>;
        fn mod_screen_open_anywhere(&mut self, mod_id: &str) -> bool;
    }

    fn net_send_to_server(&mut self, _channel: &str, _data: &[u8]) -> Result<(), SendError> {
        Err(SendError {
            status: HostError::Unsupported,
            message: String::new(),
        })
    }

    fn gui_texture_drop(&mut self, _texture: u64) {}
}

pub struct ClientContext {
    id: String,
    host: Box<dyn ClientHost>,
    textures: HashMap<u32, u64>,
    texture_count: Arc<AtomicUsize>,
    next_texture: u32,
}

impl ClientContext {
    pub(crate) fn new(id: String, host: Box<dyn ClientHost>, texture_count: Arc<AtomicUsize>) -> Self {
        Self { id, host, textures: HashMap::new(), texture_count, next_texture: 0 }
    }
}

impl ClientContext {
    pub fn log_output(&mut self, line: &crate::OutputLine) {
        let target = match line.stream {
            crate::OutputStream::Stdout => "stdout",
            crate::OutputStream::Stderr => "stderr",
        };
        let _ = self.host.log(target, &line.text);
    }
}

impl Drop for ClientContext {
    fn drop(&mut self) {
        for (_, texture) in self.textures.drain() {
            self.texture_count.fetch_sub(1, Ordering::AcqRel);
            self.host.gui_texture_drop(texture);
        }
    }
}

impl types::Host for ClientContext {}

impl logging::Host for ClientContext {
    fn log(&mut self, target: String, message: String) -> Result<(), HostError> {
        timed_host_call!(self.host.log(&target, &message))
    }
}

impl feedback::Host for ClientContext {
    fn feedback(&mut self, text: String) -> Result<(), HostError> {
        timed_host_call!(self.host.feedback(&text))
    }
}

impl hud::Host for ClientContext {
    fn text(&mut self, text: String, at: Point, rgb: u32, shadow: bool) -> Result<(), HostError> {
        timed_host_call!(self.host.text(&text, at, rgb, shadow))
    }

    fn fill(&mut self, bounds: Rect, argb: u32) -> Result<(), HostError> {
        timed_host_call!(self.host.fill(bounds, argb))
    }

    fn text_width(&mut self, text: String) -> Result<Option<f32>, HostError> {
        timed_host_call!(self.host.text_width(&text))
    }

    fn batch_len(&mut self) -> Result<u64, HostError> {
        timed_host_call!(self.host.batch_len())
    }

    fn batch_scale_since(&mut self, start: u64, origin: Point, scale: Point) -> Result<(), HostError> {
        timed_host_call!(self.host.batch_scale_since(start, origin, scale))
    }

    fn batch_clip_since(&mut self, start: u64, bounds: Rect) -> Result<(), HostError> {
        timed_host_call!(self.host.batch_clip_since(start, bounds))
    }

    fn batch_tint_since(&mut self, start: u64, alpha: f32) -> Result<(), HostError> {
        timed_host_call!(self.host.batch_tint_since(start, alpha))
    }
}

impl hud::HostGuiTexture for ClientContext {
    fn create(&mut self, width: u32, height: u32, format: TextureFormat, filter: TextureFilter) -> Result<Resource<hud::GuiTexture>, HostError> {
        let next = self.next_texture.checked_add(1).ok_or(HostError::Busy)?;
        if self.texture_count.fetch_update(Ordering::AcqRel, Ordering::Acquire, |count| {
            (count < MAX_TEXTURES).then_some(count + 1)
        }).is_err() {
            return Err(HostError::Busy);
        }
        let texture = match timed_host_call!(self.host.gui_texture_create(width, height, format, filter)) {
            Ok(texture) => texture,
            Err(error) => {
                self.texture_count.fetch_sub(1, Ordering::AcqRel);
                return Err(error);
            }
        };
        let rep = self.next_texture;
        self.next_texture = next;
        self.textures.insert(rep, texture);
        Ok(Resource::new_own(rep))
    }

    fn draw(&mut self, texture: Resource<hud::GuiTexture>, bounds: Rect, uv: Rect, argb: u32, nine_slice: Option<GuiNineSlice>) -> Result<bool, HostError> {
        let texture = *self.textures.get(&texture.rep()).ok_or(HostError::NotFound)?;
        timed_host_call!(self.host.gui_texture_draw(texture, bounds, uv, argb, nine_slice))
    }

    fn drop(&mut self, texture: Resource<hud::GuiTexture>) -> wasmtime::Result<()> {
        if let Some(texture) = self.textures.remove(&texture.rep()) {
            self.texture_count.fetch_sub(1, Ordering::AcqRel);
            timed_host_call!(self.host.gui_texture_drop(texture));
        }
        Ok(())
    }
}

impl network::Host for ClientContext {
    fn net_send_to_server(&mut self, channel: String, data: Vec<u8>) -> Result<(), SendError> {
        timed_host_call!(self.host.net_send_to_server(&channel, &data))
    }
}

impl settings::Host for ClientContext {
    fn setting(&mut self, key: String) -> Result<Option<String>, HostError> {
        timed_host_call!(self.host.setting(&self.id, &key))
    }

    fn set_setting(&mut self, key: String, value: String) -> Result<(), HostError> {
        timed_host_call!(self.host.set_setting(&self.id, &key, &value))
    }
}

impl client_world::Host for ClientContext {
    fn client_world_key(&mut self) -> Result<Option<WorldKey>, HostError> {
        timed_host_call!(self.host.client_world_key())
    }

    fn snapshot_client_world(&mut self) -> Result<WorldSnapshot, HostError> {
        timed_host_call!(self.host.snapshot_client_world())
    }

    fn playing(&mut self) -> Result<bool, HostError> {
        timed_host_call!(self.host.playing())
    }
}

impl player::Host for ClientContext {
    fn local_player_snapshot(&mut self) -> Result<Option<PlayerSnapshot>, HostError> {
        timed_host_call!(self.host.local_player_snapshot())
    }
}

impl instance::Host for ClientContext {
    fn instance_dir(&mut self) -> Result<String, HostError> {
        timed_host_call!(self.host.instance_dir())
    }
}

impl accounts::Host for ClientContext {
    fn account(&mut self) -> Result<Option<AccountSnapshot>, HostError> {
        timed_host_call!(self.host.account())
    }
}

impl screen::Host for ClientContext {
    fn mod_screen_open(&mut self, mod_id: String) -> Result<bool, HostError> {
        if mod_id != self.id {
            return Err(HostError::Invalid);
        }
        timed_host_call!(self.host.mod_screen_open(&self.id))
    }

    fn mod_screen_close(&mut self, mod_id: String) -> Result<(), HostError> {
        if mod_id != self.id {
            return Err(HostError::Invalid);
        }
        timed_host_call!(self.host.mod_screen_close(&self.id))
    }

    fn mod_screen(&mut self) -> Result<Option<String>, HostError> {
        timed_host_call!(self.host.mod_screen())
    }

    fn mod_screen_open_anywhere(&mut self, mod_id: String) -> Result<bool, HostError> {
        if mod_id != self.id {
            return Err(HostError::Invalid);
        }
        timed_host_call!(self.host.mod_screen_open_anywhere(&self.id))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::{Arc, Mutex};

    struct FakeHost {
        calls: Arc<Mutex<Vec<String>>>,
        next_texture: u64,
    }

    impl ClientHost for FakeHost {
        fn gui_texture_create(&mut self, _: u32, _: u32, _: TextureFormat, _: TextureFilter) -> Result<u64, HostError> {
            let texture = self.next_texture;
            self.next_texture += 1;
            self.calls.lock().unwrap().push(format!("create:{texture}"));
            Ok(texture)
        }

        fn gui_texture_drop(&mut self, texture: u64) {
            self.calls.lock().unwrap().push(format!("drop:{texture}"));
        }

        fn mod_screen_close(&mut self, id: &str) -> Result<(), HostError> {
            self.calls.lock().unwrap().push(format!("close:{id}"));
            Ok(())
        }

        fn mod_screen_open_anywhere(&mut self, id: &str) -> Result<bool, HostError> {
            self.calls.lock().unwrap().push(format!("anywhere:{id}"));
            Ok(true)
        }

        fn setting(&mut self, id: &str, key: &str) -> Result<Option<String>, HostError> {
            self.calls.lock().unwrap().push(format!("get:{id}:{key}"));
            Ok(None)
        }

        fn set_setting(&mut self, id: &str, key: &str, value: &str) -> Result<(), HostError> {
            self.calls.lock().unwrap().push(format!("set:{id}:{key}:{value}"));
            Ok(())
        }
    }

    fn context(runtime: &mut crate::Sandbox, id: &str, calls: Arc<Mutex<Vec<String>>>) -> ClientContext {
        ClientContext::new(id.into(), Box::new(FakeHost { calls, next_texture: 0 }), runtime.texture_count(id))
    }

    #[test]
    fn texture_limit_and_cleanup_are_per_mod() {
        let calls = Arc::new(Mutex::new(Vec::new()));
        let mut runtime = crate::Sandbox::new();
        let mut first = context(&mut runtime, "mine", calls.clone());
        let mut second = context(&mut runtime, "mine", calls.clone());
        let mut other = context(&mut runtime, "other", calls.clone());
        let mut textures = Vec::new();
        for _ in 0..64 {
            textures.push(<ClientContext as hud::HostGuiTexture>::create(
                &mut first, 1, 1, TextureFormat::Rgba8, TextureFilter::Nearest,
            ).unwrap());
        }
        assert!(matches!(
            <ClientContext as hud::HostGuiTexture>::create(
                &mut second, 1, 1, TextureFormat::Rgba8, TextureFilter::Nearest,
            ),
            Err(HostError::Busy)
        ));
        let other_texture = <ClientContext as hud::HostGuiTexture>::create(
            &mut other, 1, 1, TextureFormat::Rgba8, TextureFilter::Nearest,
        ).unwrap();
        <ClientContext as hud::HostGuiTexture>::drop(&mut other, other_texture).unwrap();
        <ClientContext as hud::HostGuiTexture>::drop(&mut first, textures.remove(0)).unwrap();
        <ClientContext as hud::HostGuiTexture>::create(
            &mut second, 1, 1, TextureFormat::Rgba8, TextureFilter::Nearest,
        ).unwrap();
        drop(first);
        <ClientContext as hud::HostGuiTexture>::create(
            &mut second, 1, 1, TextureFormat::Rgba8, TextureFilter::Nearest,
        ).unwrap();
        drop(second);
        drop(other);
        let calls = calls.lock().unwrap();
        assert_eq!(calls.iter().filter(|call| call.starts_with("create:")).count(), 67);
        assert_eq!(calls.iter().filter(|call| call.starts_with("drop:")).count(), 67);
    }

    #[test]
    fn screen_and_settings_are_bound_to_the_calling_mod() {
        let calls = Arc::new(Mutex::new(Vec::new()));
        let mut context = context(&mut crate::Sandbox::new(), "mine", calls.clone());
        assert!(matches!(screen::Host::mod_screen_close(&mut context, "other".into()), Err(HostError::Invalid)));
        assert!(matches!(screen::Host::mod_screen_open_anywhere(&mut context, "other".into()), Err(HostError::Invalid)));
        screen::Host::mod_screen_close(&mut context, "mine".into()).unwrap();
        assert!(screen::Host::mod_screen_open_anywhere(&mut context, "mine".into()).unwrap());
        settings::Host::setting(&mut context, "key".into()).unwrap();
        settings::Host::set_setting(&mut context, "key".into(), "value".into()).unwrap();
        assert_eq!(*calls.lock().unwrap(), ["close:mine", "anywhere:mine", "get:mine:key", "set:mine:key:value"]);
    }
}
