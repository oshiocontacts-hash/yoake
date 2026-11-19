use horizon_client::{mod_list::Installed as ModRow, text::Text, toasts::SystemToast};
use horizon_mod_abi as abi;
use horizon_mods::{
    Package,
    yak::{self, Manifest, PublicKey, Target, Version},
};
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
            engine: format!(
                "{}+build.{:016x}",
                abi::YOAKE_VERSION,
                abi::BUILD_FINGERPRINT
            ),
            abi: abi::ABI_VERSION,
            target: Target::host().ok(),
        })
    }

    fn check(&self, manifest: &Manifest) -> Result<(), String> {
        let issues = if manifest.bin.is_some() {
            yak::check_compatibility(
                manifest,
                &yak::Environment {
                    yoake: &self.version,
                    engine: &self.engine,
                    abi: self.abi,
                    target: self.target.ok_or("unsupported native target")?,
                },
            )
        } else {
            yak::check_data_compatibility(manifest, &self.version)
        };
        if issues.is_empty() {
            Ok(())
        } else {
            Err(format!("incompatible build: {issues:?}"))
        }
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

impl Installed {
    pub(crate) fn has_synced_config(&self) -> bool {
        self.manifest.wasm.as_ref().is_some_and(|wasm| {
            wasm.server.is_some()
                && wasm
                    .permissions
                    .iter()
                    .any(|permission| permission == yoake_wasm::package_services::CONFIG_CAPABILITY)
                && self
                    .manifest
                    .config
                    .values()
                    .any(|field| field.side == yak::ConfigSide::Synced)
        })
    }

    #[cfg(test)]
    pub(crate) fn new_for_test(root: PathBuf, manifest: Manifest) -> Self {
        Self { root, manifest, content: None, content_wasm: false, signed: false }
    }
}

#[derive(Default)]
struct Runtime {
    game_root: PathBuf,
    entries: Vec<Installed>,
    notices: Mutex<VecDeque<Notice>>,
    problems: Mutex<BTreeMap<String, String>>,
    startup_log: Vec<String>,
    synced_config: bool,
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
            )
            .plain()
        } else {
            Text::translate_with_fallback(
                "yoake.mods.dllNotLoaded",
                "%s: DLL を読みませんでした / DLL not loaded",
                &[name],
            )
            .plain()
        }
    }
}

impl Runtime {
    fn reject(&mut self, name: &str, build: &str, reason: &str) {
        self.startup_log
            .push(format!("YAK {name} skipped: {reason}"));
        self.notices
            .get_mut()
            .unwrap_or_else(|error| error.into_inner())
            .push_back(Notice {
                name: name.to_owned(),
                build: Some(build.to_owned()),
                reason: reason.to_owned(),
            });
    }

