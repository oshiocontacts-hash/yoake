//! Optional listener for the authenticated social control protocol.
use std::{
    collections::HashMap,
    io,
    net::{SocketAddr, TcpListener, TcpStream},
    sync::{Arc, Condvar, Mutex, MutexGuard, atomic::{AtomicBool, Ordering}},
    time::{Duration, Instant, SystemTime, UNIX_EPOCH},
};

use horizon_friends::{
    social::{FromRelay, SignedGrant, ToRelay},
    social_tls::{ChannelAcceptor, ControlChannel},
};
use crate::{
    social::{Direction, ForwardProgress, PairedConnection, PendingGuest, SocialRelay},
    social_tls::{HandshakeLimits, TlsAcceptor, read_certificate_and_key},
};
#[path = "social_server_stream.rs"]
mod stream;
use stream::MessageStream;

type Channel = Box<dyn ControlChannel>;
type Pending = PendingGuest<MessageStream>;
type SharedPending = Arc<(Mutex<HashMap<[u8; 32], PendingEntry>>, Condvar)>;

#[derive(Clone)]
pub struct SocialServer {
    relay: SocialRelay,
    acceptor: Arc<dyn ChannelAcceptor>,
    pending: SharedPending,
    stop: Arc<AtomicBool>,
}

#[derive(Clone, Copy, Eq, PartialEq)]
enum Outcome { Waiting, Bridged, Rejected }
struct PendingEntry {
    pending: Option<Pending>,
    guest: MessageStream,
    outcome: Outcome,
}

impl SocialServer {
    pub fn new(portal_key: [u8; 32], acceptor: Arc<dyn ChannelAcceptor>) -> Self {
        Self {
            relay: SocialRelay::new(portal_key, 1024),
            acceptor,
            pending: Arc::new((Mutex::new(HashMap::new()), Condvar::new())),
            stop: Arc::new(AtomicBool::new(false)),
        }
    }

    /// Portal-to-relay provisioning is intentionally still a function-call API;
    /// its authenticated network channel is not specified yet.
    pub fn relay(&self) -> &SocialRelay { &self.relay }

    /// Missing or invalid settings disable only this independent listener.
    pub fn from_env() -> Option<(SocketAddr, Self)> {
        Self::from_options(
            std::env::var("HORIZON_RELAY_SOCIAL_ADDR").ok(),
            std::env::var("HORIZON_RELAY_SOCIAL_CERT").ok(),
            std::env::var("HORIZON_RELAY_SOCIAL_CERT_KEY").ok(),
            std::env::var("HORIZON_RELAY_PORTAL_KEY").ok(),
        )
    }

    pub fn from_options(address: Option<String>, cert: Option<String>, key: Option<String>, portal: Option<String>) -> Option<(SocketAddr, Self)> {
        let address = match address.and_then(|value| value.parse().ok()) {
            Some(address) => address,
            None => {
                eprintln!("horizon-relay: social listener disabled: invalid or missing HORIZON_RELAY_SOCIAL_ADDR");
                return None;
            }
        };
        let configured = cert.zip(key).and_then(|(cert, key)| {
            let (cert, key) = read_certificate_and_key(cert.as_ref(), key.as_ref()).ok()?;
            TlsAcceptor::from_der(cert, key, HandshakeLimits::default()).ok()
        });
        let portal = portal.and_then(|value| decode_key(&value));
        match (configured, portal) {
            (Some(acceptor), Some(portal)) => Some((address, Self::new(portal, Arc::new(acceptor)))),
            _ => {
                eprintln!("horizon-relay: social listener disabled: invalid or missing certificate/key/portal configuration");
                None
            }
        }
    }

    pub fn spawn_local(&self) -> io::Result<SocketAddr> {
        let listener = TcpListener::bind("127.0.0.1:0")?;
        let address = listener.local_addr()?;
        let server = self.clone();
        std::thread::spawn(move || server.serve(listener));
        Ok(address)
    }

    pub fn serve(&self, listener: TcpListener) {
        let _ = listener.set_nonblocking(true);
        while !self.stop.load(Ordering::Relaxed) {
            match listener.accept() {
                Ok((stream, _)) => {
                    let _ = stream.set_nonblocking(false);
                    let server = self.clone();
                    let _ = std::thread::Builder::new().name("relay-social".into())
                        .spawn(move || { let _ = server.handle(stream); });
                }
                Err(error) if error.kind() == io::ErrorKind::WouldBlock => {
                    std::thread::sleep(Duration::from_millis(5));
                }
                Err(_) => std::thread::sleep(Duration::from_millis(50)),
            }
        }
    }

    pub fn stop(&self) { self.stop.store(true, Ordering::Relaxed); }

    fn handle(&self, stream: TcpStream) -> io::Result<()> {
        let mut channel = self.acceptor.accept(stream)?;
        let request = channel.recv_message().and_then(|bytes| {
            serde_json::from_slice(&bytes).map_err(io::Error::other)
        })?;
        match request {
            ToRelay::Guest { grant } => self.guest(channel, grant),
            ToRelay::Host { grant, token } => self.host(channel, grant, token),
        }
    }

