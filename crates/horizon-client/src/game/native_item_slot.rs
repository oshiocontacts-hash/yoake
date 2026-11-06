use super::{Game, HudBatch};
use crate::mod_api;
use horizon_client::items::ItemStack;
use horizon_client_render::{
    hud_layout::{self, GuiDraw, PoseOp},
    hud_overlays::{self, ItemDecorations},
};
use std::sync::OnceLock;

static ITEM_SLOT_COOLDOWN_LISTENED: OnceLock<bool> = OnceLock::new();

fn fan_quads(center: [f32; 2], edge: &[[f32; 2]]) -> impl Iterator<Item = [[f32; 2]; 4]> + '_ {
    edge.windows(2)
        .map(move |pair| [center, pair[0], pair[1], pair[1]])
}

// CHECK: Wire the shared decoration call site and generate both dispatchers before compiling.
#[allow(dead_code)]
impl Game<'_> {
    fn native_item_slot_cooldown_has_listener(&self) -> bool {
        if mod_api::MODS.get().is_none() {
            return false;
        }
        *ITEM_SLOT_COOLDOWN_LISTENED.get_or_init(|| {
            let mut listened = false;
            self.native_item_slot_cooldown_listened(&mut listened);
            listened
        })
    }

    /// Return false without constructing draws or looking up remaining time when unused.
    // CHECK: Exercise scaled, carried and container slots with foil and post-decoration hooks.
    pub(super) fn native_draw_item_slot_cooldown(
        &self,
        batch: &mut HudBatch,
        stack: &ItemStack,
        at: [f32; 2],
        decorations: &ItemDecorations,
        group: &str,
        partial_tick: f32,
    ) -> bool {
        if !self.native_item_slot_cooldown_has_listener() {
            return false;
        }

        let [left, top] = at;
        let mut draws = vec![
            GuiDraw::Push(vec![PoseOp::Translate(left, top)]),
            GuiDraw::Push(Vec::new()),
        ];
        hud_overlays::item_decoration_bar(&mut draws, 0, 0, decorations);
        draws.extend([GuiDraw::Pop, GuiDraw::Pop]);
        hud_layout::paint(batch, &self.gui.atlas, &draws);

        let cooldown = self
            .cooldowns
            .remaining(group, partial_tick)
            .map(|(remaining, _)| (decorations.cooldown, remaining));
        let mut canceled = false;
        self.native_item_slot_cooldown(
            batch,
            stack,
            [left, top, 16.0, 16.0],
            cooldown,
            partial_tick,
            &mut canceled,
        );

        draws.clear();
        draws.extend([
            GuiDraw::Push(vec![PoseOp::Translate(left, top)]),
            GuiDraw::Push(Vec::new()),
        ]);
        hud_overlays::item_decorations_after_bar(
            &mut draws,
            0,
            0,
            decorations,
            &self.gui.atlas.font,
            !canceled,
        );
        draws.extend([GuiDraw::Pop, GuiDraw::Pop]);
        hud_layout::paint(batch, &self.gui.atlas, &draws);
        true
    }

    /// Draw one triangle per adjacent edge pair using existing six-vertex quads.
    /// Repeat the first edge point at the end to close a full fan. @API:gui_fan@.
    pub(super) fn native_fan(
        &self,
        batch: &mut HudBatch,
        center: [f32; 2],
        edge: &[[f32; 2]],
        argb: u32,
    ) {
        for points in fan_quads(center, edge) {
            self.native_quad(batch, points, argb);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn fan_triangles_cover_each_quadrant() {
        let center = [5.0, 7.0];
        let edge = [[8.0, 7.0], [5.0, 10.0], [2.0, 7.0], [5.0, 4.0], [8.0, 7.0]];
        let quads: Vec<_> = fan_quads(center, &edge).collect();
        assert_eq!(quads.len(), 4);
        for (quadrant, points) in quads.iter().enumerate() {
            let expected = [
                center,
                edge[quadrant],
                edge[quadrant + 1],
                edge[quadrant + 1],
            ];
            assert_eq!(*points, expected);
            let first = [points[1][0] - center[0], points[1][1] - center[1]];
            let second = [points[2][0] - center[0], points[2][1] - center[1]];
            assert_eq!(first[0] * second[1] - first[1] * second[0], 9.0);
            assert_eq!(points[2], points[3]);
        }
    }

    #[test]
    fn fan_needs_two_edge_points_and_does_not_close_implicitly() {
        let center = [0.0, 0.0];
        assert_eq!(fan_quads(center, &[]).count(), 0);
        assert_eq!(fan_quads(center, &[[1.0, 0.0]]).count(), 0);
        let edge = [[1.0, 0.0], [0.0, 1.0], [-1.0, 0.0]];
        assert_eq!(fan_quads(center, &edge).count(), 2);
    }
}
