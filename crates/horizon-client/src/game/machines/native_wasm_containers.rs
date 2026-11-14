//! Whole containers for the WebAssembly services of `server-full` (fragment `wasm_services`): read
//! every slot of the container at a position (both halves of a double chest), replace the slots
//! with the same items in another arrangement, and find the containers near a point. Like the
//! hopper helpers they act where the world is simulated, on the level in place.
use super::{Game, Kind, View};
use crate::game::native_wasm_services::{conserved, same_kind};
use horizon_client::items::ItemStack;

/// Most containers one `native_containers_in` returns.
const MAX_CONTAINERS: usize = 64;
/// Farthest `native_containers_in` reaches, in blocks.
const MAX_RADIUS: f64 = 64.0;

fn kind_name(kind: Kind) -> &'static str {
    match kind {
        Kind::Generic => "generic",
        Kind::Furnace => "furnace",
        Kind::Hopper => "hopper",
        Kind::Crafter => "crafter",
        Kind::Brewing => "brewing_stand",
        Kind::Bookshelf => "chiseled_bookshelf",
        Kind::Jukebox => "jukebox",
        Kind::Mod => "mod",
        Kind::Shulker => "shulker_box",
        Kind::Composter(_) => "composter",
    }
}

/// Containers whose slots follow no special rule, so a plain rearrangement is always legal.
fn rearrangeable(kind: Kind) -> bool {
    matches!(kind, Kind::Generic | Kind::Hopper | Kind::Shulker | Kind::Mod)
}

