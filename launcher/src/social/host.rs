//! The host's room while the game runs: the launcher carries the guests' grants to the game.
//!
//! The relay joins two people when the guest's grant reaches it and the host's game has handed it
//! the same grant too. The game cannot ask the portal for the grants, so the launcher does:
//! every [`POLL`] it reads `GET /sessions/:id/pending`, writes each grant as one JSON line to the
//! game's stdin (`{"type":"pair",...}`) and acknowledges what was written. Shortly before the
//! room's time is up it asks the portal for a new ticket and writes `{"type":"reprovision",...}`.
//! A closed room gets `{"type":"end"}` and then the portal's DELETE; a game that ended (or whose
//! stdin cannot be written) gets the DELETE alone.
//!
//! The game's stdin is written by a thread of its own, through a small queue: a game that does
//! not read it can stall that thread, never the polling, the closing or the DELETE. A write that
//! has not finished within [`STALL`] means the game is gone.
//!
//! Nothing here is logged or put in an error: a line holds a grant, a signature or a ticket.
use super::rooms::{end_line, is_signature, pair_line, reprovision_line, PendingJoin, Provision};
use super::{ApiCode, SocialBackend, SocialError};
use serde_json::Value;
use std::collections::HashMap;
use std::io::Write;
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use std::sync::mpsc::{self, RecvTimeoutError, Sender, SyncSender, TrySendError};
use std::sync::{Arc, Mutex, OnceLock};
use std::time::{Duration, Instant};

/// How often the portal is asked for guests, when it answers.
pub const POLL: Duration = Duration::from_secs(2);
/// After errors the wait doubles (2, 4, 8, 16, 30 seconds) up to this; an answer resets it.
pub const MAX_POLL: Duration = Duration::from_secs(30);
/// How long before the room's time is up a new ticket is asked for (or half of what is left, when
/// the room has less).
pub const EXTEND_BEFORE: i64 = 10 * 60;
/// Two asks for a new ticket are never closer than this (the portal allows 10 a minute).
pub const MIN_EXTEND: Duration = Duration::from_secs(30);
/// Without a time to plan from, the next extension is tried after this long.
const EXTEND_FALLBACK: Duration = Duration::from_secs(5 * 60);
/// A write to the game's stdin that takes this long means the game does not read it any more.
pub const STALL: Duration = Duration::from_secs(5);
/// How long the launcher waits for the room to close (the `end` line and the DELETE).
pub const CLOSE_WAIT: Duration = Duration::from_secs(3);
/// Lines waiting for the game's stdin.
const QUEUE: usize = 16;
/// Guests taken from one answer of the portal.
pub const MAX_JOINS: usize = 32;
/// A `pair` line is at most this long (the game refuses a bigger one).
pub const MAX_LINE: usize = 64 * 1024;
/// A guest's name, in characters, as shown.
pub const MAX_NAME: usize = 64;

/// What the page is told.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Note {
    /// A guest's grant went to the game: their name.
    Guest(String),
    /// The game is gone (or cannot be written to, or the portal has no such room) and the room
    /// was closed on the portal.
    Ended,
}

// ----- The game's stdin -------------------------------------------------------------------

/// What became of a line given to the game's stdin.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Queued {
    Yes,
    /// The queue is full: try again later.
    Full,
    Broken,
}

struct Sink {
    lines: SyncSender<String>,
    failed: Arc<AtomicBool>,
    queued: Arc<AtomicUsize>,
    busy_since: Arc<Mutex<Option<Instant>>>,
    stall: Duration,
}

impl Sink {
    fn new<W: Write + Send + 'static>(mut out: W, stall: Duration) -> Self {
        let (lines, inbox) = mpsc::sync_channel::<String>(QUEUE);
        let failed = Arc::new(AtomicBool::new(false));
        let queued = Arc::new(AtomicUsize::new(0));
        let busy_since = Arc::new(Mutex::new(None));
        let (f, q, b) = (failed.clone(), queued.clone(), busy_since.clone());
        std::thread::spawn(move || {
            for line in inbox {
                if let Ok(mut busy) = b.lock() {
                    *busy = Some(Instant::now());
                }
                let ok = out
                    .write_all(line.as_bytes())
                    .and_then(|()| out.write_all(b"\n"))
                    .and_then(|()| out.flush())
                    .is_ok();
                if let Ok(mut busy) = b.lock() {
                    *busy = None;
                }
                q.fetch_sub(1, Ordering::SeqCst);
                if !ok {
                    f.store(true, Ordering::SeqCst);
                    break;
                }
            }
        });
        Self { lines, failed, queued, busy_since, stall }
    }

    /// Whether the game cannot be written to: a write failed, or one has been stuck too long.
    fn broken(&self, now: Instant) -> bool {
        self.failed.load(Ordering::SeqCst)
            || self
                .busy_since
                .lock()
                .ok()
                .and_then(|busy| *busy)
                .is_some_and(|since| now.saturating_duration_since(since) >= self.stall)
    }

    /// Queues a line. A full queue is not a broken game: the line is left for a later try, and
    /// only [`STALL`] (a write that does not finish) says the game is gone.
    fn send(&self, line: String) -> Queued {
        if self.broken(Instant::now()) {
            return Queued::Broken;
        }
        self.queued.fetch_add(1, Ordering::SeqCst);
        match self.lines.try_send(line) {
            Ok(()) => Queued::Yes,
            Err(TrySendError::Full(_)) => {
                self.queued.fetch_sub(1, Ordering::SeqCst);
                Queued::Full
            }
            Err(TrySendError::Disconnected(_)) => {
                self.queued.fetch_sub(1, Ordering::SeqCst);
                self.failed.store(true, Ordering::SeqCst);
                Queued::Broken
            }
        }
    }

    /// Waits (a little) until what was queued has been written.
    fn drain(&self, wait: Duration) {
        let until = Instant::now() + wait;
        while self.queued.load(Ordering::SeqCst) > 0
            && !self.failed.load(Ordering::SeqCst)
            && Instant::now() < until
        {
            std::thread::sleep(Duration::from_millis(10));
        }
    }
}

