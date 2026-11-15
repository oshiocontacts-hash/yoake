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
        crate::social_cert::check_validity(&certificate, unix_now())?;
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
        Ok(acceptor)
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
        let guard = if self.validity_allows(unix_now()) {
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
mod tests {
    use super::*;
    use std::{io::Read, net::TcpListener};

    #[test]
    fn sources_are_grouped_by_ipv4_address_or_ipv6_slash_64() {
        let group = |text: &str| source_group(text.parse().unwrap());
        assert_eq!(group("2001:db8:1:2:aaaa::1"), group("2001:db8:1:2:bbbb::ffff"));
        assert_ne!(group("2001:db8:1:2::1"), group("2001:db8:1:3::1"));
        assert_eq!(group("::ffff:192.0.2.7"), group("192.0.2.7"));
        assert_ne!(group("192.0.2.7"), group("192.0.2.8"));
        let net = |text: &str| source_net(text.parse().unwrap());
        assert_eq!(net("2001:db8:1:2::1"), net("2001:db8:1:ffff::9"));
        assert_ne!(net("2001:db8:1::1"), net("2001:db8:2::1"));
        assert_eq!(net("192.0.2.7"), net("192.0.2.200"));
        let wide = |text: &str| source_wide(text.parse().unwrap());
        assert_eq!(wide("192.0.2.7"), wide("192.0.99.1"));
        assert_ne!(wide("192.0.2.7"), wide("192.1.2.7"));
        assert_eq!(wide("2001:db8:1::1"), wide("2001:db8:ffff::1"));
        assert_ne!(net("192.0.2.7"), net("192.0.3.7"));
    }

    fn acceptor(limits: HandshakeLimits) -> TlsAcceptor {
        let config = ServerConfig::builder_with_provider(Arc::new(rustls::crypto::ring::default_provider()))
            .with_protocol_versions(&[&rustls::version::TLS13]).unwrap()
            .with_no_client_auth()
            .with_cert_resolver(Arc::new(rustls::server::ResolvesServerCertUsingSni::new()));
        TlsAcceptor::new(Arc::new(config), limits)
    }

    /// A connected loopback pair: (the relay's socket, the peer's socket).
    fn sockets() -> (TcpStream, TcpStream) {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let peer = TcpStream::connect(listener.local_addr().unwrap()).unwrap();
        (listener.accept().unwrap().0, peer)
    }

    fn is_closed(peer: &mut TcpStream) -> bool {
        peer.set_read_timeout(Some(Duration::from_millis(500))).unwrap();
        match peer.read(&mut [0; 1]) {
            Ok(0) => true,
            Err(error) => error.kind() == io::ErrorKind::ConnectionReset,
            Ok(_) => false,
        }
    }

    #[test]
    fn spreading_over_many_sources_does_not_protect_the_oldest_connections_of_a_net() {
        let acceptor = acceptor(HandshakeLimits { total: 8, per_ip: 4, per_net: 100, timeout: Duration::from_secs(3) });
        let (mut attackers, mut guards) = (Vec::new(), Vec::new());
        // Seven attackers, each from its own /64 but all inside one /48.
        for index in 0..7u16 {
            let (relay_side, peer) = sockets();
            let ip: IpAddr = format!("2001:db8:1:{index}::1").parse().unwrap();
            guards.push(acceptor.begin_from(ip, &relay_side).unwrap());
            attackers.push((relay_side, peer));
        }
        let (legit_side, mut legit_peer) = sockets();
        guards.push(acceptor.begin_from("198.51.100.5".parse().unwrap(), &legit_side).unwrap());
        // Full: a second real client evicts an attacker, never the first real client.
        let (second_side, _second_peer) = sockets();
        guards.push(acceptor.begin_from("203.0.113.9".parse().unwrap(), &second_side).unwrap());
        assert!(is_closed(&mut attackers[0].1), "the oldest attacker connection must be dropped");
        legit_peer.set_read_timeout(Some(Duration::from_millis(300))).unwrap();
        let error = legit_peer.read(&mut [0; 1]).unwrap_err();
        assert!(matches!(error.kind(), io::ErrorKind::WouldBlock | io::ErrorKind::TimedOut), "the real client must keep its slot: {error:?}");
    }

    #[test]
    fn one_handshake_per_net_still_protects_real_clients_from_a_wide_flood() {
        let acceptor = acceptor(HandshakeLimits { total: 8, per_ip: 4, per_net: 100, timeout: Duration::from_secs(3) });
        let (mut attackers, mut guards) = (Vec::new(), Vec::new());
        // Seven attackers in distinct /24s of one /16: every net holds exactly one.
        for index in 0..7u8 {
            let (relay_side, peer) = sockets();
            guards.push(acceptor.begin_from(format!("203.0.{index}.1").parse().unwrap(), &relay_side).unwrap());
            attackers.push((relay_side, peer));
        }
        let (legit_side, mut legit_peer) = sockets();
        guards.push(acceptor.begin_from("198.51.100.5".parse().unwrap(), &legit_side).unwrap());
        let (second_side, _second_peer) = sockets();
        guards.push(acceptor.begin_from("192.0.2.9".parse().unwrap(), &second_side).unwrap());
        assert!(is_closed(&mut attackers[0].1), "an attacker connection must be dropped");
        legit_peer.set_read_timeout(Some(Duration::from_millis(300))).unwrap();
        let error = legit_peer.read(&mut [0; 1]).unwrap_err();
        assert!(matches!(error.kind(), io::ErrorKind::WouldBlock | io::ErrorKind::TimedOut), "{error:?}");
    }

    #[test]
    fn a_net_cannot_take_more_than_its_share() {
        let acceptor = acceptor(HandshakeLimits { total: 64, per_ip: 4, per_net: 3, timeout: Duration::from_secs(3) });
        let mut kept = Vec::new();
        for index in 0..3u8 {
            let (side, peer) = sockets();
            let guard = acceptor.begin_from(format!("192.0.2.{index}").parse().unwrap(), &side).unwrap();
            kept.push((side, peer, guard));
        }
        let (side, _peer) = sockets();
        assert!(acceptor.begin_from("192.0.2.77".parse().unwrap(), &side).is_err());
        assert!(acceptor.begin_from("192.0.3.77".parse().unwrap(), &side).is_ok());
    }

    #[test]
    fn a_certificate_outside_its_validity_period_refuses_new_connections() {
        let mut acceptor = acceptor(HandshakeLimits::default());
        acceptor.validity = Some((100, 200));
        assert!(!acceptor.validity_allows(99));
        assert!(acceptor.validity_allows(100));
        assert!(!acceptor.validity_allows(200));
    }
}