    fn read(game: &Path, build: &Build, keys: &[PublicKey]) -> Self {
        let mut runtime = Self {
            game_root: game.to_path_buf(),
            ..Self::default()
        };
        let file = match File::open(game.join("yoake-mods.txt")) {
            Ok(file) => file,
            Err(error) if error.kind() == ErrorKind::NotFound => {
                runtime.add_remote(game, build, keys);
                return runtime;
            }
            Err(error) => {
                runtime.reject(
                    "yoake-mods.txt",
                    &build.version.to_string(),
                    &error.to_string(),
                );
                return runtime;
            }
        };
        let mut text = String::new();
        if let Err(error) = file.take(MAX_LIST_SIZE + 1).read_to_string(&mut text) {
            runtime.reject(
                "yoake-mods.txt",
                &build.version.to_string(),
                &error.to_string(),
            );
            return runtime;
        }
        if text.len() as u64 > MAX_LIST_SIZE {
            runtime.reject(
                "yoake-mods.txt",
                &build.version.to_string(),
                "list exceeds 1 MiB",
            );
            return runtime;
        }
        let mut seen = HashSet::new();
        for (line_index, line) in text.lines().enumerate() {
            let line = line.split('#').next().unwrap_or_default().trim();
            if line.is_empty() {
                continue;
            }
            let label = format!("yoake-mods.txt:{}", line_index + 1);
            let mut display_name = label.clone();
            let result = (|| -> Result<_, String> {
                if seen.len() >= MAX_MODS {
                    return Err("list exceeds 128 mods".into());
                }
                let fields: Vec<_> = line.split_whitespace().collect();
                if fields.len() != 2 || !safe_id(fields[0]) {
                    return Err("expected a safe id and a semantic version".into());
                }
                let version = Version::parse(fields[1]).map_err(|error| error.to_string())?;
                if version.to_string() != fields[1] {
                    return Err("version must be canonical".into());
                }
                if !seen.insert(fields[0].to_owned()) {
                    return Err("duplicate YAK id".into());
                }
                let root = game.join("yoake-mods").join(fields[0]).join(fields[1]);
                yak::check_installed_tree(&root)?;
                let manifest = Manifest::read(&root.join("yak.toml"))?;
                display_name = manifest.name_ja.as_ref().unwrap_or(&manifest.name).clone();
                if manifest.id != fields[0] || manifest.version != version {
                    return Err("list and manifest identity differ".into());
                }
                build.check(&manifest)?;
                let content = root.join("content");
                let (content, content_wasm) = if content.is_dir() {
                    let package = Package::read(&content)?;
                    if package.manifest.id != manifest.id
                        || Version::parse(&package.manifest.version).ok().as_ref() != Some(&version)
                    {
                        return Err("content and YAK identity differ".into());
                    }
                    (Some(content), package.wasm.is_some())
                } else {
                    (None, false)
                };
                let names = installed_files(&root)?;
                let names: Vec<_> = names.iter().map(String::as_str).collect();
                let signed = yak::verify_installed(&root, &names, keys).is_ok();
                Ok(Installed {
                    root,
                    manifest,
                    content,
                    content_wasm,
                    signed,
                })
            })();
            match result {
                Ok(entry) => runtime.entries.push(entry),
                Err(reason) => runtime.reject(
                    &display_name,
                    &build.version.to_string(),
                    &format!("{label}: {reason}"),
                ),
            }
        }
        runtime.add_remote(game, build, keys);
        runtime.synced_config = runtime.entries.iter().any(Installed::has_synced_config);
        runtime
    }

    /// MX3JN1: the sandboxed packages fetched from the host this launch joins (`--remote-mods`), after the player's own
    /// (an id the player has already stays theirs).
    fn add_remote(&mut self, game: &Path, build: &Build, keys: &[PublicKey]) {
        let Some(key) = horizon_client::join_mods::remote_key() else {
            return;
        };
        // M1: these packages are read once at start, before any connection exists, so the launch's own `--server` must
        // be the host of the key (the code of a package still needs a trusted signature, and `has_local` counts the
        // folder only for a join of that host)
        if !horizon_client::join_mods::launch_may_use_remote_from(&key, std::env::args().skip(1)) {
            log::warn!("remote packages of {key} not loaded: this launch does not join that host");
            return;
        }
        for entry in remote_installed(game, &key, build, keys) {
            if self.entries.iter().any(|other| other.manifest.id == entry.manifest.id) {
                log::warn!("remote YAK {}: you have a package with this id, yours is used", entry.manifest.id);
            } else {
                self.entries.push(entry);
            }
        }
        self.synced_config = self.entries.iter().any(Installed::has_synced_config);
    }
}

/// Whether this process is a dedicated server (`--dedicated`): no client, no DLL mods.
pub(crate) fn dedicated() -> bool {
    std::env::args()
        .skip(1)
        .any(|argument| argument == "--dedicated")
}

fn safe_id(id: &str) -> bool {
    !id.is_empty()
        && id.len() <= 64
        && id.bytes().all(|byte| {
            byte.is_ascii_lowercase() || byte.is_ascii_digit() || matches!(byte, b'_' | b'-')
        })
}

