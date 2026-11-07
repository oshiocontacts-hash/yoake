use super::Game;
use horizon_client::Result;
use horizon_protocol::text::literal;

const MAX_PENDING_KICKS: usize = 64;

#[derive(Default)]
pub(super) struct NativeServerOps {
    kicks: Vec<PendingKick>,
    max_players: Option<usize>,
    saves_start: Option<bool>,
    saves_end: Option<bool>,
    saving: bool,
}

struct PendingKick {
    player: u128,
    reason: String,
}

impl NativeServerOps {
    pub(super) fn save_in_progress(&self) -> bool {
        self.saving
    }

    pub(super) fn set_max_players(&mut self, max: usize) {
        self.max_players = Some(max);
    }

    fn request_kick(&mut self, player: u128, reason: &str) -> bool {
        if let Some(pending) = self
            .kicks
            .iter_mut()
            .find(|pending| pending.player == player)
        {
            pending.reason = reason.to_owned();
        } else {
            if self.kicks.len() >= MAX_PENDING_KICKS {
                return false;
            }
            self.kicks.push(PendingKick {
                player,
                reason: reason.to_owned(),
            });
        }
        true
    }

    fn kick_connection(
        player: u128,
        owner: Option<u128>,
        guests: impl IntoIterator<Item = (u128, u32)>,
    ) -> Option<u32> {
        if owner == Some(player) {
            return None;
        }
        guests
            .into_iter()
            .find_map(|(uuid, conn)| (uuid == player).then_some(conn))
    }

    fn request_save(&mut self, flush: bool, detached: bool) {
        let pending = if detached && !self.saving {
            &mut self.saves_end
        } else {
            &mut self.saves_start
        };
        *pending = Some(pending.unwrap_or(false) || flush);
    }
}

