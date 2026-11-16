use super::*;
use crate::social_tls::{HandshakeLimits, TlsAcceptor};
use horizon_friends::{
    social::{FromRelay, Grant, ProvisionTicket, ToRelay, decode_hex},
    social_client::{Cancellation, Endpoint, Host, join},
    social_control::SignedProvision,
    social_stream::Bridge,
    social_tls::{ClientChannel, client_channel},
};
use rcgen::{
    BasicConstraints, CertificateParams, ExtendedKeyUsagePurpose, IsCa, Issuer, KeyPair,
    KeyUsagePurpose,
};
use ring::{
    digest::{SHA256, digest},
    signature::Ed25519KeyPair,
};
use std::{
    io::{Read, Write},
    net::Shutdown,
    process::Command,
    sync::mpsc,
    thread::{self, JoinHandle},
};

const TOKEN: &str = "abababababababababababababababababababababababababababababababab";
const SEED: &str = "9d61b19deffd5a60ba844af492ec2cc44449c5697b326919703bac031cae7f60";
const PUBLIC: &str = "d75a980182b10ab7d54bfed3c964073a0ee172f3daa62325af021a68f707511a";
const SESSION: &str = "session-client-test";
const PRINCIPAL: &str = "member:private-test-host";

fn hex(bytes: &[u8]) -> String {
    bytes.iter().map(|byte| format!("{byte:02x}")).collect()
}

fn sign(bytes: &[u8]) -> String {
    hex(
        Ed25519KeyPair::from_seed_unchecked(&decode_hex::<32>(SEED).unwrap())
            .unwrap()
            .sign(bytes)
            .as_ref(),
    )
}

fn issued_at() -> u64 {
    std::env::var("HORIZON_SOCIAL_LOG_TEST_TIME")
        .ok()
        .and_then(|value| value.parse().ok())
        .unwrap_or_else(unix_time)
}

fn grant(nonce: u8) -> SignedGrant {
    let grant = Grant {
        v: 1,
        session_id: SESSION.into(),
        host_principal: PRINCIPAL.into(),
        guest_principal: "guest:private-test-guest".into(),
        mc_profile: None,
        relay_only: true,
        exp: issued_at() + 240,
        nonce: hex(&[nonce; 32]),
    };
    SignedGrant {
        signature: sign(&grant.signing_bytes().unwrap()),
        grant,
    }
}

fn provision() -> SignedProvision {
    let issued = issued_at();
    let ticket = ProvisionTicket {
        v: 1,
        session_id: SESSION.into(),
        host_principal: PRINCIPAL.into(),
        token_sha256: hex(digest(&SHA256, &decode_hex::<32>(TOKEN).unwrap()).as_ref()),
        bytes_per_second: 1_000_000,
        burst_bytes: 65536,
        max_connections: 8,
        idle_secs: 20,
        session_exp: issued + 3600,
        relay_id: "test-relay".into(),
        exp: issued + 240,
        nonce: hex(&[23; 32]),
    };
    SignedProvision {
        signature: sign(&ticket.signing_bytes().unwrap()),
        ticket,
    }
}

fn certificates() -> (Vec<u8>, Vec<u8>, Vec<u8>) {
    let mut ca_params = CertificateParams::new(Vec::<String>::new()).unwrap();
    ca_params.is_ca = IsCa::Ca(BasicConstraints::Constrained(0));
    ca_params.key_usages = vec![
        KeyUsagePurpose::KeyCertSign,
        KeyUsagePurpose::DigitalSignature,
    ];
    let ca_key = KeyPair::generate().unwrap();
    let ca = ca_params.self_signed(&ca_key).unwrap();
    let issuer = Issuer::new(ca_params, ca_key);
    let mut leaf = CertificateParams::new(vec!["127.0.0.1".into()]).unwrap();
    leaf.extended_key_usages = vec![ExtendedKeyUsagePurpose::ServerAuth];
    leaf.key_usages = vec![KeyUsagePurpose::DigitalSignature];
    let leaf_key = KeyPair::generate().unwrap();
    let cert = leaf.signed_by(&leaf_key, &issuer).unwrap();
    (
        ca.der().to_vec(),
        cert.der().to_vec(),
        leaf_key.serialize_der(),
    )
}

