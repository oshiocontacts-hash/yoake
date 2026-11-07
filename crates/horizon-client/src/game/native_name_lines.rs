//! Additional name-tag lines, collected after vanilla visibility and positioned with the name.
use super::{Game, view_vector};
use horizon_client::{
    font::Patch,
    hud::{HudAtlas, HudBatch},
    mesher::Vertex,
    mod_api,
    text::Text,
};

/// One additional line above an entity's name, optionally preceded by a GUI-atlas sprite.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct NameTagLine {
    pub text: String,
    pub sprite: Option<String>,
}

pub(super) fn dispatch_name_tag_lines(
    game: &Game<'_>,
    entity_id: u32,
    kind: &str,
    lines: &mut Vec<NameTagLine>,
) {
    mod_api::dispatch_name_tag_lines(game, mod_api::mods(), entity_id, kind, lines);
}

fn extra_line_lift(base_lift: f64, index: usize, unit: f32) -> f64 {
    base_lift + f64::from(9.0 * 1.15 * unit) * (index + 1) as f64
}

pub(super) fn collect_name_tag_lines(
    visible: bool,
    collect: impl FnOnce(&mut Vec<NameTagLine>),
) -> Vec<NameTagLine> {
    let mut lines = Vec::new();
    if visible {
        collect(&mut lines);
    }
    lines
}

#[allow(clippy::too_many_arguments)]
pub(super) fn name_tag_line_quads_at(
    atlas: &HudAtlas,
    origin: glam::DVec3,
    base_lift: f64,
    lines: &[NameTagLine],
    text_alpha: u32,
    background: bool,
    text_layer: u32,
    light: u32,
    unit: f32,
    out: &mut Vec<Vertex>,
    view: (f32, f32),
) -> Vec<Patch> {
    if lines.is_empty() {
        return Vec::new();
    }
    let forward = glam::DVec3::from_array(view_vector(view.0, view.1));
    let right = forward.cross(glam::DVec3::Y).normalize_or_zero();
    let up = right.cross(forward).normalize_or_zero();
    let away = forward.as_vec3() * 0.002;
    let mut batch = HudBatch::new([1, 1]);
    for (index, line) in lines.iter().enumerate() {
        let text = Text::literal(&line.text).resolve_legacy();
        let sprite = line
            .sprite
            .as_deref()
            .and_then(|name| atlas.sprite_or_missing(name));
        let sprite_width = if sprite.is_some() { 10.0 } else { 0.0 };
        let width = atlas.font.width_text(&text) + sprite_width;
        let left = -width / 2.0;
        batch.vertices.clear();
        let behind = if background {
            batch.fill(atlas, left - 1.0, -1.0, width + 1.0, 10.0, 0x3F00_0000);
            batch.vertices.len()
        } else {
            0
        };
        if let Some(sprite) = sprite {
            batch.rect_uv(left, 0.0, 8.0, 8.0, sprite, text_alpha << 24 | 0xFF_FFFF);
        }
        batch.text_styled(
            atlas,
            &text,
            left + sprite_width,
            0.0,
            text_alpha << 24 | 0xFF_FFFF,
            false,
        );
        let lift = extra_line_lift(base_lift, index, unit);
        let at = (origin + glam::DVec3::new(0.0, lift, 0.0)).as_vec3();
        for (quad_index, quad) in batch.vertices.as_chunks::<6>().0.iter().enumerate() {
            let shift = if quad_index * 6 < behind {
                away
            } else {
                glam::Vec3::ZERO
            };
            for vertex in [quad[5], quad[2], quad[1], quad[0]] {
                out.push(Vertex {
                    normal: [0; 4],
                    pos: (at + right.as_vec3() * (vertex.pos[0] * unit)
                        - up.as_vec3() * (vertex.pos[1] * unit)
                        + shift)
                        .to_array(),
                    uv: vertex.uv,
                    layer: text_layer,
                    color: vertex.color,
                    light,
                    glint_uv: [0.0; 2],
                });
            }
        }
    }
    std::mem::take(&mut batch.patches)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn no_extra_lines_leave_geometry_unchanged() {
        let atlas = HudAtlas::boot();
        let mut vertices = Vec::new();
        let patches = name_tag_line_quads_at(
            &atlas,
            glam::DVec3::ZERO,
            0.0,
            &[],
            255,
            true,
            0,
            240,
            0.025,
            &mut vertices,
            (0.0, 0.0),
        );
        assert!(vertices.is_empty());
        assert!(patches.is_empty());
    }

    #[test]
    fn collected_lines_use_vanilla_spacing_above_existing_lines() {
        let unit = 0.025;
        let spacing = f64::from(9.0_f32 * 1.15 * unit);
        assert_eq!(extra_line_lift(0.5, 0, unit), 0.5 + spacing);
        assert_eq!(extra_line_lift(0.5, 1, unit), 0.5 + spacing * 2.0);
    }

    #[test]
    fn mod_line_draws_in_the_name_tag_pass() {
        let atlas = HudAtlas::boot();
        let mut vertices = Vec::new();
        name_tag_line_quads_at(
            &atlas,
            glam::DVec3::ZERO,
            0.0,
            &[NameTagLine {
                text: "Voice".to_owned(),
                sprite: None,
            }],
            127,
            true,
            2,
            0,
            0.025,
            &mut vertices,
            (0.0, 0.0),
        );
        assert!(!vertices.is_empty());
        assert!(
            vertices
                .iter()
                .all(|vertex| vertex.layer == 2 && vertex.light == 0)
        );
        assert!(vertices.iter().all(|vertex| vertex.pos[1] > 0.0));
    }

    #[test]
    fn hidden_names_never_collect_mod_lines() {
        // CHECK: This gate is also false for crouching and hook-forced hidden names.
        let lines = collect_name_tag_lines(false, |_| panic!("hidden name collected lines"));
        assert!(lines.is_empty());
    }

    #[test]
    fn loaded_mods_without_line_listeners_do_not_collect() {
        struct TestHost;
        impl mod_api::Host for TestHost {}

        struct UnrelatedMod;
        impl mod_api::Hooks for UnrelatedMod {}

        let mods: Vec<Box<dyn mod_api::Hooks>> = vec![Box::new(UnrelatedMod)];
        let mut listened = false;
        mod_api::dispatch_name_tag_lines_listened(&TestHost, &mods, &mut listened);
        assert!(!mods.is_empty());
        assert!(!listened);
        let lines = collect_name_tag_lines(listened, |_| panic!("unlistened name collected lines"));
        assert!(lines.is_empty());
    }

    #[test]
    fn visible_names_collect_once_in_order_for_both_passes() {
        let mut calls = 0;
        let lines = collect_name_tag_lines(true, |lines| {
            calls += 1;
            lines.push(NameTagLine {
                text: "Title".to_owned(),
                sprite: None,
            });
            lines.push(NameTagLine {
                text: "Voice".to_owned(),
                sprite: Some("nestgate:voice".to_owned()),
            });
        });
        assert_eq!(calls, 1);
        assert_eq!(lines[0].text, "Title");
        assert_eq!(lines[1].sprite.as_deref(), Some("nestgate:voice"));
    }
}
