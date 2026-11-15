//! rustls adapter and handshake admission limits for the social listener.
use std::{
    collections::HashMap,
    io,
    net::{IpAddr, Shutdown, TcpStream},
    path::Path,
    sync::{Arc, Mutex},
    time::Duration,
};

use horizon_friends::social_tls::{ChannelAcceptor, ControlChannel, server_channel};
use rustls::{ServerConfig, pki_types::{CertificateDer, PrivateKeyDer, PrivatePkcs8KeyDer}};

const MESSAGE_TIMEOUT: Duration = Duration::from_millis(50);

#[derive(Clone, Copy)]
pub struct HandshakeLimits {
    pub total: usize,
    pub per_ip: usize,
    pub timeout: Duration,
}
impl Default for HandshakeLimits {
    fn default() -> Self {
        Self { total: 64, per_ip: 4, timeout: Duration::from_secs(10) }
    }
}

#[derive(Default)]
struct Active {
    total: usize,
    by_ip: HashMap<IpAddr, usize>,
}

pub struct TlsAcceptor {
    config: Arc<ServerConfig>,
    limits: HandshakeLimits,
    active: Arc<Mutex<Active>>,
}

impl TlsAcceptor {
    pub fn new(config: Arc<ServerConfig>, limits: HandshakeLimits) -> Self {
        Self { config, limits, active: Arc::new(Mutex::new(Active::default())) }
    }

    pub fn from_der(certificates: Vec<u8>, key: Vec<u8>, limits: HandshakeLimits) -> io::Result<Self> {
        let config = ServerConfig::builder_with_protocol_versions(&[&rustls::version::TLS13])
            .with_no_client_auth()
            .with_single_cert(
                vec![CertificateDer::from(certificates)],
                PrivateKeyDer::Pkcs8(PrivatePkcs8KeyDer::from(key)),
            )
            .map_err(io::Error::other)?;
        Ok(Self::new(Arc::new(config), limits))
    }

    fn begin(&self, ip: IpAddr) -> io::Result<HandshakeGuard> {
        let mut active = self.active.lock().unwrap_or_else(|poisoned| poisoned.into_inner());
        let count = active.by_ip.get(&ip).copied().unwrap_or(0);
        if active.total >= self.limits.total || count >= self.limits.per_ip {
            return Err(io::Error::new(io::ErrorKind::ConnectionRefused, "social TLS handshake limit reached"));
        }
        active.total += 1;
        *active.by_ip.entry(ip).or_default() += 1;
        Ok(HandshakeGuard { active: self.active.clone(), ip })
    }
}

impl ChannelAcceptor for TlsAcceptor {
    fn accept(&self, stream: TcpStream) -> io::Result<Box<dyn ControlChannel>> {
        let guard = match stream.peer_addr().map_err(io::Error::other)
            .and_then(|peer| self.begin(peer.ip()))
        {
            Ok(guard) => guard,
            Err(error) => {
                let _ = stream.shutdown(Shutdown::Both);
                return Err(error);
            }
        };
        let result = server_channel(stream, self.config.clone(), self.limits.timeout, MESSAGE_TIMEOUT);
        drop(guard);
        result.map(|channel| Box::new(channel) as Box<dyn ControlChannel>)
    }
}

struct HandshakeGuard {
    active: Arc<Mutex<Active>>,
    ip: IpAddr,
}
impl Drop for HandshakeGuard {
    fn drop(&mut self) {
        let mut active = self.active.lock().unwrap_or_else(|poisoned| poisoned.into_inner());
        active.total = active.total.saturating_sub(1);
        if let Some(count) = active.by_ip.get_mut(&self.ip) {
            *count = count.saturating_sub(1);
            if *count == 0 { active.by_ip.remove(&self.ip); }
        }
    }
}

/// DER paths are deliberate: no PEM decoder dependency is needed.
pub fn read_certificate_and_key(cert: &Path, key: &Path) -> io::Result<(Vec<u8>, Vec<u8>)> {
    key_file_is_private(key)?;
    Ok((std::fs::read(cert)?, std::fs::read(key)?))
}

fn key_file_is_private(path: &Path) -> io::Result<()> {
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        if std::fs::metadata(path)?.permissions().mode() & 0o077 != 0 {
            return Err(io::Error::new(io::ErrorKind::PermissionDenied, "social TLS private key is accessible by group or others"));
        }
    }
    // Windows ACLs cannot be represented by Unix mode bits; the deployer manages them.
    let _ = path;
    Ok(())
}

#[cfg(all(test, unix))]
mod tests {
    use super::*;
    use std::os::unix::fs::PermissionsExt;

    #[test]
    fn group_or_other_key_permissions_are_refused() {
        let path = std::env::temp_dir().join(format!("yoake-social-key-{}", std::process::id()));
        std::fs::write(&path, b"test").unwrap();
        let mut permissions = std::fs::metadata(&path).unwrap().permissions();
        permissions.set_mode(0o600);
        std::fs::set_permissions(&path, permissions).unwrap();
        assert!(key_file_is_private(&path).is_ok());
        let mut permissions = std::fs::metadata(&path).unwrap().permissions();
        permissions.set_mode(0o644);
        std::fs::set_permissions(&path, permissions).unwrap();
        assert_eq!(read_certificate_and_key(Path::new("unused"), &path).unwrap_err().kind(), io::ErrorKind::PermissionDenied);
        let _ = std::fs::remove_file(path);
    }
}
