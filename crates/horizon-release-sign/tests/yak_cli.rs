#![cfg(feature = "yak")]

use horizon_release_sign::{SigningKey, YOAKE_MODS_V1, verify_bytes, yak};
use std::fs::{self, File};
use std::io::Write;
use std::path::{Path, PathBuf};
use std::process::{Command, Output};
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::{SystemTime, UNIX_EPOCH};
use zip::{ZipWriter, write::SimpleFileOptions};

struct Temp(PathBuf);

impl Temp {
    fn new() -> Self {
        static NEXT: AtomicU64 = AtomicU64::new(0);
        let stamp = SystemTime::now().duration_since(UNIX_EPOCH).unwrap().as_nanos();
        let path = std::env::temp_dir().join(format!("horizon-yak-sign-{}-{stamp}-{}", std::process::id(), NEXT.fetch_add(1, Ordering::Relaxed)));
        fs::create_dir(&path).unwrap();
        Self(path)
    }
}

impl Drop for Temp {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.0);
    }
}

fn tool(args: &[&str], path: &Path) -> Output {
    Command::new(env!("CARGO_BIN_EXE_horizon-release-sign")).args(args).arg(path).output().unwrap()
}

#[test]
fn yak_cli_signs_and_verifies_using_only_a_generated_test_key() {
    let temp = Temp::new();
    let archive_path = temp.0.join("sample.yak");
    let mut archive = ZipWriter::new(File::create(&archive_path).unwrap());
    archive.start_file("yak.toml", SimpleFileOptions::default().compression_method(zip::CompressionMethod::Stored)).unwrap();
    archive.write_all(b"format = 1\nid = 'sample-mod'\nversion = '0.3.1'\nname = 'Sample'\nauthors = ['Test author']\ndescription = 'Test package'\n[yoake]\nversion = '^0.3'\n[side]\nclient = true\nserver = false\nrequired_on_join = false\n").unwrap();
    drop(archive.finish().unwrap());
    let (key, secret) = SigningKey::generate().unwrap();
    let key_path = temp.0.join("test-key.pk8");
    fs::write(&key_path, secret).unwrap();
    let signed = tool(&["sign", "--tag", "yoake-yak-v1", "--key-file", key_path.to_str().unwrap()], &archive_path);
    assert!(signed.status.success(), "{}", String::from_utf8_lossy(&signed.stderr));
    assert_eq!(String::from_utf8(signed.stdout).unwrap().trim(), key.public_base64());
    assert!(tool(&["verify", "--tag", "yoake-yak-v1", "--key", &key.public_base64()], &archive_path).status.success());
    assert!(!tool(&["verify", "--tag", "yoake-yak-v1"], &archive_path).status.success());
    let (wrong, _) = SigningKey::generate().unwrap();
    assert!(!tool(&["verify", "--tag", "yoake-yak-v1", "--key", &wrong.public_base64()], &archive_path).status.success());
    let parsed = yak::Archive::new(File::open(&archive_path).unwrap()).unwrap();
    assert_eq!(parsed.verify_signature(&[key.public()]).unwrap(), Some(key.public()));
}

#[test]
fn catalog_cli_signs_exact_json_and_refuses_sidecar_overwrite() {
    let temp = Temp::new();
    let path = temp.0.join("index.json");
    let bytes = b"{\"schema\":1,\"generated\":\"2026-11-13T00:00:00Z\",\"entries\":[]}\n";
    fs::write(&path, bytes).unwrap();
    let (key, secret) = SigningKey::generate().unwrap();
    let key_path = temp.0.join("test-key.pk8");
    fs::write(&key_path, secret).unwrap();
    let args = ["sign", "--tag", "yoake-mods-v1", "--key-file", key_path.to_str().unwrap()];
    assert!(tool(&args, &path).status.success());
    let signature = fs::read_to_string(temp.0.join("index.json.sig")).unwrap();
    verify_bytes(YOAKE_MODS_V1, bytes, &signature, &[key.public()]).unwrap();
    assert!(tool(&["verify", "--tag", "yoake-mods-v1", "--key", &key.public_base64()], &path).status.success());
    assert!(!tool(&args, &path).status.success());
    fs::write(&path, b"{}").unwrap();
    assert!(!tool(&["verify", "--tag", "yoake-mods-v1", "--key", &key.public_base64()], &path).status.success());
}
