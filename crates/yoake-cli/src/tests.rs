use super::*;
use std::sync::atomic::{AtomicU64, Ordering};
use yoake_wasm::test_guests::{self, Guest};

static NEXT_PROJECT: AtomicU64 = AtomicU64::new(0);

struct Project {
    root: PathBuf,
}

impl Project {
    fn new(id: &str, client: Option<&[u8]>, server: Option<&[u8]>, world: Option<&str>) -> Self {
        let timestamp = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        let root = env::temp_dir().join(format!(
            "yoake-cli-test-{}-{timestamp}-{}",
            process::id(),
            NEXT_PROJECT.fetch_add(1, Ordering::Relaxed)
        ));
        fs::create_dir(&root).unwrap();
        fs::create_dir_all(root.join("bin/wasm")).unwrap();

        let mut manifest = format!(
            "format = 1\nid = \"{id}\"\nversion = \"0.1.0\"\nname = \"Validation Demo\"\nauthors = [\"Test\"]\ndescription = \"Fixture\"\n\n[yoake]\nversion = \"*\"\n\n[side]\nclient = {}\nserver = {}\nrequired_on_join = false\n\n[wasm]\ninterface = \"1.0\"\n",
            client.is_some(),
            server.is_some()
        );
        if let Some(bytes) = client {
            fs::write(root.join("bin/wasm/client.wasm"), bytes).unwrap();
            manifest.push_str("client = \"bin/wasm/client.wasm\"\n");
        }
        if let Some(bytes) = server {
            fs::write(root.join("bin/wasm/server.wasm"), bytes).unwrap();
            manifest.push_str("server = \"bin/wasm/server.wasm\"\n");
        }
        if let Some(world) = world {
            manifest.push_str(&format!("world = \"{world}\"\n"));
        }
        fs::write(root.join("yak.toml"), manifest).unwrap();
        Self { root }
    }
}

impl Drop for Project {
    fn drop(&mut self) {
        fs::remove_dir_all(&self.root).unwrap();
    }
}

fn check_project(project: &Project) -> Result<serde_json::Value> {
    let manifest = Manifest::read(&project.root.join("yak.toml"))?;
    check(&project.root, &manifest)
}

fn pack_project(project: &Project) -> Result<PathBuf> {
    let manifest = Manifest::read(&project.root.join("yak.toml"))?;
    pack(&project.root, &manifest)
}

fn assert_client_world(guest: &Guest) {
    let Some(bytes) = test_guests::load(guest) else {
        return;
    };
    let project = Project::new("hello-test", Some(&bytes), None, None);
    fs::write(project.root.join("WASM-NEEDS.md"), "Fixture needs\n").unwrap();

    let summary = check_project(&project).unwrap();
    assert_eq!(summary["ok"], true);
    assert_eq!(summary["id"], "hello-test");
    assert_eq!(summary["version"], "0.1.0");
    assert_eq!(summary["modules"][0]["side"], "client");
    assert_eq!(summary["modules"][0]["path"], "bin/wasm/client.wasm");
    assert_eq!(summary["modules"][0]["world"], "client-mod");
    assert_eq!(summary["wasm_needs"], json!(["Fixture needs"]));
}

#[test]
fn checks_hello() {
    assert_client_world(&test_guests::HELLO);
}

#[test]
fn checks_hello_std() {
    assert_client_world(&test_guests::HELLO_STD);
}

#[test]
fn checks_full_demo_client_and_server() {
    let Some(client) = test_guests::load(&test_guests::FULL_DEMO_CLIENT) else {
        return;
    };
    let Some(server) = test_guests::load(&test_guests::FULL_DEMO_SERVER) else {
        return;
    };
    let project = Project::new("full-demo", Some(&client), Some(&server), Some("client-full"));

    let summary = check_project(&project).unwrap();
    assert_eq!(summary["ok"], true);
    assert_eq!(summary["modules"].as_array().unwrap().len(), 2);
    assert_eq!(summary["modules"][0]["world"], "client-full");
    assert_eq!(summary["modules"][1]["world"], "server-full");
}

