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
//! Nothing here is logged or put in an error: a line holds a grant, a signature or a ticket.
use super::rooms::{end_line, pair_line, reprovision_line};
use super::SocialBackend;
use std::collections::HashSet;
use std::io::Write;
use std::sync::mpsc::{self, RecvTimeoutError, Sender};
use std::sync::Arc;
use std::time::{Duration, Instant};

/// How often the portal is asked for guests, when it answers.
pub const POLL: Duration = Duration::from_secs(2);
/// After errors the wait doubles (2, 4, 8, 16, 30 seconds) up to this; an answer resets it.
pub const MAX_POLL: Duration = Duration::from_secs(30);
/// How long before the room's time is up a new ticket is asked for (or half of what is left, when
/// the room has less).
pub const EXTEND_BEFORE: i64 = 10 * 60;
/// Without an answered time, the next extension is tried after this many seconds.
const EXTEND_FALLBACK: i64 = 5 * 60;
/// After a failed extension, again after this many seconds.
const EXTEND_RETRY: i64 = 30;

/// What the page is told.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Note {
    /// A guest's grant went to the game: their name.
    Guest(String),
    /// The game is gone (or cannot be written to) and the room was closed on the portal.
    Ended,
}

pub struct Host<W: Write> {
    api: Arc<dyn SocialBackend>,
    session_id: String,
    out: W,
    interval: Duration,
    /// Unix seconds to ask for the next ticket at.
    extend_at: i64,
    /// Nonces written to the game but not acknowledged yet (so they are not written twice).
    sent: HashSet<String>,
    /// The game cannot be written to.
    broken: bool,
}

fn extend_time(expires_at: i64, now: i64) -> i64 {
    let left = (expires_at - now).max(0);
    expires_at - EXTEND_BEFORE.min(left / 2)
}

impl<W: Write> Host<W> {
    /// `out` is the game's stdin; the room lasts to `expires_at` (unix seconds).
    pub fn new(
        api: Arc<dyn SocialBackend>,
        session_id: String,
        out: W,
        expires_at: i64,
        now: i64,
    ) -> Self {
        Self {
            api,
            session_id,
            out,
            interval: POLL,
            extend_at: extend_time(expires_at, now),
            sent: HashSet::new(),
            broken: false,
        }
    }

    /// The wait until the next poll.
    pub fn interval(&self) -> Duration {
        self.interval
    }

    pub fn extend_at(&self) -> i64 {
        self.extend_at
    }

    /// The game cannot be written to: treat it as ended.
    pub fn broken(&self) -> bool {
        self.broken
    }

    fn write(&mut self, line: &str) -> bool {
        let ok = self
            .out
            .write_all(line.as_bytes())
            .and_then(|()| self.out.write_all(b"\n"))
            .and_then(|()| self.out.flush())
            .is_ok();
        if !ok {
            self.broken = true;
        }
        ok
    }

    fn fail(&mut self) {
        self.interval = (self.interval * 2).min(MAX_POLL);
    }

    /// One round: reads the pending guests, hands each grant to the game, acknowledges. Returns
    /// the names of the guests handed over.
    pub fn poll(&mut self) -> Vec<String> {
        let mut names = Vec::new();
        let joins = match self.api.pending_joins(&self.session_id) {
            Ok(joins) => joins,
            Err(_) => {
                self.fail();
                return names;
            }
        };
        let mut handed: Vec<String> = Vec::new();
        for join in &joins {
            let nonce = join.grant.expose().nonce().to_string();
            if self.sent.contains(&nonce) {
                // Written before, the acknowledgement did not get through: ask again.
                handed.push(nonce);
                continue;
            }
            if !self.write(&pair_line(join)) {
                break;
            }
            self.sent.insert(nonce.clone());
            handed.push(nonce);
            names.push(join.guest.name.clone());
        }
        if !handed.is_empty() {
            match self.api.ack_joins(&self.session_id, &handed) {
                Ok(()) => {
                    for nonce in &handed {
                        self.sent.remove(nonce);
                    }
                }
                Err(_) => {
                    self.fail();
                    return names;
                }
            }
        }
        self.interval = POLL;
        names
    }

