//! Sandboxed WebAssembly client mods of installed `.yak` files (`docs/yoake-wasm.md`).
//!
//! Every accepted module is instantiated once at startup and registered in the same hook slots as a
//! DLL mod (`dll_mods.rs`, which this mirrors), after the DLL mods. Without a wasm mod nothing in here
//! runs: no Engine, no thread, no wrapper around the existing hooks. A trap (or fuel/time/memory limit)
//! disables only that mod, with one log line.
//!
//! Developer mode (`YOAKE_WASM_UNSIGNED=1`, or the legacy superset
//! `YOAKE_DLL_MODS_UNSIGNED=1`) additionally watches unsigned local wasm files and, when one changes
//! on disk, swaps in a fresh instance at the next frame or tick on the main thread.
//! `YOAKE_WASM_DEV_DIR` names a folder of raw `<id>.wasm` components that are loaded the same way.
//! Raw file ids use a `dev-` prefix and cannot reuse an installed YAK id, keeping settings and
//! storage namespaces separate from installed mods.

#[path = "wasm_mods/client_services.rs"]
mod client_services;
#[path = "wasm_mods/dev_shaders.rs"]
mod dev_shaders;
#[path = "wasm_mods/full.rs"]
mod full;
#[path = "wasm_mods/gpu_services.rs"]
mod gpu_services;
#[path = "wasm_mods/mob_services.rs"]
mod mob_services;
#[path = "wasm_mods/mob_runtime.rs"]
mod mob_runtime;
#[path = "wasm_mods/package.rs"]
pub(crate) mod package;
#[path = "wasm_mods/package_ui.rs"]
pub(crate) mod package_ui;
#[path = "wasm_mods/resources.rs"]
mod resources;
#[path = "wasm_mods/server_services.rs"]
mod server_services;
// Test mode only (`--yoake-test`): the operations of the engine test driver (`yoake_test.rs`).
#[path = "wasm_mods/test_driver.rs"]
pub(super) mod test_driver;

use super::dll_mods::{StaticMod, frame_time, native_identity, safe_component};
use crate::mod_api::*;
use horizon_mod_abi as abi;
use horizon_protocol::{
    buf::{Codec, Writer},
    item::Slot,
};
pub(crate) use server_services::{attachment_registry, prepare_attachments, reset_attachments};
use std::{
    cell::Cell,
    collections::{HashMap, HashSet},
    path::PathBuf,
    ptr,
    sync::{
        Arc, Mutex, OnceLock, TryLockError,
        atomic::{AtomicBool, AtomicU32, Ordering},
    },
    time::{Duration, SystemTime},
};
use yoake_wasm::bindings::yoake::mods::types as w;
use yoake_wasm::inject::Injection;
use yoake_wasm::{ClientHost, ClientMod, Sandbox};

const MAX_MODS: usize = 128;
const MAX_LOG_CHARS: usize = 4096;
const POLL_INTERVAL: Duration = Duration::from_secs(1);

/// Lines produced before the game's logger exists are kept and written by [`log_startup`].
static STARTUP: Mutex<Option<Vec<String>>> = Mutex::new(Some(Vec::new()));
static SANDBOX: OnceLock<Mutex<Sandbox>> = OnceLock::new();

#[cfg(test)]
static GUEST_TEST_LOCK: Mutex<()> = Mutex::new(());

fn emit(message: String) {
    let mut startup = STARTUP.lock().unwrap_or_else(|error| error.into_inner());
    match startup.as_mut() {
        Some(lines) => lines.push(message),
        None => {
            drop(startup);
            log::info!("{message}");
        }
    }
}

/// Writes the startup lines, now that the log is open. One line says whether a wasm engine exists.
pub(super) fn log_startup() {
    let lines = STARTUP
        .lock()
        .unwrap_or_else(|error| error.into_inner())
        .take();
    for line in lines.unwrap_or_default() {
        log::info!("{line}");
    }
}

/// FR-F2: gives the mod its folders (`/data`, `/game`, maybe `/host/<letter>`) and worker before it is compiled. Without a known
/// game folder the mod gets none (the old behaviour).
fn register_io(id: &str, host_filesystem: bool, worker: Option<&Vec<u8>>) {
    let Some(game) = crate::yak_mods::game_root() else { return };
    let mut io = yoake_wasm::ModIo::new(yoake_wasm::FileAccess::new(game, id, host_filesystem));
    io.worker = worker.cloned();
    io.log = yoake_wasm::LogSink::new(|line| log::info!("{line}"));
    sandbox().lock().unwrap_or_else(|error| error.into_inner()).set_mod_io(id, io);
}

fn sandbox() -> &'static Mutex<Sandbox> {
    SANDBOX.get_or_init(|| Mutex::new(Sandbox::new()))
}

/// Which hooks a module subscribed to, one bit per catalog hook name.
const HOOKS: &[&str] = &[
    "tick",
    "key",
    "hud",
    "net_from_server",
    "frame",
    "local_player_tick_end",
    "net_frames_listened",
    "net_frame_received",
    "net_session_ended",
    "gui_layer_pre",
    "gui_layer_post",
    "screen_render_post",
    "screen_key_pressed_pre",
    "screen_char_typed_pre",
    "screen_mouse_button_pressed_pre",
    "screen_mouse_button_released_pre",
    "screen_mouse_scrolled_pre",
    "settings",
    "settings_click",
];
const TICK: u32 = 1 << 0;
const KEY: u32 = 1 << 1;
const HUD: u32 = 1 << 2;
const NET_FROM_SERVER: u32 = 1 << 3;
const FRAME: u32 = 1 << 4;
const LOCAL_PLAYER_TICK_END: u32 = 1 << 5;
const NET_FRAMES_LISTENED: u32 = 1 << 6;
const NET_FRAME_RECEIVED: u32 = 1 << 7;
const NET_SESSION_ENDED: u32 = 1 << 8;
const GUI_LAYER_PRE: u32 = 1 << 9;
const GUI_LAYER_POST: u32 = 1 << 10;
const SCREEN_RENDER_POST: u32 = 1 << 11;
const SCREEN_KEY_PRESSED_PRE: u32 = 1 << 12;
const SCREEN_CHAR_TYPED_PRE: u32 = 1 << 13;
const SCREEN_MOUSE_BUTTON_PRESSED_PRE: u32 = 1 << 14;
const SCREEN_MOUSE_BUTTON_RELEASED_PRE: u32 = 1 << 15;
const SCREEN_MOUSE_SCROLLED_PRE: u32 = 1 << 16;
const SETTINGS: u32 = 1 << 17;
const SETTINGS_CLICK: u32 = 1 << 18;

fn subscription_mask(module: &ClientMod) -> u32 {
    HOOKS.iter().enumerate().fold(0, |mask, (bit, name)| {
        mask | if module.subscribed(name) { 1 << bit } else { 0 }
    })
}

// ---------------------------------------------------------------------------------------------
// Startup
// ---------------------------------------------------------------------------------------------

/// One module to run: from an installed .yak, or (developer mode) a raw file of `YOAKE_WASM_DEV_DIR`.
struct Source {
    id: String,
    version: String,
    sha256: Option<String>,
    required_on_join: bool,
    /// The module targets `client-full` rather than `client-mod`.
    full: bool,
    mixin_targets: Vec<String>,
    permissions: Vec<String>,
    host_filesystem: bool,
    worker: Option<Vec<u8>>,
    gpu: bool,
    bytes: Vec<u8>,
    /// Watched for hot reload (developer mode and not signed).
    watch: Option<PathBuf>,
}

fn dev_sources_with(
    developer: bool,
    dir: impl FnOnce() -> Option<std::ffi::OsString>,
    reserved_ids: &HashSet<String>,
) -> Vec<Source> {
    if !developer {
        return Vec::new();
    }
    let Some(dir) = dir().filter(|dir| !dir.is_empty()) else {
        return Vec::new();
    };
    let Ok(entries) = std::fs::read_dir(&dir) else {
        emit(format!(
            "wasm: YOAKE_WASM_DEV_DIR {} cannot be read",
            PathBuf::from(&dir).display()
        ));
        return Vec::new();
    };
    let mut files: Vec<_> = entries.flatten().map(|entry| entry.path()).collect();
    files.sort();
    let full = std::env::var("YOAKE_WASM_DEV_FULL").is_ok_and(|value| value == "1");
    let gpu = full && std::env::var("YOAKE_WASM_DEV_GPU").is_ok_and(|value| value == "1");
    let mut sources = Vec::new();
    for path in files {
        if path.extension().and_then(|extension| extension.to_str()) != Some("wasm") {
            continue;
        }
        let Some(name) = path.file_stem().and_then(|stem| stem.to_str()) else {
            continue;
        };
        let id = format!("dev-{}", name.replace('.', "-"));
        if !safe_component(&id) {
            emit(format!(
                "wasm: dev module {} skipped: unsafe id",
                path.display()
            ));
            continue;
        }
        if reserved_ids.contains(&id) {
            emit(format!(
                "wasm: dev module {} skipped: id conflicts with an installed mod",
                path.display()
            ));
            continue;
        }
        match read_bounded(&path) {
            Ok(bytes) => sources.push(Source {
                id,
                version: "0.0.0-dev".into(),
                sha256: None,
                required_on_join: false,
                full,
                gpu,
                mixin_targets: Vec::new(),
                permissions: Vec::new(),
                host_filesystem: false,
                worker: None,
                bytes,
                watch: Some(path),
            }),
            Err(error) => emit(format!(
                "wasm: dev module {} skipped: {error}",
                path.display()
            )),
        }
    }
    sources
}

