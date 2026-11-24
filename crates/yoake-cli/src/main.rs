mod engine_test;

#[cfg(test)]
#[path = "../../../tests/support/temp_dir.rs"]
mod test_support;
mod scaffold;
#[cfg(test)]
mod tests;

use horizon_mods::yak::{Manifest, MAX_WASM_SIZE, check_plain_path, pack_directory};
use serde_json::json;
use std::collections::BTreeMap;
use std::env;
use std::ffi::OsStr;
use std::fs::{self, File, OpenOptions};
use std::io::Read;
use std::path::{Path, PathBuf};
use std::process::{self, Command};
use std::time::{SystemTime, UNIX_EPOCH};
use yoake_wasm::{ClientFullMod, ClientHost, ClientMod, Sandbox, ServerFullMod, ServerHost};

type Result<T> = std::result::Result<T, String>;
const FULL_UNWIRED: &[&str] = &["input_key", "item_model_select"];
const USAGE: &str = "usage: yoake new <minigame|hud-only|custom-mob|utility|script> <name> | yoake <build|pack|check> | yoake dev [--test] | yoake test [--game EXE] [--filter TEXT] [--timeout-ms N] [--clients 1|2] [--module WASM] [--keep] [-- GAME-ARGS...]";
const CHECK_HELP: &str = "yoake check validates existing Wasm modules without rebuilding them; if src/**, Cargo.toml or yak.toml is newer, run yoake build first";

fn main() {
    let arguments: Vec<_> = env::args_os().skip(1).collect();
    let is_dev = arguments.first().is_some_and(|argument| argument.as_os_str() == OsStr::new("dev"));
    let result = arguments.into_iter()
        .map(|argument| argument.into_string().map_err(|_| "arguments must be valid UTF-8".to_owned()))
        .collect::<Result<Vec<_>>>()
        .and_then(|arguments| run(&arguments))
        .unwrap_or_else(|error| {
            if is_dev { dev_error(&error) } else { json!({"ok": false, "errors": [explain_error(&error)]}) }
        });
    if let Some(errors) = result.get("errors").and_then(serde_json::Value::as_array) {
        for error in errors.iter().filter_map(serde_json::Value::as_str) {
            eprintln!("yoake: {error}");
        }
    }
    println!("{result}");
    if result.get("ok").and_then(serde_json::Value::as_bool) != Some(true) {
        process::exit(1);
    }
}

fn explain_error(error: &str) -> String {
    if error.contains("is a link or reparse point") {
        format!("{error}; move the project to a normal folder without links or reparse points")
    } else {
        error.to_owned()
    }
}

fn reserved_name(name: &str) -> bool {
    let stem = name.split('.').next().unwrap_or(name).to_ascii_uppercase();
    matches!(stem.as_str(), "CON" | "PRN" | "AUX" | "NUL")
        || (stem.len() == 4 && (stem.starts_with("COM") || stem.starts_with("LPT"))
            && matches!(stem.as_bytes()[3], b'1'..=b'9'))
}

fn read_manifest(root: &Path) -> Result<Manifest> {
    let path = root.join("yak.toml");
    check_plain_path(&path)?;
    let manifest = Manifest::read(&path)?;
    for (field, value) in [("id", &manifest.id), ("name", &manifest.name)] {
        if reserved_name(value) {
            return Err(format!("{field} {value:?} is a reserved Windows name"));
        }
    }
    Ok(manifest)
}

fn run(arguments: &[String]) -> Result<serde_json::Value> {
    match arguments {
        [flag] if flag == "--help" || flag == "-h" => Ok(json!({"ok": true, "errors": [], "help": format!("{USAGE}\n{CHECK_HELP}")})),
        [command, flag] if command == "check" && (flag == "--help" || flag == "-h") => Ok(json!({"ok": true, "errors": [], "help": CHECK_HELP})),
        [command, template, name] if command == "new" => {
            let path = scaffold::create(template, name)?;
            Ok(json!({"ok": true, "errors": [], "path": path.display().to_string()}))
        }
        [command, rest @ ..] if command == "dev" => Ok(dev(rest)),
        [command, rest @ ..] if command == "test" => engine_test::test_command(rest),
        [command] if command == "build" || command == "pack" || command == "check" => {
            let root = env::current_dir().map_err(|error| error.to_string())?;
            process_project(&root, command, build)
        }
        _ => Err(USAGE.into()),
    }
}

fn process_project(root: &Path, command: &str, build_fn: impl FnOnce(&Path, &Manifest) -> Result<()>) -> Result<serde_json::Value> {
    let manifest = read_manifest(root)?;
    if command == "build" || (command == "pack" && manifest.wasm.is_some()) {
        build_fn(root, &manifest)?;
    }
    let mut summary = check(root, &manifest)?;
    if command == "pack" {
        summary["output"] = json!(pack(root, &manifest)?.display().to_string());
    }
    Ok(summary)
}

