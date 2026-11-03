//! Server permission levels for native mods (docs/native-mods.md,
//! API @API:player_permission_level@): recompute the hub's players' levels and
//! resend their permission and command packets through PlayerList's own path.
use super::Game;

impl Game<'_> {
    /// `PlayerList.sendPlayerPermissionLevel` for everyone: recompute every placed player's level
    /// (the ops list, the settings, and `native_player_permission_level`) and send each their level
    /// and command tree again. False when this game runs no server or its players are not reachable
    /// right now (call it from a server hook such as `native_server_tick`).
    /// API @API:player_permission_level@.
    pub(super) fn native_refresh_permissions(&mut self) -> bool {
        if self.lan_acting.is_some() {
            return false;
        }
        let Some(host) = &mut self.lan else {
            return false;
        };
        host.hub.resend_commands();
        true
    }
}