fn read_bounded(path: &std::path::Path) -> Result<Vec<u8>, String> {
    use std::io::Read;
    let metadata = std::fs::symlink_metadata(path).map_err(|error| error.to_string())?;
    if !metadata.is_file() {
        return Err("module is not a regular file".into());
    }
    let file = std::fs::File::open(path).map_err(|error| error.to_string())?;
    if !file
        .metadata()
        .map_err(|error| error.to_string())?
        .is_file()
    {
        return Err("module is not a regular file".into());
    }
    let mut bytes = Vec::new();
    file.take(horizon_mods::yak::MAX_WASM_SIZE + 1)
        .read_to_end(&mut bytes)
        .map_err(|error| error.to_string())?;
    if bytes.len() as u64 > horizon_mods::yak::MAX_WASM_SIZE {
        return Err("module exceeds 32 MiB".into());
    }
    Ok(bytes)
}

fn installed_source(module: crate::yak_mods::WasmMod, developer: bool) -> Source {
    let watch = (developer && !module.signed).then(|| module.path.clone());
    Source {
        id: module.mod_id,
        version: module.version.to_string(),
        sha256: Some(module.sha256),
        // Hot reload changes the bytes during development, so this digest cannot be a stable join requirement.
        required_on_join: module.required_on_join && watch.is_none(),
        full: module.full,
        gpu: module.gpu,
        mixin_targets: module.mixin_targets,
        permissions: module.permissions,
        host_filesystem: module.host_filesystem,
        worker: module.worker,
        bytes: module.bytes,
        watch,
    }
}

/// One package's modules once accepted: its hook objects, its client module's world and the
/// identity the multiplayer check sees.
struct Accepted {
    gpu: bool,
    gpu_fingerprint: String,
    id: String,
    version: String,
    /// The module's SHA-256 when the package is required on join.
    digest: Option<String>,
    hooks: Vec<Box<dyn Hooks>>,
    client_full: Option<yoake_wasm::ClientFullMod>,
    client_reload: Option<Arc<Reload>>,
    server_full: Option<yoake_wasm::ServerFullMod>,
    /// The server module's bytes and its `service.commands` grant, kept to start each world afresh.
    server_source: Option<full::ServerSource>,
    worlds: Vec<&'static str>,
}

struct InjectionMod {
    _injection: Injection,
}

impl Hooks for InjectionMod {
    fn tick(&self, _host: &mut dyn Host, _event: &mut TickEvent) {
        if self._injection.has_config_notifications() {
            self._injection.notify_config_changes();
        }
    }

    fn frame(&self, _host: &mut dyn Host, _event: &mut FrameEvent) {
        if self._injection.has_config_notifications() {
            self._injection.notify_config_changes();
        }
    }
}

fn load_injection(
    id: &str,
    bytes: &[u8],
    targets: &[String],
    side: yoake_inject_rt::Side,
    permissions: &[String],
) -> Result<Injection, String> {
    let config_side = match side {
        yoake_inject_rt::Side::Client => horizon_mods::yak::ConfigRuntimeSide::Client,
        _ => horizon_mods::yak::ConfigRuntimeSide::Server,
    };
    load_injection_with_services(
        id,
        bytes,
        targets,
        side,
        permissions,
        package::services(id, config_side),
    )
}

fn load_injection_with_services(
    id: &str,
    bytes: &[u8],
    targets: &[String],
    side: yoake_inject_rt::Side,
    permissions: &[String],
    services: Result<yoake_wasm::package_services::ServiceHandle, String>,
) -> Result<Injection, String> {
    let mut sandbox = sandbox().lock().unwrap_or_else(|error| error.into_inner());
    match services {
        Ok(services) => Injection::instantiate_with_services(&mut sandbox, id, bytes, targets, side, permissions, services),
        Err(error) => {
            eprintln!("[yoake-inject:{id}] package services unavailable: {error}; loading without services");
            Injection::instantiate_with_permissions(&mut sandbox, id, bytes, targets, side, permissions)
        }
    }
    .map_err(|error| error.to_string())
}

fn gpu_fingerprint(version: &str, bytes: &[u8]) -> String {
    use sha2::{Digest, Sha256};

    let mut digest = Sha256::new();
    digest.update((version.len() as u64).to_le_bytes());
    digest.update(version.as_bytes());
    digest.update(bytes);
    format!("{:x}", digest.finalize())
}

