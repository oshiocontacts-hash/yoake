use horizon_client::{mod_list::Installed as ModRow, text::Text, toasts::SystemToast};
use horizon_mod_abi as abi;
use horizon_mods::{Package, yak::{self, Manifest, PublicKey, Target, Version}};
use std::{
    collections::{BTreeMap, HashSet, VecDeque},
    fs::File,
    io::{ErrorKind, Read},
    path::{Path, PathBuf},
    sync::{Mutex, OnceLock},
};

pub(crate) mod shaders;
#[cfg(test)]
mod tests;

const MAX_LIST_SIZE: u64 = 1024 * 1024;
const MAX_MODS: usize = 128;
static RUNTIME: OnceLock<Runtime> = OnceLock::new();

pub(crate) struct Build {
    pub(crate) version: Version,
    pub(crate) engine: String,
    pub(crate) abi: u32,
    pub(crate) target: Option<Target>,
}

impl Build {
    pub(crate) fn current() -> Result<Self, String> {
        Ok(Self {
            version: Version::parse(abi::YOAKE_VERSION).map_err(|error| error.to_string())?,
            engine: format!("{}+build.{:016x}", abi::YOAKE_VERSION, abi::BUILD_FINGERPRINT),
            abi: abi::ABI_VERSION,
            target: Target::host().ok(),
        })
    }

    fn check(&self, manifest: &Manifest) -> Result<(), String> {
        let issues = if manifest.bin.is_some() {
            yak::check_compatibility(manifest, &yak::Environment {
                yoake: &self.version,
                engine: &self.engine,
                abi: self.abi,
                target: self.target.ok_or("unsupported native target")?,
            })
        } else {
            yak::check_data_compatibility(manifest, &self.version)
        };
        if issues.is_empty() { Ok(()) } else { Err(format!("incompatible build: {issues:?}")) }
    }
}

#[derive(Clone)]
pub(crate) struct Installed {
    pub(crate) root: PathBuf,
    pub(crate) manifest: Manifest,
    pub(crate) content: Option<PathBuf>,
    content_wasm: bool,
    signed: bool,
}

#[derive(Default)]
struct Runtime {
    entries: Vec<Installed>,
    notices: Mutex<VecDeque<Notice>>,
    problems: Mutex<BTreeMap<String, String>>,
    startup_log: Vec<String>,
}

struct Notice {
    name: String,
    build: Option<String>,
    reason: String,
}

impl Notice {
    fn title(&self) -> String {
        let name = Text::literal(&self.name);
        if let Some(build) = &self.build {
            Text::translate_with_fallback(
                "yoake.mods.notLoaded",
                "%s は %s で読みませんでした / Not loaded on this build",
                &[name, Text::literal(build)],
            ).plain()
        } else {
            Text::translate_with_fallback(
                "yoake.mods.dllNotLoaded",
                "%s: DLL を読みませんでした / DLL not loaded",
                &[name],
            ).plain()
        }
    }
}

impl Runtime {
    fn reject(&mut self, name: &str, build: &str, reason: &str) {
        self.startup_log.push(format!("YAK {name} skipped: {reason}"));
        self.notices.get_mut().unwrap_or_else(|error| error.into_inner()).push_back(Notice {
            name: name.to_owned(), build: Some(build.to_owned()), reason: reason.to_owned(),
        });
    }

    fn read(game: &Path, build: &Build, keys: &[PublicKey]) -> Self {
        Self::read_as(game, build, keys, dedicated())
    }

