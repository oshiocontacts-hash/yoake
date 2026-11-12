//! Mod-owned inventory storage and the bounded menu opening payload.
use crate::{
    items::ItemStack,
    menu::{Env, Menu},
};

pub const MENU_CHANNEL: &str = "horizon:menu_open";
pub const MAX_MENU_EXTRA: usize = 65_536;

/// Interior-mutability storage. Simulation must not change any slot or component.
pub trait ItemHandler: Send + Sync {
    fn slots(&self) -> usize;
    fn get_stack(&self, slot: usize) -> Option<ItemStack>;
    fn set_stack(&self, slot: usize, stack: Option<ItemStack>);
    fn stack_limit(&self, stack: &ItemStack) -> u32;
    fn slot_limit(&self, _slot: usize) -> u32 {
        99
    }
    fn may_place(&self, slot: usize, _stack: &ItemStack) -> bool {
        slot < self.slots()
    }

    /// The unaccepted remainder. Limits include the item's own maximum stack size.
    fn insert(&self, slot: usize, stack: ItemStack, simulate: bool) -> Option<ItemStack> {
        if slot >= self.slots() || !self.may_place(slot, &stack) {
            return Some(stack);
        }
        let current = self.get_stack(slot);
        if current
            .as_ref()
            .is_some_and(|held| !held.same_item_same_components(&stack))
        {
            return Some(stack);
        }
        let count = current.as_ref().map_or(0, |held| held.count);
        let limit = self.slot_limit(slot).min(self.stack_limit(&stack));
        let moved = stack.count.min(limit.saturating_sub(count));
        if moved > 0 && !simulate {
            self.set_stack(
                slot,
                Some(ItemStack {
                    count: count + moved,
                    ..stack.clone()
                }),
            );
        }
        (moved < stack.count).then(|| ItemStack {
            count: stack.count - moved,
            ..stack
        })
    }

    /// The removed stack, never more than `count`.
    fn extract(&self, slot: usize, count: u32, simulate: bool) -> Option<ItemStack> {
        if slot >= self.slots() {
            return None;
        }
        let mut stack = self.get_stack(slot)?;
        let moved = count.min(stack.count);
        if moved == 0 {
            return None;
        }
        if !simulate {
            self.set_stack(
                slot,
                (moved < stack.count).then(|| ItemStack {
                    count: stack.count - moved,
                    ..stack.clone()
                }),
            );
        }
        stack.count = moved;
        Some(stack)
    }
}

pub fn snapshot(handler: &dyn ItemHandler) -> Vec<Option<ItemStack>> {
    (0..handler.slots())
        .map(|slot| handler.get_stack(slot))
        .collect()
}

pub fn load(menu: &mut Menu, env: &Env, handler: &dyn ItemHandler) {
    for slot in 0..handler.slots() {
        menu.set_slot(env, slot, handler.get_stack(slot));
    }
}

/// Apply a menu transaction through the handler, restoring storage on rejection.
pub fn store(menu: &Menu, handler: &dyn ItemHandler) -> bool {
    let desired = (0..handler.slots())
        .map(|slot| menu.slot(slot).cloned())
        .collect::<Vec<_>>();
    replace(handler, &desired)
}

