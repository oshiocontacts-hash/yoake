use std::collections::HashMap;
use std::fmt;
use std::io::{self, Read, Write};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use horizon_friends::social::{
    GrantError, GrantVerifier, HostTokenVerifier, RegisterSession, SignedGrant, VerifiedGrant,
};

const BUFFER_BYTES: usize = 16 * 1024;

#[derive(Clone, Copy, Debug)]
pub struct SessionLimits {
    pub max_connections: usize,
    pub max_nonces: usize,
    pub bytes_per_second: u64,
    pub burst_bytes: usize,
    pub idle_timeout: Duration,
}

impl SessionLimits {
    fn valid(self) -> bool {
        self.max_connections > 0
            && self.max_nonces > 0
            && self.bytes_per_second > 0
            && self.burst_bytes > 0
            && !self.idle_timeout.is_zero()
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum RelayError {
    Grant(GrantError),
    InvalidLimits,
    SessionExists,
    SessionLimit,
    UnknownSession,
    ConnectionLimit,
    WrongToken,
    PairingMismatch,
    TimedOut,
    Unavailable,
}

impl fmt::Display for RelayError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Grant(error) => write!(formatter, "{error}"),
            Self::InvalidLimits => formatter.write_str("invalid social relay limits"),
            Self::SessionExists => formatter.write_str("social session ID was already registered"),
            Self::SessionLimit => formatter.write_str("social relay session capacity reached"),
            Self::UnknownSession => formatter.write_str("social session is unavailable"),
            Self::ConnectionLimit => formatter.write_str("social session connection capacity reached"),
            Self::WrongToken => formatter.write_str("social host authentication failed"),
            Self::PairingMismatch => formatter.write_str("social relay pairing does not match"),
            Self::TimedOut => formatter.write_str("social relay connection timed out"),
            Self::Unavailable => formatter.write_str("social relay is unavailable"),
        }
    }
}

impl std::error::Error for RelayError {}

impl From<GrantError> for RelayError {
    fn from(error: GrantError) -> Self {
        Self::Grant(error)
    }
}

struct Slot {
    id: u64,
    paired: bool,
    last_activity: Instant,
}

struct Session {
    host_principal: String,
    token: HostTokenVerifier,
    limits: SessionLimits,
    active: bool,
    used: HashMap<[u8; 32], u64>,
    slots: HashMap<[u8; 32], Slot>,
    next_slot_id: u64,
    tokens: f64,
    last_refill: Instant,
}

impl Session {
    fn prune(&mut self, unix_time: u64, now: Instant) {
        self.used.retain(|_, expires| *expires > unix_time);
        self.slots.retain(|_, slot| {
            now.checked_duration_since(slot.last_activity)
                .unwrap_or_default()
                < self.limits.idle_timeout
        });
    }
}

#[derive(Clone)]
pub struct SocialRelay {
    verifier: Arc<GrantVerifier>,
    sessions: Arc<Mutex<HashMap<String, Session>>>,
    max_sessions: usize,
}

impl SocialRelay {
    pub fn new(pinned_portal_key: [u8; 32], max_sessions: usize) -> Self {
        Self {
            verifier: Arc::new(GrantVerifier::new(pinned_portal_key)),
            sessions: Arc::new(Mutex::new(HashMap::new())),
            max_sessions,
        }
    }

    pub fn provision(
        &self,
        request: &RegisterSession,
        host_token: &str,
        limits: SessionLimits,
        now: Instant,
    ) -> Result<(), RelayError> {
        request.validate()?;
        if !limits.valid() {
            return Err(RelayError::InvalidLimits);
        }
        let token = HostTokenVerifier::new(host_token)?;
        let mut sessions = self.sessions.lock().map_err(|_| RelayError::Unavailable)?;
        if sessions.contains_key(&request.session_id) {
            return Err(RelayError::SessionExists);
        }
        if sessions.len() >= self.max_sessions {
            return Err(RelayError::SessionLimit);
        }
        sessions.insert(
            request.session_id.clone(),
            Session {
                host_principal: request.host_principal.clone(),
                token,
                limits,
                active: true,
                used: HashMap::new(),
                slots: HashMap::new(),
                next_slot_id: 0,
                tokens: limits.burst_bytes as f64,
                last_refill: now,
            },
        );
        Ok(())
    }