    /// `read` for a client (`dedicated` false) or a dedicated server.
    fn read_as(game: &Path, build: &Build, keys: &[PublicKey], dedicated: bool) -> Self {
        let mut runtime = Self::default();
        let file = match File::open(game.join("yoake-mods.txt")) {
            Ok(file) => file,
            Err(error) if error.kind() == ErrorKind::NotFound => return runtime,
            Err(error) => {
                runtime.reject("yoake-mods.txt", &build.version.to_string(), &error.to_string());
                return runtime;
            }
        };
        let mut text = String::new();
        if let Err(error) = file.take(MAX_LIST_SIZE + 1).read_to_string(&mut text) {
            runtime.reject("yoake-mods.txt", &build.version.to_string(), &error.to_string());
            return runtime;
        }
        if text.len() as u64 > MAX_LIST_SIZE {
            runtime.reject("yoake-mods.txt", &build.version.to_string(), "list exceeds 1 MiB");
            return runtime;
        }
        let mut seen = HashSet::new();
        for (line_index, line) in text.lines().enumerate() {
            let line = line.split('#').next().unwrap_or_default().trim();
            if line.is_empty() { continue; }
            let label = format!("yoake-mods.txt:{}", line_index + 1);
            let mut display_name = label.clone();
            let result = (|| -> Result<_, String> {
                if seen.len() >= MAX_MODS { return Err("list exceeds 128 mods".into()); }
                let fields: Vec<_> = line.split_whitespace().collect();
                if fields.len() != 2 || !safe_id(fields[0]) {
                    return Err("expected a safe id and a semantic version".into());
                }
                let version = Version::parse(fields[1]).map_err(|error| error.to_string())?;
                if version.to_string() != fields[1] { return Err("version must be canonical".into()); }
                if !seen.insert(fields[0].to_owned()) { return Err("duplicate YAK id".into()); }
                let root = game.join("yoake-mods").join(fields[0]).join(fields[1]);
                yak::check_installed_tree(&root)?;
                let manifest = Manifest::read(&root.join("yak.toml"))?;
                display_name = manifest.name_ja.as_ref().unwrap_or(&manifest.name).clone();
                if manifest.id != fields[0] || manifest.version != version {
                    return Err("list and manifest identity differ".into());
                }
                build.check(&manifest)?;
                // A dedicated server has no client: a server-only package is its whole business.
                if !manifest.side.client && !(dedicated && manifest.side.server) { return Err("server-only YAK".into()); }
                let content = root.join("content");
                let (content, content_wasm) = if content.is_dir() {
                    let package = Package::read(&content)?;
                    if package.manifest.id != manifest.id
                        || Version::parse(&package.manifest.version).ok().as_ref() != Some(&version)
                    {
                        return Err("content and YAK identity differ".into());
                    }
                    (Some(content), package.wasm.is_some())
                } else { (None, false) };
                let names = installed_files(&root)?;
                let names: Vec<_> = names.iter().map(String::as_str).collect();
                let signed = yak::verify_installed(&root, &names, keys).is_ok();
                Ok(Installed { root, manifest, content, content_wasm, signed })
            })();
            match result {
                Ok(entry) => runtime.entries.push(entry),
                Err(reason) => runtime.reject(&display_name, &build.version.to_string(), &format!("{label}: {reason}")),
            }
        }
        runtime
    }
}

/// Whether this process is a dedicated server (`--dedicated`): no client, no DLL mods.
pub(crate) fn dedicated() -> bool {
    std::env::args().skip(1).any(|argument| argument == "--dedicated")
}

fn safe_id(id: &str) -> bool {
    !id.is_empty() && id.len() <= 64
        && id.bytes().all(|byte| byte.is_ascii_lowercase() || byte.is_ascii_digit() || matches!(byte, b'_' | b'-'))
}

fn installed_files(root: &Path) -> Result<Vec<String>, String> {
    fn visit(root: &Path, path: &Path, names: &mut Vec<String>) -> Result<(), String> {
        for entry in std::fs::read_dir(path).map_err(|error| error.to_string())? {
            let entry = entry.map_err(|error| error.to_string())?;
            if entry.file_type().map_err(|error| error.to_string())?.is_dir() {
                visit(root, &entry.path(), names)?;
            } else {
                let path = entry.path();
                let name = path.strip_prefix(root).map_err(|error| error.to_string())?
                    .to_str().ok_or("non-UTF-8 installed path")?.replace('\\', "/");
                if name != "SIGNATURE" { names.push(name); }
            }
        }
        Ok(())
    }
    let mut names = Vec::new();
    visit(root, root, &mut names)?;
    Ok(names)
}

