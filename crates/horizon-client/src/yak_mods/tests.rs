use super::*;
use horizon_client::shader_pack_list;
use horizon_release_sign::{SigningKey, yak::Signature};
use std::{
    fs,
    sync::atomic::{AtomicU64, Ordering},
    time::{SystemTime, UNIX_EPOCH},
};

static NEXT_FIXTURE: AtomicU64 = AtomicU64::new(0);

struct Fixture(PathBuf);

impl Fixture {
    fn new() -> Self {
        let stamp = SystemTime::now().duration_since(UNIX_EPOCH).unwrap().as_nanos();
        let serial = NEXT_FIXTURE.fetch_add(1, Ordering::Relaxed);
        let root = std::env::temp_dir().join(format!("yoake-installed-{}-{stamp}-{serial}", std::process::id()));
        fs::create_dir(&root).unwrap();
        Self(root)
    }

    fn write(&self, path: &Path, bytes: impl AsRef<[u8]>) {
        assert!(path.starts_with(&self.0));
        fs::create_dir_all(path.parent().unwrap()).unwrap();
        fs::write(path, bytes).unwrap();
    }

    fn list(&self, text: &str) {
        self.write(&self.0.join("yoake-mods.txt"), text);
    }

    fn install(&self, source: &str) -> Installed {
        let manifest = Manifest::parse(source).unwrap();
        let root = self.0.join("yoake-mods").join(&manifest.id).join(manifest.version.to_string());
        self.write(&root.join("yak.toml"), source);
        Installed { root, manifest, content: None, content_wasm: false, signed: false }
    }

    fn binary(&self) -> Installed {
        let entry = self.install(&program());
        self.write(&entry.root.join("bin/windows-x64/sample.dll"), b"not an executable DLL");
        entry
    }

    fn sign(&self, entry: &Installed, key: &SigningKey) {
        let mut files: BTreeMap<_, _> = installed_files(&entry.root).unwrap().into_iter().map(|name| {
            let digest = yak::hash_installed_file(&entry.root, &name).unwrap();
            (name, digest)
        }).collect();
        files.insert("bin/linux-x64/libsample.so".into(), "11".repeat(32));
        let signature = Signature::sign(files, key);
        self.write(&entry.root.join("SIGNATURE"), serde_json::to_vec(&signature).unwrap());
    }

    fn content(&self, root: &Path, id: &str, version: &str) {
        self.write(&root.join("horizon-mod.toml"), format!(
            r#"id = "{id}"
name = "Content"
version = "{version}"
api = "0.2"
capabilities = ["registries"]
"#,
        ));
    }
}

impl Drop for Fixture {
    fn drop(&mut self) {
        if self.0.parent() == Some(std::env::temp_dir().as_path()) {
            let _ = fs::remove_dir_all(&self.0);
        }
    }
}

fn build() -> Build {
    Build {
        version: Version::parse("0.3.2").unwrap(),
        engine: "0.3.2+build.test".into(),
        abi: 2,
        target: Some(Target::WindowsX64),
    }
}

