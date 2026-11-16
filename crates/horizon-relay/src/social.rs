use std::collections::HashMap;
use std::fmt;
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use horizon_friends::social::{
    GrantError, GrantVerifier, MAX_GRANT_LIFETIME_SECS, HostTokenVerifier, ProvisionTicket, RegisterSession,
    SignedGrant, VerifiedGrant,
};

mod forward;
pub use forward::{Direction, ForwardError, ForwardProgress, PairedConnection};

#[cfg(test)]
mod tests;

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
            && self.burst_bytes >= MIN_BURST_BYTES
            && !self.idle_timeout.is_zero()
    }
}

/// Most provision-ticket nonces remembered at once; a full cache refuses new tickets.
pub const MAX_TICKET_NONCES: usize = 4096;
/// Caps on what a signed ticket may ask for (the portal signs, but the relay owns its resources).
pub const MAX_TICKET_BYTES_PER_SECOND: u64 = 1 << 30;
pub const MAX_TICKET_BURST_BYTES: u64 = 64 << 20;
pub const MAX_TICKET_CONNECTIONS: u32 = 256;
pub const MAX_TICKET_IDLE_SECS: u32 = 3600;
/// Grant nonces remembered per ticket-provisioned session (the ticket has no field for it).
const TICKET_SESSION_NONCES: usize = 1024;

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
    /// Host token failures since the last lock-out.
    failures: u8,
    /// Host attempts on this slot are refused until then.
    locked_until: Option<Instant>,
}

/// Wrong host tokens tolerated per waiting guest before it is dropped.
const MAX_TOKEN_FAILURES: u8 = 3;
/// How long a slot refuses every host after that many wrong tokens. The guest is never dropped for it.
const TOKEN_LOCKOUT: Duration = Duration::from_secs(30);
/// Smallest burst: it must cover the biggest message, or one message could exceed the rate cap.
const MIN_BURST_BYTES: usize = horizon_friends::social_tls::MAX_MESSAGE_SIZE;

struct Session {
    host_principal: String,
    token: HostTokenVerifier,
    /// Digest the session was provisioned with by ticket; `None` for function-call provisioning.
    token_sha256: Option<[u8; 32]>,
    limits: SessionLimits,
    active: bool,
    /// Unix time until which a retired session's id stays reserved: no grant issued before the
    /// retirement can still be valid after it, so the id is free again then.
    retired_until: u64,
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
    /// Ticket nonce -> ticket `exp`. Locked only after `sessions`.
    ticket_nonces: Arc<Mutex<HashMap<[u8; 32], u64>>>,
    max_sessions: usize,
}

impl SocialRelay {
    pub fn new(pinned_portal_key: [u8; 32], max_sessions: usize) -> Self {
        Self {
            verifier: Arc::new(GrantVerifier::new(pinned_portal_key)),
            sessions: Arc::new(Mutex::new(HashMap::new())),
            ticket_nonces: Arc::new(Mutex::new(HashMap::new())),
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
        let unix_now = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).unwrap_or_default().as_secs();
        sessions.retain(|_, session| session.active || session.retired_until > unix_now);
        if sessions.contains_key(&request.session_id) {
            return Err(RelayError::SessionExists);
        }
        // Live sessions and retired-id tombstones are both bounded.
        if sessions.len() >= self.max_sessions.saturating_mul(4)
            || sessions.values().filter(|session| session.active).count() >= self.max_sessions
        {
            return Err(RelayError::SessionLimit);
        }
        sessions.insert(
            request.session_id.clone(),
            Session {
                host_principal: request.host_principal.clone(),
                token,
                token_sha256: None,
                limits,
                active: true,
                retired_until: 0,
                used: HashMap::new(),
                slots: HashMap::new(),
                next_slot_id: 0,
                tokens: limits.burst_bytes as f64,
                last_refill: now,
            },
        );
        Ok(())
    }