    pub fn retire(&self, session_id: &str) -> Result<(), RelayError> {
        let mut sessions = self.sessions.lock().map_err(|_| RelayError::Unavailable)?;
        let session = sessions.get_mut(session_id).ok_or(RelayError::UnknownSession)?;
        session.active = false;
        session.slots.clear();
        Ok(())
    }

    pub fn begin_guest<GUEST>(
        &self,
        signed: &SignedGrant,
        guest: GUEST,
        unix_time: u64,
        now: Instant,
    ) -> Result<PendingGuest<GUEST>, RelayError> {
        let mut sessions = self.sessions.lock().map_err(|_| RelayError::Unavailable)?;
        let session = sessions
            .get_mut(&signed.grant.session_id)
            .filter(|session| session.active)
            .ok_or(RelayError::UnknownSession)?;
        let verified = self.verifier.verify_at(
            signed,
            &signed.grant.session_id,
            &session.host_principal,
            unix_time,
        )?;
        session.prune(unix_time, now);
        let nonce = verified.nonce();
        if session.used.contains_key(&nonce) || session.slots.contains_key(&nonce) {
            return Err(RelayError::Grant(GrantError::Replay));
        }
        if session.used.len() >= session.limits.max_nonces {
            return Err(RelayError::Grant(GrantError::ReplayCacheFull));
        }
        if session.slots.len() >= session.limits.max_connections {
            return Err(RelayError::ConnectionLimit);
        }
        let slot_id = session.next_slot_id.checked_add(1).ok_or(RelayError::Unavailable)?;
        session.next_slot_id = slot_id;
        session.used.insert(nonce, verified.expires_at());
        session.slots.insert(nonce, Slot { id: slot_id, paired: false, last_activity: now });
        Ok(PendingGuest {
            relay: self.clone(),
            signed: signed.clone(),
            verified,
            slot_id,
            guest: Some(guest),
            armed: true,
        })
    }
}

pub struct PendingGuest<GUEST> {
    relay: SocialRelay,
    signed: SignedGrant,
    verified: VerifiedGrant,
    slot_id: u64,
    guest: Option<GUEST>,
    armed: bool,
}

impl<GUEST> PendingGuest<GUEST> {
    pub fn verified_grant(&self) -> &VerifiedGrant {
        &self.verified
    }

    pub fn pair<HOST>(
        &mut self,
        signed: &SignedGrant,
        host_token: &str,
        host: HOST,
        unix_time: u64,
        now: Instant,
    ) -> Result<PairedConnection<GUEST, HOST>, RelayError> {
        if !self.armed || *signed != self.signed {
            return Err(RelayError::PairingMismatch);
        }
        let mut sessions = self.relay.sessions.lock().map_err(|_| RelayError::Unavailable)?;
        let session = sessions
            .get_mut(self.verified.session_id())
            .filter(|session| session.active)
            .ok_or(RelayError::UnknownSession)?;
        self.relay.verifier.verify_at(
            signed,
            self.verified.session_id(),
            &session.host_principal,
            unix_time,
        )?;
        if !session.token.verify(host_token) {
            return Err(RelayError::WrongToken);
        }
        session.prune(unix_time, now);
        let slot = session.slots.get_mut(&self.verified.nonce())
            .filter(|slot| slot.id == self.slot_id)
            .ok_or(RelayError::TimedOut)?;
        if slot.paired {
            return Err(RelayError::PairingMismatch);
        }
        let guest = self.guest.take().ok_or(RelayError::PairingMismatch)?;
        slot.paired = true;
        slot.last_activity = now;
        self.armed = false;
        Ok(PairedConnection {
            relay: self.relay.clone(),
            verified: self.verified.clone(),
            slot_id: self.slot_id,
            guest,
            host,
            guest_to_host: Buffer::new(),
            host_to_guest: Buffer::new(),
        })
    }
}

impl<GUEST> Drop for PendingGuest<GUEST> {
    fn drop(&mut self) {
        if self.armed {
            release_slot(&self.relay, &self.verified, self.slot_id);
        }
    }
}

struct Buffer {
    bytes: Vec<u8>,
    offset: usize,
}