#[test]
fn checks_hello_example_manifest() {
    let Some(client) = test_guests::load(&test_guests::HELLO) else {
        return;
    };
    let project = Project::new("hello-example", None, None, None);
    fs::write(
        project.root.join("yak.toml"),
        include_str!("../../yoake-wasm-sdk/examples/hello/yak.toml"),
    )
    .unwrap();
    fs::write(project.root.join("bin/wasm/hello.wasm"), client).unwrap();

    let summary = check_project(&project).unwrap();
    assert_eq!(summary["id"], "wasm-hello");
    assert_eq!(summary["modules"][0]["world"], "client-mod");
}

#[test]
fn checks_full_demo_example_manifest() {
    let Some(client) = test_guests::load(&test_guests::FULL_DEMO_CLIENT) else {
        return;
    };
    let Some(server) = test_guests::load(&test_guests::FULL_DEMO_SERVER) else {
        return;
    };
    let project = Project::new("full-example", None, None, None);
    fs::write(
        project.root.join("yak.toml"),
        include_str!("../../yoake-wasm-sdk/examples/full-demo/yak.toml"),
    )
    .unwrap();
    fs::write(project.root.join("bin/wasm/full-demo-client.wasm"), client).unwrap();
    fs::write(project.root.join("bin/wasm/full-demo-server.wasm"), server).unwrap();

    let summary = check_project(&project).unwrap();
    assert_eq!(summary["id"], "full-demo");
    assert_eq!(summary["modules"][0]["world"], "client-full");
    assert_eq!(summary["modules"][1]["world"], "server-full");
}

#[test]
fn rejects_missing_required_exports() {
    let project = Project::new("broken-mod", Some(b"\0asm\x0d\0\x01\0"), None, None);

    let error = check_project(&project).unwrap_err().to_string();
    assert!(error.contains("client-mod"), "{error}");
    assert!(error.contains("bin/wasm/client.wasm"), "{error}");
}

#[test]
fn rejects_component_for_a_different_world() {
    let Some(client) = test_guests::load(&test_guests::HELLO) else {
        return;
    };
    let project = Project::new("wrong-world", Some(&client), None, Some("client-full"));

    let error = check_project(&project).unwrap_err().to_string();
    assert!(error.contains("client-full"), "{error}");
    assert!(error.contains("bin/wasm/client.wasm"), "{error}");
}

#[test]
fn rejects_invalid_component() {
    let project = Project::new("invalid-mod", Some(b"not a component"), None, None);

    let error = check_project(&project).unwrap_err().to_string();
    assert!(error.contains("client-mod"), "{error}");
    assert!(error.contains("not a valid component"), "{error}");
}

#[test]
fn rejects_unknown_permission() {
    let project = Project::new("bad-permission", Some(b"unused"), None, None);
    let manifest_path = project.root.join("yak.toml");
    let mut manifest = fs::read_to_string(&manifest_path).unwrap();
    manifest.push_str("permissions = [\"not.a.permission\"]\n");
    fs::write(manifest_path, manifest).unwrap();

    let error = check_project(&project).unwrap_err().to_string();
    assert!(error.contains("unknown wasm permission"), "{error}");
}

#[test]
fn rejects_oversized_component() {
    let project = Project::new("oversized-mod", Some(b"unused"), None, None);
    fs::OpenOptions::new()
        .write(true)
        .open(project.root.join("bin/wasm/client.wasm"))
        .unwrap()
        .set_len(MAX_WASM_SIZE + 1)
        .unwrap();

    let error = check_project(&project).unwrap_err().to_string();
    assert!(error.contains("size limit"), "{error}");
}

