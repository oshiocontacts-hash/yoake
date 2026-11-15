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

impl<GUEST, HOST> PairedConnection<GUEST, HOST> {
    pub(super) fn new(relay: SocialRelay, verified: VerifiedGrant, slot_id: u64, guest: GUEST, host: HOST) -> Self {
        Self { relay, verified, slot_id, guest, host, guest_to_host: Buffer::new(), host_to_guest: Buffer::new() }
    }
}

impl<GUEST: Read + Write, HOST: Read + Write> PairedConnection<GUEST, HOST> {
    pub fn verified_grant(&self) -> &VerifiedGrant {
        &self.verified
    }

    /// Check the limits and take the byte budget for one transfer (lock held, no I/O).
    fn reserve(&self, now: Instant) -> Result<usize, ForwardError> {
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
        let budget = (session.tokens as usize).min(BUFFER_BYTES);
        session.tokens -= budget as f64;
        Ok(budget)
    }

    /// Return the unused part of the reservation and note activity (lock held, no I/O).
    fn settle(&self, budget: usize, used: usize, now: Instant) {
        let Ok(mut sessions) = self.relay.sessions.lock() else { return; };
        let Some(session) = sessions.get_mut(self.verified.session_id()) else { return; };
        session.tokens = (session.tokens + (budget - used) as f64).min(session.limits.burst_bytes as f64);
        if used > 0 {
            if let Some(slot) = session.slots.get_mut(&self.verified.nonce()).filter(|slot| slot.id == self.slot_id) {
                slot.last_activity = now;
            }
        }
    }

    pub fn forward_once(
        &mut self,
        direction: Direction,
        now: Instant,
    ) -> Result<ForwardProgress, ForwardError> {
        let budget = self.reserve(now)?;
        if budget == 0 {
            return Ok(ForwardProgress::Blocked);
        }
        let progress = match direction {
            Direction::GuestToHost => transfer(&mut self.guest, &mut self.host, &mut self.guest_to_host, budget),
            Direction::HostToGuest => transfer(&mut self.host, &mut self.guest, &mut self.host_to_guest, budget),
        };
        let used = match progress {
            Ok(ForwardProgress::Transferred(written)) => written,
            _ => 0,
        };
        self.settle(budget, used, now);
        progress
    }
}

/// `writer` must follow the `Write` contract: bytes it accepted are never offered again,
/// and `WouldBlock` means none were accepted.
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
