use std::{
    collections::VecDeque,
    net::TcpListener,
    sync::{Arc, Mutex},
    thread,
};

use rustls::{
    ClientConfig, ClientConnection, RootCertStore, ServerConfig, ServerConnection,
    pki_types::{CertificateDer, PrivatePkcs8KeyDer, ServerName},
};

use super::*;
use crate::social_tls::{ClientChannel, ServerChannel, client_channel, server_channel};

pub(super) trait TestTransport: io::Read + io::Write + Send {
    fn shutdown(&self, how: Shutdown) -> io::Result<()>;
}

impl TestTransport for TcpStream {
    fn shutdown(&self, how: Shutdown) -> io::Result<()> {
        TcpStream::shutdown(self, how)
    }
}

struct BufferedBytes {
    bytes: VecDeque<u8>,
    capacity: usize,
    frozen: bool,
    closed: bool,
}

#[derive(Clone)]
struct MemoryPipe(Arc<Mutex<BufferedBytes>>);

impl MemoryPipe {
    fn new(capacity: usize) -> Self {
        Self(Arc::new(Mutex::new(BufferedBytes {
            bytes: VecDeque::new(),
            capacity,
            frozen: false,
            closed: false,
        })))
    }

    fn freeze(&self, frozen: bool) {
        self.0.lock().unwrap().frozen = frozen;
    }

    fn len(&self) -> usize {
        self.0.lock().unwrap().bytes.len()
    }
}

struct MemoryStream {
    incoming: MemoryPipe,
    outgoing: MemoryPipe,
}

impl io::Read for MemoryStream {
    fn read(&mut self, bytes: &mut [u8]) -> io::Result<usize> {
        if bytes.is_empty() {
            return Ok(0);
        }
        let mut pipe = self.incoming.0.lock().unwrap();
        if pipe.frozen || (pipe.bytes.is_empty() && !pipe.closed) {
            return Err(io::ErrorKind::WouldBlock.into());
        }
        let count = bytes.len().min(pipe.bytes.len());
        for byte in &mut bytes[..count] {
            *byte = pipe.bytes.pop_front().unwrap();
        }
        Ok(count)
    }
}

impl io::Write for MemoryStream {
    fn write(&mut self, bytes: &[u8]) -> io::Result<usize> {
        if bytes.is_empty() {
            return Ok(0);
        }
        let mut pipe = self.outgoing.0.lock().unwrap();
        if pipe.closed {
            return Err(io::ErrorKind::BrokenPipe.into());
        }
        let count = bytes.len().min(pipe.capacity - pipe.bytes.len());
        if count == 0 {
            return Err(io::ErrorKind::WouldBlock.into());
        }
        pipe.bytes.extend(&bytes[..count]);
        Ok(count)
    }

    fn flush(&mut self) -> io::Result<()> {
        Ok(())
    }
}

impl TestTransport for MemoryStream {
    fn shutdown(&self, how: Shutdown) -> io::Result<()> {
        if matches!(how, Shutdown::Read | Shutdown::Both) {
            self.incoming.0.lock().unwrap().closed = true;
        }
        if matches!(how, Shutdown::Write | Shutdown::Both) {
            self.outgoing.0.lock().unwrap().closed = true;
        }
        Ok(())
    }
}

impl Drop for MemoryStream {
    fn drop(&mut self) {
        self.shutdown(Shutdown::Both).unwrap();
    }
}

fn configs() -> (
    Arc<ServerConfig>,
    Arc<ClientConfig>,
    CertificateDer<'static>,
) {
    let certified = rcgen::generate_simple_self_signed(vec!["localhost".into()]).unwrap();
    let certificate = certified.cert.der().clone();
    let server =
        ServerConfig::builder_with_provider(Arc::new(rustls::crypto::ring::default_provider()))
            .with_protocol_versions(&[&rustls::version::TLS13])
            .unwrap()
            .with_no_client_auth()
            .with_single_cert(
                vec![certificate.clone()],
                PrivatePkcs8KeyDer::from(certified.signing_key.serialize_der()).into(),
            )
            .unwrap();
    let mut roots = RootCertStore::empty();
    roots.add(certificate.clone()).unwrap();
    let client =
        ClientConfig::builder_with_provider(Arc::new(rustls::crypto::ring::default_provider()))
            .with_protocol_versions(&[&rustls::version::TLS13])
            .unwrap()
            .with_root_certificates(roots)
            .with_no_client_auth();
    (Arc::new(server), Arc::new(client), certificate)
}