#[test]
fn bundled_sdk_contains_wit_and_macro_sources() {
    let project = Project::new("bundled-test", Some(b"unused"), None, None);
    scaffold::write_bundled_sdk(&project.root).unwrap();
    let sdk = project.root.join(".yoake/yoake-wasm-sdk");
    assert!(sdk.join("Cargo.toml").is_file());
    assert!(sdk.join("src/full.rs").is_file());
    assert!(sdk.join("test-macros/src/lib.rs").is_file());
    assert!(sdk.join("../yoake-wasm/wit/yoake-mod.wit").is_file());
    assert!(sdk.join("../yoake-wasm/wit/yoake-full.wit").is_file());
}

#[test]
fn dev_install_creates_and_replaces_component() {
    let project = Project::new("dev-test", Some(b"first"), None, None);
    let manifest = Manifest::read(&project.root.join("yak.toml")).unwrap();
    let directory = project.root.join("developer");

    install_for_reload(&project.root, &manifest, &directory).unwrap();
    assert_eq!(fs::read(directory.join("dev-test.wasm")).unwrap(), b"first");
    fs::write(project.root.join("bin/wasm/client.wasm"), b"second").unwrap();
    install_for_reload(&project.root, &manifest, &directory).unwrap();
    assert_eq!(fs::read(directory.join("dev-test.wasm")).unwrap(), b"second");
    assert_eq!(fs::read_dir(&directory).unwrap().count(), 1);
}

#[test]
fn content_demo_package_replaces_same_version() {
    let Some(client) = test_guests::load(&test_guests::HELLO) else {
        return;
    };
    let project = Project::new("content-demo", Some(&client), None, None);
    fs::create_dir(project.root.join("content")).unwrap();
    let content = project.root.join("content/demo.json");
    fs::write(&content, "{\"revision\":1}").unwrap();
    check_project(&project).unwrap();

    let first = pack_project(&project).unwrap();
    assert_eq!(first, project.root.join("dist/content-demo-0.1.0.yak"));
    let first_extract = project.root.join("first-extract");
    {
        let mut archive = horizon_mods::yak::Archive::new(File::open(&first).unwrap()).unwrap();
        archive
            .extract_target(&first_extract, horizon_mods::yak::Target::WindowsX64, &[])
            .unwrap();
    }
    assert_eq!(fs::read_to_string(first_extract.join("content/demo.json")).unwrap(), "{\"revision\":1}");

    fs::write(&content, "{\"revision\":2}").unwrap();
    let second = pack_project(&project).unwrap();
    assert_eq!(second, first);
    let second_extract = project.root.join("second-extract");
    {
        let mut archive = horizon_mods::yak::Archive::new(File::open(&second).unwrap()).unwrap();
        archive
            .extract_target(&second_extract, horizon_mods::yak::Target::WindowsX64, &[])
            .unwrap();
    }
    assert_eq!(fs::read_to_string(second_extract.join("content/demo.json")).unwrap(), "{\"revision\":2}");
    assert_eq!(fs::read_dir(project.root.join("dist")).unwrap().count(), 1);
}

#[test]
fn rejects_invalid_init_subscriptions_and_traps() {
    for (guest, side, world, expected) in [
        (&test_guests::INVALID_HOOK, "client", None, "tock"),
        (&test_guests::TRAPPING_INIT, "client", None, "init"),
        (&test_guests::UNWIRED_CLIENT_HOOK, "client", Some("client-full"), "input_key"),
        (&test_guests::WRONG_SERVER_HOOK, "server", None, "hud"),
    ] {
        let Some(bytes) = test_guests::load(guest) else {
            continue;
        };
        let project = if side == "client" {
            Project::new("invalid-init", Some(&bytes), None, world)
        } else {
            Project::new("invalid-init", None, Some(&bytes), world)
        };
        let error = check_project(&project).unwrap_err();
        assert!(error.contains(expected), "expected {expected:?} in {error:?}");
    }
}