fn build(root: &Path, manifest: &Manifest) -> Result<()> {
    let wasm = manifest.wasm.as_ref().ok_or("yak.toml must declare [wasm]")?;
    let modules = wasm.modules();
    let cargo_file = fs::read_to_string(root.join("Cargo.toml")).map_err(|error| error.to_string())?;
    let cargo: toml::Table = cargo_file.parse().map_err(|error| format!("Cargo.toml: {error}"))?;
    let package_name = cargo.get("package").and_then(|package| package.get("name")).and_then(toml::Value::as_str).ok_or("Cargo.toml needs package.name")?;
    let library_name = cargo.get("lib").and_then(|library| library.get("name")).and_then(toml::Value::as_str).unwrap_or(package_name).replace('-', "_");
    let target = target_directory(root, env::var_os("CARGO_TARGET_DIR").as_deref());
    // The worker (`[wasm] worker`, world worker-mod) is built like a module, from its own feature.
    let worker = wasm.worker.as_deref();
    let multi_module = modules.len() + usize::from(worker.is_some()) > 1;
    if multi_module {
        let features = cargo.get("features").and_then(toml::Value::as_table);
        let sides = modules.iter().map(|(side, _)| *side).chain(worker.map(|_| "worker"));
        if !sides.clone().all(|side| features.is_some_and(|features| features.contains_key(side))) {
            return Err("yoake build, dev, and pack require Cargo features named `client` and `server` for a multi-module package; independently built modules can still be validated with yoake check".into());
        }
    }
    let artifact = target.join("wasm32-wasip2").join("release").join(format!("{library_name}.wasm"));
    for (side, path) in modules {
        let destination = root.join(path);
        check_plain_path(&destination)?;
        cargo_wasm(root, &target, multi_module.then_some(side), side, &artifact, &destination)?;
    }
    if let Some(path) = worker {
        let destination = root.join(path);
        check_plain_path(&destination)?;
        cargo_wasm(root, &target, multi_module.then_some("worker"), "worker", &artifact, &destination)?;
    }
    Ok(())
}

/// Builds the crate for `wasm32-wasip2` (with the Cargo feature `features`, if any) and copies the artifact to
/// `destination`, dating it from the start of the build so `yoake check` sees it as fresh.
fn cargo_wasm(root: &Path, target: &Path, features: Option<&str>, label: &str, artifact: &Path, destination: &Path) -> Result<()> {
    let mut command = Command::new("cargo");
    command.current_dir(root).args(["build", "--release", "--target", "wasm32-wasip2", "--target-dir"]).arg(target);
    if let Some(features) = features {
        command.args(["--features", features]);
    }
    let build_started = SystemTime::now();
    let output = command.output().map_err(|error| format!("could not start cargo: {error}"))?;
    eprint!("{}", String::from_utf8_lossy(&output.stdout));
    if !output.status.success() {
        return Err(format!("cargo build for {label} failed: {}", String::from_utf8_lossy(&output.stderr)));
    }
    eprint!("{}", String::from_utf8_lossy(&output.stderr));
    let parent = destination.parent().ok_or("module has no parent directory")?;
    fs::create_dir_all(parent).map_err(|error| error.to_string())?;
    fs::copy(artifact, destination).map_err(|error| format!("{}: {error}", artifact.display()))?;
    File::options().write(true).open(destination)
        .and_then(|file| file.set_modified(build_started))
        .map_err(|error| format!("{}: {error}", destination.display()))?;
    Ok(())
}

fn target_directory(root: &Path, configured: Option<&OsStr>) -> PathBuf {
    match configured.filter(|value| !value.is_empty()) {
        Some(value) => {
            let path = PathBuf::from(value);
            if path.is_absolute() { path } else { root.join(path) }
        }
        None => root.join("target"),
    }
}

struct StubClient;
impl ClientHost for StubClient {}

struct StubServer;
impl ServerHost for StubServer {}

