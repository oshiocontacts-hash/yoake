use crate::{
    social::{GrantVerifier, HostGrantVerifier, SignedGrant},
    social_control::{Reply, Request, SignedProvision, invalid},
    social_stream::{Bridge, now},
    social_tls::{ClientChannel, ControlChannel, client_channel},
};
use std::{
    io,
    net::{Shutdown, TcpStream, ToSocketAddrs},
    sync::{
        Arc, Mutex, Weak,
        atomic::{AtomicBool, Ordering},
    },
    time::{Duration, Instant},
};

const CONNECT_TIMEOUT: Duration = Duration::from_secs(4);

#[derive(Clone, Default)]
pub struct Cancellation(Arc<CancelState>);

#[derive(Default)]
struct CancelState {
    ended: AtomicBool,
    sockets: Mutex<Vec<Weak<TcpStream>>>,
}

impl Cancellation {
    pub fn end(&self) {
        self.0.ended.store(true, Ordering::Release);
        if let Ok(mut sockets) = self.0.sockets.lock() {
            for socket in sockets.drain(..).filter_map(|socket| socket.upgrade()) {
                let _ = socket.shutdown(Shutdown::Both);
            }
        }
    }

    pub fn is_ended(&self) -> bool {
        self.0.ended.load(Ordering::Acquire)
    }

    fn register(&self, socket: &Arc<TcpStream>, limit: usize) -> io::Result<()> {
        let mut sockets = self.0.sockets.lock().map_err(|_| invalid())?;
        sockets.retain(|socket| socket.strong_count() != 0);
        if self.is_ended() || sockets.len() >= limit {
            let _ = socket.shutdown(Shutdown::Both);
            return Err(invalid());
        }
        sockets.push(Arc::downgrade(socket));
        Ok(())
    }
}

#[derive(Clone)]
pub struct Endpoint {
    authority: String,
    server_name: String,
    roots: Vec<Vec<u8>>,
    public_key: [u8; 32],
}

impl Endpoint {
    pub fn new(authority: String, roots: Vec<Vec<u8>>, public_key: [u8; 32]) -> io::Result<Self> {
        let (hostname, port) = authority.rsplit_once(':').ok_or_else(invalid)?;
        let server_name = hostname.trim_start_matches('[').trim_end_matches(']');
        if server_name.is_empty()
            || server_name
                .chars()
                .any(|character| character.is_control() || character.is_whitespace())
            || port.parse::<u16>().ok().filter(|port| *port != 0).is_none()
            || roots.is_empty()
            || roots.len() > 8
        {
            return Err(invalid());
        }
        rustls::pki_types::ServerName::try_from(server_name.to_owned()).map_err(|_| invalid())?;
        let mut store = rustls::RootCertStore::empty();
        for root in &roots {
            store
                .add(rustls::pki_types::CertificateDer::from(root.clone()))
                .map_err(|_| invalid())?;
        }
        Ok(Self {
            server_name: server_name.to_owned(),
            authority,
            roots,
            public_key,
        })
    }

    fn connect(
        &self,
        cancellation: &Cancellation,
        limit: usize,
    ) -> io::Result<(ClientChannel, Arc<TcpStream>)> {
        if cancellation.is_ended() {
            return Err(invalid());
        }
        let started = Instant::now();
        let addresses = self
            .authority
            .to_socket_addrs()
            .map_err(|_| unavailable())?;
        for address in addresses {
            let remaining = CONNECT_TIMEOUT.saturating_sub(started.elapsed());
            if remaining.is_zero() || cancellation.is_ended() {
                break;
            }
            let Ok(stream) = TcpStream::connect_timeout(&address, remaining) else {
                continue;
            };
            let socket = Arc::new(stream.try_clone().map_err(|_| unavailable())?);
            cancellation.register(&socket, limit)?;
            let remaining = CONNECT_TIMEOUT.saturating_sub(started.elapsed());
            if remaining.is_zero() {
                return Err(unavailable());
            }
            let channel = client_channel(stream, &self.roots, &self.server_name, remaining)
                .map_err(|_| unavailable())?;
            if cancellation.is_ended() {
                let _ = socket.shutdown(Shutdown::Both);
                return Err(invalid());
            }
            return Ok((channel, socket));
        }
        Err(unavailable())
    }
}

fn unavailable() -> io::Error {
    io::Error::new(io::ErrorKind::ConnectionAborted, "social relay unavailable")
}

fn send(channel: &mut ClientChannel, request: &Request) -> io::Result<()> {
    channel
        .send_message(&serde_json::to_vec(request).map_err(|_| invalid())?)
        .map_err(|_| unavailable())
}