pub(crate) fn initialize() {
    if RUNTIME.get().is_some() { return; }
    let args: Vec<_> = std::env::args().skip(1).collect();
    if args.iter().any(|argument| matches!(argument.as_str(), "--help" | "-h" | "--launch-api")) {
        return;
    }
    let game = args.windows(2).rev().find(|pair| matches!(pair[0].as_str(), "--game-dir" | "--gameDir"))
        .map(|pair| PathBuf::from(&pair[1])).unwrap_or_else(|| PathBuf::from("."));
    let game = std::path::absolute(&game).unwrap_or(game);
    if std::fs::symlink_metadata(game.join("yoake-mods.txt")).is_err_and(|error| error.kind() == ErrorKind::NotFound) {
        return;
    }
    let runtime = match Build::current() {
        Ok(build) => Runtime::read(&game, &build, &yak::trusted_keys()),
        Err(reason) => {
            let mut runtime = Runtime::default();
            runtime.reject("yoake-mods.txt", abi::YOAKE_VERSION, &reason);
            runtime
        }
    };
    let _ = RUNTIME.set(runtime);
}

pub(crate) fn entries() -> &'static [Installed] {
    RUNTIME.get().map(|runtime| runtime.entries.as_slice()).unwrap_or_default()
}

pub(crate) fn log_startup() {
    if let Some(runtime) = RUNTIME.get() {
        for message in &runtime.startup_log { log::warn!("{message}"); }
        let problems = runtime.problems.lock().unwrap_or_else(|error| error.into_inner());
        for (id, reason) in problems.iter() { log::warn!("YAK {id} native library refused: {reason}"); }
    }
}

pub(crate) fn refuse_binary(entry: &Installed, reason: &str) {
    log::warn!("YAK {} native library refused: {reason}", entry.manifest.id);
    eprintln!("YAK {} native library refused: {reason}", entry.manifest.id);
    if let Some(runtime) = RUNTIME.get() {
        let mut problems = runtime.problems.lock().unwrap_or_else(|error| error.into_inner());
        if problems.insert(entry.manifest.id.clone(), reason.to_owned()).is_none() {
            let name = entry.manifest.name_ja.as_ref().unwrap_or(&entry.manifest.name);
            runtime.notices.lock().unwrap_or_else(|error| error.into_inner()).push_back(Notice {
                name: name.to_owned(), build: None, reason: reason.to_owned(),
            });
        }
    }
}

pub(crate) fn next_notice() -> Option<SystemToast> {
    let notice = RUNTIME.get()?.notices.lock().unwrap_or_else(|error| error.into_inner()).pop_front()?;
    Some(SystemToast::new(notice.title(), Some(notice.reason)))
}

pub(crate) fn binary_preflight(entry: &Installed, build: &Build, keys: &[PublicKey], allow_unsigned: bool) -> Result<(PathBuf, String), String> {
    yak::check_installed_tree(&entry.root)?;
    let manifest = Manifest::read(&entry.root.join("yak.toml"))?;
    if manifest.id != entry.manifest.id || manifest.version != entry.manifest.version
        || manifest.side.required_on_join != entry.manifest.side.required_on_join
    {
        return Err("YAK identity changed after discovery".into());
    }
    build.check(&manifest)?;
    if !manifest.side.client { return Err("server-only YAK".into()); }
    let bin = manifest.bin.as_ref().ok_or("no binary declared")?;
    let target = build.target.ok_or("unsupported native target")?;
    let name = format!("bin/{target}/{}", target.library_filename(&bin.library));
    let digest = yak::hash_installed_file(&entry.root, &name)?;
    if let Err(error) = yak::verify_installed(&entry.root, &[&name], keys) {
        if !allow_unsigned { return Err(error); }
        log::warn!("YAK {}: developer unsigned override: {error}", manifest.id);
    }
    Ok((entry.root.join(name), digest))
}

