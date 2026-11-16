use std::time::Instant;

use wasmtime::component::Resource;

use crate::bindings::yoake::mods::hud::GuiTexture;
use crate::full_bindings::client::yoake::mods::client_services;
use crate::host::ClientContext;

pub use crate::full_bindings::client::yoake::mods::client_types;
use client_types::*;

macro_rules! client_services {
    ($($name:ident($($argument:ident: $kind:ty),*) -> $result:ty;)*) => {
        pub trait ClientServices: Send {
            $(fn $name(&mut self, $($argument: $kind),*) -> Result<$result, String> {
                $(let _ = $argument;)*
                Err("unsupported client service".to_owned())
            })*

            fn draw_world_textured_quad(&mut self, _color: u32, _corners: Vec<Vec3>, _texture: u64, _uv: Box2) -> Result<(), String> {
                Err("unsupported client service".to_owned())
            }

            fn draw_world_billboard(&mut self, _center: Vec3, _color: u32, _size: Vec2, _texture: u64, _uv: Box2) -> Result<(), String> {
                Err("unsupported client service".to_owned())
            }

            fn gui_texture_write(&mut self, _bytes: Vec<u8>, _origin: (u32, u32), _size: (u32, u32), _texture: u64) -> Result<(), String> {
                Err("unsupported client service".to_owned())
            }
        }

        impl client_services::Host for ClientContext {
            $(fn $name(&mut self, $($argument: $kind),*) -> Result<$result, String> {
                self.with_client_services(|host| host.$name($($argument),*))
            })*

            fn draw_world_textured_quad(&mut self, color: u32, corners: Vec<Vec3>, texture: Resource<GuiTexture>, uv: Box2) -> Result<(), String> {
                let texture = self.client_texture(&texture)?;
                self.with_client_services(|host| host.draw_world_textured_quad(color, corners, texture, uv))
            }

            fn draw_world_billboard(&mut self, center: Vec3, color: u32, size: Vec2, texture: Resource<GuiTexture>, uv: Box2) -> Result<(), String> {
                let texture = self.client_texture(&texture)?;
                self.with_client_services(|host| host.draw_world_billboard(center, color, size, texture, uv))
            }

            fn gui_texture_write(&mut self, bytes: Vec<u8>, origin: (u32, u32), size: (u32, u32), texture: Resource<GuiTexture>) -> Result<(), String> {
                let texture = self.client_texture(&texture)?;
                self.with_client_services(|host| host.gui_texture_write(bytes, origin, size, texture))
            }
        }
    };
}

impl ClientContext {
    fn client_texture(&self, texture: &Resource<GuiTexture>) -> Result<u64, String> {
        self.textures
            .get(&texture.rep())
            .copied()
            .ok_or_else(|| "invalid or foreign GUI texture".to_owned())
    }

    fn with_client_services<T>(
        &mut self,
        operation: impl FnOnce(&mut dyn ClientServices) -> Result<T, String>,
    ) -> Result<T, String> {
        let began = Instant::now();
        let result = match self.host.client_services() {
            Some(host) => operation(host),
            None => Err("unsupported client services".to_owned()),
        };
        crate::add_host_time(began.elapsed());
        result
    }
}

impl client_types::Host for ClientContext {}

client_services! {
    register_key_binding(category: String, default_key: String, id: String, label: String) -> ();
    key_binding_state(id: String) -> KeyState;
    client_view_hit(max_distance: f64) -> Option<ViewHit>;
    display_name(id: String, registry: String) -> String;
    owner_mod(namespace: String) -> Option<(String, String)>;
    draw_gui_item(at: Vec2, scale: f32, stack: ItemStack) -> bool;
    screen_kind() -> Option<String>;
    screen_slot_rects() -> Option<Vec<SlotRect>>;
    draw_world_line(color: u32, end: Vec3, start: Vec3, width: f32) -> ();
    draw_world_quad(color: u32, corners: Vec<Vec3>) -> ();
    draw_post_tint(color: u32) -> ();
    draw_post_vignette(color: u32) -> ();
    register_post_shader(id: String, uniform_size: u32, wgsl: String) -> ();
    unregister_post_shader(id: String) -> ();
    set_post_shader_uniforms(bytes: Vec<u8>, id: String) -> ();
    enable_post_shader(enabled: bool, id: String) -> ();
    post_shader_state(id: String) -> bool;
    map_columns(below: Option<i32>, chunk: (i32, i32)) -> Option<MapTile>;
    map_colors() -> Vec<u32>;
    draw_mob_as_player(appearance: Vec<u8>) -> bool;
    client_players() -> Vec<MapPlayer>;
    client_entities_in(center: Vec3, radius: f64) -> Vec<MapEntity>;
    camera() -> Option<CameraState>;
    camera_override_call(value: Option<CameraOverride>) -> ();
    audio_stream_open(anchor: PcmAnchor, category: String, channels: u16, sample_rate: u32) -> u32;
    audio_stream_push(samples: Vec<f32>, stream: u32) -> u32;
    audio_stream_push_i16(samples: Vec<i16>, stream: u32) -> u32;
    audio_stream_queued(stream: u32) -> Option<u32>;
    audio_stream_dropped(stream: u32) -> Option<u64>;
    audio_stream_move(anchor: PcmAnchor, stream: u32) -> ();
    audio_stream_close(stream: u32) -> bool;
    hud_place(corner: HudCorner, size: Vec2) -> Option<Vec2>;
}
