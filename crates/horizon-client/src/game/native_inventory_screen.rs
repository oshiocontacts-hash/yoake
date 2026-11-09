//! Client inventory-screen queries for native mods.

use std::{
    cell::RefCell,
    time::{Duration, Instant},
};

use horizon_client::{
    font::Font,
    hud::HudBatch,
    items::ItemStack,
    mod_api::{self, NativeScreenView},
    text::Text,
};

use super::{Game, Screen};

/// The vanilla menu represented by an inventory screen.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
#[non_exhaustive]
pub enum NativeInventoryContext {
    PlayerInventory,
    CraftingTable,
    Container,
}

/// The logical panel and crafting-slot layout of the player inventory.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct NativeInventoryLayout {
    pub offset: [f32; 2],
    pub width: u32,
    pub show_crafting: bool,
}

impl Default for NativeInventoryLayout {
    fn default() -> Self {
        Self {
            offset: [0.0, 0.0],
            width: 176,
            show_crafting: true,
        }
    }
}

const MAX_TEXT_BYTES: usize = 4096;
const MAX_WIDTH_QUERIES_PER_MOD: usize = 8192;
const MAX_TRACKED_MODS: usize = 256;
const WIDTH_WINDOW: Duration = Duration::from_secs(1);

struct WidthCaller {
    index: usize,
    since: Instant,
    queries: usize,
}

#[derive(Default)]
struct WidthBudget {
    callers: Vec<WidthCaller>,
}

impl WidthBudget {
    fn admit(&mut self, index: usize, now: Instant) -> bool {
        if let Some(caller) = self.callers.iter_mut().find(|caller| caller.index == index) {
            if now.saturating_duration_since(caller.since) >= WIDTH_WINDOW {
                caller.since = now;
                caller.queries = 0;
            }
            if caller.queries >= MAX_WIDTH_QUERIES_PER_MOD {
                return false;
            }
            caller.queries += 1;
            return true;
        }
        self.callers
            .retain(|caller| now.saturating_duration_since(caller.since) < WIDTH_WINDOW);
        if self.callers.len() >= MAX_TRACKED_MODS {
            return false;
        }
        self.callers.push(WidthCaller {
            index,
            since: now,
            queries: 1,
        });
        true
    }
}

thread_local! {
    static WIDTH_BUDGET: RefCell<WidthBudget> = RefCell::new(WidthBudget::default());
}

fn nonempty_carried(stack: Option<&ItemStack>) -> Option<&ItemStack> {
    stack.filter(|stack| stack.count != 0 && &*stack.item != "minecraft:air")
}

fn measured_width(font: &Font, text: &str) -> f32 {
    font.width_text(&Text::literal(text))
}

fn background_if_listened(listened: bool, draw: impl FnOnce() -> bool) -> bool {
    listened && draw()
}

fn layout_if_listened(
    listened: bool,
    update: impl FnOnce(&mut NativeInventoryLayout),
) -> Option<NativeInventoryLayout> {
    if !listened {
        return None;
    }
    let mut layout = NativeInventoryLayout::default();
    update(&mut layout);
    for offset in &mut layout.offset {
        *offset = if offset.is_finite() {
            offset.clamp(-4096.0, 4096.0)
        } else {
            0.0
        };
    }
    layout.width = layout.width.clamp(1, 4096);
    Some(layout)
}

pub(super) fn inventory_origin_for_layout(
    origin: (f32, f32),
    layout: Option<NativeInventoryLayout>,
) -> (f32, f32) {
    match layout {
        Some(layout) => (
            (origin.0 + layout.offset[0]).trunc(),
            (origin.1 + layout.offset[1]).trunc(),
        ),
        None => origin,
    }
}

pub(super) fn dispatch_inventory_screen_layout_listened(game: &Game<'_>, listened: &mut bool) {
    mod_api::dispatch_inventory_screen_layout_listened(game, mod_api::mods(), listened);
}

pub(super) fn dispatch_inventory_screen_layout(
    game: &Game<'_>,
    layout: &mut NativeInventoryLayout,
) {
    mod_api::dispatch_inventory_screen_layout(game, mod_api::mods(), layout);
}

pub(super) fn dispatch_inventory_background_pre_listened(game: &Game<'_>, listened: &mut bool) {
    mod_api::dispatch_inventory_background_pre_listened(game, mod_api::mods(), listened);
}

