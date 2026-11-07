use super::{Game, lan::LanSettings};
use horizon_client::lan;

#[derive(Clone, Copy, Debug)]
pub struct NativeLanPublish {
    pub port: Option<u16>,
    pub loopback_only: bool,
    pub announce: bool,
    pub guest_commands: bool,
    pub game_mode: Option<u8>,
    pub max_players: Option<u16>,
}

impl Default for NativeLanPublish {
    fn default() -> Self {
        Self {
            port: None,
            loopback_only: false,
            announce: true,
            guest_commands: false,
            game_mode: None,
            max_players: None,
        }
    }
}

fn publish_settings(opts: &NativeLanPublish) -> Result<LanSettings, String> {
    if opts.game_mode.is_some_and(|mode| mode > 3) {
        return Err("game mode must be between 0 and 3".into());
    }
    Ok(LanSettings {
        port: opts.port.unwrap_or(0),
        guest_commands: opts.guest_commands,
        force_game_mode: opts.game_mode.is_some(),
        loopback_only: opts.loopback_only,
        announce: opts.announce,
        max_players: opts
            .max_players
            .map_or(lan::MAX_PLAYERS, |max| usize::from(max.clamp(1, 1000))),
        game_mode: opts.game_mode,
    })
}

impl Game<'_> {
    pub(super) fn native_lan_publish_impl(
        &mut self,
        opts: &NativeLanPublish,
    ) -> Result<u16, String> {
        if let Some(server) = &self.integrated {
            let opts = *opts;
            return server
                .call(move |game| game.native_lan_publish_impl(&opts))
                .ok_or_else(|| super::integrated::NO_ANSWER.to_owned())?;
        }
        if self.options.execution_role != super::execution_role::ExecutionRole::IntegratedServer {
            return Err("LAN publication requires an integrated server".into());
        }
        let settings = publish_settings(opts)?;
        self.publish(settings)
    }

    pub(super) fn native_lan_close_impl(&mut self) -> bool {
        let closed = if let Some(server) = &self.integrated {
            server.call(|game| game.unpublish()).unwrap_or(false)
        } else {
            self.unpublish()
        };
        if closed {
            self.friends_lan_closed();
        }
        closed
    }

    pub(super) fn native_lan_port_impl(&self) -> Option<u16> {
        if let Some(server) = &self.integrated {
            return server.published();
        }
        self.published_port()
    }

    pub(super) fn native_set_max_players_impl(&mut self, max: u16) -> bool {
        if let Some(server) = &self.integrated {
            return server
                .call(move |game| game.native_set_max_players_impl(max))
                .unwrap_or(false);
        }
        if !self.native_server_ops_available() {
            return false;
        }
        let max = usize::from(max.clamp(1, 1000));
        if self.native_server_ops_detached() {
            self.native_server_ops.set_max_players(max);
        } else if let Some(host) = self.lan.as_mut() {
            host.settings.max_players = max;
            host.hub.set_max_players(max);
        } else {
            return false;
        }
        true
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::net::{IpAddr, Ipv4Addr};

    #[test]
    fn defaults_preserve_existing_publish_settings() {
        let settings = publish_settings(&NativeLanPublish::default()).unwrap();
        assert_eq!(settings.port, 0);
        assert_eq!(settings.bind_address(), None);
        assert!(settings.announce);
        assert!(!settings.guest_commands);
        assert!(!settings.force_game_mode);
        assert_eq!(settings.max_players, lan::MAX_PLAYERS);
        assert_eq!(settings.game_mode, None);
    }

    #[test]
    fn mod_can_select_private_publish_settings() {
        let settings = publish_settings(&NativeLanPublish {
            port: Some(25570),
            loopback_only: true,
            announce: false,
            guest_commands: true,
            game_mode: Some(2),
            max_players: Some(24),
        })
        .unwrap();
        assert_eq!(settings.port, 25570);
        assert_eq!(
            settings.bind_address(),
            Some(IpAddr::V4(Ipv4Addr::LOCALHOST))
        );
        assert!(!settings.announce);
        assert!(settings.guest_commands);
        assert!(settings.force_game_mode);
        assert_eq!(settings.max_players, 24);
        assert_eq!(settings.game_mode, Some(2));
    }

    #[test]
    fn player_limit_is_clamped_and_invalid_modes_are_rejected() {
        for (requested, expected) in [(0, 1), (1, 1), (1000, 1000), (u16::MAX, 1000)] {
            let settings = publish_settings(&NativeLanPublish {
                max_players: Some(requested),
                ..Default::default()
            })
            .unwrap();
            assert_eq!(settings.max_players, expected);
        }
        assert!(
            publish_settings(&NativeLanPublish {
                game_mode: Some(4),
                ..Default::default()
            })
            .is_err()
        );
    }
}