pub(super) fn extend(
    mods: Mods,
    identities: &'static [super::NativeMod],
) -> (Mods, &'static [super::NativeMod]) {
    let developer = crate::yak_mods::UnsignedSwitches::from_env().wasm;
    // A dedicated server has no client: its client modules only give the package's identity.
    let dedicated = crate::yak_mods::dedicated();
    let entries = crate::yak_mods::entries();
    let mut reserved_ids: HashSet<String> = identities
        .iter()
        .map(|identity| identity.id.to_owned())
        .collect();
    reserved_ids.extend(entries.iter().map(|entry| entry.manifest.id.clone()));
    let mut sources: Vec<Source> = crate::yak_mods::wasm_client_mods()
        .into_iter()
        .map(|module| installed_source(module, developer))
        .collect();
    sources.extend(dev_sources_with(
        developer,
        || std::env::var_os("YOAKE_WASM_DEV_DIR"),
        &reserved_ids,
    ));
    let servers = crate::yak_mods::wasm_mods(crate::yak_mods::WasmSide::Server);
    if sources.is_empty() && servers.is_empty() {
        emit("wasm: no mods (engine not started)".into());
        return (mods, identities);
    }

    let mut names: HashSet<String> = identities
        .iter()
        .map(|identity| identity.id.to_owned())
        .collect();
    let mut accepted: Vec<Accepted> = Vec::new();
    // Packages whose client module did not load: their server module does not run either, so the
    // package never takes another module's digest as its identity.
    let mut client_failed: HashSet<String> = HashSet::new();
    let mut watches = Vec::new();
    for source in sources {
        if identities.len() + accepted.len() >= MAX_MODS || !names.insert(source.id.clone()) {
            emit(format!(
                "wasm: mod {} skipped: duplicate or too many native identities",
                source.id
            ));
            continue;
        }
        register_io(&source.id, source.host_filesystem, source.worker.as_ref());
        let digest = source
            .required_on_join
            .then(|| source.sha256.clone())
            .flatten();
        let mut package = Accepted {
            gpu_fingerprint: gpu_fingerprint(&source.version, &source.bytes),
            id: source.id.clone(),
            version: source.version.clone(),
            digest,
            hooks: Vec::new(),
            client_full: None,
            client_reload: None,
            server_full: None,
            server_source: None,
            worlds: Vec::new(),
            gpu: source.gpu,
        };
        if dedicated {
            accepted.push(package);
            continue;
        }
        if !source.mixin_targets.is_empty() {
            match load_injection(
                &source.id,
                &source.bytes,
                &source.mixin_targets,
                yoake_inject_rt::Side::Client,
                &source.permissions,
            ) {
                Ok(injection) => {
                    package.hooks.push(Box::new(InjectionMod {
                        _injection: injection,
                    }));
                    package.worlds.push("inject-client");
                }
                Err(error) => {
                    emit(format!("wasm: mod {} not loaded: {error}", source.id));
                    names.remove(&source.id);
                    client_failed.insert(source.id.clone());
                    continue;
                }
            }
        } else if source.full {
            let reload = source.watch.as_ref().map(|path| {
                Arc::new(Reload {
                    shaders: source
                        .gpu
                        .then(|| dev_shaders::DevShaders::new(path, &source.id))
                        .flatten()
                        .map(|shaders| Arc::new(Mutex::new(shaders))),
                    ..Reload::default()
                })
            });
            match full::load_client_with_shaders(
                &full::ModuleSource {
                    id: &source.id,
                    bytes: &source.bytes,
                    commands: false,
                    gpu: source.gpu,
                },
                reload.as_ref().and_then(|reload| reload.shaders.clone()),
            ) {
                Ok(module) => {
                    package.client_full = Some(module);
                    package.client_reload =
                        source.watch.as_ref().zip(reload).map(|(path, reload)| {
                            watches.push(Watch::new(path.clone(), Arc::clone(&reload)));
                            reload
                        });
                    package.worlds.push("client-full");
                }
                Err(error) => {
                    emit(format!("wasm: mod {} not loaded: {error}", source.id));
                    names.remove(&source.id);
                    client_failed.insert(source.id.clone());
                    continue;
                }
            }
        } else {
            let module = {
                let mut sandbox = sandbox().lock().unwrap_or_else(|error| error.into_inner());
                ClientMod::instantiate(
                    &mut sandbox,
                    source.id.clone(),
                    &source.bytes,
                    Box::new(GameHost::new(&source.id)),
                )
            };
            match module {
                Ok(module) => {
                    let reload = source.watch.as_ref().map(|path| {
                        let reload = Arc::new(Reload::default());
                        watches.push(Watch::new(path.clone(), Arc::clone(&reload)));
                        reload
                    });
                    package.hooks.push(Box::new(WasmMod::new(
                        source.id.clone(),
                        source.version.clone(),
                        module,
                        reload,
                    )));
                    package.worlds.push("client-mod");
                }
                Err(error) => {
                    emit(format!("wasm: mod {} not loaded: {error}", source.id));
                    names.remove(&source.id);
                    client_failed.insert(source.id.clone());
                    continue;
                }
            }
        }
        accepted.push(package);
    }
    for server in servers {
        if client_failed.contains(&server.mod_id) {
            emit(format!(
                "wasm: mod {} server module skipped: its client module did not load",
                server.mod_id
            ));
            continue;
        }
        let position = accepted
            .iter()
            .position(|package| package.id == server.mod_id);
        if position.is_none()
            && (identities.len() + accepted.len() >= MAX_MODS
                || !names.insert(server.mod_id.clone()))
        {
            emit(format!(
                "wasm: mod {} skipped: duplicate or too many native identities",
                server.mod_id
            ));
            continue;
        }
        register_io(&server.mod_id, server.host_filesystem, server.worker.as_ref());
        let digest = server.required_on_join.then(|| server.sha256.clone());
        let mut package = match position {
            Some(position) => accepted.swap_remove(position),
            None => Accepted {
                gpu_fingerprint: String::new(),
                id: server.mod_id.clone(),
                version: server.version.to_string(),
                digest,
                hooks: Vec::new(),
                client_full: None,
                client_reload: None,
                gpu: false,
                server_full: None,
                server_source: None,
                worlds: Vec::new(),
            },
        };
        let injection_loaded = if !server.mixin_targets.is_empty() {
            match load_injection(
                &server.mod_id,
                &server.bytes,
                &server.mixin_targets,
                yoake_inject_rt::Side::Server,
                &server.permissions,
            ) {
                Ok(injection) => {
                    package.hooks.push(Box::new(InjectionMod {
                        _injection: injection,
                    }));
                    package.worlds.push("inject-server");
                    true
                }
                Err(error) => {
                    emit(format!(
                        "wasm: mod {} server injection not loaded: {error}",
                        server.mod_id
                    ));
                    false
                }
            }
        } else {
            if server.permissions.iter().any(|p| p == "service.mobs") {
                mob_services::grant(&server.mod_id);
            }
            let source =
                full::ServerSource::new(&server.mod_id, server.bytes.clone(), server.commands);
            match full::load_server(&source.module()) {
                Ok(module) => {
                    if server.commands {
                        emit(format!(
                            "wasm: mod {} may run server commands (service.commands; the dangerous ones stay denied)",
                            server.mod_id
                        ));
                    }
                    package.server_full = Some(module);
                    package.server_source = Some(source);
                    package.worlds.push("server-full");
                }
                Err(error) => emit(format!(
                    "wasm: mod {} server module not loaded: {error}",
                    server.mod_id
                )),
            }
            false
        };
        // A package that only had a server module and lost it has nothing to run.
        if position.is_some() || package.server_full.is_some() || injection_loaded {
            accepted.push(package);
        } else {
            names.remove(&server.mod_id);
        }
    }
    if !watches.is_empty() {
        spawn_watcher(watches);
    }
    if accepted.is_empty() {
        return (mods, identities);
    }
    let running = accepted
        .iter()
        .filter(|package| !package.worlds.is_empty())
        .count();
    if running == 0 {
        emit("wasm: no mods (engine not started)".into());
    } else {
        emit(format!("wasm: {running} mod(s), engine started"));
    }

    let mut combined: Vec<Box<dyn Hooks>> = (0..mods.len())
        .map(|index| {
            Box::new(StaticMod {
                original: mods.clone(),
                index,
            }) as Box<dyn Hooks>
        })
        .collect();
    let mut owned = identities.to_vec();
    for mut package in accepted {
        if package.client_full.is_some() || package.server_full.is_some() {
            let mut full_mod = full::FullMod::new(
                package.id.clone(),
                package.client_full.take(),
                package.server_full.take(),
            )
            .with_client_reload(package.client_reload.take())
            .with_gpu(package.gpu, &package.version, &package.gpu_fingerprint);
            if let Some(source) = package.server_source.take() {
                full_mod = full_mod.with_server_source(source);
            }
            package.hooks.push(Box::new(full_mod));
        }
        if !package.worlds.is_empty() {
            emit(format!(
                "wasm: mod {} {} loaded ({})",
                package.id,
                package.version,
                package.worlds.join(", ")
            ));
        }
        let id: &'static str = Box::leak(package.id.clone().into_boxed_str());
        let (version, side) = native_identity(&package.version, package.digest.as_deref());
        let version: &'static str = Box::leak(version.into_boxed_str());
        owned.push(super::NativeMod { id, version, side });
        combined.extend(package.hooks);
    }
    (Arc::from(combined), Box::leak(owned.into_boxed_slice()))
}

// ---------------------------------------------------------------------------------------------
// Hot reload (developer mode only)
// ---------------------------------------------------------------------------------------------

#[derive(Default)]
struct Reload {
    ready: AtomicBool,
    bytes: Mutex<Option<Vec<u8>>>,
    shaders: Option<Arc<Mutex<dev_shaders::DevShaders>>>,
    #[cfg(test)]
    test_limits: Option<yoake_wasm::TimeLimits>,
}

struct Watch {
    path: PathBuf,
    reload: Arc<Reload>,
    /// The file as it was last seen, and as it was last handed over.
    seen: Option<(SystemTime, u64)>,
    applied: Option<(SystemTime, u64)>,
    shaders_pending: bool,
}

impl Watch {
    fn new(path: PathBuf, reload: Arc<Reload>) -> Self {
        let state = Self::stat(&path);
        Self {
            path,
            reload,
            seen: state,
            applied: state,
            shaders_pending: false,
        }
    }

    fn stat(path: &std::path::Path) -> Option<(SystemTime, u64)> {
        let metadata = std::fs::symlink_metadata(path).ok()?;
        if !metadata.is_file() {
            return None;
        }
        Some((metadata.modified().ok()?, metadata.len()))
    }

    /// A change counts once the file has stayed the same for a whole poll (a build may still be writing).
    fn poll(&mut self) {
        self.shaders_pending |= self.reload.shaders.as_ref().is_some_and(|shaders| {
            shaders
                .lock()
                .unwrap_or_else(|error| error.into_inner())
                .poll()
        });
        let now = Self::stat(&self.path);
        if now != self.seen {
            self.seen = now;
        } else if now.is_some() && (now != self.applied || self.shaders_pending) {
            self.applied = now;
            match read_bounded(&self.path) {
                Ok(bytes) => {
                    self.shaders_pending = false;
                    *self
                        .reload
                        .bytes
                        .lock()
                        .unwrap_or_else(|error| error.into_inner()) = Some(bytes);
                    self.reload.ready.store(true, Ordering::Release);
                }
                Err(error) => emit(format!(
                    "wasm: {} cannot be reloaded: {error}",
                    self.path.display()
                )),
            }
        }
    }
}

fn spawn_watcher(mut watches: Vec<Watch>) {
    let spawned = std::thread::Builder::new()
        .name("yoake-wasm-watch".into())
        .spawn(move || {
            loop {
                std::thread::sleep(POLL_INTERVAL);
                for watch in &mut watches {
                    watch.poll();
                }
            }
        });
    if let Err(error) = spawned {
        emit(format!("wasm: hot reload unavailable: {error}"));
    }
}

// ---------------------------------------------------------------------------------------------
// The hook object
// ---------------------------------------------------------------------------------------------

struct State {
    module: ClientMod,
    reported: bool,
}

struct WasmMod {
    id: String,
    version: String,
    state: Mutex<State>,
    mask: AtomicU32,
    reload: Option<Arc<Reload>>,
}

enum HostAccess<'call> {
    None,
    Read(&'call dyn Host),
    Write(&'call mut dyn Host),
}

impl HostAccess<'_> {
    fn read(&self) -> Option<&dyn Host> {
        match self {
            Self::None => None,
            Self::Read(host) => Some(*host),
            Self::Write(host) => Some(&**host),
        }
    }

    fn write(&mut self) -> Option<&mut dyn Host> {
        match self {
            Self::Write(host) => Some(&mut **host),
            _ => None,
        }
    }
}

