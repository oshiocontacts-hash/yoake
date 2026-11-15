use horizon_client::{
    containers::{BlockEntity, FurnaceKind},
    hud::HudBatch,
    mod_api::NativeInventoryContext,
};

use super::{Game, Screen, stations::Kind};

fn station_kind(kind: Kind) -> &'static str {
    match kind {
        Kind::Enchanting => "minecraft:enchantment",
        Kind::Anvil => "minecraft:anvil",
        Kind::Grindstone => "minecraft:grindstone",
        Kind::Smithing => "minecraft:smithing",
        Kind::Stonecutter => "minecraft:stonecutter",
        Kind::Merchant => "minecraft:merchant",
        Kind::Beacon => "minecraft:beacon",
        Kind::Cartography => "minecraft:cartography_table",
        Kind::Loom => "minecraft:loom",
    }
}

fn container_kind(entity: &BlockEntity) -> Option<&'static str> {
    match entity {
        BlockEntity::Furnace(furnace) => Some(match furnace.kind {
            FurnaceKind::Furnace => "minecraft:furnace",
            FurnaceKind::BlastFurnace => "minecraft:blast_furnace",
            FurnaceKind::Smoker => "minecraft:smoker",
        }),
        BlockEntity::Brewing(_) => Some("minecraft:brewing_stand"),
        BlockEntity::Crafter(_) => Some("minecraft:crafter_3x3"),
        _ => None,
    }
}

fn chest_kind(rows: usize) -> Option<&'static str> {
    match rows {
        1 => Some("minecraft:generic_9x1"),
        2 => Some("minecraft:generic_9x2"),
        3 => Some("minecraft:generic_9x3"),
        4 => Some("minecraft:generic_9x4"),
        5 => Some("minecraft:generic_9x5"),
        6 => Some("minecraft:generic_9x6"),
        _ => None,
    }
}

fn screen_kind<'a>(
    id: &'static str,
    menu: Option<&'a str>,
    mod_screen: bool,
    local_kind: Option<&'static str>,
    context: Option<NativeInventoryContext>,
) -> Option<&'a str> {
    if id != "inventory" {
        return (id != "mod").then_some(id);
    }
    if mod_screen {
        return None;
    }
    if let Some(menu) = menu.filter(|menu| !menu.is_empty()) {
        return (menu == "inventory" || menu.starts_with("minecraft:")).then_some(menu);
    }
    local_kind.or(match context {
        Some(NativeInventoryContext::PlayerInventory) => Some("inventory"),
        Some(NativeInventoryContext::CraftingTable) => Some("minecraft:crafting"),
        _ => None,
    })
}

fn slot_rectangles(
    slots: impl IntoIterator<Item = usize>,
    origin: (f32, f32),
    covered: bool,
    mut position: impl FnMut(usize) -> (f32, f32),
) -> Vec<(usize, [f32; 4])> {
    if covered {
        return Vec::new();
    }
    slots
        .into_iter()
        .map(|slot| {
            let (x, y) = position(slot);
            (slot, [origin.0 + x, origin.1 + y, 16.0, 16.0])
        })
        .collect()
}

