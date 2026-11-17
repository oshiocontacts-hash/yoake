use crate::{
    social::{FromRelay, GrantVerifier, HostGrantVerifier, SignedGrant, ToRelay},
    social_control::{SignedProvision, invalid},
    social_stream::{Bridge, now},
    social_tls::{ChannelTimeouts, ClientChannel, ControlChannel, client_channel},
};
use std::{
    io,
    net::{Shutdown, TcpStream, ToSocketAddrs},
    sync::{
        Arc, Mutex, Weak,
        atomic::{AtomicBool, AtomicU64, Ordering},
    },
    time::{Duration, Instant},
};

const CONNECT_TIMEOUT: Duration = Duration::from_secs(4);
const PAIR_TIMEOUT: Duration = Duration::from_secs(2);
const HOST_RETRY_TIMEOUT: Duration = Duration::from_secs(3);
const RETIRE_TIMEOUT: Duration = Duration::from_millis(1800);

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
        self.connect_until(cancellation, limit, Instant::now() + CONNECT_TIMEOUT)
    }

    fn connect_until(
        &self,
        cancellation: &Cancellation,
        limit: usize,
        deadline: Instant,
    ) -> io::Result<(ClientChannel, Arc<TcpStream>)> {
        if cancellation.is_ended() {
            return Err(invalid());
        }
        let addresses = self
            .authority
            .to_socket_addrs()
            .map_err(|_| unavailable())?;
        for address in addresses {
            let remaining = deadline.saturating_duration_since(Instant::now());
            if remaining.is_zero() || cancellation.is_ended() {
                break;
            }
            let Ok(stream) = TcpStream::connect_timeout(&address, remaining) else {
                continue;
            };
            let socket = Arc::new(stream.try_clone().map_err(|_| unavailable())?);
            cancellation.register(&socket, limit)?;
            let remaining = deadline.saturating_duration_since(Instant::now());
            if remaining.is_zero() {
                return Err(unavailable());
            }
            let mut channel = client_channel(stream, &self.roots, &self.server_name, remaining)
                .map_err(|_| unavailable())?;
            let remaining = deadline.saturating_duration_since(Instant::now());
            channel.set_timeouts(ChannelTimeouts {
                recv: remaining,
                write: remaining,
            });
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

fn send(channel: &mut ClientChannel, request: &ToRelay) -> io::Result<()> {
    channel
        .send_message(&serde_json::to_vec(request).map_err(|_| invalid())?)
        .map_err(|_| unavailable())
}

fn reply(channel: &mut ClientChannel) -> io::Result<FromRelay> {
    serde_json::from_slice(&channel.recv_message().map_err(|_| unavailable())?)
        .map_err(|_| invalid())
}

fn paired(
    channel: &mut ClientChannel,
    cancellation: &Cancellation,
    expires: u64,
    deadline: Instant,
) -> io::Result<()> {
    let deadline = deadline.min(Instant::now() + PAIR_TIMEOUT);
    loop {
        if cancellation.is_ended() || now() >= expires || Instant::now() >= deadline {
            return Err(invalid());
        }
        if let Some(bytes) = channel
            .try_recv_message(Duration::from_millis(16))
            .map_err(|_| unavailable())?
        {
            match serde_json::from_slice(&bytes).map_err(|_| invalid())? {
                FromRelay::Waiting {} => {}
                FromRelay::Bridged {} => return Ok(()),
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
    expires: Arc<AtomicU64>,
    provision: Mutex<SignedProvision>,
    renewal: Mutex<()>,
    retirement: Mutex<()>,
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
        let host = Self {
            verifier: HostGrantVerifier::new(endpoint.public_key, 4096),
            endpoint,
            session_id,
            host_principal,
            host_token,
            cancellation,
            expires: Arc::new(AtomicU64::new(expires)),
            provision: Mutex::new(provision.clone()),
            renewal: Mutex::new(()),
            retirement: Mutex::new(()),
            max_connections,
            retired: AtomicBool::new(false),
        };
        host.reprovision(provision)?;
        Ok(host)
    }

    fn provision_until(&self, provision: &SignedProvision, deadline: Instant) -> io::Result<()> {
        provision.verify(&self.endpoint.public_key, &self.host_token, now())?;
        if provision.ticket.session_id != self.session_id
            || provision.ticket.host_principal != self.host_principal
        {
            return Err(invalid());
        }
        let (mut channel, socket) =
            self.endpoint
                .connect_until(&self.cancellation, self.max_connections + 4, deadline)?;
        let result = (|| {
            send(
                &mut channel,
                &ToRelay::Provision {
                    ticket: provision.ticket.clone(),
                    signature: provision.signature.clone(),
                    host_token: self.host_token.clone(),
                },
            )?;
            if matches!(reply(&mut channel)?, FromRelay::Provisioned {}) {
                Ok(())
            } else {
                Err(invalid())
            }
        })();
        let _ = socket.shutdown(Shutdown::Both);
        result
    }

    pub fn reprovision(&self, provision: SignedProvision) -> io::Result<()> {
        let _renewal = self.renewal.lock().map_err(|_| invalid())?;
        self.provision_until(&provision, Instant::now() + CONNECT_TIMEOUT)?;
        self.expires
            .store(provision.ticket.session_exp, Ordering::Release);
        *self.provision.lock().map_err(|_| invalid())? = provision;
        Ok(())
    }

    pub fn pair(&self, grant: SignedGrant) -> io::Result<Bridge> {
        if self.cancellation.is_ended() || now() >= self.expires_at() {
            return Err(invalid());
        }
        let verified = self
            .verifier
            .begin_client_at(&grant, &self.session_id, &self.host_principal, now())
            .map_err(|_| invalid())?;
        let deadline = Instant::now() + HOST_RETRY_TIMEOUT;
        while !self.cancellation.is_ended()
            && now() < verified.expires_at()
            && now() < self.expires_at()
            && Instant::now() < deadline
        {
            let attempt = (|| {
                let (mut channel, socket) = self.endpoint.connect_until(
                    &self.cancellation,
                    self.max_connections + 4,
                    deadline,
                )?;
                send(
                    &mut channel,
                    &ToRelay::Host {
                        grant: grant.clone(),
                        token: self.host_token.clone(),
                    },
                )?;
                paired(
                    &mut channel,
                    &self.cancellation,
                    verified.expires_at(),
                    deadline,
                )?;
                Ok::<_, io::Error>((channel, socket))
            })();
            if let Ok((channel, socket)) = attempt {
                verified.bridged().map_err(|_| invalid())?;
                return Ok(Bridge::new(channel, socket, Arc::clone(&self.expires)));
            }
            if let Ok(_renewal) = self.renewal.try_lock() {
                let provision = self.provision.lock().map_err(|_| invalid())?.clone();
                let _ = self.provision_until(&provision, deadline);
            }
            std::thread::sleep(Duration::from_millis(100));
        }
        Err(unavailable())
    }

    pub fn expires_at(&self) -> u64 {
        self.expires.load(Ordering::Acquire)
    }

    pub fn end(&self) -> io::Result<()> {
        self.cancellation.end();
        let _retirement = self.retirement.lock().map_err(|_| invalid())?;
        if self.retired.load(Ordering::Acquire) {
            return Ok(());
        }
        let (mut channel, socket) = self.endpoint.connect_until(
            &Cancellation::default(),
            1,
            Instant::now() + RETIRE_TIMEOUT,
        )?;
        let result = (|| {
            send(
                &mut channel,
                &ToRelay::Retire {
                    session_id: self.session_id.clone(),
                    host_token: self.host_token.clone(),
                },
            )?;
            if matches!(reply(&mut channel)?, FromRelay::Retired {}) {
                self.retired.store(true, Ordering::Release);
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
        .verify_client_at(&grant, session_id, &grant.grant.host_principal, now())
        .map_err(|_| invalid())?;
    let (mut channel, socket) = endpoint.connect(cancellation, 1)?;
    send(&mut channel, &ToRelay::Guest { grant })?;
    paired(
        &mut channel,
        cancellation,
        verified.expires_at(),
        Instant::now() + PAIR_TIMEOUT,
    )?;
    Ok(Bridge::new(channel, socket, Arc::new(AtomicU64::new(0))))
}
