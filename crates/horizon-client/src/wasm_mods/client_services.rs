use super::*;
use yoake_wasm::{ClientServices, client_types as ct};

pub(super) fn identity(id: &str) -> Option<(&'static str, usize)> {
    static IDS: OnceLock<Mutex<HashMap<String, (&'static str, usize)>>> = OnceLock::new();
    let mut ids = IDS
        .get_or_init(Default::default)
        .lock()
        .unwrap_or_else(|error| error.into_inner());
    if let Some(identity) = ids.get(id) {
        return Some(*identity);
    }
    if ids.len() >= MAX_MODS {
        return None;
    }
    let identity = (
        Box::leak(id.to_owned().into_boxed_str()) as &'static str,
        usize::MAX - ids.len(),
    );
    ids.insert(id.to_owned(), identity);
    Some(identity)
}

impl GameHost {
    pub(super) fn client_call<R>(
        &self,
        operation: impl FnOnce(&mut Invocation<'_>) -> Result<R, String>,
    ) -> Result<R, String> {
        if self.native_id.is_empty() {
            return Err("client service identity limit reached; restart required".to_owned());
        }
        let _caller = crate::crash::enter_native_mod(crate::crash::NativeModCall {
            id: Some(self.native_id),
            index: self.native_index,
            hook: "wasm-client-service",
        });
        let _sound = crate::mod_api::enter_local_sound_caller(self.native_index);
        service(|invocation| Ok(operation(invocation))).map_err(|error| format!("{error:?}"))?
    }
}

fn read_host<'borrow>(invocation: &'borrow Invocation<'_>) -> Result<&'borrow dyn Host, String> {
    invocation
        .host
        .read()
        .ok_or_else(|| "client host unavailable in this hook".to_owned())
}

fn write_host<'borrow>(
    invocation: &'borrow mut Invocation<'_>,
) -> Result<&'borrow mut dyn Host, String> {
    invocation
        .host
        .write()
        .ok_or_else(|| "mutable client host unavailable in this hook".to_owned())
}

fn point(value: ct::Vec3) -> [f64; 3] {
    [value.x, value.y, value.z]
}
fn vector(value: [f64; 3]) -> ct::Vec3 {
    ct::Vec3 {
        x: value[0],
        y: value[1],
        z: value[2],
    }
}
fn pair(value: ct::Vec2) -> [f32; 2] {
    [value.x, value.y]
}
fn rect(value: ct::Box2) -> [f32; 4] {
    [value.x, value.y, value.width, value.height]
}

fn quad(corners: Vec<ct::Vec3>) -> Result<[[f64; 3]; 4], String> {
    corners
        .into_iter()
        .map(point)
        .collect::<Vec<_>>()
        .try_into()
        .map_err(|_| "quad requires four corners".to_owned())
}

fn anchor(value: ct::PcmAnchor) -> Result<NativeStreamAnchor, String> {
    match (value.entity, value.position) {
        (Some(entity), None) => Ok(NativeStreamAnchor::Entity(entity)),
        (None, Some(position)) if point(position).iter().all(|value| value.is_finite()) => {
            Ok(NativeStreamAnchor::Position(point(position)))
        }
        (None, None) => Ok(NativeStreamAnchor::Listener),
        _ => Err("invalid PCM anchor".to_owned()),
    }
}

impl ClientServices for GameHost {
    fn register_key_binding(
        &mut self,
        category: String,
        default_key: String,
        id: String,
        label: String,
    ) -> Result<(), String> {
        self.client_call(|invocation| {
            write_host(invocation)?.register_key_binding(&id, &default_key, &category, &label)
        })
    }

    fn key_binding_state(&mut self, id: String) -> Result<ct::KeyState, String> {
        self.client_call(|invocation| {
            Ok(ct::KeyState {
                down: read_host(invocation)?.key_binding_state(&id),
            })
        })
    }

    fn client_view_hit(&mut self, max_distance: f64) -> Result<Option<ct::ViewHit>, String> {
        self.client_call(|invocation| {
            Ok(write_host(invocation)?
                .client_view_hit(max_distance)
                .map(|hit| {
                    let mut result = ct::ViewHit {
                        position: vector(hit.position),
                        block: None,
                        face: None,
                        entity: None,
                        uuid: None,
                    };
                    match hit.target {
                        NativeViewTarget::Block { position, face } => {
                            result.block = Some((position[0], position[1], position[2]));
                            result.face = Some((face[0], face[1], face[2]));
                        }
                        NativeViewTarget::Entity { entity_id, uuid } => {
                            result.entity = Some(entity_id);
                            result.uuid = Some(full::uuid_text(uuid));
                        }
                    }
                    result
                }))
        })
    }

