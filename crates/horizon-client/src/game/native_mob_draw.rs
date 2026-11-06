//! Mobs drawn by native mods on the client: `native_mob_submit` (vanilla's
//! `EntityRenderer.submit` for a mob, NeoForge's `RenderLivingEvent.Pre` when
//! it cancels) gets the mob's render state and a [`BlockEntityDraw`] posed at
//! its feet and turned with its body, so a mod draws a model of its own with
//! the helpers block entities draw with (`native_draw_cube`,
//! `native_draw_quad`, `native_draw_block`, `native_draw_item`). A player
//! has `native_player_submit` (`RenderPlayerEvent.Pre`), and every mob and
//! player in view gets `native_mob_layer` after its vanilla model
//! (`EntityRenderersEvent.AddLayers`), in the same frame.

use super::{Dim, Game, Hand, native_block_entity_draw::BlockEntityDraw, shield::OFFHAND_SLOT};
use glam::{DVec3, Mat4};
use horizon_client::{
    block_entity_render::diffuse_lights,
    items::ItemStack,
    mesher::Vertex,
    mobs::{Mob, anim, living::wrap},
    pose::Pose,
};

/// One mob's frame, as `EntityRenderer.extractRenderState` fills a
/// `LivingEntityRenderState`: what a model reads to pose and animate itself.
/// Angles are in degrees, interpolated to the frame's partial tick.
// Native mods read it; nothing in Horizon does.
#[derive(Clone, Debug)]
pub struct NativeMobRender<'a> {
    /// The mob's network id, as the mob helpers and mod messages name it.
    pub id: u32,
    /// Its entity type (`minecraft:villager`).
    pub kind: &'a str,
    /// Its `CustomName` as plain text, when it has one.
    pub custom_name: Option<&'a str>,
    /// `isBaby`.
    pub baby: bool,
    /// The frame's partial tick, 0 to 1.
    pub partial: f32,
    /// `ageInTicks`: ticks lived plus the partial tick, for idle motion.
    pub age: f32,
    /// `bodyRot`: the way the body faces (0 is south, 90 west).
    pub body_yaw: f32,
    /// `yRot`: the head's turn from the body, -180 to 180.
    pub head_yaw: f32,
    /// `xRot`: the head's pitch, positive looking down.
    pub pitch: f32,
    /// `walkAnimationPos`: how far the legs have swung, for a walk cycle
    /// (a vanilla quadruped's legs swing `cos(pos * 0.6662) * 1.4 * speed` radians).
    pub walk_position: f32,
    /// `walkAnimationSpeed`: how fast it walks, 0 standing to about 1.
    pub walk_speed: f32,
    /// Ticks left of the red hurt flash (10 when hit).
    pub hurt_time: i32,
    /// Ticks since it died, 0 while alive (vanilla tips the body over 20).
    pub death_time: i32,
    pub on_ground: bool,
}

impl<'a> NativeMobRender<'a> {
    /// The render state of `m` at `pose.partial` (`LivingEntityRenderer.extractRenderState`).
    pub(super) fn of(m: &'a Mob, pose: anim::State) -> Self {
        let partial = pose.partial;
        let body = m.previous_body_yaw + wrap(m.body_yaw - m.previous_body_yaw) * partial;
        // The walk cycle, head turn and age as the vanilla models read them:
        // no walking while riding or dead, a baby's legs three times as far
        // (`WalkAnimationState.position(partial)`, `positionScale`).
        Self {
            id: m.id,
            kind: &m.kind.id,
            custom_name: m.custom_name.as_deref(),
            baby: m.is_baby(),
            partial,
            age: pose.age,
            body_yaw: body,
            head_yaw: pose.y_rot,
            pitch: pose.x_rot,
            walk_position: pose.pos,
            walk_speed: pose.speed,
            hurt_time: m.hurt_time,
            death_time: m.death_time,
            on_ground: m.on_ground,
        }
    }
}