    /// When `now` (unix seconds) has reached the extension time, asks for a new ticket and hands
    /// it to the game.
    pub fn extend_if_due(&mut self, now: i64) {
        if self.broken || now < self.extend_at {
            return;
        }
        match self.api.reprovision_room(&self.session_id) {
            Ok(answer) => {
                self.write(&reprovision_line(&answer.provision));
                self.extend_at = match answer.expires_at {
                    Some(at) => extend_time(at, now).max(now + 1),
                    None => now + EXTEND_FALLBACK,
                };
            }
            Err(_) => self.extend_at = now + EXTEND_RETRY,
        }
    }

    /// The player closes the room: the game is told, then the portal.
    pub fn close(&mut self) -> super::Result<()> {
        if !self.broken {
            self.write(&end_line());
        }
        self.api.close_room(&self.session_id)
    }

    /// The game is gone: the portal's room goes too.
    pub fn game_ended(&mut self) -> super::Result<()> {
        self.api.close_room(&self.session_id)
    }
}

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
    /// Closes the room: `end` to the game, then DELETE. Waits for the portal's answer, so run it
    /// off the UI thread. `None` when the driver is already gone (the caller closes the room).
    pub fn close(&self) -> Option<super::Result<()>> {
        let (reply, answer) = mpsc::channel();
        self.commands.send(Command::Close(reply)).ok()?;
        answer.recv().ok()
    }

    /// The game ended: stop polling and delete the room.
    pub fn game_ended(&self) {
        let _ = self.commands.send(Command::GameEnded);
    }
}

fn unix_now() -> i64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map_or(0, |d| d.as_secs() as i64)
}

