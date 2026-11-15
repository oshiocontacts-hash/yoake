mod scaffold;
#[cfg(test)]
mod tests;

use horizon_mods::yak::{Manifest, MAX_WASM_SIZE, check_plain_path, pack_directory};
use serde_json::json;
use std::collections::BTreeMap;
use std::env;
use std::fs::{self, File, OpenOptions};
use std::io::Read;
use std::path::{Path, PathBuf};
use std::process::{self, Command};
use std::time::{SystemTime, UNIX_EPOCH};
use yoake_wasm::{ClientFullMod, ClientHost, ClientMod, Sandbox, ServerFullMod, ServerHost};

type Result<T> = std::result::Result<T, String>;

fn main() {
    let arguments: Vec<String> = env::args().skip(1).collect();
    if arguments.first().is_some_and(|argument| argument == "dev") {
        let (errors, needs) = dev(&arguments[1..]);
        println!("{}", json!({"ok": errors.is_empty(), "errors": errors, "tests": [], "wasm_needs": needs}));
        if !errors.is_empty() {
            process::exit(1);
        }
        return;
    }
    if let Err(error) = run(&arguments) {
        eprintln!("yoake: {error}");
        process::exit(1);
    }
}

fn run(arguments: &[String]) -> Result<()> {
    match arguments {
        [command, template, name] if command == "new" => scaffold::create(template, name),
        [command] if command == "build" || command == "pack" || command == "check" => {
            let root = env::current_dir().map_err(|error| error.to_string())?;
            let manifest = Manifest::read(&root.join("yak.toml"))?;
            if command == "build" {
                build(&root, &manifest)?;
            }
            let summary = check(&root, &manifest)?;
            if command == "pack" {
                println!("{}", pack(&root, &manifest)?.display());
            } else {
                println!("{summary}");
            }
            Ok(())
        }
        _ => Err("usage: yoake new <minigame|hud-only|custom-mob|utility|script> <name> | yoake <build|pack|check|dev>".into()),
    }
}

fn build(root: &Path, manifest: &Manifest) -> Result<()> {
    let wasm = manifest.wasm.as_ref().ok_or("yak.toml must declare [wasm]")?;
    let modules = wasm.modules();
    let cargo_file = fs::read_to_string(root.join("Cargo.toml")).map_err(|error| error.to_string())?;
    let cargo: toml::Table = cargo_file.parse().map_err(|error| format!("Cargo.toml: {error}"))?;
    let package_name = cargo.get("package").and_then(|package| package.get("name")).and_then(toml::Value::as_str).ok_or("Cargo.toml needs package.name")?;
    let library_name = cargo.get("lib").and_then(|library| library.get("name")).and_then(toml::Value::as_str).unwrap_or(package_name).replace('-', "_");
    let target = root.join("target");
    let multi_module = modules.len() > 1;
    if multi_module {
        let features = cargo.get("features").and_then(toml::Value::as_table);
        if !modules.iter().all(|(side, _)| features.is_some_and(|features| features.contains_key(*side))) {
            return Err("building client and server modules requires Cargo features named `client` and `server`; otherwise build them separately before yoake check or yoake pack".into());
        }
    }
    let artifact = target.join("wasm32-wasip2").join("release").join(format!("{library_name}.wasm"));
    for (side, path) in modules {
        let destination = root.join(path);
        check_plain_path(&destination)?;
        let mut command = Command::new("cargo");
        command.current_dir(root).args(["build", "--release", "--target", "wasm32-wasip2", "--target-dir"]).arg(&target);
        if multi_module {
            command.args(["--features", side]);
        }
        let output = command.output().map_err(|error| format!("could not start cargo: {error}"))?;
        if !output.status.success() {
            return Err(format!("cargo build for {side} failed: {}", String::from_utf8_lossy(&output.stderr)));
        }
        let parent = destination.parent().ok_or("module has no parent directory")?;
        fs::create_dir_all(parent).map_err(|error| error.to_string())?;
        fs::copy(&artifact, &destination).map_err(|error| format!("{}: {error}", artifact.display()))?;
    }
    Ok(())
}

struct StubClient;
impl ClientHost for StubClient {}

struct StubServer;
impl ServerHost for StubServer {}

fn check(root: &Path, manifest: &Manifest) -> Result<serde_json::Value> {
    let wasm = manifest.wasm.as_ref().ok_or("yak.toml must declare [wasm]")?;
    let mut sandbox = Sandbox::new();
    let mut checked = Vec::new();
    for (side, path) in wasm.modules() {
        let module_path = root.join(path);
        check_plain_path(&module_path)?;
        let file = File::open(&module_path).map_err(|error| format!("{}: {error}", module_path.display()))?;
        let metadata = file.metadata().map_err(|error| format!("{}: {error}", module_path.display()))?;
        if !metadata.is_file() || metadata.len() > MAX_WASM_SIZE {
            return Err(format!("{} is not a regular Wasm file within the {MAX_WASM_SIZE}-byte size limit", module_path.display()));
        }
        let mut bytes = Vec::new();
        file.take(MAX_WASM_SIZE + 1).read_to_end(&mut bytes)
            .map_err(|error| format!("{}: {error}", module_path.display()))?;
        if bytes.len() as u64 > MAX_WASM_SIZE {
            return Err(format!("{} exceeds the {MAX_WASM_SIZE}-byte size limit", module_path.display()));
        }
        let world = if side == "server" { "server-full" } else { wasm.world.as_deref().unwrap_or("client-mod") };
        let result = match world {
            "client-mod" => ClientMod::validate_component(&mut sandbox, &manifest.id, &bytes, Box::new(StubClient)),
            "client-full" => ClientFullMod::validate_component(&mut sandbox, &manifest.id, &bytes, Box::new(StubClient)),
            "server-full" => ServerFullMod::validate_component(&mut sandbox, &manifest.id, &bytes, Box::new(StubServer)),
            _ => return Err(format!("{}: unsupported {side} world {world}", module_path.display())),
        };
        result.map_err(|error| format!("{} ({side}, {world}): {error}", module_path.display()))?;
        checked.push(json!({"side": side, "path": path, "world": world}));
    }
    Ok(json!({"ok": true, "id": manifest.id, "version": manifest.version.to_string(), "modules": checked, "wasm_needs": needs(root)?}))
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

fn dev(arguments: &[String]) -> (Vec<String>, Vec<String>) {
    let root = match env::current_dir() {
        Ok(root) => root,
        Err(error) => return (vec![error.to_string()], Vec::new()),
    };
    let needs = match needs(&root) {
        Ok(needs) => needs,
        Err(error) => return (vec![error], Vec::new()),
    };
    let mut errors = Vec::new();
    if !arguments.is_empty() {
        errors.push("usage: yoake dev".into());
    } else {
        let result = Manifest::read(&root.join("yak.toml"))
            .and_then(|manifest| {
                build(&root, &manifest)?;
                check(&root, &manifest)?;
                let directory = env::var_os("YOAKE_WASM_DEV_DIR").filter(|value| !value.is_empty())
                    .map(PathBuf::from).ok_or("set YOAKE_WASM_DEV_DIR to the engine's dev directory")?;
                install_for_reload(&root, &manifest, &directory)
            });
        if let Err(error) = result {
            errors.push(error);
        }
    }
    (errors, needs)
}
