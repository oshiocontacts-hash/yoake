mod social_support;
use horizon_friends::social_tls::{
    ChannelAcceptor, ChannelTimeouts, ClientChannel, ControlChannel, MAX_MESSAGE_SIZE, client_channel,
};
use horizon_relay::{social_server::SocialServer, social_tls::{HandshakeLimits, TlsAcceptor}};
use rustls::{AlertDescription, CertificateError, ClientConfig, ClientConnection, Error as TlsError, RootCertStore, StreamOwned};
use rustls::pki_types::{CertificateDer, ServerName};
use social_support::*;
use std::{
    io::{self, ErrorKind, Read, Write},
    net::{Shutdown, SocketAddr, TcpListener, TcpStream},
    sync::Arc,
    time::{Duration, Instant},
};

fn acceptor(tls: &TestTls) -> Arc<TlsAcceptor> {
    Arc::new(TlsAcceptor::from_der(tls.cert.clone(), tls.key.clone(), HandshakeLimits::default()).unwrap())
}

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

fn tls_error(error: &io::Error) -> Option<&TlsError> {
    error.get_ref().and_then(|inner| inner.downcast_ref::<TlsError>())
}

/// The relay ended the connection at once: EOF or reset, not our own read timeout.
fn assert_socket_closed(socket: &mut TcpStream, within: Duration) {
    socket.set_read_timeout(Some(Duration::from_secs(3))).unwrap();
    let started = Instant::now();
    match socket.read(&mut [0; 1]) {
        Ok(0) => {}
        Err(error) if error.kind() == ErrorKind::ConnectionReset || error.kind() == ErrorKind::ConnectionAborted => {}
        other => panic!("expected the relay to close the socket, got {other:?}"),
    }
    assert!(started.elapsed() < within, "close took {:?}", started.elapsed());
}

fn assert_socket_open(socket: &mut TcpStream) {
    socket.set_read_timeout(Some(Duration::from_millis(300))).unwrap();
    let error = socket.read(&mut [0; 1]).unwrap_err();
    assert!(matches!(error.kind(), ErrorKind::WouldBlock | ErrorKind::TimedOut), "{error:?}");
}

#[test]
fn right_ca_but_wrong_sni_is_refused() {
    let (_server, address, tls) = setup(Duration::from_secs(5));
    let error = client_channel(
        TcpStream::connect(address).unwrap(), &[&tls.ca], "other.test", Duration::from_secs(2),
    ).err().unwrap();
    assert!(matches!(
        tls_error(&error),
        Some(TlsError::InvalidCertificate(CertificateError::NotValidForName | CertificateError::NotValidForNameContext { .. }))
    ), "{error:?}");
}

#[test]
fn tls12_only_client_is_refused() {
    let (_server, address, tls) = setup(Duration::from_secs(5));
    let mut roots = RootCertStore::empty();
    roots.add(CertificateDer::from(tls.ca)).unwrap();
    let config = ClientConfig::builder_with_provider(Arc::new(rustls::crypto::ring::default_provider()))
        .with_protocol_versions(&[&rustls::version::TLS12]).unwrap()
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
    let error = result.unwrap_err();
    assert!(matches!(
        tls_error(&error),
        Some(TlsError::AlertReceived(AlertDescription::ProtocolVersion | AlertDescription::HandshakeFailure))
    ), "{error:?}");
}

#[test]
fn incomplete_handshake_times_out_and_closes() {
    let (_server, address, _tls) = custom_server(HandshakeLimits {
        total: 2, per_ip: 2, timeout: Duration::from_millis(200),
    });
    let mut socket = TcpStream::connect(address).unwrap();
    assert_socket_closed(&mut socket, Duration::from_millis(1500));
}

#[test]
fn close_without_notify_is_unexpected_eof() {
    let tls = tls("relay.test");
    let (mut client, server) = connection(acceptor(&tls), &tls);
    drop(server);
    assert_eq!(client.recv_message().unwrap_err().kind(), ErrorKind::UnexpectedEof);
}

#[test]
fn close_with_notify_is_reported_as_an_orderly_close() {
    let tls = tls("relay.test");
    let (mut client, mut server) = connection(acceptor(&tls), &tls);
    server.shutdown();
    assert_eq!(client.recv_message().unwrap_err().kind(), ErrorKind::ConnectionAborted);
}

