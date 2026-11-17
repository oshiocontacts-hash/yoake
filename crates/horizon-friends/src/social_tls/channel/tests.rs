use std::{net::TcpListener, sync::Arc, thread};

use rustls::{ServerConfig, pki_types::PrivatePkcs8KeyDer};
use socket2::SockRef;

use super::*;
use crate::social_tls::{ClientChannel, ServerChannel, client_channel, server_channel};

fn channels(write_timeout: Duration) -> (ServerChannel, ClientChannel) {
    let certified = rcgen::generate_simple_self_signed(vec!["localhost".into()]).unwrap();
    let certificate = certified.cert.der().clone();
    let config = ServerConfig::builder_with_provider(Arc::new(
        rustls::crypto::ring::default_provider(),
    ))
    .with_protocol_versions(&[&rustls::version::TLS13])
    .unwrap()
    .with_no_client_auth()
    .with_single_cert(
        vec![certificate.clone()],
        PrivatePkcs8KeyDer::from(certified.signing_key.serialize_der()).into(),
    )
    .unwrap();
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let address = listener.local_addr().unwrap();
    let client = thread::spawn(move || {
        let socket = TcpStream::connect(address).unwrap();
        SockRef::from(&socket).set_recv_buffer_size(4096).unwrap();
        client_channel(socket, &[certificate], "localhost", Duration::from_secs(10)).unwrap()
    });
    let (socket, _) = listener.accept().unwrap();
    SockRef::from(&socket).set_send_buffer_size(4096).unwrap();
    let server = server_channel(
        socket,
        Arc::new(config),
        Duration::from_secs(10),
        ChannelTimeouts { recv: Duration::from_secs(10), write: write_timeout },
    )
    .unwrap();
    (server, client.join().unwrap())
}

fn message(sequence: usize) -> Vec<u8> {
    let mut bytes = vec![57; MAX_MESSAGE_SIZE];
    bytes[..8].copy_from_slice(&(sequence as u64).to_be_bytes());
    bytes
}

fn backpressure(server: &mut ServerChannel) -> (usize, Vec<u8>) {
    let deadline = Instant::now() + Duration::from_secs(10);
    let mut completed = 0;
    let mut blocked_since = None;
    loop {
        assert!(Instant::now() < deadline, "socket never reached backpressure");
        let bytes = message(completed);
        if server.try_send_message(&bytes).unwrap() {
            completed += 1;
            blocked_since = None;
        } else if blocked_since.get_or_insert_with(Instant::now).elapsed()
            >= Duration::from_millis(25)
        {
            return (completed, bytes);
        }
        thread::sleep(Duration::from_millis(1));
    }
}

#[test]
fn pending_write_retries_deliver_each_frame_once() {
    let (mut server, mut client) = channels(Duration::from_secs(10));
    let (completed, pending) = backpressure(&mut server);
    let reading = thread::spawn(move || {
        for sequence in 0..=completed {
            assert_eq!(client.recv_message().unwrap(), message(sequence));
        }
        assert_eq!(client.recv_message().unwrap(), b"next message");
    });
    while !server.try_send_message(&pending).unwrap() {
        thread::sleep(Duration::from_millis(1));
    }
    server.send_message(b"next message").unwrap();
    reading.join().unwrap();
}

#[test]
fn pending_write_retries_keep_the_original_deadline() {
    let (mut server, _client) = channels(Duration::from_secs(2));
    let (_, pending) = backpressure(&mut server);
    let deadline = Instant::now() + Duration::from_secs(10);
    loop {
        assert!(Instant::now() < deadline, "retries extended the write deadline");
        match server.try_send_message(&pending) {
            Ok(false) => thread::sleep(Duration::from_millis(1)),
            Ok(true) => panic!("stalled peer unexpectedly accepted the pending frame"),
            Err(error) => {
                assert_eq!(error.kind(), io::ErrorKind::TimedOut);
                break;
            }
        }
    }
    assert_eq!(
        server.try_send_message(&pending).unwrap_err().kind(),
        io::ErrorKind::BrokenPipe,
    );
}

#[test]
fn replacing_a_pending_write_is_fatal() {
    let (mut server, _client) = channels(Duration::from_secs(10));
    let (_, pending) = backpressure(&mut server);
    assert_eq!(
        server.try_send_message(b"different message").unwrap_err().kind(),
        io::ErrorKind::InvalidInput,
    );
    assert_eq!(
        server.try_send_message(&pending).unwrap_err().kind(),
        io::ErrorKind::BrokenPipe,
    );
}
