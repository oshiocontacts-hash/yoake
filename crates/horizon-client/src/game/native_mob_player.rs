use horizon_client::{
    items::ItemStack,
    mobs::{Mob, render},
    save, skins,
};
use horizon_world::nbt::{self, Limits, Tag};
use horizon_world::player_inventory::EquipmentSlot;

use super::{
    Game,
    native_avatar_equipment::{NativeEquipmentDrawOptions, NativeHumanoidPart},
    native_block_entity_draw::BlockEntityDraw,
    native_mob_draw::NativeMobRender,
};

const MESSAGE_LIMIT: usize = 30_000;
const PROFILE_NAME_LIMIT: usize = 64;
const TEXTURES_LIMIT: usize = 16_384;
const SIGNATURE_LIMIT: usize = 8_192;
const NAME_TAG_LIMIT: usize = 1_024;
const NBT_STRING_LIMIT: usize = 4_096;
const SKIN_IDLE_FRAMES: u16 = 120;
const NBT_LIMITS: Limits = Limits {
    max_bytes: MESSAGE_LIMIT,
    max_nodes: 4_096,
    max_depth: 32,
};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum NativePlayerModel {
    Auto,
    Wide,
    Slim,
}

#[derive(Clone, Debug, PartialEq)]
pub struct NativePlayerAppearance {
    pub uuid: u128,
    pub profile_name: String,
    pub textures: Option<(String, Option<String>)>,
    pub model: NativePlayerModel,
    pub main_hand_right: bool,
    pub hands: [Option<ItemStack>; 2],
    pub armor: [Option<ItemStack>; 4],
    pub cape: bool,
    pub name_tag: Option<String>,
}

impl NativePlayerAppearance {
    /// Encodes version 1, or returns an empty vector if fields are invalid or exceed limits.
    pub fn encode(&self) -> Vec<u8> {
        self.try_encode().unwrap_or_default()
    }

    fn try_encode(&self) -> Option<Vec<u8>> {
        let mut bytes = Vec::new();
        bytes.push(1);
        bytes.extend_from_slice(&self.uuid.to_be_bytes());
        bytes.push(match self.model {
            NativePlayerModel::Auto => 0,
            NativePlayerModel::Wide => 1,
            NativePlayerModel::Slim => 2,
        });
        bytes.push(u8::from(self.main_hand_right) | (u8::from(self.cape) << 1));
        put_string(&mut bytes, &self.profile_name, PROFILE_NAME_LIMIT)?;
        bytes.push(u8::from(self.textures.is_some()));
        if let Some((value, signature)) = &self.textures {
            put_string(&mut bytes, value, TEXTURES_LIMIT)?;
            put_optional_string(&mut bytes, signature.as_deref(), SIGNATURE_LIMIT)?;
        }
        put_optional_string(&mut bytes, self.name_tag.as_deref(), NAME_TAG_LIMIT)?;
        for stack in self.hands.iter().chain(&self.armor) {
            if let Some(stack) = stack {
                if stack.item.is_empty() || stack.item.as_ref() == "minecraft:air" {
                    return None;
                }
                if !(1..=99).contains(&stack.count) {
                    return None;
                }
                let tag = save::item_to_nbt(stack);
                if !bounded_nbt(&tag) {
                    return None;
                }
                put_blob(
                    &mut bytes,
                    &nbt::write_network(&tag, NBT_LIMITS).ok()?,
                    MESSAGE_LIMIT,
                )?;
            } else {
                put_blob(&mut bytes, &[], MESSAGE_LIMIT)?;
            }
        }
        (bytes.len() <= MESSAGE_LIMIT).then_some(bytes)
    }

