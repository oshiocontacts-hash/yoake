//! TLS 1.3 control channels authenticated by a private relay CA and checked SNI.
//! Sockets stay blocking; handshake and every message operation have deadlines.
use std::{
    io::{self, Read, Write},
    net::{Shutdown, TcpStream},
    ops::DerefMut,
    sync::Arc,
    time::Duration,
};

use rustls::{
    ClientConfig, ClientConnection, ConnectionCommon, RootCertStore, ServerConfig, ServerConnection,
    SideData, StreamOwned,
    pki_types::{CertificateDer, ServerName},
};

pub const MAX_MESSAGE_SIZE: usize = 16 * 1024;
pub const CHANNEL_BINDING_LABEL: &[u8] = b"EXPORTER-yoake-social-binding-v1";

/// The listener's transport-independent control connection contract.
pub trait ControlChannel: Send {
    fn send_message(&mut self, bytes: &[u8]) -> io::Result<()>;
    fn recv_message(&mut self) -> io::Result<Vec<u8>>;
    fn channel_binding(&self) -> io::Result<[u8; 32]>;
    fn shutdown(&mut self);
}

/// Converts an accepted TCP socket into a secured control channel.
pub trait ChannelAcceptor: Send + Sync {
    fn accept(&self, stream: TcpStream) -> io::Result<Box<dyn ControlChannel>>;
}

pub struct TlsChannel<C> {
    stream: StreamOwned<C, TcpStream>,
    timeout: Duration,
    incoming: Vec<u8>,
}

pub type ClientChannel = TlsChannel<ClientConnection>;
pub type ServerChannel = TlsChannel<ServerConnection>;

/// Connect using only `ca_certificate` as a trust root and verify `server_name`.
pub fn client_channel(
    stream: TcpStream,
    ca_certificate: &[u8],
    server_name: &str,
    timeout: Duration,
) -> io::Result<ClientChannel> {
    let mut roots = RootCertStore::empty();
    roots.add(CertificateDer::from(ca_certificate.to_vec()))
        .map_err(|error| io::Error::new(io::ErrorKind::InvalidData, error))?;
    let config = ClientConfig::builder_with_protocol_versions(&[&rustls::version::TLS13])
        .with_root_certificates(roots)
        .with_no_client_auth();
    let name = ServerName::try_from(server_name.to_owned())
        .map_err(|error| io::Error::new(io::ErrorKind::InvalidInput, error))?;
    let connection = ClientConnection::new(Arc::new(config), name).map_err(io::Error::other)?;
    handshake(connection, stream, timeout, timeout)
}

/// Accept a TLS 1.3 relay connection. Kept here so client and relay share framing.
pub fn server_channel(
    stream: TcpStream,
    config: Arc<ServerConfig>,
    handshake_timeout: Duration,
    message_timeout: Duration,
) -> io::Result<ServerChannel> {
    let connection = ServerConnection::new(config).map_err(io::Error::other)?;
    handshake(connection, stream, handshake_timeout, message_timeout)
}

fn handshake<C, S>(
    connection: C,
    stream: TcpStream,
    handshake_timeout: Duration,
    message_timeout: Duration,
) -> io::Result<TlsChannel<C>>
where
    C: DerefMut<Target = ConnectionCommon<S>>,
    S: SideData,
{
    stream.set_nonblocking(false)?;
    stream.set_read_timeout(Some(handshake_timeout))?;
    stream.set_write_timeout(Some(handshake_timeout))?;
    let deadline = crate::deadline(&stream, handshake_timeout)?;
    let mut stream = StreamOwned::new(connection, stream);
    while stream.conn.is_handshaking() {
        stream.conn.complete_io(&mut stream.sock)?;
    }
    drop(deadline);
    stream.sock.set_read_timeout(Some(message_timeout))?;
    stream.sock.set_write_timeout(Some(message_timeout))?;
    Ok(TlsChannel { stream, timeout: message_timeout, incoming: Vec::new() })
}

impl<C, S> TlsChannel<C>
where
    C: DerefMut<Target = ConnectionCommon<S>>,
    S: SideData,
{
    /// TLS 1.3 exporter output for the later end-to-end layer; no context is used.
    pub fn channel_binding(&self) -> io::Result<[u8; 32]> {
        self.stream.conn.export_keying_material([0; 32], CHANNEL_BINDING_LABEL, None)
            .map_err(io::Error::other)
    }

    fn deadline(&self) -> io::Result<crate::Deadline> {
        crate::deadline(&self.stream.sock, self.timeout)
    }

    fn take_frame(&mut self) -> io::Result<Option<Vec<u8>>> {
        if self.incoming.len() < 4 { return Ok(None); }
        let length = u32::from_be_bytes(self.incoming[..4].try_into().unwrap()) as usize;
        if length > MAX_MESSAGE_SIZE {
            return Err(io::Error::new(io::ErrorKind::InvalidData, "social TLS frame is too large"));
        }
        if self.incoming.len() < length + 4 { return Ok(None); }
        let frame: Vec<u8> = self.incoming.drain(..length + 4).collect();
        Ok(Some(frame[4..].to_vec()))
    }
}

impl<C, S> ControlChannel for TlsChannel<C>
where
    C: DerefMut<Target = ConnectionCommon<S>> + Send,
    S: SideData,
{
    fn send_message(&mut self, bytes: &[u8]) -> io::Result<()> {
        if bytes.len() > MAX_MESSAGE_SIZE {
            return Err(io::Error::new(io::ErrorKind::InvalidInput, "social TLS message is too large"));
        }
        let _deadline = self.deadline()?;
        self.stream.write_all(&(bytes.len() as u32).to_be_bytes())?;
        self.stream.write_all(bytes)?;
        self.stream.flush()
    }

    fn recv_message(&mut self) -> io::Result<Vec<u8>> {
        let _deadline = self.deadline()?;
        loop {
            if let Some(frame) = self.take_frame()? { return Ok(frame); }
            let mut bytes = [0; 8192];
            match self.stream.read(&mut bytes) {
                Ok(0) => return Err(io::Error::new(io::ErrorKind::UnexpectedEof, "TLS peer closed the channel")),
                Ok(count) => self.incoming.extend_from_slice(&bytes[..count]),
                Err(error) if matches!(error.kind(), io::ErrorKind::WouldBlock | io::ErrorKind::TimedOut) => {
                    return Err(io::Error::new(io::ErrorKind::WouldBlock, "social TLS message deadline elapsed"));
                }
                Err(error) => return Err(error),
            }
        }
    }

    fn channel_binding(&self) -> io::Result<[u8; 32]> { TlsChannel::channel_binding(self) }

    fn shutdown(&mut self) {
        self.stream.conn.send_close_notify();
        let _ = self.stream.flush();
        let _ = self.stream.sock.shutdown(Shutdown::Both);
    }
}
