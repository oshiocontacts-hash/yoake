use std::sync::{Arc, mpsc};
use std::thread;
use std::time::{Duration, Instant};

use super::Game;

const MAX_IN_FLIGHT_JOINS: usize = 4;
const JOIN_RESULT_RETENTION: Duration = Duration::from_secs(60);

/// Public account identity without credentials.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct NativeAccount {
    pub name: String,
    pub uuid: u128,
    pub online: bool,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum NativeJoinPoll {
    Pending,
    Joined,
    Failed(String),
    Unknown,
}

#[derive(Default)]
pub(super) struct NativeSessionJoins {
    last_ticket: u32,
    joins: Vec<NativeJoin>,
}

struct NativeJoin {
    ticket: u32,
    receiver: mpsc::Receiver<CompletedJoin>,
    finished: Option<CompletedJoin>,
}

struct CompletedJoin {
    completed_at: Instant,
    result: Result<(), String>,
}

fn account_identity(client: bool, account: Option<(&str, u128)>) -> Option<NativeAccount> {
    if !client {
        return None;
    }
    account.map(|(name, uuid)| NativeAccount {
        name: name.to_owned(),
        uuid,
        online: true,
    })
}

pub(super) fn account(game: &Game<'_>) -> Option<NativeAccount> {
    let auth = game.auth();
    account_identity(
        game.options.execution_role.is_client(),
        auth.as_ref().map(|auth| (auth.name.as_str(), auth.uuid)),
    )
}

impl NativeSessionJoins {
    fn start_with<Account>(
        &mut self,
        client: bool,
        account: Option<Arc<Account>>,
        server_hash: &str,
        join: impl FnOnce(&Account, &str) -> Result<(), String> + Send + 'static,
    ) -> Option<u32>
    where
        Account: Send + Sync + 'static,
    {
        if !client || server_hash.is_empty() {
            return None;
        }
        let account = account?;
        self.expire();
        if self
            .joins
            .iter()
            .filter(|join| join.finished.is_none())
            .count()
            >= MAX_IN_FLIGHT_JOINS
        {
            return None;
        }
        let ticket = self.last_ticket.checked_add(1)?;
        let (sender, receiver) = mpsc::channel();
        let server_hash = server_hash.to_owned();
        drop(
            thread::Builder::new()
                .name("native-session-join".to_owned())
                .spawn(move || {
                    let result = join(&account, &server_hash);
                    let _ = sender.send(CompletedJoin {
                        completed_at: Instant::now(),
                        result,
                    });
                })
                .ok()?,
        );
        self.last_ticket = ticket;
        self.joins.push(NativeJoin {
            ticket,
            receiver,
            finished: None,
        });
        Some(ticket)
    }

    fn poll(&mut self, ticket: u32) -> NativeJoinPoll {
        self.expire();
        let Some(index) = self.joins.iter().position(|join| join.ticket == ticket) else {
            return NativeJoinPoll::Unknown;
        };
        let Some(finished) = self.joins[index].finished.take() else {
            return NativeJoinPoll::Pending;
        };
        self.joins.swap_remove(index);
        match finished.result {
            Ok(()) => NativeJoinPoll::Joined,
            Err(message) => NativeJoinPoll::Failed(message),
        }
    }

    pub(super) fn expire(&mut self) {
        if !self.joins.is_empty() {
            self.expire_at(Instant::now());
        }
    }

    fn expire_at(&mut self, now: Instant) {
        self.joins.retain_mut(|join| {
            if join.finished.is_none() {
                match join.receiver.try_recv() {
                    Ok(finished) => join.finished = Some(finished),
                    Err(mpsc::TryRecvError::Empty) => {}
                    Err(mpsc::TryRecvError::Disconnected) => {
                        join.finished = Some(CompletedJoin {
                            completed_at: now,
                            result: Err(join_error(horizon_client::auth::JoinError::Unavailable)),
                        });
                    }
                }
            }
            join.finished.as_ref().is_none_or(|finished| {
                now.saturating_duration_since(finished.completed_at) < JOIN_RESULT_RETENTION
            })
        });
    }
}

fn join_error(error: horizon_client::auth::JoinError) -> String {
    match error.reason() {
        Ok(key) => key.to_owned(),
        Err(message) => message.to_owned(),
    }
}

