use super::{Game, lan_actions::send_item_cooldown};
use horizon_client::{
    cooldowns::{self, Cooldowns},
    items::ItemStack,
};
use std::{cell::Cell, sync::Arc};

/// A cooldown group, or an item whose effective component selects the group.
#[derive(Clone, Copy, Debug)]
pub enum NativeCooldownTarget<'a> {
    ItemId(&'a str),
    Stack(&'a ItemStack),
    Group(&'a str),
}

thread_local! {
    static SETTING_ITEM_COOLDOWN: Cell<bool> = const { Cell::new(false) };
}

struct CooldownGuard;

impl CooldownGuard {
    fn enter() -> Result<Self, String> {
        SETTING_ITEM_COOLDOWN.with(|setting| {
            if setting.replace(true) {
                Err("an item cooldown setter is already running".to_string())
            } else {
                Ok(Self)
            }
        })
    }
}

impl Drop for CooldownGuard {
    fn drop(&mut self) {
        SETTING_ITEM_COOLDOWN.with(|setting| setting.set(false));
    }
}

fn validate_ticks(ticks: i32) -> Result<(), String> {
    if ticks < 0 {
        Err("item cooldown ticks must not be negative".to_string())
    } else {
        Ok(())
    }
}

fn set_cooldown(store: &mut Cooldowns, group: Arc<str>, ticks: i32) {
    if ticks == 0 {
        store.remove(&group);
    } else {
        store.add(group, ticks);
    }
}

// CHECK: Declare/export this module and generate both Host helpers before compiling.
#[allow(dead_code)]
impl Game<'_> {
    fn native_cooldown_group<'a>(
        &self,
        target: NativeCooldownTarget<'a>,
    ) -> Option<(Arc<str>, Option<&'a str>)> {
        match target {
            NativeCooldownTarget::ItemId(item) => {
                let info = self.items().get(item)?;
                let group = info
                    .use_cooldown
                    .as_ref()
                    .and_then(|cooldown| cooldown.group.clone())
                    .unwrap_or_else(|| Arc::from(item));
                Some((group, Some(item)))
            }
            NativeCooldownTarget::Stack(stack) => {
                let info = self.items().get(&stack.item)?;
                Some((
                    cooldowns::group(stack, info.use_cooldown.as_ref()),
                    Some(&stack.item),
                ))
            }
            NativeCooldownTarget::Group(group) => Some((Arc::from(group), None)),
        }
    }

    /// Set or remove an online server player's cooldown. Item starts remain cancellable.
    /// @API:item_cooldown_helpers@.
    // CHECK: Verify actor, owner and guest packet isolation and the existing cancellable hook.
    pub(super) fn native_set_item_cooldown(
        &mut self,
        player: u128,
        target: NativeCooldownTarget<'_>,
        ticks: i32,
    ) -> Result<bool, String> {
        validate_ticks(ticks)?;
        if self.options.execution_role.is_client() {
            return Ok(false);
        }
        let _guard = CooldownGuard::enter()?;
        let Some((group, item)) = self.native_cooldown_group(target) else {
            return Ok(false);
        };
        let actor = self.lan_acting_uuid == Some(player);
        let name = if actor {
            self.lan_acting_name.clone()
        } else {
            self.lan.as_ref().and_then(|host| {
                host.hub
                    .guests
                    .iter()
                    .find(|guest| {
                        guest.placed && guest.uuid == player && host.play.contains_key(&guest.conn)
                    })
                    .map(|guest| guest.name.clone())
            })
        };
        let Some(name) = name else {
            return Ok(false);
        };
        if ticks > 0
            && let Some(item) = item
        {
            let mut canceled = false;
            self.native_player_item_cooldown(player, &name, item, ticks, &mut canceled);
            if canceled {
                return Ok(false);
            }
        }
        if actor && self.lan_acting_uuid == Some(player) {
            set_cooldown(&mut self.cooldowns, group.clone(), ticks);
            self.guest_cooldowns.push((group, ticks));
            return Ok(true);
        }
        let Some(host) = self.lan.as_mut() else {
            return Ok(false);
        };
        let Some(conn) = host
            .hub
            .guests
            .iter()
            .find(|guest| {
                guest.placed && guest.uuid == player && host.play.contains_key(&guest.conn)
            })
            .map(|guest| guest.conn)
        else {
            return Ok(false);
        };
        let Some(play) = host.play.get_mut(&conn) else {
            return Ok(false);
        };
        set_cooldown(&mut play.cooldowns, group.clone(), ticks);
        send_item_cooldown(&host.hub, conn, &group, ticks);
        Ok(true)
    }

    /// Exact whole remaining and total ticks for an online server player.
    /// @API:item_cooldown_helpers@.
    pub(super) fn native_item_cooldown_remaining(
        &self,
        player: u128,
        target: NativeCooldownTarget<'_>,
    ) -> Option<(i32, i32)> {
        if self.options.execution_role.is_client() {
            return None;
        }
        let (group, _) = self.native_cooldown_group(target)?;
        if self.lan_acting_uuid == Some(player) && self.lan_acting_name.is_some() {
            return self.cooldowns.remaining_ticks(&group);
        }
        let host = self.lan.as_ref()?;
        let guest = host.hub.guests.iter().find(|guest| {
            guest.placed && guest.uuid == player && host.play.contains_key(&guest.conn)
        })?;
        host.play
            .get(&guest.conn)?
            .cooldowns
            .remaining_ticks(&group)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn set_replace_remove_and_expire() {
        let mut store = Cooldowns::default();
        let group: Arc<str> = Arc::from("test:cooldown");
        set_cooldown(&mut store, group.clone(), 20);
        assert_eq!(store.remaining_ticks(&group), Some((20, 20)));
        store.tick();
        set_cooldown(&mut store, group.clone(), 3);
        assert_eq!(store.remaining_ticks(&group), Some((3, 3)));
        set_cooldown(&mut store, group.clone(), 0);
        assert_eq!(store.remaining_ticks(&group), None);
        set_cooldown(&mut store, group.clone(), 1);
        assert_eq!(store.tick(), vec![group.clone()]);
        assert_eq!(store.remaining_ticks(&group), None);
    }

    #[test]
    fn player_stores_are_isolated() {
        let mut actor = Cooldowns::default();
        let mut guest = Cooldowns::default();
        let group: Arc<str> = Arc::from("test:shared_group");
        set_cooldown(&mut actor, group.clone(), 20);
        assert_eq!(guest.remaining_ticks(&group), None);
        set_cooldown(&mut guest, group.clone(), 7);
        set_cooldown(&mut actor, group.clone(), 0);
        assert_eq!(actor.remaining_ticks(&group), None);
        assert_eq!(guest.remaining_ticks(&group), Some((7, 7)));
    }

    #[test]
    fn negative_ticks_are_rejected() {
        assert!(validate_ticks(-1).is_err());
        assert!(validate_ticks(i32::MIN).is_err());
        assert!(validate_ticks(0).is_ok());
        assert!(validate_ticks(i32::MAX).is_ok());
    }

    #[test]
    fn setters_cannot_reenter_and_the_guard_resets() {
        let guard = CooldownGuard::enter().unwrap();
        assert!(CooldownGuard::enter().is_err());
        drop(guard);
        assert!(CooldownGuard::enter().is_ok());
    }
}
