mod scaffold;

use horizon_mods::yak::{Manifest, MAX_WASM_SIZE, check_plain_path, pack_directory};
use serde_json::json;
use std::collections::BTreeMap;
use std::env;
use std::fs::{self, File, OpenOptions};
use std::path::{Path, PathBuf};
use std::process::{self, Command};
use std::time::{SystemTime, UNIX_EPOCH};

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
            if command == "check" {
                for need in needs(&root)? {
                    println!("{need}");
                }
            }
            preflight(&root, &manifest)?;
            if command == "check" {
                return Err("full WIT world and ABI verification is unavailable; name checks are only a preflight".into());
            } else {
                eprintln!("WIT name checks are only a preflight; the engine validates the full component ABI");
                println!("{}", pack(&root, &manifest)?.display());
            }
            Ok(())
        }
        _ => Err("usage: yoake new <minigame|hud-only|custom-mob|utility|script> <name> | yoake <build|pack|check|dev>".into()),
    }
}

fn build(root: &Path, manifest: &Manifest) -> Result<()> {
    let wasm = manifest.wasm.as_ref().ok_or("yak.toml must declare [wasm]")?;
    let modules = wasm.modules();
    if modules.len() != 1 {
        return Err("build requires exactly one declared Wasm module; build multi-module projects separately and use yoake pack".into());
    }
    let cargo_file = fs::read_to_string(root.join("Cargo.toml")).map_err(|error| error.to_string())?;
    let cargo: toml::Table = cargo_file.parse().map_err(|error| format!("Cargo.toml: {error}"))?;
    let package_name = cargo.get("package").and_then(|package| package.get("name")).and_then(toml::Value::as_str).ok_or("Cargo.toml needs package.name")?;
    let library_name = cargo.get("lib").and_then(|library| library.get("name")).and_then(toml::Value::as_str).unwrap_or(package_name).replace('-', "_");
    let target = root.join("target");
    let output = Command::new("cargo").current_dir(root).args(["build", "--release", "--target", "wasm32-wasip2", "--target-dir"]).arg(&target).output().map_err(|error| format!("could not start cargo: {error}"))?;
    if !output.status.success() {
        return Err(format!("cargo build failed: {}", String::from_utf8_lossy(&output.stderr)));
    }
    let artifact = target.join("wasm32-wasip2").join("release").join(format!("{library_name}.wasm"));
    let destination = root.join(modules[0].1);
    check_plain_path(&destination)?;
    let parent = destination.parent().ok_or("module has no parent directory")?;
    fs::create_dir_all(parent).map_err(|error| error.to_string())?;
    fs::copy(&artifact, &destination).map_err(|error| format!("{}: {error}", artifact.display()))?;
    Ok(())
}

fn preflight(root: &Path, manifest: &Manifest) -> Result<()> {
    let wasm = manifest.wasm.as_ref().ok_or("yak.toml must declare [wasm]")?;
    for (side, path) in wasm.modules() {
        let path = root.join(path);
        check_plain_path(&path)?;
        let metadata = fs::metadata(&path).map_err(|error| format!("{}: {error}", path.display()))?;
        if !metadata.is_file() || metadata.len() > MAX_WASM_SIZE {
            return Err(format!("{} is not a regular Wasm file within the size limit", path.display()));
        }
        let output = Command::new("wasm-tools").args(["component", "wit"]).arg(&path).output().map_err(|error| format!("could not start wasm-tools: {error}"))?;
        if !output.status.success() {
            return Err(format!("{} is not a readable component: {}", path.display(), String::from_utf8_lossy(&output.stderr)));
        }
        let wit = String::from_utf8(output.stdout).map_err(|error| error.to_string())?;
        let hook = if side == "server" { "server-hooks" } else if wasm.world.as_deref() == Some("client-full") { "client-hooks" } else { "hooks" };
        if !wit.contains(hook) || !wit.contains("init") {
            return Err(format!("{} does not expose expected {hook} and init names", path.display()));
        }
    }
    Ok(())
}

fn pack(root: &Path, manifest: &Manifest) -> Result<PathBuf> {
    let directory = root.join("dist");
    check_plain_path(&directory)?;
    fs::create_dir_all(&directory).map_err(|error| error.to_string())?;
    let stamp = SystemTime::now().duration_since(UNIX_EPOCH).map_err(|error| error.to_string())?.as_nanos();
    let output = directory.join(format!("{}-{}-{stamp}.yak", manifest.id, manifest.version));
    pack_directory(root, &output, &BTreeMap::new())?;
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

fn install_for_reload(root: &Path, manifest: &Manifest) -> Result<()> {
    let wasm = manifest.wasm.as_ref().ok_or("yak.toml must declare [wasm]")?;
    if wasm.world.as_deref().is_some_and(|world| world != "client-mod") {
        return Err("the existing raw Wasm hot reload only supports client-mod".into());
    }
    let source = wasm.modules().into_iter().find(|(side, _)| *side == "client")
        .map(|(_, path)| root.join(path)).ok_or("hot reload requires a client module")?;
    let directory = env::var_os("YOAKE_WASM_DEV_DIR").filter(|value| !value.is_empty())
        .map(PathBuf::from).ok_or("set YOAKE_WASM_DEV_DIR to the running engine's dev directory")?;
    let target = directory.join(format!("{}.wasm", manifest.id));
    check_plain_path(&source)?;
    check_plain_path(&target)?;
    if !target.is_file() {
        return Err(format!("{} must exist when the engine starts; the watcher does not discover new files", target.display()));
    }
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
                preflight(&root, &manifest)?;
                pack(&root, &manifest)?;
                install_for_reload(&root, &manifest)
            });
        if let Err(error) = result {
            errors.push(error);
        }
        errors.push("full WIT world and ABI verification is unavailable; engine acceptance is unconfirmed".into());
        errors.push("offscreen Wasm test-world and two-client protocols are not implemented by the engine; no tests ran".into());
    }
    (errors, needs)
}