fn installed_files(root: &Path) -> Result<Vec<String>, String> {
    fn visit(root: &Path, path: &Path, names: &mut Vec<String>) -> Result<(), String> {
        for entry in std::fs::read_dir(path).map_err(|error| error.to_string())? {
            let entry = entry.map_err(|error| error.to_string())?;
            if entry
                .file_type()
                .map_err(|error| error.to_string())?
                .is_dir()
            {
                visit(root, &entry.path(), names)?;
            } else {
                let path = entry.path();
                let name = path
                    .strip_prefix(root)
                    .map_err(|error| error.to_string())?
                    .to_str()
                    .ok_or("non-UTF-8 installed path")?
                    .replace('\\', "/");
                if name != "SIGNATURE" {
                    names.push(name);
                }
            }
        }
        Ok(())
    }
    let mut names = Vec::new();
    visit(root, root, &mut names)?;
    Ok(names)
}

/// The runtime of a start in `game`. R1: the game root is known whenever a game dir is, **also without** `yoake-mods.txt`
/// (a full-access-only install has none): the full-access loader finds `<root>/mods` through it. The sandboxed list is
/// read only when there is a `yoake-mods.txt` or a remote host key.
fn startup_runtime(game: &Path, has_remote_key: bool) -> Runtime {
    if std::fs::symlink_metadata(game.join("yoake-mods.txt")).is_err_and(|error| error.kind() == ErrorKind::NotFound) && !has_remote_key {
        return Runtime { game_root: game.to_path_buf(), ..Runtime::default() };
    }
    match Build::current() {
        Ok(build) => Runtime::read(game, &build, &yak::trusted_keys()),
        Err(reason) => {
            let mut runtime = Runtime::default();
            runtime.reject("yoake-mods.txt", abi::YOAKE_VERSION, &reason);
            runtime
        }
    }
}

pub(crate) fn initialize() {
    // MX3JN1: what the connection code (in the client library) offers joining players and counts as installed
    horizon_client::join_mods::set_providers(
        || entries().iter().map(|entry| (entry.manifest.id.clone(), entry.manifest.version.to_string())).collect(),
        server_runs_package,
    );
    if RUNTIME.get().is_some() {
        return;
    }
    let args: Vec<_> = std::env::args().skip(1).collect();
    if args
        .iter()
        .any(|argument| matches!(argument.as_str(), "--help" | "-h" | "--launch-api"))
    {
        return;
    }
    let game = args
        .windows(2)
        .rev()
        .find(|pair| matches!(pair[0].as_str(), "--game-dir" | "--gameDir"))
        .map(|pair| PathBuf::from(&pair[1]))
        .unwrap_or_else(|| PathBuf::from("."));
    let game = std::path::absolute(&game).unwrap_or(game);
    let _ = RUNTIME.set(startup_runtime(&game, horizon_client::join_mods::remote_key().is_some()));
}

/// Whether the server thread runs the full-access package with this SHA-256 (lowercase hex).
fn server_runs_package(sha256: &str) -> bool {
    #[cfg(feature = "yoake2-slice")]
    {
        crate::yoake2_mods::server_runs_package(sha256)
    }
    #[cfg(not(feature = "yoake2-slice"))]
    {
        let _ = sha256;
        false
    }
}

pub(crate) fn game_root() -> Option<&'static Path> {
    RUNTIME.get().map(|runtime| runtime.game_root.as_path())
}

pub(crate) fn entries() -> &'static [Installed] {
    RUNTIME
        .get()
        .map(|runtime| runtime.entries.as_slice())
        .unwrap_or_default()
}

pub(crate) fn has_synced_config() -> bool {
    RUNTIME.get().is_some_and(|runtime| runtime.synced_config)
}

pub(crate) fn log_startup() {
    if let Some(runtime) = RUNTIME.get() {
        for message in &runtime.startup_log {
            log::warn!("{message}");
        }
        let problems = runtime
            .problems
            .lock()
            .unwrap_or_else(|error| error.into_inner());
        for (id, reason) in problems.iter() {
            log::warn!("YAK {id} native library refused: {reason}");
        }
    }
}

