//! Players on the seats of a custom vehicle (F4M-4d, reviewer decision 1): the host in the window and
//! the LAN guests hold seat claims in the vehicle's `CustomState` like the mob riders do, so no two
//! riders ever share a seat. A player claims its seat when it boards (`riding::claim_player`); once a
//! tick this makes the claims match who really rides (a player that got off, disconnected or died frees
//! its seat; a rider that arrived without a claim, such as one restored from a save, takes one).
use super::Game;
use horizon_client::{lan::playerdata::uuid_ints, mobs::custom::ai::riding};
use horizon_protocol::packets::play::{self, Clientbound as C};

impl Game<'_> {
    /// Who rides which vehicle: (the player's UUID, the vehicle's id). The host in the window while no
    /// guest's state is swapped in, and every server player with a vehicle that is not dead.
    fn riding_players(&mut self) -> Vec<([i32; 4], u32)> {
        let mut out = Vec::new();
        if self.lan_acting.is_none()
            && let Some(vehicle) = self.riding
        {
            out.push((self.level.ensure_player_uuid(), vehicle));
        }
        if let Some(host) = self.lan.as_ref() {
            for g in &host.hub.guests {
                let (Some(vehicle), false) = (g.vehicle, g.dead || g.health <= 0.0) else {
                    continue;
                };
                let uuid = uuid_ints(g.uuid);
                if !out.iter().any(|(p, _)| *p == uuid) {
                    out.push((uuid, vehicle));
                }
            }
        }
        out
    }

    /// Makes the player claims of every custom vehicle with seats match who rides it (once a tick,
    /// before the mob riders are settled so the players keep their seats): claims of players that do not
    /// ride it any more are freed, players without a claim take one, and a player no seat is left for
    /// (it was put on a full vehicle by something that did not ask) gets off.
    pub(super) fn sync_player_seats(&mut self) {
        // A client is told the claims by its server (`apply_remote_seat_claims`); and in a guest's turn
        // the host's state is not the one riding.
        if self.remote.is_some() || self.lan_acting.is_some() {
            return;
        }
        if !self.mobs.iter().any(riding::has_seats) {
            return;
        }
        let riders = self.riding_players();
        let evict = riding::sync_players(&mut self.mobs, &riders);
        for (player, vehicle) in evict {
            self.put_player_off(player, vehicle);
        }
    }

    /// Takes `player` off `vehicle` without asking anyone (it has no seat there).
    fn put_player_off(&mut self, player: [i32; 4], vehicle: u32) {
        if self.level.player_uuid == Some(player) && self.riding == Some(vehicle) {
            self.riding = None;
            if let Some(m) = self.mobs.iter_mut().find(|m| m.id == vehicle) {
                m.rider = None;
            }
            return;
        }
        let Some(host) = self.lan.as_mut() else { return };
        if let Some(g) = host.hub.guests.iter_mut().find(|g| uuid_ints(g.uuid) == player && g.vehicle == Some(vehicle)) {
            (g.vehicle, g.vehicle_good) = (None, None);
            g.send(&C::SetPassengers(play::SetPassengers { vehicle: vehicle as i32, passengers: Vec::new() }));
        }
    }

    /// Whether `vehicle` is a custom mob with seats (it seats several riders, host and guests too).
    pub(super) fn mob_has_seats(&self, vehicle: u32) -> bool {
        self.mobs.iter().any(|m| m.id == vehicle && riding::has_seats(m))
    }

    /// The AI's boarding of a custom vehicle with seats (honours the boarding cooldown): the native mods
    /// may veto it, as they may a `/ride` (`native_mob_may_ride`).
    pub(super) fn board_seated_mob(&mut self, rider: u32, vehicle: u32) -> bool {
        let Ok(plan) = riding::plan_board(&self.mobs, rider, vehicle, None, true) else { return false };
        if !self.native_mob_may_ride(rider, vehicle, true) {
            return false;
        }
        let seat = riding::commit_board(&mut self.mobs, rider, vehicle, plan);
        log::info!("Horizon custom-mob seat: AI boarding: mob {rider} takes seat {seat} of {vehicle}");
        true
    }

    /// A script's `seat-set`: puts a mob on a seat or takes the one on it off, unless a native mod vetoes
    /// the boarding or the dismounting (`native_mob_may_ride`).
    pub(super) fn script_seat_set(&mut self, vehicle: u32, seat: usize, rider: Option<u32>) {
        let Some((who, mounting)) = riding::plan_seat_set(&self.mobs, vehicle, seat, rider) else {
            log::info!("Horizon custom-mob seat: seat-set vehicle {vehicle} seat {seat} rider {rider:?}: nothing to do");
            return;
        };
        if !self.native_mob_may_ride(who, vehicle, mounting) {
            log::info!("Horizon custom-mob seat: seat-set vehicle {vehicle} seat {seat} rider {rider:?}: vetoed by a native mod");
            return;
        }
        riding::seat_set(&mut self.mobs, vehicle, seat, rider);
        let now = self.mobs.iter().find(|m| m.id == vehicle).and_then(|v| riding::claim_of(v, who));
        log::info!("Horizon custom-mob seat: seat-set vehicle {vehicle} seat {seat} rider {rider:?}: mob {who} now holds seat {now:?}");
    }
}