// CHECK: Integrate the Game state, redraw expiry, wrappers, and NativeJoinPoll re-exports together.
pub(super) fn session_join_start(game: &mut Game<'_>, server_hash: &str) -> Option<u32> {
    let client = game.options.execution_role.is_client();
    let auth = client.then(|| game.auth()).flatten();
    game.native_session_joins
        .start_with(client, auth, server_hash, |account, server_hash| {
            horizon_client::auth::join_server(account, server_hash).map_err(join_error)
        })
}

pub(super) fn session_join_poll(game: &mut Game<'_>, ticket: u32) -> NativeJoinPoll {
    game.native_session_joins.poll(ticket)
}

// CHECK: Run these worker/poll tests through the client gameplay test target after integration.
#[cfg(test)]
mod tests {
    use super::*;

    fn wait_until_finished(joins: &mut NativeSessionJoins, ticket: u32) -> Instant {
        let deadline = Instant::now() + Duration::from_secs(5);
        loop {
            joins.expire();
            let join = joins
                .joins
                .iter()
                .find(|join| join.ticket == ticket)
                .expect("ticket must remain until taken");
            if let Some(finished) = &join.finished {
                return finished.completed_at;
            }
            assert!(Instant::now() < deadline, "join worker did not finish");
            thread::yield_now();
        }
    }

    #[test]
    fn no_mod_account_projection_does_not_invent_an_offline_account() {
        assert_eq!(account_identity(true, None), None);
        assert_eq!(account_identity(false, Some(("Owner", 7))), None);
    }

    #[test]
    fn mod_receives_only_public_online_identity() {
        assert_eq!(
            account_identity(true, Some(("Owner", 7))),
            Some(NativeAccount {
                name: "Owner".to_owned(),
                uuid: 7,
                online: true,
            })
        );
    }

    #[test]
    fn no_start_keeps_worker_state_unallocated() {
        let mut joins = NativeSessionJoins::default();
        joins.expire();
        for ticket in [0, 1, u32::MAX] {
            assert_eq!(joins.poll(ticket), NativeJoinPoll::Unknown);
        }
        assert!(joins.joins.is_empty());
        assert_eq!(joins.joins.capacity(), 0);
        assert_eq!(joins.last_ticket, 0);
    }

    #[test]
    fn rejected_starts_do_not_issue_tickets_or_allocate_workers() {
        let mut joins = NativeSessionJoins::default();
        for (client, account, server_hash) in [
            (false, Some(Arc::new(())), "hash"),
            (true, None, "hash"),
            (true, Some(Arc::new(())), ""),
        ] {
            assert_eq!(
                joins.start_with(client, account, server_hash, |_, _| {
                    panic!("unexpected join")
                }),
                None
            );
        }
        assert!(joins.joins.is_empty());
        assert_eq!(joins.joins.capacity(), 0);
        assert_eq!(joins.last_ticket, 0);
    }

    #[test]
    fn worker_results_are_pending_then_taken_once() {
        let mut joins = NativeSessionJoins::default();
        for result in [
            Ok(()),
            Err(join_error(horizon_client::auth::JoinError::InvalidSession)),
            Err("network failure".to_owned()),
        ] {
            let expected = match &result {
                Ok(()) => NativeJoinPoll::Joined,
                Err(message) => NativeJoinPoll::Failed(message.clone()),
            };
            let caller = thread::current().id();
            let (release, blocked) = mpsc::channel();
            let ticket = joins
                .start_with(
                    true,
                    Some(Arc::new(7)),
                    "-signed-hash",
                    move |account, server_hash| {
                        assert_ne!(thread::current().id(), caller);
                        assert_eq!(*account, 7);
                        assert_eq!(server_hash, "-signed-hash");
                        blocked
                            .recv_timeout(Duration::from_secs(5))
                            .expect("worker was not released");
                        result
                    },
                )
                .expect("join must start");
            assert_eq!(joins.poll(ticket), NativeJoinPoll::Pending);
            release.send(()).expect("worker must be waiting");
            wait_until_finished(&mut joins, ticket);
            assert_eq!(joins.poll(ticket), expected);
            assert_eq!(joins.poll(ticket), NativeJoinPoll::Unknown);
            assert!(joins.joins.is_empty());
        }
    }

