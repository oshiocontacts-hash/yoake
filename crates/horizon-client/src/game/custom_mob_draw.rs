//! Drawing and client events of fully custom mobs (FR-F4-MOBS.md sections 3.7 and 4, F4M-6). The
//! logic (pose evaluation, vertex output, layers, LOD, event routing) is in
//! `mobs/custom/render_state.rs` and is tested without a window; this file is the glue to the game:
//! its camera, atlas, items, audio and particles.
//!
//! One `kind.custom` test in the mob loop of `creatures.rs` (`mob_geometry`) calls
//! [`Game::custom_mob_geometry`]; the per-tick call [`Game::tick_custom_mob_events`] drains the
//! animation events and plays them. A world without a custom type does neither.
use super::Game;
use glam::{DVec3, Mat4, Vec3};
use horizon_client::{
    items::ItemStack,
    mesher::Vertex,
    mobs::{
        Mob,
        custom::{
            CustomType,
            render_state::{self as rs, DrawCtx, EventSink, MobFacts, PoseCtx, TypeRender},
        },
    },
};
use horizon_client::mobs::custom::script;
use std::sync::Arc;

/// How far from the player (blocks) a custom mob's animation events are heard and seen.
const EVENT_RANGE: f64 = 64.0;

impl Game<'_> {
    /// The animated locators of `vehicle` as the renderer last posed them (F4M-4d): a name gives the
    /// locator in the mob's frame (blocks, unturned, unscaled), `None` before the first draw or for a name
    /// the model lacks. A rider on a seat that follows a locator sits where the model's pose puts it.
    pub(super) fn posed_locators<'a>(&'a self, vehicle: &'a Mob) -> impl Fn(&str) -> Option<[f64; 3]> + 'a {
        move |name| {
            let tr = vehicle.kind.custom.as_ref()?.render.as_ref()?;
            let render = self.custom_mob_render.try_borrow().ok()?;
            render.animated_locator(vehicle.id, tr, name)
        }
    }

    /// Draws custom mob `mob` of type `ct` (whose model is `tr`) into `out`: pose, body, layers and
    /// items. `distance` is the camera's distance to it (the LOD), `light` the packed light of the
    /// entity pass. The cull test, the shadow, the name tag and the outline are the caller's.
    #[allow(clippy::too_many_arguments)]
    pub(super) fn custom_mob_geometry(
        &self,
        mob: &Mob,
        ct: &Arc<CustomType>,
        tr: &TypeRender,
        camera: DVec3,
        alpha: f64,
        distance: f64,
        light: u32,
        lookup: &dyn Fn(&str) -> Option<[f32; 4]>,
        out: &mut Vec<Vertex>,
    ) {
        let partial = alpha as f32;
        let feet = mob.previous.lerp(mob.position, alpha);
        let feet = DVec3::new(feet.x, feet.y, feet.z);
        let yaw = mob.previous_body_yaw
            + horizon_client::mobs::living::wrap(mob.body_yaw - mob.previous_body_yaw) * partial;
        let roll = rs::death_roll_for(ct, mob.death_time, partial);
        let scale = tr.scale * mob.model_scale();
        let lod = tr.lod_mode(distance as f32);
        let rel = (feet - camera).as_vec3();
        let mut render = self.custom_mob_render.borrow_mut();
        let frame = render.frame();
        let (state, mirror) = render.parts(mob.id, tr);
        let anim = match &self.remote {
            Some(remote) => remote.custom_mobs.get(mob.id as i32),
            None => mirror.get(mob.id),
        };
        let mut ctx = PoseCtx::new(self.world.game_time, partial, MobFacts::of(mob));
        ctx.frame = frame;
        ctx.lod = lod;
        ctx.day_fraction = self.world.day_time.rem_euclid(24000) as f32 / 24000.0;
        if tr.procedural.is_some() && lod != rs::LodMode::Static {
            // Targets are per client and cosmetic (the viewer; the mob's own target).
            ctx.view = Some(rs::model_point_of(-rel, scale, yaw, roll));
            ctx.target = self.beam_target(mob.ai.target, alpha).map(|t| rs::model_point_of((t - feet).as_vec3(), scale, yaw, roll));
            ctx.model_to_world = rs::model_to_world_px([feet.x, feet.y, feet.z], scale, yaw, roll);
        }
        // `look_at: owner` (F4M-10): the owner when it is this client's own player (other owners
        // are not tracked on the client yet).
        if tr.procedural.is_some()
            && lod != rs::LodMode::Static
            && let (Some(owner), Some(me), Some(p)) = (mob.extra.owner, self.level.player_uuid, self.player.as_ref())
            && owner == me
        {
            let eye = DVec3::new(p.position.x, p.position.y + 1.62, p.position.z);
            ctx.owner = Some(rs::model_point_of((eye - feet).as_vec3(), scale, yaw, roll));
        }
        // `mob-pose` (F4M-10): the script's bone writes, only for a `procedural_script` type and
        // only when this frame evaluates the pose (so inside the LOD range that does).
        if tr.script_pose
            && lod != rs::LodMode::Static
            && lod != rs::LodMode::Hidden
            && state.due(lod, frame, mob.id)
            && let Some(rt) = script::client_runtime()
        {
            let speed = (mob.velocity.x * mob.velocity.x + mob.velocity.z * mob.velocity.z).sqrt() as f32;
            let pf = script::MobPoseFrame {
                time: (self.world.game_time as f32 + partial) / 20.0,
                partial,
                ground_speed: speed,
                head_yaw: horizon_client::mobs::living::wrap(mob.head_yaw - mob.body_yaw),
                head_pitch: mob.head_pitch,
                on_ground: mob.on_ground,
                target: ctx.target.map(|t| script::ScriptVec3 { x: t.x, y: t.y, z: t.z }),
            };
            state.writes = script::bone_writes(&rt.mob_pose(mob.id, &ct.id, &pf), tr.skeleton.len());
        }
        state.update(ct, tr, anim, &ctx);
        if tr.script_pose {
            let at = [feet.x, feet.y, feet.z];
            let places = (0..tr.locators.len())
                .filter_map(|i| {
                    let p = tr.locator_point(i, &state.matrices)?;
                    Some(script::LocatorPlace {
                        name: tr.locators[i].name.clone(),
                        position: rs::model_point_to_world(at, scale, yaw, roll, p),
                        yaw,
                        pitch: 0.0,
                    })
                })
                .collect();
            script::set_locators(mob.id, places);
        }
        let world_from_model = rs::world_from_model(rel, scale, yaw, roll);
        let red = mob.hurt_time > 0 || mob.death_time > 0;
        let texture_overrides = self.custom_mob_texture_overrides(mob.id);
        let draw = DrawCtx {
            world_from_model,
            light,
            overlay: horizon_client::overlay::living(0.0, red),
            lod,
            atlas: lookup,
            texture_overrides: texture_overrides.as_ref(),
        };
        tr.draw(state, &draw, out);
        if lod != rs::LodMode::Hidden && lod != rs::LodMode::Static {
            self.custom_mob_items(mob, tr, &state.matrices, rel, scale, yaw, roll, light, out);
        }
    }

    /// The items of `item_slots`: the main hand, the off hand and the head item at their locators,
    /// through the engine's own item draw path (the ground display, as a fox's mouth item).
    #[allow(clippy::too_many_arguments)]
    fn custom_mob_items(
        &self,
        mob: &Mob,
        tr: &TypeRender,
        matrices: &[Mat4],
        rel: Vec3,
        scale: f32,
        yaw: f32,
        roll: f32,
        light: u32,
        out: &mut Vec<Vertex>,
    ) {
        use horizon_assets::item_model::DisplayContext as Context;
        let slots = tr.item_slots;
        if slots.mainhand.is_none() && slots.offhand.is_none() && slots.head.is_none() {
            return;
        }
        let head = mob.extra.head.as_deref().map(|h| ItemStack::new(h, 1));
        let jobs = [
            (slots.mainhand, horizon_client::mobs::render::held_item_stack(&mob.extra, false), Context::Ground, 7, horizon_client::cit::Hand::Main),
            (slots.offhand, horizon_client::mobs::render::held_item_stack(&mob.extra, true), Context::Ground, 7, horizon_client::cit::Hand::Off),
            (slots.head, head, Context::Head, 5, horizon_client::cit::Hand::Main),
        ];
        let basis = Mat4::from_translation(rel) * rs::mob_basis(scale, yaw, roll);
        for (locator, stack, context, ordinal, hand) in jobs {
            let (Some(i), Some(stack)) = (locator, stack) else {
                continue;
            };
            let Some(frame) = tr.locator_frame(i, matrices) else {
                continue;
            };
            // Blocks: the locator's position and the bone's orientation, no scale.
            let at = Mat4::from_translation(frame.w_axis.truncate() / 16.0)
                * Mat4::from_cols(frame.x_axis, frame.y_axis, frame.z_axis, glam::Vec4::W);
            let owner = super::item_draw::owner_at(
                [mob.position.x, mob.position.y, mob.position.z],
                mob.body_yaw,
                &mob.kind.id,
            );
            let state = self.resolve_item_in_hand(&stack, context, Some(&owner), mob.id as i32 + ordinal, hand);
            self.submit_item_world(&state, basis * at, light, out);
        }
    }

    /// Once per game tick: brings the local custom mobs' mirrors up to date and plays what the
    /// animations of every custom mob in range gave since the last tick (sound and particle events
    /// at their locator, custom events to the script hook). Nothing to do, and nothing done, in a
    /// world without a custom type.
    pub(super) fn tick_custom_mob_events(&mut self) {
        use horizon_client::mobs::custom::registry;
        if registry::registry().read().is_ok_and(|r| r.is_empty()) {
            return;
        }
        let Some(player) = self.player.as_ref().map(|p| DVec3::new(p.position.x, p.position.y, p.position.z)) else {
            return;
        };
        let tick = self.world.game_time;
        let local = self.remote.is_none();
        let viewer = self.level.player_uuid;
        let render = self.custom_mob_render.get_mut();
        let frame = render.frame();
        let mut buf = render.take_events();
        for mob in &self.mobs {
            let (Some(ct), true) = (mob.kind.custom.as_ref(), (DVec3::new(mob.position.x, mob.position.y, mob.position.z) - player).length() < EVENT_RANGE) else {
                continue;
            };
            let (Some(tr), Some(ta)) = (ct.render.as_ref(), ct.anim.as_ref()) else {
                continue;
            };
            let mob_state = if local {
                render.mirror.sync(mob, tick, frame, viewer);
                render.mirror.get_mut(mob.id)
            } else {
                self.remote.as_mut().and_then(|r| r.custom_mobs.get_mut(mob.id as i32))
            };
            let Some(mob_state) = mob_state else {
                continue;
            };
            buf.clear();
            mob_state.poll_events(tick, 0.0, &mut buf);
            if buf.is_empty() {
                continue;
            }
            // One info line a tick that has events, for the end-to-end run (tools/custom_mob_render_e2e.py).
            log::info!("Horizon custom-mob events: mob {} tick {tick} count {}", mob.id, buf.len());
            let matrices = render.matrices_of(mob.id).unwrap_or(tr.rest_matrices.as_slice());
            let yaw = mob.body_yaw;
            let roll = rs::death_roll_for(ct, mob.death_time, 0.0);
            let mut sink = GameSink { game_audio: &mut self.audio, particles: &mut self.particles, mob: mob.id };
            rs::route_events(
                ta,
                tr,
                buf.as_slice(),
                matrices,
                [mob.position.x, mob.position.y, mob.position.z],
                tr.scale * mob.model_scale(),
                yaw,
                roll,
                &mut sink,
            );
        }
        render.put_events(buf);
        // `synced-changed-client` (F4M-10); the queue is emptied even with no script.
        let changes = match self.remote.as_mut() {
            Some(r) => r.custom_mobs.take_changes(),
            None => render.mirror.take_changes(),
        };
        if let Some(rt) = script::client_runtime() {
            for c in changes {
                if let Some(ct) = self.mobs.iter().find(|m| m.id as i32 == c.entity).and_then(|m| m.kind.custom.clone())
                    && let Some(def) = ct.synced.get(c.slot)
                {
                    let (o, n) = (script::script_value(def, &c.old), script::script_value(def, &c.new));
                    rt.synced_changed(c.entity as u32, &c.name, &o, &n);
                }
            }
        }
        render.mirror.retain(frame.saturating_sub(600));
    }
}