/// Which player a [`NativeMobRender`] is, for `native_player_submit` and
/// `native_mob_layer`.
/// Expanded player state is available from API @API:player_render_state@.
/// Stacks are borrowed for the callback; clone them to retain their components.
// Native mods read it; nothing in Horizon does.
#[derive(Clone, Copy, Debug)]
#[non_exhaustive]
pub struct NativePlayerRender<'a> {
    /// The player's profile name.
    pub name: &'a str,
    /// Whether it is the player of this window, drawn in third person.
    pub local: bool,
    /// Its profile UUID, or `None` when the client has not received it.
    pub uuid: Option<u128>,
    /// The client's pose name: `STANDING`, `CROUCHING`, `SWIMMING`,
    /// `FALL_FLYING`, `SLEEPING`, `SPIN_ATTACK`, or `DYING` when dead.
    pub pose: &'static str,
    /// Crouching is binary in the client: 1 for the crouching pose, else 0.
    pub crouch_amount: f32,
    /// The vanilla model's swim blend, interpolated at this frame's partial tick.
    pub swim_amount: f32,
    /// Whether the player is a passenger, as the vanilla model reads it.
    pub riding: bool,
    /// Whether the client's pose is sleeping.
    pub sleeping: bool,
    pub sprinting: bool,
    /// The frame's interpolated attack swing, clamped to 0 through 1.
    pub attack_progress: f32,
    /// The logical hand swinging; the main hand when no swing arm is set.
    pub swing_hand: Hand,
    /// The logical hand using an item, or `None` while not using one.
    pub using_item: Option<Hand>,
    /// Elapsed use ticks plus the frame's partial tick; 0 while not using.
    pub use_ticks: f32,
    /// Whether the main hand is the physical right hand.
    pub main_hand_right: bool,
    /// The complete main-hand stack, if available; accepted by `native_draw_item`.
    pub main_hand: Option<&'a ItemStack>,
    /// The complete off-hand stack, if available. Remote players currently
    /// retain only its item id, so their off-hand stack is unavailable here.
    pub off_hand: Option<&'a ItemStack>,
    /// Complete armour stacks in head, chest, legs, feet order.
    pub armor: [Option<&'a ItemStack>; 4],
    /// Whether the chest stack is `minecraft:elytra`.
    pub elytra: bool,
    pub on_fire: bool,
    pub invisible: bool,
    pub glowing: bool,
    /// The client's synchronized frozen tick count.
    pub frozen_ticks: i32,
}