#[test]
fn a_receive_timeout_is_not_fatal_and_keeps_the_channel_usable() {
    let tls = tls("relay.test");
    let (mut client, mut server) = connection(acceptor(&tls), &tls);
    assert!(client.try_recv_message(Duration::from_millis(300)).unwrap().is_none());
    server.send_message(b"after the wait").unwrap();
    assert_eq!(client.recv_message().unwrap(), b"after the wait");
    client.send_message(b"back").unwrap();
    assert_eq!(server.try_recv_message(Duration::from_secs(2)).unwrap().unwrap(), b"back");
}

#[test]
fn channel_binding_matches_per_connection_and_changes_between_connections() {
    let tls = tls("relay.test");
    let acceptor = acceptor(&tls);
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
    let (mut client, mut server) = connection(acceptor(&tls), &tls);
    let error = client.send_message(&vec![0; MAX_MESSAGE_SIZE + 1]).err().unwrap();
    assert_eq!(error.kind(), ErrorKind::InvalidInput);
    client.shutdown();
    server.shutdown();
}

#[test]
fn incoming_oversized_length_is_rejected_before_allocation() {
    let tls = tls("relay.test");
    let acceptor = acceptor(&tls);
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let address = listener.local_addr().unwrap();
    let task = std::thread::spawn(move || {
        let (stream, _) = listener.accept().unwrap();
        acceptor.accept(stream).unwrap()
    });
    let mut stream = raw_client(address, &tls.ca);
    stream.write_all(&((MAX_MESSAGE_SIZE as u32) + 1).to_be_bytes()).unwrap();
    stream.flush().unwrap();
    let mut server = task.join().unwrap();
    assert_eq!(server.recv_message().err().unwrap().kind(), ErrorKind::InvalidData);
    server.shutdown();
}

/// Forwards client bytes to `target`, flipping the last byte (inside the AEAD tag)
/// of the second TLS application-data record: the first is the client's encrypted
/// Finished, the second is the first control message, after the handshake.
fn flipping_proxy(target: SocketAddr) -> SocketAddr {
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let address = listener.local_addr().unwrap();
    std::thread::spawn(move || {
        let (mut client, _) = listener.accept().unwrap();
        let mut server = TcpStream::connect(target).unwrap();
        let mut reader = server.try_clone().unwrap();
        let mut writer = client.try_clone().unwrap();
        std::thread::spawn(move || { let _ = io::copy(&mut reader, &mut writer); });
        let (mut header, mut remaining, mut application, mut flip) = (Vec::new(), 0usize, 0, false);
        let mut bytes = [0; 4096];
        while let Ok(count) = client.read(&mut bytes) {
            if count == 0 { break; }
            for byte in &mut bytes[..count] {
                if remaining == 0 {
                    header.push(*byte);
                    if header.len() == 5 {
                        remaining = u16::from_be_bytes([header[3], header[4]]) as usize;
                        if header[0] == 0x17 {
                            application += 1;
                            flip = application == 2;
                        }
                        header.clear();
                    }
                } else {
                    remaining -= 1;
                    if flip && remaining == 0 {
                        *byte ^= 1;
                        flip = false;
                    }
                }
            }
            if server.write_all(&bytes[..count]).is_err() { break; }
        }
        let _ = server.shutdown(Shutdown::Both);
    });
    address
}

#[test]
fn a_flipped_byte_in_a_post_handshake_record_is_rejected() {
    let tls = tls("relay.test");
    let acceptor = acceptor(&tls);
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let proxy = flipping_proxy(listener.local_addr().unwrap());
    let task = std::thread::spawn(move || {
        let (stream, _) = listener.accept().unwrap();
        // The bad record may arrive in the same read as the client's Finished, so it can
        // surface from `accept` itself; either way the relay must refuse it.
        acceptor.accept(stream).and_then(|mut server| server.recv_message().map(drop))
    });
    // The handshake itself is untouched and completes.
    let mut client = client_channel(TcpStream::connect(proxy).unwrap(), &[&tls.ca], "relay.test", Duration::from_secs(2)).unwrap();
    client.send_message(b"tampered in flight").unwrap();
    let error = task.join().unwrap().unwrap_err();
    assert!(matches!(tls_error(&error), Some(TlsError::DecryptError)), "{error:?}");
    // The relay answers with an alert and closes: the client sees an error, not a timeout.
    let started = Instant::now();
    let error = client.recv_message().unwrap_err();
    assert_ne!(error.kind(), ErrorKind::WouldBlock, "{error:?}");
    assert!(started.elapsed() < Duration::from_millis(1500));
}

