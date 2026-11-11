use super::Game;
use std::{
    collections::{HashMap, VecDeque},
    sync::{
        Arc, Mutex,
        atomic::{AtomicBool, AtomicU64, AtomicUsize, Ordering},
    },
};

use super::native_lan::NativeLanPublish;

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct LanTicket(u64);

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum LanOperationStatus {
    Pending,
    Done { port: Option<u16> },
    Failed(String),
    Unknown,
}

#[derive(Clone, Copy)]
enum LanOperation {
    Publish(NativeLanPublish),
    Close,
}

#[derive(Default)]
struct LanRequests {
    available: bool,
    queue: VecDeque<(LanTicket, LanOperation)>,
    statuses: HashMap<LanTicket, LanOperationStatus>,
}

static NEXT_LAN_TICKET: AtomicU64 = AtomicU64::new(1);

#[derive(Default)]
pub(super) struct ClipboardRequests {
    queue: VecDeque<String>,
}

impl ClipboardRequests {
    fn request(&mut self, available: bool, text: &str) -> bool {
        if !available || text.len() > 32 * 1024 {
            return false;
        }
        self.queue.push_back(text.to_owned());
        true
    }

    fn drain(&mut self, mut write: impl FnMut(&str)) {
        for text in self.queue.drain(..) {
            write(&text);
        }
    }
}

#[derive(Clone, Default)]
pub(super) struct DeferredRequests {
    shared: Arc<SharedRequests>,
}

#[derive(Default)]
struct SharedRequests {
    permission_refresh: AtomicBool,
    mod_screen: AtomicUsize,
    lan: Mutex<LanRequests>,
}

impl DeferredRequests {
    fn request_lan(&self, available: bool, operation: LanOperation) -> Option<LanTicket> {
        if !available {
            return None;
        }
        let mut requests = self.shared.lan.try_lock().ok()?;
        if !requests.available {
            return None;
        }
        let ticket = LanTicket(NEXT_LAN_TICKET.fetch_add(1, Ordering::Relaxed));
        requests
            .statuses
            .insert(ticket, LanOperationStatus::Pending);
        requests.queue.push_back((ticket, operation));
        Some(ticket)
    }

    pub(super) fn set_lan_available(&self, available: bool) {
        if let Ok(mut requests) = self.shared.lan.lock() {
            requests.available = available;
            if !available {
                while let Some((ticket, _)) = requests.queue.pop_front() {
                    requests.statuses.insert(
                        ticket,
                        LanOperationStatus::Failed("integrated server stopped".into()),
                    );
                }
            }
        }
    }

    fn poll_lan(&self, ticket: LanTicket) -> LanOperationStatus {
        let Ok(requests) = self.shared.lan.try_lock() else {
            return LanOperationStatus::Pending;
        };
        requests
            .statuses
            .get(&ticket)
            .cloned()
            .unwrap_or(LanOperationStatus::Unknown)
    }

    fn take_lan(&self) -> VecDeque<(LanTicket, LanOperation)> {
        self.shared.lan.try_lock().map_or_else(
            |_| VecDeque::new(),
            |mut requests| std::mem::take(&mut requests.queue),
        )
    }

    fn finish_lan(&self, ticket: LanTicket, status: LanOperationStatus) {
        if let Ok(mut requests) = self.shared.lan.lock() {
            requests.statuses.insert(ticket, status);
        }
    }

    fn complete_lan(
        &self,
        queued: VecDeque<(LanTicket, LanOperation)>,
        mut execute: impl FnMut(LanOperation) -> LanOperationStatus,
    ) {
        for (ticket, operation) in queued {
            self.finish_lan(ticket, execute(operation));
        }
    }

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
    pub(super) fn native_request_lan_publish_impl(
        &mut self,
        opts: NativeLanPublish,
    ) -> Option<LanTicket> {
        let available = self.integrated.is_some()
            || self.options.execution_role
                == super::execution_role::ExecutionRole::IntegratedServer;
        self.native_deferred
            .request_lan(available, LanOperation::Publish(opts))
    }

    pub(super) fn native_request_lan_close_impl(&mut self) -> Option<LanTicket> {
        let available = self.integrated.is_some()
            || self.options.execution_role
                == super::execution_role::ExecutionRole::IntegratedServer;
        self.native_deferred
            .request_lan(available, LanOperation::Close)
    }

    pub(super) fn native_poll_lan_operation_impl(&self, ticket: LanTicket) -> LanOperationStatus {
        self.native_deferred.poll_lan(ticket)
    }

    pub(super) fn native_request_clipboard_write_impl(&mut self, text: &str) -> bool {
        let available = self.options.execution_role == super::execution_role::ExecutionRole::Client
            && self.window.is_some();
        self.native_clipboard.request(available, text)
    }

