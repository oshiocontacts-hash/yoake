//! The game half of F4M-4c: a right click on a custom mob reaches the native `mob-interact` dispatch
//! (`mobs::custom::ai::interact`), and a breeding of custom mobs becomes a baby of the type's
//! `offspring`. Both are only reached for mobs of a custom type; vanilla mobs never get here.
use super::{Game, Hand};
use horizon_client::{
    items::ItemStack,
    mobs::{
        Mob,
        custom::ai::{
            breed,
            interact::{self, Action, Context, Reply},
            riding::{self, RiderKind},
        },
    },
};
use horizon_core::random::RandomSource;

impl Game<'_> {
    /// Right click on custom mob `index`. True when the dispatch took the click (the held item is
    /// used up, the player seated or the mob fed as the outcome says); false hands it on to the
    /// engine's own handlers (the vanilla paths ignore a custom mob).
    pub(super) fn interact_custom_mob(&mut self, index: usize) -> bool {
        let m = &self.mobs[index];
        let id = m.id;
        if m.kind.custom.is_none() || !m.alive() || Some(id) == self.riding {
            return false;
        }
        let player = self.acting_player_uuid();
        let sneaking = self.keys.contains(&winit::keyboard::KeyCode::ShiftLeft)
            || self.keys.contains(&winit::keyboard::KeyCode::ShiftRight);
        let item = self.held().map(|h| h.item.to_string());
        // The seat the player would get: the type must be rideable by it now, and a seat free.
        let ride_seat = {
            let w = self.mob_world_base(&[]);
            let m = &self.mobs[index];
            let aboard: Vec<RiderKind> = self
                .mobs
                .iter()
                .filter(|k| k.extra.mount == Some(id))
                .map(riding::kind_of)
                .collect();
            let player_aboard = m.rider.is_some() || self.ridden_by_guest(id);
            riding::rideable(m, &w, Some(player))
                .then(|| riding::free_seat(m, RiderKind::Player, player_aboard, &aboard))
                .flatten()
        };
        let hand = if self.hand == Hand::Off { interact::Hand::Off } else { interact::Hand::Main };
        let outcome = {
            let tags = &self.res.items().item_tags;
            let matches = |pattern: &str, item: &str| match pattern.strip_prefix('#') {
                Some(tag) => tags.get(tag).is_some_and(|t| t.contains(item)),
                None => pattern == item,
            };
            let ctx = Context {
                player,
                hand,
                item: item.as_deref(),
                sneaking,
                matches: &matches,
                ride_seat,
            };
            interact::dispatch_global(&mut self.mobs[index], &ctx)
        };
        if outcome.reply == Reply::Pass {
            return false;
        }
        let held = self.held().cloned();
        for action in outcome.actions {
            match action {
                Action::Saddle => self.mobs[index].set_saddle(held.clone()),
                Action::Give { item, count } => {
                    if let Some(rest) = self.give(ItemStack::new(&item, count)) {
                        self.spawn_item(rest, self.eye_position().unwrap_or([0.0; 3]), [0.0; 3], 40);
                    }
                }
                Action::Ride { .. } => {
                    if self.native_may_board(id) {
                        self.riding = Some(id);
                        self.trigger_player("minecraft:started_riding");
                    }
                }
                Action::Event(event) => {
                    if event == 7 {
                        // Taming: the advancement and the mod hook, as for a horse.
                        let view = self.mob_entity(&self.mobs[index]);
                        self.trigger_entity("minecraft:tame_animal", "entity", &view);
                        let kind = self.mobs[index].kind.id.clone();
                        let owner = Some(horizon_client::lan::playerdata::ints_uuid(player));
                        self.native_entity_tame(id, &kind, owner);
                    }
                    self.mob_event_particles(id, event as i8);
                }
                Action::Eat => {
                    let at = self.mobs[index].center();
                    self.audio.play("minecraft:entity.generic.eat", Some(at), 1.0, 1.0);
                }
            }
        }
        if outcome.reply == Reply::Consume {
            self.consume_held();
        }
        self.swing_arm();
        true
    }

    /// `Animal.finalizeSpawnChildFromBreeding` for a custom parent: the baby of the type's
    /// `offspring` where the parent stands, then the data part (`breed::complete`: the partner's
    /// pause, the baby's age and scale, an owner handed on, the `mob-bred` notice).
    pub(super) fn custom_breed(&mut self, parent: u32, partner: u32) {
        let Some(pi) = self.mobs.iter().position(|m| m.id == parent) else {
            return;
        };
        let Some(kind) = breed::offspring_kind(&self.mobs[pi]) else {
            return;
        };
        let at = self.mobs[pi].position;
        let Some(child_id) = self.spawn_mob(&kind, [at.x, at.y, at.z], true) else {
            return;
        };
        let xp = 1 + self.mobs[pi].random.next_int_bound(7).unwrap_or(0) as u32;
        let (Some(pi), Some(ci)) = (
            self.mobs.iter().position(|m| m.id == parent),
            self.mobs.iter().position(|m| m.id == child_id),
        ) else {
            return;
        };
        let mut p: Mob = self.mobs[pi].clone();
        let mut q: Option<Mob> = self.mobs.iter().find(|m| m.id == partner).cloned();
        let mut child: Mob = self.mobs[ci].clone();
        horizon_client::mobs::spawning::bred_offspring(&mut child);
        breed::complete(&mut p, q.as_mut(), &mut child);
        self.mobs[pi] = p;
        self.mobs[ci] = child;
        if let Some(q) = q
            && let Some(slot) = self.mobs.iter_mut().find(|m| m.id == partner)
        {
            *slot = q;
        }
        self.spawn_breeding_orb([at.x, at.y, at.z], xp);
    }
}
