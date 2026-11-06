//! `Player.getCooldowns()` (`ItemCooldowns`) as the game keeps it: the
//! `use_cooldown` component's entries by cooldown group, counted down once a
//! tick, and the percent `GuiGraphicsExtractor.itemCooldown` shades a slot
//! by. The world crate's `item_cooldowns` models vanilla's hash map and its
//! observers for the parity harness; this is the same data with the lookup a
//! slot needs, and no observer (vanilla's own player uses the plain class).
use crate::items::ItemStack;
use horizon_world::nbt::Tag;
use std::collections::HashMap;
use std::sync::Arc;

/// A stack's `minecraft:use_cooldown` component.
#[derive(Clone, Debug, PartialEq)]
pub struct UseCooldown {
    pub seconds: f32,
    /// `cooldown_group`: the group the entry is filed under, else the item.
    pub group: Option<Arc<str>>,
}
impl UseCooldown {
    /// `UseCooldown.ticks`: the seconds in ticks, truncated.
    pub fn ticks(&self) -> i32 {
        (self.seconds * 20.0) as i32
    }
    /// The component as `items.json` holds it.
    pub fn from_json(v: &serde_json::Value) -> Option<Self> {
        let seconds = v.get("seconds")?.as_f64()? as f32;
        Some(Self {
            seconds,
            group: v
                .get("cooldown_group")
                .and_then(|g| g.as_str())
                .map(Arc::from),
        })
    }
    /// The component as a stack's saved `components` holds it.
    pub fn from_tag(t: &Tag) -> Option<Self> {
        let Tag::Compound(c) = t else {
            return None;
        };
        let text = |k: &str| match c.get(&horizon_world::nbt::JavaString::from(k)) {
            Some(Tag::String(s)) => Some(s.to_string_lossy()),
            _ => None,
        };
        // NbtOps.getNumberValue: any numeric tag (vanilla's codec reads a
        // positive float).
        let seconds = horizon_world::datafix::dynamic::as_number(
            c.get(&horizon_world::nbt::JavaString::from("seconds"))?,
        )?
        .float_value();
        Some(Self {
            seconds,
            group: text("cooldown_group").map(Arc::from),
        })
    }
}

/// The component a stack carries: its own patch first (`ItemStack.get`), then
/// the item's default from `items.json` (`ItemInfo::use_cooldown`).
pub fn component(stack: &ItemStack, default: Option<&UseCooldown>) -> Option<UseCooldown> {
    stack
        .data()
        .and_then(|d| d.other("minecraft:use_cooldown"))
        .and_then(UseCooldown::from_tag)
        .or_else(|| default.cloned())
}

/// `ItemCooldowns.getCooldownGroup`: the component's group, else the item id.
pub fn group(stack: &ItemStack, default: Option<&UseCooldown>) -> Arc<str> {
    component(stack, default)
        .and_then(|c| c.group)
        .unwrap_or_else(|| Arc::clone(&stack.item))
}

