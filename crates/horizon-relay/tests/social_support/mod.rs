#![allow(dead_code)]
use horizon_friends::{
    social::{FromRelay, Grant, ProvisionTicket, RegisterSession, SignedGrant, ToRelay},
    social_tls::{ClientChannel, ControlChannel, client_channel},
};
use horizon_relay::{
    social::{SessionLimits, SocialRelay},
    social_server::SocialServer,
    social_tls::{HandshakeLimits, TlsAcceptor},
};
use rcgen::{BasicConstraints, CertificateParams, ExtendedKeyUsagePurpose, IsCa, Issuer, KeyPair, KeyUsagePurpose};
use ring::signature::Ed25519KeyPair;
use rustls::{ClientConfig, ClientConnection, RootCertStore, StreamOwned, pki_types::{CertificateDer, ServerName}};
use std::{io::ErrorKind, net::{SocketAddr, TcpStream}, sync::Arc, time::{Duration, Instant, SystemTime, UNIX_EPOCH}};

pub const RELAY_ID: &str = "relay-1";
pub const TOKEN: &str = "abababababababababababababababababababababababababababababababab";
const PORTAL_SEED: &str = "9d61b19deffd5a60ba844af492ec2cc44449c5697b326919703bac031cae7f60";
const PORTAL_PUBLIC: &str = "d75a980182b10ab7d54bfed3c964073a0ee172f3daa62325af021a68f707511a";

pub struct TestTls {
    pub ca: Vec<u8>,
    pub cert: Vec<u8>,
    pub key: Vec<u8>,
}

pub fn tls(name: &str) -> TestTls {
    let mut ca_params = CertificateParams::new(Vec::<String>::new()).unwrap();
    ca_params.is_ca = IsCa::Ca(BasicConstraints::Constrained(0));
    ca_params.key_usages = vec![KeyUsagePurpose::KeyCertSign, KeyUsagePurpose::DigitalSignature];
    let ca_key = KeyPair::generate().unwrap();
    let ca = ca_params.self_signed(&ca_key).unwrap();
    let issuer = Issuer::new(ca_params, ca_key);
    let mut leaf_params = CertificateParams::new(vec![name.to_owned()]).unwrap();
    leaf_params.extended_key_usages = vec![ExtendedKeyUsagePurpose::ServerAuth];
    leaf_params.key_usages = vec![KeyUsagePurpose::DigitalSignature];
    let leaf_key = KeyPair::generate().unwrap();
    let cert = leaf_params.signed_by(&leaf_key, &issuer).unwrap();
    TestTls {
        ca: ca.der().to_vec(),
        cert: cert.der().to_vec(),
        key: leaf_key.serialize_der(),
    }
}

/// A leaf valid only for the given years, signed by its own CA.
pub fn tls_valid_between(name: &str, from: i32, to: i32) -> TestTls {
    let mut ca_params = CertificateParams::new(Vec::<String>::new()).unwrap();
    ca_params.is_ca = IsCa::Ca(BasicConstraints::Constrained(0));
    ca_params.key_usages = vec![KeyUsagePurpose::KeyCertSign, KeyUsagePurpose::DigitalSignature];
    let ca_key = KeyPair::generate().unwrap();
    let ca = ca_params.self_signed(&ca_key).unwrap();
    let issuer = Issuer::new(ca_params, ca_key);
    let mut leaf_params = CertificateParams::new(vec![name.to_owned()]).unwrap();
    leaf_params.extended_key_usages = vec![ExtendedKeyUsagePurpose::ServerAuth];
    leaf_params.not_before = rcgen::date_time_ymd(from, 1, 1);
    leaf_params.not_after = rcgen::date_time_ymd(to, 1, 1);
    let leaf_key = KeyPair::generate().unwrap();
    let cert = leaf_params.signed_by(&leaf_key, &issuer).unwrap();
    TestTls { ca: ca.der().to_vec(), cert: cert.der().to_vec(), key: leaf_key.serialize_der() }
}

