use super::*;
use crate::social_cert::{expires_within_warning, warning_log_due};
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
    for index in 0..7u16 {
        let (relay_side, peer) = sockets();
        let ip: IpAddr = format!("2001:db8:1:{index}::1").parse().unwrap();
        guards.push(acceptor.begin_from(ip, &relay_side).unwrap());
        attackers.push((relay_side, peer));
    }
    let (legit_side, mut legit_peer) = sockets();
    guards.push(acceptor.begin_from("198.51.100.5".parse().unwrap(), &legit_side).unwrap());
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

#[test]
fn the_expiry_warning_covers_seven_days_but_not_expiration() {
    let week = 7 * 24 * 60 * 60;
    assert!(!expires_within_warning(1_000 + week + 1, 1_000));
    assert!(expires_within_warning(1_000 + week, 1_000));
    assert!(expires_within_warning(1_001, 1_000));
    assert!(!expires_within_warning(1_000, 1_000));
    assert!(!expires_within_warning(999, 1_000));
}

#[test]
fn warning_logging_is_limited_to_once_an_hour() {
    assert!(warning_log_due(1_000, i64::MIN));
    assert!(!warning_log_due(1_000, 1_000));
    assert!(!warning_log_due(4_599, 1_000));
    assert!(warning_log_due(4_600, 1_000));
}