/// The game services of one hook call. They live only for the call; the guest reaches them through
/// `ACTIVE`, set while (and only while) a hook runs on this thread.
struct Invocation<'call> {
    host: HostAccess<'call>,
    batch: Option<&'call mut HudBatch>,
    batch_start: usize,
    client: ClientExtras<'call>,
}

#[derive(Default)]
struct ClientExtras<'call> {
    world_draws: Option<&'call mut crate::gfx::native_services::WorldDraws>,
    screen_kind: Option<String>,
    corners: Option<&'call mut Corners>,
    mob: Option<&'call NativeMobRender<'call>>,
    mob_draw: Option<&'call mut BlockEntityDraw>,
    camera: Option<&'call mut Option<CameraOverride>>,
}

thread_local! {
    static ACTIVE: Cell<*mut Invocation<'static>> = const { Cell::new(ptr::null_mut()) };
}

struct Scope {
    previous: *mut Invocation<'static>,
}

impl Scope {
    fn enter(invocation: &mut Invocation<'_>) -> Self {
        let pointer = (invocation as *mut Invocation<'_>).cast::<Invocation<'static>>();
        let scope = Self {
            previous: ACTIVE.with(|active| active.replace(pointer)),
        };
        resources::drain();
        scope
    }
}

impl Drop for Scope {
    fn drop(&mut self) {
        resources::drain();
        ACTIVE.with(|active| active.set(self.previous));
    }
}

pub(crate) fn drain_resources(host: &mut dyn Host) {
    resources::drain_with(host);
}

pub(crate) fn resource_owners() -> HashSet<usize> {
    resources::owners()
}

/// Runs `operation` against the current hook's services. A service called while another is running
/// (re-entrancy) or outside a hook is refused.
fn service<R>(
    operation: impl FnOnce(&mut Invocation<'_>) -> Result<R, w::HostError>,
) -> Result<R, w::HostError> {
    struct Restore(*mut Invocation<'static>);
    impl Drop for Restore {
        fn drop(&mut self) {
            ACTIVE.with(|active| active.set(self.0));
        }
    }
    let pointer = ACTIVE.with(|active| active.replace(ptr::null_mut()));
    if pointer.is_null() {
        return Err(w::HostError::Unsupported);
    }
    let _restore = Restore(pointer);
    // SAFETY: the pointer was stored by `Scope::enter` from a live `&mut Invocation` that outlives the
    // hook call this runs under (the guest only runs inside it, on this thread), and `ACTIVE` was
    // cleared above so no second reference exists while this one does.
    let invocation = unsafe { &mut *pointer };
    match std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| operation(invocation))) {
        Ok(result) => result,
        Err(payload) => {
            std::mem::forget(payload);
            Err(w::HostError::Panic)
        }
    }
}

impl WasmMod {
    fn new(id: String, version: String, module: ClientMod, reload: Option<Arc<Reload>>) -> Self {
        let mask = subscription_mask(&module);
        Self {
            id,
            version,
            state: Mutex::new(State {
                module,
                reported: false,
            }),
            mask: AtomicU32::new(mask),
            reload,
        }
    }

    fn invoke<'call, R>(
        &self,
        bit: u32,
        host: HostAccess<'call>,
        batch: Option<&'call mut HudBatch>,
        call: impl FnOnce(&mut ClientMod) -> Option<R>,
    ) -> Option<R> {
        if self.mask.load(Ordering::Relaxed) & bit == 0 {
            return None;
        }
        let mut state = match self.state.try_lock() {
            Ok(state) => state,
            Err(TryLockError::WouldBlock) => return None,
            Err(TryLockError::Poisoned(error)) => {
                self.mask.store(0, Ordering::Relaxed);
                drop(error);
                log::error!("wasm mod {} disabled: poisoned instance", self.id);
                return None;
            }
        };
        let batch_start = batch.as_deref().map_or(0, |batch| batch.vertices.len());
        let mut invocation = Invocation {
            host,
            batch,
            batch_start,
            client: ClientExtras::default(),
        };
        let result = {
            let _scope = Scope::enter(&mut invocation);
            call(&mut state.module)
        };
        self.after_call(&mut state);
        result
    }

    fn after_call(&self, state: &mut State) {
        if state.module.is_disabled() {
            self.mask.store(0, Ordering::Relaxed);
            if !state.reported {
                state.reported = true;
                log::error!(
                    "wasm mod {} disabled: trapped or exceeded its limits (see the line above)",
                    self.id
                );
            }
        }
    }

    /// Developer mode: swaps in a freshly built module at a safe point (a frame or tick on the main thread).
    fn reload_if_changed(&self, host: &mut dyn Host) {
        let Some(reload) = &self.reload else { return };
        if !reload.ready.swap(false, Ordering::Acquire) {
            return;
        }
        let Some(bytes) = reload
            .bytes
            .lock()
            .unwrap_or_else(|error| error.into_inner())
            .take()
        else {
            return;
        };
        let Ok(mut state) = self.state.try_lock() else {
            reload.ready.store(true, Ordering::Release);
            *reload
                .bytes
                .lock()
                .unwrap_or_else(|error| error.into_inner()) = Some(bytes);
            return;
        };
        let mut invocation = Invocation {
            host: HostAccess::Write(host),
            batch: None,
            batch_start: 0,
            client: ClientExtras::default(),
        };
        let result = {
            let _scope = Scope::enter(&mut invocation);
            let mut sandbox = sandbox().lock().unwrap_or_else(|error| error.into_inner());
            ClientMod::instantiate(
                &mut sandbox,
                self.id.clone(),
                &bytes,
                Box::new(GameHost::new(&self.id)),
            )
        };
        match result {
            Ok(module) => {
                self.mask
                    .store(subscription_mask(&module), Ordering::Relaxed);
                *state = State {
                    module,
                    reported: false,
                };
                log::info!("wasm mod {} reloaded", self.id);
            }
            Err(error) => log::warn!(
                "wasm mod {} not reloaded (the old instance keeps running): {error}",
                self.id
            ),
        }
    }
}

fn flow(cancel: Option<bool>) -> Flow {
    if cancel == Some(true) {
        Flow::Cancel
    } else {
        Flow::Continue
    }
}

fn screen_view(screen: &NativeScreenView<'_>) -> w::ScreenView {
    w::ScreenView {
        id: screen.id.to_owned(),
        rect: w::Rect {
            x: screen.rect[0],
            y: screen.rect[1],
            width: screen.rect[2],
            height: screen.rect[3],
        },
        mouse: w::Point {
            x: screen.mouse[0],
            y: screen.mouse[1],
        },
        partial_tick: screen.partial_tick,
    }
}

fn net_state(state: NetState) -> w::NetState {
    match state {
        NetState::Login => w::NetState::Login,
        NetState::Configuration => w::NetState::Configuration,
        NetState::Play => w::NetState::Play,
    }
}

impl Hooks for WasmMod {
    fn tick(&self, host: &mut dyn Host, _event: &mut TickEvent) {
        if self.reload.is_some() {
            self.reload_if_changed(host);
        }
        if self.mask.load(Ordering::Relaxed) & TICK != 0 && host.window() {
            self.invoke(TICK, HostAccess::Write(host), None, |module| module.tick(0));
        }
    }

    fn key(&self, host: &mut dyn Host, event: &mut KeyEvent<'_>) {
        self.invoke(KEY, HostAccess::Write(host), None, |module| {
            module.key(event.key, event.pressed)
        });
    }

    fn hud(&self, host: &mut dyn Host, event: &mut HudEvent<'_>) {
        self.invoke(
            HUD,
            HostAccess::Write(host),
            Some(&mut *event.batch),
            |module| module.hud(),
        );
    }

    fn net_from_server(&self, host: &mut dyn Host, event: &mut NetFromServerEvent<'_>) {
        self.invoke(NET_FROM_SERVER, HostAccess::Write(host), None, |module| {
            module.net_from_server(event.channel, event.data)
        });
    }

    fn frame(&self, host: &mut dyn Host, event: &mut FrameEvent) {
        if self.reload.is_some() {
            self.reload_if_changed(host);
        }
        let elapsed = event.elapsed.as_nanos().min(u64::MAX as u128) as u64;
        self.invoke(FRAME, HostAccess::Write(host), None, |module| {
            module.frame(elapsed)
        });
    }

    fn local_player_tick_end(&self, host: &mut dyn Host, _event: &mut LocalPlayerTickEndEvent) {
        self.invoke(
            LOCAL_PLAYER_TICK_END,
            HostAccess::Write(host),
            None,
            |module| module.local_player_tick_end(0),
        );
    }

    fn net_frames_listened(&self, event: &mut NetFramesListenedEvent) {
        let listened = event.listened;
        if let Some(result) = self.invoke(NET_FRAMES_LISTENED, HostAccess::None, None, |module| {
            module.net_frames_listened(listened)
        }) {
            event.listened |= result;
        }
    }