pub fn portal_key() -> [u8; 32] { hex(PORTAL_PUBLIC) }

pub fn provision(relay: &SocialRelay, idle: Duration) {
    provision_with(relay, idle, 1_000_000, 64 * 1024, 64);
}

pub fn provision_with(relay: &SocialRelay, idle: Duration, bytes_per_second: u64, burst_bytes: usize, max_connections: usize) {
    relay.provision(
        &RegisterSession::relay_only("session-1".into(), "member:host".into()),
        TOKEN,
        SessionLimits {
            max_connections,
            max_nonces: 1024,
            bytes_per_second,
            burst_bytes,
            idle_timeout: idle,
        },
        unix_time(),
        std::time::Instant::now(),
    ).unwrap();
}

pub fn server(tls: &TestTls, limits: HandshakeLimits) -> SocialServer {
    let acceptor = TlsAcceptor::from_der(tls.cert.clone(), tls.key.clone(), limits).unwrap();
    SocialServer::new(portal_key(), RELAY_ID, Arc::new(acceptor)).unwrap()
}

pub fn setup(idle: Duration) -> (SocialServer, SocketAddr, TestTls) {
    let tls = tls("relay.test");
    let server = server(&tls, HandshakeLimits::default());
    provision(server.relay(), idle);
    let address = server.spawn_local().unwrap();
    (server, address, tls)
}

/// A relay with no session: hosts have to open one with a ticket.
pub fn setup_unprovisioned() -> (SocialServer, SocketAddr, TestTls) {
    let tls = tls("relay.test");
    let server = server(&tls, HandshakeLimits::default());
    let address = server.spawn_local().unwrap();
    (server, address, tls)
}

/// A provision message signed with the portal test key, for the host token `token`.
pub fn provision_message(session: &str, exp: u64, nonce: u8, token: &str) -> ToRelay {
    provision_message_for(session, "member:host", RELAY_ID, exp, nonce, token)
}

pub fn provision_message_for(session: &str, host: &str, relay_id: &str, exp: u64, nonce: u8, token: &str) -> ToRelay {
    let digest = ring::digest::digest(&ring::digest::SHA256, &hex::<32>(token));
    let ticket = ProvisionTicket {
        v: 1,
        session_id: session.into(),
        host_principal: host.into(),
        token_sha256: digest.as_ref().iter().map(|byte| format!("{byte:02x}")).collect(),
        bytes_per_second: 1_000_000,
        burst_bytes: 64 * 1024,
        max_connections: 64,
        idle_secs: 5,
        session_exp: unix_time() + 3600,
        relay_id: relay_id.into(),
        exp,
        nonce: format!("{nonce:02x}").repeat(32),
    };
    let key = Ed25519KeyPair::from_seed_unchecked(&hex::<32>(PORTAL_SEED)).unwrap();
    let signature = key.sign(&ticket.signing_bytes().unwrap()).as_ref().iter()
        .map(|byte| format!("{byte:02x}")).collect();
    ToRelay::Provision { ticket, signature, host_token: token.into() }
}

/// Send one message on a fresh connection and return the relay's first reply.
pub fn ask(address: SocketAddr, tls: &TestTls, message: &ToRelay) -> FromRelay {
    let mut channel = client(address, tls);
    send_to(&mut channel, message);
    recv_from(&mut channel)
}

pub fn setup_with(idle: Duration, bytes_per_second: u64, burst_bytes: usize, max_connections: usize) -> (SocialServer, SocketAddr, TestTls) {
    let tls = tls("relay.test");
    let server = server(&tls, HandshakeLimits::default());
    provision_with(server.relay(), idle, bytes_per_second, burst_bytes, max_connections);
    let address = server.spawn_local().unwrap();
    (server, address, tls)
}

