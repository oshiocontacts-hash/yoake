use super::support::*;
use horizon_mods::yak::{Archive, Signature, Target, canonical_hashes};
use horizon_release_sign::{SigningKey, YOAKE_MODS_V1, YOAKE_YAK_V1};
use std::fs::{self, File};
use std::io::Cursor;

fn signed_with(signature: Signature, bytes: &[u8]) -> Vec<u8> {
    let mut files = unpack(bytes);
    files.insert("SIGNATURE".into(), toml::to_string(&signature).unwrap().into_bytes());
    repack(&files)
}

#[test]
fn canonical_hash_list_is_sorted_and_excludes_signature() {
    let mut archive = Archive::new(Cursor::new(data_zip())).unwrap();
    let hashes = archive.hashes().clone();
    let expected = format!("{}  content/example.json\n{}  yak.toml\n", hashes["content/example.json"], hashes["yak.toml"]);
    assert_eq!(canonical_hashes(&hashes), expected);
    let (key, _) = SigningKey::generate().unwrap();
    let signed = signed_with(Signature::sign(hashes.clone(), &key), &data_zip());
    archive = Archive::new(Cursor::new(signed)).unwrap();
    assert_eq!(archive.hashes(), &hashes);
    assert_eq!(archive.canonical_hash_list(), expected);
    assert_eq!(archive.verify_signature(&[key.public()]).unwrap(), Some(key.public()));
}

#[test]
fn signing_replaces_atomically_and_extraction_requires_a_pinned_key() {
    let temp = Temp::new();
    let path = temp.write("sample.yak", data_zip());
    let (key, _) = SigningKey::generate().unwrap();
    horizon_release_sign::yak::sign_file(&path, &key).unwrap();
    let mut archive = Archive::new(File::open(&path).unwrap()).unwrap();
    assert!(archive.verify_signature(&[]).is_err());
    let (wrong, _) = SigningKey::generate().unwrap();
    assert!(archive.verify_signature(&[wrong.public()]).is_err());
    assert!(archive.extract_target(&temp.join("untrusted"), Target::WindowsX64, &[]).is_err());
    assert!(!temp.join("untrusted").exists());
    archive.extract_target(&temp.join("trusted"), Target::WindowsX64, &[key.public()]).unwrap();
    assert_eq!(fs::read(temp.join("trusted/content/example.json")).unwrap(), b"{}");
    drop(archive);
    horizon_release_sign::yak::sign_file(&path, &wrong).unwrap();
    let archive = Archive::new(File::open(&path).unwrap()).unwrap();
    assert_eq!(archive.verify_signature(&[wrong.public()]).unwrap(), Some(wrong.public()));
    assert!(archive.verify_signature(&[key.public()]).is_err());
}

#[test]
fn tampered_file_and_inexact_file_lists_are_rejected() {
    let bytes = data_zip();
    let archive = Archive::new(Cursor::new(&bytes)).unwrap();
    let (key, _) = SigningKey::generate().unwrap();
    let signature = Signature::sign(archive.hashes().clone(), &key);
    let mut files = unpack(&signed_with(signature.clone(), &bytes));
    files.insert("content/example.json".into(), b"[]".to_vec());
    reject(repack(&files));
    let mut omitted = signature.clone();
    omitted.files.remove("content/example.json");
    reject(signed_with(omitted, &bytes));
    let mut added = signature.clone();
    added.files.insert("content/not-present".into(), "0".repeat(64));
    reject(signed_with(added, &bytes));
    let mut malformed = signature;
    malformed.signature = "not base64".into();
    reject(signed_with(malformed, &bytes));
}

#[test]
fn replacing_hashes_does_not_bypass_crypto_or_domain_separation() {
    let bytes = data_zip();
    let archive = Archive::new(Cursor::new(&bytes)).unwrap();
    let (key, _) = SigningKey::generate().unwrap();
    let mut signature = Signature::sign(archive.hashes().clone(), &key);
    signature.signature = key.sign(YOAKE_MODS_V1, canonical_hashes(archive.hashes()).as_bytes());
    let archive = Archive::new(Cursor::new(signed_with(signature, &bytes))).unwrap();
    assert!(archive.verify_signature(&[key.public()]).is_err());
    let archive = Archive::new(Cursor::new(&bytes)).unwrap();
    let signature = Signature::sign(archive.hashes().clone(), &key);
    let mut files = unpack(&bytes);
    files.insert("content/example.json".into(), b"[]".to_vec());
    let updated = Archive::new(Cursor::new(repack(&files))).unwrap();
    let forged = Signature { files: updated.hashes().clone(), ..signature };
    let archive = Archive::new(Cursor::new(signed_with(forged, &repack(&files)))).unwrap();
    assert!(archive.verify_signature(&[key.public()]).is_err());
    assert_ne!(key.sign(YOAKE_YAK_V1, b"payload"), key.sign(YOAKE_MODS_V1, b"payload"));
}
