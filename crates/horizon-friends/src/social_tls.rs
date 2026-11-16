//! TLS 1.3 control channels authenticated by a private relay CA and checked SNI.
//!
//! Sockets are nonblocking and waits are short sleep-polls (no kernel socket
//! timeouts). The deadlines (handshake, first message, writes) are `Instant`s
//! checked in the loops, so no helper thread exists per call and an idle channel
//! is never killed. A write that misses its deadline is fatal: bytes handed to rustls are
//! never handed again, so frames cannot be duplicated or misaligned.
use std::{io, net::TcpStream, sync::Arc, time::Duration};

use rustls::{
    ClientConfig, ClientConnection, RootCertStore, ServerConfig, ServerConnection,
    pki_types::{CertificateDer, ServerName},
};

mod channel;
pub use channel::TlsChannel;

pub const MAX_MESSAGE_SIZE: usize = 16 * 1024;
pub const CHANNEL_BINDING_LABEL: &[u8] = b"EXPORTER-yoake-social-binding-v1";
/// Default deadline for a whole `send_message`; missing it closes the channel.
pub const WRITE_TIMEOUT: Duration = Duration::from_secs(10);
/// Default wait of `recv_message`; the relay uses it for the first control message.
pub const FIRST_MESSAGE_TIMEOUT: Duration = Duration::from_secs(5);

/// Per-channel deadlines used after the handshake.
#[derive(Clone, Copy, Debug)]
pub struct ChannelTimeouts {
    /// How long `recv_message` waits for a whole message. Expiry is not fatal.
    pub recv: Duration,
    /// Deadline for one whole `send_message`. Expiry is fatal.
    pub write: Duration,
}

impl Default for ChannelTimeouts {
    fn default() -> Self {
        Self { recv: FIRST_MESSAGE_TIMEOUT, write: WRITE_TIMEOUT }
    }
}

/// The listener's transport-independent control connection contract.
///
/// `channel_binding` differs per hop: the guest-relay and host-relay channels
/// are separate TLS sessions, so a later end-to-end layer must not assume both
/// ends share one value.
pub trait ControlChannel: Send {
    /// Sends one whole message or fails; a failure (including a missed write
    /// deadline) is fatal for the channel.
    fn send_message(&mut self, bytes: &[u8]) -> io::Result<()>;
    /// Waits up to the channel's receive timeout; `WouldBlock` means none
    /// arrived yet and the channel is still usable.
    fn recv_message(&mut self) -> io::Result<Vec<u8>>;
    /// Waits up to `wait`; `Ok(None)` means no whole message yet (partial data is kept).
    fn try_recv_message(&mut self, wait: Duration) -> io::Result<Option<Vec<u8>>>;
    fn channel_binding(&self) -> io::Result<[u8; 32]>;
    fn shutdown(&mut self);
}

/// Converts an accepted TCP socket into a secured control channel.
pub trait ChannelAcceptor: Send + Sync {
    fn accept(&self, stream: TcpStream) -> io::Result<Box<dyn ControlChannel>>;
}

pub type ClientChannel = TlsChannel<ClientConnection>;
pub type ServerChannel = TlsChannel<ServerConnection>;

/// Connect trusting only `ca_certificates` (several may be given so a CA can be
/// rotated with an overlap) and verify `server_name`. `timeout` bounds the
/// handshake and is the default `recv_message` wait.
pub fn client_channel<CA: AsRef<[u8]>>(
    stream: TcpStream,
    ca_certificates: &[CA],
    server_name: &str,
    timeout: Duration,
) -> io::Result<ClientChannel> {
    if ca_certificates.is_empty() {
        return Err(io::Error::new(io::ErrorKind::InvalidInput, "no CA certificate given"));
    }
    let mut roots = RootCertStore::empty();
    for certificate in ca_certificates {
        roots.add(CertificateDer::from(certificate.as_ref().to_vec()))
            .map_err(|error| io::Error::new(io::ErrorKind::InvalidData, error))?;
    }
    let provider = Arc::new(rustls::crypto::ring::default_provider());
    let config = ClientConfig::builder_with_provider(provider)
        .with_protocol_versions(&[&rustls::version::TLS13])
        .map_err(io::Error::other)?
        .with_root_certificates(roots)
        .with_no_client_auth();
    let name = ServerName::try_from(server_name.to_owned())
        .map_err(|error| io::Error::new(io::ErrorKind::InvalidInput, error))?;
    let connection = ClientConnection::new(Arc::new(config), name).map_err(io::Error::other)?;
    let timeouts = ChannelTimeouts { recv: timeout, ..ChannelTimeouts::default() };
    channel::handshake(connection, stream, timeout, timeouts)
}

/// Accept a TLS 1.3 relay connection. Kept here so client and relay share framing.
pub fn server_channel(
    stream: TcpStream,
    config: Arc<ServerConfig>,
    handshake_timeout: Duration,
    timeouts: ChannelTimeouts,
) -> io::Result<ServerChannel> {
    let connection = ServerConnection::new(config).map_err(io::Error::other)?;
    channel::handshake(connection, stream, handshake_timeout, timeouts)
}
