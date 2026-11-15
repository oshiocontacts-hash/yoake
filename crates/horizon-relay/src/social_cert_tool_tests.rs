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
    assert_eq!((to - from) as u64, LEAF_VALIDITY_SECONDS + LEAF_BACKDATE_SECONDS);
    let now = SystemTime::now().duration_since(UNIX_EPOCH).unwrap().as_secs() as i64;
    assert!(from <= now - 3590 && from >= now - 3700, "not_before is about one hour back");
    assert!(to > now + LEAF_VALIDITY_SECONDS as i64 - 100);

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

#[test]
fn a_trailing_dot_is_refused_and_upper_case_is_lowered() {
    let dir = TestDir::new();
    assert_eq!(generate(&dir.0, "relay.test.").unwrap_err().kind(), io::ErrorKind::InvalidInput);
    assert!(!dir.0.exists());
    generate(&dir.0, "Relay.Test").unwrap();
    let leaf = std::fs::read(dir.0.join("leaf.der")).unwrap();
    assert!(leaf.windows(10).any(|w| w == b"relay.test"));
}

#[test]
fn renewing_for_a_different_host_fails_and_changes_nothing() {
    let dir = TestDir::new();
    generate(&dir.0, "relay.test").unwrap();
    let names = ["ca.der", "ca-key.der", "leaf.der", "leaf-key.der"];
    let before: Vec<_> = names.iter().map(|n| std::fs::read(dir.0.join(n)).unwrap()).collect();
    let error = renew(&dir.0, "other.test").unwrap_err();
    assert_eq!(error.kind(), io::ErrorKind::InvalidData);
    let after: Vec<_> = names.iter().map(|n| std::fs::read(dir.0.join(n)).unwrap()).collect();
    assert_eq!(before, after);
    let mut files: Vec<_> = std::fs::read_dir(&dir.0).unwrap().map(|e| e.unwrap().file_name().into_string().unwrap()).collect();
    files.sort();
    assert_eq!(files, ["ca-key.der", "ca.der", "leaf-key.der", "leaf.der"], "no .new or .old left");
}

#[test]
fn renewal_removes_leftover_new_files_and_keeps_a_leaf_present() {
    let dir = TestDir::new();
    generate(&dir.0, "relay.test").unwrap();
    std::fs::write(dir.0.join("leaf.der.new"), b"junk").unwrap();
    std::fs::write(dir.0.join("leaf-key.der.new"), b"junk").unwrap();
    renew(&dir.0, "relay.test").unwrap();
    assert!(!dir.0.join("leaf.der.new").exists() && !dir.0.join("leaf-key.der.new").exists());
    assert!(dir.0.join("leaf.der.old").exists() && dir.0.join("leaf-key.der.old").exists());
}

#[test]
fn a_failure_part_way_removes_only_the_files_this_run_created() {
    let dir = TestDir::new();
    create_private_dir(&dir.0).unwrap();
    std::fs::write(dir.0.join("existing"), b"mine").unwrap();
    let items = [
        (dir.0.join("a"), &b"1"[..], false),
        (dir.0.join("b"), &b"2"[..], true),
        (dir.0.join("missing-dir").join("c"), &b"3"[..], false),
    ];
    assert!(files::write_all_new(&items).is_err());
    assert!(!dir.0.join("a").exists() && !dir.0.join("b").exists());
    let items = [(dir.0.join("a"), &b"1"[..], false), (dir.0.join("existing"), &b"2"[..], false)];
    assert_eq!(files::write_all_new(&items).unwrap_err().kind(), io::ErrorKind::AlreadyExists);
    assert!(!dir.0.join("a").exists());
    assert_eq!(std::fs::read(dir.0.join("existing")).unwrap(), b"mine");
}

#[cfg(unix)]
#[test]
fn renew_refuses_a_group_writable_directory_and_generate_makes_0700() {
    use std::os::unix::fs::PermissionsExt;
    let dir = TestDir::new();
    generate(&dir.0, "relay.test").unwrap();
    assert_eq!(std::fs::metadata(&dir.0).unwrap().permissions().mode() & 0o777, 0o700);
    std::fs::set_permissions(&dir.0, std::fs::Permissions::from_mode(0o770)).unwrap();
    assert_eq!(renew(&dir.0, "relay.test").unwrap_err().kind(), io::ErrorKind::PermissionDenied);
}