    fn net_frame_received(&self, event: &mut NetFrameReceivedEvent<'_>) {
        if self.mask.load(Ordering::Relaxed) & NET_FRAME_RECEIVED == 0 {
            return;
        }
        let frame = event.frame;
        let received = w::ReceivedFrame {
            state: net_state(frame.state),
            packet_id: frame.id,
            body: frame.body.to_vec(),
            monotonic_ns: frame_time(frame.at),
            session: frame.session,
        };
        self.invoke(NET_FRAME_RECEIVED, HostAccess::None, None, |module| {
            module.net_frame_received(&received)
        });
    }

    fn net_session_ended(&self, event: &mut NetSessionEndedEvent<'_>) {
        self.invoke(NET_SESSION_ENDED, HostAccess::None, None, |module| {
            module.net_session_ended(event.session, event.reason)
        });
    }

    fn gui_layer_pre(&self, host: &mut dyn Host, event: &mut GuiLayerPreEvent<'_>) -> Flow {
        flow(self.invoke(
            GUI_LAYER_PRE,
            HostAccess::Write(host),
            Some(&mut *event.batch),
            |module| module.gui_layer_pre(event.layer),
        ))
    }

    fn gui_layer_post(&self, host: &mut dyn Host, event: &mut GuiLayerPostEvent<'_>) {
        self.invoke(
            GUI_LAYER_POST,
            HostAccess::Write(host),
            Some(&mut *event.batch),
            |module| module.gui_layer_post(event.layer),
        );
    }

    fn screen_render_post(&self, host: &dyn Host, event: &mut ScreenRenderPostEvent<'_>) {
        if self.mask.load(Ordering::Relaxed) & SCREEN_RENDER_POST == 0 {
            return;
        }
        let screen = screen_view(event.screen);
        self.invoke(
            SCREEN_RENDER_POST,
            HostAccess::Read(host),
            Some(&mut *event.batch),
            |module| module.screen_render_post(&screen),
        );
    }

    fn screen_key_pressed_pre(
        &self,
        host: &dyn Host,
        event: &mut ScreenKeyPressedPreEvent<'_>,
    ) -> Flow {
        if self.mask.load(Ordering::Relaxed) & SCREEN_KEY_PRESSED_PRE == 0 {
            return Flow::Continue;
        }
        let screen = screen_view(event.screen);
        flow(self.invoke(
            SCREEN_KEY_PRESSED_PRE,
            HostAccess::Read(host),
            None,
            |module| {
                module.screen_key_pressed_pre(&screen, event.key, event.repeat, event.modifiers)
            },
        ))
    }

    fn screen_char_typed_pre(
        &self,
        host: &dyn Host,
        event: &mut ScreenCharTypedPreEvent<'_>,
    ) -> Flow {
        if self.mask.load(Ordering::Relaxed) & SCREEN_CHAR_TYPED_PRE == 0 {
            return Flow::Continue;
        }
        let screen = screen_view(event.screen);
        flow(self.invoke(
            SCREEN_CHAR_TYPED_PRE,
            HostAccess::Read(host),
            None,
            |module| module.screen_char_typed_pre(&screen, event.c as u32, event.modifiers),
        ))
    }

    fn screen_mouse_button_pressed_pre(
        &self,
        host: &dyn Host,
        event: &mut ScreenMouseButtonPressedPreEvent<'_>,
    ) -> Flow {
        if self.mask.load(Ordering::Relaxed) & SCREEN_MOUSE_BUTTON_PRESSED_PRE == 0 {
            return Flow::Continue;
        }
        let screen = screen_view(event.screen);
        flow(self.invoke(
            SCREEN_MOUSE_BUTTON_PRESSED_PRE,
            HostAccess::Read(host),
            None,
            |module| module.screen_mouse_button_pressed_pre(&screen, event.button, event.modifiers),
        ))
    }

    fn screen_mouse_button_released_pre(
        &self,
        host: &dyn Host,
        event: &mut ScreenMouseButtonReleasedPreEvent<'_>,
    ) -> Flow {
        if self.mask.load(Ordering::Relaxed) & SCREEN_MOUSE_BUTTON_RELEASED_PRE == 0 {
            return Flow::Continue;
        }
        let screen = screen_view(event.screen);
        flow(self.invoke(
            SCREEN_MOUSE_BUTTON_RELEASED_PRE,
            HostAccess::Read(host),
            None,
            |module| {
                module.screen_mouse_button_released_pre(&screen, event.button, event.modifiers)
            },
        ))
    }

    fn screen_mouse_scrolled_pre(
        &self,
        host: &dyn Host,
        event: &mut ScreenMouseScrolledPreEvent<'_>,
    ) -> Flow {
        if self.mask.load(Ordering::Relaxed) & SCREEN_MOUSE_SCROLLED_PRE == 0 {
            return Flow::Continue;
        }
        let screen = screen_view(event.screen);
        flow(self.invoke(
            SCREEN_MOUSE_SCROLLED_PRE,
            HostAccess::Read(host),
            None,
            |module| {
                module.screen_mouse_scrolled_pre(
                    &screen,
                    (event.scroll[0], event.scroll[1]),
                    event.modifiers,
                )
            },
        ))
    }

    fn settings(&self, host: &mut dyn Host, event: &mut SettingsEvent<'_>) {
        if let Some(buttons) = self.invoke(SETTINGS, HostAccess::Write(host), None, |module| {
            module.settings_for(event.mod_id)
        }) {
            event
                .buttons
                .extend(buttons.into_iter().take(abi::MAX_SETTINGS_BUTTONS));
        }
    }

    fn settings_click(&self, host: &mut dyn Host, event: &mut SettingsClickEvent<'_>) {
        self.invoke(SETTINGS_CLICK, HostAccess::Write(host), None, |module| {
            module.settings_click_for(event.mod_id, event.button as u64)
        });
    }
}

// ---------------------------------------------------------------------------------------------
// The services a mod may call: the same ones DLL mods get, through `Invocation`
// ---------------------------------------------------------------------------------------------

struct GameHost {
    id: String,
    textures: HashMap<u64, NativeGuiTexture>,
    next_texture: u64,
    native_id: &'static str,
    native_index: usize,
    streams: HashMap<u32, NativeAudioStreamId>,
    next_stream: u32,
    gpu_enabled: bool,
    gpu_ready: Arc<AtomicBool>,
    #[cfg(test)]
    gpu_recording_fault: Option<fn(&wgpu::Device)>,
    dev_shaders: Option<Arc<Mutex<dev_shaders::DevShaders>>>,
    resources: resources::Registration,
}

impl GameHost {
    fn new(id: &str) -> Self {
        let (native_id, native_index) = client_services::identity(id).unwrap_or(("", usize::MAX));
        Self {
            id: id.to_owned(),
            textures: HashMap::new(),
            next_texture: 1,
            native_id,
            native_index,
            streams: HashMap::new(),
            next_stream: 1,
            gpu_enabled: false,
            gpu_ready: Arc::new(AtomicBool::new(true)),
            #[cfg(test)]
            gpu_recording_fault: None,
            dev_shaders: None,
            resources: resources::Registration::new(native_id, native_index),
        }
    }

    fn with_gpu(mut self, enabled: bool) -> Self {
        self.gpu_enabled = enabled;
        self
    }
}

fn text_ok(text: &str) -> Result<(), w::HostError> {
    if text.len() > abi::MAX_TEXT_BYTES {
        Err(w::HostError::Invalid)
    } else {
        Ok(())
    }
}

fn finite(values: &[f32]) -> Result<(), w::HostError> {
    if values.iter().all(|value| value.is_finite()) {
        Ok(())
    } else {
        Err(w::HostError::Invalid)
    }
}

fn rect_values(rect: &w::Rect) -> [f32; 4] {
    [rect.x, rect.y, rect.width, rect.height]
}

fn batch_range<'borrow>(
    invocation: &'borrow mut Invocation<'_>,
    start: u64,
) -> Result<(&'borrow mut HudBatch, usize), w::HostError> {
    let Some(batch) = invocation.batch.as_deref_mut() else {
        return Err(w::HostError::Invalid);
    };
    let start = usize::try_from(start)
        .ok()
        .and_then(|start| invocation.batch_start.checked_add(start))
        .filter(|start| *start <= batch.vertices.len())
        .ok_or(w::HostError::Invalid)?;
    Ok((batch, start))
}

impl ClientHost for GameHost {
    fn services(&mut self) -> Result<yoake_wasm::package_services::ServiceHandle, String> {
        package::services(&self.id, horizon_mods::yak::ConfigRuntimeSide::Client)
    }
    fn release_resources(&mut self) {
        for texture in self.textures.keys() {
            client_services::unregister_named_texture(&client_services::named_texture_id(&self.id, *texture));
        }
        self.textures.clear();
        self.resources
            .release(self.streams.drain().map(|(_, stream)| stream).collect());
        resources::drain();
    }

