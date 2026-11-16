//! Forwarding between a paired guest and host.
//!
//! The sessions lock is held only to check limits and charge budget, and again
//! to record activity; the reads and writes in between run without it.
use std::io::{self, Read, Write};
use std::time::Instant;

use horizon_friends::social::VerifiedGrant;

use super::{BandwidthReservation, RESERVATION_LIFETIME, RelayError, SessionLimits, SocialRelay, release_slot};

const BUFFER_BYTES: usize = horizon_friends::social_tls::MAX_MESSAGE_SIZE;

struct Buffer {
    bytes: Vec<u8>,
    length: usize,
    offset: usize,
    /// The bandwidth for `bytes` was already taken.
    paid: bool,
    error: Option<io::Error>,
}

impl Buffer {
    fn new() -> Self {
        Self { bytes: vec![0; BUFFER_BYTES], length: 0, offset: 0, paid: false, error: None }
    }

    fn read(&mut self, source: &mut impl Read) {
        if self.length != 0 || self.error.is_some() { return; }
        match source.read(&mut self.bytes) {
            Ok(0) => {
                self.error = Some(io::ErrorKind::UnexpectedEof.into());
            }
            Ok(count) => self.length = count,
            Err(error) if error.kind() == io::ErrorKind::WouldBlock => {}
            Err(error) => {
                self.error = Some(error);
            }
        }
    }

