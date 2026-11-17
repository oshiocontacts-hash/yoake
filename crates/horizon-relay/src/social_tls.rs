//! rustls adapter and connection admission limits for the social listener.
//!
//! A slot is held from accept until the first control message arrives, so it covers
//! the TLS handshake and the wait for that message. Slots are counted per source
//! group (an IPv4 address, or an IPv6 /64, after `to_canonical()`) and per coarser
//! net (IPv4 /24, IPv6 /48). When the total is full, the oldest connection of the
//! busiest net inside the busiest wide group (IPv4 /16, IPv6 /32) is dropped. This only means something when clients reach the relay
//! directly: behind a proxy or load balancer every client shares the proxy's
//! address; that needs the PROXY protocol, which is not implemented. Run the
//! social listener without a proxy.
use std::{
    collections::{BTreeMap, HashMap},
    io,
    net::{IpAddr, Ipv6Addr, Shutdown, TcpStream},
    sync::{Arc, Mutex, atomic::{AtomicBool, Ordering}},
    time::{Duration, SystemTime, UNIX_EPOCH},
};

use horizon_friends::social_tls::{ChannelAcceptor, ChannelTimeouts, ControlChannel, server_channel};
use rustls::{ServerConfig, pki_types::{CertificateDer, PrivateKeyDer, PrivatePkcs8KeyDer}};

pub use crate::social_cert::read_certificate_and_key;

#[derive(Clone, Copy)]
pub struct HandshakeLimits {
    /// Connections before their first control message, at once; when full, the oldest one of
    /// the busiest net is dropped to admit a new one.
    pub total: usize,
    /// Such connections per source group (IPv4 address or IPv6 /64); extra ones are refused.
    pub per_ip: usize,
    /// Such connections per coarse net (IPv4 /24 or IPv6 /48); extra ones are refused.
    pub per_net: usize,
    /// Deadline for the TLS handshake itself.
    pub timeout: Duration,
}
impl Default for HandshakeLimits {
    fn default() -> Self {
        Self { total: 256, per_ip: 4, per_net: 32, timeout: Duration::from_secs(4) }
    }
}

/// IPv4-mapped addresses become IPv4; IPv6 addresses are grouped by /64.
fn source_group(ip: IpAddr) -> IpAddr {
    match ip.to_canonical() {
        IpAddr::V6(v6) => {
            let mut octets = v6.octets();
            octets[8..].fill(0);
            IpAddr::V6(Ipv6Addr::from(octets))
        }
        v4 => v4,
    }
}

/// The coarser net a source belongs to: IPv4 /24, IPv6 /48.
fn source_net(ip: IpAddr) -> IpAddr {
    match ip.to_canonical() {
        IpAddr::V6(v6) => {
            let mut octets = v6.octets();
            octets[6..].fill(0);
            IpAddr::V6(Ipv6Addr::from(octets))
        }
        IpAddr::V4(v4) => {
            let mut octets = v4.octets();
            octets[3] = 0;
            IpAddr::V4(octets.into())
        }
    }
}

/// The widest tier used for eviction: IPv4 /16, IPv6 /32.
fn source_wide(ip: IpAddr) -> IpAddr {
    match ip.to_canonical() {
        IpAddr::V6(v6) => {
            let mut octets = v6.octets();
            octets[4..].fill(0);
            IpAddr::V6(Ipv6Addr::from(octets))
        }
        IpAddr::V4(v4) => {
            let mut octets = v4.octets();
            octets[2..].fill(0);
            IpAddr::V4(octets.into())
        }
    }
}

struct InFlight {
    group: IpAddr,
    net: IpAddr,
    wide: IpAddr,
    stream: TcpStream,
}

#[derive(Default)]
struct Active {
    next: u64,
    /// Ordered by start, so the first entry is the oldest.
    flights: BTreeMap<u64, InFlight>,
}

pub struct TlsAcceptor {
    config: Arc<ServerConfig>,
    limits: HandshakeLimits,
    timeouts: ChannelTimeouts,
    active: Arc<Mutex<Active>>,
    /// Certificate validity (Unix seconds), checked on every accept.
    validity: Option<(i64, i64)>,
    out_of_validity_logged: AtomicBool,
    last_expiry_warning: std::sync::atomic::AtomicI64,
}

