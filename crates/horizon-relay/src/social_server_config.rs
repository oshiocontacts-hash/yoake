use std::{net::SocketAddr, path::Path};

use horizon_friends::social::decode_hex;

use crate::social_tls::{HandshakeLimits, TlsAcceptor, read_certificate_and_key};

pub(super) struct Settings {
    pub address: SocketAddr,
    pub acceptor: TlsAcceptor,
    pub portal_key: [u8; 32],
}

/// `Ok(None)`: nothing is configured at all (a normal, quiet state).
/// `Err`: something is set but unusable; the message names what.
pub(super) fn settings(
    address: Option<String>,
    cert: Option<String>,
    key: Option<String>,
    portal: Option<String>,
) -> Result<Option<Settings>, String> {
    if address.is_none() && cert.is_none() && key.is_none() && portal.is_none() {
        return Ok(None);
    }
    let address = address.ok_or("HORIZON_RELAY_SOCIAL_ADDR is missing")?
        .parse().map_err(|_| "HORIZON_RELAY_SOCIAL_ADDR is not a socket address".to_owned())?;
    let cert = cert.ok_or("HORIZON_RELAY_SOCIAL_CERT is missing")?;
    let key = key.ok_or("HORIZON_RELAY_SOCIAL_CERT_KEY is missing")?;
    let portal = portal.ok_or("HORIZON_RELAY_PORTAL_KEY is missing")?;
    let portal_key = decode_hex::<32>(&portal)
        .ok_or("HORIZON_RELAY_PORTAL_KEY must be 64 lowercase hexadecimal characters")?;
    let (cert, key) = read_certificate_and_key(Path::new(&cert), Path::new(&key)).map_err(|error| error.to_string())?;
    let acceptor = TlsAcceptor::from_der(cert, key, HandshakeLimits::default()).map_err(|error| error.to_string())?;
    Ok(Some(Settings { address, acceptor, portal_key }))
}