    fn pending(&self) -> Option<usize> {
        if self.length == 0 { None } else { Some(if self.paid { 0 } else { self.length }) }
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum Direction {
    GuestToHost,
    HostToGuest,
}

impl Direction {
    fn index(self) -> usize {
        match self { Self::GuestToHost => 0, Self::HostToGuest => 1 }
    }

    pub(crate) fn opposite(self) -> Self {
        match self { Self::GuestToHost => Self::HostToGuest, Self::HostToGuest => Self::GuestToHost }
    }
}

pub(super) struct PairBandwidth {
    tokens: [f64; 2],
    last_refill: Instant,
    pending: [Option<usize>; 2],
    recent_usage: f64,
}

impl PairBandwidth {
    pub(super) fn new(limits: SessionLimits, now: Instant) -> Self {
        let burst = (limits.burst_bytes as f64 / 2.0).max(BUFFER_BYTES as f64);
        Self { tokens: [burst; 2], last_refill: now, pending: [None; 2], recent_usage: 0.0 }
    }

    fn refill(&mut self, limits: SessionLimits, now: Instant) {
        let elapsed = now.checked_duration_since(self.last_refill).unwrap_or_default();
        let burst = (limits.burst_bytes as f64 / 2.0).max(BUFFER_BYTES as f64);
        for tokens in &mut self.tokens {
            *tokens = (*tokens + elapsed.as_secs_f64() * limits.bytes_per_second as f64 / 2.0).min(burst);
        }
        self.recent_usage *= (-elapsed.as_secs_f64()).exp();
        self.last_refill = now.max(self.last_refill);
    }

    fn available(&self, direction: Direction) -> bool {
        let index = direction.index();
        let other = 1 - index;
        let Some(need) = self.pending[index].filter(|need| *need > 0) else { return false; };
        let waiting = self.pending[other].is_some_and(|need| need > 0);
        let borrowed = if waiting { 0.0 } else { self.tokens[other] };
        self.tokens[index] + borrowed >= need as f64
    }

    fn take(&mut self, direction: Direction, need: usize) {
        let index = direction.index();
        let own = self.tokens[index].min(need as f64);
        self.tokens[index] -= own;
        self.tokens[1 - index] -= need as f64 - own;
        self.pending[index] = Some(0);
        self.recent_usage += need as f64;
    }
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
    contention_yield: bool,
}

impl<GUEST, HOST> PairedConnection<GUEST, HOST> {
    pub(super) fn new(relay: SocialRelay, verified: VerifiedGrant, slot_id: u64, guest: GUEST, host: HOST) -> Self {
        Self {
            relay, verified, slot_id, guest, host,
            guest_to_host: Buffer::new(), host_to_guest: Buffer::new(), contention_yield: false,
        }
    }
}

impl<GUEST: Read + Write, HOST: Read + Write> PairedConnection<GUEST, HOST> {
    pub fn verified_grant(&self) -> &VerifiedGrant {
        &self.verified
    }

    /// Check the slot and refill only its buckets; admit a pending direction if funded.
    /// Lock held, no I/O. `Ok(false)`: not enough bandwidth yet.
    fn admit(&mut self, now: Instant, direction: Option<Direction>) -> Result<bool, ForwardError> {
        let unavailable = || ForwardError::Relay(RelayError::Unavailable);
        let mut sessions = self.relay.sessions.lock().map_err(|_| unavailable())?;
        let session = sessions
            .get_mut(self.verified.session_id())
            .filter(|session| session.active)
            .ok_or(ForwardError::Relay(RelayError::UnknownSession))?;
        let nonce = self.verified.nonce();
        session.prune_idle(now);
        session.slots.get(&nonce)
            .filter(|slot| slot.paired && slot.id == self.slot_id)
            .ok_or(ForwardError::Relay(RelayError::TimedOut))?;
        let Some(direction) = direction else { return Ok(true); };
        let elapsed = now.checked_duration_since(session.last_refill).unwrap_or_default();
        session.tokens = (session.tokens + elapsed.as_secs_f64() * session.limits.bytes_per_second as f64)
            .min(session.limits.burst_bytes as f64);
        session.last_refill = now.max(session.last_refill);
        let slot = session.slots.get_mut(&nonce);
        debug_assert!(slot.is_some());
        let slot = slot.ok_or_else(unavailable)?;
        let recently_active = slot.last_transfer.is_some();
        let bandwidth = &mut slot.bandwidth;
        bandwidth.refill(session.limits, now);
        bandwidth.pending = [self.guest_to_host.pending(), self.host_to_guest.pending()];
        let Some(need) = bandwidth.pending[direction.index()] else { return Ok(false); };
        if need == 0 { return Ok(true); }
        if !bandwidth.available(direction) { return Ok(false); }
        let over_share = session.active_pairs > 1 && bandwidth.recent_usage
            > session.limits.bytes_per_second as f64 / session.active_pairs as f64;
        let reserved = session.reservation.as_ref()
            .filter(|reservation| reservation.nonce != nonce)
            .filter(|_| over_share || (session.active_pairs == 1 && recently_active))
            .map_or(0, |reservation| reservation.need);
        if session.tokens < (need + reserved) as f64 {
            self.contention_yield |= over_share;
            if !over_share && session.tokens < need as f64 && session.reservation.is_none() {
                session.reservation = Some(BandwidthReservation {
                    nonce, direction, need,
                    expires_at: now.checked_add(RESERVATION_LIFETIME).unwrap_or(now),
                });
            }
            return Ok(false);
        }
        session.tokens -= need as f64;
        bandwidth.take(direction, need);
        if session.reservation.as_ref().is_some_and(|reservation|
            reservation.nonce == nonce && reservation.direction == direction
        ) {
            session.reservation = None;
        }
        Ok(true)
    }

    pub(crate) fn take_contention_yield(&mut self) -> bool {
        std::mem::take(&mut self.contention_yield)
    }

    fn note_activity(&self, now: Instant) {
        let Ok(mut sessions) = self.relay.sessions.lock() else { return; };
        if let Some(session) = sessions.get_mut(self.verified.session_id()) {
            if let Some(slot) = session.slots.get_mut(&self.verified.nonce()).filter(|slot| slot.id == self.slot_id) {
                slot.bandwidth.pending = [self.guest_to_host.pending(), self.host_to_guest.pending()];
                session.note_transfer(self.verified.nonce(), now);
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
        self.guest_to_host.read(&mut self.guest);
        self.host_to_guest.read(&mut self.host);
        let progress = self.step(direction, now)?;
        if matches!(progress, ForwardProgress::Transferred(_)) { self.note_activity(now); }
        Ok(progress)
    }

    fn step(&mut self, direction: Direction, now: Instant) -> Result<ForwardProgress, ForwardError> {
        let buffer = match direction {
            Direction::GuestToHost => &mut self.guest_to_host,
            Direction::HostToGuest => &mut self.host_to_guest,
        };
        if let Some(error) = buffer.error.take() { return Err(ForwardError::Io(error)); }
        if !self.admit(now, Some(direction))? { return Ok(ForwardProgress::Blocked); }
        let buffer = match direction {
            Direction::GuestToHost => &mut self.guest_to_host,
            Direction::HostToGuest => &mut self.host_to_guest,
        };
        buffer.paid = true;
        let written = match direction {
            Direction::GuestToHost => self.host.write(&buffer.bytes[buffer.offset..buffer.length]),
            Direction::HostToGuest => self.guest.write(&buffer.bytes[buffer.offset..buffer.length]),
        };
        let written = match written {
            Ok(0) => return Err(ForwardError::Io(io::ErrorKind::WriteZero.into())),
            Ok(written) => written,
            Err(error) if error.kind() == io::ErrorKind::WouldBlock => return Ok(ForwardProgress::Blocked),
            Err(error) => return Err(ForwardError::Io(error)),
        };
        buffer.offset += written;
        if buffer.offset == buffer.length {
            buffer.length = 0;
            buffer.offset = 0;
            buffer.paid = false;
        }
        Ok(ForwardProgress::Transferred(written))
    }
}

impl<GUEST, HOST> Drop for PairedConnection<GUEST, HOST> {
    fn drop(&mut self) {
        release_slot(&self.relay, &self.verified, self.slot_id);
    }
}
