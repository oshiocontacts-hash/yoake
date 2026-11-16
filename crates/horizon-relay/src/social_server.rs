//! Optional listener for the authenticated social control protocol.
//!
//! A guest registers its grant and waits; a host presenting the same grant and
//! the session token takes the guest over and the host's thread forwards between
//! the two channels. A bad host never disturbs a waiting guest. Documented limits:
//! a waiting guest that disconnects is noticed only when its slot hits the idle
//! limit, a bridged connection can outlive the grant's `exp`, and there is no
//! clock-skew allowance on `exp`.
use std::{
    collections::HashMap,
    io,
    net::{SocketAddr, TcpListener, TcpStream},
    sync::{Arc, Mutex, MutexGuard, atomic::{AtomicBool, AtomicU64, AtomicUsize, Ordering}},
    time::{Duration, Instant, SystemTime, UNIX_EPOCH},
};

use horizon_friends::{
    social::{FromRelay, SignedGrant, ToRelay, decode_hex},
    social_tls::{ChannelAcceptor, ControlChannel},
};
use crate::social::{Direction, ForwardProgress, PairedConnection, PendingGuest, SocialRelay};
#[path = "social_server_config.rs"]
mod config;
#[path = "social_server_stream.rs"]
mod stream;
use stream::MessageStream;

#[cfg(test)]
#[path = "social_client_tests.rs"]
mod client_tests;

type Channel = Box<dyn ControlChannel>;
type Pending = PendingGuest<MessageStream>;
type Key = (String, [u8; 32]);
/// Waiting guests by (session id, grant nonce).
type PendingMap = Arc<Mutex<HashMap<Key, PendingEntry>>>;

const PENDING_POLL: Duration = Duration::from_millis(20);
/// Forwarding naps between 1 ms (after traffic) and this (long idle).
const MAX_IDLE_NAP: Duration = Duration::from_millis(16);
/// Live connection threads (handshaking, waiting or bridging) before new sockets are dropped unread.
const DEFAULT_MAX_CONNECTIONS: usize = 2048;

#[derive(Clone)]
pub struct SocialServer {
    relay: SocialRelay,
    acceptor: Arc<dyn ChannelAcceptor>,
    pending: PendingMap,
    stop: Arc<AtomicBool>,
    live: Arc<AtomicUsize>,
    max_connections: usize,
    ticks: Arc<AtomicU64>,
}

struct PendingEntry {
    pending: Pending,
    guest: MessageStream,
    slot: u64,
}

impl SocialServer {
    pub fn new(portal_key: [u8; 32], acceptor: Arc<dyn ChannelAcceptor>) -> Self {
        Self {
            relay: SocialRelay::new(portal_key, 1024),
            acceptor,
            pending: Arc::new(Mutex::new(HashMap::new())),
            stop: Arc::new(AtomicBool::new(false)),
            live: Arc::new(AtomicUsize::new(0)),
            max_connections: DEFAULT_MAX_CONNECTIONS,
            ticks: Arc::new(AtomicU64::new(0)),
        }
    }

    /// Cap on connection threads; beyond it new sockets are closed before any work.
    pub fn with_max_connections(mut self, max: usize) -> Self {
        self.max_connections = max;
        self
    }

    /// Forwarding loop iterations so far, across all bridges (a cheap measure of idle cost).
    pub fn forward_ticks(&self) -> u64 { self.ticks.load(Ordering::Relaxed) }

    /// Portal-to-relay provisioning is intentionally still a function-call API;
    /// its authenticated network channel is not specified yet.
    pub fn relay(&self) -> &SocialRelay { &self.relay }

    /// Missing or invalid settings disable only this independent listener.
    pub fn from_env() -> Option<(SocketAddr, Self)> {
        let var = |name| std::env::var(name).ok();
        Self::from_options(
            var("HORIZON_RELAY_SOCIAL_ADDR"),
            var("HORIZON_RELAY_SOCIAL_CERT"),
            var("HORIZON_RELAY_SOCIAL_CERT_KEY"),
            var("HORIZON_RELAY_PORTAL_KEY"),
        )
    }