// ----- One room --------------------------------------------------------------------------

/// A grant handed to the game: until when it matters, and whether the portal knows.
struct Seen {
    expires_at: i64,
    acked: bool,
}

pub struct Host {
    api: Arc<dyn SocialBackend>,
    session_id: String,
    sink: Sink,
    interval: Duration,
    /// When the next ticket is asked for.
    extend_due: Instant,
    /// The grants written to the game, by nonce, until they lapse: not written twice, and not
    /// acknowledged again once acknowledged.
    seen: HashMap<String, Seen>,
    ack_wait: Duration,
    ack_retry_at: Instant,
    /// The portal has no such room (any more).
    gone: bool,
}

/// When to ask for the next ticket, in unix seconds, for a room that lasts to `expires_at`.
fn extend_time(expires_at: i64, now: i64) -> i64 {
    let left = (expires_at - now).max(0);
    expires_at - EXTEND_BEFORE.min(left / 2)
}

/// The room's end as the ticket says it (`session_exp`, unix seconds; milliseconds are read too).
fn session_exp(provision: &Provision) -> Option<i64> {
    let at = provision.ticket.expose().get("session_exp")?.as_i64()?;
    Some(if at > 100_000_000_000 { at / 1000 } else { at })
}

/// The portal says the room is closed already, or the sign-in is gone.
fn is_gone(error: SocialError) -> bool {
    matches!(
        error,
        SocialError::SignedOut | SocialError::NotAvailable | SocialError::Api(ApiCode::NotFound)
    )
}

/// A room that is closed already is as good as one just closed.
pub fn closed_ok(result: super::Result<()>) -> super::Result<()> {
    match result {
        Err(SocialError::Api(ApiCode::NotFound)) => Ok(()),
        other => other,
    }
}

/// A guest's name as shown: no control characters, at most [`MAX_NAME`] characters.
fn clean_name(name: &str) -> String {
    let cleaned: String = name.chars().filter(|c| !c.is_control()).take(MAX_NAME).collect();
    let cleaned = cleaned.trim();
    if cleaned.is_empty() {
        "?".to_string()
    } else {
        cleaned.to_string()
    }
}

impl Host {
    /// `out` is the game's stdin; the room lasts to `expires_at` (unix seconds).
    pub fn new<W: Write + Send + 'static>(
        api: Arc<dyn SocialBackend>,
        session_id: String,
        out: W,
        expires_at: i64,
        now_unix: i64,
    ) -> Self {
        Self::with_stall(api, session_id, out, expires_at, now_unix, STALL)
    }

    pub fn with_stall<W: Write + Send + 'static>(
        api: Arc<dyn SocialBackend>,
        session_id: String,
        out: W,
        expires_at: i64,
        now_unix: i64,
        stall: Duration,
    ) -> Self {
        let wait = (extend_time(expires_at, now_unix) - now_unix).max(0) as u64;
        let now = Instant::now();
        Self {
            api,
            session_id,
            sink: Sink::new(out, stall),
            interval: POLL,
            extend_due: now + Duration::from_secs(wait),
            seen: HashMap::new(),
            ack_wait: POLL,
            ack_retry_at: now,
            gone: false,
        }
    }

    /// The wait until the next poll.
    pub fn interval(&self) -> Duration {
        self.interval
    }

    pub fn extend_due(&self) -> Instant {
        self.extend_due
    }

    /// The game cannot be written to: treat it as ended.
    pub fn broken(&self, now: Instant) -> bool {
        self.sink.broken(now)
    }

    /// The portal has no such room any more.
    pub fn gone(&self) -> bool {
        self.gone
    }

    fn note_error(&mut self, error: SocialError) {
        if is_gone(error) {
            self.gone = true;
        }
    }

    /// Whether a guest's grant is one to hand to the game: it is for this room, has not lapsed,
    /// has a signature of the right shape, and its line is not too big.
    fn acceptable(&self, join: &PendingJoin, now_unix: i64) -> bool {
        join.expires_at > now_unix
            && is_signature(join.signature.expose())
            && join.grant.expose().expose().get("session_id").and_then(Value::as_str)
                == Some(self.session_id.as_str())
            && pair_line(join).len() <= MAX_LINE
    }

    /// One round: reads the pending guests, hands each grant to the game, acknowledges. Returns
    /// the names of the guests handed over. A join that does not fit is dropped and not
    /// acknowledged.
    pub fn poll(&mut self, now: Instant, now_unix: i64) -> Vec<String> {
        self.seen.retain(|_, seen| seen.expires_at > now_unix);
        let mut names = Vec::new();
        let joins = match self.api.pending_joins(&self.session_id) {
            Ok(joins) => joins,
            Err(error) => {
                self.note_error(error);
                self.interval = (self.interval * 2).min(MAX_POLL);
                return names;
            }
        };
        self.interval = POLL;
        // The limit comes after the check: a bad join in front cannot crowd out a good one.
        let fresh: Vec<&PendingJoin> = joins
            .iter()
            .filter(|join| {
                self.acceptable(join, now_unix) && !self.seen.contains_key(join.grant.expose().nonce())
            })
            .take(MAX_JOINS)
            .collect();
        for join in fresh {
            let nonce = join.grant.expose().nonce();
            // A full queue: this round goes no further. The rest is neither remembered nor
            // acknowledged, and comes again with the next answer.
            if self.sink.send(pair_line(join)) != Queued::Yes {
                break;
            }
            self.seen.insert(nonce.to_string(), Seen { expires_at: join.expires_at, acked: false });
            names.push(clean_name(&join.guest.name));
        }
        self.acknowledge(now);
        names
    }

    /// Tells the portal which grants went to the game. A failure only delays this again (the
    /// wait doubles up to [`MAX_POLL`]); polling goes on at its own pace.
    fn acknowledge(&mut self, now: Instant) {
        let unacked: Vec<String> =
            self.seen.iter().filter(|(_, seen)| !seen.acked).map(|(n, _)| n.clone()).collect();
        if unacked.is_empty() || now < self.ack_retry_at {
            return;
        }
        match self.api.ack_joins(&self.session_id, &unacked) {
            Ok(()) => {
                for nonce in &unacked {
                    if let Some(seen) = self.seen.get_mut(nonce) {
                        seen.acked = true;
                    }
                }
                self.ack_wait = POLL;
            }
            Err(error) => {
                self.note_error(error);
                self.ack_retry_at = now + self.ack_wait;
                self.ack_wait = (self.ack_wait * 2).min(MAX_POLL);
            }
        }
    }

    /// When the extension is due, asks for a new ticket and hands it to the game. The next one
    /// is planned from the ticket's `session_exp`, never sooner than [`MIN_EXTEND`] from now.
    pub fn extend_if_due(&mut self, now: Instant, now_unix: i64) {
        if now < self.extend_due || self.broken(now) {
            return;
        }
        let wait = match self.api.reprovision_room(&self.session_id) {
            Ok(answer) => {
                // A line the game could not take now (the queue is full) is not lost: the
                // ticket is asked for again after the minimum wait.
                if self.sink.send(reprovision_line(&answer.provision)) == Queued::Full {
                    self.extend_due = now + MIN_EXTEND;
                    return;
                }
                match session_exp(&answer.provision).or(answer.expires_at) {
                    Some(at) => {
                        Duration::from_secs((extend_time(at, now_unix) - now_unix).max(0) as u64)
                    }
                    None => EXTEND_FALLBACK,
                }
            }
            Err(error) => {
                self.note_error(error);
                MIN_EXTEND
            }
        };
        self.extend_due = now + wait.max(MIN_EXTEND);
    }

    /// The player closes the room: the game is told, then the portal.
    pub fn close(&mut self) -> super::Result<()> {
        if self.sink.send(end_line()) == Queued::Yes {
            self.sink.drain(Duration::from_secs(1));
        }
        closed_ok(self.api.close_room(&self.session_id))
    }

    /// The game is gone: the portal's room goes too.
    pub fn game_ended(&mut self) -> super::Result<()> {
        closed_ok(self.api.close_room(&self.session_id))
    }
}