pub(super) fn dispatch_inventory_background_pre(
    game: &Game<'_>,
    batch: &mut HudBatch,
    screen: &NativeScreenView<'_>,
    canceled: &mut bool,
) {
    mod_api::dispatch_inventory_background_pre(game, mod_api::mods(), batch, screen, canceled);
}

fn inventory_context(
    inventory_screen: bool,
    mod_screen: bool,
    menu: Option<&str>,
    local_container: bool,
    craft_size: usize,
) -> Option<NativeInventoryContext> {
    if !inventory_screen || mod_screen {
        return None;
    }
    if let Some(menu) = menu.filter(|menu| !menu.is_empty()) {
        return Some(match menu {
            "inventory" => NativeInventoryContext::PlayerInventory,
            "minecraft:crafting" => NativeInventoryContext::CraftingTable,
            _ => NativeInventoryContext::Container,
        });
    }
    Some(if local_container {
        NativeInventoryContext::Container
    } else if craft_size == 3 {
        NativeInventoryContext::CraftingTable
    } else {
        NativeInventoryContext::PlayerInventory
    })
}

impl Game<'_> {
    pub(super) fn native_refresh_player_inventory_layout(&mut self) {
        let layout = if self.native_inventory_context_impl(self.screen)
            == Some(NativeInventoryContext::PlayerInventory)
        {
            let mut listened = false;
            self.native_inventory_screen_layout_listened(&mut listened);
            layout_if_listened(listened, |layout| {
                self.native_inventory_screen_layout(layout);
            })
        } else {
            None
        };
        self.recipe_ui.inventory_layout = layout;
    }

    pub(super) fn native_player_inventory_layout(&self) -> Option<NativeInventoryLayout> {
        self.recipe_ui.inventory_layout.filter(|_| {
            self.native_inventory_context_impl(self.screen)
                == Some(NativeInventoryContext::PlayerInventory)
        })
    }

    pub(super) fn native_player_inventory_hide_crafting(&self) -> bool {
        self.native_player_inventory_layout()
            .is_some_and(|layout| !layout.show_crafting)
    }

    pub(super) fn native_player_inventory_background_pre_impl(&self, batch: &mut HudBatch) -> bool {
        let mut listened = false;
        self.native_inventory_background_pre_listened(&mut listened);
        background_if_listened(listened, || {
            let screen = self.screen_view(batch, self.screen);
            let mut canceled = false;
            self.native_inventory_background_pre(batch, &screen, &mut canceled);
            canceled
        })
    }

    pub(super) fn native_inventory_background_hidden(&self) -> bool {
        self.recipe_ui.inventory_background_hidden
            && self.native_inventory_context_impl(self.screen)
                == Some(NativeInventoryContext::PlayerInventory)
    }

    pub(super) fn native_inventory_context_impl(
        &self,
        screen: Screen,
    ) -> Option<NativeInventoryContext> {
        inventory_context(
            screen == Screen::Inventory,
            self.native_screen_shown(),
            self.remote_menu_type(),
            self.container.is_some() || self.station.is_some() || self.mount_active(),
            self.craft_size,
        )
    }

    pub(super) fn native_screen_carried_stack_impl(&self) -> Option<&ItemStack> {
        nonempty_carried(self.carried.as_ref())
    }

    pub(super) fn native_inventory_text_width_impl(&self, text: &str) -> Option<f32> {
        if text.len() > MAX_TEXT_BYTES {
            return None;
        }
        let caller = horizon_client::crash::native_mod_call()?;
        if !WIDTH_BUDGET.with(|budget| budget.borrow_mut().admit(caller.index, Instant::now())) {
            return None;
        }
        Some(measured_width(&self.gui.atlas.font, text))
    }
}

#[cfg(test)]
mod tests {
    use std::{collections::HashMap, path::Path};

    use horizon_client::hud::Rect;

    use super::*;