/// `ItemCooldowns`: start and end ticks by group, and the tick counter.
#[derive(Clone, Debug, Default)]
pub struct Cooldowns {
    tick_count: i32,
    entries: HashMap<Arc<str>, (i32, i32)>,
}
impl Cooldowns {
    /// `getCooldownPercent`: what is left of the entry, from 1 to 0.
    pub fn percent(&self, group: &str, partial_tick: f32) -> f32 {
        let Some((start, end)) = self.entries.get(group) else {
            return 0.0;
        };
        let duration = end.wrapping_sub(*start) as f32;
        let remaining = *end as f32 - (self.tick_count as f32 + partial_tick);
        (remaining / duration).clamp(0.0, 1.0)
    }
    /// Remaining and total ticks for an active entry, including partial ticks.
    pub fn remaining(&self, group: &str, partial_tick: f32) -> Option<(f32, f32)> {
        let (start, end) = self.entries.get(group)?;
        let total = end.wrapping_sub(*start);
        if total <= 0 {
            return None;
        }
        let total = total as f32;
        let remaining = (end.wrapping_sub(self.tick_count) as f32 - partial_tick).clamp(0.0, total);
        (remaining > 0.0).then_some((remaining, total))
    }
    /// Exact whole remaining and total ticks for an active entry.
    pub fn remaining_ticks(&self, group: &str) -> Option<(i32, i32)> {
        let (start, end) = self.entries.get(group)?;
        let total = end.wrapping_sub(*start);
        let remaining = end.wrapping_sub(self.tick_count).clamp(0, total.max(0));
        (remaining > 0 && total > 0).then_some((remaining, total))
    }
    /// `isOnCooldown`: whether the group still has time to run.
    pub fn is_on_cooldown(&self, group: &str) -> bool {
        self.percent(group, 0.0) > 0.0
    }
    /// `addCooldown`.
    pub fn add(&mut self, group: Arc<str>, ticks: i32) {
        self.entries.insert(
            group,
            (self.tick_count, self.tick_count.wrapping_add(ticks)),
        );
    }
    /// `ItemCooldowns.tick`: the counter, then every entry that has run out,
    /// which it hands back (`onCooldownEnded`).
    pub fn tick(&mut self) -> Vec<Arc<str>> {
        self.tick_count = self.tick_count.wrapping_add(1);
        let done: Vec<Arc<str>> = self
            .entries
            .iter()
            .filter(|(_, (_, end))| *end <= self.tick_count)
            .map(|(group, _)| Arc::clone(group))
            .collect();
        self.entries.retain(|_, (_, end)| *end > self.tick_count);
        done
    }
    /// `removeCooldown`.
    pub fn remove(&mut self, group: &str) {
        self.entries.remove(group);
    }
    pub fn is_empty(&self) -> bool {
        self.entries.is_empty()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn stack(item: &str) -> ItemStack {
        ItemStack::new(item, 1)
    }

    #[test]
    fn the_percent_falls_from_one_to_zero_over_the_entry() {
        let mut c = Cooldowns::default();
        c.add(Arc::from("minecraft:ender_pearl"), 20);
        assert!(c.is_on_cooldown("minecraft:ender_pearl"));
        assert_eq!(c.percent("minecraft:ender_pearl", 0.0), 1.0);
        for _ in 0..10 {
            assert!(c.tick().is_empty());
        }
        assert_eq!(c.percent("minecraft:ender_pearl", 0.0), 0.5);
        // The partial tick moves it within the tick.
        assert_eq!(c.percent("minecraft:ender_pearl", 0.5), 0.475);
        for _ in 0..9 {
            c.tick();
        }
        assert!(c.is_on_cooldown("minecraft:ender_pearl"));
        // ItemCooldowns.tick drops the entry at its end tick, and says so.
        assert_eq!(c.tick(), [Arc::<str>::from("minecraft:ender_pearl")]);
        assert!(!c.is_on_cooldown("minecraft:ender_pearl"));
        assert!(c.is_empty());
        assert_eq!(c.percent("minecraft:ender_pearl", 0.0), 0.0);
    }

    #[test]
    fn the_group_comes_from_the_stack_then_the_item() {
        let pearl = stack("minecraft:ender_pearl");
        let default = UseCooldown::from_json(&json!({"seconds": 1.0})).expect("the component");
        assert_eq!(&*group(&pearl, Some(&default)), "minecraft:ender_pearl");
        // A stack's own component overrides the item's, group included.
        let mut grouped = stack("minecraft:ender_pearl");
        grouped.data_mut().set_other(
            "minecraft:use_cooldown",
            Some(Tag::compound([
                ("seconds", Tag::Float(2.0f32.to_bits())),
                ("cooldown_group", Tag::String("minecraft:pearls".into())),
            ])),
        );
        let c = component(&grouped, Some(&default)).expect("the stack's component");
        assert_eq!(c.ticks(), 40);
        assert_eq!(&*group(&grouped, Some(&default)), "minecraft:pearls");
        // An item without the component has no cooldown.
        assert!(component(&stack("minecraft:dirt"), None).is_none());
    }

    #[test]
    fn remaining_uses_the_stored_interval_and_partial_ticks() {
        let mut cooldowns = Cooldowns::default();
        assert_eq!(cooldowns.remaining("test:missing", 0.0), None);
        cooldowns.add(Arc::from("test:group"), 20);
        assert_eq!(cooldowns.remaining("test:group", 0.0), Some((20.0, 20.0)));
        for _ in 0..10 {
            cooldowns.tick();
        }
        assert_eq!(cooldowns.remaining("test:group", 0.5), Some((9.5, 20.0)));
        assert_eq!(cooldowns.remaining_ticks("test:group"), Some((10, 20)));
        assert_eq!(cooldowns.percent("test:group", 0.5), 9.5 / 20.0);
        assert_eq!(cooldowns.remaining("test:group", -20.0), Some((20.0, 20.0)));
        assert_eq!(cooldowns.remaining("test:group", 10.0), None);
        assert_eq!(cooldowns.remaining_ticks("test:group"), Some((10, 20)));
    }

    #[test]
    fn remaining_replaces_removes_and_expires() {
        let mut cooldowns = Cooldowns::default();
        let group: Arc<str> = Arc::from("test:group");
        cooldowns.add(group.clone(), 20);
        cooldowns.tick();
        cooldowns.add(group.clone(), 3);
        assert_eq!(cooldowns.remaining(&group, 0.25), Some((2.75, 3.0)));
        cooldowns.remove(&group);
        assert_eq!(cooldowns.remaining(&group, 0.0), None);
        cooldowns.add(group.clone(), 1);
        assert_eq!(cooldowns.remaining(&group, 0.5), Some((0.5, 1.0)));
        assert_eq!(cooldowns.remaining(&group, 1.0), None);
        assert_eq!(cooldowns.tick(), vec![group.clone()]);
        assert_eq!(cooldowns.remaining(&group, 0.0), None);
        assert_eq!(cooldowns.remaining_ticks(&group), None);
        cooldowns.add(group.clone(), 0);
        assert_eq!(cooldowns.remaining(&group, 0.0), None);
    }

    #[test]
    fn whole_remaining_ticks_do_not_round_large_durations() {
        let mut cooldowns = Cooldowns::default();
        cooldowns.add(Arc::from("test:group"), 16_777_217);
        assert_eq!(
            cooldowns.remaining_ticks("test:group"),
            Some((16_777_217, 16_777_217))
        );
        cooldowns.tick();
        assert_eq!(
            cooldowns.remaining_ticks("test:group"),
            Some((16_777_216, 16_777_217))
        );
    }

    #[test]
    fn the_seconds_read_from_any_numeric_tag() {
        let from = |seconds: Tag| UseCooldown::from_tag(&Tag::compound([("seconds", seconds)]));
        // An integer tag was dropped before (#7638); seconds: 5 is 100 ticks.
        let int = from(Tag::Int(5)).expect("the component");
        assert_eq!(int.seconds, 5.0);
        assert_eq!(int.ticks(), 100);
        // The other numeric tags read the same way.
        assert_eq!(from(Tag::Byte(5)).unwrap().seconds, 5.0);
        assert_eq!(from(Tag::Long(5)).unwrap().seconds, 5.0);
        assert_eq!(from(Tag::Double(2.5f64.to_bits())).unwrap().seconds, 2.5);
        // The float path keeps working.
        assert_eq!(from(Tag::Float(5.0f32.to_bits())).unwrap().seconds, 5.0);
    }
}