#[test]
fn full_world_unwired_hooks_match_game() {
    let game = include_str!("../../horizon-client/src/wasm_mods/full_adapters.rs");
    let declaration = format!("pub(crate) const UNWIRED: &[&str] = &{:?};", FULL_UNWIRED);
    assert!(game.contains(&declaration));
}

#[test]
fn pack_builds_wasm_before_checking_it() {
    let Some(bytes) = test_guests::load(&test_guests::HELLO) else {
        return;
    };
    let project = Project::new("rebuild-pack", Some(b"stale"), None, None);
    let summary = process_project(&project.root, "pack", |root, _| {
        fs::write(root.join("bin/wasm/client.wasm"), &bytes).map_err(|error| error.to_string())
    }).unwrap();
    assert_eq!(summary["ok"], true);
    assert!(project.root.join("dist/rebuild-pack-0.1.0.yak").is_file());
}

#[test]
fn packs_data_only_and_native_only_without_building_wasm() {
    for native in [false, true] {
        let project = Project::new("non-wasm", None, None, None);
        let manifest_path = project.root.join("yak.toml");
        let manifest = fs::read_to_string(&manifest_path).unwrap()
            .replace("client = false", "client = true")
            .replace("\n[wasm]\ninterface = \"1.0\"\n", "\n");
        let manifest = if native {
            fs::create_dir_all(project.root.join("bin/windows-x64")).unwrap();
            fs::write(project.root.join("bin/windows-x64/sample.dll"), b"native fixture").unwrap();
            format!("{manifest}\n[bin]\nlibrary = \"sample\"\ntargets = [\"windows-x64\"]\nengine = \"1.0.0\"\nabi = 1\n")
        } else {
            fs::create_dir(project.root.join("content")).unwrap();
            fs::write(project.root.join("content/example.json"), "{}\n").unwrap();
            manifest
        };
        fs::write(manifest_path, manifest).unwrap();
        let summary = process_project(&project.root, "pack", |_, _| panic!("unexpected Wasm build")).unwrap();
        assert_eq!(summary["ok"], true);
        let archive_path = project.root.join("dist/non-wasm-0.1.0.yak");
        let mut archive = horizon_mods::yak::Archive::new(File::open(archive_path).unwrap()).unwrap();
        let extract = project.root.join("extracted");
        archive.extract_target(&extract, horizon_mods::yak::Target::WindowsX64, &[]).unwrap();
        let expected = if native { "bin/windows-x64/sample.dll" } else { "content/example.json" };
        assert!(extract.join(expected).is_file());
    }
}

#[test]
fn dev_reports_success_when_hot_reload_is_unavailable() {
    for (client, server, world, directory, reason) in [
        (Some(b"guest".as_slice()), None, None, None, "YOAKE_WASM_DEV_DIR"),
        (Some(b"guest".as_slice()), None, Some("client-full"), Some("reload"), "client-mod"),
        (None, Some(b"guest".as_slice()), None, Some("reload"), "client-mod"),
    ] {
        let project = Project::new("dev-unavailable", client, server, world);
        let directory = directory.map(|name| project.root.join(name));
        let result = dev_project(&project.root, directory.as_deref(), |_, _| Ok(()), |_, _| Ok(()),
            |_, _, _| panic!("unsupported reload attempted"));
        assert_eq!(result["ok"], true);
        assert_eq!(result["built"], true);
        assert_eq!(result["checked"], true);
        assert_eq!(result["installed"], false);
        assert!(result["notes"].to_string().contains(reason));
    }
}