fn reply(channel: &mut ClientChannel) -> io::Result<Reply> {
    serde_json::from_slice(&channel.recv_message().map_err(|_| unavailable())?)
        .map_err(|_| invalid())
}

fn paired(
    channel: &mut ClientChannel,
    cancellation: &Cancellation,
    expires: u64,
) -> io::Result<()> {
    loop {
        if cancellation.is_ended() || now() >= expires {
            return Err(invalid());
        }
        if let Some(bytes) = channel
            .try_recv_message(Duration::from_millis(16))
            .map_err(|_| unavailable())?
        {
            match serde_json::from_slice(&bytes).map_err(|_| invalid())? {
                Reply::Waiting {} => {}
                Reply::Bridged {} => return Ok(()),
                _ => return Err(invalid()),
            }
        }
    }
}

pub struct Host {
    endpoint: Endpoint,
    session_id: String,
    host_principal: String,
    host_token: String,
    verifier: HostGrantVerifier,
    cancellation: Cancellation,
    expires: u64,
    max_connections: usize,
    retired: AtomicBool,
}

impl Host {
    pub fn provision(
        endpoint: Endpoint,
        session_id: String,
        host_principal: String,
        host_token: String,
        provision: SignedProvision,
        cancellation: Cancellation,
    ) -> io::Result<Self> {
        provision.verify(&endpoint.public_key, &host_token, now())?;
        if provision.ticket.session_id != session_id
            || provision.ticket.host_principal != host_principal
        {
            return Err(invalid());
        }
        let expires = provision.ticket.session_exp;
        let max_connections = provision.ticket.max_connections as usize;
        let (mut channel, socket) = endpoint.connect(&cancellation, max_connections)?;
        send(
            &mut channel,
            &Request::Provision {
                ticket: provision.ticket,
                signature: provision.signature,
                host_token: host_token.clone(),
            },
        )?;
        let result = reply(&mut channel);
        let _ = socket.shutdown(Shutdown::Both);
        if !matches!(result?, Reply::Provisioned {}) {
            return Err(invalid());
        }
        Ok(Self {
            verifier: HostGrantVerifier::new(endpoint.public_key, 4096),
            endpoint,
            session_id,
            host_principal,
            host_token,
            cancellation,
            expires,
            max_connections,
            retired: AtomicBool::new(false),
        })
    }

    pub fn pair(&self, grant: SignedGrant) -> io::Result<Bridge> {
        if self.cancellation.is_ended() || now() >= self.expires {
            return Err(invalid());
        }
        let verified = self
            .verifier
            .authorize_at(&grant, &self.session_id, &self.host_principal, now())
            .map_err(|_| invalid())?;
        let (mut channel, socket) = self
            .endpoint
            .connect(&self.cancellation, self.max_connections)?;
        send(
            &mut channel,
            &Request::Host {
                grant,
                token: self.host_token.clone(),
            },
        )?;
        paired(&mut channel, &self.cancellation, verified.expires_at())?;
        Ok(Bridge::new(channel, socket, self.expires))
    }

    pub fn expires_at(&self) -> u64 {
        self.expires
    }

    pub fn end(&self) -> io::Result<()> {
        self.cancellation.end();
        if self.retired.swap(true, Ordering::AcqRel) {
            return Ok(());
        }
        let (mut channel, socket) = self.endpoint.connect(&Cancellation::default(), 1)?;
        let result = (|| {
            send(
                &mut channel,
                &Request::Retire {
                    session_id: self.session_id.clone(),
                    host_token: self.host_token.clone(),
                },
            )?;
            if matches!(reply(&mut channel)?, Reply::Retired {}) {
                Ok(())
            } else {
                Err(invalid())
            }
        })();
        let _ = socket.shutdown(Shutdown::Both);
        result
    }
}

impl Drop for Host {
    fn drop(&mut self) {
        self.cancellation.end();
    }
}

pub fn join(
    endpoint: Endpoint,
    session_id: &str,
    grant: SignedGrant,
    cancellation: &Cancellation,
) -> io::Result<Bridge> {
    let verified = GrantVerifier::new(endpoint.public_key)
        .verify_at(&grant, session_id, &grant.grant.host_principal, now())
        .map_err(|_| invalid())?;
    let (mut channel, socket) = endpoint.connect(cancellation, 1)?;
    send(&mut channel, &Request::Guest { grant })?;
    paired(&mut channel, cancellation, verified.expires_at())?;
    Ok(Bridge::new(channel, socket, 0))
}