    /// Open a session from a portal-signed ticket and the host token it commits to.
    ///
    /// Checked in this order: signature and version, then `exp` (not past, at most 300 s ahead),
    /// the token digest, the limits, and the id format. A repeat for an active session with the
    /// same principal and token digest is `Ok` and changes nothing (a retry after a lost reply),
    /// even though its nonce is already remembered. Everything else for an existing id is
    /// refused, and so is a nonce that was already used to open a session. The nonce is stored
    /// only when a session is actually created.
    pub fn provision_ticket(
        &self,
        ticket: &ProvisionTicket,
        signature: &str,
        host_token: &str,
        unix_time: u64,
        now: Instant,
    ) -> Result<(), RelayError> {
        // No lock yet: unauthenticated callers learn nothing about sessions.
        let verified = self.verifier.verify_ticket(ticket, signature, unix_time)?;
        if !verified.token_matches(host_token) {
            return Err(RelayError::WrongToken);
        }
        let limits = ticket_limits(ticket)?;
        let token = HostTokenVerifier::new(host_token)?;
        let mut sessions = self.sessions.lock().map_err(|_| RelayError::Unavailable)?;
        let real_now = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).unwrap_or_default().as_secs();
        sessions.retain(|_, session| session.active || session.retired_until > real_now);
        if let Some(existing) = sessions.get(&ticket.session_id) {
            let same = existing.active
                && existing.host_principal == ticket.host_principal
                && existing.token_sha256 == Some(verified.token_sha256());
            return if same { Ok(()) } else { Err(RelayError::SessionExists) };
        }
        if sessions.len() >= self.max_sessions.saturating_mul(4)
            || sessions.values().filter(|session| session.active).count() >= self.max_sessions
        {
            return Err(RelayError::SessionLimit);
        }
        let mut nonces = self.ticket_nonces.lock().map_err(|_| RelayError::Unavailable)?;
        nonces.retain(|_, expires| *expires > unix_time);
        if nonces.contains_key(&verified.nonce()) {
            return Err(RelayError::Grant(GrantError::Replay));
        }
        if nonces.len() >= MAX_TICKET_NONCES {
            return Err(RelayError::Grant(GrantError::ReplayCacheFull));
        }
        nonces.insert(verified.nonce(), verified.expires_at());
        sessions.insert(
            ticket.session_id.clone(),
            Session {
                host_principal: ticket.host_principal.clone(),
                token,
                token_sha256: Some(verified.token_sha256()),
                limits,
                active: true,
                retired_until: 0,
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
        session.slots = HashMap::new(); // not clear(): give the memory back
        session.used = HashMap::new();
        let unix_now = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).unwrap_or_default().as_secs();
        session.retired_until = unix_now + MAX_GRANT_LIFETIME_SECS;
        Ok(())
    }

