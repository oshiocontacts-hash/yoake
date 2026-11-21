//! Client side of `horizon:mob_sync` (F4M-5b): a server's custom mobs' tracked values, animation
//! clocks and triggers are kept in `Remote.custom_mobs` (`mobs::custom::remote` has the store and its
//! rules). A single-player game and a LAN host read the mob's own state and get no payload.
use super::Game;

impl Game<'_> {
    /// A `horizon:mob_sync` payload the server sent the window.
    pub(super) fn remote_mob_sync(&mut self, data: &[u8]) {
        let mobs = &self.mobs;
        let kind_of = |entity: i32| {
            mobs.iter()
                .find(|m| m.id as i32 == entity)
                .and_then(|m| m.kind.custom.clone())
        };
        let Some(remote) = self.remote.as_mut() else {
            return;
        };
        match remote.custom_mobs.apply_payload(data, &kind_of) {
            // One line a payload, for the end-to-end run (tools/mob_sync_e2e.py) and for a person
            // asking why a custom mob stands still: what was applied and how many mobs are known.
            Ok(done) => log::info!(
                "Horizon mob-sync: snapshots={} deltas={} triggers={} dropped={} incompatible={} known={}",
                done.snapshots,
                done.deltas,
                done.triggers,
                done.dropped,
                done.incompatible,
                remote.custom_mobs.len()
            ),
            Err(e) => log::debug!("Horizon: a bad mob sync payload: {e}"),
        }
    }

    /// A server's custom vehicle seats its mob riders as the server says (`Seats` of `horizon:mob_sync`, F4M-4d):
    /// the claims it sent replace the ones the client worked out, so both place a rider on the same seat.
    pub(super) fn apply_remote_seat_claims(&mut self) {
        let own_uuid = self.acting_player_uuid();
        let Some(remote) = self.remote.as_ref() else { return };
        remote.custom_mobs.copy_seat_claims(&mut self.mobs, own_uuid, remote.entity_id);
    }
}