/// Which half of a package a WebAssembly module belongs to.
#[allow(dead_code)]
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum WasmSide { Client, Server }

/// One accepted WebAssembly module of an enabled .yak. Nothing here runs the module.
#[allow(dead_code)]
#[derive(Clone, Debug)]
pub(crate) struct WasmMod {
    pub(crate) mod_id: String,
    pub(crate) version: Version,
    /// Lowercase hex SHA-256 of `bytes`.
    pub(crate) sha256: String,
    pub(crate) bytes: Vec<u8>,
    pub(crate) side: WasmSide,
    /// The module targets a full-catalog world (`client-full`, or any server module).
    pub(crate) full: bool,
    pub(crate) gpu: bool,
    pub(crate) required_on_join: bool,
    /// The package asked for `service.commands` (server module only).
    pub(crate) commands: bool,
    /// The installed file the bytes were read from (watched for hot reload in developer mode).
    pub(crate) path: PathBuf,
    /// Whether the .yak carries a valid signature from a trusted key.
    pub(crate) signed: bool,
}

pub(crate) fn unsigned_allowed() -> bool {
    std::env::var_os("YOAKE_DLL_MODS_UNSIGNED").as_deref() == Some(std::ffi::OsStr::new("1"))
}

/// Client-side WebAssembly modules of the enabled .yak files; empty when there are none.
pub(crate) fn wasm_client_mods() -> Vec<WasmMod> {
    wasm_mods(WasmSide::Client)
}

/// Modules of the enabled .yak files for one side.
pub(crate) fn wasm_mods(side: WasmSide) -> Vec<WasmMod> {
    let entries = entries();
    if !entries.iter().any(|entry| entry.manifest.wasm.is_some()) { return Vec::new(); }
    wasm_mods_from(entries, side, &yak::trusted_keys(), unsigned_allowed())
}

pub(crate) fn wasm_mods_from(installed: &[Installed], side: WasmSide, keys: &[PublicKey], allow_unsigned: bool) -> Vec<WasmMod> {
    let mut mods = Vec::new();
    for entry in installed {
        let Some(wasm) = &entry.manifest.wasm else { continue };
        let path = match side { WasmSide::Client => &wasm.client, WasmSide::Server => &wasm.server };
        let Some(path) = path else { continue };
        match wasm_preflight(entry, path, side, keys, allow_unsigned) {
            Ok((bytes, sha256, signed)) => mods.push(WasmMod {
                mod_id: entry.manifest.id.clone(),
                version: entry.manifest.version.clone(),
                sha256, bytes, side,
                full: side == WasmSide::Server || wasm.client_full(),
                gpu: side == WasmSide::Client && wasm.has_permission("service.gpu"),
                required_on_join: entry.manifest.side.required_on_join,
                commands: side == WasmSide::Server && wasm.has_permission("service.commands"),
                path: entry.root.join(path),
                signed,
            }),
            Err(reason) => {
                log::warn!("YAK {} wasm module refused: {reason}", entry.manifest.id);
                eprintln!("YAK {} wasm module refused: {reason}", entry.manifest.id);
            }
        }
    }
    mods
}