    pub fn begin_guest<GUEST>(
        &self,
        signed: &SignedGrant,
        guest: GUEST,
        unix_time: u64,
        now: Instant,
    ) -> Result<PendingGuest<GUEST>, RelayError> {
        // The signature comes first, without any lock, so unauthenticated callers learn
        // nothing about sessions and the global lock never covers the crypto.
        let checked = self.verifier.verify_signature(signed)?;
        let mut sessions = self.sessions.lock().map_err(|_| RelayError::Unavailable)?;
        let session = sessions
            .get_mut(&signed.grant.session_id)
            .filter(|session| session.active)
            .ok_or(RelayError::UnknownSession)?;
        let verified = checked.check(&signed.grant.session_id, &session.host_principal, unix_time)?;
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
        session.slots.insert(nonce, Slot { id: slot_id, paired: false, last_activity: now, failures: 0, locked_until: None });
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

/// The ticket's limits, or `InvalidLimits` when one is zero or beyond the relay's caps.
fn ticket_limits(ticket: &ProvisionTicket) -> Result<SessionLimits, RelayError> {
    let in_range = ticket.bytes_per_second > 0
        && ticket.bytes_per_second <= MAX_TICKET_BYTES_PER_SECOND
        && ticket.burst_bytes >= MIN_BURST_BYTES as u64
        && ticket.burst_bytes <= MAX_TICKET_BURST_BYTES
        && ticket.max_connections > 0
        && ticket.max_connections <= MAX_TICKET_CONNECTIONS
        && ticket.idle_secs > 0
        && ticket.idle_secs <= MAX_TICKET_IDLE_SECS;
    if !in_range {
        return Err(RelayError::InvalidLimits);
    }
    let limits = SessionLimits {
        max_connections: ticket.max_connections as usize,
        max_nonces: TICKET_SESSION_NONCES,
        bytes_per_second: ticket.bytes_per_second,
        burst_bytes: ticket.burst_bytes as usize,
        idle_timeout: Duration::from_secs(u64::from(ticket.idle_secs)),
    };
    if limits.valid() { Ok(limits) } else { Err(RelayError::InvalidLimits) }
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

    pub fn slot_id(&self) -> u64 { self.slot_id }

    pub fn is_expired(&self, now: Instant) -> bool { self.probe().is_expired(now) }

    /// A cheap handle for watching this guest's slot without owning the guest.
    pub fn probe(&self) -> SlotProbe {
        SlotProbe {
            relay: self.relay.clone(),
            session_id: self.verified.session_id().to_owned(),
            nonce: self.verified.nonce(),
            slot_id: self.slot_id,
        }
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
        // `signed == self.signed`, whose signature was verified when the guest arrived.
        if self.verified.expires_at() <= unix_time {
            return Err(RelayError::Grant(GrantError::Expired));
        }
        session.prune(unix_time, now);
        let slot = session.slots.get_mut(&self.verified.nonce())
            .filter(|slot| slot.id == self.slot_id)
            .ok_or(RelayError::TimedOut)?;
        if slot.locked_until.is_some_and(|until| now < until) {
            return Err(RelayError::WrongToken);
        }
        if !session.token.verify(host_token) {
            slot.failures += 1;
            if slot.failures >= MAX_TOKEN_FAILURES {
                slot.failures = 0;
                slot.locked_until = Some(now.checked_add(TOKEN_LOCKOUT).unwrap_or(now));
            }
            return Err(RelayError::WrongToken);
        }
        if slot.paired {
            return Err(RelayError::PairingMismatch);
        }
        let guest = self.guest.take().ok_or(RelayError::PairingMismatch)?;
        slot.paired = true;
        slot.last_activity = now;
        self.armed = false;
        Ok(PairedConnection::new(self.relay.clone(), self.verified.clone(), self.slot_id, guest, host))
    }
}

impl<GUEST> Drop for PendingGuest<GUEST> {
    fn drop(&mut self) {
        if self.armed {
            release_slot(&self.relay, &self.verified, self.slot_id);
        }
    }
}

pub struct SlotProbe {
    relay: SocialRelay,
    session_id: String,
    nonce: [u8; 32],
    slot_id: u64,
}

impl SlotProbe {
    /// True once the slot is gone, retired, or idle past the limit. Takes only the sessions lock.
    pub fn is_expired(&self, now: Instant) -> bool {
        let Ok(sessions) = self.relay.sessions.lock() else { return true; };
        let Some(session) = sessions.get(&self.session_id).filter(|s| s.active) else { return true; };
        match session.slots.get(&self.nonce) {
            Some(slot) if slot.id == self.slot_id => {
                now.checked_duration_since(slot.last_activity).unwrap_or_default() >= session.limits.idle_timeout
            }
            _ => true,
        }
    }
}

pub(super) fn release_slot(relay: &SocialRelay, verified: &VerifiedGrant, slot_id: u64) {
    if let Ok(mut sessions) = relay.sessions.lock() {
        if let Some(session) = sessions.get_mut(verified.session_id()) {
            if session.slots.get(&verified.nonce()).is_some_and(|slot| slot.id == slot_id) {
                session.slots.remove(&verified.nonce());
            }
        }
    }
}

