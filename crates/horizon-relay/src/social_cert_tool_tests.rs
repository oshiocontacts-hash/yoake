use super::*;
use crate::social_cert::{certificate_extension, certificate_validity};
use std::sync::atomic::{AtomicU64, Ordering};

static NEXT_DIR: AtomicU64 = AtomicU64::new(0);

struct TestDir(PathBuf);
impl TestDir {
    fn new() -> Self {
        let id = NEXT_DIR.fetch_add(1, Ordering::Relaxed);
        Self(std::env::temp_dir().join(format!("yoake-social-cert-unit-{}-{id}", std::process::id())))
    }
}
impl Drop for TestDir {
    fn drop(&mut self) { let _ = std::fs::remove_dir_all(&self.0); }
}

#[test]
fn generated_certificates_have_the_required_constraints_and_validity() {
    let dir = TestDir::new();
    generate(&dir.0, "relay.test").unwrap();
    let ca = std::fs::read(dir.0.join("ca.der")).unwrap();
    let leaf = std::fs::read(dir.0.join("leaf.der")).unwrap();
    assert_eq!(certificate_extension(&ca, BASIC_CONSTRAINTS_OID).unwrap(), CA_BASIC_CONSTRAINTS);
    // nameConstraints is OID 2.5.29.30 = 55 1D 1E.
    assert!(certificate_extension(&ca, &[0x55, 0x1d, 0x1e]).is_ok(), "DNS name constraints");

    let san = certificate_extension(&leaf, &[0x55, 0x1d, 0x11]).unwrap();
    // SEQUENCE { [2] dNSName "relay.test" }: lengths follow from the name (10 bytes).
    let host = b"relay.test";
    let mut expected_san = vec![0x30, (host.len() + 2) as u8, 0x82, host.len() as u8];
    expected_san.extend_from_slice(host);
    assert_eq!(san, expected_san);
    assert_eq!(
        certificate_extension(&leaf, &[0x55, 0x1d, 0x25]).unwrap(),
        &[0x30, 0x0a, 0x06, 0x08, 0x2b, 0x06, 0x01, 0x05, 0x05, 0x07, 0x03, 0x01],
        "serverAuth only",
    );
    let (from, to) = certificate_validity(&leaf).unwrap();
    assert_eq!((to - from) as u64, LEAF_VALIDITY_SECONDS);

    #[cfg(unix)]
    for name in ["ca-key.der", "leaf-key.der"] {
        use std::os::unix::fs::PermissionsExt;
        let mode = std::fs::metadata(dir.0.join(name)).unwrap().permissions().mode();
        assert_eq!(mode & 0o777, 0o600, "{name}");
    }
}

#[test]
fn an_ip_address_is_not_accepted_as_a_dns_host() {
    let dir = TestDir::new();
    assert_eq!(generate(&dir.0, "192.0.2.1").unwrap_err().kind(), io::ErrorKind::InvalidInput);
}