    /// Rejects unknown versions, oversized fields, malformed stacks and trailing bytes.
    pub fn decode(bytes: &[u8]) -> Option<Self> {
        if bytes.len() > MESSAGE_LIMIT {
            return None;
        }
        let mut input = Input(bytes);
        if input.byte()? != 1 {
            return None;
        }
        let uuid = u128::from_be_bytes(input.take(16)?.try_into().ok()?);
        let model = match input.byte()? {
            0 => NativePlayerModel::Auto,
            1 => NativePlayerModel::Wide,
            2 => NativePlayerModel::Slim,
            _ => return None,
        };
        let flags = input.byte()?;
        if flags & !3 != 0 {
            return None;
        }
        let profile_name = input.string(PROFILE_NAME_LIMIT)?;
        let textures = if input.present()? {
            Some((
                input.string(TEXTURES_LIMIT)?,
                input.optional_string(SIGNATURE_LIMIT)?,
            ))
        } else {
            None
        };
        let name_tag = input.optional_string(NAME_TAG_LIMIT)?;
        let hands = [input.stack()?, input.stack()?];
        let armor = [
            input.stack()?,
            input.stack()?,
            input.stack()?,
            input.stack()?,
        ];
        if !input.0.is_empty() {
            return None;
        }
        Some(Self {
            uuid,
            profile_name,
            textures,
            model,
            main_hand_right: flags & 1 != 0,
            hands,
            armor,
            cape: flags & 2 != 0,
            name_tag,
        })
    }
}

fn texture_model(value: &str) -> NativePlayerModel {
    let Some(json) = horizon_client::skins::base64_decode(value)
        .and_then(|bytes| serde_json::from_slice::<serde_json::Value>(&bytes).ok())
    else {
        return NativePlayerModel::Auto;
    };
    match json
        .get("textures")
        .and_then(|textures| textures.get("SKIN"))
        .and_then(|skin| skin.get("metadata"))
        .and_then(|metadata| metadata.get("model"))
        .and_then(serde_json::Value::as_str)
    {
        Some("slim") => NativePlayerModel::Slim,
        Some("default" | "wide" | "classic") => NativePlayerModel::Wide,
        _ => NativePlayerModel::Auto,
    }
}

impl Game<'_> {
    pub(super) fn native_player_appearance_impl(
        &self,
        player: u128,
    ) -> Option<NativePlayerAppearance> {
        super::native_entity_query::require_server_context(
            self.options.execution_role,
            self.remote.is_some(),
            self.lan_acting.is_some(),
            self.lan.is_some(),
        )
        .ok()?;
        horizon_client::crash::native_mod_call()?;
        self.native_lifecycle.entity_query_tick()?;
        let hub = &self.lan.as_ref()?.hub;
        let guest = hub
            .guests
            .iter()
            .find(|guest| guest.placed && guest.uuid == player)?;
        let textures = guest
            .properties
            .iter()
            .find(|property| property.name == "textures")
            .or_else(|| {
                if hub.singleplayer_owner != Some(player) {
                    return None;
                }
                hub.lan
                    .shared
                    .config
                    .owner_profile
                    .as_deref()?
                    .properties()
                    .iter()
                    .find(|property| property.name == "textures")
            })
            .map(|property| (property.value.clone(), property.signature.clone()));
        let model = textures
            .as_ref()
            .map_or(NativePlayerModel::Auto, |(value, _)| texture_model(value));
        Some(NativePlayerAppearance {
            uuid: guest.uuid,
            profile_name: guest.name.clone(),
            textures,
            model,
            main_hand_right: guest.info.main_hand != 0,
            hands: [
                guest.inventory.get(guest.selected).cloned().flatten(),
                guest.offhand.clone(),
            ],
            armor: guest.armor.clone(),
            cape: guest.info.model_customisation & 1 != 0,
            name_tag: Some(guest.name.clone()),
        })
    }
}

#[derive(Default)]
pub(super) struct NativeMobPlayerCache {
    skins: Vec<AppearanceSkin>,
    player: Option<Mob>,
    pending_trims: Vec<ItemStack>,
}

struct AppearanceSkin {
    uuid: u128,
    property: Option<(String, Option<String>)>,
    textures: Option<skins::Textures>,
    prepared: bool,
    location: String,
    slim: bool,
    downloaded: bool,
    cape: Option<String>,
    used: bool,
    idle_frames: u16,
}

impl AppearanceSkin {
    fn new(appearance: &NativePlayerAppearance) -> Self {
        let location = horizon_client::tab_list::default_skin(appearance.uuid).to_owned();
        Self {
            uuid: appearance.uuid,
            property: appearance.textures.clone(),
            textures: None,
            prepared: false,
            slim: location.contains("/slim/"),
            location,
            downloaded: false,
            cape: None,
            used: true,
            idle_frames: 0,
        }
    }