struct Fixture {
    endpoint: Endpoint,
    roots: Vec<Vec<u8>>,
    address: SocketAddr,
    server: SocialServer,
    running_server: Arc<Mutex<SocialServer>>,
    stop: Arc<AtomicBool>,
    sockets: Arc<Mutex<Vec<TcpStream>>>,
    listener: Option<JoinHandle<()>>,
    workers: mpsc::Receiver<JoinHandle<()>>,
}

impl Fixture {
    fn new() -> Self {
        let (root, cert, key) = certificates();
        let acceptor = TlsAcceptor::from_der(
            cert,
            key,
            HandshakeLimits {
                per_ip: 64,
                ..HandshakeLimits::default()
            },
        )
        .unwrap();
        let server = SocialServer::new(
            decode_hex::<32>(PUBLIC).unwrap(),
            "test-relay",
            Arc::new(acceptor),
        )
        .unwrap();
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        listener.set_nonblocking(true).unwrap();
        let address = listener.local_addr().unwrap();
        let stop = Arc::new(AtomicBool::new(false));
        let sockets = Arc::new(Mutex::new(Vec::new()));
        let (sender, workers) = mpsc::channel();
        let running = stop.clone();
        let active_sockets = sockets.clone();
        let running_server = Arc::new(Mutex::new(server.clone()));
        let accepting_server = running_server.clone();
        let listener = thread::spawn(move || {
            while !running.load(Ordering::Acquire) {
                match listener.accept() {
                    Ok((socket, _)) => {
                        lock_unpoison(&active_sockets).push(socket.try_clone().unwrap());
                        let server = lock_unpoison(&accepting_server).clone();
                        sender
                            .send(thread::spawn(move || {
                                let _ = server.handle(socket);
                            }))
                            .unwrap();
                    }
                    Err(error) if error.kind() == io::ErrorKind::WouldBlock => {
                        thread::sleep(Duration::from_millis(1))
                    }
                    Err(_) => break,
                }
            }
        });
        let roots = vec![root];
        let endpoint = Endpoint::new(
            address.to_string(),
            roots.clone(),
            decode_hex::<32>(PUBLIC).unwrap(),
        )
        .unwrap();
        Self {
            endpoint,
            roots,
            address,
            server,
            running_server,
            stop,
            sockets,
            listener: Some(listener),
            workers,
        }
    }

    fn restart(&mut self) {
        self.server.relay.retire(SESSION, unix_time()).unwrap();
        for socket in lock_unpoison(&self.sockets).drain(..) {
            let _ = socket.shutdown(Shutdown::Both);
        }
        self.server = SocialServer::new(
            decode_hex::<32>(PUBLIC).unwrap(),
            "test-relay",
            self.server.acceptor.clone(),
        )
        .unwrap();
        *lock_unpoison(&self.running_server) = self.server.clone();
    }

    fn host(&self) -> Arc<Host> {
        Arc::new(
            Host::provision(
                self.endpoint.clone(),
                SESSION.into(),
                PRINCIPAL.into(),
                TOKEN.into(),
                provision(),
                Cancellation::default(),
            )
            .unwrap(),
        )
    }

    fn raw(&self, request: &ToRelay) -> ClientChannel {
        let mut channel = client_channel(
            TcpStream::connect(self.address).unwrap(),
            &self.roots,
            "127.0.0.1",
            Duration::from_secs(2),
        )
        .unwrap();
        channel
            .send_message(&serde_json::to_vec(request).unwrap())
            .unwrap();
        channel
    }

    fn wait_pending(&self, signed: &SignedGrant) {
        let key = grant_key(signed).unwrap();
        let deadline = Instant::now() + Duration::from_secs(3);
        while !lock_unpoison(&self.server.pending).contains_key(&key) {
            assert!(
                Instant::now() < deadline,
                "guest did not enter waiting state"
            );
            thread::sleep(Duration::from_millis(1));
        }
    }