pub fn client(address: SocketAddr, tls: &TestTls) -> ClientChannel {
    client_channel(TcpStream::connect(address).unwrap(), &[&tls.ca], "relay.test", Duration::from_secs(2)).unwrap()
}

pub fn send_to(channel: &mut dyn ControlChannel, message: &ToRelay) {
    channel.send_message(&serde_json::to_vec(message).unwrap()).unwrap();
}

pub fn recv_from(channel: &mut dyn ControlChannel) -> FromRelay {
    let bytes = channel.recv_message().unwrap();
    serde_json::from_slice(&bytes).unwrap()
}

pub fn guest(address: SocketAddr, tls: &TestTls, grant: &SignedGrant) -> ClientChannel {
    let mut channel = client(address, tls);
    send_to(&mut channel, &ToRelay::Guest { grant: grant.clone() });
    channel
}

pub fn host(address: SocketAddr, tls: &TestTls, grant: &SignedGrant, token: &str) -> ClientChannel {
    let mut channel = client(address, tls);
    send_to(&mut channel, &ToRelay::Host { grant: grant.clone(), token: token.into() });
    channel
}

pub fn signed(session: &str, exp: u64, nonce: u8) -> SignedGrant {
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

pub fn unix_time() -> u64 {
    SystemTime::now().duration_since(UNIX_EPOCH).unwrap().as_secs()
}

pub fn assert_rejected(channel: &mut dyn ControlChannel) {
    assert!(matches!(recv_from(channel), FromRelay::Rejected {}));
}

/// The peer closed the channel (not a client-side timeout, which is `WouldBlock`),
/// and well before the client's own 2 s receive timeout.
pub fn assert_peer_closed(channel: &mut dyn ControlChannel) {
    let started = Instant::now();
    let error = channel.recv_message().unwrap_err();
    assert!(
        matches!(error.kind(), ErrorKind::UnexpectedEof | ErrorKind::ConnectionAborted | ErrorKind::ConnectionReset | ErrorKind::BrokenPipe),
        "expected a peer close, got {error:?}",
    );
    assert!(started.elapsed() < Duration::from_millis(1500), "close took {:?}", started.elapsed());
}

/// A raw TLS 1.3 client with the explicit crypto provider, for tests that bypass `client_channel`.
pub fn raw_client_config(ca: &[u8]) -> Arc<ClientConfig> {
    let mut roots = RootCertStore::empty();
    roots.add(CertificateDer::from(ca.to_vec())).unwrap();
    Arc::new(
        ClientConfig::builder_with_provider(Arc::new(rustls::crypto::ring::default_provider()))
            .with_protocol_versions(&[&rustls::version::TLS13]).unwrap()
            .with_root_certificates(roots)
            .with_no_client_auth(),
    )
}

pub fn raw_client(address: SocketAddr, ca: &[u8]) -> StreamOwned<ClientConnection, TcpStream> {
    let connection = ClientConnection::new(raw_client_config(ca), ServerName::try_from("relay.test".to_owned()).unwrap()).unwrap();
    let socket = TcpStream::connect(address).unwrap();
    socket.set_read_timeout(Some(Duration::from_secs(2))).unwrap();
    let mut stream = StreamOwned::new(connection, socket);
    while stream.conn.is_handshaking() {
        stream.conn.complete_io(&mut stream.sock).unwrap();
    }
    stream
}

/// A signed grant with an arbitrary nonce text (the signature is junk when the text is not valid hex).
pub fn grant_with_nonce(nonce: &str, exp: u64) -> SignedGrant {
    let mut grant = signed("session-1", exp, 0);
    grant.grant.nonce = nonce.to_owned();
    grant.signature = "00".repeat(64);
    grant
}

fn hex<const N: usize>(text: &str) -> [u8; N] {
    let mut output = [0; N];
    for (index, byte) in output.iter_mut().enumerate() {
        *byte = u8::from_str_radix(&text[index * 2..index * 2 + 2], 16).unwrap();
    }
    output
}
