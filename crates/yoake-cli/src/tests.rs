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
        if client.is_some() {
            manifest.push_str("client = \"bin/wasm/client.wasm\"\n");
        }
        if server.is_some() {
            manifest.push_str("server = \"bin/wasm/server.wasm\"\n");
        }
        if let Some(world) = world {
            manifest.push_str(&format!("world = \"{world}\"\n"));
        }
        fs::write(root.join("yak.toml"), manifest).unwrap();
        if let Some(bytes) = client {
            fs::write(root.join("bin/wasm/client.wasm"), bytes).unwrap();
        }
        if let Some(bytes) = server {
            fs::write(root.join("bin/wasm/server.wasm"), bytes).unwrap();
        }
        Self { root }
    }
}

impl Drop for Project {
    fn drop(&mut self) {
        crate::test_support::remove_dir_all(&self.root);
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
    assert!(sdk.join("src/gpu.rs").is_file());
    assert!(sdk.join("src/mixin.rs").is_file());
    assert!(sdk.join("src/engine.wgsl").is_file());
    assert!(sdk.join("test-macros/src/lib.rs").is_file());
    assert!(sdk.join("../yoake-wasm/wit/yoake-mod.wit").is_file());
    assert!(sdk.join("../yoake-wasm/wit/yoake-full.wit").is_file());
    assert!(sdk.join("../yoake-wasm/wit/yoake-test.wit").is_file());
    assert!(sdk.join("../yoake-wasm/wit/inject-sites.json").is_file());
    assert!(sdk.join("../yoake-wasm/wit/state-catalog.json").is_file());
    assert!(sdk.join("../yoake-wasm/wit/coverage.json").is_file());
    assert!(sdk.join("src/testing/engine.rs").is_file());
    assert!(!sdk.join("examples").exists());
    assert!(!sdk.join("target").exists());
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

fn assert_invalid_init(guest: &Guest, side: &str, world: Option<&str>, expected: &str) {
    let Some(bytes) = test_guests::load(guest) else {
        return;
    };
    let project = if side == "client" {
        Project::new("invalid-init", Some(&bytes), None, world)
    } else {
        Project::new("invalid-init", None, Some(&bytes), world)
    };
    let error = check_project(&project).unwrap_err();
    assert!(error.contains(expected), "expected {expected:?} in {error:?}");
    if expected == "hud" || expected == "server_about_to_start" {
        assert!(error.contains("client") && error.contains("server"), "{error}");
    }
    if expected == "init" {
        let json_error = json!({"ok": false, "errors": [explain_error(&error)]}).to_string();
        assert!(json_error.len() < 512, "{json_error}");
        assert!(!json_error.to_lowercase().contains("backtrace"), "{json_error}");
    }
}

#[test]
fn rejects_invalid_init_subscriptions_and_traps() {
    assert_invalid_init(&test_guests::INVALID_HOOK, "client", None, "tock");
    assert_invalid_init(&test_guests::TRAPPING_INIT, "client", None, "init");
}

#[test]
fn rejects_unwired_client_hook() {
    assert_invalid_init(&test_guests::UNWIRED_CLIENT_HOOK, "client", Some("client-full"), "input_key");
}

#[test]
fn rejects_server_hook_on_client() {
    assert_invalid_init(&test_guests::WRONG_CLIENT_HOOK, "client", Some("client-full"), "server_about_to_start");
}

#[test]
fn rejects_client_hook_on_server() {
    assert_invalid_init(&test_guests::WRONG_SERVER_HOOK, "server", None, "hud");
}

#[test]
fn check_rejects_stale_wasm_for_each_source_kind() {
    let project = Project::new("freshness", Some(b"not a component"), None, None);
    fs::create_dir_all(project.root.join("src/nested")).unwrap();
    for path in ["Cargo.toml", "src/lib.rs", "src/nested/module.rs"] {
        fs::write(project.root.join(path), "fixture").unwrap();
    }
    let module = project.root.join("bin/wasm/client.wasm");
    let old = UNIX_EPOCH + std::time::Duration::from_secs(1_600_000_000);
    let built = std::time::SystemTime::now() + std::time::Duration::from_secs(60);
    let newer = built + std::time::Duration::from_secs(60);
    let set_modified = |path: &Path, time| {
        File::options().write(true).open(path).unwrap().set_modified(time).unwrap();
    };
    let sources = ["yak.toml", "Cargo.toml", "src/lib.rs", "src/nested/module.rs"];
    for source in sources {
        set_modified(&project.root.join(source), old);
    }
    set_modified(&module, built);
    ensure_module_is_fresh(&project.root, &module, &fs::metadata(&module).unwrap()).unwrap();
    for source in sources {
        let path = project.root.join(source);
        set_modified(&path, newer);
        let error = check_project(&project).unwrap_err();
        assert!(error.contains("stale") && error.contains("run yoake build"), "{error}");
        assert!(error.contains(path.file_name().unwrap().to_str().unwrap()), "{error}");
        set_modified(&path, built);
        ensure_module_is_fresh(&project.root, &module, &fs::metadata(&module).unwrap()).unwrap();
    }
}

#[test]
fn check_rejects_stale_server_wasm_and_documents_fix() {
    let project = Project::new("server-freshness", None, Some(b"not a component"), None);
    let module = project.root.join("bin/wasm/server.wasm");
    let manifest = project.root.join("yak.toml");
    let old = UNIX_EPOCH + std::time::Duration::from_secs(1_600_000_000);
    File::options().write(true).open(module).unwrap().set_modified(old).unwrap();
    File::options().write(true).open(manifest).unwrap().set_modified(old + std::time::Duration::from_secs(1)).unwrap();
    let error = check_project(&project).unwrap_err();
    assert!(error.contains("server.wasm") && error.contains("stale") && error.contains("run yoake build"), "{error}");
    let summary = run(&["check".into(), "--help".into()]).unwrap();
    assert!(summary["help"].as_str().unwrap().contains("run yoake build"));
}

#[test]
fn example_wasm_artifacts_are_gitignored() {
    let workspace = Path::new(env!("CARGO_MANIFEST_DIR")).join("../..");
    for path in ["examples/demo/bin/wasm/client.wasm", "crates/yoake-wasm-sdk/examples/demo/bin/wasm/client.wasm"] {
        let status = Command::new("git").current_dir(&workspace)
            .args(["check-ignore", "--no-index", "--quiet", path]).status().unwrap();
        assert!(status.success(), "{path} is not ignored");
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
            |_, _, _| panic!("unsupported reload attempted"), |_, _| Ok(engine_test::TestRun::default()));
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
    let success = dev_project(&project.root, Some(&directory), |_, _| Ok(()), |_, _| Ok(()), |_, _, _| Ok(()), |_, _| Ok(engine_test::TestRun::default()));
    assert_eq!((success["ok"].as_bool(), success["built"].as_bool(), success["checked"].as_bool(), success["installed"].as_bool()),
        (Some(true), Some(true), Some(true), Some(true)));
    let failed_build = dev_project(&project.root, Some(&directory), |_, _| Err("build failed".into()), |_, _| Ok(()), |_, _, _| Ok(()), |_, _| Ok(engine_test::TestRun::default()));
    assert_eq!((failed_build["ok"].as_bool(), failed_build["built"].as_bool(), failed_build["checked"].as_bool()),
        (Some(false), Some(false), Some(false)));
    let failed_check = dev_project(&project.root, Some(&directory), |_, _| Ok(()), |_, _| Err("check failed".into()), |_, _, _| Ok(()), |_, _| Ok(engine_test::TestRun::default()));
    assert_eq!((failed_check["ok"].as_bool(), failed_check["built"].as_bool(), failed_check["checked"].as_bool()),
        (Some(false), Some(true), Some(false)));
    let failed_install = dev_project(&project.root, Some(&directory), |_, _| Ok(()), |_, _| Ok(()), |_, _, _| Err("install failed".into()), |_, _| Ok(engine_test::TestRun::default()));
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

// ---------------------------------------------------------------------------------------------
// yoake test: the engine's test runner
// ---------------------------------------------------------------------------------------------

use engine_test::{TestRun, TestSettings};

fn strings(list: &[&str]) -> Vec<String> {
    list.iter().map(|text| (*text).to_owned()).collect()
}

#[test]
fn test_options_are_read_and_the_game_comes_from_the_flag_or_the_environment() {
    let settings = engine_test::parse(&strings(&["--game", "g.exe", "--filter", "place", "--timeout-ms", "5000", "--clients", "2", "--keep", "--module", "t.wasm", "--", "--resources", "res"])).unwrap();
    assert_eq!(settings, TestSettings {
        game: Some("g.exe".into()), module: Some("t.wasm".into()), filter: Some("place".into()),
        timeout_ms: Some("5000".into()), clients: Some("2".into()), keep: true, extra: strings(&["--resources", "res"]),
    });
    assert!(engine_test::parse(&strings(&["--bogus"])).unwrap_err().contains("unknown option"));
    assert!(engine_test::parse(&strings(&["--game"])).unwrap_err().contains("missing value"));
    assert_eq!(engine_test::game_executable(&settings, Some(OsStr::new("env.exe"))), Some("g.exe".into()), "the flag wins");
    let none = TestSettings::default();
    assert_eq!(engine_test::game_executable(&none, Some(OsStr::new("env.exe"))), Some("env.exe".into()));
    assert_eq!(engine_test::game_executable(&none, Some(OsStr::new(""))), None);
    assert_eq!(engine_test::game_executable(&none, None), None);
}

#[test]
fn the_games_report_is_its_last_json_line_and_failures_become_errors() {
    let report = engine_test::parse_report("noise\n{\"ok\":false,\"tests\":[{\"name\":\"a\",\"ok\":true,\"ms\":3},{\"name\":\"b\",\"ok\":false,\"ms\":4,\"error\":\"expected stone\"}],\"errors\":[]}\n").unwrap();
    let run = engine_test::summarize(&report);
    assert_eq!(run.tests.len(), 2);
    assert_eq!(run.errors, ["test b failed: expected stone"]);
    assert!(engine_test::parse_report("no json here\n").unwrap_err().contains("no report"));
    assert!(engine_test::parse_report("{broken\n").unwrap_err().contains("not JSON"));
    // Set-up errors and notes come through; a failed run with no detail still says so.
    let run = engine_test::summarize(&serde_json::json!({"ok": false, "errors": ["no game"], "tests": []}));
    assert_eq!(run.errors, ["no game"]);
    let run = engine_test::summarize(&serde_json::json!({"ok": true, "tests": [], "note": "no test matches the filter"}));
    assert!(run.errors.is_empty());
    assert_eq!(run.notes, ["no test matches the filter"]);
    assert_eq!(engine_test::summarize(&serde_json::json!({"ok": false})).errors, ["the game reported a failed run"]);
}

#[test]
fn dev_fills_the_tests_and_fails_when_one_fails() {
    let project = Project::new("dev-tests", Some(b"guest"), None, None);
    let directory = project.root.join("reload");
    let passing = dev_project(&project.root, Some(&directory), |_, _| Ok(()), |_, _| Ok(()), |_, _, _| Ok(()), |_, _| {
        Ok(TestRun { tests: vec![serde_json::json!({"name": "a", "ok": true, "ms": 5})], ..TestRun::default() })
    });
    assert_eq!(passing["ok"], true);
    assert_eq!(passing["tests"], serde_json::json!([{"name": "a", "ok": true, "ms": 5}]));
    let failing = dev_project(&project.root, Some(&directory), |_, _| Ok(()), |_, _| Ok(()), |_, _, _| Ok(()), |_, _| {
        Ok(TestRun {
            tests: vec![serde_json::json!({"name": "b", "ok": false, "ms": 1, "error": "boom"})],
            errors: vec!["test b failed: boom".into()],
            notes: Vec::new(),
        })
    });
    assert_eq!(failing["ok"], false);
    assert_eq!(failing["built"], true);
    assert_eq!(failing["tests"][0]["name"], "b");
    assert_eq!(failing["errors"], serde_json::json!(["test b failed: boom"]));
    // The runner itself failing (no report) is an error of the run, with the tests left empty.
    let broken = dev_project(&project.root, Some(&directory), |_, _| Ok(()), |_, _| Ok(()), |_, _, _| Ok(()), |_, _| Err("the game printed no report".into()));
    assert_eq!(broken["ok"], false);
    assert_eq!(broken["tests"], serde_json::json!([]));
}

#[test]
fn a_project_without_tests_says_so_and_a_missing_game_is_a_note() {
    let project = Project::new("dev-no-tests", Some(b"guest"), None, None);
    let manifest = read_manifest(&project.root).unwrap();
    // No Cargo.toml, no test module: a note, not a failure.
    let run = engine_test::dev_tests(&project.root, &manifest).unwrap();
    assert!(run.tests.is_empty() && run.errors.is_empty());
    assert!(run.notes[0].contains("no tests"), "{:?}", run.notes);
    assert_eq!(engine_test::build_module(&project.root, &manifest, None).unwrap(), None);
    // The module is at its conventional path: it is found without a build.
    let module = engine_test::module_path(&project.root, &manifest);
    assert_eq!(module.file_name().unwrap(), "dev-no-tests-tests.wasm");
    assert_eq!(module.parent().unwrap().file_name().unwrap(), "wasm");
    fs::create_dir_all(module.parent().unwrap()).unwrap();
    fs::write(&module, b"component").unwrap();
    assert_eq!(engine_test::build_module(&project.root, &manifest, None).unwrap(), Some(module.clone()));
    // A module that is given must exist.
    assert!(engine_test::build_module(&project.root, &manifest, Some(&project.root.join("nope.wasm"))).is_err());
    assert_eq!(engine_test::build_module(&project.root, &manifest, Some(&module)).unwrap(), Some(module));
}

/// The stand-in for the game in `the_game_is_started_with_the_runner_flags`: this test binary started with
/// `--exact tests::fake_engine --nocapture --` (the arguments the game would get follow, as test filters).
/// It prints a report that echoes its arguments. Run normally (no `--yoake-test`) it does nothing.
#[test]
fn fake_engine() {
    let arguments: Vec<String> = env::args().collect();
    let Some(at) = arguments.iter().position(|argument| argument == "--yoake-test") else { return };
    let rest = arguments[at..].join(" ");
    println!("a progress line");
    println!("{}", json!({"ok": true, "tests": [{"name": "echo", "ok": true, "ms": 1, "error": rest}], "errors": []}));
}

#[test]
fn the_game_is_started_with_the_runner_flags() {
    let settings = TestSettings {
        filter: Some("place".into()), timeout_ms: Some("900".into()), clients: Some("2".into()), keep: true, extra: strings(&["--resources", "res"]), ..TestSettings::default()
    };
    let lead = strings(&["--exact", "tests::fake_engine", "--nocapture", "--"]);
    let report = engine_test::run_game(Path::new("project"), &env::current_exe().unwrap(), &lead, Path::new("t.wasm"), &settings).unwrap();
    assert_eq!(
        report["tests"][0]["error"],
        "--yoake-test project --yoake-test-module t.wasm --yoake-test-filter place --yoake-test-timeout-ms 900 --yoake-test-clients 2 --yoake-test-keep --resources res"
    );
    let missing = engine_test::run_game(Path::new("project"), Path::new("no/such/game"), &[], Path::new("t.wasm"), &settings).unwrap_err();
    assert!(missing.contains("could not start"), "{missing}");
}