pub(crate) fn refuse_binary(entry: &Installed, reason: &str) {
    log::warn!("YAK {} native library refused: {reason}", entry.manifest.id);
    eprintln!("YAK {} native library refused: {reason}", entry.manifest.id);
    if let Some(runtime) = RUNTIME.get() {
        let mut problems = runtime
            .problems
            .lock()
            .unwrap_or_else(|error| error.into_inner());
        if problems
            .insert(entry.manifest.id.clone(), reason.to_owned())
            .is_none()
        {
            let name = entry
                .manifest
                .name_ja
                .as_ref()
                .unwrap_or(&entry.manifest.name);
            runtime
                .notices
                .lock()
                .unwrap_or_else(|error| error.into_inner())
                .push_back(Notice {
                    name: name.to_owned(),
                    build: None,
                    reason: reason.to_owned(),
                });
        }
    }
}

pub(crate) fn next_notice() -> Option<SystemToast> {
    let notice = RUNTIME
        .get()?
        .notices
        .lock()
        .unwrap_or_else(|error| error.into_inner())
        .pop_front()?;
    Some(SystemToast::new(notice.title(), Some(notice.reason)))
}

pub(crate) fn binary_preflight(
    entry: &Installed,
    build: &Build,
    keys: &[PublicKey],
    allow_unsigned: bool,
) -> Result<(PathBuf, String), String> {
    yak::check_installed_tree(&entry.root)?;
    let manifest = Manifest::read(&entry.root.join("yak.toml"))?;
    if manifest.id != entry.manifest.id
        || manifest.version != entry.manifest.version
        || manifest.side.required_on_join != entry.manifest.side.required_on_join
    {
        return Err("YAK identity changed after discovery".into());
    }
    build.check(&manifest)?;
    if !manifest.side.client {
        return Err("server-only YAK".into());
    }
    let bin = manifest.bin.as_ref().ok_or("no binary declared")?;
    let target = build.target.ok_or("unsupported native target")?;
    let name = format!("bin/{target}/{}", target.library_filename(&bin.library));
    let digest = yak::hash_installed_file(&entry.root, &name)?;
    if let Err(error) = yak::verify_installed(&entry.root, &[&name], keys) {
        if !allow_unsigned {
            return Err(error);
        }
        log::warn!("YAK {}: developer unsigned override: {error}", manifest.id);
    }
    Ok((entry.root.join(name), digest))
}

/// Which half of a package a WebAssembly module belongs to.
#[allow(dead_code)]
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum WasmSide {
    Client,
    Server,
}

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
    pub(crate) mixin_targets: Vec<String>,
    pub(crate) permissions: Vec<String>,
}

/// Developer switches for unsigned code. The legacy DLL switch remains a superset so existing
/// native-mod workflows keep their previous WASM behavior.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) struct UnsignedSwitches {
    pub(crate) dll: bool,
    pub(crate) wasm: bool,
}

impl UnsignedSwitches {
    pub(crate) fn from_env() -> Self {
        let dll = std::env::var_os("YOAKE_DLL_MODS_UNSIGNED");
        let wasm = std::env::var_os("YOAKE_WASM_UNSIGNED");
        Self::from_values(dll.as_deref(), wasm.as_deref())
    }

    pub(crate) fn from_values(
        dll: Option<&std::ffi::OsStr>,
        wasm: Option<&std::ffi::OsStr>,
    ) -> Self {
        let dll = dll == Some(std::ffi::OsStr::new("1"));
        let wasm = wasm == Some(std::ffi::OsStr::new("1"));
        Self { dll, wasm: dll || wasm }
    }
}

/// Client-side WebAssembly modules of the enabled .yak files; empty when there are none.
pub(crate) fn wasm_client_mods() -> Vec<WasmMod> {
    wasm_mods(WasmSide::Client)
}

/// Modules of the enabled .yak files for one side.
pub(crate) fn wasm_mods(side: WasmSide) -> Vec<WasmMod> {
    let entries = entries();
    if !entries.iter().any(|entry| entry.manifest.wasm.is_some()) {
        return Vec::new();
    }
    // A verified test worker also takes the unsigned package under test; the DLL switch is not involved.
    wasm_mods_from(
        entries,
        side,
        &yak::trusted_keys(),
        UnsignedSwitches::from_env().wasm || crate::yoake_test::is_worker(),
    )
}