    fn pair(&self, host: &Host, signed: SignedGrant) -> (Bridge, Bridge) {
        let endpoint = self.endpoint.clone();
        let pending = signed.clone();
        let guest =
            thread::spawn(move || join(endpoint, SESSION, pending, &Cancellation::default()));
        self.wait_pending(&signed);
        let host = host.pair(signed).unwrap();
        let guest = guest.join().unwrap().unwrap();
        host.set_read_timeout(Some(Duration::from_secs(4))).unwrap();
        guest
            .set_read_timeout(Some(Duration::from_secs(4)))
            .unwrap();
        (host, guest)
    }
}

impl Drop for Fixture {
    fn drop(&mut self) {
        self.stop.store(true, Ordering::Release);
        if let Some(listener) = self.listener.take() {
            listener.join().unwrap();
        }
        let _ = self.server.relay.retire(SESSION, unix_time());
        for socket in lock_unpoison(&self.sockets).drain(..) {
            let _ = socket.shutdown(Shutdown::Both);
        }
        for worker in self.workers.try_iter() {
            worker.join().unwrap();
        }
    }
}

#[test]
fn social_client_reprovisions_and_recovers_after_relay_restart() {
    let mut fixture = Fixture::new();
    let host = fixture.host();
    let (mut first_host, mut first_guest) = fixture.pair(&host, grant(31));
    let mut renewed = provision();
    renewed.ticket.session_exp += 600;
    renewed.ticket.nonce = hex(&[32; 32]);
    renewed.signature = sign(&renewed.ticket.signing_bytes().unwrap());
    host.reprovision(renewed.clone()).unwrap();
    assert_eq!(host.expires_at(), renewed.ticket.session_exp);
    first_guest.write_all(b"renewed").unwrap();
    let mut received = [0; 7];
    first_host.read_exact(&mut received).unwrap();
    assert_eq!(&received, b"renewed");
    fixture.restart();
    host.reprovision(renewed).unwrap();
    let (mut restored_host, mut restored_guest) = fixture.pair(&host, grant(33));
    restored_guest.write_all(b"restored").unwrap();
    let mut received = [0; 8];
    restored_host.read_exact(&mut received).unwrap();
    assert_eq!(&received, b"restored");
    host.end().unwrap();
}

#[test]
fn social_client_retries_host_until_a_guest_arrives_after_restart() {
    let mut fixture = Fixture::new();
    let host = fixture.host();
    fixture.restart();
    let signed = grant(34);
    let pairing_host = host.clone();
    let pending = signed.clone();
    let pairing = thread::spawn(move || pairing_host.pair(pending));
    thread::sleep(Duration::from_millis(250));
    let mut guest = join(
        fixture.endpoint.clone(),
        SESSION,
        signed,
        &Cancellation::default(),
    )
    .unwrap();
    let mut bridge = pairing.join().unwrap().unwrap();
    guest.write_all(b"retry").unwrap();
    bridge
        .set_read_timeout(Some(Duration::from_secs(1)))
        .unwrap();
    let mut received = [0; 5];
    bridge.read_exact(&mut received).unwrap();
    assert_eq!(&received, b"retry");
    host.end().unwrap();
}

#[test]
fn social_client_failed_pair_can_retry_the_same_grant() {
    let fixture = Fixture::new();
    let host = fixture.host();
    let signed = grant(35);
    assert!(host.pair(signed.clone()).is_err());
    let (mut bridge, mut guest) = fixture.pair(&host, signed);
    guest.write_all(b"unburned").unwrap();
    let mut received = [0; 8];
    bridge.read_exact(&mut received).unwrap();
    assert_eq!(&received, b"unburned");
    host.end().unwrap();
}

#[test]
fn social_client_waiting_guest_has_a_short_pairing_deadline() {
    let fixture = Fixture::new();
    let host = fixture.host();
    let started = Instant::now();
    assert!(
        join(
            fixture.endpoint.clone(),
            SESSION,
            grant(36),
            &Cancellation::default()
        )
        .is_err()
    );
    assert!(started.elapsed() < Duration::from_secs(4));
    host.end().unwrap();
}

