//! Vanilla equipment materials drawn on frames supplied by native mods.
//! Native equipment drawing requires API @API:equipment_draw@.

use super::{
    Game,
    native_block_entity_draw::BlockEntityDraw,
    native_mob_draw::{NativeMobRender, NativePlayerRender},
};
use glam::{Mat4, Vec3};
use horizon_client::{items::ItemStack, mesher::Vertex, mobs::render};
use horizon_world::player_inventory::EquipmentSlot;

impl Game<'_> {
    /// Draw one armour part directly on a camera-relative world frame.
    #[allow(clippy::too_many_arguments)]
    pub fn native_draw_armor_part(
        &self,
        draw: &mut BlockEntityDraw,
        stack: &ItemStack,
        slot: EquipmentSlot,
        part: NativeHumanoidPart,
        frame: Mat4,
        options: NativeEquipmentDrawOptions,
    ) -> bool {
        let layer = match (slot, part) {
            (EquipmentSlot::Head, NativeHumanoidPart::Head) => "helmet",
            (
                EquipmentSlot::Chest,
                NativeHumanoidPart::Body
                | NativeHumanoidPart::LeftArm
                | NativeHumanoidPart::RightArm,
            ) => "chestplate",
            (
                EquipmentSlot::Legs,
                NativeHumanoidPart::Body
                | NativeHumanoidPart::LeftLeg
                | NativeHumanoidPart::RightLeg,
            ) => "leggings",
            (EquipmentSlot::Feet, NativeHumanoidPart::LeftLeg | NativeHumanoidPart::RightLeg) => {
                "boots"
            }
            _ => return false,
        };
        let name = part.name();
        let Some(layers) = self.armor_stack_layers(stack, layer) else {
            return false;
        };
        if layers
            .iter()
            .any(|(model, ..)| render::find_part(model, name).is_none())
        {
            return false;
        }
        // CHECK: Compare pack-backed dye, trim and glint against vanilla armour on the lead build.
        let frame = draw.equipment_frame(frame);
        let start = draw.equipment_vertices().len();
        for (model, uv, color, overlay, emissive) in layers {
            let light = if emissive {
                render::FULL_BRIGHT
            } else {
                options.light
            };
            render::draw_part_frame(
                model,
                name,
                frame,
                uv,
                color,
                light,
                options.overlay.code(overlay),
                draw.equipment_vertices(),
            );
        }
        let drawn = draw.equipment_vertices().len() > start;
        if drawn {
            draw.stage_equipment_outline(start, options.outline);
        }
        drawn
    }

    /// Draw an existing player's articulated back layer on a supplied parent frame.
    #[allow(clippy::too_many_arguments)]
    pub fn native_draw_player_back(
        &self,
        draw: &mut BlockEntityDraw,
        player: &NativeMobRender<'_>,
        who: &NativePlayerRender<'_>,
        chest: Option<&ItemStack>,
        layer: NativeBackLayer,
        back: Mat4,
        options: NativeEquipmentDrawOptions,
    ) -> bool {
        let mob = if who.local {
            self.player_model.as_ref()
        } else {
            self.mobs.iter().find(|mob| mob.id == player.id)
        };
        let Some(mob) = mob.filter(|mob| mob.id == player.id && mob.kind.id == "minecraft:player")
        else {
            return false;
        };
        // CHECK: Verify cape and wing articulation on translated and rotated back frames in game.
        let back = draw.equipment_frame(back);
        let start = draw.equipment_vertices().len();
        match layer {
            NativeBackLayer::Wings => self.wings_geometry(
                chest.map(|stack| &*stack.item),
                chest,
                mob.cape.as_deref(),
                false,
                back,
                mob.extra.elytra.angles(player.partial),
                mob.extra.crouching,
                options.light,
                options.overlay.code(self.wings_overlay(chest)),
                draw.equipment_vertices(),
            ),
            NativeBackLayer::Cape => {
                let Some(rect) = mob
                    .cape
                    .as_deref()
                    .and_then(|cape| self.gui.atlas.sprites.get(cape))
                else {
                    return false;
                };
                if horizon_client::mobs::trader::is_invisible(mob) {
                    return false;
                }
                let layers = chest
                    .and_then(|stack| self.res.items().get(&*stack.item))
                    .and_then(|item| item.equip_asset.as_ref())
                    .and_then(|asset| self.armor_stand_assets.equipment.get(asset));
                if layers.is_some_and(|layers| layers.contains_key("wings")) {
                    return false;
                }
                let mut back = back;
                if layers.is_some_and(|layers| layers.contains_key("humanoid")) {
                    back *= Mat4::from_translation(Vec3::new(0.0, -0.053125, 0.06875));
                }
                render::cape_on_frame(
                    mob,
                    player.partial,
                    back,
                    [rect.u0, rect.v0, rect.u1, rect.v1],
                    options.light,
                    draw.equipment_vertices(),
                );
            }
        }
        let drawn = draw.equipment_vertices().len() > start;
        if drawn {
            draw.stage_equipment_outline(start, options.outline);
        }
        drawn
    }

    pub(super) fn mark_equipment_outlines(
        &self,
        vertices: &[Vertex],
        start: usize,
        inherited: Option<u32>,
        ranges: &[EquipmentOutlineRange],
    ) {
        // CHECK: Verify glowing replacements receive one outline in both player perspectives.
        visit_equipment_outlines(vertices, start, inherited, ranges, |vertices, rgb| {
            self.mark_outline(vertices, rgb);
        });
    }
}

