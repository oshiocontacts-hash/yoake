#![allow(dead_code)]
use horizon_friends::{
    social::{FromRelay, Grant, RegisterSession, SignedGrant, ToRelay},
    social_tls::{ClientChannel, ControlChannel, client_channel},
};
use horizon_relay::{
    social::{SessionLimits, SocialRelay},
    social_server::SocialServer,
    social_tls::{HandshakeLimits, TlsAcceptor},
};
use rcgen::{BasicConstraints, CertificateParams, ExtendedKeyUsagePurpose, IsCa, Issuer, KeyPair, KeyUsagePurpose};
use ring::signature::Ed25519KeyPair;
use std::{net::{SocketAddr, TcpStream}, sync::Arc, time::{Duration, SystemTime, UNIX_EPOCH}};

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
    ca_params.is_ca = IsCa::Ca(BasicConstraints::Unconstrained);
    ca_params.key_usages = vec![KeyUsagePurpose::KeyCertSign, KeyUsagePurpose::DigitalSignature];
    let ca_key = KeyPair::generate().unwrap();
    let ca = ca_params.self_signed(&ca_key).unwrap();
    let issuer = Issuer::new(ca_params, ca_key);
    let mut leaf_params = CertificateParams::new(vec![name.to_owned()]).unwrap();
    leaf_params.extended_key_usages = vec![ExtendedKeyUsagePurpose::ServerAuth];
    let leaf_key = KeyPair::generate().unwrap();
    let cert = leaf_params.signed_by(&leaf_key, &issuer).unwrap();
    TestTls {
        ca: ca.der().to_vec(),
        cert: cert.der().to_vec(),
        key: leaf_key.serialize_der(),
    }
}

pub fn portal_key() -> [u8; 32] { hex(PORTAL_PUBLIC) }

pub fn provision(relay: &SocialRelay, idle: Duration) {
    relay.provision(
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
}

pub fn server(tls: &TestTls, limits: HandshakeLimits) -> SocialServer {
    let acceptor = TlsAcceptor::from_der(tls.cert.clone(), tls.key.clone(), limits).unwrap();
    SocialServer::new(portal_key(), Arc::new(acceptor))
}

pub fn setup(idle: Duration) -> (SocialServer, SocketAddr, TestTls) {
    let tls = tls("relay.test");
    let server = server(&tls, HandshakeLimits::default());
    provision(server.relay(), idle);
    let address = server.spawn_local().unwrap();
    (server, address, tls)
}

pub fn client(address: SocketAddr, tls: &TestTls) -> ClientChannel {
    client_channel(TcpStream::connect(address).unwrap(), &tls.ca, "relay.test", Duration::from_secs(2)).unwrap()
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

fn hex<const N: usize>(text: &str) -> [u8; N] {
    let mut output = [0; N];
    for (index, byte) in output.iter_mut().enumerate() {
        *byte = u8::from_str_radix(&text[index * 2..index * 2 + 2], 16).unwrap();
    }
    output
}