    fn display_name(&mut self, id: String, registry: String) -> Result<String, String> {
        self.client_call(|invocation| Ok(read_host(invocation)?.display_name(&registry, &id)))
    }

    fn owner_mod(&mut self, namespace: String) -> Result<Option<(String, String)>, String> {
        self.client_call(|invocation| Ok(read_host(invocation)?.owner_mod(&namespace)))
    }

    fn draw_gui_item(
        &mut self,
        at: ct::Vec2,
        scale: f32,
        stack: ct::ItemStack,
    ) -> Result<bool, String> {
        let stack = full::item_from(&full::et::ItemStack {
            id: stack.item,
            count: stack.count,
            components: stack.components,
        })
        .ok_or_else(|| "invalid item stack".to_owned())?;
        self.client_call(|invocation| {
            let batch = invocation
                .batch
                .as_deref_mut()
                .ok_or("HUD batch unavailable")?;
            Ok(invocation
                .host
                .read()
                .ok_or("client host unavailable")?
                .draw_gui_item(batch, &stack, pair(at), scale))
        })
    }

    fn screen_kind(&mut self) -> Result<Option<String>, String> {
        self.client_call(|invocation| Ok(invocation.client.screen_kind.clone()))
    }

    fn screen_slot_rects(&mut self) -> Result<Option<Vec<ct::SlotRect>>, String> {
        self.client_call(|invocation| {
            Ok(read_host(invocation)?.screen_slot_rects().map(|slots| {
                slots
                    .into_iter()
                    .map(|(slot, bounds)| ct::SlotRect {
                        slot: slot as u32,
                        bounds: ct::Box2 {
                            x: bounds[0],
                            y: bounds[1],
                            width: bounds[2],
                            height: bounds[3],
                        },
                    })
                    .collect()
            }))
        })
    }

    fn draw_world_line(
        &mut self,
        color: u32,
        end: ct::Vec3,
        start: ct::Vec3,
        width: f32,
    ) -> Result<(), String> {
        self.client_call(|invocation| {
            if let Some(draws) = invocation.client.world_draws.as_deref_mut() {
                draws.set_owner(self.native_id);
                draws.line(point(start), point(end), color, width)
            } else {
                write_host(invocation)?.draw_world_line(point(start), point(end), color, width)
            }
        })
    }

    fn draw_world_quad(&mut self, color: u32, corners: Vec<ct::Vec3>) -> Result<(), String> {
        let corners = quad(corners)?;
        self.client_call(|invocation| {
            if let Some(draws) = invocation.client.world_draws.as_deref_mut() {
                draws.set_owner(self.native_id);
                draws.quad(corners, color)
            } else {
                write_host(invocation)?.draw_world_quad(corners, color)
            }
        })
    }

    fn draw_world_textured_quad(
        &mut self,
        color: u32,
        corners: Vec<ct::Vec3>,
        texture: u64,
        uv: ct::Box2,
    ) -> Result<(), String> {
        let corners = quad(corners)?;
        let texture = self
            .textures
            .get(&texture)
            .ok_or("invalid or foreign GUI texture")?;
        self.client_call(|invocation| {
            if let Some(draws) = invocation.client.world_draws.as_deref_mut() {
                draws.set_owner(self.native_id);
                draws.textured_quad(corners, rect(uv), color, texture)
            } else {
                write_host(invocation)?.draw_world_textured_quad(corners, rect(uv), color, texture)
            }
        })
    }

    fn draw_world_billboard(
        &mut self,
        center: ct::Vec3,
        color: u32,
        size: ct::Vec2,
        texture: u64,
        uv: ct::Box2,
    ) -> Result<(), String> {
        let texture = self
            .textures
            .get(&texture)
            .ok_or("invalid or foreign GUI texture")?;
        self.client_call(|invocation| {
            if let Some(draws) = invocation.client.world_draws.as_deref_mut() {
                draws.set_owner(self.native_id);
                draws.billboard(point(center), pair(size), rect(uv), color, Some(texture))
            } else {
                write_host(invocation)?.draw_world_billboard(
                    point(center),
                    pair(size),
                    rect(uv),
                    color,
                    Some(texture),
                )
            }
        })
    }

    fn draw_post_tint(&mut self, color: u32) -> Result<(), String> {
        self.client_call(|invocation| write_host(invocation)?.draw_post_tint(color))
    }

    fn draw_post_vignette(&mut self, color: u32) -> Result<(), String> {
        self.client_call(|invocation| write_host(invocation)?.draw_post_vignette(color))
    }