#[test]
fn per_source_limit_refuses_the_extra_connection_and_keeps_the_first() {
    let (_server, address, _tls) = custom_server(HandshakeLimits {
        total: 8, per_ip: 1, timeout: Duration::from_secs(3),
    });
    let mut stalled = TcpStream::connect(address).unwrap();
    std::thread::sleep(Duration::from_millis(100));
    let mut extra = TcpStream::connect(address).unwrap();
    assert_socket_closed(&mut extra, Duration::from_millis(1500));
    assert_socket_open(&mut stalled);
}

#[test]
fn a_full_total_limit_drops_the_oldest_handshake_for_the_new_one() {
    let (_server, address, _tls) = custom_server(HandshakeLimits {
        total: 1, per_ip: 4, timeout: Duration::from_secs(3),
    });
    let mut oldest = TcpStream::connect(address).unwrap();
    std::thread::sleep(Duration::from_millis(100));
    let mut newest = TcpStream::connect(address).unwrap();
    assert_socket_closed(&mut oldest, Duration::from_millis(1500));
    assert_socket_open(&mut newest);
}

#[test]
fn handshake_slots_are_released_once_the_handshake_completes() {
    let (_server, address, tls) = custom_server(HandshakeLimits {
        total: 1, per_ip: 1, timeout: Duration::from_secs(3),
    });
    let first = client(address, &tls);
    std::thread::sleep(Duration::from_millis(300)); // the relay finishes its side
    // `first` stays connected and idle; its handshake no longer counts.
    let second = client(address, &tls);
    drop((first, second));
}

#[test]
fn writing_to_a_peer_that_never_reads_times_out_and_is_fatal() {
    let tls = tls("relay.test");
    let (mut client, _server) = connection(acceptor(&tls), &tls);
    client.set_timeouts(ChannelTimeouts { recv: Duration::from_secs(2), write: Duration::from_millis(300) });
    let started = Instant::now();
    let mut failure = None;
    for _ in 0..20_000 {
        if let Err(error) = client.send_message(&[7; MAX_MESSAGE_SIZE]) {
            failure = Some(error);
            break;
        }
    }
    let error = failure.expect("the write buffers cannot be unbounded");
    assert_eq!(error.kind(), ErrorKind::TimedOut);
    assert!(started.elapsed() < Duration::from_secs(30));
    // Nothing is retried on the same channel: bytes already handed to TLS are never repeated.
    assert_eq!(client.send_message(b"again").unwrap_err().kind(), ErrorKind::BrokenPipe);
}

#[test]
fn a_slow_reader_gets_every_message_once_and_in_order() {
    let tls = tls("relay.test");
    let (mut client, mut server) = connection(acceptor(&tls), &tls);
    const COUNT: u32 = 300;
    let message = |index: u32| {
        let mut bytes = vec![(index % 251) as u8; MAX_MESSAGE_SIZE];
        bytes[..4].copy_from_slice(&index.to_be_bytes());
        bytes
    };
    let reader = std::thread::spawn(move || {
        std::thread::sleep(Duration::from_millis(500)); // the writer fills every buffer meanwhile
        for index in 0..COUNT {
            let bytes = server.recv_message().unwrap();
            assert_eq!(bytes, message(index), "message {index} is duplicated, lost or misaligned");
        }
    });
    for index in 0..COUNT {
        client.send_message(&message(index)).unwrap();
    }
    reader.join().unwrap();
}

#[test]
fn certificate_and_key_are_checked_at_startup() {
    let good = tls("relay.test");
    let other = tls("relay.test");
    let limits = HandshakeLimits::default;
    let error = TlsAcceptor::from_der(good.cert.clone(), other.key.clone(), limits()).err().unwrap();
    assert!(error.to_string().contains("does not match"), "{error}");
    let expired = tls_valid_between("relay.test", 2015, 2020);
    let error = TlsAcceptor::from_der(expired.cert, expired.key, limits()).err().unwrap();
    assert!(error.to_string().contains("expired"), "{error}");
    let future = tls_valid_between("relay.test", 2090, 2095);
    let error = TlsAcceptor::from_der(future.cert, future.key, limits()).err().unwrap();
    assert!(error.to_string().contains("not valid yet"), "{error}");
    assert!(TlsAcceptor::from_der(b"junk".to_vec(), good.key, limits()).is_err());
}