/// Handler operations must honor simulation and serialize access during a click.
pub fn replace(handler: &dyn ItemHandler, desired: &[Option<ItemStack>]) -> bool {
    if desired.len() != handler.slots() {
        return false;
    }
    let before = snapshot(handler);
    let accepted = before
        .iter()
        .zip(desired)
        .enumerate()
        .all(|(slot, (old, new))| {
            if old == new {
                return true;
            }
            let same = old
                .as_ref()
                .zip(new.as_ref())
                .is_some_and(|(old, new)| old.same_item_same_components(new));
            let removed = old.as_ref().map_or(0, |stack| {
                if same {
                    stack.count.saturating_sub(new.as_ref().unwrap().count)
                } else {
                    stack.count
                }
            });
            if removed > 0 {
                let expected = ItemStack {
                    count: removed,
                    ..old.as_ref().unwrap().clone()
                };
                if handler.extract(slot, removed, true).as_ref() != Some(&expected)
                    || handler.extract(slot, removed, false).as_ref() != Some(&expected)
                {
                    return false;
                }
            }
            let inserted = new.as_ref().map_or(0, |stack| {
                if same {
                    stack.count.saturating_sub(old.as_ref().unwrap().count)
                } else {
                    stack.count
                }
            });
            if inserted > 0 {
                let stack = ItemStack {
                    count: inserted,
                    ..new.as_ref().unwrap().clone()
                };
                if handler.insert(slot, stack.clone(), true).is_some()
                    || handler.insert(slot, stack, false).is_some()
                {
                    return false;
                }
            }
            handler.get_stack(slot) == *new
        });
    if accepted && snapshot(handler) == desired {
        return true;
    }
    for (slot, stack) in before.into_iter().enumerate() {
        if handler.get_stack(slot) != stack {
            handler.set_stack(slot, stack);
        }
    }
    false
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct MenuOpening {
    pub window: i32,
    pub menu_type: String,
    pub extra: Vec<u8>,
}

impl MenuOpening {
    pub fn encode(&self) -> Option<Vec<u8>> {
        let name = self.menu_type.as_bytes();
        if self.window <= 0 || name.is_empty() || name.len() > 256 || self.extra.len() > MAX_MENU_EXTRA {
            return None;
        }
        let mut bytes = Vec::with_capacity(10 + name.len() + self.extra.len());
        bytes.extend_from_slice(&self.window.to_be_bytes());
        bytes.extend_from_slice(&(name.len() as u16).to_be_bytes());
        bytes.extend_from_slice(name);
        bytes.extend_from_slice(&(self.extra.len() as u32).to_be_bytes());
        bytes.extend_from_slice(&self.extra);
        Some(bytes)
    }

    pub fn decode(bytes: &[u8]) -> Option<Self> {
        if bytes.len() < 10 || bytes.len() > MAX_MENU_EXTRA + 266 {
            return None;
        }
        let window = i32::from_be_bytes(bytes.get(..4)?.try_into().ok()?);
        let length = usize::from(u16::from_be_bytes(bytes.get(4..6)?.try_into().ok()?));
        if window <= 0 || length == 0 || length > 256 {
            return None;
        }
        let menu_type = std::str::from_utf8(bytes.get(6..6 + length)?).ok()?;
        let offset = 6 + length;
        let extra_length = u32::from_be_bytes(bytes.get(offset..offset + 4)?.try_into().ok()?) as usize;
        let extra = bytes.get(offset + 4..)?;
        if extra_length > MAX_MENU_EXTRA || extra_length != extra.len() {
            return None;
        }
        Some(Self {
            window,
            menu_type: menu_type.into(),
            extra: extra.to_vec(),
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{
        containers::ModMenuType,
        menu::{Input, Kind},
    };
    use std::sync::{Arc, Mutex};

    struct Backpack(Mutex<ItemStack>);

    impl ItemHandler for Backpack {
        fn slots(&self) -> usize {
            9
        }
        fn get_stack(&self, slot: usize) -> Option<ItemStack> {
            self.0.lock().unwrap().data().and_then(|data| {
                data.container
                    .iter()
                    .find(|(index, _)| usize::from(*index) == slot)
                    .map(|(_, stack)| stack.clone())
            })
        }
        fn set_stack(&self, slot: usize, stack: Option<ItemStack>) {
            let mut backpack = self.0.lock().unwrap();
            let data = backpack.data_mut();
            data.container.retain(|(index, _)| usize::from(*index) != slot);
            if let Some(stack) = stack {
                data.container.push((slot as u8, stack));
            }
            data.wire = Default::default();
            backpack.tidy();
        }
        fn slot_limit(&self, _slot: usize) -> u32 {
            16
        }
        fn stack_limit(&self, _stack: &ItemStack) -> u32 {
            64
        }
    }

    struct RejectCommit(Backpack);

    impl ItemHandler for RejectCommit {
        fn slots(&self) -> usize {
            self.0.slots()
        }
        fn get_stack(&self, slot: usize) -> Option<ItemStack> {
            self.0.get_stack(slot)
        }
        fn set_stack(&self, slot: usize, stack: Option<ItemStack>) {
            self.0.set_stack(slot, stack);
        }
        fn stack_limit(&self, stack: &ItemStack) -> u32 {
            self.0.stack_limit(stack)
        }
        fn slot_limit(&self, slot: usize) -> u32 {
            self.0.slot_limit(slot)
        }
        fn insert(&self, slot: usize, stack: ItemStack, simulate: bool) -> Option<ItemStack> {
            if slot == 1 && !simulate {
                Some(stack)
            } else {
                self.0.insert(slot, stack, simulate)
            }
        }
    }

    #[test]
    fn backpack_open_predict_move_close_and_reopen_preserves_components() {
        crate::menu::tests::with_empty_inventory_env(|env| {
            let kind = Box::leak(Box::new(ModMenuType {
                id: "test:backpack",
                slots: 9,
                data_slots: 0,
                results: Vec::new(),
            }));
            let opening = MenuOpening {
                window: 1,
                menu_type: kind.id.into(),
                extra: vec![0, 3, 255, 9],
            };
            let received = MenuOpening::decode(&opening.encode().unwrap()).unwrap();
            assert_eq!(received, opening);
            let server = Arc::new(Backpack(Mutex::new(ItemStack::new("test:backpack", 1))));
            let client = Arc::new(Backpack(Mutex::new(ItemStack::new("test:backpack", 1))));
            let mut authoritative = Menu::new(Kind::Mod(kind));
            authoritative.item_handler = Some(server.clone());
            let mut prediction = Menu::new(Kind::Mod(kind));
            prediction.item_handler = Some(client.clone());
            for menu in [&mut authoritative, &mut prediction] {
                menu.set_player_item(0, Some(ItemStack::new("minecraft:stone", 20)));
                menu.click(env, 36, 0, Input::Pickup).unwrap();
                menu.click(env, 0, 0, Input::Pickup).unwrap();
                assert_eq!(menu.carried.as_ref().unwrap().count, 4);
                menu.click(env, 36, 0, Input::Pickup).unwrap();
            }
            assert_eq!(authoritative.slot(0), prediction.slot(0));
            assert_eq!(authoritative.player_item(0).unwrap().count, 4);
            assert_eq!(authoritative.player_item(0), prediction.player_item(0));
            assert!(authoritative.carried.is_none());
            assert!(store(&authoritative, server.as_ref()));
            assert!(store(&prediction, client.as_ref()));
            let saved = server.0.lock().unwrap().clone();
            assert_eq!(saved.data().unwrap().container[0].1.count, 16);
            drop(authoritative);
            drop(prediction);
            let restored = Backpack(Mutex::new(saved));
            let mut reopened = Menu::new(Kind::Mod(kind));
            load(&mut reopened, env, &restored);
            assert_eq!(reopened.slot(0).unwrap().count, 16);
            assert_eq!(reopened.slot(0), client.get_stack(0).as_ref());
        });
    }

    #[test]
    fn rejected_storage_commit_restores_slots_and_predicted_click() {
        let handler = Arc::new(RejectCommit(Backpack(Mutex::new(ItemStack::new(
            "test:backpack",
            1,
        )))));
        let mut desired = vec![None; handler.slots()];
        desired[0] = Some(ItemStack::new("minecraft:stone", 12));
        desired[1] = Some(ItemStack::new("minecraft:stone", 4));
        assert!(!replace(handler.as_ref(), &desired));
        assert!(snapshot(handler.as_ref()).iter().all(Option::is_none));
        crate::menu::tests::with_empty_inventory_env(|env| {
            let kind = Box::leak(Box::new(ModMenuType {
                id: "test:backpack",
                slots: 9,
                data_slots: 0,
                results: Vec::new(),
            }));
            let mut menu = Menu::new(Kind::Mod(kind));
            menu.item_handler = Some(handler.clone());
            menu.carried = Some(ItemStack::new("minecraft:stone", 4));
            menu.click(env, 1, 0, Input::Pickup).unwrap();
            assert!(menu.slot(1).is_none());
            assert_eq!(menu.carried.as_ref().unwrap().count, 4);
            assert!(handler.get_stack(1).is_none());
        });
    }

    #[test]
    fn handler_simulation_and_payload_bounds_are_read_only() {
        let backpack = Backpack(Mutex::new(ItemStack::new("test:backpack", 1)));
        assert_eq!(
            backpack
                .insert(0, ItemStack::new("minecraft:stone", 20), true)
                .unwrap()
                .count,
            4,
        );
        assert!(backpack.get_stack(0).is_none());
        assert!(
            backpack
                .insert(0, ItemStack::new("minecraft:stone", 12), false)
                .is_none()
        );
        assert_eq!(backpack.extract(0, 7, true).unwrap().count, 7);
        assert_eq!(backpack.get_stack(0).unwrap().count, 12);
        assert!(MenuOpening::decode(&[0; 10]).is_none());
        let opening = MenuOpening {
            window: 1,
            menu_type: "test:backpack".into(),
            extra: vec![0; MAX_MENU_EXTRA + 1],
        };
        assert!(opening.encode().is_none());
        let opening = MenuOpening {
            extra: vec![0; MAX_MENU_EXTRA],
            ..opening
        };
        let mut encoded = opening.encode().unwrap();
        assert_eq!(MenuOpening::decode(&encoded), Some(opening));
        encoded.push(0);
        assert!(MenuOpening::decode(&encoded).is_none());
        encoded.truncate(8);
        assert!(MenuOpening::decode(&encoded).is_none());
    }
}
