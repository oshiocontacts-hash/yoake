use super::Game;
use std::sync::{
    Arc,
    atomic::{AtomicBool, AtomicUsize, Ordering},
};

#[derive(Clone, Default)]
pub(super) struct DeferredRequests {
    shared: Arc<SharedRequests>,
}

#[derive(Default)]
struct SharedRequests {
    permission_refresh: AtomicBool,
    mod_screen: AtomicUsize,
}

impl DeferredRequests {
    pub(super) fn request_permission_refresh(&self, server_exists: bool) -> bool {
        if server_exists {
            self.shared
                .permission_refresh
                .store(true, Ordering::Release);
        }
        server_exists
    }

    pub(super) fn request_mod_screen(&self, index: Option<usize>) -> bool {
        let Some(index) = index else {
            return false;
        };
        self.shared.mod_screen.store(index + 1, Ordering::Release);
        true
    }

    fn take_permission_refresh(&self) -> bool {
        self.shared.permission_refresh.load(Ordering::Acquire)
            && self.shared.permission_refresh.swap(false, Ordering::AcqRel)
    }

    fn take_mod_screen(&self) -> Option<usize> {
        if self.shared.mod_screen.load(Ordering::Acquire) == 0 {
            return None;
        }
        self.shared
            .mod_screen
            .swap(0, Ordering::AcqRel)
            .checked_sub(1)
    }
}

impl Game<'_> {
    pub(super) fn native_drain_permission_refresh(&mut self) {
        if !self
            .native_deferred
            .shared
            .permission_refresh
            .load(Ordering::Acquire)
        {
            return;
        }
        if self.lan.is_none() || !self.native_deferred.take_permission_refresh() {
            return;
        }
        let allow_commands = self.allow_commands();
        if let Some(host) = self.lan.as_mut() {
            if host.hub.config.integrated {
                host.hub.world_allow_commands = allow_commands;
            }
            host.hub.resend_changed_permissions();
        }
    }

    pub(super) fn native_drain_mod_screen_request(&mut self) {
        let Some(index) = self.native_deferred.take_mod_screen() else {
            return;
        };
        if let Some(entry) = horizon_protocol::mod_list::native::local().get(index) {
            self.mod_screen_open_anywhere(&entry.id);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::DeferredRequests;

    #[test]
    fn idle_and_missing_server_requests_do_nothing() {
        let requests = DeferredRequests::default();
        assert!(!requests.take_permission_refresh());
        assert_eq!(requests.take_mod_screen(), None);
        assert!(!requests.request_permission_refresh(false));
        assert!(!requests.take_permission_refresh());
    }

    #[test]
    fn permission_requests_coalesce_across_game_threads() {
        let client = DeferredRequests::default();
        let server = client.clone();
        assert!(client.request_permission_refresh(true));
        assert!(server.request_permission_refresh(true));
        assert!(server.take_permission_refresh());
        assert!(!server.take_permission_refresh());
        assert!(client.request_permission_refresh(true));
        assert!(server.take_permission_refresh());
    }

    #[test]
    fn last_loaded_mod_screen_request_wins_across_game_threads() {
        let client = DeferredRequests::default();
        let server = client.clone();
        assert!(client.request_mod_screen(Some(0)));
        assert!(server.request_mod_screen(Some(1)));
        assert!(!server.request_mod_screen(None));
        assert_eq!(client.take_mod_screen(), Some(1));
        assert_eq!(client.take_mod_screen(), None);
    }

    #[test]
    fn requests_only_run_after_callback_returns() {
        let requests = DeferredRequests::default();
        let mut effects = 0;
        let callback = |requests: &DeferredRequests| {
            assert!(requests.request_permission_refresh(true));
            assert!(requests.request_mod_screen(Some(0)));
            assert_eq!(effects, 0);
        };
        callback(&requests);
        if requests.take_permission_refresh() {
            effects += 1;
        }
        if requests.take_mod_screen().is_some() {
            effects += 1;
        }
        assert_eq!(effects, 2);
    }
}
