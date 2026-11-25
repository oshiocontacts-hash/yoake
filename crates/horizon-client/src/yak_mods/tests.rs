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
        let stamp = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        let serial = NEXT_FIXTURE.fetch_add(1, Ordering::Relaxed);
        let root = std::env::temp_dir().join(format!(
            "yoake-installed-{}-{stamp}-{serial}",
            std::process::id()
        ));
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
        let root = self
            .0
            .join("yoake-mods")
            .join(&manifest.id)
            .join(manifest.version.to_string());
        self.write(&root.join("yak.toml"), source);
        Installed {
            root,
            manifest,
            content: None,
            content_wasm: false,
            signed: false,
        }
    }

    fn binary(&self) -> Installed {
        let entry = self.install(&program());
        self.write(
            &entry.root.join("bin/windows-x64/sample.dll"),
            b"not an executable DLL",
        );
        entry
    }

    fn sign(&self, entry: &Installed, key: &SigningKey) {
        let mut files: BTreeMap<_, _> = installed_files(&entry.root)
            .unwrap()
            .into_iter()
            .map(|name| {
                let digest = yak::hash_installed_file(&entry.root, &name).unwrap();
                (name, digest)
            })
            .collect();
        files.insert("bin/linux-x64/libsample.so".into(), "11".repeat(32));
        let signature = Signature::sign(files, key);
        self.write(
            &entry.root.join("SIGNATURE"),
            serde_json::to_vec(&signature).unwrap(),
        );
    }

    fn content(&self, root: &Path, id: &str, version: &str) {
        self.write(
            &root.join("horizon-mod.toml"),
            format!(
                r#"id = "{id}"
name = "Content"
version = "{version}"
api = "0.2"
capabilities = ["registries"]
"#,
            ),
        );
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
    format!(
        r#"format = 1
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
"#
    )
}

fn dependent(id: &str, dependency: &str, requirement: &str) -> String {
    data(id) + &format!("\n[dependencies]\n{dependency} = \"{requirement}\"\n")
}

fn program() -> String {
    data("sample_mod")
        + r#"
[bin]
library = "sample"
targets = ["windows-x64"]
engine = "0.3.2+build.test"
abi = 2
"#
}

#[test]
fn unsigned_switches_scope_wasm_and_native_code_separately() {
    let wasm_only = UnsignedSwitches::from_values(None, Some(std::ffi::OsStr::new("1")));
    assert!(!wasm_only.dll);
    assert!(wasm_only.wasm);

    let legacy = UnsignedSwitches::from_values(Some(std::ffi::OsStr::new("1")), None);
    assert!(legacy.dll);
    assert!(legacy.wasm);

    let disabled = UnsignedSwitches::from_values(None, None);
    assert!(!disabled.dll);
    assert!(!disabled.wasm);

    let invalid = UnsignedSwitches::from_values(
        Some(std::ffi::OsStr::new("true")),
        Some(std::ffi::OsStr::new("true")),
    );
    assert!(!invalid.dll);
    assert!(!invalid.wasm);
}

fn shader_source() -> String {
    data("shader_mod")
        + r#"
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
        env!("CARGO_MANIFEST_DIR"),
        "/../horizon-shader-api/tests/samples/schema-1",
    )))
    .unwrap()
}

#[test]
fn missing_list_does_not_inspect_installed_packages_or_create_files() {
    let fixture = Fixture::new();
    fixture.write(
        &fixture.0.join("yoake-mods/broken/1.2.3/yak.toml"),
        b"invalid TOML",
    );
    let before = fs::read_dir(&fixture.0).unwrap().count();
    let runtime = Runtime::read(&fixture.0, &build(), &[]);
    assert!(runtime.entries.is_empty());
    assert!(!runtime.synced_config);
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
    let ids: Vec<_> = runtime
        .entries
        .iter()
        .map(|entry| entry.manifest.id.as_str())
        .collect();
    assert_eq!(ids, ["second_mod", "first_mod"]);
    assert!(runtime.entries.iter().all(|entry| !entry.signed));
    assert_eq!(
        runtime.entries[0].content.as_deref(),
        Some(second.root.join("content").as_path())
    );
    assert!(runtime.notices.lock().unwrap().is_empty());
}