#[test]
fn social_client_stalled_write_does_not_block_reads() {
    let fixture = Fixture::new();
    let host = fixture.host();
    let signed = grant(37);
    let mut guest = fixture.raw(&ToRelay::Guest {
        grant: signed.clone(),
    });
    assert!(matches!(
        serde_json::from_slice::<FromRelay>(&guest.recv_message().unwrap()).unwrap(),
        FromRelay::Waiting {}
    ));
    fixture.wait_pending(&signed);
    let mut bridge = host.pair(signed).unwrap();
    assert!(matches!(
        serde_json::from_slice::<FromRelay>(&guest.recv_message().unwrap()).unwrap(),
        FromRelay::Bridged {}
    ));
    let mut writer = bridge.try_clone().unwrap();
    // The test relay forwards 1 MB/s and the guest never reads, so the write
    // stalls once the sockets on the way are full. How much they take is up
    // to the OS (Windows grows loopback send buffers past 16 MiB), so the
    // payload is far more than any of them holds and the write can only end
    // by the bridge being cut.
    let writing = thread::spawn(move || writer.write_all(&vec![57; 128 * 1024 * 1024]));
    thread::sleep(Duration::from_millis(100));
    assert!(!writing.is_finished());
    guest.send_message(b"read while writing").unwrap();
    bridge
        .set_read_timeout(Some(Duration::from_secs(1)))
        .unwrap();
    let mut received = [0; 18];
    bridge.read_exact(&mut received).unwrap();
    assert_eq!(&received, b"read while writing");
    host.end().unwrap();
    assert!(writing.join().unwrap().is_err());
}

#[test]
fn social_client_multiple_guests_and_duplex_stream() {
    let fixture = Fixture::new();
    let host = fixture.host();
    let (mut first_host, mut first_guest) = fixture.pair(&host, grant(1));
    let (mut second_host, mut second_guest) = fixture.pair(&host, grant(2));
    first_guest.write_all(b"first guest").unwrap();
    second_guest.write_all(b"second guest").unwrap();
    let mut first = [0; 11];
    let mut second = [0; 12];
    first_host.read_exact(&mut first).unwrap();
    second_host.read_exact(&mut second).unwrap();
    assert_eq!(&first, b"first guest");
    assert_eq!(&second, b"second guest");
    let mut host_reader = first_host.try_clone().unwrap();
    let mut guest_reader = first_guest.try_clone().unwrap();
    let read_host = thread::spawn(move || {
        let mut bytes = vec![0; 65537];
        host_reader.read_exact(&mut bytes).unwrap();
        bytes
    });
    let read_guest = thread::spawn(move || {
        let mut bytes = vec![0; 65539];
        guest_reader.read_exact(&mut bytes).unwrap();
        bytes
    });
    let write_host = thread::spawn(move || first_host.write_all(&vec![61; 65539]).unwrap());
    let write_guest = thread::spawn(move || first_guest.write_all(&vec![37; 65537]).unwrap());
    write_host.join().unwrap();
    write_guest.join().unwrap();
    assert_eq!(read_host.join().unwrap(), vec![37; 65537]);
    assert_eq!(read_guest.join().unwrap(), vec![61; 65539]);
    host.end().unwrap();
    assert_eq!(second_guest.read(&mut [0]).unwrap(), 0);
}

