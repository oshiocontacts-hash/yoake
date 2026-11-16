mod social_support;

use horizon_relay::{social_cert::tool, social_tls::HandshakeLimits};
use social_support::*;
use std::{
    path::PathBuf,
    sync::atomic::{AtomicU64, Ordering},
};

static NEXT_DIR: AtomicU64 = AtomicU64::new(0);

struct TestDir(PathBuf);
impl TestDir {
    fn new() -> Self {
        let id = NEXT_DIR.fetch_add(1, Ordering::Relaxed);
        Self(std::env::temp_dir().join(format!("yoake-social-cert-integration-{}-{id}", std::process::id())))
    }
}
impl Drop for TestDir {
    fn drop(&mut self) { let _ = std::fs::remove_dir_all(&self.0); }
}

fn generated(dir: &TestDir) -> TestTls {
    TestTls {
        ca: std::fs::read(dir.0.join("ca.der")).unwrap(),
        cert: std::fs::read(dir.0.join("leaf.der")).unwrap(),
        key: std::fs::read(dir.0.join("leaf-key.der")).unwrap(),
    }
}

fn assert_generated_pair_handshakes(tls: &TestTls) {
    let server = server(tls, HandshakeLimits::default());
    let address = server.spawn_local().unwrap();
    let stream = raw_client(address, &tls.ca);
    drop(stream);
    server.stop();
}

#[test]
fn a_client_pinning_the_generated_ca_accepts_the_generated_leaf() {
    let dir = TestDir::new();
    tool::generate(&dir.0, "relay.test").unwrap();
    assert_generated_pair_handshakes(&generated(&dir));
}

#[test]
fn renewal_keeps_the_ca_backs_up_the_old_pair_and_issues_a_working_leaf() {
    let dir = TestDir::new();
    tool::generate(&dir.0, "relay.test").unwrap();
    let original = generated(&dir);
    let original_ca_key = std::fs::read(dir.0.join("ca-key.der")).unwrap();
    tool::renew(&dir.0, "relay.test").unwrap();

    let renewed = generated(&dir);
    assert_eq!(renewed.ca, original.ca);
    assert_eq!(std::fs::read(dir.0.join("ca-key.der")).unwrap(), original_ca_key);
    assert_ne!(renewed.cert, original.cert);
    assert_ne!(renewed.key, original.key);
    assert_eq!(std::fs::read(dir.0.join("leaf.der.old")).unwrap(), original.cert);
    assert_eq!(std::fs::read(dir.0.join("leaf-key.der.old")).unwrap(), original.key);
    assert_generated_pair_handshakes(&renewed);
}

#[test]
fn generation_refuses_to_overwrite_any_existing_output() {
    let dir = TestDir::new();
    tool::generate(&dir.0, "relay.test").unwrap();
    let before = generated(&dir);
    let before_ca_key = std::fs::read(dir.0.join("ca-key.der")).unwrap();
    let error = tool::generate(&dir.0, "relay.test").unwrap_err();
    assert_eq!(error.kind(), std::io::ErrorKind::AlreadyExists);
    let after = generated(&dir);
    assert_eq!(after.ca, before.ca);
    assert_eq!(after.cert, before.cert);
    assert_eq!(after.key, before.key);
    assert_eq!(std::fs::read(dir.0.join("ca-key.der")).unwrap(), before_ca_key);
}