fn data(id: &str) -> String {
    format!(r#"format = 1
id = "{id}"
version = "1.2.3"
name = "Installed sample"
name_ja = "導入サンプル"
authors = ["Test author"]
description = "Headless installed-view fixture."
[yoake]
version = ">=0.3.2, <0.4"
[side]
client = true
server = true
required_on_join = false
"#)
}

fn program() -> String {
    data("sample_mod") + r#"
[bin]
library = "sample"
targets = ["windows-x64"]
engine = "0.3.2+build.test"
abi = 2
"#
}

fn shader_source() -> String {
    data("shader_mod") + r#"
[[shaders]]
id = "kage"
name = "Kage"
[shaders.options.shadow_map]
values = [512, 1024, 2048]
default = 1024
label_ja = "影の解像度"
[shaders.options.reflections]
values = [false, true]
default = true
label_ja = "水面の反射"
[shaders.options.tonemap]
values = ["None", "Soft", "Filmic"]
default = "Soft"
"#
}

fn sample_pack() -> horizon_shader_api::Pack {
    horizon_shader_api::Pack::read(Path::new(concat!(
        env!("CARGO_MANIFEST_DIR"), "/../horizon-shader-api/tests/samples/schema-1",
    ))).unwrap()
}

#[test]
fn missing_list_does_not_inspect_installed_packages_or_create_files() {
    let fixture = Fixture::new();
    fixture.write(&fixture.0.join("yoake-mods/broken/1.2.3/yak.toml"), b"invalid TOML");
    let before = fs::read_dir(&fixture.0).unwrap().count();
    let runtime = Runtime::read(&fixture.0, &build(), &[]);
    assert!(runtime.entries.is_empty());
    assert!(runtime.startup_log.is_empty());
    assert!(runtime.notices.lock().unwrap().is_empty());
    assert_eq!(fs::read_dir(&fixture.0).unwrap().count(), before);
    assert!(!fixture.0.join("yoake-mods.txt").exists());
    assert!(!fixture.0.join("shaderpacks").exists());
}

#[test]
fn list_comments_and_unsigned_content_preserve_load_order() {
    let fixture = Fixture::new();
    let second = fixture.install(&data("second_mod"));
    fixture.install(&data("first_mod"));
    fixture.content(&second.root.join("content"), "second_mod", "1.2.3");
    fixture.list("# launcher order\n\nsecond_mod 1.2.3 # dependency first\nfirst_mod 1.2.3\n");
    let runtime = Runtime::read(&fixture.0, &build(), &[]);
    let ids: Vec<_> = runtime.entries.iter().map(|entry| entry.manifest.id.as_str()).collect();
    assert_eq!(ids, ["second_mod", "first_mod"]);
    assert!(runtime.entries.iter().all(|entry| !entry.signed));
    assert_eq!(runtime.entries[0].content.as_deref(), Some(second.root.join("content").as_path()));
    assert!(runtime.notices.lock().unwrap().is_empty());
}

#[test]
fn incompatible_entries_are_skipped_with_individual_consumable_notices() {
    let fixture = Fixture::new();
    fixture.install(&program().replace("0.3.2+build.test", "0.3.2+build.other"));
    fixture.install(&data("future_mod").replace(">=0.3.2, <0.4", ">=0.4"));
    fixture.install(&data("good_mod"));
    fixture.list("sample_mod 1.2.3\nfuture_mod 1.2.3\ngood_mod 1.2.3\n");
    let runtime = Runtime::read(&fixture.0, &build(), &[]);
    assert_eq!(runtime.entries.len(), 1);
    assert_eq!(runtime.entries[0].manifest.id, "good_mod");
    assert_eq!(runtime.startup_log.len(), 2);
    let mut notices = runtime.notices.lock().unwrap();
    for _ in 0..2 {
        let notice = notices.pop_front().unwrap();
        let title = notice.title();
        assert!(title.contains("導入サンプル"));
        assert!(title.contains("0.3.2"));
        assert!(title.contains("Not loaded"));
        assert!(notice.reason.contains("incompatible build"));
    }
    assert!(notices.pop_front().is_none());
}

#[test]
fn unsafe_paths_duplicate_ids_and_inner_identity_mismatches_are_rejected() {
    let fixture = Fixture::new();
    let entry = fixture.install(&data("sample_mod"));
    fixture.content(&entry.root.join("content"), "other_mod", "1.2.3");
    fixture.list("../outside 1.2.3\nsample_mod 1.2.3\nsample_mod 1.2.3\n");
    let runtime = Runtime::read(&fixture.0, &build(), &[]);
    assert!(runtime.entries.is_empty());
    assert_eq!(runtime.startup_log.len(), 3);
    assert!(yak::hash_installed_file(&entry.root, "../yak.toml").is_err());
    assert!(yak::hash_installed_file(&entry.root, "bin/windows-x64/../../yak.toml").is_err());
}

#[test]
fn signed_fake_dll_passes_integrity_without_executing_and_retains_other_target_hashes() {
    let fixture = Fixture::new();
    let entry = fixture.binary();
    let (key, _) = SigningKey::generate().unwrap();
    fixture.sign(&entry, &key);
    fixture.list("sample_mod 1.2.3\n");
    let runtime = Runtime::read(&fixture.0, &build(), &[key.public()]);
    assert!(runtime.entries[0].signed);
    let (path, digest) = binary_preflight(&runtime.entries[0], &build(), &[key.public()], false).unwrap();
    assert_eq!(path, entry.root.join("bin/windows-x64/sample.dll"));
    assert_eq!(digest, yak::hash_installed_file(&entry.root, "bin/windows-x64/sample.dll").unwrap());
    assert_eq!(digest.len(), 64);
    assert!(binary_preflight(&entry, &build(), &[], false).is_err());
}

#[test]
fn tampered_dll_manifest_and_signature_are_refused_before_execution() {
    let fixture = Fixture::new();
    let entry = fixture.binary();
    let (key, _) = SigningKey::generate().unwrap();
    fixture.sign(&entry, &key);
    fixture.write(&entry.root.join("bin/windows-x64/sample.dll"), b"tampered bytes");
    assert!(binary_preflight(&entry, &build(), &[key.public()], false).unwrap_err().contains("checksum mismatch"));
    fixture.sign(&entry, &key);
    fixture.write(&entry.root.join("yak.toml"), program().replace("Headless installed-view", "Changed installed-view"));
    assert!(binary_preflight(&entry, &build(), &[key.public()], false).unwrap_err().contains("yak.toml"));
    fixture.sign(&entry, &key);
    fixture.write(&entry.root.join("SIGNATURE"), b"not a signature");
    assert!(binary_preflight(&entry, &build(), &[key.public()], false).is_err());
}

#[test]
fn unsigned_override_bypasses_only_signature_not_build_target_or_missing_file_checks() {
    let fixture = Fixture::new();
    let entry = fixture.binary();
    assert!(binary_preflight(&entry, &build(), &[], false).is_err());
    assert!(binary_preflight(&entry, &build(), &[], true).is_ok());
    let mut incompatible = build();
    incompatible.engine.push_str(".different");
    assert!(binary_preflight(&entry, &incompatible, &[], true).is_err());
    incompatible = build();
    incompatible.abi += 1;
    assert!(binary_preflight(&entry, &incompatible, &[], true).is_err());
    incompatible = build();
    incompatible.target = Some(Target::LinuxX64);
    assert!(binary_preflight(&entry, &incompatible, &[], true).is_err());
    incompatible.target = None;
    assert!(binary_preflight(&entry, &incompatible, &[], true).is_err());
    fs::remove_file(entry.root.join("bin/windows-x64/sample.dll")).unwrap();
    assert!(binary_preflight(&entry, &build(), &[], true).is_err());
}

#[test]
fn preflight_refuses_replaced_identity_and_escaping_library_names() {
    let fixture = Fixture::new();
    let entry = fixture.binary();
    fixture.write(&entry.root.join("yak.toml"), program().replace("sample_mod", "other_mod"));
    assert!(binary_preflight(&entry, &build(), &[], true).unwrap_err().contains("identity changed"));
    fixture.write(&entry.root.join("yak.toml"), program().replace(r#"library = "sample""#, r#"library = "../outside""#));
    assert!(binary_preflight(&entry, &build(), &[], true).is_err());
}

#[cfg(unix)]
#[test]
fn installed_symlinks_are_rejected_even_with_unsigned_override() {
    let fixture = Fixture::new();
    let entry = fixture.binary();
    let binary = entry.root.join("bin/windows-x64/sample.dll");
    let external = fixture.0.join("external.dll");
    fixture.write(&external, b"external bytes");
    fs::remove_file(&binary).unwrap();
    std::os::unix::fs::symlink(&external, &binary).unwrap();
    assert!(yak::check_installed_tree(&entry.root).is_err());
    assert!(binary_preflight(&entry, &build(), &[], true).is_err());
}

#[cfg(unix)]
#[test]
fn preflight_rechecks_manifest_symlinks_after_discovery() {
    let fixture = Fixture::new();
    let entry = fixture.binary();
    let external = fixture.0.join("external.toml");
    fixture.write(&external, program());
    fs::remove_file(entry.root.join("yak.toml")).unwrap();
    std::os::unix::fs::symlink(&external, entry.root.join("yak.toml")).unwrap();
    assert!(binary_preflight(&entry, &build(), &[], true).is_err());
}

#[test]
fn yak_content_replaces_legacy_copy_but_preserves_unrelated_content() {
    let fixture = Fixture::new();
    let entry = fixture.install(&data("sample_mod"));
    fixture.content(&entry.root.join("content"), "sample_mod", "1.2.3");
    let legacy = fixture.0.join("horizon-mods/old_sample");
    let unrelated = fixture.0.join("horizon-mods/other");
    fixture.content(&legacy, "sample_mod", "0.1.0");
    fixture.content(&unrelated, "other_mod", "1.0.0");
    fixture.list("sample_mod 1.2.3\n");
    let runtime = Runtime::read(&fixture.0, &build(), &[]);
    let packages = content_packages(vec![legacy.clone(), unrelated.clone()], &runtime.entries);
    assert_eq!(packages.len(), 2);
    assert_eq!(packages[0].0, unrelated);
    assert!(!packages[0].2);
    assert_eq!(packages[1].0, entry.root.join("content"));
    assert_eq!(packages[1].1.manifest.version, "1.2.3");
    assert!(packages[1].2);
    let legacy_only = content_packages(vec![legacy.clone()], &[]);
    assert_eq!(legacy_only.len(), 1);
    assert_eq!(legacy_only[0].0, legacy);
    assert!(!legacy_only[0].2);
}

#[test]
fn mods_rows_show_yak_source_trust_and_existing_wasm_restrictions() {
    let fixture = Fixture::new();
    let entry = fixture.install(&data("sample_mod"));
    fixture.content(&entry.root.join("content"), "sample_mod", "1.2.3");
    fixture.write(&entry.root.join("content/mod.wasm"), b"not executable");
    fixture.list("sample_mod 1.2.3\n");
    let runtime = Runtime::read(&fixture.0, &build(), &[]);
    let mut rows = Vec::new();
    populate_rows_from(&mut rows, &runtime.entries, &BTreeMap::new(), "Native port required");
    assert_eq!(rows.len(), 1);
    assert!(rows[0].description.contains(".yak"));
    assert!(rows[0].description.contains("Unsigned or unverified"));
    assert_eq!(rows[0].problem.as_deref(), Some("Native port required"));
    let problems = BTreeMap::from([("sample_mod".into(), "Native signature refused".into())]);
    populate_rows_from(&mut rows, &runtime.entries, &problems, "Native port required");
    assert_eq!(rows.len(), 1);
    assert_eq!(rows[0].problem.as_deref(), Some("Native signature refused"));
}

#[test]
fn declared_shader_options_reach_picker_preferences_and_shadow_inputs() {
    let fixture = Fixture::new();
    let entry = fixture.install(&shader_source());
    let original = sample_pack();
    for (name, bytes) in &original.files {
        fixture.write(&entry.root.join("shaders/kage").join(name), bytes);
    }
    let shader = &entry.manifest.shaders[0];
    assert_eq!(shader.options["shadow_map"].default, yak::OptionValue::Integer(1024));
    let dir = fixture.0.join("shaderpacks");
    let packs = shaders::scan_installed(&dir, std::slice::from_ref(&entry));
    assert_eq!(packs.len(), 1);
    assert_eq!(packs[0].title, "Kage (.yak)");
    assert_eq!(packs[0].kind, shader_pack_list::Kind::Horizon);
    let mut selected = packs[0].clone();
    let index = selected.options.iter().position(|option| option.id == "shadow_map").unwrap();
    assert!(selected.options[index].name.contains("影の解像度"));
    assert_eq!(selected.values[index], 1.0);
    assert!(selected.step_option(index, false, false));
    fs::create_dir_all(&dir).unwrap();
    selected.save_values(&dir);
    assert!(dir.join(format!("{}.txt", selected.file)).is_file());
    assert!(!entry.root.join("shaders/kage.txt").exists());
    let mut pack = sample_pack();
    let constants = shaders::apply_parameters(&mut pack, shader, &dir.join(&selected.file)).unwrap();
    assert!(!constants.contains_key("shadow_map"));
    assert_eq!(constants["reflections"], 1.0);
    assert_eq!(constants["tonemap"], 1.0);
    assert_eq!(pack.manifest.shadow.as_ref().unwrap().resolution, 2048);
    let report = horizon_shader_api::check(&pack).unwrap();
    assert_eq!(report.shadow.unwrap().resolution, 2048);
    selected.reset_options();
    selected.save_values(&dir);
    let mut pack = sample_pack();
    shaders::apply_parameters(&mut pack, shader, &dir.join(&selected.file)).unwrap();
    assert_eq!(pack.manifest.shadow.unwrap().resolution, 1024);
}

#[test]
fn unsupported_shadow_sizes_are_refused_before_pack_check() {
    let fixture = Fixture::new();
    let entry = fixture.install(&shader_source().replace("[512, 1024, 2048]", "[512, 768, 2048]").replace("default = 1024", "default = 768"));
    let mut pack = sample_pack();
    assert!(shaders::apply_parameters(&mut pack, &entry.manifest.shaders[0], &fixture.0.join("preference")).is_err());
}