    fn register_post_shader(
        &mut self,
        id: String,
        uniform_size: u32,
        wgsl: String,
    ) -> Result<(), String> {
        self.client_call(|invocation| {
            write_host(invocation)?.register_post_shader(
                &id,
                &wgsl,
                NativePostUniformLayout { size: uniform_size },
            )
        })?;
        self.resources.claim_shader(id);
        Ok(())
    }

    fn unregister_post_shader(&mut self, id: String) -> Result<(), String> {
        self.client_call(|invocation| write_host(invocation)?.unregister_post_shader(&id))?;
        self.resources.remove_shader(&id);
        Ok(())
    }

    fn set_post_shader_uniforms(&mut self, bytes: Vec<u8>, id: String) -> Result<(), String> {
        self.client_call(|invocation| write_host(invocation)?.set_post_shader_uniforms(&id, &bytes))
    }

    fn enable_post_shader(&mut self, enabled: bool, id: String) -> Result<(), String> {
        self.client_call(|invocation| write_host(invocation)?.enable_post_shader(&id, enabled))
    }

    fn post_shader_state(&mut self, id: String) -> Result<bool, String> {
        self.client_call(|invocation| read_host(invocation)?.post_shader_state(&id))
    }

    fn map_columns(
        &mut self,
        below: Option<i32>,
        chunk: (i32, i32),
    ) -> Result<Option<ct::MapTile>, String> {
        self.client_call(|invocation| {
            Ok(write_host(invocation)?
                .map_columns(
                    [chunk.0, chunk.1],
                    below.map_or(NativeMapLayer::Surface, NativeMapLayer::Below),
                )
                .map(|tile| ct::MapTile {
                    color: tile.color.to_vec(),
                    height: tile.height.to_vec(),
                    depth: tile.depth.to_vec(),
                }))
        })
    }

    fn map_colors(&mut self) -> Result<Vec<u32>, String> {
        self.client_call(|invocation| Ok(read_host(invocation)?.map_colors().to_vec()))
    }

    fn client_players(&mut self) -> Result<Vec<ct::MapPlayer>, String> {
        self.client_call(|invocation| {
            Ok(read_host(invocation)?
                .client_players()
                .into_iter()
                .map(|player| ct::MapPlayer {
                    dimension: player.dimension,
                    name: player.name,
                    position: vector(player.pos),
                    uuid: full::uuid_text(player.uuid),
                    yaw: player.yaw,
                })
                .collect())
        })
    }

    fn client_entities_in(
        &mut self,
        center: ct::Vec3,
        radius: f64,
    ) -> Result<Vec<ct::MapEntity>, String> {
        if !radius.is_finite() || radius < 0.0 || radius > f32::MAX as f64 {
            return Err("invalid entity radius".to_owned());
        }
        self.client_call(|invocation| {
            Ok(read_host(invocation)?
                .client_entities_in(point(center), radius as f32)
                .into_iter()
                .map(|entity| ct::MapEntity {
                    id: entity.id,
                    kind: entity.kind,
                    position: vector(entity.pos),
                    yaw: entity.yaw,
                })
                .collect())
        })
    }

    fn camera(&mut self) -> Result<Option<ct::CameraState>, String> {
        self.client_call(|invocation| {
            Ok(read_host(invocation)?
                .camera()
                .map(|camera| ct::CameraState {
                    position: vector(camera.position),
                    eye_height: camera.eye_height,
                    partial_tick: camera.partial_tick,
                    yaw: camera.yaw,
                    pitch: camera.pitch,
                    fov_degrees: camera.fov_degrees,
                    near: camera.near,
                    far: camera.far,
                    view: camera.view.to_vec(),
                    projection: camera.projection.to_vec(),
                    dimension: camera.dimension,
                }))
        })
    }

    fn camera_override_call(&mut self, value: Option<ct::CameraOverride>) -> Result<(), String> {
        let value = value.map(|value| CameraOverride {
            position: point(value.position),
            yaw: value.yaw,
            pitch: value.pitch,
            roll: value.roll,
            fov: value.fov_degrees,
        });
        if value.as_ref().is_some_and(|value| !value.is_valid()) {
            return Err("invalid camera override".to_owned());
        }
        self.client_call(|invocation| {
            *invocation
                .client
                .camera
                .as_deref_mut()
                .ok_or("camera override is only available in its hook")? = value;
            Ok(())
        })
    }

    fn gui_texture_write(
        &mut self,
        bytes: Vec<u8>,
        origin: (u32, u32),
        size: (u32, u32),
        texture: u64,
    ) -> Result<(), String> {
        let texture = self
            .textures
            .get(&texture)
            .ok_or("invalid or foreign GUI texture")?;
        self.client_call(|invocation| {
            read_host(invocation)?
                .gui_texture_write(texture, [origin.0, origin.1], [size.0, size.1], &bytes)
                .map_err(|error| format!("{error:?}"))
        })
    }

