mod social_support;
use horizon_friends::social_tls::{
    ChannelAcceptor, ClientChannel, ControlChannel, MAX_MESSAGE_SIZE, client_channel,
};
use horizon_relay::{social_server::SocialServer, social_tls::{HandshakeLimits, TlsAcceptor}};
use rustls::{ClientConfig, ClientConnection, RootCertStore, StreamOwned, pki_types::{CertificateDer, ServerName}};
use social_support::*;
use std::{
    io::{self, Read, Write},
    net::{Shutdown, SocketAddr, TcpListener, TcpStream},
    sync::Arc,
    time::Duration,
};

fn connection(acceptor: Arc<TlsAcceptor>, tls: &TestTls) -> (ClientChannel, Box<dyn ControlChannel>) {
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let address = listener.local_addr().unwrap();
    let task = std::thread::spawn(move || {
        let (stream, _) = listener.accept().unwrap();
        acceptor.accept(stream).unwrap()
    });
    let client = client(address, tls);
    let server = task.join().unwrap();
    (client, server)
}

fn custom_server(limits: HandshakeLimits) -> (SocialServer, SocketAddr, TestTls) {
    let tls = tls("relay.test");
    let server = server(&tls, limits);
    let address = server.spawn_local().unwrap();
    (server, address, tls)
}

#[test]
fn right_ca_but_wrong_sni_is_refused() {
    let (_server, address, tls) = setup(Duration::from_secs(5));
    let result = client_channel(
        TcpStream::connect(address).unwrap(), &tls.ca, "other.test", Duration::from_secs(2),
    );
    assert!(result.err().is_some());
}

#[test]
fn tls12_only_client_is_refused() {
    let (_server, address, tls) = setup(Duration::from_secs(5));
    let mut roots = RootCertStore::empty();
    roots.add(CertificateDer::from(tls.ca)).unwrap();
    let config = ClientConfig::builder_with_protocol_versions(&[&rustls::version::TLS12])
        .with_root_certificates(roots)
        .with_no_client_auth();
    let connection = ClientConnection::new(
        Arc::new(config), ServerName::try_from("relay.test".to_owned()).unwrap(),
    ).unwrap();
    let socket = TcpStream::connect(address).unwrap();
    socket.set_read_timeout(Some(Duration::from_secs(2))).unwrap();
    let mut stream = StreamOwned::new(connection, socket);
    let mut result = Ok(());
    while stream.conn.is_handshaking() {
        result = stream.conn.complete_io(&mut stream.sock).map(drop);
        if result.is_err() { break; }
    }
    assert!(result.is_err());
}

#[test]
fn incomplete_handshake_times_out_and_closes() {
    let (_server, address, _tls) = custom_server(HandshakeLimits {
        total: 2, per_ip: 2, timeout: Duration::from_millis(100),
    });
    let mut socket = TcpStream::connect(address).unwrap();
    socket.set_read_timeout(Some(Duration::from_secs(2))).unwrap();
    assert_eq!(socket.read(&mut [0; 1]).unwrap_or(0), 0);
}

#[test]
fn close_without_notify_is_an_error() {
    let tls = tls("relay.test");
    let acceptor = Arc::new(TlsAcceptor::from_der(
        tls.cert.clone(), tls.key.clone(), HandshakeLimits::default(),
    ).unwrap());
    let (mut client, server) = connection(acceptor, &tls);
    drop(server);
    assert!(client.recv_message().is_err());
}

#[test]
fn channel_binding_matches_per_connection_and_changes_between_connections() {
    let tls = tls("relay.test");
    let acceptor = Arc::new(TlsAcceptor::from_der(
        tls.cert.clone(), tls.key.clone(), HandshakeLimits::default(),
    ).unwrap());
    let (mut client, mut server) = connection(acceptor.clone(), &tls);
    let first = client.channel_binding().unwrap();
    assert_eq!(first, server.channel_binding().unwrap());
    client.shutdown();
    server.shutdown();
    let (mut client, mut server) = connection(acceptor, &tls);
    assert_ne!(first, client.channel_binding().unwrap());
    assert_eq!(client.channel_binding().unwrap(), server.channel_binding().unwrap());
    client.shutdown();
    server.shutdown();
}