    fn guest(&self, channel: Channel, grant: SignedGrant) -> io::Result<()> {
        let stream = MessageStream::new(channel);
        let Some(key) = grant_key(&grant) else { return reject(stream); };
        let pending = match self.relay.begin_guest(&grant, stream.clone(), unix_time(), Instant::now()) {
            Ok(pending) => pending,
            Err(_) => return reject(stream),
        };
        let entry = PendingEntry { pending: Some(pending), guest: stream.clone(), outcome: Outcome::Waiting };
        let (lock, wake) = &*self.pending;
        lock_unpoison(lock).insert(key, entry);
        if let Err(error) = stream.send_from_relay(&FromRelay::Waiting {}) {
            lock_unpoison(lock).remove(&key);
            stream.shutdown();
            return Err(error);
        }
        let mut guard = lock_unpoison(lock);
        loop {
            let expired = guard.get(&key).and_then(|entry| entry.pending.as_ref())
                .is_some_and(|pending| pending.is_expired(Instant::now()));
            let outcome = guard.get(&key).map(|entry| entry.outcome);
            if expired || outcome != Some(Outcome::Waiting) {
                guard.remove(&key);
                drop(guard);
                if outcome != Some(Outcome::Bridged) { stream.shutdown(); }
                return Ok(());
            }
            guard = wake.wait_timeout(guard, Duration::from_millis(10))
                .unwrap_or_else(|poisoned| poisoned.into_inner()).0;
        }
    }

    fn host(&self, channel: Channel, grant: SignedGrant, token: String) -> io::Result<()> {
        let host = MessageStream::new(channel);
        let Some(key) = grant_key(&grant) else { return reject(host); };
        let (lock, _) = &*self.pending;
        let (result, guest) = {
            let mut guard = lock_unpoison(lock);
            let Some(entry) = guard.get_mut(&key).filter(|entry| entry.outcome == Outcome::Waiting) else {
                drop(guard);
                return reject(host);
            };
            let guest = entry.guest.clone();
            let mut pending = entry.pending.take().expect("waiting entry has a pending guest");
            let result = pending.pair(&grant, &token, host.clone(), unix_time(), Instant::now());
            (result, guest)
        };
        let finish = |outcome| {
            let (lock, wake) = &*self.pending;
            if let Some(entry) = lock_unpoison(lock).get_mut(&key) { entry.outcome = outcome; }
            wake.notify_all();
        };
        match result {
            Ok(mut pair) => {
                let host_sent = host.send_from_relay(&FromRelay::Bridged {}).is_ok();
                let guest_sent = guest.send_from_relay(&FromRelay::Bridged {}).is_ok();
                let bridged = host_sent && guest_sent;
                finish(if bridged { Outcome::Bridged } else { Outcome::Rejected });
                if !bridged {
                    host.shutdown();
                    guest.shutdown();
                    return Ok(());
                }
                forward(&mut pair);
                drop(pair);
                host.shutdown();
                guest.shutdown();
                Ok(())
            }
            Err(_) => {
                let _ = host.send_from_relay(&FromRelay::Rejected {});
                let _ = guest.send_from_relay(&FromRelay::Rejected {});
                finish(Outcome::Rejected);
                host.shutdown();
                guest.shutdown();
                Ok(())
            }
        }
    }
}

fn forward(pair: &mut PairedConnection<MessageStream, MessageStream>) {
    loop {
        let mut active = false;
        for direction in [Direction::GuestToHost, Direction::HostToGuest] {
            match pair.forward_once(direction, Instant::now()) {
                Ok(ForwardProgress::Transferred(_)) => active = true,
                Ok(ForwardProgress::Blocked) => {}
                Ok(ForwardProgress::Eof) | Err(_) => return,
            }
        }
        if !active { std::thread::sleep(Duration::from_millis(2)); }
    }
}

fn reject(stream: MessageStream) -> io::Result<()> {
    let _ = stream.send_from_relay(&FromRelay::Rejected {});
    stream.shutdown();
    Ok(())
}
fn lock_unpoison<T>(mutex: &Mutex<T>) -> MutexGuard<'_, T> {
    mutex.lock().unwrap_or_else(|poisoned| poisoned.into_inner())
}
fn unix_time() -> u64 {
    SystemTime::now().duration_since(UNIX_EPOCH).unwrap_or_default().as_secs()
}
fn grant_key(grant: &SignedGrant) -> Option<[u8; 32]> { decode_key(&grant.grant.nonce) }
fn decode_key(text: &str) -> Option<[u8; 32]> {
    if text.len() != 64 { return None; }
    let mut output = [0; 32];
    for (index, byte) in output.iter_mut().enumerate() {
        *byte = u8::from_str_radix(&text[index * 2..index * 2 + 2], 16).ok()?;
    }
    Some(output)
}