#[test]
fn a_missing_dependency_skips_the_dependent_with_a_notice() {
    let fixture = Fixture::new();
    fixture.install(&dependent("addon_mod", "absent_mod", "^1"));
    fixture.install(&data("healthy_mod"));
    fixture.list("addon_mod 1.2.3\nhealthy_mod 1.2.3\n");
    let runtime = Runtime::read(&fixture.0, &build(), &[]);
    let ids: Vec<_> = runtime
        .entries
        .iter()
        .map(|entry| entry.manifest.id.as_str())
        .collect();
    assert_eq!(ids, ["healthy_mod"]);
    assert!(runtime.startup_log.join("\n").contains("missing required dependency absent_mod ^1"));
    let notices = runtime.notices.lock().unwrap();
    assert_eq!(notices.len(), 1);
    assert!(notices[0].reason.contains("dependency resolution"));
}

#[test]
fn an_incompatible_dependency_version_skips_only_the_dependent() {
    let fixture = Fixture::new();
    fixture.install(&dependent("addon_mod", "base_mod", ">=2"));
    fixture.install(&data("base_mod"));
    fixture.list("addon_mod 1.2.3\nbase_mod 1.2.3\n");
    let runtime = Runtime::read(&fixture.0, &build(), &[]);
    let ids: Vec<_> = runtime
        .entries
        .iter()
        .map(|entry| entry.manifest.id.as_str())
        .collect();
    assert_eq!(ids, ["base_mod"]);
    let log = runtime.startup_log.join("\n");
    assert!(log.contains("dependency base_mod requires version >=2"));
    assert!(log.contains("installed version is 1.2.3"));
}

#[test]
fn dependencies_load_before_their_dependents() {
    let fixture = Fixture::new();
    fixture.install(&dependent("addon_mod", "base_mod", "^1"));
    fixture.install(&data("base_mod"));
    fixture.list("addon_mod 1.2.3\nbase_mod 1.2.3\n");
    let runtime = Runtime::read(&fixture.0, &build(), &[]);
    let ids: Vec<_> = runtime
        .entries
        .iter()
        .map(|entry| entry.manifest.id.as_str())
        .collect();
    assert_eq!(ids, ["base_mod", "addon_mod"]);
    assert!(runtime.startup_log.is_empty());
}