impl<'a> NativePlayerRender<'a> {
    fn of(
        mob: &'a Mob,
        state: anim::State,
        who: (&'a str, bool),
        uuid: Option<u128>,
        held: [Option<&'a ItemStack>; 2],
    ) -> Self {
        let main_hand_right = !mob.main_arm_left();
        let using_item = (mob.extra.using_item > 0).then_some(if mob.extra.using_off_hand {
            Hand::Off
        } else {
            Hand::Main
        });
        Self {
            name: who.0,
            local: who.1,
            uuid,
            pose: if !mob.alive() || mob.death_time > 0 {
                "DYING"
            } else {
                mob.extra.avatar_pose.name()
            },
            crouch_amount: if mob.extra.avatar_pose == Pose::Crouching {
                1.0
            } else {
                0.0
            },
            swim_amount: mob.extra.swim.amount(state.partial),
            riding: mob.extra.passenger,
            sleeping: mob.extra.avatar_pose == Pose::Sleeping,
            sprinting: mob.extra.sprinting,
            attack_progress: state.swing.clamp(0.0, 1.0),
            swing_hand: if mob
                .extra
                .swing_right_arm
                .is_some_and(|right| right != main_hand_right)
            {
                Hand::Off
            } else {
                Hand::Main
            },
            using_item,
            use_ticks: if using_item.is_some() {
                mob.extra.using_item as f32 + state.partial
            } else {
                0.0
            },
            main_hand_right,
            main_hand: held[0],
            off_hand: held[1],
            armor: mob.extra.armor.each_ref().map(Option::as_ref),
            elytra: mob.extra.armor[1]
                .as_ref()
                .is_some_and(|stack| stack.item.as_ref() == "minecraft:elytra"),
            on_fire: mob.is_on_fire(),
            invisible: mob.invisible,
            glowing: mob.glowing,
            frozen_ticks: mob.ticks_frozen,
        }
    }
}

fn if_player_render_listened(listened: bool, draw: impl FnOnce() -> bool) -> bool {
    listened && draw()
}

/// The pose a mod's mob model starts from: at the feet, `at` (relative to the
/// camera), turned with the body so +z is the way it faces, +x its left and +y
/// up, in blocks.
pub(super) fn mob_pose(at: DVec3, body_yaw: f32) -> Mat4 {
    Mat4::from_translation(at.as_vec3()) * Mat4::from_rotation_y((-body_yaw).to_radians())
}

impl Game<'_> {
    fn player_render<'a>(
        &'a self,
        mob: &'a Mob,
        who: (&'a str, bool),
        state: anim::State,
    ) -> NativePlayerRender<'a> {
        let uuid = if who.1 {
            self.remote.as_ref().map(|remote| remote.profile()).or_else(|| {
                self.level.player_uuid.map(|words| {
                    words.iter().fold(0_u128, |uuid, &word| {
                        (uuid << 32) | u128::from(word as u32)
                    })
                })
            })
        } else {
            self.remote_player_uuid(mob.id)
        };
        let held = if who.1 {
            [self.stack_in(self.selected), self.stack_in(OFFHAND_SLOT)]
        } else {
            [mob.extra.main_hand_stack(), None]
        };
        let mut render = NativePlayerRender::of(mob, state, who, uuid, held);
        if who.1 {
            if let Some(player) = &self.player {
                render.sprinting = player.sprinting;
                render.on_fire = player.fire_ticks > 0;
                render.frozen_ticks = player.ticks_frozen;
            }
            render.invisible = self.own_invisible();
            render.glowing = self.potions.effects.get("glowing").is_some();
        }
        render
    }