    fn matches(&self, appearance: &NativePlayerAppearance) -> bool {
        self.uuid == appearance.uuid && self.property == appearance.textures
    }

    fn begin_frame(&mut self) -> bool {
        if self.used {
            self.used = false;
            self.idle_frames = 0;
            true
        } else {
            self.idle_frames = self.idle_frames.saturating_add(1);
            false
        }
    }

    fn reset(&mut self) {
        self.location = horizon_client::tab_list::default_skin(self.uuid).to_owned();
        self.slim = self.location.contains("/slim/");
        self.downloaded = false;
        self.cape = None;
    }
}

impl NativeMobPlayerCache {
    fn queue_trims(&mut self, armor: &[Option<ItemStack>; 4]) {
        for stack in armor.iter().flatten() {
            if stack
                .extra
                .as_ref()
                .is_some_and(|extra| extra.trim.is_some())
                && !self.pending_trims.contains(stack)
            {
                self.pending_trims.push(stack.clone());
            }
        }
    }

    fn appearance_skin(&mut self, appearance: &NativePlayerAppearance) -> &mut AppearanceSkin {
        let index = match self
            .skins
            .iter()
            .position(|entry| entry.matches(appearance))
        {
            Some(index) => index,
            None => {
                self.skins.push(AppearanceSkin::new(appearance));
                self.skins.len() - 1
            }
        };
        let entry = &mut self.skins[index];
        entry.used = true;
        entry
    }

    fn prune_inactive_skins(&mut self) {
        self.skins
            .retain(|entry| entry.idle_frames < SKIN_IDLE_FRAMES);
    }
}

fn take_pending_trims(cache: &mut Option<NativeMobPlayerCache>) -> Vec<ItemStack> {
    cache
        .as_mut()
        .map_or_else(Vec::new, |cache| std::mem::take(&mut cache.pending_trims))
}

fn player_origin(pose: glam::Mat4, body_yaw: f32) -> glam::Mat4 {
    pose * glam::Mat4::from_rotation_y(body_yaw.to_radians())
}

fn model_slim(model: NativePlayerModel, auto_slim: bool) -> bool {
    match model {
        NativePlayerModel::Auto => auto_slim,
        NativePlayerModel::Wide => false,
        NativePlayerModel::Slim => true,
    }
}