// ----- The driver ------------------------------------------------------------------------

enum Command {
    Close(Sender<super::Result<()>>),
    GameEnded,
}

/// What the launcher keeps of a running host driver.
#[derive(Clone)]
pub struct HostHandle {
    commands: Sender<Command>,
}

impl HostHandle {
    /// Closes the room: `end` to the game, then DELETE. Waits [`CLOSE_WAIT`] for it, so run it
    /// off the UI thread. `None`: the driver is gone or did not answer in time, and the caller
    /// closes the room itself (a room that is closed already is not an error: [`closed_ok`]).
    pub fn close(&self) -> Option<super::Result<()>> {
        self.close_within(CLOSE_WAIT)
    }

    pub fn close_within(&self, wait: Duration) -> Option<super::Result<()>> {
        self.ask_close()?.recv_timeout(wait).ok()
    }

    /// Asks the driver to close the room; the answer comes on the receiver.
    fn ask_close(&self) -> Option<mpsc::Receiver<super::Result<()>>> {
        let (reply, answer) = mpsc::channel();
        self.commands.send(Command::Close(reply)).ok()?;
        Some(answer)
    }

    /// The game ended: stop polling and delete the room.
    pub fn game_ended(&self) {
        let _ = self.commands.send(Command::GameEnded);
    }
}

/// The drivers running, for the launcher's own quit. Each driver takes its entry out when it
/// ends, so nothing here keeps one alive: a driver stops only by `close` or `game_ended`.
#[derive(Default)]
pub struct Registry {
    entries: Mutex<Vec<(u64, HostHandle)>>,
    next: AtomicUsize,
}

impl Registry {
    fn add(&self, handle: HostHandle) -> u64 {
        let id = self.next.fetch_add(1, Ordering::SeqCst) as u64;
        if let Ok(mut entries) = self.entries.lock() {
            entries.push((id, handle));
        }
        id
    }

    fn remove(&self, id: u64) {
        if let Ok(mut entries) = self.entries.lock() {
            entries.retain(|(entry, _)| *entry != id);
        }
    }

    #[cfg(test)]
    pub fn len(&self) -> usize {
        self.entries.lock().map_or(0, |entries| entries.len())
    }

    /// Closes every room here, for up to [`CLOSE_WAIT`] in all: every driver is asked first, and
    /// the answers are waited for until one common deadline.
    pub fn close_all(&self) {
        self.close_all_within(CLOSE_WAIT);
    }

    fn close_all_within(&self, wait: Duration) {
        let all: Vec<_> = self
            .entries
            .lock()
            .map(|mut entries| entries.drain(..).collect())
            .unwrap_or_default();
        let deadline = Instant::now() + wait;
        let asked: Vec<_> = all.iter().filter_map(|(_, handle)| handle.ask_close()).collect();
        for answer in asked {
            let _ = answer.recv_timeout(deadline.saturating_duration_since(Instant::now()));
        }
    }
}

fn active() -> Arc<Registry> {
    static ACTIVE: OnceLock<Arc<Registry>> = OnceLock::new();
    ACTIVE.get_or_init(Default::default).clone()
}

/// The launcher is quitting, or the sign-in is about to go: the hosted room (if any) is closed,
/// for up to [`CLOSE_WAIT`].
pub fn close_on_quit() {
    active().close_all();
}

/// Takes the driver's entry out of its registry when the driver's thread ends.
struct Unregister(Arc<Registry>, u64);