#[allow(dead_code)]
impl Game<'_> {
    pub(super) fn native_screen_kind(
        &self,
        screen: Screen,
        context: Option<NativeInventoryContext>,
    ) -> Option<&str> {
        let is_inventory = screen == Screen::Inventory;
        let local_kind = is_inventory
            .then(|| {
                self.station
                    .as_ref()
                    .map(|station| station_kind(station.kind))
                    .or_else(|| {
                        self.container_kind().and_then(|entity| {
                            if matches!(entity, BlockEntity::Chest(_)) {
                                self.machine_screen_menu_kind()
                                    .or_else(|| {
                                        self.container_block_is_shulker()
                                            .then_some("minecraft:shulker_box")
                                    })
                                    .or_else(|| self.chest_rows().and_then(chest_kind))
                            } else {
                                container_kind(entity)
                            }
                        })
                    })
            })
            .flatten();
        screen_kind(
            screen.id(),
            is_inventory.then(|| self.remote_menu_type()).flatten(),
            is_inventory && self.native_screen_shown(),
            local_kind,
            context,
        )
    }

    /// Returns the visible inventory screen's slot IDs and icon rectangles in
    /// GUI pixels, or None if no known vanilla slot screen is visible.
    /// Slot IDs are the IDs used by the game's inventory interaction helpers.
    /// API @API:screen_slot_rects@.
    pub(super) fn native_screen_slot_rects(&self) -> Option<Vec<(usize, [f32; 4])>> {
        let view = self.observed_screen_view()?;
        if view.kind.is_none()
            || !matches!(view.id, "inventory" | "creative")
            || !matches!(self.screen, Screen::Inventory | Screen::Creative)
        {
            return None;
        }
        let renderer = self.renderer.as_ref()?;
        let batch = HudBatch::new([renderer.config.width, renderer.config.height]);
        Some(slot_rectangles(
            self.slots(),
            self.inventory_origin(&batch),
            self.book_covers_menu(batch.width),
            |slot| self.slot_position(slot),
        ))
    }
}

#[cfg(test)]
mod tests {
    use super::{chest_kind, screen_kind, slot_rectangles, station_kind};
    use crate::game::{native_inventory_screen::NativeInventoryContext, stations::Kind};

    #[test]
    fn screen_kinds_distinguish_vanilla_menus_and_local_stations() {
        assert_eq!(
            screen_kind("inventory", Some("minecraft:generic_9x3"), false, None, None),
            Some("minecraft:generic_9x3")
        );
        assert_eq!(
            screen_kind(
                "inventory",
                None,
                false,
                Some(station_kind(Kind::Anvil)),
                None,
            ),
            Some("minecraft:anvil")
        );
        assert_eq!(
            screen_kind(
                "inventory",
                None,
                false,
                None,
                Some(NativeInventoryContext::PlayerInventory),
            ),
            Some("inventory")
        );
        assert_eq!(
            screen_kind(
                "inventory",
                None,
                false,
                None,
                Some(NativeInventoryContext::CraftingTable),
            ),
            Some("minecraft:crafting")
        );
    }

    #[test]
    fn chest_rows_identify_supported_vanilla_menus() {
        assert_eq!(chest_kind(1), Some("minecraft:generic_9x1"));
        assert_eq!(chest_kind(3), Some("minecraft:generic_9x3"));
        assert_eq!(chest_kind(6), Some("minecraft:generic_9x6"));
        assert_eq!(chest_kind(0), None);
        assert_eq!(chest_kind(7), None);
    }

    #[test]
    fn ambiguous_or_mod_screens_do_not_claim_a_vanilla_kind() {
        assert_eq!(
            screen_kind("inventory", None, false, None, Some(NativeInventoryContext::Container)),
            None
        );
        assert_eq!(
            screen_kind("inventory", Some("example:menu"), false, None, None),
            None
        );
        assert_eq!(
            screen_kind("inventory", Some("minecraft:furnace"), true, None, None),
            None
        );
        assert_eq!(
            screen_kind("creative", None, false, None, None),
            Some("creative")
        );
    }

    #[test]
    fn keeps_slot_ids_and_offsets_rectangles_by_the_screen_origin() {
        let rectangles = slot_rectangles([300, 0, 200], (100.0, 40.0), false, |slot| match slot {
            300 => (8.0, 18.0),
            0 => (8.0, 84.0),
            200 => (124.0, 35.0),
            _ => unreachable!(),
        });
        assert_eq!(
            rectangles,
            vec![
                (300, [108.0, 58.0, 16.0, 16.0]),
                (0, [108.0, 124.0, 16.0, 16.0]),
                (200, [224.0, 75.0, 16.0, 16.0]),
            ]
        );
    }

    #[test]
    fn empty_screens_have_no_slot_rectangles() {
        assert!(slot_rectangles([], (2.0, 3.0), false, |_| unreachable!()).is_empty());
        assert!(slot_rectangles([1], (2.0, 3.0), true, |_| unreachable!()).is_empty());
    }
}