    #[test]
    fn four_in_flight_joins_exclude_completed_unpolled_results() {
        let mut joins = NativeSessionJoins::default();
        let mut workers = Vec::new();
        for _ in 0..MAX_IN_FLIGHT_JOINS {
            let (release, blocked) = mpsc::channel();
            let ticket = joins
                .start_with(true, Some(Arc::new(())), "hash", move |_, _| {
                    blocked
                        .recv_timeout(Duration::from_secs(5))
                        .expect("worker was not released");
                    Ok(())
                })
                .expect("join must start");
            assert!(workers.iter().all(|(previous, _)| *previous != ticket));
            assert_eq!(joins.poll(ticket), NativeJoinPoll::Pending);
            workers.push((ticket, release));
        }
        assert_eq!(
            joins.start_with(true, Some(Arc::new(())), "hash", |_, _| {
                panic!("fifth in-flight join must not run")
            }),
            None
        );
        let (completed_ticket, release) = workers.remove(0);
        release.send(()).expect("worker must be waiting");
        wait_until_finished(&mut joins, completed_ticket);
        let (release, blocked) = mpsc::channel();
        let ticket = joins
            .start_with(true, Some(Arc::new(())), "hash", move |_, _| {
                blocked
                    .recv_timeout(Duration::from_secs(5))
                    .expect("worker was not released");
                Ok(())
            })
            .expect("completed unpolled result must release its slot");
        assert_ne!(ticket, completed_ticket);
        assert!(workers.iter().all(|(previous, _)| *previous != ticket));
        workers.push((ticket, release));
        assert_eq!(joins.joins.len(), MAX_IN_FLIGHT_JOINS + 1);
        assert_eq!(joins.poll(completed_ticket), NativeJoinPoll::Joined);
        assert_eq!(joins.poll(completed_ticket), NativeJoinPoll::Unknown);
        for (ticket, release) in workers {
            release.send(()).expect("worker must be waiting");
            wait_until_finished(&mut joins, ticket);
            assert_eq!(joins.poll(ticket), NativeJoinPoll::Joined);
        }
        assert!(joins.joins.is_empty());
    }

    #[test]
    fn completed_results_expire_at_sixty_seconds() {
        let mut joins = NativeSessionJoins::default();
        let ticket = joins
            .start_with(true, Some(Arc::new(())), "hash", |_, _| Ok(()))
            .expect("join must start");
        let completed_at = wait_until_finished(&mut joins, ticket);
        joins.expire_at(completed_at + JOIN_RESULT_RETENTION - Duration::from_nanos(1));
        assert_eq!(joins.joins.len(), 1);
        joins.expire_at(completed_at + JOIN_RESULT_RETENTION);
        assert!(joins.joins.is_empty());
        assert_eq!(joins.poll(ticket), NativeJoinPoll::Unknown);
    }

    #[test]
    fn unread_results_expire_from_worker_completion() {
        let mut joins = NativeSessionJoins::default();
        let ticket = joins
            .start_with(true, Some(Arc::new(())), "hash", |_, _| Ok(()))
            .expect("join must start");
        let deadline = Instant::now() + Duration::from_secs(5);
        while !joins.joins.is_empty() {
            joins.expire_at(Instant::now() + JOIN_RESULT_RETENTION);
            assert!(Instant::now() < deadline, "unread result did not expire");
            thread::yield_now();
        }
        assert_eq!(joins.poll(ticket), NativeJoinPoll::Unknown);
    }

    #[test]
    fn pending_work_does_not_use_the_result_retention_deadline() {
        let mut joins = NativeSessionJoins::default();
        let (release, blocked) = mpsc::channel();
        let ticket = joins
            .start_with(true, Some(Arc::new(())), "hash", move |_, _| {
                blocked
                    .recv_timeout(Duration::from_secs(5))
                    .expect("worker was not released");
                Ok(())
            })
            .expect("join must start");
        joins.expire_at(Instant::now() + JOIN_RESULT_RETENTION);
        assert_eq!(joins.poll(ticket), NativeJoinPoll::Pending);
        release.send(()).expect("worker must be waiting");
        wait_until_finished(&mut joins, ticket);
        assert_eq!(joins.poll(ticket), NativeJoinPoll::Joined);
    }

    #[test]
    fn session_join_returns_existing_authentication_reasons() {
        use horizon_client::auth::JoinError;

        assert_eq!(
            join_error(JoinError::InvalidSession),
            "disconnect.loginFailedInfo.invalidSession"
        );
        assert_eq!(join_error(JoinError::Other("detail".to_owned())), "detail");
    }
}