fn wasm_preflight(entry: &Installed, name: &str, side: WasmSide, keys: &[PublicKey], allow_unsigned: bool) -> Result<(Vec<u8>, String, bool), String> {
    yak::check_installed_tree(&entry.root)?;
    let manifest = Manifest::read(&entry.root.join("yak.toml"))?;
    if manifest.id != entry.manifest.id || manifest.version != entry.manifest.version
        || manifest.side.required_on_join != entry.manifest.side.required_on_join
    {
        return Err("YAK identity changed after discovery".into());
    }
    if manifest.bin.is_some() { return Err("package declares both native and wasm code".into()); }
    let wasm = manifest.wasm.as_ref().ok_or("no wasm declared")?;
    let declared = match side { WasmSide::Client => &wasm.client, WasmSide::Server => &wasm.server };
    if declared.as_deref() != Some(name) { return Err("wasm path changed after discovery".into()); }
    if side == WasmSide::Client && !manifest.side.client { return Err("server-only YAK".into()); }
    if side == WasmSide::Server && !manifest.side.server { return Err("client-only YAK".into()); }
    let path = entry.root.join(name);
    yak::check_plain_path(&path)?;
    let file = File::open(&path).map_err(|error| format!("{name}: {error}"))?;
    let metadata = file.metadata().map_err(|error| error.to_string())?;
    if !metadata.is_file() || metadata.len() > yak::MAX_WASM_SIZE {
        return Err("wasm module is not a regular file of at most 32 MiB".into());
    }
    let mut bytes = Vec::new();
    file.take(yak::MAX_WASM_SIZE + 1).read_to_end(&mut bytes).map_err(|error| error.to_string())?;
    if bytes.len() as u64 > yak::MAX_WASM_SIZE { return Err("wasm module exceeds 32 MiB".into()); }
    let digest: String = <sha2::Sha256 as sha2::Digest>::digest(&bytes).iter().map(|byte| format!("{byte:02x}")).collect();
    let mut signed = true;
    if let Err(error) = yak::verify_installed(&entry.root, &[name], keys) {
        if !allow_unsigned { return Err(error); }
        signed = false;
        log::warn!("YAK {}: developer unsigned override: {error}", manifest.id);
    }
    if yak::hash_installed_file(&entry.root, name)? != digest {
        return Err("wasm module changed while loading".into());
    }
    Ok((bytes, digest, signed))
}

pub(crate) fn content_packages(paths: Vec<PathBuf>, installed: &[Installed]) -> Vec<(PathBuf, Package, bool)> {
    let mut packages = Vec::new();
    for path in paths {
        if let Ok(package) = Package::read(&path) {
            if installed.iter().any(|entry| entry.content.is_some() && entry.manifest.id == package.manifest.id) {
                log::warn!("YAK {} takes precedence over {}", package.manifest.id, path.display());
            } else { packages.push((path, package, false)); }
        }
    }
    for entry in installed {
        if let Some(path) = &entry.content {
            match Package::read(path) {
                Ok(package) if package.manifest.id == entry.manifest.id
                    && Version::parse(&package.manifest.version).ok().as_ref() == Some(&entry.manifest.version) => {
                        packages.push((path.clone(), package, true));
                    }
                Ok(_) => log::warn!("YAK {}: content identity changed", entry.manifest.id),
                Err(error) => log::warn!("YAK {}: content not loaded: {error}", entry.manifest.id),
            }
        }
    }
    packages
}

pub(crate) fn populate_rows(rows: &mut Vec<ModRow>, stopped: &str) {
    if let Some(runtime) = RUNTIME.get() {
        let problems = runtime.problems.lock().unwrap_or_else(|error| error.into_inner());
        populate_rows_from(rows, &runtime.entries, &problems, stopped);
    }
}

fn populate_rows_from(rows: &mut Vec<ModRow>, installed: &[Installed], problems: &BTreeMap<String, String>, stopped: &str) {
    for entry in installed {
        let existing = rows.iter().position(|row| row.id == entry.manifest.id).map(|index| rows.remove(index));
        let problem = problems.get(&entry.manifest.id).cloned()
            .or_else(|| entry.content_wasm.then(|| stopped.to_owned()));
        let signature = if entry.signed {
            Text::translate_with_fallback("yoake.mods.verified", "署名確認済み / Verified signature", &[])
        } else {
            Text::translate_with_fallback("yoake.mods.unverified", "未署名・未検証 / Unsigned or unverified", &[])
        };
        let source = Text::translate_with_fallback("yoake.mods.yakSource", ".yak · %s", &[signature]).plain();
        rows.push(ModRow {
            id: entry.manifest.id.clone(),
            title: format!("{} {}", entry.manifest.name_ja.as_ref().unwrap_or(&entry.manifest.name), entry.manifest.version),
            description: format!("{}\n{source}", entry.manifest.description),
            problem,
            dev: true,
            native: existing.and_then(|row| row.native),
        });
    }
}