/// Runs `host` on its own thread until the room is closed or the game ends. `notify` is called
/// from that thread.
pub fn spawn<W: Write + Send + 'static>(
    mut host: Host<W>,
    notify: impl Fn(Note) + Send + 'static,
) -> HostHandle {
    let (commands, inbox) = mpsc::channel();
    std::thread::spawn(move || {
        let mut next_poll = Instant::now();
        loop {
            let until_poll = next_poll.saturating_duration_since(Instant::now());
            let until_extend = Duration::from_secs((host.extend_at() - unix_now()).max(0) as u64);
            match inbox.recv_timeout(until_poll.min(until_extend)) {
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
            if Instant::now() >= next_poll {
                for name in host.poll() {
                    notify(Note::Guest(name));
                }
                next_poll = Instant::now() + host.interval();
            }
            host.extend_if_due(unix_now());
            if host.broken() {
                // The game's stdin is closed: it is gone.
                let _ = host.game_ended();
                notify(Note::Ended);
                return;
            }
        }
    });
    HostHandle { commands }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::social::rooms::Audience;
    use crate::social::SocialError;
    use crate::social_mock::MockSocial;
    use serde_json::Value;
    use std::sync::Mutex;

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

    fn hosted() -> (Arc<MockSocial>, String, i64) {
        let mock = Arc::new(MockSocial::blank());
        let room = mock.create_room(&Audience::Friends, None).unwrap();
        (mock, room.session_id, room.expires_at)
    }

    fn host_of<W: Write>(mock: &Arc<MockSocial>, id: &str, out: W, expires: i64, now: i64) -> Host<W> {
        Host::new(mock.clone(), id.into(), out, expires, now)
    }

    #[test]
    fn a_guest_who_joins_is_written_as_a_pair_line_and_acknowledged() {
        let (mock, id, expires) = hosted();
        let lines = Lines::default();
        let mut host = host_of(&mock, &id, lines.clone(), expires, 0);
        assert!(host.poll().is_empty());
        let nonce = mock.guest_joins(&id, "g1", "Mika");
        assert_eq!(mock.pending_joins(&id).unwrap().len(), 1);
        assert_eq!(host.poll(), vec!["Mika".to_string()]);
        let written = lines.json();
        assert_eq!(written.len(), 1);
        assert_eq!(written[0]["type"], "pair");
        assert_eq!(written[0]["grant"]["grant"]["nonce"], nonce);
        assert_eq!(written[0]["grant"]["signature"], "mock-signature");
        assert_eq!(written[0].as_object().unwrap().len(), 2);
        // Acknowledged: gone from the portal's list, and not written again.
        assert!(mock.pending_joins(&id).unwrap().is_empty());
        assert!(host.poll().is_empty());
        assert_eq!(lines.json().len(), 1);
        assert!(lines.raw().ends_with('\n'));
    }

    #[test]
    fn the_wait_grows_on_errors_and_comes_back() {
        let (mock, id, expires) = hosted();
        let mut host = host_of(&mock, &id, Lines::default(), expires, 0);
        assert_eq!(host.interval(), Duration::from_secs(2));
        let mut seen = Vec::new();
        for _ in 0..6 {
            mock.fail_next(SocialError::Offline);
            host.poll();
            seen.push(host.interval().as_secs());
        }
        assert_eq!(seen, [4, 8, 16, 30, 30, 30]);
        host.poll();
        assert_eq!(host.interval(), POLL);
    }

    #[test]
    fn the_extension_comes_before_the_end_and_is_planned_again() {
        let (mock, id, _) = hosted();
        // A room with an hour left: ten minutes before the end; with less, half of what is left.
        let lines = Lines::default();
        let mut host = host_of(&mock, &id, lines.clone(), 10_000, 0);
        assert_eq!(host.extend_at(), 10_000 - 600);
        assert_eq!(host_of(&mock, &id, Lines::default(), 10_000, 9_000).extend_at(), 9_500);
        host.extend_if_due(9_399);
        assert!(lines.json().is_empty());
        host.extend_if_due(9_400);
        let written = lines.json();
        assert_eq!(written.len(), 1);
        assert_eq!(written[0]["type"], "reprovision");
        assert_eq!(written[0]["provision"]["signature"], "mock-signature");
        assert!(written[0]["provision"]["ticket"].is_object());
        // The mock's room lasts four hours from its opening: the next one is planned from that.
        assert!(host.extend_at() > 9_400 + 3_600);

        // A failed one writes nothing and is tried again soon.
        let lines2 = Lines::default();
        let mut late = host_of(&mock, &id, lines2.clone(), 100, 0);
        mock.fail_next(SocialError::Offline);
        late.extend_if_due(100);
        assert!(lines2.json().is_empty());
        assert_eq!(late.extend_at(), 130);
    }

    #[test]
    fn closing_writes_end_and_deletes_the_room() {
        let (mock, id, expires) = hosted();
        let lines = Lines::default();
        let mut host = host_of(&mock, &id, lines.clone(), expires, 0);
        host.close().unwrap();
        assert_eq!(lines.json(), vec![serde_json::json!({ "type": "end" })]);
        assert!(mock.rooms().unwrap().is_empty());
    }

    #[test]
    fn a_game_that_cannot_be_written_to_is_an_ended_game() {
        let (mock, id, expires) = hosted();
        let mut host = host_of(&mock, &id, Closed, expires, 0);
        mock.guest_joins(&id, "g1", "Mika");
        host.poll();
        assert!(host.broken());
        // Nothing was acknowledged: the guest's grant is still the portal's.
        assert_eq!(mock.pending_joins(&id).unwrap().len(), 1);
        host.game_ended().unwrap();
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
    fn no_secret_is_in_a_debug_print() {
        let (mock, id, _) = hosted();
        mock.guest_joins(&id, "g1", "Mika");
        let joins = mock.pending_joins(&id).unwrap();
        let shown = format!("{joins:?} {:?}", Note::Guest("Mika".into()));
        assert!(!shown.contains("mock-signature") && !shown.contains("\"session\""), "{shown}");
        assert!(shown.contains("<redacted>"));
    }
}