pub(crate) fn wasm_mods_from(
    installed: &[Installed],
    side: WasmSide,
    keys: &[PublicKey],
    allow_unsigned: bool,
) -> Vec<WasmMod> {
    let mut mods = Vec::new();
    for entry in installed {
        let Some(wasm) = &entry.manifest.wasm else {
            continue;
        };
        // MX3L2: a full-access package holds a core module for the engine_call ABI, not a component. This
        // sandboxed path never loads it (the full-access loader, `yoake2_mods`, is the only one that may).
        if wasm.full_access() {
            log::info!("YAK {}: full-access package, left to the full-access loader (not loaded as a sandboxed component)", entry.manifest.id);
            continue;
        }
        let path = match side {
            WasmSide::Client => &wasm.client,
            WasmSide::Server => &wasm.server,
        };
        let Some(path) = path else { continue };
        match wasm_preflight(entry, path, side, keys, allow_unsigned) {
            Ok((bytes, sha256, signed)) => mods.push(WasmMod {
                mod_id: entry.manifest.id.clone(),
                version: entry.manifest.version.clone(),
                sha256,
                bytes,
                side,
                full: side == WasmSide::Server || wasm.client_full(),
                gpu: side == WasmSide::Client && wasm.has_permission("service.gpu"),
                required_on_join: entry.manifest.side.required_on_join,
                commands: side == WasmSide::Server && wasm.has_permission("service.commands"),
                permissions: wasm.permissions.clone(),
                path: entry.root.join(path),
                signed,
                mixin_targets: entry
                    .manifest
                    .mixin
                    .as_ref()
                    .map(|mixin| mixin.targets.clone())
                    .unwrap_or_default(),
            }),
            Err(reason) => {
                log::warn!("YAK {} wasm module refused: {reason}", entry.manifest.id);
                eprintln!("YAK {} wasm module refused: {reason}", entry.manifest.id);
            }
        }
    }
    mods
}

fn wasm_preflight(
    entry: &Installed,
    name: &str,
    side: WasmSide,
    keys: &[PublicKey],
    allow_unsigned: bool,
) -> Result<(Vec<u8>, String, bool), String> {
    yak::check_installed_tree(&entry.root)?;
    let manifest = Manifest::read(&entry.root.join("yak.toml"))?;
    if manifest.id != entry.manifest.id
        || manifest.version != entry.manifest.version
        || manifest.side.required_on_join != entry.manifest.side.required_on_join
        || manifest.wasm != entry.manifest.wasm
        || manifest.mixin != entry.manifest.mixin
    {
        return Err("YAK identity changed after discovery".into());
    }
    if manifest.bin.is_some() {
        return Err("package declares both native and wasm code".into());
    }
    let wasm = manifest.wasm.as_ref().ok_or("no wasm declared")?;
    if wasm.full_access() {
        return Err("full-access package: only the full-access loader may load it".into());
    }
    let declared = match side {
        WasmSide::Client => &wasm.client,
        WasmSide::Server => &wasm.server,
    };
    if declared.as_deref() != Some(name) {
        return Err("wasm path changed after discovery".into());
    }
    if side == WasmSide::Client && !manifest.side.client {
        return Err("server-only YAK".into());
    }
    if side == WasmSide::Server && !manifest.side.server {
        return Err("client-only YAK".into());
    }
    let path = entry.root.join(name);
    yak::check_plain_path(&path)?;
    let file = File::open(&path).map_err(|error| format!("{name}: {error}"))?;
    let metadata = file.metadata().map_err(|error| error.to_string())?;
    if !metadata.is_file() || metadata.len() > yak::MAX_WASM_SIZE {
        return Err("wasm module is not a regular file of at most 32 MiB".into());
    }
    let mut bytes = Vec::new();
    file.take(yak::MAX_WASM_SIZE + 1)
        .read_to_end(&mut bytes)
        .map_err(|error| error.to_string())?;
    if bytes.len() as u64 > yak::MAX_WASM_SIZE {
        return Err("wasm module exceeds 32 MiB".into());
    }
    let digest: String = <sha2::Sha256 as sha2::Digest>::digest(&bytes)
        .iter()
        .map(|byte| format!("{byte:02x}"))
        .collect();
    let signed = if allow_unsigned {
        match std::fs::symlink_metadata(entry.root.join("SIGNATURE")) {
            Ok(_) => {
                // The developer override is for packages with no signature. A present signature
                // must still be trusted and match every installed file.
                yak::verify_installed(&entry.root, &[name], keys)?;
                true
            }
            Err(error) if error.kind() == ErrorKind::NotFound => {
                log::warn!("YAK {}: developer unsigned override: no SIGNATURE file", manifest.id);
                false
            }
            Err(error) => return Err(format!("SIGNATURE: {error}")),
        }
    } else {
        yak::verify_installed(&entry.root, &[name], keys)?;
        true
    };
    if yak::hash_installed_file(&entry.root, name)? != digest {
        return Err("wasm module changed while loading".into());
    }
    Ok((bytes, digest, signed))
}


