//! Forwarding between a paired guest and host.
//!
//! The sessions lock is held only to check limits and reserve budget, and again
//! to settle it; the reads and writes in between run without it.
use std::io::{self, Read, Write};
use std::time::Instant;

use horizon_friends::social::VerifiedGrant;

use super::{RelayError, SocialRelay, release_slot};

const BUFFER_BYTES: usize = 16 * 1024;

struct Buffer {
    bytes: Vec<u8>,
    offset: usize,
    /// The bandwidth for `bytes` was already taken.
    paid: bool,
}

impl Buffer {
    fn new() -> Self {
        Self { bytes: Vec::new(), offset: 0, paid: false }
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

impl<GUEST, HOST> PairedConnection<GUEST, HOST> {
    pub(super) fn new(relay: SocialRelay, verified: VerifiedGrant, slot_id: u64, guest: GUEST, host: HOST) -> Self {
        Self { relay, verified, slot_id, guest, host, guest_to_host: Buffer::new(), host_to_guest: Buffer::new() }
    }
}

impl<GUEST: Read + Write, HOST: Read + Write> PairedConnection<GUEST, HOST> {
    pub fn verified_grant(&self) -> &VerifiedGrant {
        &self.verified
    }

    /// Check the slot and refill the bucket; with `need`, take that much if it is there.
    /// Lock held, no I/O. `Ok(false)`: not enough bandwidth yet.
    fn admit(&self, now: Instant, need: Option<usize>) -> Result<bool, ForwardError> {
        let unavailable = || ForwardError::Relay(RelayError::Unavailable);
        let mut sessions = self.relay.sessions.lock().map_err(|_| unavailable())?;
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
        let Some(need) = need else { return Ok(true); };
        // `SessionLimits` require a burst of at least one maximum message, so `need` always fits.
        let need = (need as f64).min(session.limits.burst_bytes as f64);
        if session.tokens < need { return Ok(false); }
        session.tokens -= need;
        Ok(true)
    }

    fn note_activity(&self, now: Instant) {
        let Ok(mut sessions) = self.relay.sessions.lock() else { return; };
        if let Some(session) = sessions.get_mut(self.verified.session_id()) {
            if let Some(slot) = session.slots.get_mut(&self.verified.nonce()).filter(|slot| slot.id == self.slot_id) {
                slot.last_activity = now;
            }
        }
    }

    /// Move one chunk (one whole message for the TLS bridge). Nothing is reserved while
    /// waiting for input: the read comes first with no wait, and bandwidth is charged for
    /// the whole chunk only once it is in hand.
    pub fn forward_once(
        &mut self,
        direction: Direction,
        now: Instant,
    ) -> Result<ForwardProgress, ForwardError> {
        self.admit(now, None)?;
        let progress = match direction {
            Direction::GuestToHost => self.step(Side::Guest, now),
            Direction::HostToGuest => self.step(Side::Host, now),
        }?;
        if matches!(progress, ForwardProgress::Transferred(_)) { self.note_activity(now); }
        Ok(progress)
    }

    fn step(&mut self, from: Side, now: Instant) -> Result<ForwardProgress, ForwardError> {
        let buffer = match from { Side::Guest => &mut self.guest_to_host, Side::Host => &mut self.host_to_guest };
        if buffer.bytes.is_empty() {
            buffer.bytes.resize(BUFFER_BYTES, 0);
            let read = match from { Side::Guest => self.guest.read(&mut buffer.bytes), Side::Host => self.host.read(&mut buffer.bytes) };
            match read {
                Ok(0) => {
                    buffer.bytes.clear();
                    return Err(ForwardError::Io(io::ErrorKind::UnexpectedEof.into()));
                }
                Ok(count) => {
                    buffer.bytes.truncate(count);
                    buffer.offset = 0;
                    buffer.paid = false;
                }
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
        if !buffer.paid {
            let need = buffer.bytes.len();
            if !self.admit(now, Some(need))? { return Ok(ForwardProgress::Blocked); }
            let buffer = match from { Side::Guest => &mut self.guest_to_host, Side::Host => &mut self.host_to_guest };
            buffer.paid = true;
        }
        let buffer = match from { Side::Guest => &mut self.guest_to_host, Side::Host => &mut self.host_to_guest };
        let written = match from {
            Side::Guest => self.host.write(&buffer.bytes[buffer.offset..]),
            Side::Host => self.guest.write(&buffer.bytes[buffer.offset..]),
        };
        let written = match written {
            Ok(0) => return Err(ForwardError::Io(io::ErrorKind::WriteZero.into())),
            Ok(written) => written,
            Err(error) if error.kind() == io::ErrorKind::WouldBlock => return Ok(ForwardProgress::Blocked),
            Err(error) => return Err(ForwardError::Io(error)),
        };
        buffer.offset += written;
        if buffer.offset == buffer.bytes.len() {
            buffer.bytes.clear();
            buffer.offset = 0;
            buffer.paid = false;
        }
        Ok(ForwardProgress::Transferred(written))
    }
}

#[derive(Clone, Copy)]
enum Side { Guest, Host }

impl<GUEST, HOST> Drop for PairedConnection<GUEST, HOST> {
    fn drop(&mut self) {
        release_slot(&self.relay, &self.verified, self.slot_id);
    }
}
