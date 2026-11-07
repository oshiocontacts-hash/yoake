use std::path::{Component, Path};

use super::Game;
use horizon_client::title::Mode;

#[derive(Clone, Debug, PartialEq, Eq)]
struct Connection {
    single_player: bool,
    address: Option<String>,
}

#[derive(Debug, PartialEq, Eq)]
enum Request {
    OpenWorld(String),
    Connect(String),
    Leave,
}

#[derive(Clone, Copy, Default)]
struct RequestContext {
    client: bool,
    title: bool,
    playing: bool,
    busy: bool,
}

impl RequestContext {
    fn accepts(self, request: &Request) -> bool {
        self.client
            && !self.busy
            && match request {
                Request::OpenWorld(_) | Request::Connect(_) => self.title,
                Request::Leave => self.playing,
            }
    }
}

#[derive(Default)]
pub(super) struct NativeSession {
    destination: Option<Connection>,
    logged_in: bool,
    leaving: bool,
    pending: Option<Request>,
    shutting_down: bool,
}

impl NativeSession {
    pub(super) fn connecting(&mut self, single_player: bool, address: Option<&str>) {
        self.destination = Some(Connection {
            single_player,
            address: address.map(str::to_owned),
        });
        self.logged_in = false;
        self.leaving = false;
        self.pending = None;
    }

    fn ready(&mut self) -> Option<Connection> {
        if self.logged_in || self.leaving || self.shutting_down {
            return None;
        }
        let destination = self.destination.clone()?;
        self.logged_in = true;
        Some(destination)
    }

    fn left(&mut self) -> bool {
        self.destination = None;
        self.pending = None;
        self.leaving = true;
        std::mem::take(&mut self.logged_in)
    }

    pub(super) fn finish_leave(&mut self) {
        self.leaving = false;
    }

    fn enqueue(&mut self, request: Request, context: RequestContext) -> bool {
        if self.pending.is_some()
            || self.leaving
            || self.shutting_down
            || !context.accepts(&request)
        {
            return false;
        }
        self.pending = Some(request);
        true
    }

    fn take_request(&mut self, context: RequestContext) -> Option<Request> {
        let request = self.pending.take()?;
        (!self.leaving && !self.shutting_down && context.accepts(&request)).then_some(request)
    }

    fn shutdown(&mut self) -> Option<bool> {
        if self.shutting_down {
            return None;
        }
        self.shutting_down = true;
        Some(self.left())
    }
}

fn world_folder(folder: &str) -> bool {
    if folder.trim().is_empty() || folder.contains(['/', '\\']) {
        return false;
    }
    let mut components = Path::new(folder).components();
    matches!(components.next(), Some(Component::Normal(_))) && components.next().is_none()
}

fn disconnect_claimed(
    reason: &str,
    key: Option<&str>,
    from_server: bool,
    notify: impl FnOnce(&str, Option<&str>, bool, &mut bool),
) -> bool {
    let mut claimed = false;
    notify(reason, key, from_server, &mut claimed);
    claimed
}

// CHECK: Integrate the hook wrappers, exports, and frame/shutdown call sites before stamping.
impl Game<'_> {
    fn native_request_context(&self) -> RequestContext {
        let busy = self.rest_loader.is_some()
            || self.pending_entry.is_some()
            || self.pending_pack_reload
            || self.pack_reload.is_some()
            || self.integrated_starting.is_some()
            || self.creation_job.is_some()
            || self.pending_save.is_some();
        RequestContext {
            client: self.options.execution_role.is_client(),
            title: self.title.as_ref().is_some_and(|title| {
                matches!(
                    &title.mode,
                    Mode::Main | Mode::Select { .. } | Mode::Multiplayer(_)
                )
            }) && self.remote.is_none()
                && self.player.is_none()
                && self.integrated.is_none(),
            playing: self.title.is_none()
                && match &self.remote {
                    Some(remote) => remote.logged_in && remote.reconfiguring.is_none(),
                    None => self.player.is_some(),
                },
            busy,
        }
    }

    pub(super) fn native_session_ready(&mut self) {
        if let Some(destination) = self.native_session.ready() {
            self.native_client_logged_in(destination.single_player, destination.address.as_deref());
        }
    }

    pub(super) fn native_session_left(&mut self) {
        if self.native_session.left() {
            self.native_client_logged_out();
        }
    }

    pub(super) fn native_disconnect_claimed(
        &mut self,
        reason: &str,
        key: Option<&str>,
        from_server: bool,
    ) -> bool {
        disconnect_claimed(
            reason,
            key,
            from_server,
            |reason, key, from_server, claimed| {
                self.native_client_disconnected(reason, key, from_server, claimed);
            },
        )
    }

    pub(super) fn native_run_session_request(&mut self) {
        let context = self.native_request_context();
        let Some(request) = self.native_session.take_request(context) else {
            return;
        };
        let result = match request {
            Request::OpenWorld(folder) => self.open_world(&folder, None, None, false, None),
            Request::Connect(address) => {
                let name = self.options.player_name();
                self.connect_server_with_options(&address, &name, None, false)
            }
            Request::Leave => self.close_world(),
        };
        if let Err(error) = result {
            if let Some(title) = &mut self.title {
                title.failed(error.to_string());
            }
            self.capture(false);
        }
    }

    pub(super) fn native_finish_session_shutdown(&mut self) {
        let Some(logged_in) = self.native_session.shutdown() else {
            return;
        };
        if logged_in {
            self.native_client_logged_out();
        }
        self.native_client_shutdown();
    }
}

