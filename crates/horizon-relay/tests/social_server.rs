mod social_support;
use horizon_friends::{social::FromRelay, social_tls::ControlChannel};
use horizon_relay::social_server::SocialServer;
use social_support::*;
use std::time::Duration;

#[test]
fn valid_guest_and_host_bridge_payloads_both_ways() {
    let (_server, address, tls) = setup(Duration::from_secs(5));
    let grant = signed("session-1", unix_time() + 60, 1);
    let mut guest = guest(address, &tls, &grant);
    assert!(matches!(recv_from(&mut guest), FromRelay::Waiting {}));
    let mut host = host(address, &tls, &grant, TOKEN);
    assert!(matches!(recv_from(&mut host), FromRelay::Bridged {}));
    assert!(matches!(recv_from(&mut guest), FromRelay::Bridged {}));
    guest.send_message(b"hello host").unwrap();
    assert_eq!(host.recv_message().unwrap(), b"hello host");
    host.send_message(b"hello guest").unwrap();
    assert_eq!(guest.recv_message().unwrap(), b"hello guest");
}

#[test]
fn wrong_host_token_is_rejected_and_closed() {
    let (_server, address, tls) = setup(Duration::from_secs(5));
    let grant = signed("session-1", unix_time() + 60, 2);
    let mut guest = guest(address, &tls, &grant);
    assert!(matches!(recv_from(&mut guest), FromRelay::Waiting {}));
    let mut host = host(address, &tls, &grant, &"cd".repeat(32));
    assert_rejected(&mut host);
    assert!(host.recv_message().is_err());
    assert_rejected(&mut guest);
    assert!(guest.recv_message().is_err());
}

#[test]
fn expired_grants_and_wrong_sessions_are_rejected() {
    let (_server, address, tls) = setup(Duration::from_secs(5));
    let expired = signed("session-1", unix_time() - 1, 3);
    let mut channel = guest(address, &tls, &expired);
    assert_rejected(&mut channel);
    assert!(channel.recv_message().is_err());
    let wrong = signed("session-2", unix_time() + 60, 4);
    let mut channel = guest(address, &tls, &wrong);
    assert_rejected(&mut channel);
    assert!(channel.recv_message().is_err());
}

#[test]
fn wrong_relay_ca_fails_the_handshake() {
    let (_server, address, _tls) = setup(Duration::from_secs(5));
    let other = tls("relay.test");
    let stream = std::net::TcpStream::connect(address).unwrap();
    let result = horizon_friends::social_tls::client_channel(
        stream, &other.ca, "relay.test", Duration::from_secs(2),
    );
    assert!(result.is_err());
}

#[test]
fn pending_guest_is_closed_after_the_idle_timeout() {
    let (_server, address, tls) = setup(Duration::from_millis(100));
    let grant = signed("session-1", unix_time() + 60, 5);
    let mut guest = guest(address, &tls, &grant);
    assert!(matches!(recv_from(&mut guest), FromRelay::Waiting {}));
    assert!(guest.recv_message().is_err());
}

#[test]
fn idle_bridged_pair_is_closed() {
    let (_server, address, tls) = setup(Duration::from_millis(100));
    let grant = signed("session-1", unix_time() + 60, 6);
    let mut guest = guest(address, &tls, &grant);
    assert!(matches!(recv_from(&mut guest), FromRelay::Waiting {}));
    let mut host = host(address, &tls, &grant, TOKEN);
    assert!(matches!(recv_from(&mut host), FromRelay::Bridged {}));
    assert!(matches!(recv_from(&mut guest), FromRelay::Bridged {}));
    assert!(host.recv_message().is_err());
    assert!(guest.recv_message().is_err());
}

#[test]
fn no_social_configuration_does_not_start_a_listener() {
    assert!(SocialServer::from_options(None, None, None, None).is_none());
}