    fn client_services(&mut self) -> Option<&mut dyn yoake_wasm::ClientServices> {
        Some(self)
    }

    fn gpu_services(&mut self) -> Option<&mut dyn yoake_wasm::GpuServices> {
        if self.gpu_enabled { Some(self) } else { None }
    }
    fn log(&mut self, target: &str, message: &str) -> Result<(), w::HostError> {
        let clean = |text: &str| -> String {
            text.chars()
                .take(MAX_LOG_CHARS)
                .map(|c| if c.is_control() { ' ' } else { c })
                .collect()
        };
        emit(format!(
            "wasm mod {} [{}]: {}",
            self.id,
            clean(target),
            clean(message)
        ));
        Ok(())
    }

    fn feedback(&mut self, text: &str) -> Result<(), w::HostError> {
        text_ok(text)?;
        service(|invocation| {
            invocation
                .host
                .write()
                .ok_or(w::HostError::Unsupported)?
                .feedback(Text::literal(text));
            Ok(())
        })
    }

    fn text(
        &mut self,
        text: &str,
        at: w::Point,
        rgb: u32,
        shadow: bool,
    ) -> Result<(), w::HostError> {
        text_ok(text)?;
        finite(&[at.x, at.y])?;
        service(|invocation| {
            let batch = invocation
                .batch
                .as_deref_mut()
                .ok_or(w::HostError::Invalid)?;
            invocation
                .host
                .read()
                .ok_or(w::HostError::Unsupported)?
                .text(batch, text, [at.x, at.y], rgb, shadow);
            Ok(())
        })
    }

    fn fill(&mut self, bounds: w::Rect, argb: u32) -> Result<(), w::HostError> {
        let values = rect_values(&bounds);
        finite(&values)?;
        service(|invocation| {
            let batch = invocation
                .batch
                .as_deref_mut()
                .ok_or(w::HostError::Invalid)?;
            invocation
                .host
                .read()
                .ok_or(w::HostError::Unsupported)?
                .fill(batch, values, argb);
            Ok(())
        })
    }

    fn text_width(&mut self, text: &str) -> Result<Option<f32>, w::HostError> {
        text_ok(text)?;
        service(|invocation| {
            Ok(invocation
                .host
                .read()
                .ok_or(w::HostError::Unsupported)?
                .text_width(text))
        })
    }

    fn batch_len(&mut self) -> Result<u64, w::HostError> {
        service(|invocation| {
            let batch = invocation.batch.as_deref().ok_or(w::HostError::Invalid)?;
            Ok((batch.vertices.len() - invocation.batch_start) as u64)
        })
    }

    fn batch_scale_since(
        &mut self,
        start: u64,
        origin: w::Point,
        scale: w::Point,
    ) -> Result<(), w::HostError> {
        finite(&[origin.x, origin.y, scale.x, scale.y])?;
        service(|invocation| {
            let (batch, start) = batch_range(invocation, start)?;
            batch.scale_since(start, origin.x, origin.y, scale.x, scale.y);
            Ok(())
        })
    }

    fn batch_clip_since(&mut self, start: u64, bounds: w::Rect) -> Result<(), w::HostError> {
        let values = rect_values(&bounds);
        finite(&values)?;
        service(|invocation| {
            let (batch, start) = batch_range(invocation, start)?;
            batch.clip_since(
                start,
                bounds.x,
                bounds.y,
                bounds.x + bounds.width,
                bounds.y + bounds.height,
            );
            Ok(())
        })
    }

    fn batch_tint_since(&mut self, start: u64, alpha: f32) -> Result<(), w::HostError> {
        if !alpha.is_finite() || !(0.0..=1.0).contains(&alpha) {
            return Err(w::HostError::Invalid);
        }
        service(|invocation| {
            let (batch, start) = batch_range(invocation, start)?;
            for vertex in &mut batch.vertices[start..] {
                let channel = ((vertex.color >> 24) as f32 * alpha).round() as u32;
                vertex.color = (vertex.color & 0x00ff_ffff) | (channel << 24);
            }
            Ok(())
        })
    }

    fn gui_texture_create(
        &mut self,
        width: u32,
        height: u32,
        format: w::TextureFormat,
        filter: w::TextureFilter,
    ) -> Result<u64, w::HostError> {
        if width == 0
            || height == 0
            || width > 16384
            || height > 16384
            || self.textures.len() >= 256
        {
            return Err(w::HostError::Invalid);
        }
        let format = match format {
            w::TextureFormat::Rgba8 => wgpu::TextureFormat::Rgba8Unorm,
            w::TextureFormat::Bgra8 => wgpu::TextureFormat::Bgra8Unorm,
            w::TextureFormat::Rgba16Float => wgpu::TextureFormat::Rgba16Float,
        };
        let filter = match filter {
            w::TextureFilter::Nearest => NativeGuiTextureFilter::Nearest,
            w::TextureFilter::Linear => NativeGuiTextureFilter::Linear,
        };
        let id = self.next_texture;
        let next = id.checked_add(1).ok_or(w::HostError::Disabled)?;
        let texture = self
            .client_call(|invocation| {
                invocation
                    .host
                    .read()
                    .ok_or_else(|| "client host unavailable".to_owned())?
                    .gui_texture_create([width, height], format, filter)
                    .map_err(|error| format!("{error:?}"))
            })
            .map_err(|_| w::HostError::HostError)?;
        self.next_texture = next;
        self.textures.insert(id, texture);
        Ok(id)
    }

    fn gui_texture_draw(
        &mut self,
        texture: u64,
        bounds: w::Rect,
        uv: w::Rect,
        argb: u32,
        nine_slice: Option<w::GuiNineSlice>,
    ) -> Result<bool, w::HostError> {
        let (rect, uv) = (rect_values(&bounds), rect_values(&uv));
        finite(&rect)?;
        finite(&uv)?;
        let nine = nine_slice.map(|nine| NativeGuiNineSlice {
            source_size: [nine.source_size.0, nine.source_size.1],
            border: [nine.border.0, nine.border.1, nine.border.2, nine.border.3],
        });
        let texture = self.textures.get(&texture).ok_or(w::HostError::NotFound)?;
        service(|invocation| {
            let batch = invocation
                .batch
                .as_deref_mut()
                .ok_or(w::HostError::Invalid)?;
            Ok(invocation
                .host
                .read()
                .ok_or(w::HostError::Unsupported)?
                .gui_texture(batch, texture, rect, uv, argb, nine))
        })
    }

    fn gui_texture_drop(&mut self, texture: u64) {
        self.textures.remove(&texture);
        client_services::unregister_named_texture(&client_services::named_texture_id(&self.id, texture));
    }

    fn setting(&mut self, mod_id: &str, key: &str) -> Result<Option<String>, w::HostError> {
        if !safe_component(key) {
            return Err(w::HostError::Invalid);
        }
        service(|invocation| {
            Ok(invocation
                .host
                .read()
                .ok_or(w::HostError::Unsupported)?
                .setting(mod_id, key))
        })
    }

    fn set_setting(&mut self, mod_id: &str, key: &str, value: &str) -> Result<(), w::HostError> {
        if !safe_component(key) {
            return Err(w::HostError::Invalid);
        }
        text_ok(value)?;
        service(|invocation| {
            invocation
                .host
                .write()
                .ok_or(w::HostError::Unsupported)?
                .set_setting(mod_id, key, value);
            Ok(())
        })
    }

    fn client_world_key(&mut self) -> Result<Option<w::WorldKey>, w::HostError> {
        service(|invocation| {
            let host = invocation.host.read().ok_or(w::HostError::Unsupported)?;
            Ok(match host.client_world_key() {
                None => None,
                Some(NativeClientWorld::Singleplayer { folder }) => {
                    Some(w::WorldKey::Singleplayer(folder))
                }
                Some(NativeClientWorld::Server { address }) => Some(w::WorldKey::Server(address)),
                Some(NativeClientWorld::LanGuest { address }) => Some(w::WorldKey::Lan(address)),
                Some(NativeClientWorld::Friend) => Some(w::WorldKey::Friend),
                Some(NativeClientWorld::Replay { title }) => Some(w::WorldKey::Replay(title)),
                #[allow(unreachable_patterns)]
                Some(_) => return Err(w::HostError::Unsupported),
            })
        })
    }

    fn snapshot_client_world(&mut self) -> Result<w::WorldSnapshot, w::HostError> {
        service(|invocation| {
            let host = invocation.host.write().ok_or(w::HostError::Unsupported)?;
            let mut frames = Vec::new();
            let mut oversized = false;
            let report = host.snapshot_client_world(&mut |state, packet_id, bytes| {
                if bytes.len() > abi::MAX_FRAME_BYTES {
                    oversized = true;
                    return;
                }
                frames.push(w::WorldFrame {
                    state: net_state(state),
                    packet_id,
                    body: bytes.to_vec(),
                });
            });
            if oversized || report.missing.len() > abi::MAX_SETTINGS_BUTTONS {
                return Err(w::HostError::Invalid);
            }
            Ok(w::WorldSnapshot {
                frames,
                report: w::SnapshotReport {
                    written: report.written as u64,
                    missing: report.missing,
                },
            })
        })
    }