fn visit_equipment_outlines(
    vertices: &[Vertex],
    start: usize,
    inherited: Option<u32>,
    ranges: &[EquipmentOutlineRange],
    mut emit: impl FnMut(&[Vertex], u32),
) {
    let mut cursor = start;
    for range in ranges {
        let begin = range.start.max(cursor).min(vertices.len());
        let end = range.end.max(begin).min(vertices.len());
        if let Some(rgb) = inherited
            && cursor < begin
        {
            emit(&vertices[cursor..begin], rgb);
        }
        let rgb = match range.policy {
            NativeEquipmentOutline::Inherit => inherited,
            NativeEquipmentOutline::Disabled => None,
            NativeEquipmentOutline::Rgb(rgb) => Some(rgb),
        };
        if let Some(rgb) = rgb
            && begin < end
        {
            emit(&vertices[begin..end], rgb);
        }
        cursor = end;
    }
    if let Some(rgb) = inherited
        && cursor < vertices.len()
    {
        emit(&vertices[cursor..], rgb);
    }
}

/// A humanoid armour model part, in the part's local block coordinates.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[non_exhaustive]
pub enum NativeHumanoidPart {
    Head,
    Body,
    LeftArm,
    RightArm,
    LeftLeg,
    RightLeg,
}

impl NativeHumanoidPart {
    fn name(self) -> &'static str {
        match self {
            Self::Head => "head",
            Self::Body => "body",
            Self::LeftArm => "left_arm",
            Self::RightArm => "right_arm",
            Self::LeftLeg => "left_leg",
            Self::RightLeg => "right_leg",
        }
    }
}

/// An articulated player back layer.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[non_exhaustive]
pub enum NativeBackLayer {
    Wings,
    Cape,
}

/// How an equipment helper chooses the packed overlay code.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[non_exhaustive]
pub enum NativeEquipmentOverlay {
    /// Keep the vanilla equipment resolver's zero or armour-glint code.
    Vanilla,
    /// Draw without an overlay, including without glint.
    None,
}

impl NativeEquipmentOverlay {
    fn code(self, resolved: u32) -> u32 {
        match self {
            Self::Vanilla => resolved,
            Self::None => 0,
        }
    }
}

/// How replacement equipment participates in the entity's outline pass.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[non_exhaustive]
pub enum NativeEquipmentOutline {
    /// Use the entity's ordinary team-coloured glowing outline, if any.
    Inherit,
    /// Omit this equipment from the outline pass.
    Disabled,
    /// Use this RGB colour, even when the entity is not glowing.
    Rgb(u32),
}

/// Lighting, overlay and outline choices for native equipment geometry.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[non_exhaustive]
pub struct NativeEquipmentDrawOptions {
    pub light: u32,
    pub overlay: NativeEquipmentOverlay,
    pub outline: NativeEquipmentOutline,
}

impl NativeEquipmentDrawOptions {
    /// Vanilla-equivalent overlays and inherited outlines at `light`.
    pub const fn new(light: u32) -> Self {
        Self {
            light,
            overlay: NativeEquipmentOverlay::Vanilla,
            outline: NativeEquipmentOutline::Inherit,
        }
    }

    pub const fn with_overlay(mut self, overlay: NativeEquipmentOverlay) -> Self {
        self.overlay = overlay;
        self
    }

    pub const fn with_outline(mut self, outline: NativeEquipmentOutline) -> Self {
        self.outline = outline;
        self
    }
}

#[derive(Clone, Copy)]
pub(super) struct EquipmentOutlineRange {
    pub start: usize,
    pub end: usize,
    pub policy: NativeEquipmentOutline,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn equipment_overlays_preserve_the_resolvers_exclusive_codes() {
        for resolved in [0, render::ARMOR_GLINT] {
            assert_eq!(NativeEquipmentOverlay::Vanilla.code(resolved), resolved);
            assert_eq!(NativeEquipmentOverlay::None.code(resolved), 0);
        }
        let options = NativeEquipmentDrawOptions::new(240)
            .with_overlay(NativeEquipmentOverlay::None)
            .with_outline(NativeEquipmentOutline::Disabled);
        assert_eq!(options.light, 240);
        assert_eq!(options.overlay, NativeEquipmentOverlay::None);
        assert_eq!(options.outline, NativeEquipmentOutline::Disabled);
    }

    #[test]
    fn replacement_equipment_gets_at_most_one_outline() {
        let vertices: Vec<Vertex> = (0..12)
            .map(|index| Vertex {
                pos: [index as f32, 0.0, 0.0],
                ..bytemuck::Zeroable::zeroed()
            })
            .collect();
        let ranges = [
            EquipmentOutlineRange {
                start: 3,
                end: 5,
                policy: NativeEquipmentOutline::Inherit,
            },
            EquipmentOutlineRange {
                start: 5,
                end: 7,
                policy: NativeEquipmentOutline::Rgb(0x12_34_56),
            },
            EquipmentOutlineRange {
                start: 8,
                end: 10,
                policy: NativeEquipmentOutline::Disabled,
            },
        ];
        for inherited in [None, Some(0xab_cd_ef)] {
            let mut colors = [None; 12];
            visit_equipment_outlines(&vertices, 2, inherited, &ranges, |part, rgb| {
                for vertex in part {
                    let color = &mut colors[vertex.pos[0] as usize];
                    assert!(color.is_none(), "a vertex was outlined twice");
                    *color = Some(rgb);
                }
            });
            assert_eq!(
                colors,
                [
                    None,
                    None,
                    inherited,
                    inherited,
                    inherited,
                    Some(0x12_34_56),
                    Some(0x12_34_56),
                    inherited,
                    None,
                    None,
                    inherited,
                    inherited,
                ]
            );
        }
        visit_equipment_outlines(&vertices, 2, None, &[], |_, _| {
            panic!("no outline was requested")
        });
    }
}
