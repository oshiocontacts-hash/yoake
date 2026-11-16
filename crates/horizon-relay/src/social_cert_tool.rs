//! Creation and offline renewal of the social listener's private-CA certificates.
use rcgen::{
    BasicConstraints, CertificateParams, DistinguishedName, DnType, ExtendedKeyUsagePurpose,
    GeneralSubtree, IsCa, Issuer, KeyPair, KeyUsagePurpose, NameConstraints, PublicKeyData,
};
use rustls::{
    RootCertStore,
    client::{WebPkiServerVerifier, danger::ServerCertVerifier},
    pki_types::{CertificateDer, ServerName, UnixTime},
};
use std::{
    io,
    path::{Path, PathBuf},
    sync::Arc,
    time::{Duration, SystemTime, UNIX_EPOCH},
};

#[path = "social_cert_files.rs"]
mod files;
use files::{check_dir_not_shared, create_private_dir, ensure_absent, path_error, remove_if_present, replace_pairs, sync_dir, write_all_new};

pub const LEAF_VALIDITY_SECONDS: u64 = 30 * 24 * 60 * 60;
/// The leaf starts this long before "now", so players whose clock runs a bit behind still accept it.
pub const LEAF_BACKDATE_SECONDS: u64 = 60 * 60;
const CA_WARNING_SECONDS: i64 = 90 * 24 * 60 * 60;
const CA_VALIDITY_SECONDS: u64 = 5 * 365 * 24 * 60 * 60;
const FILES: [&str; 4] = ["ca.der", "ca-key.der", "leaf.der", "leaf-key.der"];
const BASIC_CONSTRAINTS_OID: &[u8] = &[0x55, 0x1d, 0x13];
const CA_BASIC_CONSTRAINTS: &[u8] = &[0x30, 0x06, 0x01, 0x01, 0xff, 0x02, 0x01, 0x00];

fn rcgen_error(error: rcgen::Error) -> io::Error {
    io::Error::new(io::ErrorKind::InvalidData, format!("certificate generation failed: {error}"))
}

fn validate_host(host: &str) -> io::Result<String> {
    let host = host.to_ascii_lowercase();
    if host.ends_with('.') {
        return Err(io::Error::new(io::ErrorKind::InvalidInput, "--host must not end with a dot"));
    }
    match ServerName::try_from(host.clone()) {
        Ok(ServerName::DnsName(_)) => Ok(host),
        _ => Err(io::Error::new(io::ErrorKind::InvalidInput, "--host must be a valid DNS name, not an IP address")),
    }
}

/// Check the leaf the way a pinning client would: chain to `ca_der`, the CA's
/// name constraint, and the host name.
fn verify_leaf(ca_der: &[u8], leaf_der: &[u8], host: &str) -> io::Result<()> {
    let invalid = |message: String| io::Error::new(io::ErrorKind::InvalidData, message);
    let mut roots = RootCertStore::empty();
    roots.add(CertificateDer::from(ca_der.to_vec())).map_err(|error| invalid(format!("ca.der is not usable as a trust anchor: {error}")))?;
    let provider = Arc::new(rustls::crypto::ring::default_provider());
    let verifier = WebPkiServerVerifier::builder_with_provider(Arc::new(roots), provider)
        .build()
        .map_err(|error| invalid(format!("can't build the certificate verifier: {error}")))?;
    let name = ServerName::try_from(host.to_owned()).map_err(|error| invalid(format!("bad host: {error}")))?;
    let leaf = CertificateDer::from(leaf_der.to_vec());
    verifier
        .verify_server_cert(&leaf, &[], &name, &[], UnixTime::now())
        .map(|_| ())
        .map_err(|error| invalid(format!("the new leaf for {host} would not be accepted by clients pinning ca.der ({error}); is --host the name this CA was created for?")))
}

fn ca_params(host: &str, now: SystemTime) -> io::Result<CertificateParams> {
    let mut params = CertificateParams::new(Vec::<String>::new()).map_err(rcgen_error)?;
    params.distinguished_name = DistinguishedName::new();
    params.distinguished_name.push(DnType::CommonName, "Yoake social relay CA");
    params.is_ca = IsCa::Ca(BasicConstraints::Constrained(0));
    params.key_usages = vec![KeyUsagePurpose::KeyCertSign, KeyUsagePurpose::CrlSign];
    params.name_constraints = Some(NameConstraints {
        permitted_subtrees: vec![GeneralSubtree::DnsName(host.to_owned())],
        excluded_subtrees: vec![GeneralSubtree::DnsName(format!(".{host}"))],
    });
    params.use_authority_key_identifier_extension = true;
    params.not_before = now.into();
    params.not_after = (now + Duration::from_secs(CA_VALIDITY_SECONDS)).into();
    Ok(params)
}