#[test]
fn social_client_rejects_bad_grant_and_preserves_waiting_guest() {
    let fixture = Fixture::new();
    let host = fixture.host();
    let mut wrong = grant(3);
    wrong.grant.guest_principal = "guest:forged".into();
    assert!(
        join(
            fixture.endpoint.clone(),
            SESSION,
            wrong.clone(),
            &Cancellation::default()
        )
        .is_err()
    );
    let mut rejected = fixture.raw(&ToRelay::Guest { grant: wrong });
    assert!(matches!(
        serde_json::from_slice::<FromRelay>(&rejected.recv_message().unwrap()).unwrap(),
        FromRelay::Rejected {}
    ));
    let signed = grant(4);
    let mut guest = fixture.raw(&ToRelay::Guest {
        grant: signed.clone(),
    });
    assert!(matches!(
        serde_json::from_slice::<FromRelay>(&guest.recv_message().unwrap()).unwrap(),
        FromRelay::Waiting {}
    ));
    fixture.wait_pending(&signed);
    let mut bad_host = fixture.raw(&ToRelay::Host {
        grant: signed.clone(),
        token: "00".repeat(32),
    });
    assert!(matches!(
        serde_json::from_slice::<FromRelay>(&bad_host.recv_message().unwrap()).unwrap(),
        FromRelay::Rejected {}
    ));
    let mut bridge = host.pair(signed).unwrap();
    assert!(matches!(
        serde_json::from_slice::<FromRelay>(&guest.recv_message().unwrap()).unwrap(),
        FromRelay::Bridged {}
    ));
    guest.send_message(b"still waiting safely").unwrap();
    bridge
        .set_read_timeout(Some(Duration::from_secs(3)))
        .unwrap();
    let mut bytes = [0; 20];
    bridge.read_exact(&mut bytes).unwrap();
    assert_eq!(&bytes, b"still waiting safely");
    host.end().unwrap();
}

#[test]
fn social_client_pins_ca_and_cancels_pending_join() {
    let fixture = Fixture::new();
    let (untrusted, _, _) = certificates();
    let wrong_endpoint = Endpoint::new(
        fixture.address.to_string(),
        vec![untrusted],
        decode_hex::<32>(PUBLIC).unwrap(),
    )
    .unwrap();
    assert!(
        Host::provision(
            wrong_endpoint,
            SESSION.into(),
            PRINCIPAL.into(),
            TOKEN.into(),
            provision(),
            Cancellation::default()
        )
        .is_err()
    );
    let host = fixture.host();
    let cancellation = Cancellation::default();
    let pending = cancellation.clone();
    let endpoint = fixture.endpoint.clone();
    let signed = grant(5);
    let waiting = signed.clone();
    let guest = thread::spawn(move || join(endpoint, SESSION, waiting, &pending));
    fixture.wait_pending(&signed);
    cancellation.end();
    assert!(guest.join().unwrap().is_err());
    host.end().unwrap();
}

#[test]
fn social_client_logs_do_not_contain_credentials() {
    const CHILD: &str = "YOAKE_SOCIAL_LOG_TEST_CHILD";
    if std::env::var_os(CHILD).is_some() {
        let fixture = Fixture::new();
        let host = fixture.host();
        let (mut host_stream, mut guest_stream) = fixture.pair(&host, grant(91));
        guest_stream.write_all(b"hello").unwrap();
        host_stream.read_exact(&mut [0; 5]).unwrap();
        let mut wrong = grant(92);
        wrong.signature = "00".repeat(64);
        assert!(
            join(
                fixture.endpoint.clone(),
                SESSION,
                wrong,
                &Cancellation::default()
            )
            .is_err()
        );
        host.end().unwrap();
        return;
    }
    let issued = unix_time();
    let output = Command::new(std::env::current_exe().unwrap())
        .args([
            "--exact",
            "social_server::client_tests::social_client_logs_do_not_contain_credentials",
            "--nocapture",
        ])
        .env(CHILD, "1")
        .env("HORIZON_SOCIAL_LOG_TEST_TIME", issued.to_string())
        .output()
        .unwrap();
    assert!(output.status.success(), "credential-capture child failed");
    let captured = [output.stdout, output.stderr].concat();
    let captured = String::from_utf8_lossy(&captured);
    let mut signed_provision = provision();
    signed_provision.ticket.session_exp = issued + 3600;
    signed_provision.ticket.exp = issued + 240;
    signed_provision.signature = sign(&signed_provision.ticket.signing_bytes().unwrap());
    let mut signed_grant = grant(91);
    signed_grant.grant.exp = issued + 240;
    signed_grant.signature = sign(&signed_grant.grant.signing_bytes().unwrap());
    for secret in [
        TOKEN.to_owned(),
        PRINCIPAL.to_owned(),
        "guest:private-test-guest".into(),
        hex(&[91; 32]),
        hex(&[92; 32]),
        signed_provision.signature,
        signed_grant.signature,
    ] {
        assert!(
            !captured.contains(&secret),
            "credential leaked to captured logs"
        );
    }
}