impl Buffer {
    fn new() -> Self {
        Self { bytes: Vec::new(), offset: 0 }
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum Direction {
    GuestToHost,
    HostToGuest,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum ForwardProgress {
    Transferred(usize),
    Blocked,
    Eof,
}

#[derive(Debug)]
pub enum ForwardError {
    Relay(RelayError),
    Io(io::Error),
}

pub struct PairedConnection<GUEST, HOST> {
    relay: SocialRelay,
    verified: VerifiedGrant,
    slot_id: u64,
    guest: GUEST,
    host: HOST,
    guest_to_host: Buffer,
    host_to_guest: Buffer,
}

impl<GUEST: Read + Write, HOST: Read + Write> PairedConnection<GUEST, HOST> {
    pub fn verified_grant(&self) -> &VerifiedGrant {
        &self.verified
    }

    pub fn forward_once(
        &mut self,
        direction: Direction,
        now: Instant,
    ) -> Result<ForwardProgress, ForwardError> {
        let mut sessions = self.relay.sessions.lock().map_err(|_| ForwardError::Relay(RelayError::Unavailable))?;
        let session = sessions
            .get_mut(self.verified.session_id())
            .filter(|session| session.active)
            .ok_or(ForwardError::Relay(RelayError::UnknownSession))?;
        let nonce = self.verified.nonce();
        let slot = session.slots.get(&nonce)
            .filter(|slot| slot.paired && slot.id == self.slot_id)
            .ok_or(ForwardError::Relay(RelayError::TimedOut))?;
        if now.checked_duration_since(slot.last_activity).unwrap_or_default() >= session.limits.idle_timeout {
            session.slots.remove(&nonce);
            return Err(ForwardError::Relay(RelayError::TimedOut));
        }
        let elapsed = now.checked_duration_since(session.last_refill).unwrap_or_default();
        session.tokens = (session.tokens + elapsed.as_secs_f64() * session.limits.bytes_per_second as f64)
            .min(session.limits.burst_bytes as f64);
        session.last_refill = now.max(session.last_refill);
        let budget = (session.tokens as usize).min(BUFFER_BYTES);
        if budget == 0 {
            return Ok(ForwardProgress::Blocked);
        }
        let progress = match direction {
            Direction::GuestToHost => transfer(&mut self.guest, &mut self.host, &mut self.guest_to_host, budget)?,
            Direction::HostToGuest => transfer(&mut self.host, &mut self.guest, &mut self.host_to_guest, budget)?,
        };
        if let ForwardProgress::Transferred(written) = progress {
            session.tokens -= written as f64;
            if let Some(slot) = session.slots.get_mut(&nonce) {
                slot.last_activity = now;
            }
        }
        Ok(progress)
    }
}

fn transfer<READER: Read, WRITER: Write>(
    reader: &mut READER,
    writer: &mut WRITER,
    buffer: &mut Buffer,
    budget: usize,
) -> Result<ForwardProgress, ForwardError> {
    if buffer.bytes.is_empty() {
        buffer.bytes.resize(budget, 0);
        match reader.read(&mut buffer.bytes) {
            Ok(0) => {
                buffer.bytes.clear();
                return Ok(ForwardProgress::Eof);
            }
            Ok(read) => buffer.bytes.truncate(read),
            Err(error) if error.kind() == io::ErrorKind::WouldBlock => {
                buffer.bytes.clear();
                return Ok(ForwardProgress::Blocked);
            }
            Err(error) => {
                buffer.bytes.clear();
                return Err(ForwardError::Io(error));
            }
        }
    }
    let end = (buffer.offset + budget).min(buffer.bytes.len());
    let written = match writer.write(&buffer.bytes[buffer.offset..end]) {
        Ok(0) => return Err(ForwardError::Io(io::ErrorKind::WriteZero.into())),
        Ok(written) => written,
        Err(error) if error.kind() == io::ErrorKind::WouldBlock => {
            return Ok(ForwardProgress::Blocked);
        }
        Err(error) => return Err(ForwardError::Io(error)),
    };
    buffer.offset += written;
    if buffer.offset == buffer.bytes.len() {
        buffer.bytes.clear();
        buffer.offset = 0;
    }
    Ok(ForwardProgress::Transferred(written))
}

impl<GUEST, HOST> Drop for PairedConnection<GUEST, HOST> {
    fn drop(&mut self) {
        release_slot(&self.relay, &self.verified, self.slot_id);
    }
}

fn release_slot(relay: &SocialRelay, verified: &VerifiedGrant, slot_id: u64) {
    if let Ok(mut sessions) = relay.sessions.lock() {
        if let Some(session) = sessions.get_mut(verified.session_id()) {
            if session.slots.get(&verified.nonce()).is_some_and(|slot| slot.id == slot_id) {
                session.slots.remove(&verified.nonce());
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use horizon_friends::social::Grant;
    use ring::signature::Ed25519KeyPair;
    use std::cell::RefCell;
    use std::io::Cursor;
    use std::rc::Rc;

    struct TestStream {
        incoming: Cursor<Vec<u8>>,
        outgoing: Rc<RefCell<Vec<u8>>>,
    }

    impl TestStream {
        fn new(input: &[u8]) -> (Self, Rc<RefCell<Vec<u8>>>) {
            let outgoing = Rc::new(RefCell::new(Vec::new()));
            (Self { incoming: Cursor::new(input.to_vec()), outgoing: outgoing.clone() }, outgoing)
        }

        fn empty() -> Self {
            Self::new(&[]).0
        }
    }

    impl Read for TestStream {
        fn read(&mut self, bytes: &mut [u8]) -> io::Result<usize> {
            self.incoming.read(bytes)
        }
    }

    impl Write for TestStream {
        fn write(&mut self, bytes: &[u8]) -> io::Result<usize> {
            self.outgoing.borrow_mut().extend_from_slice(bytes);
            Ok(bytes.len())
        }

        fn flush(&mut self) -> io::Result<()> {
            Ok(())
        }
    }

    const SEED: &str = "9d61b19deffd5a60ba844af492ec2cc44449c5697b326919703bac031cae7f60";
    const PUBLIC: &str = "d75a980182b10ab7d54bfed3c964073a0ee172f3daa62325af021a68f707511a";

    fn hex<const LENGTH: usize>(value: &str) -> [u8; LENGTH] {
        let mut output = [0; LENGTH];
        for (index, byte) in output.iter_mut().enumerate() {
            *byte = u8::from_str_radix(&value[index * 2..index * 2 + 2], 16).unwrap();
        }
        output
    }

    fn signed(nonce: u8, exp: u64) -> SignedGrant {
        let grant = Grant {
            v: 1,
            session_id: "session-1".into(),
            host_principal: "member:host".into(),
            guest_principal: "guest:alice".into(),
            mc_profile: Some("profile-1".into()),
            relay_only: true,
            exp,
            nonce: format!("{nonce:02x}").repeat(32),
        };
        let key = Ed25519KeyPair::from_seed_unchecked(&hex::<32>(SEED)).unwrap();
        let signature = key.sign(&grant.signing_bytes().unwrap()).as_ref().iter()
            .map(|byte| format!("{byte:02x}")).collect();
        SignedGrant { grant, signature }
    }

    fn setup(max_connections: usize, idle_timeout: Duration, burst_bytes: usize, rate: u64) -> (SocialRelay, Instant) {
        let now = Instant::now();
        let relay = SocialRelay::new(hex::<32>(PUBLIC), 4);
        let request = RegisterSession::relay_only("session-1".into(), "member:host".into());
        relay.provision(&request, &"ab".repeat(32), SessionLimits {
            max_connections,
            max_nonces: 8,
            bytes_per_second: rate,
            burst_bytes,
            idle_timeout,
        }, now).unwrap();
        (relay, now)
    }

    #[test]
    fn pairing_requires_the_host_token_and_the_exact_guest_grant() {
        let (relay, now) = setup(2, Duration::from_secs(30), 32, 32);
        let grant = signed(1, 200);
        let (guest, guest_output) = TestStream::new(b"hello");
        let (host, host_output) = TestStream::new(b"welcome");
        let mut pending = relay.begin_guest(&grant, guest, 100, now).unwrap();
        assert_eq!(pending.verified_grant().guest_principal(), "guest:alice");
        assert!(matches!(pending.pair(&grant, &"cd".repeat(32), TestStream::empty(), 100, now), Err(RelayError::WrongToken)));
        assert!(matches!(pending.pair(&signed(2, 200), &"ab".repeat(32), TestStream::empty(), 100, now), Err(RelayError::PairingMismatch)));
        let mut pair = pending.pair(&grant, &"ab".repeat(32), host, 100, now).unwrap();
        assert_eq!(pair.verified_grant().mc_profile(), Some("profile-1"));
        assert_eq!(pair.forward_once(Direction::GuestToHost, now).unwrap(), ForwardProgress::Transferred(5));
        assert_eq!(pair.forward_once(Direction::HostToGuest, now).unwrap(), ForwardProgress::Transferred(7));
        assert_eq!(host_output.borrow().as_slice(), b"hello");
        assert_eq!(guest_output.borrow().as_slice(), b"welcome");
    }

    #[test]
    fn connection_capacity_is_released_without_allowing_grant_replay() {
        let (relay, now) = setup(1, Duration::from_secs(30), 32, 32);
        let first = signed(1, 200);
        let second = signed(2, 200);
        let mut pending = relay.begin_guest(&first, TestStream::empty(), 100, now).unwrap();
        assert!(matches!(relay.begin_guest(&second, TestStream::empty(), 100, now), Err(RelayError::ConnectionLimit)));
        let pair = pending.pair(&first, &"ab".repeat(32), TestStream::empty(), 100, now).unwrap();
        assert!(matches!(relay.begin_guest(&second, TestStream::empty(), 100, now), Err(RelayError::ConnectionLimit)));
        drop(pair);
        assert!(matches!(relay.begin_guest(&first, TestStream::empty(), 100, now), Err(RelayError::Grant(GrantError::Replay))));
        assert!(relay.begin_guest(&second, TestStream::empty(), 100, now).is_ok());
    }

    #[test]
    fn idle_pending_and_paired_connections_expire() {
        let (relay, now) = setup(1, Duration::from_secs(5), 32, 32);
        let first = signed(1, 200);
        let second = signed(2, 200);
        let mut stale = relay.begin_guest(&first, TestStream::empty(), 100, now).unwrap();
        let later = now + Duration::from_secs(6);
        let (guest, _) = TestStream::new(b"late");
        let (host, host_output) = TestStream::new(&[]);
        let mut pending = relay.begin_guest(&second, guest, 106, later).unwrap();
        assert!(matches!(stale.pair(&first, &"ab".repeat(32), TestStream::empty(), 106, later), Err(RelayError::TimedOut)));
        let mut pair = pending.pair(&second, &"ab".repeat(32), host, 106, later).unwrap();
        assert!(matches!(pair.forward_once(Direction::GuestToHost, later + Duration::from_secs(5)), Err(ForwardError::Relay(RelayError::TimedOut))));
        assert!(host_output.borrow().is_empty());
        assert!(relay.begin_guest(&signed(3, 200), TestStream::empty(), 111, later + Duration::from_secs(5)).is_ok());
    }

    #[test]
    fn bandwidth_budget_is_shared_across_directions() {
        let (relay, now) = setup(1, Duration::from_secs(30), 4, 4);
        let grant = signed(1, 200);
        let (guest, guest_output) = TestStream::new(b"1234");
        let (host, host_output) = TestStream::new(b"abcd");
        let mut pending = relay.begin_guest(&grant, guest, 100, now).unwrap();
        let mut pair = pending.pair(&grant, &"ab".repeat(32), host, 100, now).unwrap();
        assert_eq!(pair.forward_once(Direction::GuestToHost, now).unwrap(), ForwardProgress::Transferred(4));
        assert_eq!(pair.forward_once(Direction::HostToGuest, now).unwrap(), ForwardProgress::Blocked);
        assert_eq!(pair.forward_once(Direction::HostToGuest, now + Duration::from_secs(1)).unwrap(), ForwardProgress::Transferred(4));
        assert_eq!(host_output.borrow().as_slice(), b"1234");
        assert_eq!(guest_output.borrow().as_slice(), b"abcd");
    }

    #[test]
    fn stale_handle_cannot_remove_a_reused_slot() {
        let (relay, now) = setup(1, Duration::from_secs(5), 32, 32);
        let stale = relay.begin_guest(&signed(1, 105), TestStream::empty(), 100, now).unwrap();
        let later = now + Duration::from_secs(6);
        let fresh_grant = signed(1, 200);
        let mut fresh = relay.begin_guest(&fresh_grant, TestStream::empty(), 106, later).unwrap();
        drop(stale);
        assert!(fresh.pair(&fresh_grant, &"ab".repeat(32), TestStream::empty(), 106, later).is_ok());
    }
}