pub(super) fn dispatch_client_logged_in(
    game: &mut Game<'_>,
    single_player: bool,
    address: Option<&str>,
) {
    if !game.options.execution_role.is_client() {
        return;
    }
    let mods = crate::mod_api::mods();
    if !mods.is_empty() {
        crate::mod_api::dispatch_client_logged_in(game, mods, single_player, address);
    }
}

pub(super) fn dispatch_client_logged_out(game: &mut Game<'_>) {
    if !game.options.execution_role.is_client() {
        return;
    }
    let mods = crate::mod_api::mods();
    if !mods.is_empty() {
        crate::mod_api::dispatch_client_logged_out(game, mods);
    }
}

pub(super) fn dispatch_client_disconnected(
    game: &mut Game<'_>,
    reason: &str,
    key: Option<&str>,
    from_server: bool,
    claimed: &mut bool,
) {
    if !game.options.execution_role.is_client() {
        return;
    }
    let mods = crate::mod_api::mods();
    if !mods.is_empty() {
        crate::mod_api::dispatch_client_disconnected(game, mods, reason, key, from_server, claimed);
    }
}

pub(super) fn request_open_world(game: &mut Game<'_>, folder: &str) -> bool {
    if !world_folder(folder) {
        return false;
    }
    let context = game.native_request_context();
    game.native_session
        .enqueue(Request::OpenWorld(folder.to_owned()), context)
}

pub(super) fn request_connect(game: &mut Game<'_>, address: &str) -> bool {
    if address.trim().is_empty() {
        return false;
    }
    let context = game.native_request_context();
    game.native_session
        .enqueue(Request::Connect(address.to_owned()), context)
}

pub(super) fn request_leave(game: &mut Game<'_>) -> bool {
    let context = game.native_request_context();
    game.native_session.enqueue(Request::Leave, context)
}

pub(super) fn dispatch_client_shutdown(game: &mut Game<'_>) {
    if !game.options.execution_role.is_client() {
        return;
    }
    let mods = crate::mod_api::mods();
    if !mods.is_empty() {
        crate::mod_api::dispatch_client_shutdown(game, mods);
    }
}

// CHECK: These tests are authored only; run them after applying the integration patch.
#[cfg(test)]
mod tests {
    use super::*;

    fn title_context() -> RequestContext {
        RequestContext {
            client: true,
            title: true,
            ..Default::default()
        }
    }

    fn playing_context() -> RequestContext {
        RequestContext {
            client: true,
            playing: true,
            ..Default::default()
        }
    }

    #[test]
    fn no_mod_keeps_disconnect_unclaimed_and_has_no_requests() {
        assert!(!disconnect_claimed(
            "Disconnected",
            None,
            false,
            |_, _, _, _| {}
        ));
        let mut session = NativeSession::default();
        assert_eq!(session.take_request(title_context()), None);
        assert_eq!(session.take_request(playing_context()), None);
    }

    #[test]
    fn mod_claim_receives_the_original_disconnect_context() {
        assert!(disconnect_claimed(
            "Server closed",
            Some("disconnect.closed"),
            true,
            |reason, key, from_server, claimed| {
                assert_eq!(reason, "Server closed");
                assert_eq!(key, Some("disconnect.closed"));
                assert!(from_server);
                assert!(!*claimed);
                *claimed = true;
            },
        ));
    }