/// Plays routed events through the game's audio and particle engines.
struct GameSink<'a> {
    game_audio: &'a mut super::Audio,
    particles: &'a mut horizon_client::particles::ParticleEngine,
    mob: u32,
}

impl EventSink for GameSink<'_> {
    fn sound(&mut self, id: &str, at: [f64; 3], volume: f32, pitch: f32) {
        log::info!("Horizon custom-mob sound: mob {} {id} at {at:?}", self.mob);
        // A bare name is a vanilla sound event; a namespaced one (a mod's) plays as it is.
        if id.contains(':') {
            self.game_audio.play(id, Some(at), volume, pitch);
        } else {
            self.game_audio.play(&format!("minecraft:{id}"), Some(at), volume, pitch);
        }
    }
    fn particle(&mut self, id: &str, at: [f64; 3]) {
        log::info!("Horizon custom-mob particle: mob {} {id} at {at:?}", self.mob);
        use horizon_client::particles::{ParticleOptions, ParticleType};
        let full;
        let id = if id.contains(':') {
            id
        } else {
            full = format!("minecraft:{id}");
            &full
        };
        match ParticleType::from_name(id) {
            Some(ty) => self.particles.send_particles(&ParticleOptions::simple(ty), at, [0.0; 3], 0.0, 1),
            None => log::debug!("Horizon: custom mob particle {id} is not a particle of the engine"),
        }
    }
    fn custom(&mut self, name: &str, at: [f64; 3]) {
        // The client hook `mob-anim-event-client` (F4M-10) takes it; nothing is played for it here.
        log::info!("Horizon custom-mob event: mob {} {name:?} at {at:?}", self.mob);
        if let Some(rt) = script::client_runtime() {
            rt.mob_anim_event(self.mob, name);
        }
    }
}