fn memory_channel<C>(conn: C, sock: MemoryStream, timeouts: ChannelTimeouts) -> TlsChannel<C> {
    TlsChannel {
        conn,
        sock: Box::new(sock),
        incoming: Vec::new(),
        pending_send: None,
        timeouts,
        broken: false,
    }
}

fn channels(write_timeout: Duration) -> (ServerChannel, ClientChannel, MemoryPipe) {
    let (server_config, client_config, _) = configs();
    let server_to_client = MemoryPipe::new(4096);
    let client_to_server = MemoryPipe::new(4096);
    let timeouts = ChannelTimeouts {
        recv: Duration::from_secs(10),
        write: write_timeout,
    };
    let mut server = memory_channel(
        ServerConnection::new(server_config).unwrap(),
        MemoryStream {
            incoming: client_to_server.clone(),
            outgoing: server_to_client.clone(),
        },
        timeouts,
    );
    let mut client = memory_channel(
        ClientConnection::new(client_config, ServerName::try_from("localhost").unwrap()).unwrap(),
        MemoryStream {
            incoming: server_to_client.clone(),
            outgoing: client_to_server.clone(),
        },
        timeouts,
    );
    for _ in 0..64 {
        server.write_step().unwrap();
        client.write_step().unwrap();
        server.read_step(Duration::ZERO).unwrap();
        client.read_step(Duration::ZERO).unwrap();
        if !server.conn.is_handshaking()
            && !client.conn.is_handshaking()
            && !server.conn.wants_write()
            && !client.conn.wants_write()
            && server_to_client.len() == 0
            && client_to_server.len() == 0
        {
            assert_eq!(
                server.channel_binding().unwrap(),
                client.channel_binding().unwrap()
            );
            return (server, client, server_to_client);
        }
    }
    panic!("in-memory TLS handshake did not finish");
}

#[test]
fn tcp_channels_send_and_receive() {
    let (config, _, certificate) = configs();
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let address = listener.local_addr().unwrap();
    let client = thread::spawn(move || {
        let socket = TcpStream::connect(address).unwrap();
        client_channel(socket, &[certificate], "localhost", Duration::from_secs(10)).unwrap()
    });
    let (socket, _) = listener.accept().unwrap();
    let mut server = server_channel(
        socket,
        config,
        Duration::from_secs(10),
        ChannelTimeouts::default(),
    )
    .unwrap();
    let mut client = client.join().unwrap();
    server.send_message(b"server message").unwrap();
    assert_eq!(client.recv_message().unwrap(), b"server message");
    client.send_message(b"client message").unwrap();
    assert_eq!(server.recv_message().unwrap(), b"client message");
    assert_eq!(
        server.channel_binding().unwrap(),
        client.channel_binding().unwrap()
    );
}

fn message(sequence: usize) -> Vec<u8> {
    let mut bytes = vec![57; MAX_MESSAGE_SIZE];
    bytes[..8].copy_from_slice(&(sequence as u64).to_be_bytes());
    bytes
}

fn backpressure(
    server: &mut ServerChannel,
    client: &mut ClientChannel,
    pipe: &MemoryPipe,
) -> Vec<u8> {
    pipe.freeze(true);
    let pending = message(0);
    assert!(!server.try_send_message(&pending).unwrap());
    assert_eq!(pipe.len(), 4096);
    assert_eq!(client.try_recv_message(Duration::ZERO).unwrap(), None);
    assert_eq!(pipe.len(), 4096);
    pending
}

fn finish_pending(
    server: &mut ServerChannel,
    client: &mut ClientChannel,
    pipe: &MemoryPipe,
    pending: &[u8],
) -> Vec<Vec<u8>> {
    pipe.freeze(false);
    let mut received = Vec::new();
    for _ in 0..32 {
        let complete = server.try_send_message(pending).unwrap();
        while let Some(bytes) = client.try_recv_message(Duration::ZERO).unwrap() {
            received.push(bytes);
        }
        if complete {
            return received;
        }
    }
    panic!("pending frame did not finish after unfreezing the peer");
}