    fn playing(&mut self) -> Result<bool, w::HostError> {
        service(|invocation| {
            Ok(invocation
                .host
                .read()
                .ok_or(w::HostError::Unsupported)?
                .playing())
        })
    }

    fn local_player_snapshot(&mut self) -> Result<Option<w::PlayerSnapshot>, w::HostError> {
        service(|invocation| {
            let host = invocation.host.read().ok_or(w::HostError::Unsupported)?;
            let Some(player) = host.local_player_snapshot() else {
                return Ok(None);
            };
            let hand = |hand: Hand| {
                if matches!(hand, Hand::Main) {
                    abi::HAND_MAIN
                } else {
                    abi::HAND_OFF
                }
            };
            let equipment = player
                .equipment
                .iter()
                .map(|stack| {
                    let mut writer = Writer::new();
                    Slot::write(stack, &mut writer);
                    writer.data
                })
                .collect();
            let flags = [
                (player.on_ground, abi::PLAYER_ON_GROUND),
                (player.sprinting, abi::PLAYER_SPRINTING),
                (player.swimming, abi::PLAYER_SWIMMING),
                (player.fall_flying, abi::PLAYER_FALL_FLYING),
                (player.vehicle.is_some(), abi::PLAYER_HAS_VEHICLE),
                (player.main_hand_right, abi::PLAYER_MAIN_HAND_RIGHT),
                (player.in_water, abi::PLAYER_IN_WATER),
                (player.eye_in_water, abi::PLAYER_EYE_IN_WATER),
                (player.flying, abi::PLAYER_FLYING),
                (player.max_health.is_some(), abi::PLAYER_HAS_MAX_HEALTH),
            ]
            .into_iter()
            .fold(0, |flags, (enabled, flag)| {
                flags | if enabled { flag } else { 0 }
            });
            Ok(Some(w::PlayerSnapshot {
                entity_id: player.entity_id,
                uuid: player.uuid.to_be_bytes().to_vec(),
                pos: (player.pos[0], player.pos[1], player.pos[2]),
                yaw: player.yaw,
                pitch: player.pitch,
                head_yaw: player.head_yaw,
                body_yaw: player.body_yaw,
                flags,
                pose: player.pose.id() as u32,
                swing_hand: player
                    .swing
                    .map_or(abi::HAND_NONE, |(value, _)| hand(value)),
                swing_ticks: player.swing.map_or(0, |(_, ticks)| ticks),
                using_hand: player.using_item.map_or(abi::HAND_NONE, hand),
                use_ticks: player.use_ticks,
                equipment,
                vehicle: player.vehicle.unwrap_or_default(),
                skin_parts: u32::from(player.skin_parts),
                dimension: player.dimension.to_string(),
                game_mode: u32::from(player.game_mode),
                health: player.health,
                max_health: player.max_health.unwrap_or_default(),
                food_level: player.food_level,
            }))
        })
    }

    fn instance_dir(&mut self) -> Result<String, w::HostError> {
        service(|invocation| {
            let path = invocation
                .host
                .read()
                .ok_or(w::HostError::Unsupported)?
                .instance_dir();
            path.to_str()
                .map(str::to_owned)
                .ok_or(w::HostError::Unsupported)
        })
    }

    fn account(&mut self) -> Result<Option<w::AccountSnapshot>, w::HostError> {
        service(|invocation| {
            let host = invocation.host.read().ok_or(w::HostError::Unsupported)?;
            Ok(host.account().map(|account| w::AccountSnapshot {
                name: account.name,
                uuid: account.uuid.to_be_bytes().to_vec(),
                online: account.online,
            }))
        })
    }

    fn mod_screen_open(&mut self, mod_id: &str) -> Result<bool, w::HostError> {
        let opened = service(|invocation| {
            Ok(invocation
                .host
                .write()
                .ok_or(w::HostError::Unsupported)?
                .mod_screen_open(mod_id))
        })?;
        if opened {
            self.resources.claim_screen();
        }
        Ok(opened)
    }

    fn mod_screen_close(&mut self, mod_id: &str) -> Result<(), w::HostError> {
        service(|invocation| {
            invocation
                .host
                .write()
                .ok_or(w::HostError::Unsupported)?
                .mod_screen_close(mod_id);
            Ok(())
        })?;
        self.resources.close_screen();
        Ok(())
    }

    fn mod_screen(&mut self) -> Result<Option<String>, w::HostError> {
        service(|invocation| {
            Ok(invocation
                .host
                .read()
                .ok_or(w::HostError::Unsupported)?
                .mod_screen()
                .map(str::to_owned))
        })
    }

    fn mod_screen_open_anywhere(&mut self, mod_id: &str) -> Result<bool, w::HostError> {
        let opened = service(|invocation| {
            Ok(invocation
                .host
                .write()
                .ok_or(w::HostError::Unsupported)?
                .mod_screen_open_anywhere(mod_id))
        })?;
        if opened {
            self.resources.claim_screen();
        }
        Ok(opened)
    }