impl Game<'_> {
    pub(super) fn native_mob_player_trim_stacks(&self) -> Vec<ItemStack> {
        take_pending_trims(&mut self.native_mob_player_cache.borrow_mut())
    }

    pub(super) fn native_prepare_mob_players(&mut self) {
        let Some(mut cache) = self.native_mob_player_cache.get_mut().take() else {
            return;
        };
        for entry in &mut cache.skins {
            if !entry.begin_frame() {
                continue;
            }
            if !entry.prepared {
                entry.textures = entry.property.as_ref().map(|(value, signature)| {
                    let signature = if signature.is_some() {
                        skins::Signature::Signed
                    } else {
                        skins::Signature::Unsigned
                    };
                    skins::unpack(value, signature, &skins::TEXTURE_URIS)
                });
                entry.prepared = true;
            }
            let Some(textures) = entry.textures.clone() else {
                entry.reset();
                continue;
            };
            let location = {
                let mut downloads = self.skin_downloads.borrow_mut();
                let require_secure = self
                    .remote
                    .as_ref()
                    .is_none_or(|remote| entry.uuid != remote.profile());
                skins::lookup(&textures, require_secure, |kind, url| {
                    downloads.state(kind, url)
                })
            };
            let Some(location) = location.filter(|location| self.again_skin(location)) else {
                entry.reset();
                continue;
            };
            entry.location = location;
            entry.slim = textures.slim;
            entry.downloaded = true;
            entry.cape = skins::cape(&textures).filter(|cape| self.again_skin(cape));
        }
        cache.prune_inactive_skins();
        *self.native_mob_player_cache.get_mut() = Some(cache);
    }

    pub(super) fn native_draw_mob_as_player_impl(
        &self,
        mob: &NativeMobRender<'_>,
        draw: &mut BlockEntityDraw,
        appearance: &NativePlayerAppearance,
    ) -> bool {
        if appearance
            .textures
            .as_ref()
            .is_some_and(|(value, signature)| {
                value.len() > TEXTURES_LIMIT
                    || signature
                        .as_ref()
                        .is_some_and(|signature| signature.len() > SIGNATURE_LIMIT)
            })
        {
            return false;
        }
        let Some(kind) = self.res.player_type().as_ref().cloned() else {
            return false;
        };
        let (skin, slim, cape, mut player) = {
            let mut borrowed = self.native_mob_player_cache.borrow_mut();
            let cache = borrowed.get_or_insert_with(NativeMobPlayerCache::default);
            cache.queue_trims(&appearance.armor);
            let entry = cache.appearance_skin(appearance);
            let available = entry.downloaded
                && self.skin_slots.contains(&entry.location)
                && self.gui.atlas.sprites.contains_key(&entry.location);
            let (skin, slim) = if available {
                self.skin_downloads.borrow_mut().drawn(&entry.location);
                let slim = model_slim(appearance.model, entry.slim);
                (entry.location.clone(), slim)
            } else {
                if entry.downloaded {
                    self.skin_downloads.borrow_mut().again(&entry.location);
                }
                let skin = horizon_client::tab_list::default_skin(appearance.uuid).to_owned();
                let slim = model_slim(appearance.model, skin.contains("/slim/"));
                (skin, slim)
            };
            let cape = entry
                .cape
                .as_ref()
                .filter(|cape| {
                    appearance.cape
                        && self.skin_slots.contains(cape)
                        && self.gui.atlas.sprites.contains_key(*cape)
                })
                .cloned();
            if let Some(cape) = &cape {
                self.skin_downloads.borrow_mut().drawn(cape);
            }
            let player = cache.player.take().unwrap_or_else(|| {
                Mob::new(
                    mob.id,
                    kind.clone(),
                    horizon_core::geometry::Vec3::ZERO,
                    mob.body_yaw,
                )
            });
            (skin, slim, cape, player)
        };
        player.kind = kind;
        player.id = mob.id;
        player.position = horizon_core::geometry::Vec3::ZERO;
        player.previous = player.position;
        player.yaw = mob.body_yaw;
        player.previous_yaw = mob.body_yaw;
        player.body_yaw = mob.body_yaw;
        player.previous_body_yaw = mob.body_yaw;
        player.head_yaw = mob.body_yaw + mob.head_yaw;
        player.previous_head_yaw = player.head_yaw;
        player.head_pitch = mob.pitch;
        player.previous_head_pitch = mob.pitch;
        player.on_ground = mob.on_ground;
        player.health = if mob.death_time > 0 { 0.0 } else { 20.0 };
        player.hurt_time = mob.hurt_time;
        player.death_time = mob.death_time;
        player.age = (mob.age - mob.partial).max(0.0) as u32;
        player.limb_position = mob.walk_position + mob.walk_speed * (1.0 - mob.partial);
        player.limb_speed = mob.walk_speed;
        player.previous_limb_speed = mob.walk_speed;
        player.extra.main_hand_left = !appearance.main_hand_right;
        player
            .extra
            .set_main_hand_stack(appearance.hands[0].clone());
        player.extra.off_hand = appearance.hands[1].as_ref().map(|stack| stack.item.clone());
        player.skin = Some(skin);
        player.slim = slim;
        player.cape = cape;
        let raw_origin = player_origin(draw.pose_matrix(), mob.body_yaw);
        let origin = draw.equipment_frame(raw_origin);
        let light = draw.light;
        let start = draw.equipment_vertices().len();
        let atlas = |name: &str| {
            self.gui
                .atlas
                .sprites
                .get(name)
                .map(|rect| [rect.u0, rect.v0, rect.u1, rect.v1])
        };
        render::render_posed(
            &player,
            mob.partial,
            origin,
            &atlas,
            light,
            self.res.mob_data(),
            draw.equipment_vertices(),
        );
        if draw.equipment_vertices().len() == start {
            if let Some(cache) = self.native_mob_player_cache.borrow_mut().as_mut() {
                cache.player = Some(player);
            }
            return false;
        }
        let parts: [(EquipmentSlot, &[(NativeHumanoidPart, &str)]); 4] = [
            (EquipmentSlot::Head, &[(NativeHumanoidPart::Head, "head")]),
            (
                EquipmentSlot::Chest,
                &[
                    (NativeHumanoidPart::Body, "body"),
                    (NativeHumanoidPart::LeftArm, "left_arm"),
                    (NativeHumanoidPart::RightArm, "right_arm"),
                ],
            ),
            (
                EquipmentSlot::Legs,
                &[
                    (NativeHumanoidPart::Body, "body"),
                    (NativeHumanoidPart::LeftLeg, "left_leg"),
                    (NativeHumanoidPart::RightLeg, "right_leg"),
                ],
            ),
            (
                EquipmentSlot::Feet,
                &[
                    (NativeHumanoidPart::LeftLeg, "left_leg"),
                    (NativeHumanoidPart::RightLeg, "right_leg"),
                ],
            ),
        ];
        for (stack, (slot, pieces)) in appearance.armor.iter().zip(parts) {
            let Some(stack) = stack else {
                continue;
            };
            for &(part, name) in pieces {
                let Some(frame) = render::part_transform(
                    &player,
                    mob.partial,
                    glam::DVec3::ZERO,
                    self.res.mob_data(),
                    name,
                ) else {
                    continue;
                };
                self.native_draw_armor_part(
                    draw,
                    stack,
                    slot,
                    part,
                    raw_origin * frame,
                    NativeEquipmentDrawOptions::new(light),
                );
            }
        }
        if let Some(rect) = player
            .cape
            .as_deref()
            .and_then(|cape| self.gui.atlas.sprites.get(cape))
        {
            let layers = appearance.armor[1]
                .as_ref()
                .and_then(|stack| self.res.items().get(&stack.item))
                .and_then(|item| item.equip_asset.as_ref())
                .and_then(|asset| self.armor_stand_assets.equipment.get(asset));
            if !layers.is_some_and(|layers| layers.contains_key("wings")) {
                let mut base =
                    origin * render::base_transform(&player, mob.partial, glam::DVec3::ZERO);
                if layers.is_some_and(|layers| layers.contains_key("humanoid")) {
                    base *= glam::Mat4::from_translation(glam::Vec3::new(0.0, -0.053125, 0.06875));
                }
                render::cape(
                    &player,
                    mob.partial,
                    base,
                    [rect.u0, rect.v0, rect.u1, rect.v1],
                    light,
                    self.res.mob_data(),
                    draw.equipment_vertices(),
                );
            }
        }
        let chest = appearance.armor[1].as_ref();
        self.wings_geometry(
            chest.map(|stack| &*stack.item),
            chest,
            player.cape.as_deref(),
            false,
            origin * render::base_transform(&player, mob.partial, glam::DVec3::ZERO),
            horizon_client::mobs::ElytraAnimation::RESTING,
            false,
            light,
            self.wings_overlay(chest),
            draw.equipment_vertices(),
        );
        let at = self
            .mobs
            .iter()
            .find(|carrier| carrier.id == mob.id)
            .map(|carrier| [carrier.position.x, carrier.position.y, carrier.position.z])
            .unwrap_or([0.0; 3]);
        let mut owner = super::item_draw::owner_at(at, mob.body_yaw, &player.kind.id);
        owner.main_arm_left = !appearance.main_hand_right;
        for (index, stack) in appearance.hands.iter().enumerate() {
            let Some(stack) = stack else {
                continue;
            };
            let off_hand = index == 1;
            let left = off_hand != owner.main_arm_left;
            let Some(frame) = render::held_item_transform(
                &player,
                mob.partial,
                glam::DVec3::ZERO,
                self.res.mob_data(),
                left,
            ) else {
                continue;
            };
            use horizon_assets::item_model::DisplayContext;
            let display = if left {
                DisplayContext::ThirdPersonLeftHand
            } else {
                DisplayContext::ThirdPersonRightHand
            };
            let hand = if off_hand {
                horizon_client::cit::Hand::Off
            } else {
                horizon_client::cit::Hand::Main
            };
            let mut state = self.resolve_item_in_hand(
                stack,
                display,
                Some(&owner),
                (mob.id as i32).wrapping_add(if left { 1 } else { 2 }),
                hand,
            );
            if state.native_owner.is_some() {
                state.native_entity_id = Some(mob.id as i32);
                state.native_swing_progress = self
                    .mobs
                    .iter()
                    .find(|entity| entity.id == mob.id)
                    .map(|entity| horizon_client::mobs::anim::state(entity, mob.partial).swing);
            }
            self.submit_item_world(&state, origin * frame, light, draw.equipment_vertices());
        }
        if let Some(cache) = self.native_mob_player_cache.borrow_mut().as_mut() {
            cache.player = Some(player);
        }
        // CHECK: Compare carrier motion, hurt/death poses, equipment and capes with vanilla in-game.
        true
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn appearance() -> NativePlayerAppearance {
        NativePlayerAppearance {
            uuid: 42,
            profile_name: "Alex".into(),
            textures: Some(("textures".into(), Some("signature".into()))),
            model: NativePlayerModel::Slim,
            main_hand_right: false,
            hands: [Some(ItemStack::new("minecraft:stick", 3)), None],
            armor: [
                None,
                None,
                Some(ItemStack::new("minecraft:iron_leggings", 1)),
                None,
            ],
            cape: true,
            name_tag: Some("Alex".into()),
        }
    }

    #[test]
    fn appearance_round_trips_equipment_and_profile() {
        let mut original = appearance();
        let chest = original.armor[2].as_mut().unwrap();
        chest.data_mut().trim = Some(("minecraft:iron".into(), "minecraft:sentry".into()));
        let encoded = original.encode();
        assert!(!encoded.is_empty());
        assert_eq!(NativePlayerAppearance::decode(&encoded), Some(original));
    }

    #[test]
    fn appearance_rejects_truncation_invalid_headers_and_trailing_data() {
        let encoded = appearance().encode();
        for end in 0..encoded.len() {
            assert!(NativePlayerAppearance::decode(&encoded[..end]).is_none());
        }
        let mut invalid = encoded.clone();
        invalid[0] = 2;
        assert!(NativePlayerAppearance::decode(&invalid).is_none());
        invalid = encoded.clone();
        invalid[17] = 3;
        assert!(NativePlayerAppearance::decode(&invalid).is_none());
        invalid = encoded.clone();
        invalid[18] = 4;
        assert!(NativePlayerAppearance::decode(&invalid).is_none());
        invalid = encoded;
        invalid.push(0);
        assert!(NativePlayerAppearance::decode(&invalid).is_none());
    }

    #[test]
    fn appearance_rejects_oversized_fields_and_messages() {
        let mut oversized = appearance();
        oversized.profile_name = "a".repeat(PROFILE_NAME_LIMIT + 1);
        assert!(oversized.encode().is_empty());
        oversized = appearance();
        oversized.textures = Some(("x".repeat(TEXTURES_LIMIT + 1), None));
        assert!(oversized.encode().is_empty());
        oversized.textures = Some(("x".into(), Some("s".repeat(SIGNATURE_LIMIT + 1))));
        assert!(oversized.encode().is_empty());
        assert!(NativePlayerAppearance::decode(&vec![0; MESSAGE_LIMIT + 1]).is_none());
    }

    #[test]
    fn appearance_rejects_stacks_outside_item_codec_limits() {
        let mut invalid = appearance();
        invalid.hands[0] = Some(ItemStack::new("minecraft:stick", 100));
        assert!(invalid.encode().is_empty());
        invalid.hands[0] = Some(ItemStack::new("minecraft:air", 1));
        assert!(invalid.encode().is_empty());
    }

    #[test]
    fn skins_are_shared_by_profile_but_not_by_carrier() {
        let mut cache = NativeMobPlayerCache::default();
        let mut appearance = appearance();
        cache.appearance_skin(&appearance);
        cache.appearance_skin(&appearance);
        assert_eq!(cache.skins.len(), 1);
        appearance.textures = Some(("other".into(), None));
        cache.appearance_skin(&appearance);
        assert_eq!(cache.skins.len(), 2);
        appearance.uuid += 1;
        cache.appearance_skin(&appearance);
        assert_eq!(cache.skins.len(), 3);
    }

    #[test]
    fn many_visible_profiles_do_not_evict_each_other() {
        let mut cache = NativeMobPlayerCache::default();
        let mut appearance = appearance();
        for uuid in 0..129 {
            appearance.uuid = uuid;
            cache.appearance_skin(&appearance);
        }
        appearance.uuid = 0;
        cache.appearance_skin(&appearance);
        assert_eq!(cache.skins.len(), 129);
        assert!(cache.skins.iter().all(|skin| skin.used));
    }

    #[test]
    fn inactive_profiles_expire_without_evicting_visible_ones() {
        let mut cache = NativeMobPlayerCache::default();
        let mut appearance = appearance();
        cache.appearance_skin(&appearance);
        appearance.uuid += 1;
        cache.appearance_skin(&appearance);
        for _ in 0..=SKIN_IDLE_FRAMES {
            for skin in &mut cache.skins {
                skin.begin_frame();
            }
            cache.prune_inactive_skins();
            cache.appearance_skin(&appearance);
        }
        assert_eq!(cache.skins.len(), 1);
        assert_eq!(cache.skins[0].uuid, appearance.uuid);
    }

    #[test]
    fn explicit_model_is_used_while_a_skin_is_pending() {
        assert!(model_slim(NativePlayerModel::Slim, false));
        assert!(!model_slim(NativePlayerModel::Wide, true));
        assert!(model_slim(NativePlayerModel::Auto, true));
        assert!(!model_slim(NativePlayerModel::Auto, false));
    }

    #[test]
    fn without_a_mod_trim_preparation_does_not_create_a_cache() {
        let mut cache = None;
        assert!(take_pending_trims(&mut cache).is_empty());
        assert!(cache.is_none());
    }

    #[test]
    fn mod_appearance_queues_trimmed_armor_once_for_the_next_frame() {
        let mut appearance = appearance();
        let stack = appearance.armor[2].as_mut().unwrap();
        stack.data_mut().trim = Some(("minecraft:iron".into(), "minecraft:sentry".into()));
        let expected = stack.clone();
        let mut cache = Some(NativeMobPlayerCache::default());
        cache.as_mut().unwrap().queue_trims(&appearance.armor);
        cache.as_mut().unwrap().queue_trims(&appearance.armor);
        assert_eq!(take_pending_trims(&mut cache), vec![expected]);
        assert!(take_pending_trims(&mut cache).is_empty());
    }

    #[test]
    fn carrier_frames_remain_independent_for_the_same_appearance() {
        let first = glam::Mat4::from_translation(glam::Vec3::new(1.0, 2.0, 3.0))
            * glam::Mat4::from_rotation_y((-45.0_f32).to_radians());
        let second = glam::Mat4::from_translation(glam::Vec3::new(4.0, 5.0, 6.0))
            * glam::Mat4::from_rotation_y(90.0_f32.to_radians());
        assert_eq!(
            player_origin(first, 45.0).transform_point3(glam::Vec3::ZERO),
            glam::Vec3::new(1.0, 2.0, 3.0),
        );
        assert_eq!(
            player_origin(second, -90.0).transform_point3(glam::Vec3::ZERO),
            glam::Vec3::new(4.0, 5.0, 6.0),
        );
    }
}