    pub(super) fn native_drain_clipboard(&mut self) {
        use horizon_client::clipboard::Clipboard;
        let clipboard = &mut self.clipboard;
        self.native_clipboard.drain(|text| clipboard.set(text));
    }

    pub(super) fn native_drain_lan_requests(&mut self) {
        let queued = self.native_deferred.take_lan();
        if queued.is_empty() {
            return;
        }
        let deferred = self.native_deferred.clone();
        deferred.complete_lan(queued, |operation| match operation {
            LanOperation::Publish(opts) => match self.native_lan_publish_impl(&opts) {
                Ok(port) => LanOperationStatus::Done { port: Some(port) },
                Err(error) => LanOperationStatus::Failed(error),
            },
            LanOperation::Close => {
                if self.native_lan_close_impl() {
                    LanOperationStatus::Done { port: None }
                } else {
                    LanOperationStatus::Failed("LAN publication could not be closed".into())
                }
            }
        });
    }

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
    use super::{ClipboardRequests, DeferredRequests, LanOperation, LanOperationStatus, LanTicket};
    use crate::game::native_lan::NativeLanPublish;

    #[test]
    fn lan_queue_is_idle_without_requests_and_rejects_missing_server() {
        let requests = DeferredRequests::default();
        assert!(requests.take_lan().is_empty());
        assert!(requests.request_lan(false, LanOperation::Close).is_none());
        assert!(requests.request_lan(true, LanOperation::Close).is_none());
        assert!(requests.take_lan().is_empty());
        assert_eq!(requests.poll_lan(LanTicket(0)), LanOperationStatus::Unknown);
    }

    #[test]
    fn lan_tickets_are_pending_until_drained_and_completed() {
        let client = DeferredRequests::default();
        let server = client.clone();
        client.set_lan_available(true);
        let publish = server
            .request_lan(true, LanOperation::Publish(NativeLanPublish::default()))
            .unwrap();
        let close = server.request_lan(true, LanOperation::Close).unwrap();
        assert_ne!(publish, close);
        assert_eq!(client.poll_lan(publish), LanOperationStatus::Pending);
        assert_eq!(server.poll_lan(close), LanOperationStatus::Pending);
        let mut processed = Vec::new();
        client.complete_lan(client.take_lan(), |operation| match operation {
            LanOperation::Publish(_) => {
                processed.push("publish");
                LanOperationStatus::Done { port: Some(25565) }
            }
            LanOperation::Close => {
                processed.push("close");
                LanOperationStatus::Done { port: None }
            }
        });
        assert_eq!(processed, vec!["publish", "close"]);
        assert!(client.take_lan().is_empty());
        assert_eq!(
            server.poll_lan(publish),
            LanOperationStatus::Done { port: Some(25565) }
        );
        assert_eq!(
            server.poll_lan(close),
            LanOperationStatus::Done { port: None }
        );
        let failed = server.request_lan(true, LanOperation::Close).unwrap();
        client.complete_lan(client.take_lan(), |_| {
            LanOperationStatus::Failed("not published".into())
        });
        assert_eq!(
            server.poll_lan(failed),
            LanOperationStatus::Failed("not published".into())
        );
        assert_eq!(
            DeferredRequests::default().poll_lan(failed),
            LanOperationStatus::Unknown
        );
    }

    #[test]
    fn stopping_server_fails_queued_lan_requests() {
        let client = DeferredRequests::default();
        let server = client.clone();
        client.set_lan_available(true);
        let ticket = server.request_lan(true, LanOperation::Close).unwrap();
        client.set_lan_available(false);
        assert_eq!(
            server.poll_lan(ticket),
            LanOperationStatus::Failed("integrated server stopped".into())
        );
        assert!(client.take_lan().is_empty());
        assert!(server.request_lan(true, LanOperation::Close).is_none());
        client.set_lan_available(true);
        assert!(server.request_lan(true, LanOperation::Close).is_some());
    }

    #[test]
    fn clipboard_requests_enforce_utf8_limit_and_apply_once() {
        let mut requests = ClipboardRequests::default();
        let mut writes = Vec::new();
        requests.drain(|text| writes.push(text.to_owned()));
        assert!(writes.is_empty());
        assert!(!requests.request(false, "unavailable"));
        assert!(!requests.request(true, &"あ".repeat(10_923)));
        let limit = "x".repeat(32 * 1024);
        assert!(requests.request(true, &limit));
        assert!(requests.request(true, "next"));
        requests.drain(|text| writes.push(text.to_owned()));
        requests.drain(|text| writes.push(text.to_owned()));
        assert_eq!(writes, vec![limit, "next".to_owned()]);
    }

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