#[test]
fn oversized_frame_is_refused_before_allocation() {
    let tls = tls("relay.test");
    let acceptor = Arc::new(TlsAcceptor::from_der(
        tls.cert.clone(), tls.key.clone(), HandshakeLimits::default(),
    ).unwrap());
    let (mut client, mut server) = connection(acceptor, &tls);
    let error = client.send_message(&vec![0; MAX_MESSAGE_SIZE + 1]).err().unwrap();
    assert_eq!(error.kind(), io::ErrorKind::InvalidInput);
    client.shutdown();
    server.shutdown();
}

#[test]
fn incoming_oversized_length_is_rejected_before_allocation() {
    let tls = tls("relay.test");
    let acceptor = Arc::new(TlsAcceptor::from_der(
        tls.cert.clone(), tls.key.clone(), HandshakeLimits::default(),
    ).unwrap());
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let address = listener.local_addr().unwrap();
    let task = std::thread::spawn(move || {
        let (stream, _) = listener.accept().unwrap();
        acceptor.accept(stream).unwrap()
    });
    let mut roots = RootCertStore::empty();
    roots.add(CertificateDer::from(tls.ca)).unwrap();
    let config = ClientConfig::builder_with_protocol_versions(&[&rustls::version::TLS13])
        .with_root_certificates(roots).with_no_client_auth();
    let connection = ClientConnection::new(
        Arc::new(config), ServerName::try_from("relay.test".to_owned()).unwrap(),
    ).unwrap();
    let mut stream = StreamOwned::new(connection, TcpStream::connect(address).unwrap());
    while stream.conn.is_handshaking() {
        stream.conn.complete_io(&mut stream.sock).unwrap();
    }
    stream.write_all(&((MAX_MESSAGE_SIZE as u32) + 1).to_be_bytes()).unwrap();
    stream.flush().unwrap();
    let mut server = task.join().unwrap();
    assert_eq!(server.recv_message().err().unwrap().kind(), io::ErrorKind::InvalidData);
    server.shutdown();
}

fn flipping_proxy(target: SocketAddr) -> SocketAddr {
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let address = listener.local_addr().unwrap();
    std::thread::spawn(move || {
        let (mut client, _) = listener.accept().unwrap();
        let mut server = TcpStream::connect(target).unwrap();
        let mut reader = server.try_clone().unwrap();
        let mut writer = client.try_clone().unwrap();
        std::thread::spawn(move || { let _ = io::copy(&mut reader, &mut writer); });
        let mut bytes = [0; 4096];
        if let Ok(count) = client.read(&mut bytes) && count > 0 {
            bytes[0] ^= 1;
            let _ = server.write_all(&bytes[..count]);
        }
        let _ = server.shutdown(Shutdown::Both);
    });
    address
}

#[test]
fn flipped_tls_byte_is_rejected() {
    let (_server, address, tls) = setup(Duration::from_secs(5));
    let proxy = flipping_proxy(address);
    let result = client_channel(
        TcpStream::connect(proxy).unwrap(), &tls.ca, "relay.test", Duration::from_secs(2),
    );
    assert!(result.err().is_some());
}

fn limit_refuses_extra(total: usize, per_ip: usize) {
    let (_server, address, _tls) = custom_server(HandshakeLimits {
        total, per_ip, timeout: Duration::from_secs(2),
    });
    let _stalled = TcpStream::connect(address).unwrap();
    std::thread::sleep(Duration::from_millis(50));
    let mut extra = TcpStream::connect(address).unwrap();
    extra.set_read_timeout(Some(Duration::from_secs(1))).unwrap();
    assert_eq!(extra.read(&mut [0; 1]).unwrap_or(0), 0);
}

#[test]
fn total_handshake_limit_refuses_extra_connection() {
    limit_refuses_extra(1, 2);
}

#[test]
fn per_ip_handshake_limit_refuses_extra_connection() {
    limit_refuses_extra(2, 1);
}
