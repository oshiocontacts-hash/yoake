//! Creation and offline renewal of the social listener's private-CA certificates.
use rcgen::{
    BasicConstraints, CertificateParams, DistinguishedName, DnType, ExtendedKeyUsagePurpose,
    GeneralSubtree, IsCa, Issuer, KeyPair, KeyUsagePurpose, NameConstraints, PublicKeyData,
};
use rustls::pki_types::ServerName;
use std::{
    fs::OpenOptions,
    io::{self, Write},
    path::{Path, PathBuf},
    time::{Duration, SystemTime, UNIX_EPOCH},
};

pub const LEAF_VALIDITY_SECONDS: u64 = 30 * 24 * 60 * 60;
const CA_VALIDITY_SECONDS: u64 = 5 * 365 * 24 * 60 * 60;
const FILES: [&str; 4] = ["ca.der", "ca-key.der", "leaf.der", "leaf-key.der"];
const BASIC_CONSTRAINTS_OID: &[u8] = &[0x55, 0x1d, 0x13];
const CA_BASIC_CONSTRAINTS: &[u8] = &[0x30, 0x06, 0x01, 0x01, 0xff, 0x02, 0x01, 0x00];

fn rcgen_error(error: rcgen::Error) -> io::Error {
    io::Error::new(io::ErrorKind::InvalidData, format!("certificate generation failed: {error}"))
}

fn validate_host(host: &str) -> io::Result<()> {
    match ServerName::try_from(host.to_owned()) {
        Ok(ServerName::DnsName(_)) => Ok(()),
        _ => Err(io::Error::new(io::ErrorKind::InvalidInput, "--host must be a valid DNS name, not an IP address")),
    }
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
    params.not_before = now.into();
    params.not_after = (now + Duration::from_secs(LEAF_VALIDITY_SECONDS)).into();
    let key = KeyPair::generate().map_err(rcgen_error)?;
    let certificate = params.signed_by(&key, issuer).map_err(rcgen_error)?;
    Ok((certificate.der().to_vec(), key.serialize_der()))
}

fn path_error(error: io::Error, path: &Path) -> io::Error {
    io::Error::new(error.kind(), format!("{}: {error}", path.display()))
}

fn ensure_absent(path: &Path) -> io::Result<()> {
    match std::fs::metadata(path) {
        Ok(_) => Err(io::Error::new(io::ErrorKind::AlreadyExists, format!("refusing to overwrite {}", path.display()))),
        Err(error) if error.kind() == io::ErrorKind::NotFound => Ok(()),
        Err(error) => Err(path_error(error, path)),
    }
}

fn write_new(path: &Path, bytes: &[u8], private: bool) -> io::Result<()> {
    let mut options = OpenOptions::new();
    options.write(true).create_new(true);
    #[cfg(not(unix))]
    let _ = private;
    #[cfg(unix)]
    if private {
        use std::os::unix::fs::OpenOptionsExt;
        options.mode(0o600);
    }
    let mut file = options.open(path).map_err(|error| path_error(error, path))?;
    file.write_all(bytes).and_then(|()| file.sync_all()).map_err(|error| path_error(error, path))
}

/// Create a new five-year CA and 30-day leaf, without replacing any existing file.
pub fn generate(dir: &Path, host: &str) -> io::Result<()> {
    validate_host(host)?;
    std::fs::create_dir_all(dir).map_err(|error| path_error(error, dir))?;
    for name in FILES { ensure_absent(&dir.join(name))?; }
    let now = SystemTime::now();
    let params = ca_params(host, now)?;
    let ca_key = KeyPair::generate().map_err(rcgen_error)?;
    let ca = params.self_signed(&ca_key).map_err(rcgen_error)?;
    let issuer = Issuer::new(params, ca_key);
    let (leaf, leaf_key) = leaf_pair(host, &issuer, now)?;
    write_new(&dir.join("ca.der"), ca.der().as_ref(), false)?;
    write_new(&dir.join("ca-key.der"), issuer.key().serialize_der().as_ref(), true)?;
    write_new(&dir.join("leaf.der"), &leaf, false)?;
    write_new(&dir.join("leaf-key.der"), &leaf_key, true)?;
    Ok(())
}

fn replace_with_backup(current: &Path, temporary: &Path, old: &Path) -> io::Result<()> {
    match std::fs::remove_file(old) {
        Ok(()) => {}
        Err(error) if error.kind() == io::ErrorKind::NotFound => {}
        Err(error) => return Err(path_error(error, old)),
    }
    std::fs::rename(current, old).map_err(|error| path_error(error, current))?;
    if let Err(error) = std::fs::rename(temporary, current) {
        let _ = std::fs::rename(old, current);
        return Err(path_error(error, current));
    }
    Ok(())
}

/// Renew only the leaf. Each new file is completely written before an atomic rename.
pub fn renew(dir: &Path, host: &str) -> io::Result<()> {
    validate_host(host)?;
    let (ca, ca_key) = crate::social_cert::read_certificate_and_key(&dir.join("ca.der"), &dir.join("ca-key.der"))?;
    for name in ["leaf.der", "leaf-key.der"] {
        std::fs::metadata(dir.join(name)).map_err(|error| path_error(error, &dir.join(name)))?;
    }
    let now = SystemTime::now();
    let unix_now = now.duration_since(UNIX_EPOCH).unwrap_or_default().as_secs() as i64;
    crate::social_cert::check_validity(&ca, unix_now)?;
    let ca_key = KeyPair::try_from(ca_key.as_slice()).map_err(rcgen_error)?;
    if crate::social_cert::certificate_public_key(&ca)? != ca_key.subject_public_key_info().as_slice() {
        return Err(io::Error::new(io::ErrorKind::InvalidData, "ca.der does not match ca-key.der"));
    }
    if crate::social_cert::certificate_extension(&ca, BASIC_CONSTRAINTS_OID)? != CA_BASIC_CONSTRAINTS {
        return Err(io::Error::new(io::ErrorKind::InvalidData, "ca.der is not a pathLen 0 CA"));
    }
    let issuer = Issuer::new(ca_params(host, now)?, ca_key);
    let (leaf, leaf_key) = leaf_pair(host, &issuer, now)?;
    let cert_temp = dir.join("leaf.der.new");
    let key_temp = dir.join("leaf-key.der.new");
    ensure_absent(&cert_temp)?;
    ensure_absent(&key_temp)?;
    write_new(&cert_temp, &leaf, false)?;
    write_new(&key_temp, &leaf_key, true)?;
    replace_with_backup(&dir.join("leaf.der"), &cert_temp, &dir.join("leaf.der.old"))?;
    replace_with_backup(&dir.join("leaf-key.der"), &key_temp, &dir.join("leaf-key.der.old"))?;
    Ok(())
}

pub fn generated_paths(dir: &Path) -> [PathBuf; 4] {
    FILES.map(|name| dir.join(name))
}

#[cfg(test)]
#[path = "social_cert_tool_tests.rs"]
mod tests;
