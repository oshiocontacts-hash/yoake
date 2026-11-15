//! rustls adapter and handshake admission limits for the social listener.
//!
//! Admission is per source group (an IPv4 address, or an IPv6 /64, after
//! `to_canonical()`), so it only means something when clients reach the relay
//! directly. Behind a proxy or load balancer every client shares the proxy's
//! address; that needs the PROXY protocol, which is not implemented. Run the
//! social listener without a proxy.
use std::{
    collections::BTreeMap,
    io,
    net::{IpAddr, Ipv6Addr, Shutdown, TcpStream},
    sync::{Arc, Mutex},
    time::Duration,
};

use horizon_friends::social_tls::{ChannelAcceptor, ChannelTimeouts, ControlChannel, server_channel};
use rustls::{ServerConfig, pki_types::{CertificateDer, PrivateKeyDer, PrivatePkcs8KeyDer}};

pub use crate::social_cert::read_certificate_and_key;

#[derive(Clone, Copy)]
pub struct HandshakeLimits {
    /// In-progress handshakes at once; when full, the oldest one is dropped to admit a new one.
    pub total: usize,
    /// In-progress handshakes per source group; extra ones are refused.
    pub per_ip: usize,
    pub timeout: Duration,
}
impl Default for HandshakeLimits {
    fn default() -> Self {
        Self { total: 256, per_ip: 4, timeout: Duration::from_secs(4) }
    }
}

/// IPv4 mapped addresses become IPv4; IPv6 addresses are grouped by /64.
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

struct InFlight {
    group: IpAddr,
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
}

impl TlsAcceptor {
    pub fn new(config: Arc<ServerConfig>, limits: HandshakeLimits) -> Self {
        Self { config, limits, timeouts: ChannelTimeouts::default(), active: Arc::new(Mutex::new(Active::default())) }
    }

    /// Deadlines for the first message and for writes after the handshake.
    pub fn with_timeouts(mut self, timeouts: ChannelTimeouts) -> Self {
        self.timeouts = timeouts;
        self
    }

    /// `certificate` is the leaf, DER; `key` is PKCS#8 DER. A certificate that
    /// does not match the key, or is outside its validity period, is an error.
    pub fn from_der(certificate: Vec<u8>, key: Vec<u8>, limits: HandshakeLimits) -> io::Result<Self> {
        let now = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).unwrap_or_default().as_secs();
        crate::social_cert::check_validity(&certificate, now as i64)?;
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
        Ok(Self::new(Arc::new(config), limits))
    }

    fn begin(&self, stream: &TcpStream) -> io::Result<HandshakeGuard> {
        let group = source_group(stream.peer_addr()?.ip());
        let clone = stream.try_clone()?;
        let mut active = self.active.lock().unwrap_or_else(|poisoned| poisoned.into_inner());
        if active.flights.values().filter(|flight| flight.group == group).count() >= self.limits.per_ip {
            return Err(io::Error::new(io::ErrorKind::ConnectionRefused, "social TLS handshake limit reached for this source"));
        }
        if active.flights.len() >= self.limits.total {
            // Stalled handshakes are the oldest; a real client finishes in a moment.
            if let Some(oldest) = active.flights.keys().next().copied() {
                if let Some(flight) = active.flights.remove(&oldest) {
                    let _ = flight.stream.shutdown(Shutdown::Both);
                }
            }
        }
        let id = active.next;
        active.next += 1;
        active.flights.insert(id, InFlight { group, stream: clone });
        Ok(HandshakeGuard { active: self.active.clone(), id })
    }
}

impl ChannelAcceptor for TlsAcceptor {
    fn accept(&self, stream: TcpStream) -> io::Result<Box<dyn ControlChannel>> {
        let guard = match self.begin(&stream) {
            Ok(guard) => guard,
            Err(error) => {
                let _ = stream.shutdown(Shutdown::Both);
                return Err(error);
            }
        };
        let result = server_channel(stream, self.config.clone(), self.limits.timeout, self.timeouts);
        drop(guard); // the slot is free as soon as the handshake is over, successful or not
        result.map(|channel| Box::new(channel) as Box<dyn ControlChannel>)
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

    #[test]
    fn sources_are_grouped_by_ipv4_address_or_ipv6_slash_64() {
        let group = |text: &str| source_group(text.parse().unwrap());
        assert_eq!(group("2001:db8:1:2:aaaa::1"), group("2001:db8:1:2:bbbb::ffff"));
        assert_ne!(group("2001:db8:1:2::1"), group("2001:db8:1:3::1"));
        assert_eq!(group("::ffff:192.0.2.7"), group("192.0.2.7"));
        assert_ne!(group("192.0.2.7"), group("192.0.2.8"));
    }
}