fn ensure_module_is_fresh(root: &Path, module_path: &Path, module_metadata: &fs::Metadata) -> Result<()> {
    let module_modified = module_metadata.modified().map_err(|error| format!("{}: {error}", module_path.display()))?;
    let source_root = root.join("src");
    let mut pending = vec![root.join("yak.toml"), root.join("Cargo.toml"), source_root.clone()];
    while let Some(path) = pending.pop() {
        let metadata = match fs::symlink_metadata(&path) {
            Ok(metadata) => metadata,
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => continue,
            Err(error) => return Err(format!("{}: {error}", path.display())),
        };
        check_plain_path(&path)?;
        let modified = metadata.modified().map_err(|error| format!("{}: {error}", path.display()))?;
        if modified > module_modified {
            return Err(format!("{} is stale because {} is newer; run yoake build", module_path.display(), path.display()));
        }
        if metadata.is_dir() && path.starts_with(&source_root) {
            for entry in fs::read_dir(&path).map_err(|error| format!("{}: {error}", path.display()))? {
                pending.push(entry.map_err(|error| format!("{}: {error}", path.display()))?.path());
            }
        } else if !metadata.is_file() {
            return Err(format!("{} is not a regular source file", path.display()));
        }
    }
    Ok(())
}

fn check(root: &Path, manifest: &Manifest) -> Result<serde_json::Value> {
    let mut sandbox = Sandbox::new();
    let mut checked = Vec::new();
    for (side, path) in manifest.wasm.as_ref().map_or_else(Vec::new, |wasm| wasm.declared_files()) {
        let module_path = root.join(path);
        check_plain_path(&module_path)?;
        let file = File::open(&module_path).map_err(|error| format!("{}: {error}", module_path.display()))?;
        let metadata = file.metadata().map_err(|error| format!("{}: {error}", module_path.display()))?;
        if !metadata.is_file() || metadata.len() > MAX_WASM_SIZE {
            return Err(format!("{} is not a regular Wasm file within the {MAX_WASM_SIZE}-byte size limit", module_path.display()));
        }
        ensure_module_is_fresh(root, &module_path, &metadata)?;
        if side == "worker" {
            // The worker runs on its own thread with WASI p2 (world worker-mod). Only the file is
            // checked here: yoake-wasm has no host to instantiate it in this command.
            checked.push(json!({"side": side, "path": path, "world": "worker-mod"}));
            continue;
        }
        let mut bytes = Vec::new();
        file.take(MAX_WASM_SIZE + 1).read_to_end(&mut bytes)
            .map_err(|error| format!("{}: {error}", module_path.display()))?;
        if bytes.len() as u64 > MAX_WASM_SIZE {
            return Err(format!("{} exceeds the {MAX_WASM_SIZE}-byte size limit", module_path.display()));
        }
        let world = if side == "server" { "server-full" } else {
            manifest.wasm.as_ref().and_then(|wasm| wasm.world.as_deref()).unwrap_or("client-mod")
        };
        let result = match world {
            "client-mod" => ClientMod::instantiate(&mut sandbox, &manifest.id, &bytes, Box::new(StubClient)).map(|_| ()),
            "client-full" => ClientFullMod::instantiate(&mut sandbox, &manifest.id, &bytes, Box::new(StubClient), FULL_UNWIRED).map(|_| ()),
            "server-full" => ServerFullMod::instantiate(&mut sandbox, &manifest.id, &bytes, Box::new(StubServer), FULL_UNWIRED).map(|_| ()),
            _ => return Err(format!("{}: unsupported {side} world {world}", module_path.display())),
        };
        result.map_err(|error| format!("{} ({side}, {world}): {error}", module_path.display()))?;
        checked.push(json!({"side": side, "path": path, "world": world}));
    }
    Ok(json!({"ok": true, "errors": [], "id": manifest.id, "version": manifest.version.to_string(), "modules": checked, "wasm_needs": needs(root)?}))
}

fn pack(root: &Path, manifest: &Manifest) -> Result<PathBuf> {
    let directory = root.join("dist");
    check_plain_path(&directory)?;
    fs::create_dir_all(&directory).map_err(|error| error.to_string())?;
    let stamp = SystemTime::now().duration_since(UNIX_EPOCH).map_err(|error| error.to_string())?.as_nanos();
    let output = directory.join(format!("{}-{}.yak", manifest.id, manifest.version));
    let temporary = directory.join(format!(".{}-{}-{}-{stamp}.tmp", manifest.id, manifest.version, process::id()));
    check_plain_path(&output)?;
    check_plain_path(&temporary)?;
    pack_directory(root, &temporary, &BTreeMap::new())?;
    if let Err(error) = fs::rename(&temporary, &output) {
        let _ = fs::remove_file(&temporary);
        return Err(format!("could not replace {}: {error}", output.display()));
    }
    Ok(output)
}

fn needs(root: &Path) -> Result<Vec<String>> {
    let path = root.join("WASM-NEEDS.md");
    check_plain_path(&path)?;
    match fs::read_to_string(path) {
        Ok(content) => Ok(content.lines().map(str::trim).filter(|line| !line.is_empty()).map(str::to_owned).collect()),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(Vec::new()),
        Err(error) => Err(format!("WASM-NEEDS.md: {error}")),
    }
}