#[test]
fn dev_reports_each_stage_and_failure() {
    let project = Project::new("dev-stages", Some(b"guest"), None, None);
    let directory = project.root.join("reload");
    let success = dev_project(&project.root, Some(&directory), |_, _| Ok(()), |_, _| Ok(()), |_, _, _| Ok(()));
    assert_eq!((success["ok"].as_bool(), success["built"].as_bool(), success["checked"].as_bool(), success["installed"].as_bool()),
        (Some(true), Some(true), Some(true), Some(true)));
    let failed_build = dev_project(&project.root, Some(&directory), |_, _| Err("build failed".into()), |_, _| Ok(()), |_, _, _| Ok(()));
    assert_eq!((failed_build["ok"].as_bool(), failed_build["built"].as_bool(), failed_build["checked"].as_bool()),
        (Some(false), Some(false), Some(false)));
    let failed_check = dev_project(&project.root, Some(&directory), |_, _| Ok(()), |_, _| Err("check failed".into()), |_, _, _| Ok(()));
    assert_eq!((failed_check["ok"].as_bool(), failed_check["built"].as_bool(), failed_check["checked"].as_bool()),
        (Some(false), Some(true), Some(false)));
    let failed_install = dev_project(&project.root, Some(&directory), |_, _| Ok(()), |_, _| Ok(()), |_, _, _| Err("install failed".into()));
    assert_eq!((failed_install["ok"].as_bool(), failed_install["built"].as_bool(), failed_install["checked"].as_bool(), failed_install["installed"].as_bool()),
        (Some(false), Some(true), Some(true), Some(false)));
}

#[test]
fn resolves_configured_cargo_target_directory() {
    let project = Project::new("target-test", None, None, None);
    assert_eq!(target_directory(&project.root, None), project.root.join("target"));
    assert_eq!(target_directory(&project.root, Some(OsStr::new("build-cache"))), project.root.join("build-cache"));
    let absolute = env::temp_dir().join("external-cargo-target");
    assert_eq!(target_directory(&project.root, Some(absolute.as_os_str())), absolute);
}

#[test]
fn rejects_windows_reserved_names_and_reports_reparse_paths() {
    for name in ["con", "CON.txt", "Aux.log", "nul", "PRN", "com1", "COM9.txt", "lpt1", "LPT9.log"] {
        assert!(reserved_name(name), "{name}");
    }
    for name in ["console", "com0", "com10", "lpt0", "project.con"] {
        assert!(!reserved_name(name), "{name}");
    }
    for (original, replacement) in [("id = \"safe\"", "id = \"con\""), ("name = \"Validation Demo\"", "name = \"Aux.txt\"")] {
        let project = Project::new("safe", Some(b"guest"), None, None);
        let path = project.root.join("yak.toml");
        let manifest = fs::read_to_string(&path).unwrap().replace(original, replacement);
        fs::write(path, manifest).unwrap();
        assert!(read_manifest(&project.root).unwrap_err().contains("reserved"));
    }
    let error = explain_error("C:\\link\\project is a link or reparse point");
    assert!(error.contains("C:\\link\\project"));
    assert!(error.contains("move the project to a normal folder"));
}

#[test]
#[cfg(unix)]
fn rejects_project_inside_symlink_with_actionable_error() {
    use std::os::unix::fs::symlink;

    let project = Project::new("linked-project", None, None, None);
    let root = fs::canonicalize(&project.root).unwrap();
    let link = root.join("project-link");
    symlink(&root, &link).unwrap();
    let error = explain_error(&read_manifest(&link).unwrap_err());
    assert!(error.contains(&link.display().to_string()));
    assert!(error.contains("move the project to a normal folder"));
}

#[test]
fn scaffold_failure_removes_staging_without_creating_target() {
    let project = Project::new("scaffold-test", None, None, None);
    let result = scaffold::stage_project(&project.root, "fresh", |staging| {
        fs::write(staging.join("partial"), "unfinished").map_err(|error| error.to_string())?;
        Err("synthetic failure".into())
    });
    assert!(result.unwrap_err().contains("synthetic failure"));
    assert!(!project.root.join("fresh").exists());
    assert!(fs::read_dir(&project.root).unwrap().all(|entry| !entry.unwrap().file_name().to_string_lossy().starts_with(".yoake-fresh-")));
}
