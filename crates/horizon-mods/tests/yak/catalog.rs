use super::support::*;
use horizon_mods::yak::{Archive, Catalog, Manifest, Target};
use horizon_release_sign::{SigningKey, YOAKE_MODS_V1, YOAKE_YAK_V1, sha256_hex};
use serde_json::{Value, json};
use std::io::Cursor;

fn fixture(bytes: &[u8], manifest: &Manifest) -> Value {
    json!({
        "schema": 1,
        "generated": "2026-11-13T00:00:00Z",
        "entries": [{
            "id": manifest.id,
            "versions": [{
                "version": manifest.version,
                "url": format!("https://portal.example.test/yoake/mods/{}/{}/{}-{}.yak", manifest.id, manifest.version, manifest.id, manifest.version),
                "size": bytes.len(),
                "sha256": sha256_hex(bytes),
                "manifest": manifest,
                "targets": manifest.bin.as_ref().map(|bin| bin.targets.clone()).unwrap_or_default(),
                "engine": manifest.bin.as_ref().map(|bin| bin.engine.clone())
            }]
        }]
    })
}

fn parse(value: &Value) -> Result<Catalog, String> {
    Catalog::parse(&serde_json::to_vec(value).unwrap())
}

#[test]
fn catalog_authenticates_unsigned_data_and_program_packages() {
    let manifest = Manifest::parse(DATA).unwrap();
    let data = data_zip();
    let program = program();
    let program_manifest = Manifest::parse(&program).unwrap();
    let program_bytes = stored(&[("yak.toml", program.as_bytes()), ("bin/windows-x64/sample.dll", b"fake-dll"), ("bin/linux-x64/libsample.so", b"fake-so")]);
    let (key, _) = SigningKey::generate().unwrap();
    for (bytes, manifest) in [(&data, &manifest), (&program_bytes, &program_manifest)] {
        let json = serde_json::to_vec_pretty(&fixture(bytes, manifest)).unwrap();
        let signature = key.sign(YOAKE_MODS_V1, &json);
        let catalog = Catalog::parse_verified(&json, &signature, &[key.public()]).unwrap();
        let version = &catalog.entries[0].versions[0];
        version.verify_download(&mut Cursor::new(bytes)).unwrap();
        let archive = Archive::from_catalog(Cursor::new(bytes), version).unwrap();
        assert!(archive.signature().is_none());
        assert_eq!(archive.manifest().id, "sample-mod");
    }
}

#[test]
fn catalog_signature_covers_exact_json_bytes_and_its_own_domain() {
    let json = serde_json::to_vec(&fixture(&data_zip(), &Manifest::parse(DATA).unwrap())).unwrap();
    let (key, _) = SigningKey::generate().unwrap();
    let (wrong, _) = SigningKey::generate().unwrap();
    let signature = key.sign(YOAKE_MODS_V1, &json);
    assert!(Catalog::parse_verified(&json, &signature, &[]).is_err());
    assert!(Catalog::parse_verified(&json, &signature, &[wrong.public()]).is_err());
    assert!(Catalog::parse_verified(&json, &key.sign(YOAKE_YAK_V1, &json), &[key.public()]).is_err());
    let mut changed = json.clone();
    changed.push(b'\n');
    assert!(Catalog::parse(&changed).is_ok());
    assert!(Catalog::parse_verified(&changed, &signature, &[key.public()]).is_err());
    assert!(Catalog::parse_verified(&json, &"A".repeat(4097), &[key.public()]).is_err());
}

#[test]
fn download_hash_size_and_full_manifest_summary_must_match() {
    let bytes = data_zip();
    let catalog = parse(&fixture(&bytes, &Manifest::parse(DATA).unwrap())).unwrap();
    let version = &catalog.entries[0].versions[0];
    let mut changed = bytes.clone();
    changed[0] ^= 1;
    assert!(version.verify_download(&mut Cursor::new(changed)).is_err());
    assert!(version.verify_download(&mut Cursor::new(&bytes[..bytes.len() - 1])).is_err());
    let mut longer = bytes.clone();
    longer.push(0);
    assert!(version.verify_download(&mut Cursor::new(longer)).is_err());
    let mut mismatch = version.clone();
    mismatch.manifest.name = "Another name".into();
    assert!(Archive::from_catalog(Cursor::new(&bytes), &mismatch).is_err());
    mismatch = version.clone();
    mismatch.manifest.side.required_on_join = true;
    assert!(Archive::from_catalog(Cursor::new(&bytes), &mismatch).is_err());
}

#[test]
fn catalog_schema_time_identity_and_location_are_strict() {
    let base = fixture(&data_zip(), &Manifest::parse(DATA).unwrap());
    let mut changed = base.clone();
    changed["schema"] = json!(2);
    assert!(parse(&changed).is_err());
    changed = base.clone();
    changed["unexpected"] = json!(true);
    assert!(parse(&changed).is_err());
    for generated in ["2025-02-29T00:00:00Z", "2026-13-01T00:00:00Z", "0000-01-01T00:00:00Z", "2026-01-01T24:00:00Z", "2026-01-01T00:00:00+09:00", "not a timestamp"] {
        changed = base.clone();
        changed["generated"] = json!(generated);
        assert!(parse(&changed).is_err(), "{generated}");
    }
    changed = base.clone();
    changed["generated"] = json!("2028-02-29T23:59:59Z");
    assert!(parse(&changed).is_ok());
    for url in ["http://portal.example.test/yoake/mods/sample-mod/0.3.1/sample-mod-0.3.1.yak", "https://portal.example.test/mods/sample.yak", "https://user:secret@portal.example.test/yoake/mods/sample-mod/0.3.1/sample-mod-0.3.1.yak", "https://portal.example.test/yoake/mods/sample-mod/0.3.1/sample-mod-0.3.1.yak?token=hidden"] {
        changed = base.clone();
        changed["entries"][0]["versions"][0]["url"] = json!(url);
        assert!(parse(&changed).is_err());
    }
    for (field, value) in [("version", json!("0.3.2")), ("size", json!(0)), ("sha256", json!("F".repeat(64))), ("targets", json!(["windows-x64"])), ("engine", json!("0.3.1+wrong"))] {
        changed = base.clone();
        changed["entries"][0]["versions"][0][field] = value;
        assert!(parse(&changed).is_err(), "{field}");
    }
    changed = base.clone();
    changed["entries"].as_array_mut().unwrap().push(base["entries"][0].clone());
    assert!(parse(&changed).is_err());
    changed = base.clone();
    changed["entries"][0]["versions"].as_array_mut().unwrap().push(base["entries"][0]["versions"][0].clone());
    assert!(parse(&changed).is_err());
}

#[test]
fn program_catalog_targets_and_fingerprint_cannot_disagree_with_manifest() {
    let manifest = Manifest::parse(&program()).unwrap();
    let mut value = fixture(&data_zip(), &manifest);
    assert!(parse(&value).is_ok());
    value["entries"][0]["versions"][0]["targets"] = json!([Target::WindowsX64, Target::WindowsX64]);
    assert!(parse(&value).is_err());
    value = fixture(&data_zip(), &manifest);
    value["entries"][0]["versions"][0]["engine"] = Value::Null;
    assert!(parse(&value).is_err());
}