// Native mods use these; nothing in Horizon does.
#[allow(dead_code)]
impl Game<'_> {
    fn wasm_container(&self, dimension: &str, pos: [i32; 3]) -> Result<View, String> {
        if !self.native_simulates() || dimension != self.dim.id() {
            return Err(format!("{dimension} is not the level being ticked"));
        }
        if !self.world.loaded(pos[0], pos[2]) {
            return Err("the position is not loaded".into());
        }
        let view = self
            .containers()
            .view((pos[0], pos[1], pos[2]), false)
            .ok_or("there is no container there")?;
        if matches!(view.kind, Kind::Composter(_)) || view.len() == 0 {
            return Err("there is no container there".into());
        }
        Ok(view)
    }

    /// Every slot of the container at `pos` in the level in place, both halves of a double chest
    /// (first half first). Err where no container is.
    pub(in crate::game) fn native_container_all_slots(
        &self,
        dimension: &str,
        pos: [i32; 3],
    ) -> Result<Vec<Option<ItemStack>>, String> {
        let view = self.wasm_container(dimension, pos)?;
        let c = self.containers();
        Ok((0..view.len()).map(|slot| c.get(&view, slot).cloned()).collect())
    }

    /// Replaces every slot of the container at `pos` with `slots` (as long as the container),
    /// which must hold exactly the items it holds now, in any arrangement, no stack past its
    /// item's limit and no shulker box inside a shulker box. Plain containers only (chests,
    /// barrels, hoppers, shulker boxes and mod containers): a furnace or a brewing stand has slot
    /// rules this does not know.
    pub(in crate::game) fn native_container_replace_slots(
        &mut self,
        dimension: &str,
        pos: [i32; 3],
        slots: Vec<Option<ItemStack>>,
    ) -> Result<(), String> {
        let view = self.wasm_container(dimension, pos)?;
        if !rearrangeable(view.kind) {
            return Err(format!(
                "a {} has slot rules: only plain containers can be rearranged",
                kind_name(view.kind)
            ));
        }
        if slots.len() != view.len() {
            return Err(format!("the container has {} slots, got {}", view.len(), slots.len()));
        }
        let old: Vec<Option<ItemStack>> = {
            let c = self.containers();
            (0..view.len()).map(|slot| c.get(&view, slot).cloned()).collect()
        };
        if !conserved(&old, &slots) {
            return Err("the new slots do not hold exactly the same items".into());
        }
        let items = self.res.items();
        for stack in slots.iter().flatten() {
            let max = items.get(&stack.item).map(|info| info.max_stack.max(1));
            if stack.count == 0 || max.is_none_or(|max| stack.count > max) {
                return Err(format!("{} x{} is not a valid stack", stack.item, stack.count));
            }
            if view.kind == Kind::Shulker && crate::game::storage::is_shulker_box(&stack.item) {
                return Err("a shulker box does not go into a shulker box".into());
            }
        }
        for (slot, stack) in slots.into_iter().enumerate() {
            match self.slot_mut_at(&view, slot) {
                Some(target) => *target = stack,
                None => return Err("a slot vanished while writing (nothing more written)".into()),
            }
        }
        self.view_changed(&view);
        Ok(())
    }

    /// The containers within `radius` (at most 64) of `center` in the level in place, nearest
    /// first, at most 64: a double chest once, at its first half. Each is `(position, kind, slots,
    /// block)`.
    pub(in crate::game) fn native_containers_near(
        &self,
        dimension: &str,
        center: [f64; 3],
        radius: f64,
    ) -> Result<Vec<([i32; 3], String, u32, String)>, String> {
        if !self.native_simulates() || dimension != self.dim.id() {
            return Err(format!("{dimension} is not the level being ticked"));
        }
        if !center.iter().all(|c| c.is_finite()) || !radius.is_finite() || radius < 0.0 {
            return Err("the centre and the radius must be finite".into());
        }
        let radius = radius.min(MAX_RADIUS);
        let mut hits: Vec<(f64, [i32; 3])> = Vec::new();
        for (&(x, y, z), _) in self.world.block_entities.iter() {
            let at = [f64::from(x) + 0.5, f64::from(y) + 0.5, f64::from(z) + 0.5];
            let distance = (0..3).map(|i| (at[i] - center[i]).powi(2)).sum::<f64>().sqrt();
            if distance <= radius {
                hits.push((distance, [x, y, z]));
            }
        }
        hits.sort_by(|a, b| a.0.total_cmp(&b.0).then(a.1.cmp(&b.1)));
        let mut seen: Vec<(i32, i32, i32)> = Vec::new();
        let mut out = Vec::new();
        for (_, pos) in hits {
            if out.len() >= MAX_CONTAINERS {
                break;
            }
            if seen.contains(&(pos[0], pos[1], pos[2])) {
                continue;
            }
            let Some(view) = self.containers().view((pos[0], pos[1], pos[2]), false) else {
                continue;
            };
            if matches!(view.kind, Kind::Composter(_)) || view.len() == 0 {
                continue;
            }
            seen.extend(view.parts.iter().map(|part| part.0));
            let first = view.parts[0].0;
            let state = self.world.block(first.0, first.1, first.2);
            out.push((
                [first.0, first.1, first.2],
                kind_name(view.kind).to_owned(),
                view.len() as u32,
                self.res.facts().get(state).name.to_string(),
            ));
        }
        Ok(out)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn only_plain_containers_are_rearranged() {
        assert!(rearrangeable(Kind::Generic));
        assert!(rearrangeable(Kind::Shulker));
        assert!(!rearrangeable(Kind::Furnace));
        assert!(!rearrangeable(Kind::Brewing));
        assert!(!rearrangeable(Kind::Composter(3)));
    }

    #[test]
    fn kinds_have_names() {
        assert_eq!(kind_name(Kind::Generic), "generic");
        assert_eq!(kind_name(Kind::Mod), "mod");
    }

    #[test]
    fn rearranging_checks_the_kind_of_each_item() {
        let a = ItemStack { item: "minecraft:stone".into(), count: 1, damage: 0, extra: None };
        let mut b = a.clone();
        b.damage = 3;
        assert!(same_kind(&a, &a.clone()));
        assert!(!same_kind(&a, &b));
    }
}