fn leaf_pair(host: &str, issuer: &Issuer<'_, KeyPair>, now: SystemTime) -> io::Result<(Vec<u8>, Vec<u8>)> {
    let mut params = CertificateParams::new(vec![host.to_owned()]).map_err(rcgen_error)?;
    params.is_ca = IsCa::ExplicitNoCa;
    params.extended_key_usages = vec![ExtendedKeyUsagePurpose::ServerAuth];
    params.key_usages = vec![KeyUsagePurpose::DigitalSignature];
    params.use_authority_key_identifier_extension = true;
    params.not_before = (now - Duration::from_secs(LEAF_BACKDATE_SECONDS)).into();
    params.not_after = (now + Duration::from_secs(LEAF_VALIDITY_SECONDS)).into();
    let key = KeyPair::generate().map_err(rcgen_error)?;
    let certificate = params.signed_by(&key, issuer).map_err(rcgen_error)?;
    Ok((certificate.der().to_vec(), key.serialize_der()))
}

/// Create a new five-year CA and 30-day leaf, without replacing any existing file.
pub fn generate(dir: &Path, host: &str) -> io::Result<()> {
    let host = validate_host(host)?;
    create_private_dir(dir)?;
    for name in FILES { ensure_absent(&dir.join(name))?; }
    let now = SystemTime::now();
    let params = ca_params(&host, now)?;
    let ca_key = KeyPair::generate().map_err(rcgen_error)?;
    let ca = params.self_signed(&ca_key).map_err(rcgen_error)?;
    let issuer = Issuer::new(params, ca_key);
    let (leaf, leaf_key) = leaf_pair(&host, &issuer, now)?;
    verify_leaf(ca.der().as_ref(), &leaf, &host)?;
    let ca_key_der = issuer.key().serialize_der();
    write_all_new(&[
        (dir.join("ca.der"), ca.der().as_ref(), false),
        (dir.join("ca-key.der"), ca_key_der.as_ref(), true),
        (dir.join("leaf.der"), &leaf, false),
        (dir.join("leaf-key.der"), &leaf_key, true),
    ])?;
    sync_dir(dir)
}

/// Renew only the leaf. The new pair is verified against `ca.der` and `host`
/// first; each current file is then swapped by one atomic rename.
pub fn renew(dir: &Path, host: &str) -> io::Result<()> {
    let host = validate_host(host)?;
    check_dir_not_shared(dir)?;
    let (ca, ca_key) = crate::social_cert::read_certificate_and_key(&dir.join("ca.der"), &dir.join("ca-key.der"))?;
    for name in ["leaf.der", "leaf-key.der"] {
        std::fs::metadata(dir.join(name)).map_err(|error| path_error(error, &dir.join(name)))?;
    }
    let now = SystemTime::now();
    let unix_now = now.duration_since(UNIX_EPOCH).unwrap_or_default().as_secs() as i64;
    crate::social_cert::check_validity(&ca, unix_now)?;
    let ca_left = crate::social_cert::certificate_validity(&ca)?.1 - unix_now;
    if ca_left < CA_WARNING_SECONDS {
        eprintln!("WARNING: the CA expires in {} days. Start the CA rotation now (docs/yoake-social-ops.md); once it expires, renewal stops and clients must already pin the new CA.", ca_left / 86_400);
    }
    let ca_key = KeyPair::try_from(ca_key.as_slice()).map_err(rcgen_error)?;
    if crate::social_cert::certificate_public_key(&ca)? != ca_key.subject_public_key_info().as_slice() {
        return Err(io::Error::new(io::ErrorKind::InvalidData, "ca.der does not match ca-key.der"));
    }
    if crate::social_cert::certificate_extension(&ca, BASIC_CONSTRAINTS_OID)? != CA_BASIC_CONSTRAINTS {
        return Err(io::Error::new(io::ErrorKind::InvalidData, "ca.der is not a pathLen 0 CA"));
    }
    let issuer = Issuer::new(ca_params(&host, now)?, ca_key);
    let (leaf, leaf_key) = leaf_pair(&host, &issuer, now)?;
    verify_leaf(&ca, &leaf, &host)?;
    let cert_new = dir.join("leaf.der.new");
    let key_new = dir.join("leaf-key.der.new");
    // Leftovers of an interrupted earlier run.
    remove_if_present(&cert_new)?;
    remove_if_present(&key_new)?;
    write_all_new(&[(cert_new.clone(), &leaf, false), (key_new.clone(), &leaf_key, true)])?;
    let result = replace_pairs(dir, &[
        (dir.join("leaf.der"), cert_new.clone(), dir.join("leaf.der.old")),
        (dir.join("leaf-key.der"), key_new.clone(), dir.join("leaf-key.der.old")),
    ]);
    if result.is_err() {
        let _ = std::fs::remove_file(&cert_new);
        let _ = std::fs::remove_file(&key_new);
    }
    result
}

pub fn generated_paths(dir: &Path) -> [PathBuf; 4] {
    FILES.map(|name| dir.join(name))
}

#[cfg(test)]
#[path = "social_cert_tool_tests.rs"]
mod tests;