fn full_manifest(source: String) -> Manifest {
    // A server-only full-access package: `side.client` must match the absent `wasm.client`.
    Manifest::parse(&(source.replace("[side]\nclient = true", "[side]\nclient = false") + r#"
[wasm]
interface = "1.0"
server = "bin/wasm/server.wasm"
world = "full-access"
"#)).unwrap()
}

fn test_runtime(sources: &[String]) -> Runtime {
    Runtime {
        entries: sources.iter().map(|source| {
            Installed::new_for_test(PathBuf::new(), Manifest::parse(source).unwrap())
        }).collect(),
        ..Runtime::default()
    }
}

#[test]
fn a_full_access_package_with_a_missing_dependency_and_its_dependents_are_skipped() {
    let full = full_manifest(dependent("full_addon", "absent_mod", "^1"));
    let mut runtime = test_runtime(&[
        dependent("sandbox_dependent", "full_addon", "^1"),
        data("healthy_mod"),
    ]);
    let resolution = resolve_runtime_with_full(&mut runtime, &build(), &[full]);
    assert!(resolution.refused["full_addon"].contains("missing required dependency absent_mod"));
    assert!(resolution.refused["sandbox_dependent"].contains("full_addon was not loaded"));
    assert_eq!(runtime.entries.iter().map(|entry| entry.manifest.id.as_str()).collect::<Vec<_>>(), ["healthy_mod"]);
    assert_eq!(runtime.notices.lock().unwrap().len(), 2);
}

#[test]
fn a_sandboxed_dependent_follows_a_full_access_dependency_in_the_shared_order() {
    let full = full_manifest(data("full_base"));
    let mut runtime = test_runtime(&[dependent("sandbox_addon", "full_base", "^1")]);
    let resolution = resolve_runtime_with_full(&mut runtime, &build(), &[full]);
    assert!(resolution.refused.is_empty());
    assert_eq!(resolution.order, ["full_base", "sandbox_addon"]);
    assert_eq!(runtime.entries.iter().map(|entry| entry.manifest.id.as_str()).collect::<Vec<_>>(), ["sandbox_addon"]);
    assert!(runtime.notices.lock().unwrap().is_empty());
}

#[test]
fn a_conflict_between_full_access_and_sandboxed_packages_refuses_both() {
    let sandbox = data("sandbox_mod") + "\n[conflicts]\nfull_mod = \"^1\"\n";
    let full = full_manifest(data("full_mod"));
    let mut runtime = test_runtime(&[sandbox]);
    let resolution = resolve_runtime_with_full(&mut runtime, &build(), &[full]);
    assert!(resolution.refused.contains_key("sandbox_mod"));
    assert!(resolution.refused.contains_key("full_mod"));
    assert!(resolution.order.is_empty());
    assert!(runtime.entries.is_empty());
    assert_eq!(runtime.notices.lock().unwrap().len(), 2);
}

#[test]
fn combined_sets_without_dependencies_keep_each_sets_discovery_order() {
    let full = [full_manifest(data("full_second")), full_manifest(data("full_first"))];
    let mut runtime = test_runtime(&[data("sandbox_second"), data("sandbox_first")]);
    let resolution = resolve_runtime_with_full(&mut runtime, &build(), &full);
    assert_eq!(resolution.order, ["sandbox_second", "sandbox_first", "full_second", "full_first"]);
    assert_eq!(runtime.entries.iter().map(|entry| entry.manifest.id.as_str()).collect::<Vec<_>>(), ["sandbox_second", "sandbox_first"]);
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
    let (path, digest) =
        binary_preflight(&runtime.entries[0], &build(), &[key.public()], false).unwrap();
    assert_eq!(path, entry.root.join("bin/windows-x64/sample.dll"));
    assert_eq!(
        digest,
        yak::hash_installed_file(&entry.root, "bin/windows-x64/sample.dll").unwrap()
    );
    assert_eq!(digest.len(), 64);
    assert!(binary_preflight(&entry, &build(), &[], false).is_err());
}

#[test]
fn tampered_dll_manifest_and_signature_are_refused_before_execution() {
    let fixture = Fixture::new();
    let entry = fixture.binary();
    let (key, _) = SigningKey::generate().unwrap();
    fixture.sign(&entry, &key);
    fixture.write(
        &entry.root.join("bin/windows-x64/sample.dll"),
        b"tampered bytes",
    );
    assert!(
        binary_preflight(&entry, &build(), &[key.public()], false)
            .unwrap_err()
            .contains("checksum mismatch")
    );
    fixture.sign(&entry, &key);
    fixture.write(
        &entry.root.join("yak.toml"),
        program().replace("Headless installed-view", "Changed installed-view"),
    );
    assert!(
        binary_preflight(&entry, &build(), &[key.public()], false)
            .unwrap_err()
            .contains("yak.toml")
    );
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
    fixture.write(
        &entry.root.join("yak.toml"),
        program().replace("sample_mod", "other_mod"),
    );
    assert!(
        binary_preflight(&entry, &build(), &[], true)
            .unwrap_err()
            .contains("identity changed")
    );
    fixture.write(
        &entry.root.join("yak.toml"),
        program().replace(r#"library = "sample""#, r#"library = "../outside""#),
    );
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
    populate_rows_from(
        &mut rows,
        &runtime.entries,
        &BTreeMap::new(),
        "Native port required",
    );
    assert_eq!(rows.len(), 1);
    assert!(rows[0].description.contains(".yak"));
    assert!(rows[0].description.contains("Unsigned or unverified"));
    assert_eq!(rows[0].problem.as_deref(), Some("Native port required"));
    let problems = BTreeMap::from([("sample_mod".into(), "Native signature refused".into())]);
    populate_rows_from(
        &mut rows,
        &runtime.entries,
        &problems,
        "Native port required",
    );
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
    assert_eq!(
        shader.options["shadow_map"].default,
        yak::OptionValue::Integer(1024)
    );
    let dir = fixture.0.join("shaderpacks");
    let packs = shaders::scan_installed(&dir, std::slice::from_ref(&entry));
    assert_eq!(packs.len(), 1);
    assert_eq!(packs[0].title, "Kage (.yak)");
    assert_eq!(packs[0].kind, shader_pack_list::Kind::Horizon);
    let mut selected = packs[0].clone();
    let index = selected
        .options
        .iter()
        .position(|option| option.id == "shadow_map")
        .unwrap();
    assert!(selected.options[index].name.contains("影の解像度"));
    assert_eq!(selected.values[index], 1.0);
    assert!(selected.step_option(index, false, false));
    fs::create_dir_all(&dir).unwrap();
    selected.save_values(&dir);
    assert!(dir.join(format!("{}.txt", selected.file)).is_file());
    assert!(!entry.root.join("shaders/kage.txt").exists());
    let mut pack = sample_pack();
    let constants =
        shaders::apply_parameters(&mut pack, shader, &dir.join(&selected.file)).unwrap();
    assert!(!constants.contains_key("shadow_map"));
    assert_eq!(constants["reflections"], 1.0);
    assert_eq!(constants["tonemap"], 1.0);
    assert_eq!(pack.manifest.shadow.as_ref().unwrap().resolution, 2048);
    let report = horizon_shader_api::check(&pack).unwrap();
    assert_eq!(report.shadow.unwrap().resolution, 2048);
    let mut option_pack = sample_pack();
    option_pack
        .manifest
        .options
        .push(horizon_shader_api::PackOption {
            id: "shadow_map".into(),
            name: "Shadow size".into(),
            description: String::new(),
            kind: horizon_shader_api::OptionKind::Choice {
                values: vec!["2048".into(), "1024".into(), "512".into()],
                default: 1,
            },
        });
    option_pack
        .manifest
        .shadow
        .as_mut()
        .unwrap()
        .resolution_option = Some("shadow_map".into());
    let selection =
        shaders::apply_parameters(&mut option_pack, shader, &dir.join(&selected.file)).unwrap();
    assert_eq!(selection["shadow_map"], 0.0);
    let constants: Vec<_> = selection.into_iter().collect();
    assert_eq!(
        option_pack.manifest.shadow_resolution(&constants),
        Some(2048)
    );
    selected.reset_options();
    selected.save_values(&dir);
    let mut pack = sample_pack();
    shaders::apply_parameters(&mut pack, shader, &dir.join(&selected.file)).unwrap();
    assert_eq!(pack.manifest.shadow.unwrap().resolution, 1024);
}

#[test]
fn unsupported_shadow_sizes_are_refused_before_pack_check() {
    let fixture = Fixture::new();
    let entry = fixture.install(
        &shader_source()
            .replace("[512, 1024, 2048]", "[512, 768, 2048]")
            .replace("default = 1024", "default = 768"),
    );
    let mut pack = sample_pack();
    assert!(
        shaders::apply_parameters(
            &mut pack,
            &entry.manifest.shaders[0],
            &fixture.0.join("preference")
        )
        .is_err()
    );
}

fn wasm_entry(fixture: &Fixture) -> Installed {
    let entry = fixture.install(&(data("sample_mod") + "\n[wasm]\ninterface = \"1.0\"\nclient = \"bin/wasm/sample.wasm\"\n"));
    fixture.write(&entry.root.join("bin/wasm/sample.wasm"), b"\0asm\x01\0\0\0");
    entry
}

#[test]
fn wasm_resolution_is_empty_without_wasm_entries() {
    let fixture = Fixture::new();
    let entry = fixture.binary();
    assert!(wasm_mods_from(&[entry], WasmSide::Client, &[], true).is_empty());
    assert!(wasm_mods_from(&[], WasmSide::Client, &[], true).is_empty());
}

#[test]
fn wasm_only_unsigned_switch_loads_wasm_and_never_authorizes_native_binaries() {
    let fixture = Fixture::new();
    let entry = wasm_entry(&fixture);
    assert!(wasm_mods_from(std::slice::from_ref(&entry), WasmSide::Client, &[], false).is_empty());
    let wasm_only = UnsignedSwitches::from_values(None, Some(std::ffi::OsStr::new("1")));
    let found = wasm_mods_from(std::slice::from_ref(&entry), WasmSide::Client, &[], wasm_only.wasm);
    assert_eq!(found.len(), 1);
    assert_eq!(found[0].mod_id, "sample_mod");
    assert_eq!(found[0].version, Version::parse("1.2.3").unwrap());
    assert_eq!(found[0].bytes, b"\0asm\x01\0\0\0");
    assert_eq!(found[0].sha256, yak::hash_installed_file(&entry.root, "bin/wasm/sample.wasm").unwrap());
    assert!(wasm_mods_from(std::slice::from_ref(&entry), WasmSide::Server, &[], true).is_empty());

    let native = fixture.install(&program().replace("sample_mod", "native_mod"));
    fixture.write(&native.root.join("bin/windows-x64/sample.dll"), b"not an executable DLL");
    assert!(binary_preflight(&native, &build(), &[], wasm_only.dll).is_err());

    let no_switch = UnsignedSwitches::from_values(None, None);
    assert!(wasm_mods_from(std::slice::from_ref(&entry), WasmSide::Client, &[], no_switch.wasm).is_empty());
    assert!(binary_preflight(&native, &build(), &[], no_switch.dll).is_err());

    let legacy = UnsignedSwitches::from_values(Some(std::ffi::OsStr::new("1")), None);
    assert!(binary_preflight(&native, &build(), &[], legacy.dll).is_ok());
    assert_eq!(wasm_mods_from(std::slice::from_ref(&entry), WasmSide::Client, &[], legacy.wasm).len(), 1);

    let (key, _) = SigningKey::generate().unwrap();
    fixture.sign(&entry, &key);
    assert_eq!(wasm_mods_from(std::slice::from_ref(&entry), WasmSide::Client, &[key.public()], false).len(), 1);
    fixture.write(&entry.root.join("bin/wasm/sample.wasm"), b"tampered");
    assert!(wasm_mods_from(std::slice::from_ref(&entry), WasmSide::Client, &[key.public()], false).is_empty());
}

#[test]
fn wasm_unsigned_override_accepts_a_missing_signature_but_refuses_a_tampered_one() {
    let fixture = Fixture::new();
    let entry = wasm_entry(&fixture);
    assert_eq!(wasm_mods_from(std::slice::from_ref(&entry), WasmSide::Client, &[], true).len(), 1);

    let (key, _) = SigningKey::generate().unwrap();
    fixture.sign(&entry, &key);
    assert_eq!(wasm_mods_from(std::slice::from_ref(&entry), WasmSide::Client, &[key.public()], true).len(), 1);
    fixture.write(&entry.root.join("bin/wasm/sample.wasm"), b"tampered after signing");
    assert!(wasm_mods_from(std::slice::from_ref(&entry), WasmSide::Client, &[key.public()], true).is_empty());
}

fn server_wasm_source(permissions: &str) -> String {
    data("server_mod").replace("client = true\nserver = true", "client = false\nserver = true")
        + &format!("\n[wasm]\ninterface = \"1.0\"\nserver = \"bin/wasm/server.wasm\"\n{permissions}")
}

#[test]
fn server_only_wasm_is_discovered_without_loading_a_client_module() {
    let fixture = Fixture::new();
    let entry = fixture.install(&server_wasm_source(""));
    fixture.write(&entry.root.join("bin/wasm/server.wasm"), b"\0asm\x01\0\0\0");
    fixture.list("server_mod 1.2.3\n");
    let runtime = Runtime::read(&fixture.0, &build(), &[]);
    assert_eq!(runtime.entries.len(), 1, "{:?}", runtime.startup_log);
    let found = wasm_mods_from(&runtime.entries, WasmSide::Server, &[], true);
    assert_eq!(found.len(), 1);
    assert!(found[0].full && !found[0].commands);
    assert!(wasm_mods_from(&runtime.entries, WasmSide::Client, &[], true).is_empty());
}

#[test]
fn the_commands_permission_reaches_the_server_module() {
    let fixture = Fixture::new();
    let entry = fixture.install(&server_wasm_source("permissions = [\"service.commands\"]\n"));
    fixture.write(&entry.root.join("bin/wasm/server.wasm"), b"\0asm\x01\0\0\0");
    let found = wasm_mods_from(std::slice::from_ref(&entry), WasmSide::Server, &[], true);
    assert_eq!(found.len(), 1);
    assert!(found[0].commands);
}

#[test]
fn config_sync_is_enabled_only_for_a_capable_synced_server_module() {
    let fixture = Fixture::new();
    let config = "\n[config.shared]\ntype = 'bool'\ndefault = false\nside = 'synced'\n";
    let entry =
        fixture.install(&(server_wasm_source("permissions = ['service.config']\n") + config));
    assert!(entry.has_synced_config());
    let mut local_only = entry.clone();
    local_only.manifest.config.get_mut("shared").unwrap().side = yak::ConfigSide::Client;
    assert!(!local_only.has_synced_config());
    let mut denied = entry.clone();
    denied.manifest.wasm.as_mut().unwrap().permissions.clear();
    assert!(!denied.has_synced_config());
    let mut client_only = entry.clone();
    client_only.manifest.wasm.as_mut().unwrap().server = None;
    assert!(!client_only.has_synced_config());
    fixture.list("server_mod 1.2.3\n");
    let runtime = Runtime::read(&fixture.0, &build(), &[]);
    assert!(runtime.synced_config);
}

/// MX3L2: a full-access package is a core module for the `engine_call` ABI; the sandboxed component path must
/// skip it (and refuse it in preflight) instead of trying to load it. Feature-gated so the feature-off test count
/// of the crate stays as it was.
#[cfg(feature = "yoake2-slice")]
#[test]
fn the_sandboxed_path_skips_full_access_packages() {
    let fixture = Fixture::new();
    let entry = fixture.install(&server_wasm_source("world = \"full-access\"\n"));
    assert!(entry.manifest.wasm.as_ref().unwrap().full_access());
    fixture.write(&entry.root.join("bin/wasm/server.wasm"), b"\0asm\x01\0\0\0");
    assert!(wasm_mods_from(std::slice::from_ref(&entry), WasmSide::Server, &[], true).is_empty(), "skipped on the server side");
    assert!(wasm_mods_from(std::slice::from_ref(&entry), WasmSide::Client, &[], true).is_empty());
    assert!(wasm_preflight(&entry, "bin/wasm/server.wasm", WasmSide::Server, &[], true).unwrap_err().contains("full-access"), "and refused if asked directly");
    // an ordinary server module next to it still loads
    let plain = fixture.install(&server_wasm_source("").replace("server_mod", "plain_mod"));
    fixture.write(&plain.root.join("bin/wasm/server.wasm"), b"\0asm\x01\0\0\0");
    let both = [entry, plain];
    let found = wasm_mods_from(&both, WasmSide::Server, &[], true);
    assert_eq!(found.iter().map(|m| m.mod_id.as_str()).collect::<Vec<_>>(), ["plain_mod"]);
}

/// MX3JN1: a `.yak` as the launcher stores a host's package: `<mods>/remote/<host dir>/<sha256>.yak`.
fn remote_package(fixture: &Fixture, key: &str, id: &str, extra: &str, name_it_right: bool) -> PathBuf {
    let src = fixture.0.join(format!("src_{id}"));
    let source = data(id).replace("server = true", "server = false") + extra;
    fixture.write(&src.join("yak.toml"), &source);
    fixture.write(&src.join("bin/wasm/sample.wasm"), b"\0asm\x01\0\0\0");
    let out = fixture.0.join(format!("{id}.yak"));
    horizon_mods::yak::pack_directory(&src, &out, &BTreeMap::new()).unwrap();
    let bytes = fs::read(&out).unwrap();
    let hash = yak::control::sha256_hex(&bytes);
    let name = if name_it_right { hash.clone() } else { "f".repeat(64) };
    let folder = yak::control::host_remote_dir(&fixture.0.join("mods"), key).unwrap();
    fs::create_dir_all(&folder).unwrap();
    fs::write(folder.join(format!("{name}.yak")), bytes).unwrap();
    folder.join(format!("{name}.yak"))
}

#[test]
fn a_hosts_sandboxed_packages_are_installed_for_that_host_only() {
    let fixture = Fixture::new();
    let key = "server:play.example.com:25565";
    let wasm = "\n[wasm]\ninterface = \"1.0\"\nclient = \"bin/wasm/sample.wasm\"\n";
    let good = remote_package(&fixture, key, "host_mod", wasm, true);
    let hash = good.file_stem().unwrap().to_str().unwrap().to_owned();
    // refused: not named by its hash, full-access (only the approved list may run those), server-only
    remote_package(&fixture, key, "misnamed_mod", wasm, false);
    remote_package(&fixture, key, "full_mod", &format!("{wasm}world = \"full-access\"\n"), true);
    let server_side = data("server_mod").replace("client = true", "client = false") + "\n[wasm]\ninterface = \"1.0\"\nserver = \"bin/wasm/sample.wasm\"\n";
    let src = fixture.0.join("src_server_mod");
    fixture.write(&src.join("yak.toml"), &server_side);
    fixture.write(&src.join("bin/wasm/sample.wasm"), b"\0asm\x01\0\0\0");
    let out = fixture.0.join("server_mod.yak");
    horizon_mods::yak::pack_directory(&src, &out, &BTreeMap::new()).unwrap();
    let bytes = fs::read(&out).unwrap();
    fs::write(good.with_file_name(format!("{}.yak", yak::control::sha256_hex(&bytes))), bytes).unwrap();
    // a file that hashes to other than its name
    let liar = good.with_file_name(format!("{}.yak", "a".repeat(64)));
    fs::copy(&good, &liar).unwrap();

    let found = remote_installed(&fixture.0, key, &build(), &[]);
    assert_eq!(found.iter().map(|entry| entry.manifest.id.as_str()).collect::<Vec<_>>(), ["host_mod"]);
    assert_eq!(found[0].root, fixture.0.join("yoake-remote").join("server+play.example.com+25565").join(&hash));
    assert!(found[0].root.join("yak.toml").is_file() && !found[0].signed);
    // unpacked once: a second read reuses the folder
    assert_eq!(remote_installed(&fixture.0, key, &build(), &[]).len(), 1);
    // another host's key, a friend key, and a key that is no key see none of it
    for other in ["server:other.example.com:25565", "friend:user_7", "server:../x"] {
        assert!(remote_installed(&fixture.0, other, &build(), &[]).is_empty(), "{other}");
    }
    // the code is still judged like any package's: no trusted signature, no load
    assert!(wasm_mods_from(&found, WasmSide::Client, &[], false).is_empty());
    assert_eq!(wasm_mods_from(&found, WasmSide::Client, &[], true).len(), 1, "only the developer override loads an unsigned one");
}

/// R1: a full-access-only install has no `yoake-mods.txt` (the launcher leaves it out); the game root is still known, so
/// the full-access loader finds `<root>/mods`.
#[test]
fn the_game_root_is_known_without_a_yoake_mods_list() {
    let fixture = Fixture::new();
    fixture.write(&fixture.0.join("mods").join("full.yak"), b"not read here");
    let runtime = startup_runtime(&fixture.0, false);
    assert_eq!(runtime.game_root, fixture.0);
    assert!(runtime.entries.is_empty() && runtime.startup_log.is_empty());
}