    pub fn from_options(address: Option<String>, cert: Option<String>, key: Option<String>, portal: Option<String>) -> Option<(SocketAddr, Self)> {
        match config::settings(address, cert, key, portal) {
            Ok(Some(settings)) => Some((settings.address, Self::new(settings.portal_key, Arc::new(settings.acceptor)))),
            Ok(None) => {
                println!("horizon-relay: social listener disabled (not configured)");
                None
            }
            Err(reason) => {
                eprintln!("horizon-relay: social listener disabled: {reason}");
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
                    if self.live.fetch_add(1, Ordering::AcqRel) >= self.max_connections {
                        self.live.fetch_sub(1, Ordering::AcqRel);
                        let _ = stream.shutdown(std::net::Shutdown::Both);
                        continue;
                    }
                    let _ = stream.set_nonblocking(false);
                    let server = self.clone();
                    let live = LiveGuard(self.live.clone());
                    // If the spawn fails, the closure (and `live` with it) is dropped.
                    let _ = std::thread::Builder::new().name("relay-social".into())
                        .spawn(move || { let _live = live; let _ = server.handle(stream); });
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
        });
        match request {
            Ok(ToRelay::Guest { grant }) => self.guest(channel, grant),
            Ok(ToRelay::Host { grant, token }) => self.host(channel, grant, token),
            Err(error) => {
                channel.shutdown();
                Err(error)
            }
        }
    }

    fn guest(&self, channel: Channel, grant: SignedGrant) -> io::Result<()> {
        let stream = MessageStream::new(channel);
        let Some(key) = grant_key(&grant) else { return reject(stream); };
        // Verifies the signature first, then reserves the slot; nothing can pair yet.
        let pending = match self.relay.begin_guest(&grant, stream.clone(), unix_time(), Instant::now()) {
            Ok(pending) => pending,
            Err(_) => return reject(stream),
        };
        // `waiting` goes out before the entry exists, so `bridged` can never overtake it.
        if let Err(error) = stream.send_from_relay(&FromRelay::Waiting {}) {
            stream.shutdown();
            return Err(error);
        }
        let (probe, slot) = (pending.probe(), pending.slot_id());
        {
            let mut map = lock_unpoison(&self.pending);
            if map.contains_key(&key) {
                drop(map);
                return reject(stream);
            }
            map.insert(key.clone(), PendingEntry { pending, guest: stream.clone(), slot });
        }
        loop {
            std::thread::sleep(PENDING_POLL);
            let expired = probe.is_expired(Instant::now()); // sessions lock only
            let mut map = lock_unpoison(&self.pending);
            match map.get(&key) {
                Some(entry) if entry.slot == slot => {
                    if expired {
                        let entry = map.remove(&key);
                        drop(map);
                        drop(entry);
                        stream.shutdown();
                        return Ok(());
                    }
                }
                _ => return Ok(()), // a host took it: that thread owns the connection now
            }
        }
    }

    fn host(&self, channel: Channel, grant: SignedGrant, token: String) -> io::Result<()> {
        let host = MessageStream::new(channel);
        let Some(key) = grant_key(&grant) else { return reject(host); };
        // Token and grant are checked before the guest is removed: a failure leaves it waiting.
        let (result, guest) = {
            let mut map = lock_unpoison(&self.pending);
            let Some(entry) = map.get_mut(&key) else {
                drop(map);
                return reject(host);
            };
            let guest = entry.guest.clone();
            let result = entry.pending.pair(&grant, &token, host.clone(), unix_time(), Instant::now());
            let taken = if result.is_ok() { map.remove(&key) } else { None };
            drop(map);
            drop(taken);
            (result, guest)
        };
        let Ok(mut pair) = result else { return reject(host); };
        let bridged = host.send_from_relay(&FromRelay::Bridged {}).is_ok()
            && guest.send_from_relay(&FromRelay::Bridged {}).is_ok();
        if bridged { forward(&mut pair, &self.ticks); }
        drop(pair);
        host.shutdown();
        guest.shutdown();
        Ok(())
    }
}

struct LiveGuard(Arc<AtomicUsize>);
impl Drop for LiveGuard {
    fn drop(&mut self) { self.0.fetch_sub(1, Ordering::AcqRel); }
}

/// Ends when either side closes or fails, or the slot expires. When both directions are
/// idle it backs off, so many idle bridges cost little; traffic resets it to 1 ms.
fn forward(pair: &mut PairedConnection<MessageStream, MessageStream>, ticks: &AtomicU64) {
    let mut nap = Duration::from_millis(1);
    loop {
        ticks.fetch_add(1, Ordering::Relaxed);
        let mut active = false;
        for direction in [Direction::GuestToHost, Direction::HostToGuest] {
            match pair.forward_once(direction, Instant::now()) {
                Ok(ForwardProgress::Transferred(_)) => active = true,
                Ok(ForwardProgress::Blocked) => {}
                Err(_) => return,
            }
        }
        if active {
            nap = Duration::from_millis(1);
        } else {
            std::thread::sleep(nap);
            nap = (nap * 2).min(MAX_IDLE_NAP);
        }
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
/// Unverified input: only lowercase hex of the exact length gets a key.
fn grant_key(grant: &SignedGrant) -> Option<Key> {
    Some((grant.grant.session_id.clone(), decode_hex::<32>(&grant.grant.nonce)?))
}
