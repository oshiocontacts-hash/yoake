use super::{Game, chat::PLAYER_NAME};
use horizon_client::{lan::playerdata::ints_uuid, scoreboard::Scoreboard};

fn lookup_team<'name>(
    enabled: bool,
    board: &Scoreboard,
    profile: impl FnOnce() -> Option<&'name str>,
) -> Option<String> {
    if !enabled {
        return None;
    }
    board.players_team(profile()?).map(|team| team.name.clone())
}

impl Game<'_> {
    pub(super) fn native_player_team_impl(&self, uuid: u128) -> Option<String> {
        let enabled = !horizon_client::mod_api::mods().is_empty()
            && self.native_client_world_key().is_some()
            && self.player.is_some();
        lookup_team(enabled, &self.score.board, || {
            if let Some(remote) = &self.remote {
                return remote.known_player_profile(uuid);
            }
            if self
                .level
                .player_uuid
                .is_some_and(|words| ints_uuid(words) == uuid)
            {
                return Some(PLAYER_NAME);
            }
            self.lan
                .as_ref()?
                .hub
                .guests
                .iter()
                .find(|guest| guest.placed && guest.uuid == uuid)
                .map(|guest| guest.name.as_str())
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn no_mod_does_not_resolve_a_profile_or_touch_the_board() {
        let board = Scoreboard::default();
        assert_eq!(lookup_team(false, &board, || panic!("inactive lookup")), None);
        assert!(board.teams.is_empty());
    }

    #[test]
    fn mod_reads_internal_team_names_and_tracks_membership_changes() {
        let mut board = Scoreboard::default();
        board.add_team("recorders").display_name = horizon_client::scoreboard::literal("Shown");
        board.add_to_team("Alice", "recorders");
        assert_eq!(
            lookup_team(true, &board, || Some("Alice")),
            Some("recorders".into())
        );
        assert_eq!(lookup_team(true, &board, || Some("Unknown")), None);
        assert_eq!(lookup_team(true, &board, || None), None);
        board.add_team("spectators");
        board.add_to_team("Alice", "spectators");
        assert_eq!(
            lookup_team(true, &board, || Some("Alice")),
            Some("spectators".into())
        );
        board.remove_team("spectators");
        assert_eq!(lookup_team(true, &board, || Some("Alice")), None);
    }
}