// ---------------------------------------------------------------------------------------------------------------
// MX3JN1: sandboxed packages fetched from the host this launch joins (`--remote-mods <host key>`)
// ---------------------------------------------------------------------------------------------------------------

/// The sandboxed `.yak` packages of `<game>/mods/remote/<host dir>/` as installed packages: only files named by their own
/// SHA-256 (`<sha256>.yak`, hashing to the name, at most 64 MiB), not full-access (those run only through the launcher's
/// approved list), with a client side, and compatible with this build. Each is unpacked once into
/// `<game>/yoake-remote/<host dir>/<sha256>/`. Whether the code in them runs is decided later exactly as for any
/// installed package: a trusted signature is required (`wasm_preflight`) unless the developer override is on.
pub(crate) fn remote_installed(game: &Path, host_key: &str, build: &Build, keys: &[PublicKey]) -> Vec<Installed> {
    use sha2::Digest;
    let Some(dir) = yak::control::host_remote_dir(&game.join("mods"), host_key) else {
        return Vec::new();
    };
    let Ok(read) = std::fs::read_dir(&dir) else {
        return Vec::new();
    };
    let mut files: Vec<PathBuf> = read.flatten().map(|item| item.path()).filter(|path| path.extension().is_some_and(|extension| extension == "yak")).collect();
    files.sort();
    let mut out: Vec<Installed> = Vec::new();
    for path in files {
        let result = (|| -> Result<Installed, String> {
            let stem = path.file_stem().and_then(|stem| stem.to_str()).ok_or("no file name")?.to_owned();
            if stem.len() != 64 || !stem.bytes().all(|byte| byte.is_ascii_digit() || (b'a'..=b'f').contains(&byte)) {
                return Err("a remote package is named by its SHA-256".into());
            }
            yak::check_plain_path(&path)?;
            let meta = std::fs::metadata(&path).map_err(|error| error.to_string())?;
            if !meta.is_file() || meta.len() > 64 * 1024 * 1024 {
                return Err("not a regular file of at most 64 MiB".into());
            }
            let bytes = std::fs::read(&path).map_err(|error| error.to_string())?;
            let hash: String = sha2::Sha256::digest(&bytes).iter().map(|byte| format!("{byte:02x}")).collect();
            if hash != stem {
                return Err("the file does not hash to its name".into());
            }
            let mut archive = yak::Archive::new(std::io::Cursor::new(bytes))?;
            let manifest = archive.manifest().clone();
            if manifest.wasm.as_ref().is_some_and(|wasm| wasm.full_access()) {
                return Err("full-access package: only the launcher's approved list may run it".into());
            }
            if !manifest.side.client {
                return Err("server-only YAK".into());
            }
            build.check(&manifest)?;
            let root = game.join("yoake-remote").join(yak::control::host_dir_name(host_key)).join(&stem);
            if !root.is_dir() {
                let part = root.with_extension("part");
                let _ = std::fs::remove_dir_all(&part);
                let names: Vec<String> = archive.entries().iter().map(|entry| entry.name.clone()).collect();
                for name in names {
                    let target = part.join(&name);
                    if let Some(parent) = target.parent() {
                        std::fs::create_dir_all(parent).map_err(|error| error.to_string())?;
                    }
                    let mut file = File::create_new(&target).map_err(|error| error.to_string())?;
                    archive.copy_file(&name, &mut file)?;
                }
                std::fs::rename(&part, &root).map_err(|error| error.to_string())?;
            }
            yak::check_installed_tree(&root)?;
            let installed_manifest = Manifest::read(&root.join("yak.toml"))?;
            if installed_manifest.id != manifest.id || installed_manifest.version != manifest.version {
                return Err("unpacked identity differs".into());
            }
            let content = root.join("content");
            let (content, content_wasm) = if content.is_dir() {
                let package = Package::read(&content)?;
                if package.manifest.id != manifest.id {
                    return Err("content and YAK identity differ".into());
                }
                (Some(content), package.wasm.is_some())
            } else {
                (None, false)
            };
            let names = installed_files(&root)?;
            let names: Vec<_> = names.iter().map(String::as_str).collect();
            let signed = yak::verify_installed(&root, &names, keys).is_ok();
            Ok(Installed { root, manifest: installed_manifest, content, content_wasm, signed })
        })();
        match result {
            Ok(entry) if out.iter().any(|other| other.manifest.id == entry.manifest.id) => {
                log::warn!("YAK {}: two remote packages with one id, only the first is used", entry.manifest.id);
            }
            Ok(entry) => out.push(entry),
            Err(reason) => log::warn!("remote YAK {}: not loaded: {reason}", path.display()),
        }
    }
    out
}