    #[test]
    fn inventory_context_distinguishes_local_and_remote_screens() {
        use NativeInventoryContext::{Container, CraftingTable, PlayerInventory};

        assert_eq!(
            inventory_context(true, false, None, false, 2),
            Some(PlayerInventory)
        );
        assert_eq!(
            inventory_context(true, false, None, false, 3),
            Some(CraftingTable)
        );
        assert_eq!(
            inventory_context(true, false, None, true, 2),
            Some(Container)
        );
        assert_eq!(
            inventory_context(true, false, Some("inventory"), false, 2),
            Some(PlayerInventory)
        );
        assert_eq!(
            inventory_context(true, false, Some("minecraft:crafting"), false, 2),
            Some(CraftingTable)
        );
        assert_eq!(
            inventory_context(true, false, Some("minecraft:generic_9x3"), false, 2),
            Some(Container)
        );
        assert_eq!(inventory_context(false, false, None, false, 2), None);
        assert_eq!(inventory_context(true, true, None, true, 2), None);
    }

    #[test]
    fn inventory_background_only_dispatches_when_listened() {
        let mut calls = 0;
        let canceled = background_if_listened(false, || {
            calls += 1;
            true
        });
        assert!(!canceled);
        assert_eq!(calls, 0);
        let canceled = background_if_listened(true, || {
            calls += 1;
            true
        });
        assert!(canceled);
        assert_eq!(calls, 1);
        assert!(!background_if_listened(true, || false));
    }

    #[test]
    fn inventory_layout_is_unchanged_without_a_listener() {
        let mut called = false;
        assert_eq!(layout_if_listened(false, |_| called = true), None);
        assert!(!called);
        assert_eq!(
            NativeInventoryLayout::default(),
            NativeInventoryLayout {
                offset: [0.0, 0.0],
                width: 176,
                show_crafting: true,
            }
        );
    }

    #[test]
    fn inventory_layout_dispatches_and_bounds_mod_values() {
        let layout = layout_if_listened(true, |layout| {
            layout.offset = [24.0, -12.0];
            layout.width = 240;
            layout.show_crafting = false;
        })
        .unwrap();
        assert_eq!(layout.offset, [24.0, -12.0]);
        assert_eq!(layout.width, 240);
        assert!(!layout.show_crafting);
        let bounded = layout_if_listened(true, |layout| {
            layout.offset = [f32::NAN, 8000.0];
            layout.width = 0;
        })
        .unwrap();
        assert_eq!(bounded.offset, [0.0, 4096.0]);
        assert_eq!(bounded.width, 1);
    }

    #[test]
    fn inventory_origin_preserves_vanilla_and_aligns_modified_layouts() {
        let origin = (20.5, 30.5);
        assert_eq!(inventory_origin_for_layout(origin, None), origin);
        let layout = NativeInventoryLayout {
            offset: [3.2, -2.2],
            ..NativeInventoryLayout::default()
        };
        assert_eq!(inventory_origin_for_layout(origin, Some(layout)), (23.0, 28.0));
    }

    #[test]
    fn carried_stack_borrows_the_cursor_and_ignores_empty_stacks() {
        let stack = ItemStack::new("minecraft:diamond", 3);
        assert!(nonempty_carried(None).is_none());
        assert!(std::ptr::eq(
            nonempty_carried(Some(&stack)).unwrap(),
            &stack
        ));
        assert!(nonempty_carried(Some(&ItemStack::new("minecraft:air", 1))).is_none());
        assert!(nonempty_carried(Some(&ItemStack::new("minecraft:diamond", 0))).is_none());
    }

    #[test]
    fn width_budget_is_lazy_per_mod_and_resets() {
        let mut budget = WidthBudget::default();
        let now = Instant::now();
        assert!(budget.callers.is_empty());
        for _ in 0..MAX_WIDTH_QUERIES_PER_MOD {
            assert!(budget.admit(1, now));
        }
        assert!(!budget.admit(1, now));
        assert!(budget.admit(2, now));
        assert!(budget.admit(1, now + WIDTH_WINDOW));
    }

    #[test]
    fn width_uses_the_literal_text_font_path() {
        let white = Rect {
            u0: 0.0,
            v0: 0.0,
            u1: 0.0,
            v1: 0.0,
            w: 1,
            h: 1,
        };
        let font = Font::load(
            Path::new("__horizon_missing_font_fixture__"),
            &HashMap::new(),
            &[],
            (4096, 1024),
            (512, 512),
            white,
        );
        for text in [
            "",
            "abc",
            "\u{00a7}lbold",
            "\u{0645}\u{0631}\u{062d}\u{0628}\u{0627}",
        ] {
            assert_eq!(
                measured_width(&font, text),
                font.width_text(&Text::literal(text))
            );
        }
    }
}
