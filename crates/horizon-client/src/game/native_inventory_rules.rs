//! Server-side placement rules for the player inventory crafting grid.

use horizon_client::{items::ItemStack, mod_api};

use super::Game;

pub(super) fn dispatch_inventory_slot_rules_listened(game: &Game<'_>, listened: &mut bool) {
    mod_api::dispatch_inventory_slot_rules_listened(game, mod_api::mods(), listened);
}

pub(super) fn dispatch_inventory_slot_rules(
    game: &Game<'_>,
    player: u128,
    slot: usize,
    stack: &ItemStack,
    canceled: &mut bool,
) {
    mod_api::dispatch_inventory_slot_rules(game, mod_api::mods(), player, slot, stack, canceled);
}

fn remainder_allowed(listened: bool, check: impl FnOnce() -> bool) -> bool {
    !listened || check()
}

impl Game<'_> {
    fn native_inventory_rule_applies(&self) -> bool {
        self.remote.is_none()
            && self.craft_size == 2
            && self.container.is_none()
            && self.station.is_none()
            && !self.mount_active()
    }

    fn native_inventory_rule_allows(&mut self, slot: usize, stack: &ItemStack) -> bool {
        let player = self.native_acting_id();
        let mut canceled = false;
        self.native_inventory_slot_rules(player, slot, stack, &mut canceled);
        !canceled
    }

    pub(super) fn native_inventory_carried_allowed(&mut self, slot: usize) -> bool {
        if !self.native_inventory_rule_applies() || self.carried.is_none() {
            return true;
        }
        let mut listened = false;
        self.native_inventory_slot_rules_listened(&mut listened);
        if !listened {
            return true;
        }
        let Some(stack) = self.carried.clone() else {
            return true;
        };
        self.native_inventory_rule_allows(slot, &stack)
    }

    pub(super) fn native_inventory_remainder_allowed(
        &mut self,
        slot: usize,
        stack: &ItemStack,
    ) -> bool {
        if !self.native_inventory_rule_applies() {
            return true;
        }
        let mut listened = false;
        self.native_inventory_slot_rules_listened(&mut listened);
        remainder_allowed(listened, || self.native_inventory_rule_allows(slot, stack))
    }
}

#[cfg(test)]
mod tests {
    use super::remainder_allowed;

    #[test]
    fn remainder_gate_preserves_vanilla_without_a_listener() {
        assert!(remainder_allowed(false, || panic!("unexpected rule call")));
    }

    #[test]
    fn remainder_gate_respects_the_listening_rule() {
        assert!(!remainder_allowed(true, || false));
        assert!(remainder_allowed(true, || true));
    }
}