fn install_for_reload(root: &Path, manifest: &Manifest, directory: &Path) -> Result<()> {
    let wasm = manifest.wasm.as_ref().ok_or("yak.toml must declare [wasm]")?;
    if wasm.world.as_deref().is_some_and(|world| world != "client-mod") {
        return Err("the existing raw Wasm hot reload only supports client-mod".into());
    }
    let source = wasm.modules().into_iter().find(|(side, _)| *side == "client")
        .map(|(_, path)| root.join(path)).ok_or("hot reload requires a client module")?;
    check_plain_path(directory)?;
    fs::create_dir_all(directory).map_err(|error| format!("{}: {error}", directory.display()))?;
    let target = directory.join(format!("{}.wasm", manifest.id));
    check_plain_path(&source)?;
    check_plain_path(&target)?;
    let stamp = SystemTime::now().duration_since(UNIX_EPOCH).map_err(|error| error.to_string())?.as_nanos();
    let temporary = directory.join(format!(".{}-{stamp}.tmp", manifest.id));
    check_plain_path(&temporary)?;
    let mut input = File::open(&source).map_err(|error| format!("{}: {error}", source.display()))?;
    let mut output = OpenOptions::new().write(true).create_new(true).open(&temporary)
        .map_err(|error| format!("{}: {error}", temporary.display()))?;
    let result = std::io::copy(&mut input, &mut output)
        .and_then(|_| output.sync_all())
        .and_then(|_| { drop(output); fs::rename(&temporary, &target) });
    if let Err(error) = result {
        let _ = fs::remove_file(&temporary);
        return Err(format!("could not replace {}: {error}", target.display()));
    }
    Ok(())
}

fn dev(arguments: &[String]) -> serde_json::Value {
    // The engine's tests start a game per test: they run only when asked (`--test`).
    let with_tests = arguments == ["--test"];
    if !arguments.is_empty() && !with_tests {
        return dev_error("usage: yoake dev [--test]");
    }
    let root = match env::current_dir() {
        Ok(root) => root,
        Err(error) => return dev_error(&error.to_string()),
    };
    let directory = env::var_os("YOAKE_WASM_DEV_DIR")
        .filter(|value| !value.is_empty())
        .map(PathBuf::from);
    dev_project(&root, directory.as_deref(), build,
        |root, manifest| check(root, manifest).map(|_| ()), install_for_reload,
        move |root, manifest| if with_tests { engine_test::dev_tests(root, manifest) } else { Ok(engine_test::TestRun::default()) })
}

fn dev_error(error: &str) -> serde_json::Value {
    json!({"ok": false, "built": false, "checked": false, "installed": false,
        "errors": [explain_error(error)], "notes": [], "tests": [], "wasm_needs": []})
}

fn dev_project(
    root: &Path,
    directory: Option<&Path>,
    build_project: impl FnOnce(&Path, &Manifest) -> Result<()>,
    check_project: impl FnOnce(&Path, &Manifest) -> Result<()>,
    install: impl FnOnce(&Path, &Manifest, &Path) -> Result<()>,
    run_tests: impl FnOnce(&Path, &Manifest) -> Result<engine_test::TestRun>,
) -> serde_json::Value {
    let mut built = false;
    let mut tests = Vec::new();
    let mut checked = false;
    let mut installed = false;
    let mut errors = Vec::new();
    let mut notes = Vec::new();
    let wasm_needs = match needs(root) {
        Ok(needs) => needs,
        Err(error) => return dev_error(&error),
    };
    let result = (|| -> Result<()> {
        let manifest = read_manifest(root)?;
        build_project(root, &manifest)?;
        built = true;
        check_project(root, &manifest)?;
        checked = true;
        let wasm = manifest.wasm.as_ref().ok_or("dev requires a [wasm] section")?;
        if wasm.world.as_deref() == Some("client-full") || wasm.client.is_none() {
            notes.push("raw Wasm hot reload only supports client-mod; not installed".to_owned());
        } else if let Some(directory) = directory {
            install(root, &manifest, directory)?;
            installed = true;
        } else {
            notes.push("YOAKE_WASM_DEV_DIR is unset; not installed".to_owned());
        }
        // The engine's test runner (`yoake test`): its failures are errors, so `ok` is false.
        let run = run_tests(root, &manifest)?;
        tests = run.tests;
        notes.extend(run.notes);
        errors.extend(run.errors);
        Ok(())
    })();
    if let Err(error) = result {
        errors.push(explain_error(&error));
    }
    json!({"ok": errors.is_empty(), "built": built, "checked": checked,
        "installed": installed, "errors": errors, "notes": notes, "tests": tests,
        "wasm_needs": wasm_needs})
}