    fn net_send_to_server(&mut self, channel: &str, data: &[u8]) -> Result<(), w::SendError> {
        let refuse = |status| w::SendError {
            status,
            message: String::new(),
        };
        if channel.len() > 256 || data.len() > abi::MAX_DATA_BYTES {
            return Err(refuse(w::HostError::Invalid));
        }
        service(|invocation| {
            // A hook with a shared view of the game (the screen key and mouse hooks) sends through
            // the shared path: the same checks, caps and queue.
            if let Some(host) = invocation.host.write() {
                return Ok(host.net_send_to_server(channel, data));
            }
            let host = invocation.host.read().ok_or(w::HostError::Unsupported)?;
            Ok(host.net_send_to_server_shared(channel, data))
        })
        .map_err(refuse)?
        .map_err(|message| w::SendError {
            status: w::HostError::HostError,
            message,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::cell::RefCell;

    use yoake_wasm::test_guests::{Guest, HELLO, HELLO_STD, MIXIN_DEMO};

    #[test]
    fn inject_1_1_loads_without_package_services() {
        let _site_claim_guard = crate::inject_test_support::site_claim_guard();
        const CHILD_PROCESS: &str = "YOAKE_WASM_INJECTION_TEST_CHILD";
        if std::env::var_os(CHILD_PROCESS).is_none() {
            let module = module_path!()
                .split_once("::")
                .map_or(module_path!(), |(_, module)| module);
            let name = format!("{module}::inject_1_1_loads_without_package_services");
            let mut command = std::process::Command::new(std::env::current_exe().unwrap());
            command.args(["--exact", &name, "--nocapture"]);
            command.env(CHILD_PROCESS, "1");
            #[cfg(windows)]
            {
                use std::os::windows::process::CommandExt;
                const CREATE_NO_WINDOW: u32 = 0x0800_0000;
                command.creation_flags(CREATE_NO_WINDOW);
            }
            let output = command.output().expect("run isolated injection test");
            assert!(
                output.status.success()
                    && String::from_utf8_lossy(&output.stdout).contains("1 passed;"),
                "isolated injection test failed:\n{}\n{}",
                String::from_utf8_lossy(&output.stdout),
                String::from_utf8_lossy(&output.stderr),
            );
            return;
        }
        let Some(bytes) = yoake_wasm::test_guests::load(&MIXIN_DEMO) else {
            return;
        };
        let targets = [
            "world::living_fall::calculate_fall_damage#9a9a629c2a289ab6ffb883cb0cd5891af74587992a6e2da94d8090723ede5b94",
            "world::level::bonemeal::crop_bonemeal_increase#b61d671fb9375f084f2bf026f466add0e8423efd996180dee3fb52ca18348fd1",
            "game::chat::handle_chat_input#2d6b616020cec7006f8b8c6aeb2d8f9faf0caa7a8dbeb7f38b5a4d839b0d2e62",
        ];
        let target = yoake_inject_rt::catalog::SITES
            .iter()
            .find(|site| site.id() == targets[0])
            .unwrap();
        let injection = load_injection_with_services(
            "mixin-demo",
            &bytes,
            &targets.map(str::to_owned),
            yoake_inject_rt::Side::Server,
            &["service.commands".to_owned(), "service.chat".to_owned()],
            Err("package services unavailable in test".to_owned()),
        )
        .unwrap();
        assert!(!injection.has_config_notifications());
        let _side = yoake_inject_rt::enter_side(yoake_inject_rt::Side::Server);
        let damage = || {
            target.invoke(
                vec![
                    yoake_inject_rt::Scope::default().handle("world", None),
                    yoake_inject_rt::Value::Float(12.0),
                    yoake_inject_rt::Value::Float(1.0),
                ],
                |_| yoake_inject_rt::Value::Signed(9),
            )
        };
        assert_eq!(damage(), yoake_inject_rt::Value::Signed(4));
        drop(injection);
        assert_eq!(damage(), yoake_inject_rt::Value::Signed(9));
    }

    #[test]
    fn gpu_package_fingerprint_changes_with_version_or_module() {
        let original = gpu_fingerprint("1.0.0", b"module-one");
        assert_eq!(original, gpu_fingerprint("1.0.0", b"module-one"));
        assert_ne!(original, gpu_fingerprint("1.0.1", b"module-one"));
        assert_ne!(original, gpu_fingerprint("1.0.0", b"module-two"));
        assert_ne!(gpu_fingerprint("ab", b"c"), gpu_fingerprint("a", b"bc"));
    }

    #[derive(Default)]
    struct FakeHost {
        texts: RefCell<Vec<String>>,
        fills: RefCell<u32>,
    }

    impl Host for FakeHost {
        fn window(&self) -> bool {
            true
        }

        fn text(&self, _batch: &mut HudBatch, text: &str, _at: [f32; 2], _rgb: u32, _shadow: bool) {
            self.texts.borrow_mut().push(text.to_owned());
        }

        fn fill(&self, _batch: &mut HudBatch, _rect: [f32; 4], _argb: u32) {
            *self.fills.borrow_mut() += 1;
        }
    }

    fn load(guest: &Guest, id: &str, reload: Option<Arc<Reload>>) -> Option<WasmMod> {
        let bytes = yoake_wasm::test_guests::load(guest)?;
        let module = {
            let mut sandbox = sandbox().lock().unwrap();
            ClientMod::instantiate(&mut sandbox, id, &bytes, Box::new(GameHost::new(id)))
                .expect("loads")
        };
        Some(WasmMod::new(id.into(), "1.0.0".into(), module, reload))
    }

    fn hud(module: &WasmMod, host: &mut FakeHost) {
        let mut batch = HudBatch::new([854, 480]);
        let mut corners = Corners::new(854.0, 480.0, 0.0);
        module.hud(
            host,
            &mut HudEvent {
                batch: &mut batch,
                corners: &mut corners,
            },
        );
    }

    #[test]
    fn wasm_without_mods_keeps_the_original_hooks_and_identities() {
        let mods: Mods = Arc::from(Vec::<Box<dyn Hooks>>::new());
        let identities: &'static [super::super::NativeMod] = &[];
        let (result, result_ids) = extend(mods.clone(), identities);
        assert!(Arc::ptr_eq(&mods, &result));
        assert!(std::ptr::eq(identities, result_ids));
    }

    #[test]
    fn wasm_dev_directory_is_not_read_without_wasm_developer_mode() {
        let switches = crate::yak_mods::UnsignedSwitches::from_values(None, None);
        let reserved = HashSet::new();
        let disabled = dev_sources_with(
            switches.wasm,
            || panic!("disabled WASM developer mode read the path"),
            &reserved,
        );
        assert!(disabled.is_empty());
    }

    #[test]
    fn wasm_dev_directory_loads_raw_modules_when_wasm_developer_mode_is_enabled() {
        let stamp = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        let dir =
            std::env::temp_dir().join(format!("yoake-wasm-dev-{}-{stamp}", std::process::id()));
        std::fs::create_dir(&dir).unwrap();
        let module = dir.join("hello.wasm");
        std::fs::write(&module, b"raw wasm dev fixture").unwrap();

        let switches =
            crate::yak_mods::UnsignedSwitches::from_values(None, Some(std::ffi::OsStr::new("1")));
        let reserved = HashSet::new();
        let sources = dev_sources_with(
            switches.wasm,
            || Some(dir.clone().into_os_string()),
            &reserved,
        );
        assert_eq!(sources.len(), 1);
        assert_eq!(sources[0].id, "dev-hello");
        assert_eq!(sources[0].bytes, b"raw wasm dev fixture");
        assert_eq!(sources[0].watch.as_deref(), Some(module.as_path()));

        crate::test_support::remove_dir_all(dir);
    }

    #[test]
    fn wasm_dev_directory_cannot_reuse_an_installed_mod_id() {
        let stamp = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        let dir = std::env::temp_dir().join(format!(
            "yoake-wasm-dev-collision-{}-{stamp}",
            std::process::id()
        ));
        std::fs::create_dir(&dir).unwrap();
        std::fs::write(dir.join("hello.wasm"), b"raw wasm dev fixture").unwrap();
        let reserved = HashSet::from(["dev-hello".to_owned()]);
        let sources = dev_sources_with(true, || Some(dir.clone().into_os_string()), &reserved);
        assert!(sources.is_empty());
        crate::test_support::remove_dir_all(dir);
    }

    #[test]
    fn wasm_dev_reads_and_watches_only_regular_files() {
        let stamp = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        let dir =
            std::env::temp_dir().join(format!("yoake-wasm-regular-{}-{stamp}", std::process::id()));
        std::fs::create_dir(&dir).unwrap();
        let file = dir.join("module.wasm");
        std::fs::write(&file, b"regular wasm file").unwrap();
        assert_eq!(read_bounded(&file).unwrap(), b"regular wasm file");
        assert!(read_bounded(&dir).unwrap_err().contains("regular file"));
        assert!(Watch::stat(&file).is_some());
        assert!(Watch::stat(&dir).is_none());

        #[cfg(unix)]
        {
            use std::os::unix::fs::symlink;
            let link = dir.join("linked.wasm");
            symlink(&file, &link).unwrap();
            assert!(read_bounded(&link).unwrap_err().contains("regular file"));
            assert!(Watch::stat(&link).is_none());
        }

        crate::test_support::remove_dir_all(dir);
    }

    #[test]
    fn unsigned_hot_reload_does_not_pin_a_required_join_digest() {
        let module = crate::yak_mods::WasmMod {
            mod_id: "sample_mod".into(),
            version: horizon_mods::yak::Version::parse("1.2.3").unwrap(),
            sha256: "ab".repeat(32),
            bytes: b"wasm fixture".to_vec(),
            side: crate::yak_mods::WasmSide::Client,
            full: false,
            required_on_join: true,
            commands: false,
            gpu: false,
            path: PathBuf::from("sample.wasm"),
            signed: false,
            mixin_targets: Vec::new(),
            permissions: Vec::new(),
            host_filesystem: false,
            worker: None,
        };
        let source = installed_source(module, true);
        assert!(!source.required_on_join);
        assert!(source.sha256.is_some());
        assert!(source.watch.is_some());
    }

    #[test]
    fn wasm_services_refuse_calls_made_outside_a_hook() {
        let mut host = GameHost::new("x");
        assert_eq!(host.feedback("hi"), Err(w::HostError::Unsupported));
        assert_eq!(host.playing(), Err(w::HostError::Unsupported));
        assert_eq!(host.setting("x", "../escape"), Err(w::HostError::Invalid));
        assert!(host.log("t", "line\nbreak").is_ok());
    }

    #[test]
    fn wasm_hello_draws_through_the_hud_hook_slot() {
        let _serial = GUEST_TEST_LOCK
            .lock()
            .unwrap_or_else(|error| error.into_inner());
        let Some(module) = load(&HELLO, "hello", None) else {
            eprintln!("skipped: hello.wasm is not built");
            return;
        };
        let mut host = FakeHost::default();
        let mut tick = TickEvent {};
        module.tick(&mut host, &mut tick);
        module.tick(&mut host, &mut tick);
        // Unsubscribed hooks never reach the guest.
        module.key(
            &mut host,
            &mut KeyEvent {
                key: "K",
                pressed: true,
            },
        );
        hud(&module, &mut host);
        assert_eq!(*host.texts.borrow(), ["wasm-hello #2"]);
        assert_eq!(*host.fills.borrow(), 1);
    }

    #[test]
    fn wasm_hot_reload_swaps_the_instance_at_the_next_tick() {
        let _serial = GUEST_TEST_LOCK
            .lock()
            .unwrap_or_else(|error| error.into_inner());
        let reload = Arc::new(Reload::default());
        let Some(module) = load(&HELLO, "hello", Some(Arc::clone(&reload))) else {
            eprintln!("skipped: hello.wasm is not built");
            return;
        };
        let Some(replacement) = yoake_wasm::test_guests::load(&HELLO_STD) else {
            eprintln!("skipped: hello_std.wasm is not built");
            return;
        };
        *reload.bytes.lock().unwrap() = Some(replacement);
        reload.ready.store(true, Ordering::Release);
        let mut host = FakeHost::default();
        module.tick(&mut host, &mut TickEvent {});
        hud(&module, &mut host);
        let texts = host.texts.borrow();
        assert_eq!(texts.len(), 1, "{texts:?}");
        assert!(texts[0].starts_with("wasm-hello-std #1"), "{texts:?}");
    }
}