    #[test]
    fn login_waits_for_play_and_reconfiguration_does_not_repeat_it() {
        let mut session = NativeSession::default();
        session.connecting(false, Some("example.test:25565"));
        assert!(!session.logged_in);
        assert_eq!(
            session.ready(),
            Some(Connection {
                single_player: false,
                address: Some("example.test:25565".to_owned()),
            })
        );
        assert_eq!(session.ready(), None);
        assert!(session.left());
        assert!(!session.left());
        session.finish_leave();
        assert_eq!(session.ready(), None);
    }

    #[test]
    fn transfer_balances_logout_and_a_new_local_login() {
        let mut session = NativeSession::default();
        session.connecting(false, Some("first.test"));
        assert!(session.ready().is_some());
        assert!(session.left());
        session.finish_leave();
        session.connecting(true, None);
        assert_eq!(
            session.ready(),
            Some(Connection {
                single_player: true,
                address: None,
            })
        );
        assert!(session.left());
    }

    #[test]
    fn failed_login_has_no_logout_notification() {
        let mut session = NativeSession::default();
        session.connecting(false, Some("unavailable.test"));
        assert!(!session.left());
        assert_eq!(session.ready(), None);
    }

    #[test]
    fn mod_requests_are_queued_one_at_a_time_without_rewriting_the_destination() {
        let mut session = NativeSession::default();
        let request = Request::Connect("example.test:25566".to_owned());
        assert!(session.enqueue(request, title_context()));
        assert!(!session.enqueue(
            Request::OpenWorld("Other world".to_owned()),
            title_context()
        ));
        assert_eq!(
            session.take_request(title_context()),
            Some(Request::Connect("example.test:25566".to_owned()))
        );
        assert!(session.enqueue(Request::OpenWorld("Next world".to_owned()), title_context()));
        assert_eq!(
            session.take_request(title_context()),
            Some(Request::OpenWorld("Next world".to_owned()))
        );
        assert_eq!(session.take_request(title_context()), None);
    }

    #[test]
    fn requests_reject_wrong_roles_busy_or_wrong_screens() {
        let mut session = NativeSession::default();
        for context in [
            RequestContext::default(),
            RequestContext {
                client: false,
                ..title_context()
            },
            RequestContext {
                busy: true,
                ..title_context()
            },
            playing_context(),
        ] {
            assert!(!session.enqueue(Request::Connect("example.test".to_owned()), context));
            assert!(!session.enqueue(Request::OpenWorld("World".to_owned()), context));
        }
        assert!(!session.enqueue(Request::Leave, title_context()));
        assert!(session.enqueue(Request::Leave, playing_context()));
        assert_eq!(
            session.take_request(playing_context()),
            Some(Request::Leave)
        );
    }

    #[test]
    fn stale_requests_are_dropped_instead_of_running_in_another_state() {
        let mut session = NativeSession::default();
        assert!(session.enqueue(Request::OpenWorld("World".to_owned()), title_context()));
        assert_eq!(session.take_request(playing_context()), None);
        assert_eq!(session.take_request(title_context()), None);
        assert!(session.enqueue(Request::Leave, playing_context()));
        assert!(session.ready().is_none());
        session.left();
        assert!(!session.enqueue(Request::Leave, playing_context()));
        session.finish_leave();
        assert_eq!(session.take_request(playing_context()), None);
    }

    #[test]
    fn world_requests_use_a_save_basename_not_a_path() {
        for folder in ["World", "My world", "世界", ".hidden-world"] {
            assert!(world_folder(folder));
        }
        for folder in [
            "",
            " ",
            ".",
            "..",
            "../World",
            "World/Other",
            "World\\Other",
            "/World",
        ] {
            assert!(!world_folder(folder));
        }
    }

    #[test]
    fn shutdown_is_once_only_and_clears_pending_work_after_logout() {
        let mut session = NativeSession::default();
        session.connecting(true, None);
        assert!(session.ready().is_some());
        assert!(session.enqueue(Request::Leave, playing_context()));
        assert_eq!(session.shutdown(), Some(true));
        assert_eq!(session.shutdown(), None);
        assert!(!session.left());
        assert_eq!(session.take_request(playing_context()), None);
        session.finish_leave();
        assert!(!session.enqueue(Request::Connect("example.test".to_owned()), title_context()));
        let mut title_session = NativeSession::default();
        assert_eq!(title_session.shutdown(), Some(false));
    }
}
