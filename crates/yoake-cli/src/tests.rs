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