fn put_blob(bytes: &mut Vec<u8>, value: &[u8], limit: usize) -> Option<()> {
    if value.len() > limit || bytes.len().checked_add(2 + value.len())? > MESSAGE_LIMIT {
        return None;
    }
    bytes.extend_from_slice(&u16::try_from(value.len()).ok()?.to_be_bytes());
    bytes.extend_from_slice(value);
    Some(())
}

fn put_string(bytes: &mut Vec<u8>, value: &str, limit: usize) -> Option<()> {
    put_blob(bytes, value.as_bytes(), limit)
}

fn put_optional_string(bytes: &mut Vec<u8>, value: Option<&str>, limit: usize) -> Option<()> {
    bytes.push(u8::from(value.is_some()));
    if let Some(value) = value {
        put_string(bytes, value, limit)?;
    }
    Some(())
}

struct Input<'a>(&'a [u8]);

impl<'a> Input<'a> {
    fn take(&mut self, length: usize) -> Option<&'a [u8]> {
        if length > self.0.len() {
            return None;
        }
        let (value, remaining) = self.0.split_at(length);
        self.0 = remaining;
        Some(value)
    }

    fn byte(&mut self) -> Option<u8> {
        Some(self.take(1)?[0])
    }

    fn present(&mut self) -> Option<bool> {
        match self.byte()? {
            0 => Some(false),
            1 => Some(true),
            _ => None,
        }
    }

    fn blob(&mut self, limit: usize) -> Option<&'a [u8]> {
        let length = usize::from(u16::from_be_bytes(self.take(2)?.try_into().ok()?));
        if length > limit {
            return None;
        }
        self.take(length)
    }

    fn string(&mut self, limit: usize) -> Option<String> {
        String::from_utf8(self.blob(limit)?.to_vec()).ok()
    }

    fn optional_string(&mut self, limit: usize) -> Option<Option<String>> {
        if self.present()? {
            Some(Some(self.string(limit)?))
        } else {
            Some(None)
        }
    }

    fn stack(&mut self) -> Option<Option<ItemStack>> {
        let bytes = self.blob(MESSAGE_LIMIT)?;
        if bytes.is_empty() {
            return Some(None);
        }
        let (tag, consumed) = nbt::read_network(bytes, NBT_LIMITS).ok()?;
        if consumed != bytes.len() || !bounded_nbt(&tag) {
            return None;
        }
        let Tag::String(item) = tag.get("id")? else {
            return None;
        };
        let item = item.to_string().ok()?;
        if item.is_empty() || item == "minecraft:air" {
            return None;
        }
        let Tag::Int(count) = tag.get("count")? else {
            return None;
        };
        if !(1..=99).contains(count)
            || tag
                .get("components")
                .is_some_and(|components| !matches!(components, Tag::Compound(_)))
        {
            return None;
        }
        Some(Some(save::item_from_nbt(&tag)?))
    }
}