pub(crate) fn content_packages(
    paths: Vec<PathBuf>,
    installed: &[Installed],
) -> Vec<(PathBuf, Package, bool)> {
    let mut packages = Vec::new();
    for path in paths {
        if let Ok(package) = Package::read(&path) {
            if installed
                .iter()
                .any(|entry| entry.content.is_some() && entry.manifest.id == package.manifest.id)
            {
                log::warn!(
                    "YAK {} takes precedence over {}",
                    package.manifest.id,
                    path.display()
                );
            } else {
                packages.push((path, package, false));
            }
        }
    }
    for entry in installed {
        if let Some(path) = &entry.content {
            match Package::read(path) {
                Ok(package)
                    if package.manifest.id == entry.manifest.id
                        && Version::parse(&package.manifest.version).ok().as_ref()
                            == Some(&entry.manifest.version) =>
                {
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
        let problems = runtime
            .problems
            .lock()
            .unwrap_or_else(|error| error.into_inner());
        populate_rows_from(rows, &runtime.entries, &problems, stopped);
    }
}

fn populate_rows_from(
    rows: &mut Vec<ModRow>,
    installed: &[Installed],
    problems: &BTreeMap<String, String>,
    stopped: &str,
) {
    for entry in installed {
        let existing = rows
            .iter()
            .position(|row| row.id == entry.manifest.id)
            .map(|index| rows.remove(index));
        let problem = problems
            .get(&entry.manifest.id)
            .cloned()
            .or_else(|| entry.content_wasm.then(|| stopped.to_owned()));
        let signature = if entry.signed {
            Text::translate_with_fallback(
                "yoake.mods.verified",
                "署名確認済み / Verified signature",
                &[],
            )
        } else {
            Text::translate_with_fallback(
                "yoake.mods.unverified",
                "未署名・未検証 / Unsigned or unverified",
                &[],
            )
        };
        let source =
            Text::translate_with_fallback("yoake.mods.yakSource", ".yak · %s", &[signature])
                .plain();
        rows.push(ModRow {
            id: entry.manifest.id.clone(),
            title: format!(
                "{} {}",
                entry
                    .manifest
                    .name_ja
                    .as_ref()
                    .unwrap_or(&entry.manifest.name),
                entry.manifest.version
            ),
            description: format!("{}\n{source}", entry.manifest.description),
            problem,
            dev: true,
            native: existing.and_then(|row| row.native),
        });
    }
}