    /// `native_mob_submit` for `mob`, lit by `light` (packed as the entity
    /// pass packs it): true when a mod drew it, and its vanilla model, armour
    /// and held items are left out. The name tag, shadow, fire and a glowing
    /// outline still draw as for any mob.
    pub(super) fn mod_mob_geometry(
        &self,
        mob: &Mob,
        partial: f32,
        camera: DVec3,
        light: u32,
        out: &mut Vec<Vertex>,
    ) -> bool {
        self.mod_entity_draw(mob, partial, camera, light, out, |render, draw, _state| {
            let mut drawn = false;
            self.native_mob_submit(render, draw, &mut drawn);
            drawn
        })
    }
    /// `native_player_submit` for the player `mob` (`who` says which): true
    /// when a mod drew it and the vanilla model, armour, held items, cape and
    /// other layers are left out.
    pub(super) fn mod_player_geometry(
        &self,
        mob: &Mob,
        who: (&str, bool),
        partial: f32,
        camera: DVec3,
        light: u32,
        out: &mut Vec<Vertex>,
    ) -> bool {
        // Posing a player for the mods costs, each frame: only when one listens.
        let mut listened = false;
        self.native_player_render_listened(&mut listened);
        if_player_render_listened(listened, || {
            self.mod_entity_draw(mob, partial, camera, light, out, |render, draw, state| {
                let who = self.player_render(mob, who, state);
                let mut drawn = false;
                self.native_player_submit(render, &who, draw, &mut drawn);
                drawn
            })
        })
    }
    /// `native_mob_layer` for `mob` (`who` set for a player): what a mod adds
    /// is kept, drawn after the vanilla model and its layers.
    pub(super) fn mod_mob_layer_geometry(
        &self,
        mob: &Mob,
        who: Option<(&str, bool)>,
        partial: f32,
        camera: DVec3,
        light: u32,
        out: &mut Vec<Vertex>,
    ) {
        // Every mob in view each frame: only when a mod listens.
        let mut listened = false;
        self.native_player_render_listened(&mut listened);
        if_player_render_listened(listened, || {
            self.mod_entity_draw(mob, partial, camera, light, out, |render, draw, state| {
                let who = who.map(|who| self.player_render(mob, who, state));
                self.native_mob_layer(render, who.as_ref(), draw);
                true
            })
        });
    }
    /// Gives `hook` the frame of `mob` and a draw posed at its feet; what it
    /// added to `out` stays when it answers true.
    fn mod_entity_draw(
        &self,
        mob: &Mob,
        partial: f32,
        camera: DVec3,
        light: u32,
        out: &mut Vec<Vertex>,
        hook: impl FnOnce(&NativeMobRender<'_>, &mut BlockEntityDraw, anim::State) -> bool,
    ) -> bool {
        let state = anim::state(mob, partial);
        let render = NativeMobRender::of(mob, state);
        let a = f64::from(partial);
        let feet = mob.previous.lerp(mob.position, a);
        let feet = DVec3::new(feet.x, feet.y, feet.z);
        let start = out.len();
        let mut draw = BlockEntityDraw::posed(
            partial,
            self.world.game_time,
            light,
            [feet.x, feet.y, feet.z].map(|c| c.floor() as i32),
            mob_pose(feet - camera, render.body_yaw),
            diffuse_lights(self.dim == Dim::Nether),
            std::mem::take(out),
        );
        let keep = hook(&render, &mut draw, state);
        *out = draw.into_vertices();
        if !keep {
            out.truncate(start);
        }
        keep
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use glam::Vec3;
    use horizon_client::mobs::{Brain, Flags, MobType, Part, Species};
    use horizon_core::geometry::Vec3 as Position;
    use std::{cell::Cell, sync::Arc};

    fn player() -> Mob {
        Mob::new(
            7,
            Arc::new(MobType {
                id: "minecraft:player".into(),
                species: Species::Mannequin,
                width: 0.6,
                height: 1.8,
                eye_height: 1.62,
                category: "misc".into(),
                max_health: 20.0,
                speed: 0.1,
                flying_speed: 0.4,
                attack_damage: 0.0,
                attack_knockback: 0.0,
                follow_range: 16.0,
                step_height: 0.6,
                armor: 0.0,
                armor_toughness: 0.0,
                knockback_resistance: 0.0,
                jump_strength: 0.42,
                attributes: Arc::default(),
                gravity: 0.08,
                safe_fall_distance: 3.0,
                fall_damage_multiplier: 1.0,
                water_efficiency: 0.0,
                water_slowdown: 0.8,
                air_speed: 0.02,
                baby_scale: 0.5,
                head_rot_speed: 10.0,
                max_head_x_rot: 40.0,
                max_head_y_rot: 75.0,
                ambient_interval: 120,
                flags: Flags::default(),
                model: Arc::new(Part {
                    name: "root".into(),
                    pose: [0.0, 0.0, 0.0, 0.0, 0.0, 0.0, 1.0, 1.0, 1.0],
                    polygons: vec![],
                    children: vec![],
                }),
                layers: Default::default(),
                texture: String::new(),
                brain: Brain::Passive,
                anim: anim::Anim::Rabbit,
                sounds: [""; 3],
                step_sound: None,
                food: Arc::default(),
                tempt: Arc::default(),
            }),
            Position::ZERO,
            0.0,
        )
    }

    fn stack(item: &str, count: u32, damage: u32) -> ItemStack {
        let mut stack = ItemStack::new(item, count);
        stack.damage = damage;
        stack.data_mut().repair_cost = 9;
        stack
    }

    #[test]
    fn player_state_is_only_built_for_a_listener() {
        let builds = Cell::new(0);
        for listened in [false, true] {
            let drawn = if_player_render_listened(listened, || {
                builds.set(builds.get() + 1);
                let mob = player();
                let render = NativePlayerRender::of(
                    &mob,
                    anim::state(&mob, 0.25),
                    ("Player", true),
                    None,
                    [None; 2],
                );
                assert_eq!(render.name, "Player");
                assert!(render.local);
                true
            });
            assert_eq!(drawn, listened);
            assert_eq!(builds.get(), i32::from(listened));
        }
    }

    #[test]
    fn player_state_matches_the_interpolated_frame_and_complete_stacks() {
        let mut mob = player();
        mob.extra.avatar_pose = Pose::Swimming;
        mob.extra.swim.old = 0.2;
        mob.extra.swim.amount = 0.8;
        mob.extra.passenger = true;
        mob.extra.sprinting = true;
        mob.extra.main_hand_left = true;
        mob.extra.swing_right_arm = Some(true);
        mob.extra.using_item = 7;
        mob.extra.using_off_hand = true;
        mob.anim.attack_anim_o = 0.2;
        mob.anim.attack_anim = 0.8;
        mob.fire_ticks = 12;
        mob.invisible = true;
        mob.glowing = true;
        mob.ticks_frozen = 73;
        mob.extra.armor = [
            Some(stack("minecraft:diamond_helmet", 1, 13)),
            Some(stack("minecraft:elytra", 1, 21)),
            Some(stack("minecraft:diamond_leggings", 1, 8)),
            Some(stack("minecraft:diamond_boots", 1, 3)),
        ];
        let held = [
            stack("minecraft:diamond_sword", 1, 19),
            stack("minecraft:shield", 1, 11),
        ];
        let state = anim::state(&mob, 0.25);
        let uuid = 0x12345678_9abc_def0_1234_56789abcdef0;
        let render = NativePlayerRender::of(
            &mob,
            state,
            ("Player", false),
            Some(uuid),
            [Some(&held[0]), Some(&held[1])],
        );
        assert_eq!(render.uuid, Some(uuid));
        assert_eq!(render.pose, "SWIMMING");
        assert_eq!(render.crouch_amount, 0.0);
        assert!((render.swim_amount - 0.35).abs() < 1e-6);
        assert_eq!(render.riding, mob.extra.passenger);
        assert!(!render.sleeping);
        assert_eq!(render.sprinting, mob.extra.sprinting);
        assert_eq!(render.attack_progress, state.swing);
        assert!((render.attack_progress - 0.35).abs() < 1e-6);
        assert_eq!(render.swing_hand, Hand::Off);
        assert_eq!(render.using_item, Some(Hand::Off));
        assert_eq!(render.use_ticks, 7.25);
        assert!(!render.main_hand_right);
        assert_eq!(render.main_hand, Some(&held[0]));
        assert_eq!(render.off_hand, Some(&held[1]));
        assert!(std::ptr::eq(render.main_hand.expect("main hand"), &held[0]));
        assert!(std::ptr::eq(render.off_hand.expect("off hand"), &held[1]));
        for (rendered, expected) in render.armor.into_iter().zip(&mob.extra.armor) {
            assert_eq!(rendered, expected.as_ref());
            assert!(std::ptr::eq(
                rendered.expect("rendered armor"),
                expected.as_ref().expect("equipped armor"),
            ));
        }
        assert!(render.elytra);
        assert_eq!(render.on_fire, mob.is_on_fire());
        assert_eq!(render.invisible, mob.invisible);
        assert_eq!(render.glowing, mob.glowing);
        assert_eq!(render.frozen_ticks, mob.ticks_frozen);
    }

    #[test]
    fn player_pose_names_cover_the_client_poses_and_death() {
        let mut mob = player();
        for (pose, name) in [
            (Pose::Standing, "STANDING"),
            (Pose::Crouching, "CROUCHING"),
            (Pose::Swimming, "SWIMMING"),
            (Pose::FallFlying, "FALL_FLYING"),
            (Pose::Sleeping, "SLEEPING"),
            (Pose::SpinAttack, "SPIN_ATTACK"),
        ] {
            mob.extra.avatar_pose = pose;
            let render = NativePlayerRender::of(
                &mob,
                anim::state(&mob, 0.5),
                ("Player", false),
                None,
                [None; 2],
            );
            assert_eq!(render.pose, name);
            assert_eq!(
                render.crouch_amount,
                if pose == Pose::Crouching { 1.0 } else { 0.0 },
            );
            assert_eq!(render.sleeping, pose == Pose::Sleeping);
        }
        for (health, death_time) in [(0.0, 0), (20.0, 1)] {
            mob.health = health;
            mob.death_time = death_time;
            let render = NativePlayerRender::of(
                &mob,
                anim::state(&mob, 0.5),
                ("Player", false),
                None,
                [None; 2],
            );
            assert_eq!(render.pose, "DYING");
        }
    }

    #[test]
    fn player_hands_are_logical_for_either_main_arm() {
        let mut mob = player();
        for main_left in [false, true] {
            for swing_right in [false, true] {
                for using_off in [false, true] {
                    mob.extra.main_hand_left = main_left;
                    mob.extra.swing_right_arm = Some(swing_right);
                    mob.extra.using_off_hand = using_off;
                    mob.extra.using_item = 3;
                    let render = NativePlayerRender::of(
                        &mob,
                        anim::state(&mob, 0.5),
                        ("Player", false),
                        None,
                        [None; 2],
                    );
                    assert_eq!(render.main_hand_right, !main_left);
                    assert_eq!(
                        render.swing_hand,
                        if swing_right == !main_left {
                            Hand::Main
                        } else {
                            Hand::Off
                        },
                    );
                    assert_eq!(
                        render.using_item,
                        Some(if using_off { Hand::Off } else { Hand::Main }),
                    );
                    assert_eq!(render.use_ticks, 3.5);
                }
            }
        }
        mob.extra.swing_right_arm = None;
        mob.extra.using_item = 0;
        let mut state = anim::state(&mob, 0.5);
        for (swing, expected) in [(-0.25, 0.0), (1.25, 1.0)] {
            state.swing = swing;
            let render = NativePlayerRender::of(&mob, state, ("Player", false), None, [None; 2]);
            assert_eq!(render.swing_hand, Hand::Main);
            assert_eq!(render.using_item, None);
            assert_eq!(render.use_ticks, 0.0);
            assert_eq!(render.attack_progress, expected);
        }
    }

    #[test]
    fn unavailable_remote_stacks_and_uuid_stay_unavailable() {
        let mut mob = player();
        mob.extra.set_main_hand_stack(Some(stack("minecraft:bow", 1, 6)));
        mob.extra.off_hand = Some("minecraft:shield".into());
        let render = NativePlayerRender::of(
            &mob,
            anim::state(&mob, 0.5),
            ("Remote", false),
            None,
            [mob.extra.main_hand_stack(), None],
        );
        assert_eq!(render.name, "Remote");
        assert!(!render.local);
        assert_eq!(render.uuid, None);
        assert_eq!(render.main_hand, mob.extra.main_hand_stack());
        assert!(render.main_hand.is_some());
        assert!(render.off_hand.is_none());
        assert!(render.armor.iter().all(Option::is_none));
        assert!(!render.elytra);
    }

    fn close(a: Vec3, b: Vec3) -> bool {
        (a - b).length() < 1e-5
    }

    #[test]
    fn the_pose_faces_where_the_body_faces() {
        // Entity yaw 0 faces south (+z), 90 west (-x), 180 north, 270 east.
        for (yaw, ahead, left) in [
            (0.0, Vec3::Z, Vec3::X),
            (90.0, Vec3::NEG_X, Vec3::Z),
            (180.0, Vec3::NEG_Z, Vec3::NEG_X),
            (270.0, Vec3::X, Vec3::NEG_Z),
        ] {
            let pose = mob_pose(DVec3::ZERO, yaw);
            assert!(close(pose.transform_vector3(Vec3::Z), ahead), "{yaw}");
            assert!(close(pose.transform_vector3(Vec3::X), left), "{yaw}");
            assert!(close(pose.transform_vector3(Vec3::Y), Vec3::Y), "{yaw}");
        }
    }

    #[test]
    fn the_pose_starts_at_the_feet() {
        let at = DVec3::new(3.0, -1.5, 0.25);
        let pose = mob_pose(at, 37.0);
        assert!(close(pose.transform_point3(Vec3::ZERO), at.as_vec3()));
    }
}