impl Drop for Unregister {
    fn drop(&mut self) {
        self.0.remove(self.1);
    }
}

fn unix_now() -> i64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map_or(0, |d| d.as_secs() as i64)
}

/// Runs `host` on its own thread until the room is closed or the game ends. `notify` is called
/// from that thread.
pub fn spawn(host: Host, notify: impl Fn(Note) + Send + 'static) -> HostHandle {
    spawn_in(active(), host, notify)
}

/// [`spawn`], listed in `registry` (the launcher has one; tests bring their own).
pub fn spawn_in(
    registry: Arc<Registry>,
    mut host: Host,
    notify: impl Fn(Note) + Send + 'static,
) -> HostHandle {
    let (commands, inbox) = mpsc::channel();
    let handle = HostHandle { commands };
    let entry = Unregister(registry.clone(), registry.add(handle.clone()));
    std::thread::spawn(move || {
        let _entry = entry;
        let mut next_poll = Instant::now();
        loop {
            let now = Instant::now();
            // Short enough to see a stalled stdin in time.
            let wait = next_poll
                .saturating_duration_since(now)
                .min(host.extend_due().saturating_duration_since(now))
                .min(Duration::from_millis(500));
            match inbox.recv_timeout(wait) {
                Ok(Command::Close(reply)) => {
                    let _ = reply.send(host.close());
                    return;
                }
                Ok(Command::GameEnded) | Err(RecvTimeoutError::Disconnected) => {
                    let _ = host.game_ended();
                    notify(Note::Ended);
                    return;
                }
                Err(RecvTimeoutError::Timeout) => {}
            }
            let now = Instant::now();
            if now >= next_poll {
                for name in host.poll(now, unix_now()) {
                    notify(Note::Guest(name));
                }
                next_poll = Instant::now() + host.interval();
            }
            host.extend_if_due(Instant::now(), unix_now());
            if host.broken(Instant::now()) || host.gone() {
                // The game's stdin is dead, or the portal has no such room: it is over.
                let _ = host.game_ended();
                notify(Note::Ended);
                return;
            }
        }
    });
    handle
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::social::rooms::Audience;
    use crate::social_mock::MockSocial;
    use serde_json::json;

    #[derive(Clone, Default)]
    struct Lines(Arc<Mutex<Vec<u8>>>);

    impl Write for Lines {
        fn write(&mut self, bytes: &[u8]) -> std::io::Result<usize> {
            self.0.lock().unwrap().extend_from_slice(bytes);
            Ok(bytes.len())
        }
        fn flush(&mut self) -> std::io::Result<()> {
            Ok(())
        }
    }

    impl Lines {
        fn raw(&self) -> String {
            String::from_utf8(self.0.lock().unwrap().clone()).unwrap()
        }
        fn json(&self) -> Vec<Value> {
            self.raw().lines().map(|line| serde_json::from_str(line).unwrap()).collect()
        }
    }

    struct Closed;
    impl Write for Closed {
        fn write(&mut self, _: &[u8]) -> std::io::Result<usize> {
            Err(std::io::ErrorKind::BrokenPipe.into())
        }
        fn flush(&mut self) -> std::io::Result<()> {
            Ok(())
        }
    }

    /// A game that never reads its stdin: the first write never returns.
    struct Stalled;
    impl Write for Stalled {
        fn write(&mut self, _: &[u8]) -> std::io::Result<usize> {
            loop {
                std::thread::park();
            }
        }
        fn flush(&mut self) -> std::io::Result<()> {
            Ok(())
        }
    }

    /// [`super::spawn`] with a registry of its own: tests that quit or sign out elsewhere in
    /// parallel do not close these rooms.
    fn spawn(host: Host, notify: impl Fn(Note) + Send + 'static) -> HostHandle {
        spawn_in(Arc::new(Registry::default()), host, notify)
    }

    fn hosted() -> (Arc<MockSocial>, String, i64) {
        let mock = Arc::new(MockSocial::blank());
        let room = mock.create_room(&Audience::Friends, None).unwrap();
        (mock, room.session_id, room.expires_at)
    }

    fn host_of<W: Write + Send + 'static>(
        mock: &Arc<MockSocial>,
        id: &str,
        out: W,
        expires: i64,
        now: i64,
    ) -> Host {
        Host::new(mock.clone(), id.into(), out, expires, now)
    }

    fn poll(host: &mut Host) -> Vec<String> {
        host.poll(Instant::now(), unix_now())
    }

    fn settle(host: &Host) {
        host.sink.drain(Duration::from_secs(2));
    }

    #[test]
    fn a_guest_who_joins_is_written_as_a_pair_line_and_acknowledged() {
        let (mock, id, expires) = hosted();
        let lines = Lines::default();
        let mut host = host_of(&mock, &id, lines.clone(), expires, 0);
        assert!(poll(&mut host).is_empty());
        let nonce = mock.guest_joins(&id, "g1", "Mika");
        assert_eq!(mock.pending_joins(&id).unwrap().len(), 1);
        assert_eq!(poll(&mut host), vec!["Mika".to_string()]);
        settle(&host);
        let written = lines.json();
        assert_eq!(written.len(), 1);
        assert_eq!(written[0]["type"], "pair");
        assert_eq!(written[0]["grant"]["grant"]["nonce"], nonce);
        assert_eq!(written[0]["grant"]["signature"], "ab".repeat(64));
        assert_eq!(written[0].as_object().unwrap().len(), 2);
        // Acknowledged: gone from the portal's list, and not written again.
        assert!(mock.pending_joins(&id).unwrap().is_empty());
        assert!(poll(&mut host).is_empty());
        assert_eq!(lines.json().len(), 1);
        assert!(lines.raw().ends_with('\n'));
    }

    #[test]
    fn a_failed_acknowledgement_is_sent_again_without_writing_the_pair_twice() {
        let (mock, id, expires) = hosted();
        let lines = Lines::default();
        let mut host = host_of(&mock, &id, lines.clone(), expires, 0);
        let nonce = mock.guest_joins(&id, "g1", "Mika");
        mock.fail_ack_once();
        let now = Instant::now();
        // The pair is written; the ack gets a 500, so the portal still lists the guest. Polling
        // itself goes on at its own pace.
        assert_eq!(host.poll(now, unix_now()), vec!["Mika".to_string()]);
        settle(&host);
        assert_eq!(lines.json().len(), 1);
        assert_eq!(mock.pending_joins(&id).unwrap().len(), 1);
        assert_eq!(host.interval(), POLL);
        // Not before the ack's own wait is over.
        assert!(host.poll(now + Duration::from_secs(1), unix_now()).is_empty());
        assert_eq!(mock.pending_joins(&id).unwrap().len(), 1);
        // Then only the ack is sent again: nothing is written, and the guest is gone.
        assert!(host.poll(now + Duration::from_secs(3), unix_now()).is_empty());
        settle(&host);
        assert_eq!(lines.json().len(), 1);
        assert_eq!(lines.json()[0]["grant"]["grant"]["nonce"], nonce);
        assert!(mock.pending_joins(&id).unwrap().is_empty());
        assert_eq!(host.interval(), POLL);
        assert!(host.poll(now + Duration::from_secs(9), unix_now()).is_empty());
        assert_eq!(lines.json().len(), 1);
    }

    #[test]
    fn an_acknowledged_grant_is_remembered_until_it_lapses() {
        let (mock, id, expires) = hosted();
        let lines = Lines::default();
        let mut host = host_of(&mock, &id, lines.clone(), expires, 0);
        let nonce = mock.guest_joins(&id, "g1", "Mika");
        assert_eq!(poll(&mut host).len(), 1);
        // The portal lists the same grant again (a stale answer): it is not written again.
        mock.guest_joins_custom(
            &id,
            json!({ "nonce": nonce, "session_id": id }),
            &"ab".repeat(64),
            "Mika",
            unix_now() + 240,
        );
        assert!(poll(&mut host).is_empty());
        settle(&host);
        assert_eq!(lines.json().len(), 1);
    }

    #[test]
    fn the_wait_grows_on_errors_and_comes_back() {
        let (mock, id, expires) = hosted();
        let mut host = host_of(&mock, &id, Lines::default(), expires, 0);
        assert_eq!(host.interval(), Duration::from_secs(2));
        let mut seen = Vec::new();
        for _ in 0..6 {
            mock.fail_next(SocialError::Offline);
            poll(&mut host);
            seen.push(host.interval().as_secs());
        }
        assert_eq!(seen, [4, 8, 16, 30, 30, 30]);
        assert!(!host.gone());
        poll(&mut host);
        assert_eq!(host.interval(), POLL);
    }

    #[test]
    fn joins_that_do_not_fit_are_dropped_and_never_acknowledged() {
        let (mock, id, expires) = hosted();
        let lines = Lines::default();
        let mut host = host_of(&mock, &id, lines.clone(), expires, 0);
        let sig = "ab".repeat(64);
        let soon = unix_now() + 240;
        // Too big a line, a signature of the wrong shape, lapsed, another room's.
        mock.guest_joins_custom(
            &id,
            json!({ "session_id": id, "pad": "x".repeat(MAX_LINE) }),
            &sig,
            "Big",
            soon,
        );
        mock.guest_joins_custom(&id, json!({ "session_id": id }), "SHORT", "BadSig", soon);
        mock.guest_joins_custom(&id, json!({ "session_id": id }), &"AB".repeat(64), "Upper", soon);
        mock.guest_joins_custom(&id, json!({ "session_id": id }), &sig, "Late", unix_now() - 1);
        mock.guest_joins_custom(&id, json!({ "session_id": "other-room" }), &sig, "Other", soon);
        mock.guest_joins_custom(&id, json!({}), &sig, "NoRoom", soon);
        // One that fits, with a name full of control characters and far too long.
        let name = format!("\u{7}Mi\u{1b}ka{}", "z".repeat(200));
        mock.guest_joins_custom(&id, json!({ "session_id": id }), &sig, &name, soon);
        let names = poll(&mut host);
        assert_eq!(names.len(), 1);
        assert!(names[0].chars().count() <= MAX_NAME && !names[0].chars().any(char::is_control));
        assert!(names[0].starts_with("Mika"), "{}", names[0]);
        settle(&host);
        assert_eq!(lines.json().len(), 1);
        // The six dropped ones are still the portal's: only the good one was acknowledged.
        assert_eq!(mock.pending_joins(&id).unwrap().len(), 6);
    }

    /// A game that reads its stdin only once the gate is opened.
    #[derive(Clone, Default)]
    struct Gated {
        open: Arc<(Mutex<bool>, std::sync::Condvar)>,
        lines: Lines,
    }

    impl Write for Gated {
        fn write(&mut self, bytes: &[u8]) -> std::io::Result<usize> {
            let (flag, signal) = &*self.open;
            let mut open = flag.lock().unwrap();
            while !*open {
                open = signal.wait(open).unwrap();
            }
            drop(open);
            self.lines.write(bytes)
        }
        fn flush(&mut self) -> std::io::Result<()> {
            Ok(())
        }
    }

    impl Gated {
        fn open(&self) {
            *self.open.0.lock().unwrap() = true;
            self.open.1.notify_all();
        }
    }

    #[test]
    fn forty_guests_are_all_handed_over_over_a_few_rounds_and_the_room_stays() {
        let (mock, id, expires) = hosted();
        let lines = Lines::default();
        let mut host = host_of(&mock, &id, lines.clone(), expires, 0);
        for n in 0..40 {
            mock.guest_joins(&id, &format!("g{n}"), &format!("Guest {n}"));
        }
        let mut handed = 0;
        for _ in 0..40 {
            let names = poll(&mut host);
            assert!(names.len() <= MAX_JOINS);
            handed += names.len();
            if mock.pending_joins(&id).unwrap().is_empty() {
                break;
            }
            std::thread::sleep(Duration::from_millis(20));
        }
        assert_eq!(handed, 40);
        settle(&host);
        assert!(!host.broken(Instant::now()));
        assert!(mock.pending_joins(&id).unwrap().is_empty());
        assert_eq!(mock.rooms().unwrap().len(), 1);
        let written = lines.json();
        let mut nonces: Vec<_> =
            written.iter().map(|line| line["grant"]["grant"]["nonce"].to_string()).collect();
        nonces.sort();
        nonces.dedup();
        assert_eq!((written.len(), nonces.len()), (40, 40));
    }

    #[test]
    fn a_full_queue_is_not_a_dead_game() {
        let (mock, id, expires) = hosted();
        let game = Gated::default();
        let host = Host::with_stall(
            mock.clone(),
            id.clone(),
            game.clone(),
            expires,
            0,
            Duration::from_secs(60),
        );
        let mut host = host;
        for n in 0..40 {
            mock.guest_joins(&id, &format!("g{n}"), &format!("Guest {n}"));
        }
        // The game reads nothing: its first line is stuck in the write, 16 wait in the queue,
        // and the rest is left for later.
        let handed = poll(&mut host).len();
        assert!((16..=17).contains(&handed), "{handed}");
        assert!(!host.broken(Instant::now()));
        let left = mock.pending_joins(&id).unwrap().len();
        assert_eq!(left, 40 - handed);
        assert_eq!(mock.rooms().unwrap().len(), 1);
        // Once the game reads again, the rest comes over, each once.
        game.open();
        let mut total = handed;
        for _ in 0..60 {
            total += poll(&mut host).len();
            if mock.pending_joins(&id).unwrap().is_empty() {
                break;
            }
            std::thread::sleep(Duration::from_millis(20));
        }
        assert_eq!(total, 40);
        settle(&host);
        assert_eq!(game.lines.json().len(), 40);
        assert!(!host.broken(Instant::now()));
    }

    #[test]
    fn a_reprovision_line_is_not_lost_to_a_full_queue() {
        let (mock, id, expires) = hosted();
        let game = Gated::default();
        let mut host = Host::with_stall(
            mock.clone(),
            id.clone(),
            game.clone(),
            expires,
            0,
            // The test's clock jumps far ahead: a stall limit that cannot be reached by that.
            Duration::from_secs(1 << 40),
        );
        for n in 0..40 {
            mock.guest_joins(&id, &format!("g{n}"), &format!("Guest {n}"));
        }
        poll(&mut host);
        assert!(!host.broken(Instant::now()));
        // The queue is full: the ticket's line cannot go. It is asked for again later.
        let due = host.extend_due();
        host.extend_if_due(due, unix_now());
        assert_eq!(host.extend_due(), due + MIN_EXTEND);
        assert!(!host.broken(Instant::now()));
        game.open();
        settle(&host);
        let reprovisions =
            |game: &Gated| game.lines.json().iter().filter(|l| l["type"] == "reprovision").count();
        assert_eq!(reprovisions(&game), 0);
        // The next chance: the line is written, once.
        host.extend_if_due(due + MIN_EXTEND + Duration::from_secs(1), unix_now());
        settle(&host);
        assert_eq!(reprovisions(&game), 1);
        assert_eq!(game.lines.json().last().unwrap()["type"], "reprovision");
    }

    /// A portal that answers nothing for a DELETE, for this long.
    struct Hanging;

    impl crate::social::Transport for Hanging {
        fn call(
            &self,
            method: crate::social::Method,
            _: &[&str],
            _: Option<Value>,
        ) -> Result<crate::social::Reply, crate::social::TransportError> {
            if method == crate::social::Method::Delete {
                std::thread::sleep(Duration::from_secs(3));
            }
            Ok(crate::social::Reply { status: 200, body: json!({ "joins": [] }) })
        }
    }

    #[test]
    fn closing_all_waits_once_for_all_the_drivers_together() {
        let registry = Arc::new(Registry::default());
        for n in 0..3 {
            let api = Arc::new(crate::social::HttpSocial::new(Hanging));
            let host = Host::new(api, format!("s{n}"), Lines::default(), unix_now() + 3600, unix_now());
            let _ = spawn_in(registry.clone(), host, |_| {});
        }
        assert_eq!(registry.len(), 3);
        let started = Instant::now();
        registry.close_all_within(Duration::from_millis(700));
        // One shared deadline: about 0.7 s, not 3 x 0.7 s.
        assert!(started.elapsed() < Duration::from_millis(1400), "{:?}", started.elapsed());
        assert_eq!(registry.len(), 0);
    }

    /// Records each call and whether the saved sign-in still stood when it was made.
    struct Watching {
        token: std::path::PathBuf,
        calls: Arc<Mutex<Vec<(crate::social::Method, String, bool)>>>,
    }

    impl crate::social::Transport for Watching {
        fn call(
            &self,
            method: crate::social::Method,
            segments: &[&str],
            _: Option<Value>,
        ) -> Result<crate::social::Reply, crate::social::TransportError> {
            self.calls.lock().unwrap().push((method, segments.join("/"), self.token.exists()));
            Ok(crate::social::Reply { status: 200, body: json!({ "joins": [] }) })
        }
    }

    #[test]
    fn signing_out_closes_the_hosted_room_while_the_token_still_stands() {
        crate::yoake_auth::set_before_change(close_on_quit);
        let mut random = [0_u8; 8];
        getrandom::fill(&mut random).unwrap();
        let dir = std::env::temp_dir().join(format!(
            "yoake-host-signout-{}",
            random.iter().map(|b| format!("{b:02x}")).collect::<String>()
        ));
        let account = crate::yoake_auth::YoakeAccount {
            kind: "friend".into(),
            expires_at: json!("2027-01-01T00:00:00Z"),
        };
        crate::yoake_auth::save_credentials(&dir, "secret-token", &account).unwrap();
        let calls = Arc::new(Mutex::new(Vec::new()));
        let api = Arc::new(crate::social::HttpSocial::new(Watching {
            token: dir.join("yoake-token"),
            calls: calls.clone(),
        }));
        let lines = Lines::default();
        let host = Host::new(api, "s1".into(), lines.clone(), unix_now() + 3600, unix_now());
        // The launcher's own registry, as the page's driver is in it.
        let _handle = super::spawn(host, |_| {});
        crate::yoake_auth::logout(&dir).unwrap();
        assert!(!dir.join("yoake-token").exists());
        let calls = calls.lock().unwrap();
        let delete = calls
            .iter()
            .find(|(method, path, _)| *method == crate::social::Method::Delete && path == "sessions/s1")
            .expect("the room was deleted");
        // The sign-in was still there when the DELETE went out.
        assert!(delete.2);
        assert_eq!(lines.json().last().unwrap()["type"], "end");
        let _ = std::fs::remove_dir_all(dir);
    }

    #[test]
    fn bad_joins_in_front_do_not_crowd_out_good_ones() {
        let (mock, id, expires) = hosted();
        let mut host = host_of(&mock, &id, Lines::default(), expires, 0);
        for _ in 0..MAX_JOINS + 5 {
            mock.guest_joins_custom(&id, json!({ "session_id": "elsewhere" }), &"ab".repeat(64), "Bad", unix_now() + 240);
        }
        mock.guest_joins(&id, "g", "Good");
        assert_eq!(poll(&mut host), vec!["Good".to_string()]);
    }

    #[test]
    fn a_driver_that_ends_leaves_the_registry_and_is_not_kept_alive_by_it() {
        let (mock, id, expires) = hosted();
        let registry = Arc::new(Registry::default());
        let (tx, rx) = mpsc::channel();
        let handle = spawn_in(
            registry.clone(),
            host_of(&mock, &id, Lines::default(), expires, unix_now()),
            move |note| {
                let _ = tx.send(note);
            },
        );
        assert_eq!(registry.len(), 1);
        // Dropping the handle does not stop it (the registry's copy lives): `game_ended` does.
        let copy = handle.clone();
        drop(handle);
        assert!(rx.recv_timeout(Duration::from_millis(1500)).is_err());
        copy.game_ended();
        assert_eq!(rx.recv_timeout(Duration::from_secs(10)).unwrap(), Note::Ended);
        let until = Instant::now() + Duration::from_secs(5);
        while registry.len() > 0 && Instant::now() < until {
            std::thread::sleep(Duration::from_millis(20));
        }
        assert_eq!(registry.len(), 0);
        assert!(mock.rooms().unwrap().is_empty());
    }

    #[test]
    fn the_extension_is_planned_from_the_tickets_session_exp() {
        let (mock, id, _) = hosted();
        let lines = Lines::default();
        // A room with an hour and a half left: ten minutes before the end, i.e. in 80 minutes.
        let mut host = host_of(&mock, &id, lines.clone(), 10_000, 4_600);
        let wait = host.extend_due().saturating_duration_since(Instant::now()).as_secs();
        assert!((4_790..=4_800).contains(&wait), "{wait}");
        // With less than 20 minutes left, half of what is left.
        let short = host_of(&mock, &id, Lines::default(), 10_000, 9_000);
        let wait = short.extend_due().saturating_duration_since(Instant::now()).as_secs();
        assert!((499..=500).contains(&wait), "{wait}");
        // Not due yet: nothing happens.
        host.extend_if_due(Instant::now(), unix_now());
        settle(&host);
        assert!(lines.json().is_empty());
        // Due: a new ticket is asked for and written.
        let due = host.extend_due();
        host.extend_if_due(due, unix_now());
        settle(&host);
        let written = lines.json();
        assert_eq!(written.len(), 1);
        assert_eq!(written[0]["type"], "reprovision");
        assert_eq!(written[0]["provision"]["signature"], "mock-signature");
        // The mock's ticket says four hours from the room's opening: far off.
        assert!(host.extend_due() > due + Duration::from_secs(3_600));
    }

    #[test]
    fn a_clock_that_is_off_cannot_make_the_extension_run_every_second() {
        let (mock, id, _) = hosted();
        let lines = Lines::default();
        let mut host = host_of(&mock, &id, lines.clone(), 100, 0);
        // The ticket's session_exp is long past: still never closer than 30 seconds.
        mock.set_session_exp(&id, 50);
        let due = host.extend_due();
        host.extend_if_due(due, unix_now());
        settle(&host);
        assert_eq!(lines.json().len(), 1);
        assert_eq!(host.extend_due(), due + MIN_EXTEND);
        // Ten seconds later: not asked again. Thirty-one: asked.
        host.extend_if_due(due + Duration::from_secs(10), unix_now());
        settle(&host);
        assert_eq!(lines.json().len(), 1);
        host.extend_if_due(due + Duration::from_secs(31), unix_now());
        settle(&host);
        assert_eq!(lines.json().len(), 2);

        // A failed one writes nothing and is tried again no sooner than 30 seconds.
        let lines2 = Lines::default();
        let mut late = host_of(&mock, &id, lines2.clone(), 100, 0);
        let due = late.extend_due();
        mock.fail_next(SocialError::Offline);
        late.extend_if_due(due, unix_now());
        assert!(lines2.json().is_empty());
        assert_eq!(late.extend_due(), due + MIN_EXTEND);
    }

    #[test]
    fn a_room_the_portal_does_not_know_ends_the_host() {
        let (mock, id, expires) = hosted();
        let mut host = host_of(&mock, &id, Lines::default(), expires, 0);
        mock.close_room(&id).unwrap();
        assert!(!host.gone());
        poll(&mut host);
        assert!(host.gone());
        // And a ticket for a room that is gone says so too.
        let (mock, id, _) = hosted();
        let mut host = host_of(&mock, &id, Lines::default(), 100, 0);
        mock.close_room(&id).unwrap();
        let due = host.extend_due();
        host.extend_if_due(due, unix_now());
        assert!(host.gone());
        // The driver stops by itself, and says so.
        let (mock, id, expires) = hosted();
        let (tx, rx) = mpsc::channel();
        let _handle = spawn(host_of(&mock, &id, Lines::default(), expires, unix_now()), move |note| {
            let _ = tx.send(note);
        });
        mock.close_room(&id).unwrap();
        assert_eq!(rx.recv_timeout(Duration::from_secs(10)).unwrap(), Note::Ended);
    }

    #[test]
    fn closing_writes_end_and_deletes_the_room() {
        let (mock, id, expires) = hosted();
        let lines = Lines::default();
        let mut host = host_of(&mock, &id, lines.clone(), expires, 0);
        host.close().unwrap();
        assert_eq!(lines.json(), vec![json!({ "type": "end" })]);
        assert!(mock.rooms().unwrap().is_empty());
        // A room closed already is not an error.
        assert_eq!(host.game_ended(), Ok(()));
    }

    #[test]
    fn a_game_that_cannot_be_written_to_is_an_ended_game() {
        let (mock, id, expires) = hosted();
        let mut host = host_of(&mock, &id, Closed, expires, 0);
        mock.guest_joins(&id, "g1", "Mika");
        poll(&mut host);
        std::thread::sleep(Duration::from_millis(300));
        assert!(host.broken(Instant::now()));
        host.game_ended().unwrap();
        assert!(mock.rooms().unwrap().is_empty());
    }

    #[test]
    fn a_game_that_does_not_read_its_stdin_cannot_stop_the_room_from_closing() {
        let (mock, id, expires) = hosted();
        let host = Host::with_stall(
            mock.clone(),
            id.clone(),
            Stalled,
            expires,
            unix_now(),
            Duration::from_secs(60),
        );
        let handle = spawn(host, |_| {});
        mock.guest_joins(&id, "g1", "Mika");
        std::thread::sleep(Duration::from_millis(1500));
        let started = Instant::now();
        assert_eq!(handle.close(), Some(Ok(())));
        assert!(started.elapsed() < CLOSE_WAIT, "{:?}", started.elapsed());
        assert!(mock.rooms().unwrap().is_empty());
    }

    #[test]
    fn a_stuck_write_is_a_dead_game_and_the_room_is_deleted() {
        let (mock, id, expires) = hosted();
        let (tx, rx) = mpsc::channel();
        let host = Host::with_stall(
            mock.clone(),
            id.clone(),
            Stalled,
            expires,
            unix_now(),
            Duration::from_millis(600),
        );
        let _handle = spawn(host, move |note| {
            let _ = tx.send(note);
        });
        mock.guest_joins(&id, "g1", "Mika");
        let mut last = None;
        while let Ok(note) = rx.recv_timeout(Duration::from_secs(10)) {
            let ended = note == Note::Ended;
            last = Some(note);
            if ended {
                break;
            }
        }
        assert_eq!(last, Some(Note::Ended));
        assert!(mock.rooms().unwrap().is_empty());
    }

    #[test]
    fn the_driver_stops_and_deletes_when_the_game_ends() {
        let (mock, id, expires) = hosted();
        let lines = Lines::default();
        let (tx, rx) = mpsc::channel();
        let handle = spawn(host_of(&mock, &id, lines.clone(), expires, unix_now()), move |note| {
            let _ = tx.send(note);
        });
        mock.guest_joins(&id, "g1", "Mika");
        assert_eq!(rx.recv_timeout(Duration::from_secs(10)).unwrap(), Note::Guest("Mika".into()));
        handle.game_ended();
        assert_eq!(rx.recv_timeout(Duration::from_secs(10)).unwrap(), Note::Ended);
        assert!(mock.rooms().unwrap().is_empty());
        // Gone: it cannot be asked to close, and nothing more is written.
        assert!(handle.close().is_none());
        assert_eq!(lines.json().len(), 1);
    }

    #[test]
    fn the_driver_closes_a_room_on_request() {
        let (mock, id, expires) = hosted();
        let lines = Lines::default();
        let handle = spawn(host_of(&mock, &id, lines.clone(), expires, unix_now()), |_| {});
        assert_eq!(handle.close(), Some(Ok(())));
        assert_eq!(lines.json().last().unwrap()["type"], "end");
        assert!(mock.rooms().unwrap().is_empty());
    }

    #[test]
    fn quitting_the_launcher_closes_the_hosted_room() {
        let (mock, id, expires) = hosted();
        let lines = Lines::default();
        let registry = Arc::new(Registry::default());
        let handle = spawn_in(
            registry.clone(),
            host_of(&mock, &id, lines.clone(), expires, unix_now()),
            |_| {},
        );
        assert_eq!(registry.len(), 1);
        registry.close_all();
        assert_eq!(registry.len(), 0);
        let _ = handle;
        assert_eq!(lines.json().last().unwrap()["type"], "end");
        assert!(mock.rooms().unwrap().is_empty());
    }

    #[test]
    fn no_secret_is_in_a_debug_print() {
        let (mock, id, _) = hosted();
        mock.guest_joins(&id, "g1", "Mika");
        let joins = mock.pending_joins(&id).unwrap();
        let shown = format!("{joins:?} {:?}", Note::Guest("Mika".into()));
        assert!(!shown.contains(&"ab".repeat(64)) && !shown.contains("\"session_id\""), "{shown}");
        assert!(shown.contains("<redacted>"));
    }
}