fn unix_now() -> i64 {
    SystemTime::now().duration_since(UNIX_EPOCH).unwrap_or_default().as_secs() as i64
}

impl TlsAcceptor {
    pub fn new(config: Arc<ServerConfig>, limits: HandshakeLimits) -> Self {
        Self {
            config,
            limits,
            timeouts: ChannelTimeouts::default(),
            active: Arc::new(Mutex::new(Active::default())),
            validity: None,
            out_of_validity_logged: AtomicBool::new(false),
            last_expiry_warning: std::sync::atomic::AtomicI64::new(i64::MIN),
        }
    }

    /// Deadlines for the first message and for writes after the handshake.
    pub fn with_timeouts(mut self, timeouts: ChannelTimeouts) -> Self {
        self.timeouts = timeouts;
        self
    }

    /// `certificate` is the leaf, DER; `key` is PKCS#8 DER. A certificate that
    /// does not match the key, or is outside its validity period, is an error.
    /// The validity period is checked again on every accepted connection.
    pub fn from_der(certificate: Vec<u8>, key: Vec<u8>, limits: HandshakeLimits) -> io::Result<Self> {
        let now = unix_now();
        crate::social_cert::check_validity(&certificate, now)?;
        let validity = crate::social_cert::certificate_validity(&certificate)?;
        let provider = Arc::new(rustls::crypto::ring::default_provider());
        let config = ServerConfig::builder_with_provider(provider)
            .with_protocol_versions(&[&rustls::version::TLS13])
            .map_err(io::Error::other)?
            .with_no_client_auth()
            .with_single_cert(
                vec![CertificateDer::from(certificate)],
                PrivateKeyDer::Pkcs8(PrivatePkcs8KeyDer::from(key)),
            )
            .map_err(|error| match error {
                rustls::Error::InconsistentKeys(_) => io::Error::new(
                    io::ErrorKind::InvalidData,
                    "social TLS certificate does not match the private key",
                ),
                other => io::Error::new(io::ErrorKind::InvalidData, format!("social TLS certificate or key is unusable: {other}")),
            })?;
        let mut acceptor = Self::new(Arc::new(config), limits);
        acceptor.validity = Some(validity);
        acceptor.warn_expiring(now);
        Ok(acceptor)
    }

    fn warn_expiring(&self, now: i64) {
        let Some((_, to)) = self.validity else { return; };
        if !crate::social_cert::expires_within_warning(to, now) { return; }
        let last = self.last_expiry_warning.load(Ordering::Relaxed);
        if !crate::social_cert::warning_log_due(now, last) { return; }
        if self.last_expiry_warning.compare_exchange(last, now, Ordering::Relaxed, Ordering::Relaxed).is_ok() {
            eprintln!("horizon-relay: WARNING: social TLS certificate expires in {} seconds; renew and restart", to - now);
        }
    }

    /// Outside its validity period new connections are refused; logged once per episode.
    fn validity_allows(&self, now: i64) -> bool {
        let Some((from, to)) = self.validity else { return true; };
        let ok = (from..to).contains(&now);
        if ok {
            self.out_of_validity_logged.store(false, Ordering::Relaxed);
        } else if !self.out_of_validity_logged.swap(true, Ordering::Relaxed) {
            eprintln!("horizon-relay: the social TLS certificate is outside its validity period; refusing new connections");
        }
        ok
    }

    fn begin(&self, stream: &TcpStream) -> io::Result<HandshakeGuard> {
        self.begin_from(stream.peer_addr()?.ip(), stream)
    }

