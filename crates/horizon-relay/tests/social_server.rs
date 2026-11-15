use horizon_friends::{
    social::{FromRelay, Grant, RegisterSession, SignedGrant, ToRelay},
    social_channel::{RelayIdentity, SecureChannel, client_handshake},
};
use horizon_relay::{social::SessionLimits, social_server::SocialServer};
use ring::signature::Ed25519KeyPair;
use std::{
    io,
    net::{SocketAddr, TcpStream},
    time::{Duration, SystemTime, UNIX_EPOCH},
};

const PORTAL_SEED: &str = "9d61b19deffd5a60ba844af492ec2cc44449c5697b326919703bac031cae7f60";
const PORTAL_PUBLIC: &str = "d75a980182b10ab7d54bfed3c964073a0ee172f3daa62325af021a68f707511a";
const TOKEN: &str = "abababababababababababababababababababababababababababababababab";

fn hex<const N: usize>(text: &str) -> [u8; N] {
    let mut output = [0; N];
    for (index, byte) in output.iter_mut().enumerate() {
        *byte = u8::from_str_radix(&text[index * 2..index * 2 + 2], 16).unwrap();
    }
    output
}

fn unix_time() -> u64 {
    SystemTime::now().duration_since(UNIX_EPOCH).unwrap().as_secs()
}

fn signed(session: &str, exp: u64, nonce: u8) -> SignedGrant {
    let grant = Grant {
        v: 1,
        session_id: session.into(),
        host_principal: "member:host".into(),
        guest_principal: "guest:alice".into(),
        mc_profile: Some("profile-1".into()),
        relay_only: true,
        exp,
        nonce: format!("{nonce:02x}").repeat(32),
    };
    let key = Ed25519KeyPair::from_seed_unchecked(&hex::<32>(PORTAL_SEED)).unwrap();
    let signature = key.sign(&grant.signing_bytes().unwrap()).as_ref().iter()
        .map(|byte| format!("{byte:02x}")).collect();
    SignedGrant { grant, signature }
}

fn setup(idle: Duration) -> (SocialServer, SocketAddr, [u8; 32]) {
    let identity = RelayIdentity::from_seed(&[7; 32]).unwrap();
    let relay_key = identity.public_key();
    let server = SocialServer::new(hex::<32>(PORTAL_PUBLIC), identity);
    server.relay().provision(
        &RegisterSession::relay_only("session-1".into(), "member:host".into()),
        TOKEN,
        SessionLimits {
            max_connections: 4,
            max_nonces: 16,
            bytes_per_second: 1_000_000,
            burst_bytes: 64 * 1024,
            idle_timeout: idle,
        },
        std::time::Instant::now(),
    ).unwrap();
    let address = server.spawn_local().unwrap();
    (server, address, relay_key)
}

fn connect(address: SocketAddr, relay_key: &[u8; 32]) -> SecureChannel<TcpStream> {
    let stream = TcpStream::connect(address).unwrap();
    stream.set_read_timeout(Some(Duration::from_secs(2))).unwrap();
    stream.set_write_timeout(Some(Duration::from_secs(2))).unwrap();
    client_handshake(stream, relay_key).unwrap()
}

fn guest(address: SocketAddr, key: &[u8; 32], grant: &SignedGrant) -> SecureChannel<TcpStream> {
    let mut channel = connect(address, key);
    channel.send_to_relay(&ToRelay::Guest { grant: grant.clone() }).unwrap();
    channel
}

fn host(address: SocketAddr, key: &[u8; 32], grant: &SignedGrant, token: &str) -> SecureChannel<TcpStream> {
    let mut channel = connect(address, key);
    channel.send_to_relay(&ToRelay::Host { grant: grant.clone(), token: token.into() }).unwrap();
    channel
}

fn assert_rejected(channel: &mut SecureChannel<TcpStream>) {
    assert!(matches!(channel.recv_from_relay().unwrap(), FromRelay::Rejected {}));
}

#[test]
fn valid_guest_and_host_bridge_payloads_both_ways() {
    let (_server, address, key) = setup(Duration::from_secs(5));
    let grant = signed("session-1", unix_time() + 60, 1);
    let mut guest = guest(address, &key, &grant);
    assert!(matches!(guest.recv_from_relay().unwrap(), FromRelay::Waiting {}));
    let mut host = host(address, &key, &grant, TOKEN);
    assert!(matches!(host.recv_from_relay().unwrap(), FromRelay::Bridged {}));
    assert!(matches!(guest.recv_from_relay().unwrap(), FromRelay::Bridged {}));
    guest.send_message(b"hello host").unwrap();
    assert_eq!(host.recv_message().unwrap(), b"hello host");
    host.send_message(b"hello guest").unwrap();
    assert_eq!(guest.recv_message().unwrap(), b"hello guest");
}

#[test]
fn wrong_host_token_is_rejected_and_closed() {
    let (_server, address, key) = setup(Duration::from_secs(5));
    let grant = signed("session-1", unix_time() + 60, 2);
    let mut guest = guest(address, &key, &grant);
    assert!(matches!(guest.recv_from_relay().unwrap(), FromRelay::Waiting {}));
    let mut host = host(address, &key, &grant, &"cd".repeat(32));
    assert_rejected(&mut host);
    assert!(host.recv_message().is_err());
    assert_rejected(&mut guest);
    assert!(guest.recv_message().is_err());
}

#[test]
fn expired_grants_and_wrong_sessions_are_rejected() {
    let (_server, address, key) = setup(Duration::from_secs(5));
    let expired = signed("session-1", unix_time() - 1, 3);
    let mut channel = guest(address, &key, &expired);
    assert_rejected(&mut channel);
    assert!(channel.recv_message().is_err());
    let wrong = signed("session-2", unix_time() + 60, 4);
    let mut channel = guest(address, &key, &wrong);
    assert_rejected(&mut channel);
    assert!(channel.recv_message().is_err());
}

#[test]
fn wrong_relay_pin_fails_the_handshake() {
    let (_server, address, _key) = setup(Duration::from_secs(5));
    let stream = TcpStream::connect(address).unwrap();
    let error = client_handshake(stream, &[99; 32]).err().expect("wrong relay key was accepted");
    assert_eq!(error.kind(), io::ErrorKind::PermissionDenied);
}

#[test]
fn pending_guest_is_closed_after_the_idle_timeout() {
    let (_server, address, key) = setup(Duration::from_millis(100));
    let grant = signed("session-1", unix_time() + 60, 5);
    let mut guest = guest(address, &key, &grant);
    assert!(matches!(guest.recv_from_relay().unwrap(), FromRelay::Waiting {}));
    assert!(guest.recv_message().is_err());
}

#[test]
fn idle_bridged_pair_is_closed() {
    let (_server, address, key) = setup(Duration::from_millis(100));
    let grant = signed("session-1", unix_time() + 60, 6);
    let mut guest = guest(address, &key, &grant);
    assert!(matches!(guest.recv_from_relay().unwrap(), FromRelay::Waiting {}));
    let mut host = host(address, &key, &grant, TOKEN);
    assert!(matches!(host.recv_from_relay().unwrap(), FromRelay::Bridged {}));
    assert!(matches!(guest.recv_from_relay().unwrap(), FromRelay::Bridged {}));
    assert!(host.recv_message().is_err());
    assert!(guest.recv_message().is_err());
}

#[test]
fn no_social_configuration_does_not_start_a_listener() {
    assert!(SocialServer::from_options(None, None, None).is_none());
}