#[test]
fn pending_write_retries_deliver_each_frame_once() {
    let (mut server, mut client, pipe) = channels(Duration::from_secs(10));
    server.send_message(b"before pending").unwrap();
    let pending = backpressure(&mut server, &mut client, &pipe);
    for _ in 0..8 {
        assert!(!server.try_send_message(&pending).unwrap());
    }
    assert_eq!(
        finish_pending(&mut server, &mut client, &pipe, &pending),
        vec![b"before pending".to_vec(), pending]
    );
    server.send_message(b"next message").unwrap();
    assert_eq!(client.recv_message().unwrap(), b"next message");
    assert_eq!(client.try_recv_message(Duration::ZERO).unwrap(), None);
}

#[test]
fn pending_write_retries_keep_the_original_deadline() {
    let (mut server, mut client, pipe) = channels(Duration::from_secs(10));
    let pending = backpressure(&mut server, &mut client, &pipe);
    let deadline = server.pending_send.as_ref().unwrap().deadline;
    server.set_timeouts(ChannelTimeouts {
        write: Duration::ZERO,
        ..server.timeouts
    });
    for _ in 0..8 {
        assert!(!server.try_send_message(&pending).unwrap());
        assert_eq!(server.pending_send.as_ref().unwrap().deadline, deadline);
    }
    server.set_timeouts(ChannelTimeouts {
        write: Duration::from_secs(10),
        ..server.timeouts
    });
    server.pending_send.as_mut().unwrap().deadline = Instant::now() - Duration::from_millis(1);
    assert_eq!(
        server.try_send_message(&pending).unwrap_err().kind(),
        io::ErrorKind::TimedOut,
    );
    assert_eq!(
        server.try_send_message(&pending).unwrap_err().kind(),
        io::ErrorKind::BrokenPipe,
    );
}

#[test]
fn flushed_pending_write_succeeds_after_deadline() {
    let (mut server, mut client, _) = channels(Duration::from_secs(10));
    let bytes = b"finished before retry";
    let mut frame = Vec::from((bytes.len() as u32).to_be_bytes());
    frame.extend_from_slice(bytes);
    server.conn.writer().write_all(&frame).unwrap();
    server.flush_until(after(Duration::from_secs(2))).unwrap();
    server.pending_send = Some(PendingSend {
        offset: frame.len(),
        frame,
        deadline: Instant::now() - Duration::from_millis(1),
    });

    assert!(server.try_send_message(bytes).unwrap());
    assert_eq!(client.recv_message().unwrap(), bytes);
    server.send_message(b"next message").unwrap();
    assert_eq!(client.recv_message().unwrap(), b"next message");
}

#[test]
fn queued_operations_do_not_interleave_with_pending_write() {
    let (mut server, mut client, pipe) = channels(Duration::from_secs(10));
    let pending = backpressure(&mut server, &mut client, &pipe);
    assert_eq!(
        server.queue_message(b"other").unwrap_err().kind(),
        io::ErrorKind::WouldBlock
    );
    assert_eq!(
        server.try_flush().unwrap_err().kind(),
        io::ErrorKind::WouldBlock
    );

    assert_eq!(
        finish_pending(&mut server, &mut client, &pipe, &pending),
        vec![pending]
    );
    server.queue_message(b"after pending").unwrap();
    assert!(server.try_flush().unwrap());
    assert_eq!(client.recv_message().unwrap(), b"after pending");
    assert_eq!(client.try_recv_message(Duration::ZERO).unwrap(), None);
}

#[test]
fn shutdown_with_pending_write_closes_channel() {
    let (mut server, mut client, pipe) = channels(Duration::from_secs(10));
    let pending = backpressure(&mut server, &mut client, &pipe);

    server.shutdown();

    assert_eq!(
        server.try_send_message(&pending).unwrap_err().kind(),
        io::ErrorKind::BrokenPipe
    );
    assert_eq!(
        server.queue_message(b"after shutdown").unwrap_err().kind(),
        io::ErrorKind::BrokenPipe
    );
    assert_eq!(
        server.try_flush().unwrap_err().kind(),
        io::ErrorKind::BrokenPipe
    );
}

#[test]
fn replacing_a_pending_write_is_fatal() {
    let (mut server, mut client, pipe) = channels(Duration::from_secs(10));
    let pending = backpressure(&mut server, &mut client, &pipe);
    assert_eq!(
        server
            .try_send_message(b"different message")
            .unwrap_err()
            .kind(),
        io::ErrorKind::InvalidInput,
    );
    assert_eq!(
        server.try_send_message(&pending).unwrap_err().kind(),
        io::ErrorKind::BrokenPipe,
    );
}