fn bounded_nbt(tag: &Tag) -> bool {
    fn visit(tag: &Tag, depth: usize, nodes: &mut usize, budget: &mut usize) -> Option<()> {
        if depth > NBT_LIMITS.max_depth || *nodes >= NBT_LIMITS.max_nodes {
            return None;
        }
        *nodes += 1;
        let size = match tag {
            Tag::End => 1,
            Tag::Byte(_) => 2,
            Tag::Short(_) => 3,
            Tag::Int(_) | Tag::Float(_) => 5,
            Tag::Long(_) | Tag::Double(_) => 9,
            Tag::String(value) => {
                if value.0.len() > NBT_STRING_LIMIT {
                    return None;
                }
                3 + value.0.len().checked_mul(3)?
            }
            Tag::ByteArray(values) => 5 + values.len(),
            Tag::IntArray(values) => 5 + values.len().checked_mul(4)?,
            Tag::LongArray(values) => 5 + values.len().checked_mul(8)?,
            Tag::List(values) => {
                if values.len() > NBT_LIMITS.max_nodes {
                    return None;
                }
                for value in values {
                    visit(value, depth + 1, nodes, budget)?;
                }
                6
            }
            Tag::Compound(values) => {
                for (key, value) in values {
                    if key.0.len() > NBT_STRING_LIMIT {
                        return None;
                    }
                    *budget = budget.checked_sub(3 + key.0.len().checked_mul(3)?)?;
                    visit(value, depth + 1, nodes, budget)?;
                }
                2
            }
        };
        *budget = budget.checked_sub(size)?;
        Some(())
    }
    let mut nodes = 0;
    let mut budget = MESSAGE_LIMIT;
    visit(tag, 0, &mut nodes, &mut budget).is_some()
}
