//! The game half of `horizon:mob_sync` (F4M-5b, `lan::mob_sync` has the rules): once per tick, after
//! the mobs ticked, the custom mobs' changes are collected and each Horizon client that tracks a mob
//! is sent its part, and a client that just started tracking one gets a snapshot after the mob's
//! `AddEntity` (the tracking pass queued that earlier in the same tick, on the same connection).
//!
//! With no custom type loaded this does nothing at all: no scan of the mobs, no payload.
use super::{Game, lan::LanHost};
use horizon_client::lan::mob_sync;

impl Game<'_> {
    /// Sends this tick's custom mob changes to the Horizon clients in the level in place.
    pub(super) fn flush_mob_sync(&mut self, host: &mut LanHost) {
        let hub = &mut host.hub;
        if !mob_sync::active() {
            return;
        }
        let now = self.world.game_time;
        let mut collected = Vec::new();
        for m in self.mobs.iter_mut() {
            if m.kind.custom.is_some()
                && let Some(c) = mob_sync::collect(m, now)
            {
                collected.push(c);
            }
        }
        let level = hub.config.dimension.clone();
        for g in &mut hub.guests {
            let starts = std::mem::take(&mut g.mob_sync_starts);
            if !mob_sync::reads_mob_sync(g) || g.dimension != level {
                continue;
            }
            let viewer = Some(horizon_client::lan::playerdata::uuid_ints(g.uuid));
            let mut messages = mob_sync::for_viewer_as(
                viewer,
                |entity| g.tracker.is_tracking(entity),
                &starts,
                &collected,
                |entity, viewer| {
                    self.mobs
                        .iter()
                        .find(|m| m.id as i32 == entity)
                        .and_then(|m| mob_sync::snapshot_for(m, viewer))
                },
            );
            // Who sits where on a vehicle that starts being tracked, after its snapshot (F4M-4d).
            for &entity in &starts {
                if g.tracker.is_tracking(entity)
                    && let Some(seats) = self
                        .mobs
                        .iter()
                        .find(|m| m.id as i32 == entity)
                        .and_then(mob_sync::seats_message)
                {
                    messages.push(seats);
                }
            }
            for packet in mob_sync::packets(now, messages) {
                g.send(&packet);
            }
        }
    }
}
