use std::cell::Cell;

use horizon_client::{hud::HudBatch, items::ItemStack};

use super::Game;

thread_local! {
    static DRAWING_GUI_ITEM: Cell<bool> = const { Cell::new(false) };
}

struct ItemDrawGuard;

impl ItemDrawGuard {
    fn enter() -> Option<Self> {
        DRAWING_GUI_ITEM.with(|drawing| {
            if drawing.replace(true) {
                None
            } else {
                Some(Self)
            }
        })
    }
}

impl Drop for ItemDrawGuard {
    fn drop(&mut self) {
        DRAWING_GUI_ITEM.with(|drawing| drawing.set(false));
    }
}

fn valid_item_draw(stack: &ItemStack, known: bool, at: [f32; 2], scale: f32) -> bool {
    stack.item.as_ref() != "minecraft:air"
        && stack.count != 0
        && known
        && at.into_iter().all(|coordinate| coordinate.is_finite() && coordinate.abs() <= 1_000_000.0)
        && scale.is_finite()
        && scale > 0.0
        && scale <= 64.0
}

fn draw_scaled(batch: &mut HudBatch, at: [f32; 2], scale: f32, draw: impl FnOnce(&mut HudBatch)) {
    let start = batch.vertices.len();
    draw(batch);
    batch.scale_since(start, at[0], at[1], scale, scale);
}

#[allow(dead_code)]
impl Game<'_> {
    /// Draw a component-bearing stack and its decorations, including its count,
    /// into the current HUD or screen batch. Scale 1 is a 16-pixel icon.
    /// Returns false for an empty or unknown stack or invalid geometry.
    /// API @API:gui_item@.
    pub(super) fn native_draw_gui_item(
        &self,
        batch: &mut HudBatch,
        stack: &ItemStack,
        at: [f32; 2],
        scale: f32,
    ) -> bool {
        if !valid_item_draw(stack, self.items().get(&stack.item).is_some(), at, scale) {
            return false;
        }
        let Some(_guard) = ItemDrawGuard::enter() else {
            return false;
        };
        draw_scaled(batch, at, scale, |batch| {
            self.draw_item(batch, stack, at[0], at[1]);
        });
        true
    }
}

#[cfg(test)]
mod tests {
    use horizon_client::{hud::HudBatch, items::ItemStack};

    use super::{ItemDrawGuard, draw_scaled, valid_item_draw};

    #[test]
    fn rejects_nested_item_draws_and_releases_the_guard() {
        let guard = ItemDrawGuard::enter().expect("first item draw");
        assert!(ItemDrawGuard::enter().is_none());
        assert!(ItemDrawGuard::enter().is_none());
        drop(guard);
        assert!(ItemDrawGuard::enter().is_some());
    }

    #[test]
    fn rejects_empty_unknown_or_invalid_draws() {
        let stack = ItemStack::new("minecraft:stone", 2);
        assert!(valid_item_draw(&stack, true, [12.0, 34.0], 1.0));
        assert!(!valid_item_draw(&stack, false, [12.0, 34.0], 1.0));
        assert!(!valid_item_draw(&ItemStack::new("minecraft:air", 1), true, [0.0, 0.0], 1.0));
        assert!(!valid_item_draw(&ItemStack::new("minecraft:stone", 0), true, [0.0, 0.0], 1.0));
        assert!(!valid_item_draw(&stack, true, [f32::NAN, 0.0], 1.0));
        assert!(!valid_item_draw(&stack, true, [1_000_001.0, 0.0], 1.0));
        assert!(!valid_item_draw(&stack, true, [0.0, 0.0], f32::INFINITY));
        assert!(!valid_item_draw(&stack, true, [0.0, 0.0], 0.0));
        assert!(!valid_item_draw(&stack, true, [0.0, 0.0], 65.0));
    }

    #[test]
    fn scales_only_new_vertices_about_the_icon_origin() {
        let mut batch = HudBatch::new([400, 300]);
        let uv = [[0.0, 0.0]; 4];
        batch.quad([[1.0, 2.0]; 4], uv, u32::MAX);
        draw_scaled(&mut batch, [10.0, 20.0], 2.0, |batch| {
            batch.quad(
                [[10.0, 20.0], [26.0, 20.0], [26.0, 36.0], [10.0, 36.0]],
                uv,
                u32::MAX,
            );
        });
        assert_eq!(batch.vertices[0].pos, [1.0, 2.0]);
        assert_eq!(batch.vertices[6].pos, [10.0, 20.0]);
        assert_eq!(batch.vertices[7].pos, [42.0, 20.0]);
        assert_eq!(batch.vertices[8].pos, [42.0, 52.0]);
    }
}