    fn begin_from(&self, ip: IpAddr, stream: &TcpStream) -> io::Result<HandshakeGuard> {
        let (group, net, wide) = (source_group(ip), source_net(ip), source_wide(ip));
        let clone = stream.try_clone()?;
        let mut active = self.active.lock().unwrap_or_else(|poisoned| poisoned.into_inner());
        let refuse = |what: &str| Err(io::Error::new(
            io::ErrorKind::ConnectionRefused,
            format!("social TLS connection limit reached for this {what}"),
        ));
        if active.flights.values().filter(|flight| flight.group == group).count() >= self.limits.per_ip {
            return refuse("source");
        }
        if active.flights.values().filter(|flight| flight.net == net).count() >= self.limits.per_net {
            return refuse("network");
        }
        if active.flights.len() >= self.limits.total {
            // Stalled connections are the oldest; a real client is through in a moment. Drop the
            // oldest connection of the busiest tier-by-tier: widest group first (IPv4 /16, IPv6
            // /32), then its busiest net, so spreading over many sources does not help.
            // Ties go to whoever started first.
            let all: Vec<_> = active.flights.iter().map(|(id, flight)| (*id, flight)).collect();
            let in_busiest_wide = busiest(|flight| flight.wide, &all);
            let in_busiest_net = busiest(|flight| flight.net, &in_busiest_wide);
            let victim = in_busiest_net.first().map(|(id, _)| *id);
            if let Some(flight) = victim.and_then(|id| active.flights.remove(&id)) {
                let _ = flight.stream.shutdown(Shutdown::Both);
            }
        }
        let id = active.next;
        active.next += 1;
        active.flights.insert(id, InFlight { group, net, wide, stream: clone });
        Ok(HandshakeGuard { active: self.active.clone(), id })
    }
}

/// The entries (oldest first) whose key is shared by the most entries.
fn busiest<'a>(key: fn(&InFlight) -> IpAddr, among: &[(u64, &'a InFlight)]) -> Vec<(u64, &'a InFlight)> {
    let mut counts: HashMap<IpAddr, usize> = HashMap::new();
    for (_, flight) in among { *counts.entry(key(flight)).or_default() += 1; }
    let most = counts.values().copied().max().unwrap_or(0);
    among.iter().filter(|(_, flight)| counts[&key(flight)] == most).copied().collect()
}

impl ChannelAcceptor for TlsAcceptor {
    fn accept(&self, stream: TcpStream) -> io::Result<Box<dyn ControlChannel>> {
        let now = unix_now();
        self.warn_expiring(now);
        let guard = if self.validity_allows(now) {
            self.begin(&stream)
        } else {
            Err(io::Error::new(io::ErrorKind::ConnectionRefused, "social TLS certificate is outside its validity period"))
        };
        let guard = match guard {
            Ok(guard) => guard,
            Err(error) => {
                let _ = stream.shutdown(Shutdown::Both);
                return Err(error);
            }
        };
        // On failure the guard drops here; on success it travels with the channel until the
        // first control message arrives, so a silent peer keeps costing a slot.
        let channel = server_channel(stream, self.config.clone(), self.limits.timeout, self.timeouts)?;
        Ok(Box::new(Guarded { inner: Box::new(channel), guard: Some(guard) }))
    }
}

/// A channel that still holds its admission slot.
struct Guarded {
    inner: Box<dyn ControlChannel>,
    guard: Option<HandshakeGuard>,
}

impl ControlChannel for Guarded {
    fn send_message(&mut self, bytes: &[u8]) -> io::Result<()> { self.inner.send_message(bytes) }

    fn try_send_message(&mut self, bytes: &[u8]) -> io::Result<bool> {
        self.inner.try_send_message(bytes)
    }

    fn recv_message(&mut self) -> io::Result<Vec<u8>> {
        let message = self.inner.recv_message()?;
        self.guard = None;
        Ok(message)
    }

    fn try_recv_message(&mut self, wait: Duration) -> io::Result<Option<Vec<u8>>> {
        let message = self.inner.try_recv_message(wait)?;
        if message.is_some() { self.guard = None; }
        Ok(message)
    }

    fn channel_binding(&self) -> io::Result<[u8; 32]> { self.inner.channel_binding() }

    fn shutdown(&mut self) {
        self.inner.shutdown();
        self.guard = None;
    }
}

struct HandshakeGuard {
    active: Arc<Mutex<Active>>,
    id: u64,
}
impl Drop for HandshakeGuard {
    fn drop(&mut self) {
        self.active.lock().unwrap_or_else(|poisoned| poisoned.into_inner()).flights.remove(&self.id);
    }
}

#[cfg(test)]
#[path = "social_tls_tests.rs"]
mod tests;