impl Game<'_> {
    pub(super) fn native_server_ops_available(&self) -> bool {
        !self.options.execution_role.is_client()
            && (self.lan.is_some() || self.native_lifecycle.epoch().is_some())
    }

    pub(super) fn native_server_ops_detached(&self) -> bool {
        self.lan.is_none() || self.lan_acting.is_some()
    }

    pub(super) fn native_kick_player_impl(&mut self, player: u128, reason: &str) -> bool {
        if let Some(server) = &self.integrated {
            let reason = reason.to_owned();
            return server
                .call(move |game| game.native_kick_player_impl(player, &reason))
                .unwrap_or(false);
        }
        if !self.native_server_ops_available() {
            return false;
        }
        if self.native_server_ops_detached() {
            return self.native_server_ops.request_kick(player, reason);
        }
        let Some(host) = self.lan.as_mut() else {
            return false;
        };
        let owner = host
            .hub
            .singleplayer_owner
            .or_else(|| host.owner.as_ref().map(|owner| owner.uuid));
        let Some(conn) = NativeServerOps::kick_connection(
            player,
            owner,
            host.hub.guests.iter().map(|guest| (guest.uuid, guest.conn)),
        ) else {
            return false;
        };
        host.hub.kick(conn, literal(reason));
        true
    }

    pub(super) fn native_request_save_impl(&mut self, flush: bool) -> bool {
        if let Some(server) = &self.integrated {
            return server
                .call(move |game| game.native_request_save_impl(flush))
                .unwrap_or(false);
        }
        if !self.native_server_ops_available() {
            return false;
        }
        let detached = self.native_server_ops_detached();
        self.native_server_ops.request_save(flush, detached);
        true
    }

    pub(super) fn native_server_ops_tick_start(&mut self) -> Result<()> {
        if self.native_server_ops_detached() {
            return Ok(());
        }
        if let Some(flush) = self.native_server_ops.saves_start.take() {
            self.native_server_ops_save(flush)?;
        }
        Ok(())
    }

    pub(super) fn native_server_ops_tick_end(&mut self) -> Result<()> {
        if self.native_server_ops_detached() {
            return Ok(());
        }
        let kicks = std::mem::take(&mut self.native_server_ops.kicks);
        let max_players = self.native_server_ops.max_players.take();
        if let Some(host) = self.lan.as_mut() {
            if let Some(max) = max_players {
                host.settings.max_players = max;
                host.hub.set_max_players(max);
            }
            let owner = host
                .hub
                .singleplayer_owner
                .or_else(|| host.owner.as_ref().map(|owner| owner.uuid));
            for pending in kicks {
                if let Some(conn) = NativeServerOps::kick_connection(
                    pending.player,
                    owner,
                    host.hub.guests.iter().map(|guest| (guest.uuid, guest.conn)),
                ) {
                    host.hub.kick(conn, literal(&pending.reason));
                }
            }
        }
        if let Some(flush) = self.native_server_ops.saves_end.take() {
            self.native_server_ops_save(flush)?;
        }
        Ok(())
    }

    fn native_server_ops_save(&mut self, flush: bool) -> Result<()> {
        self.native_server_ops.saving = true;
        let result = self.save_game_with(flush);
        self.native_server_ops.saving = false;
        result
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn no_mod_calls_leave_existing_tick_work_unchanged() {
        let mut ops = NativeServerOps::default();
        assert!(ops.kicks.is_empty());
        assert_eq!(ops.kicks.capacity(), 0);
        assert!(std::mem::take(&mut ops.kicks).is_empty());
        assert_eq!(ops.kicks.capacity(), 0);
        assert!(ops.max_players.is_none());
        assert!(ops.saves_start.is_none());
        assert!(ops.saves_end.is_none());
        assert!(!ops.save_in_progress());
    }

    #[test]
    fn placed_kicks_refuse_the_owner_and_unknown_players() {
        let guests = [(10, 1), (20, 2)];
        assert_eq!(NativeServerOps::kick_connection(10, Some(10), guests), None);
        assert_eq!(NativeServerOps::kick_connection(30, Some(10), guests), None);
        assert_eq!(
            NativeServerOps::kick_connection(20, Some(10), guests),
            Some(2)
        );
    }

    #[test]
    fn detached_kicks_are_accepted_then_resolved_against_the_real_hub() {
        let mut ops = NativeServerOps::default();
        assert!(ops.request_kick(10, "owner"));
        assert!(ops.request_kick(30, "gone"));
        assert!(ops.request_kick(20, "first"));
        assert!(ops.request_kick(20, "literal final reason"));
        assert_eq!(ops.kicks.len(), 3);

        let guests = [(10, 1), (20, 7), (40, 2)];
        let kicks: Vec<_> = std::mem::take(&mut ops.kicks)
            .into_iter()
            .filter_map(|pending| {
                NativeServerOps::kick_connection(pending.player, Some(10), guests)
                    .map(|conn| (pending.player, conn, pending.reason))
            })
            .collect();
        assert_eq!(kicks, vec![(20, 7, "literal final reason".to_owned())]);
        assert!(ops.kicks.is_empty());
    }

    #[test]
    fn detached_kicks_are_bounded_and_update_existing_players_when_full() {
        let mut ops = NativeServerOps::default();
        for player in 0..MAX_PENDING_KICKS {
            assert!(ops.request_kick(player as u128, "first"));
        }
        assert_eq!(ops.kicks.len(), MAX_PENDING_KICKS);
        assert!(!ops.request_kick(MAX_PENDING_KICKS as u128, "overflow"));
        assert!(ops.request_kick(0, "latest"));
        assert_eq!(ops.kicks.len(), MAX_PENDING_KICKS);
        assert_eq!(ops.kicks[0].reason, "latest");
    }

    #[test]
    fn saves_coalesce_without_weakening_flush_or_crossing_tick_boundaries() {
        let mut ops = NativeServerOps::default();
        ops.request_save(false, false);
        ops.request_save(true, false);
        ops.request_save(false, false);
        ops.request_save(false, true);
        assert_eq!(ops.saves_start.take(), Some(true));
        assert_eq!(ops.saves_end.take(), Some(false));
        assert_eq!(ops.saves_start, None);
        assert_eq!(ops.saves_end, None);
    }

    #[test]
    fn completion_callback_requests_wait_until_the_next_tick() {
        let mut ops = NativeServerOps {
            saving: true,
            ..Default::default()
        };
        ops.request_save(true, true);
        assert_eq!(ops.saves_start, Some(true));
        assert_eq!(ops.saves_end, None);
    }

    #[test]
    fn deferred_limit_uses_the_latest_request_and_reset_drops_pending_work() {
        let mut ops = NativeServerOps::default();
        ops.set_max_players(12);
        ops.set_max_players(24);
        assert!(ops.request_kick(20, "pending"));
        ops.request_save(true, true);
        assert_eq!(ops.max_players.take(), Some(24));
        ops = NativeServerOps::default();
        assert!(ops.kicks.is_empty());
        assert_eq!(ops.kicks.capacity(), 0);
        assert!(ops.max_players.is_none());
        assert!(ops.saves_start.is_none());
        assert!(ops.saves_end.is_none());
        assert!(!ops.save_in_progress());
    }
}