    fn audio_stream_open(
        &mut self,
        position: ct::PcmAnchor,
        category: String,
        channels: u16,
        sample_rate: u32,
    ) -> Result<u32, String> {
        let anchor = anchor(position)?;
        let category =
            server_services::sound_category(&category).map_err(|error| format!("{error:?}"))?;
        let next = self
            .next_stream
            .checked_add(1)
            .ok_or("PCM stream handles exhausted")?;
        let stream = self.client_call(|invocation| {
            write_host(invocation)?
                .audio_stream_open(anchor, sample_rate, channels, category)
                .map_err(|error| format!("{error:?}"))
        })?;
        let id = self.next_stream;
        self.next_stream = next;
        self.streams.insert(id, stream);
        Ok(id)
    }

    fn audio_stream_push(&mut self, samples: Vec<f32>, stream: u32) -> Result<u32, String> {
        let stream = *self
            .streams
            .get(&stream)
            .ok_or("invalid or foreign PCM stream")?;
        self.client_call(|invocation| {
            write_host(invocation)?
                .audio_stream_push(stream, &samples)
                .map(|count| count as u32)
                .map_err(|error| format!("{error:?}"))
        })
    }

    fn audio_stream_push_i16(&mut self, samples: Vec<i16>, stream: u32) -> Result<u32, String> {
        let stream = *self
            .streams
            .get(&stream)
            .ok_or("invalid or foreign PCM stream")?;
        self.client_call(|invocation| {
            write_host(invocation)?
                .audio_stream_push_i16(stream, &samples)
                .map(|count| count as u32)
                .map_err(|error| format!("{error:?}"))
        })
    }

    fn audio_stream_queued(&mut self, stream: u32) -> Result<Option<u32>, String> {
        let stream = *self
            .streams
            .get(&stream)
            .ok_or("invalid or foreign PCM stream")?;
        self.client_call(|invocation| {
            Ok(write_host(invocation)?
                .audio_stream_queued(stream)
                .map(|count| count as u32))
        })
    }

    fn audio_stream_dropped(&mut self, stream: u32) -> Result<Option<u64>, String> {
        let stream = *self
            .streams
            .get(&stream)
            .ok_or("invalid or foreign PCM stream")?;
        self.client_call(|invocation| Ok(write_host(invocation)?.audio_stream_dropped(stream)))
    }

    fn audio_stream_move(&mut self, position: ct::PcmAnchor, stream: u32) -> Result<(), String> {
        let stream = *self
            .streams
            .get(&stream)
            .ok_or("invalid or foreign PCM stream")?;
        let anchor = anchor(position)?;
        self.client_call(|invocation| {
            write_host(invocation)?
                .audio_stream_move(stream, anchor)
                .map_err(|error| format!("{error:?}"))
        })
    }

    fn audio_stream_close(&mut self, stream: u32) -> Result<bool, String> {
        let Some(handle) = self.streams.get(&stream).copied() else {
            return Ok(false);
        };
        let closed =
            self.client_call(|invocation| Ok(write_host(invocation)?.audio_stream_close(handle)))?;
        self.streams.remove(&stream);
        Ok(closed)
    }

    fn draw_mob_as_player(&mut self, appearance: Vec<u8>) -> Result<bool, String> {
        let appearance = NativePlayerAppearance::decode(&appearance)
            .ok_or_else(|| "invalid player appearance".to_owned())?;
        self.client_call(|invocation| {
            let mob = invocation.client.mob.ok_or("mob render hook required")?;
            let draw = invocation
                .client
                .mob_draw
                .as_deref_mut()
                .ok_or("mob render hook required")?;
            Ok(invocation
                .host
                .read()
                .ok_or("client host unavailable")?
                .draw_mob_as_player(mob, draw, &appearance))
        })
    }

    fn hud_place(
        &mut self,
        corner: ct::HudCorner,
        size: ct::Vec2,
    ) -> Result<Option<ct::Vec2>, String> {
        let corner = match corner {
            ct::HudCorner::TopLeft => Corner::TopLeft,
            ct::HudCorner::TopRight => Corner::TopRight,
            ct::HudCorner::BottomLeft => Corner::BottomLeft,
            ct::HudCorner::BottomRight => Corner::BottomRight,
        };
        self.client_call(|invocation| {
            let corners = invocation
                .client
                .corners
                .as_deref_mut()
                .ok_or("HUD layout hook required")?;
            Ok(invocation
                .host
                .read()
                .ok_or("client host unavailable")?
                .hud_place(corners, corner, pair(size))
                .map(|value| ct::Vec2 {
                    x: value[0],
                    y: value[1],
                }))
        })
    }
}
