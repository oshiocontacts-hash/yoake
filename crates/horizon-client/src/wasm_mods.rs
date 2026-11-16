//! Sandboxed WebAssembly client mods of installed `.yak` files (`docs/yoake-wasm.md`).
//!
//! Every accepted module is instantiated once at startup and registered in the same hook slots as a
//! DLL mod (`dll_mods.rs`, which this mirrors), after the DLL mods. Without a wasm mod nothing in here
//! runs: no Engine, no thread, no wrapper around the existing hooks. A trap (or fuel/time/memory limit)
//! disables only that mod, with one log line.
//!
//! Developer mode (`YOAKE_DLL_MODS_UNSIGNED=1`) additionally watches unsigned local wasm files and,
//! when one changes on disk, swaps in a fresh instance at the next frame or tick on the main thread.
//! `YOAKE_WASM_DEV_DIR` names a folder of raw `<id>.wasm` components that are loaded the same way.

#[path = "wasm_mods/client_services.rs"]
mod client_services;
#[path = "wasm_mods/dev_shaders.rs"]
mod dev_shaders;
#[path = "wasm_mods/full.rs"]
mod full;
#[path = "wasm_mods/gpu_services.rs"]
mod gpu_services;
#[path = "wasm_mods/resources.rs"]
mod resources;
#[path = "wasm_mods/server_services.rs"]
mod server_services;

use super::dll_mods::{StaticMod, frame_time, native_identity, safe_component};
use crate::mod_api::*;
use horizon_mod_abi as abi;
use horizon_protocol::{
    buf::{Codec, Writer},
    item::Slot,
};
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
use yoake_wasm::{ClientHost, ClientMod, Sandbox};

const MAX_MODS: usize = 128;
const MAX_LOG_CHARS: usize = 4096;
const POLL_INTERVAL: Duration = Duration::from_secs(1);

/// Lines produced before the game's logger exists are kept and written by [`log_startup`].
static STARTUP: Mutex<Option<Vec<String>>> = Mutex::new(Some(Vec::new()));
static SANDBOX: OnceLock<Mutex<Sandbox>> = OnceLock::new();

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
    gpu: bool,
    bytes: Vec<u8>,
    /// Watched for hot reload (developer mode and not signed).
    watch: Option<PathBuf>,
}

fn dev_sources() -> Vec<Source> {
    let Some(dir) = std::env::var_os("YOAKE_WASM_DEV_DIR").filter(|dir| !dir.is_empty()) else {
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
        let Some(id) = path
            .file_stem()
            .and_then(|stem| stem.to_str())
            .map(str::to_owned)
        else {
            continue;
        };
        if !safe_component(&id) {
            emit(format!(
                "wasm: dev module {} skipped: unsafe id",
                path.display()
            ));
            continue;
        }
        match read_bounded(&path) {
            Ok(bytes) => sources.push(Source {
                id: id.replace('.', "-"),
                version: "0.0.0-dev".into(),
                sha256: None,
                required_on_join: false,
                full,
                gpu,
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
    let file = std::fs::File::open(path).map_err(|error| error.to_string())?;
    let mut bytes = Vec::new();
    file.take(horizon_mods::yak::MAX_WASM_SIZE + 1)
        .read_to_end(&mut bytes)
        .map_err(|error| error.to_string())?;
    if bytes.len() as u64 > horizon_mods::yak::MAX_WASM_SIZE {
        return Err("module exceeds 32 MiB".into());
    }
    Ok(bytes)
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
    let developer = crate::yak_mods::unsigned_allowed();
    // A dedicated server has no client: its client modules only give the package's identity.
    let dedicated = crate::yak_mods::dedicated();
    let mut sources: Vec<Source> = crate::yak_mods::wasm_client_mods()
        .into_iter()
        .map(|module| Source {
            id: module.mod_id,
            version: module.version.to_string(),
            sha256: Some(module.sha256),
            required_on_join: module.required_on_join,
            full: module.full,
            gpu: module.gpu,
            bytes: module.bytes,
            watch: (developer && !module.signed).then_some(module.path),
        })
        .collect();
    if developer {
        sources.extend(dev_sources());
    }
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
        if source.full {
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
                server_full: None,
                server_source: None,
                worlds: Vec::new(),
                gpu: false,
            },
        };
        let source = full::ServerSource::new(&server.mod_id, server.bytes.clone(), server.commands);
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
        // A package that only had a server module and lost it has nothing to run.
        if position.is_some() || package.server_full.is_some() {
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
        let metadata = std::fs::metadata(path).ok()?;
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
    fn release_resources(&mut self) {
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
            let host = invocation.host.write().ok_or(w::HostError::Unsupported)?;
            Ok(host.net_send_to_server(channel, data))
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

    use yoake_wasm::test_guests::{Guest, HELLO, HELLO_STD};

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
    fn wasm_services_refuse_calls_made_outside_a_hook() {
        let mut host = GameHost::new("x");
        assert_eq!(host.feedback("hi"), Err(w::HostError::Unsupported));
        assert_eq!(host.playing(), Err(w::HostError::Unsupported));
        assert_eq!(host.setting("x", "../escape"), Err(w::HostError::Invalid));
        assert!(host.log("t", "line\nbreak").is_ok());
    }

    #[test]
    fn wasm_hello_draws_through_the_hud_hook_slot() {
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
