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
        if let Err(e) = remote.custom_mobs.apply_payload(data, &kind_of) {
            log::debug!("Horizon: a bad mob sync payload: {e}");
        }
    }
}
