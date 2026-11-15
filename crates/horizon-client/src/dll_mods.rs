//! Startup-only, trusted native libraries. No DLL means no dispatch wrappers.
//! DLL memory must obey horizon_mod_abi's contract; this is not a sandbox.

use crate::mod_api::*;
use horizon_mod_abi as abi;
use horizon_protocol::{buf::{Codec, Writer}, item::Slot};
use std::{
    cell::Cell,
    ffi::{OsStr, c_void},
    marker::PhantomData,
    path::{Path, PathBuf},
    rc::Rc,
    sync::{Arc, Mutex, OnceLock, TryLockError, atomic::{AtomicU64, Ordering}},
    time::Instant,
};

const MAX_MODS: usize = 128;
const HUD_BIT: u64 = 1 << 63;
static GENERATION: AtomicU64 = AtomicU64::new(1);

#[derive(Debug, PartialEq, Eq)]
enum LoadError {
    Unsigned,
    #[cfg_attr(any(windows, unix), allow(dead_code))]
    Platform,
    MissingEntry,
    Invalid(&'static str),
    Io(String),
}

fn verify(_path: &Path, allow_unsigned: bool) -> Result<(), LoadError> {
    if allow_unsigned { Ok(()) } else { Err(LoadError::Unsigned) }
}

struct Metadata {
    id: String,
    version: String,
    required_digest: Option<String>,
    hooks: abi::HookTableV1,
    create: unsafe extern "C" fn(*const abi::HostTableV1, abi::Handle, *mut *mut c_void) -> abi::Status,
    destroy: unsafe extern "C" fn(*mut c_void) -> abi::Status,
}

fn entry_address(entry: Option<abi::ModEntry>) -> Result<abi::ModEntry, LoadError> {
    entry.ok_or(LoadError::MissingEntry)
}

unsafe fn identity(value: abi::StrView) -> Result<String, LoadError> {
    let value = unsafe { value.as_str(256) }.map_err(|_| LoadError::Invalid("identity UTF-8"))?;
    if value.is_empty() || value.chars().count() > 64 || value.chars().any(char::is_control) {
        return Err(LoadError::Invalid("identity length or control character"));
    }
    Ok(value.to_owned())
}

/// The entry promises live, immutable, readable descriptor and hook-table memory.
unsafe fn descriptor(pointer: *const abi::ModDescriptor) -> Result<Metadata, LoadError> {
    if pointer.is_null() || !pointer.is_aligned() {
        return Err(LoadError::Invalid("descriptor pointer"));
    }
    let header = unsafe { pointer.cast::<abi::TableHeader>().read() };
    if !header.valid_for::<abi::ModDescriptor>() {
        return Err(LoadError::Invalid("descriptor ABI version, schema or size"));
    }
    let value = unsafe { pointer.read() };
    if value.build_fingerprint != abi::BUILD_FINGERPRINT {
        return Err(LoadError::Invalid("engine build fingerprint"));
    }
    for (actual, expected, label) in [
        (value.yoake_version, abi::YOAKE_VERSION, "Yoake version"),
        (value.nightly, abi::YOAKE_NIGHTLY, "Yoake nightly"),
        (value.target, abi::TARGET, "target"),
    ] {
        if unsafe { actual.as_str(4096) }.map_err(|_| LoadError::Invalid(label))? != expected {
            return Err(LoadError::Invalid(label));
        }
    }
    if value.side != abi::SIDE_CLIENT || value.flags != 0 {
        return Err(LoadError::Invalid("side or flags"));
    }
    if value.required_services & !HOST_TABLE.services != 0 {
        return Err(LoadError::Invalid("required host services"));
    }
    if value.hooks.is_null() || !value.hooks.is_aligned() {
        return Err(LoadError::Invalid("hook table pointer"));
    }
    let header = unsafe { value.hooks.cast::<abi::TableHeader>().read() };
    if !header.valid_for::<abi::HookTableV1>() {
        return Err(LoadError::Invalid("hook table ABI version, schema or size"));
    }
    let id = unsafe { identity(value.id)? };
    if !safe_component(&id) {
        return Err(LoadError::Invalid("id must be a safe settings directory component"));
    }
    Ok(Metadata {
        id,
        version: unsafe { identity(value.version)? },
        required_digest: None,
        hooks: unsafe { value.hooks.read() },
        create: value.create.ok_or(LoadError::Invalid("missing create"))?,
        destroy: value.destroy.ok_or(LoadError::Invalid("missing destroy"))?,
    })
}

#[cfg(any(windows, unix))]
fn load(path: &Path, allow_unsigned: bool) -> Result<Metadata, LoadError> {
    load_symbol(path, allow_unsigned, b"yoake_mod_entry\0")
}

#[cfg(any(windows, unix))]
fn load_symbol(path: &Path, allow_unsigned: bool, symbol: &[u8]) -> Result<Metadata, LoadError> {
    verify(path, allow_unsigned)?;
    let library = unsafe { libloading::Library::new(path) }.map_err(|error| LoadError::Io(error.to_string()))?;
    let entry = unsafe { library.get::<abi::ModEntry>(symbol) }.ok().map(|symbol| *symbol);
    let entry = entry_address(entry)?;
    let metadata = unsafe { descriptor(entry(abi::ABI_VERSION, &HOST_TABLE))? };
    let _retained_until_exit = Box::leak(Box::new(library));
    Ok(metadata)
}

#[cfg(not(any(windows, unix)))]
fn load(path: &Path, allow_unsigned: bool) -> Result<Metadata, LoadError> {
    verify(path, allow_unsigned)?;
    Err(LoadError::Platform)
}

pub(super) fn extend(mods: Mods, identities: &'static [super::NativeMod]) -> (Mods, &'static [super::NativeMod]) {
    crate::yak_mods::initialize();
    let allow_unsigned = crate::yak_mods::UnsignedSwitches::from_env().dll;
    let (mods, identities) = extend_paths(mods, identities, std::env::var_os("YOAKE_DLL_MODS").as_deref(), allow_unsigned);
    extend_yaks(mods, identities, crate::yak_mods::entries(), allow_unsigned)
}

fn extend_yaks(mods: Mods, identities: &'static [super::NativeMod], entries: &[crate::yak_mods::Installed], allow_unsigned: bool) -> (Mods, &'static [super::NativeMod]) {
    if !entries.iter().any(|entry| entry.manifest.bin.is_some()) { return (mods, identities); }
    // A dedicated server has no client: native libraries are client code.
    if crate::yak_mods::dedicated() { return (mods, identities); }
    let build = match crate::yak_mods::Build::current() {
        Ok(build) => build,
        Err(reason) => {
            for entry in entries.iter().filter(|entry| entry.manifest.bin.is_some()) {
                crate::yak_mods::refuse_binary(entry, &reason);
            }
            return (mods, identities);
        }
    };
    let keys = horizon_mods::yak::trusted_keys();
    let mut names: std::collections::HashSet<String> = identities.iter().map(|identity| identity.id.to_owned()).collect();
    let mut accepted = Vec::new();
    for entry in entries.iter().filter(|entry| entry.manifest.bin.is_some()) {
        let result = (|| {
            if identities.len() + accepted.len() >= MAX_MODS || names.contains(&entry.manifest.id) {
                return Err("duplicate or too many native identities".to_owned());
            }
            let (path, digest) = crate::yak_mods::binary_preflight(entry, &build, &keys, allow_unsigned)?;
            let mut metadata = load(&path, true).map_err(|error| format!("{error:?}"))?;
            if metadata.id != entry.manifest.id || metadata.version != entry.manifest.version.to_string() {
                return Err("DLL descriptor identity differs from yak.toml".to_owned());
            }
            metadata.required_digest = entry.manifest.side.required_on_join.then_some(digest);
            names.insert(metadata.id.clone());
            Ok(metadata)
        })();
        match result {
            Ok(metadata) => accepted.push(metadata),
            Err(reason) => crate::yak_mods::refuse_binary(entry, &reason),
        }
    }
    append(mods, identities, accepted)
}

fn extend_paths(mods: Mods, identities: &'static [super::NativeMod], paths: Option<&OsStr>, allow_unsigned: bool) -> (Mods, &'static [super::NativeMod]) {
    let Some(paths) = paths.filter(|paths| !paths.is_empty()) else { return (mods, identities) };
    let mut accepted = Vec::new();
    let mut names: std::collections::HashSet<String> = identities.iter().map(|identity| identity.id.to_owned()).collect();
    let mut files = std::collections::HashSet::<PathBuf>::new();
    for path in std::env::split_paths(paths) {
        let result = (|| {
            if identities.len() + accepted.len() >= MAX_MODS {
                return Err(LoadError::Invalid("too many native identities"));
            }
            if !path.is_absolute() || !path.extension().and_then(OsStr::to_str).is_some_and(|extension| extension.eq_ignore_ascii_case("dll")) {
                return Err(LoadError::Invalid("DLL path must be absolute with .dll extension"));
            }
            verify(&path, allow_unsigned)?;
            let path = path.canonicalize().map_err(|error| LoadError::Io(error.to_string()))?;
            if !files.insert(path.clone()) {
                return Err(LoadError::Invalid("duplicate DLL path"));
            }
            if !cfg!(windows) {
                return Err(LoadError::Platform);
            }
            let metadata = load(&path, allow_unsigned)?;
            if !names.insert(metadata.id.clone()) {
                return Err(LoadError::Invalid("duplicate native mod id"));
            }
            Ok(metadata)
        })();
        match result {
            Ok(metadata) => accepted.push(metadata),
            Err(error) => eprintln!("DLL mod {} rejected: {error:?}", path.display()),
        }
    }
    append(mods, identities, accepted)
}

fn append(mods: Mods, identities: &'static [super::NativeMod], accepted: Vec<Metadata>) -> (Mods, &'static [super::NativeMod]) {
    if accepted.is_empty() {
        return (mods, identities);
    }
    let mut combined: Vec<Box<dyn Hooks>> = (0..mods.len()).map(|index| Box::new(StaticMod { original: mods.clone(), index }) as Box<dyn Hooks>).collect();
    let mut owned = identities.to_vec();
    for metadata in accepted {
        let id: &'static str = Box::leak(metadata.id.clone().into_boxed_str());
        let (version, side) = native_identity(&metadata.version, metadata.required_digest.as_deref());
        let version: &'static str = Box::leak(version.into_boxed_str());
        owned.push(super::NativeMod { id, version, side });
        combined.push(Box::new(DllMod::new(metadata, combined.len() as u64 + 1)));
    }
    (Arc::from(combined), Box::leak(owned.into_boxed_slice()))
}

/// The identity text is limited to 64 characters (`mod_list::MAX_TEXT`): a required package shows at
/// most this many characters of its version, then `#` and 32 hex digits (128 bits) of
/// `sha256(version, 0, digest)`, so the text fits for any version and still covers all of it.
const IDENTITY_VERSION_CHARS: usize = 31;

pub(super) fn native_identity(version: &str, digest: Option<&str>) -> (String, super::NativeSide) {
    match digest {
        Some(digest) => {
            use sha2::{Digest, Sha256};
            let mut hasher = Sha256::new();
            hasher.update(version.as_bytes());
            hasher.update([0]);
            hasher.update(digest.as_bytes());
            let tag: String = hasher.finalize().iter().take(16).map(|byte| format!("{byte:02x}")).collect();
            let shown: String = version.chars().take(IDENTITY_VERSION_CHARS).collect();
            (format!("{shown}#{tag}"), super::NativeSide::Both)
        }
        None => (version.chars().take(64).collect(), super::NativeSide::Client),
    }
}

pub(super) struct StaticMod {
    pub(super) original: Mods,
    pub(super) index: usize,
}

struct InstanceState {
    pointer: *mut c_void,
    disabled: bool,
    textures: Vec<(abi::Handle, NativeGuiTexture)>,
    next_texture: u64,
}

/// Opaque mod-owned state is Send by the SDK contract, and always mutex-serialized.
unsafe impl Send for InstanceState {}

struct DllMod {
    id: String,
    handle_id: u64,
    hooks: abi::HookTableV1,
    create: unsafe extern "C" fn(*const abi::HostTableV1, abi::Handle, *mut *mut c_void) -> abi::Status,
    destroy: unsafe extern "C" fn(*mut c_void) -> abi::Status,
    state: Mutex<InstanceState>,
}

impl DllMod {
    fn new(metadata: Metadata, handle_id: u64) -> Self {
        Self { id: metadata.id, handle_id, hooks: metadata.hooks, create: metadata.create, destroy: metadata.destroy, state: Mutex::new(InstanceState { pointer: std::ptr::null_mut(), disabled: false, textures: Vec::new(), next_texture: 1 }) }
    }

    fn invoke<'call>(&self, host: &'call mut dyn Host, batch: Option<&'call mut HudBatch>, callback: impl FnOnce(*mut c_void, abi::Handle) -> abi::HookResult) {
        self.invoke_with(HostAccess::Write(host), batch, false, callback);
    }

    fn invoke_with<'call>(&self, host: HostAccess<'call>, batch: Option<&'call mut HudBatch>, cancel: bool, callback: impl FnOnce(*mut c_void, abi::Handle) -> abi::HookResult) -> Flow {
        let mut state = match self.state.try_lock() {
            Ok(state) => state,
            Err(TryLockError::WouldBlock) => return Flow::Continue,
            Err(TryLockError::Poisoned(error)) => {
                error.into_inner().disabled = true;
                log::error!("DLL mod {} disabled: poisoned instance", self.id);
                return Flow::Continue;
            }
        };
        if state.disabled { return Flow::Continue; }
        let Ok(generation) = GENERATION.try_update(Ordering::Relaxed, Ordering::Relaxed, |generation| generation.checked_add(1)) else {
            state.disabled = true;
            log::error!("DLL mod {} disabled: handle generation exhausted", self.id);
            return Flow::Continue;
        };
        let handle = abi::Handle { id: self.handle_id, generation };
        let batch_start = batch.as_deref().map_or(0, |batch| batch.vertices.len());
        let InstanceState { pointer, textures, next_texture, .. } = &mut *state;
        let mut context = Invocation { host, batch, batch_start, id: &self.id, texture_owner: self.handle_id, textures, next_texture, faulted: false };
        let result = {
            let _scope = Scope::enter(&mut context, handle);
            if pointer.is_null() {
                let status = unsafe { (self.create)(&HOST_TABLE, handle, pointer) };
                if status != abi::Status::OK || pointer.is_null() {
                    abi::HookResult::failed(if status == abi::Status::OK { abi::Status::INVALID } else { status })
                } else {
                    callback(*pointer, handle)
                }
            } else {
                callback(*pointer, handle)
            }
        };
        let faulted = context.faulted;
        if faulted || result.status != abi::Status::OK || (result.flow != abi::FLOW_CONTINUE && !(cancel && result.flow == abi::FLOW_CANCEL)) {
            state.disabled = true;
            state.textures.clear();
            log::error!("DLL mod {} disabled: status={} flow={} host_fault={}", self.id, result.status.0, result.flow, faulted);
            return Flow::Continue;
        }
        if result.flow == abi::FLOW_CANCEL { Flow::Cancel } else { Flow::Continue }
    }
}

impl Drop for DllMod {
    fn drop(&mut self) {
        let state = self.state.get_mut().unwrap_or_else(|error| error.into_inner());
        if !state.pointer.is_null() {
            let status = unsafe { (self.destroy)(state.pointer) };
            state.pointer = std::ptr::null_mut();
            if status != abi::Status::OK { log::error!("DLL mod {} destroy failed: {}", self.id, status.0); }
        }
    }
}

enum HostAccess<'call> {
    None,
    Read(&'call dyn Host),
    Write(&'call mut dyn Host),
}

impl HostAccess<'_> {
    fn read(&self) -> Option<&dyn Host> {
        match self { Self::None => None, Self::Read(host) => Some(*host), Self::Write(host) => Some(&**host) }
    }

    fn write(&mut self) -> Option<&mut dyn Host> {
        match self { Self::Write(host) => Some(&mut **host), _ => None }
    }
}

/// One call into a DLL: the host and batch live for the hook (`'call`), the instance's own
/// bookkeeping only while its lock is held (`'state`).
struct Invocation<'call, 'state> {
    host: HostAccess<'call>,
    batch: Option<&'call mut HudBatch>,
    batch_start: usize,
    id: &'state str,
    texture_owner: u64,
    textures: &'state mut Vec<(abi::Handle, NativeGuiTexture)>,
    next_texture: &'state mut u64,
    faulted: bool,
}

#[derive(Clone, Copy)]
struct Active {
    pointer: *mut c_void,
    handle: abi::Handle,
    busy: bool,
}

thread_local! {
    static ACTIVE: Cell<Option<Active>> = const { Cell::new(None) };
}

struct Scope<'scope> {
    previous: Option<Active>,
    lifetime: PhantomData<&'scope mut ()>,
    thread: PhantomData<Rc<()>>,
}

impl<'scope> Scope<'scope> {
    fn enter(context: &'scope mut Invocation<'_, '_>, handle: abi::Handle) -> Self {
        let previous = ACTIVE.with(|active| active.replace(Some(Active { pointer: (context as *mut Invocation<'_, '_>).cast(), handle, busy: false })));
        Self { previous, lifetime: PhantomData, thread: PhantomData }
    }
}

impl Drop for Scope<'_> {
    fn drop(&mut self) { ACTIVE.with(|active| active.set(self.previous)); }
}

struct ServiceScope(Active);

impl Drop for ServiceScope {
    fn drop(&mut self) { ACTIVE.with(|active| active.set(Some(self.0))); }
}

fn caught<Value>(operation: impl FnOnce() -> Value) -> Result<Value, ()> {
    match std::panic::catch_unwind(std::panic::AssertUnwindSafe(operation)) {
        Ok(value) => Ok(value),
        Err(payload) => { std::mem::forget(payload); Err(()) }
    }
}

fn service(handle: abi::Handle, operation: impl FnOnce(&mut Invocation<'_, '_>) -> abi::Status) -> abi::Status {
    ACTIVE.with(|slot| {
        let Some(active) = slot.get() else { return abi::Status::INVALID };
        if active.handle != handle || !handle.is_valid() { return abi::Status::INVALID; }
        if active.busy { return abi::Status::BUSY; }
        let _scope = ServiceScope(active);
        slot.set(Some(Active { busy: true, ..active }));
        let context = unsafe { &mut *active.pointer.cast::<Invocation<'_, '_>>() };
        match caught(|| operation(context)) {
            Ok(status) => status,
            Err(()) => {
                context.faulted = true;
                let _ = caught(|| eprintln!("DLL mod {} disabled: host service panic", context.id));
                abi::Status::PANIC
            }
        }
    })
}

fn hud_handle(host: abi::Handle) -> abi::Handle { abi::Handle { id: host.id | HUD_BIT, generation: host.generation } }

unsafe fn output_buffer<'call>(output: *mut abi::OutputBuffer) -> Result<&'call mut abi::OutputBuffer, abi::Status> {
    if output.is_null() || !output.is_aligned() { return Err(abi::Status::INVALID); }
    let output = unsafe { &mut *output };
    if output.capacity > abi::MAX_OUTPUT_BYTES || (output.capacity != 0 && output.ptr.is_null()) {
        return Err(abi::Status::INVALID);
    }
    output.len = 0;
    Ok(output)
}

pub(super) fn safe_component(value: &str) -> bool {
    if value.is_empty() || value.len() > 64 || value.ends_with('.')
        || !value.bytes().all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'-' | b'_' | b'.'))
    {
        return false;
    }
    let stem = value.split('.').next().unwrap_or_default().to_ascii_uppercase();
    !matches!(stem.as_str(), "CON" | "PRN" | "AUX" | "NUL")
        && !(stem.len() == 4
            && (stem.starts_with("COM") || stem.starts_with("LPT"))
            && matches!(stem.as_bytes()[3], b'1'..=b'9'))
}

unsafe extern "C" fn host_log(id: abi::StrView, message: abi::StrView) -> abi::Status {
    match caught(|| {
        let Ok(id) = (unsafe { id.as_str(256) }) else { return abi::Status::INVALID };
        let Ok(message) = (unsafe { message.as_str(abi::MAX_TEXT_BYTES) }) else { return abi::Status::INVALID };
        eprintln!("DLL mod {id}: {message}");
        abi::Status::OK
    }) {
        Ok(status) => status,
        Err(()) => abi::Status::PANIC,
    }
}

unsafe extern "C" fn host_feedback(handle: abi::Handle, text: abi::StrView) -> abi::Status {
    service(handle, |context| {
        let Ok(text) = (unsafe { text.as_str(abi::MAX_TEXT_BYTES) }) else { return abi::Status::INVALID };
        let Some(host) = context.host.write() else { return abi::Status::UNSUPPORTED };
        host.feedback(Text::literal(text));
        abi::Status::OK
    })
}

unsafe extern "C" fn host_world_key(handle: abi::Handle, kind: *mut u32, output: *mut abi::OutputBuffer) -> abi::Status {
    service(handle, |context| {
        if kind.is_null() || !kind.is_aligned() { return abi::Status::INVALID; }
        let Ok(output) = (unsafe { output_buffer(output) }) else { return abi::Status::INVALID };
        let Some(host) = context.host.read() else { return abi::Status::UNSUPPORTED };
        let Some(world) = host.client_world_key() else {
            unsafe { *kind = 0 };
            return abi::Status::NOT_FOUND;
        };
        let (world_kind, value) = match world {
            NativeClientWorld::Singleplayer { folder } => (abi::WORLD_SINGLEPLAYER, folder),
            NativeClientWorld::Server { address } => (abi::WORLD_SERVER, address),
            NativeClientWorld::LanGuest { address } => (abi::WORLD_LAN, address),
            NativeClientWorld::Friend => (abi::WORLD_FRIEND, String::new()),
            NativeClientWorld::Replay { title } => (abi::WORLD_REPLAY, title),
            #[allow(unreachable_patterns)]
            _ => return abi::Status::UNSUPPORTED,
        };
        unsafe { *kind = world_kind; output.write(value.as_bytes()) }
    })
}

unsafe extern "C" fn host_hud_text(handle: abi::Handle, batch: abi::Handle, text: abi::StrView, at: abi::Point, rgb: u32, shadow: u32) -> abi::Status {
    service(handle, |context| {
        if batch != hud_handle(handle) || !at.x.is_finite() || !at.y.is_finite() || shadow > 1 {
            return abi::Status::INVALID;
        }
        let Ok(text) = (unsafe { text.as_str(abi::MAX_TEXT_BYTES) }) else { return abi::Status::INVALID };
        let Some(batch) = context.batch.as_deref_mut() else { return abi::Status::INVALID };
        let Some(host) = context.host.read() else { return abi::Status::UNSUPPORTED };
        host.text(batch, text, [at.x, at.y], rgb, shadow != 0);
        abi::Status::OK
    })
}

unsafe extern "C" fn host_hud_rect(handle: abi::Handle, batch: abi::Handle, rect: abi::Rect, argb: u32) -> abi::Status {
    service(handle, |context| {
        let coordinates = [rect.x, rect.y, rect.width, rect.height];
        if batch != hud_handle(handle) || !coordinates.iter().all(|value| value.is_finite()) {
            return abi::Status::INVALID;
        }
        let Some(batch) = context.batch.as_deref_mut() else { return abi::Status::INVALID };
        let Some(host) = context.host.read() else { return abi::Status::UNSUPPORTED };
        host.fill(batch, coordinates, argb);
        abi::Status::OK
    })
}

unsafe extern "C" fn host_net_send(handle: abi::Handle, channel: abi::StrView, bytes: abi::ByteView, error: *mut abi::OutputBuffer) -> abi::Status {
    service(handle, |context| {
        let Ok(error) = (unsafe { output_buffer(error) }) else { return abi::Status::INVALID };
        let Ok(channel) = (unsafe { channel.as_str(256) }) else { return abi::Status::INVALID };
        let Ok(bytes) = (unsafe { bytes.as_slice(abi::MAX_DATA_BYTES) }) else { return abi::Status::INVALID };
        let Some(host) = context.host.write() else { return abi::Status::UNSUPPORTED };
        match host.net_send_to_server(channel, bytes) {
            Ok(()) => abi::Status::OK,
            Err(message) => {
                unsafe { error.write(message.as_bytes()) };
                abi::Status::HOST_ERROR
            }
        }
    })
}

unsafe extern "C" fn host_setting_get(handle: abi::Handle, key: abi::StrView, output: *mut abi::OutputBuffer) -> abi::Status {
    service(handle, |context| {
        let Ok(output) = (unsafe { output_buffer(output) }) else { return abi::Status::INVALID };
        let Ok(key) = (unsafe { key.as_str(64) }) else { return abi::Status::INVALID };
        if !safe_component(key) { return abi::Status::INVALID; }
        let Some(host) = context.host.read() else { return abi::Status::UNSUPPORTED };
        match host.setting(context.id, key) {
            Some(value) => unsafe { output.write(value.as_bytes()) },
            None => abi::Status::NOT_FOUND,
        }
    })
}

unsafe extern "C" fn host_setting_set(handle: abi::Handle, key: abi::StrView, value: abi::StrView) -> abi::Status {
    service(handle, |context| {
        let Ok(key) = (unsafe { key.as_str(64) }) else { return abi::Status::INVALID };
        let Ok(value) = (unsafe { value.as_str(abi::MAX_TEXT_BYTES) }) else { return abi::Status::INVALID };
        if !safe_component(key) { return abi::Status::INVALID; }
        let Some(host) = context.host.write() else { return abi::Status::UNSUPPORTED };
        host.set_setting(context.id, key, value);
        abi::Status::OK
    })
}

fn net_state(state: NetState) -> u32 {
    match state { NetState::Login => abi::NET_LOGIN, NetState::Configuration => abi::NET_CONFIGURATION, NetState::Play => abi::NET_PLAY }
}

pub(super) fn frame_time(at: Instant) -> u64 {
    static ORIGIN: OnceLock<Instant> = OnceLock::new();
    at.checked_duration_since(*ORIGIN.get_or_init(|| at)).unwrap_or_default().as_nanos().min(u128::from(u64::MAX)) as u64
}

fn hand(hand: Hand) -> u32 {
    match hand { Hand::Main => abi::HAND_MAIN, Hand::Off => abi::HAND_OFF }
}

unsafe extern "C" fn host_text_width(handle: abi::Handle, text: abi::StrView, output: *mut f32) -> abi::Status {
    service(handle, |context| {
        if output.is_null() || !output.is_aligned() { return abi::Status::INVALID; }
        let Ok(text) = (unsafe { text.as_str(abi::MAX_TEXT_BYTES) }) else { return abi::Status::INVALID };
        let Some(host) = context.host.read() else { return abi::Status::UNSUPPORTED };
        let Some(width) = host.text_width(text) else { return abi::Status::NOT_FOUND };
        unsafe { *output = width };
        abi::Status::OK
    })
}

unsafe extern "C" fn host_player(handle: abi::Handle, callback: abi::PlayerCallback, user: *mut c_void) -> abi::Status {
    service(handle, |context| {
        let Some(callback) = callback else { return abi::Status::INVALID };
        let Some(host) = context.host.read() else { return abi::Status::UNSUPPORTED };
        let Some(player) = host.local_player_snapshot() else { return abi::Status::NOT_FOUND };
        let equipment = player.equipment.map(|stack| {
            let mut writer = Writer::new();
            Slot::write(&stack, &mut writer);
            writer.data
        });
        let dimension = player.dimension.to_string();
        let flags = [
            (player.on_ground, abi::PLAYER_ON_GROUND), (player.sprinting, abi::PLAYER_SPRINTING),
            (player.swimming, abi::PLAYER_SWIMMING), (player.fall_flying, abi::PLAYER_FALL_FLYING),
            (player.vehicle.is_some(), abi::PLAYER_HAS_VEHICLE), (player.main_hand_right, abi::PLAYER_MAIN_HAND_RIGHT),
            (player.in_water, abi::PLAYER_IN_WATER), (player.eye_in_water, abi::PLAYER_EYE_IN_WATER),
            (player.flying, abi::PLAYER_FLYING), (player.max_health.is_some(), abi::PLAYER_HAS_MAX_HEALTH),
        ].into_iter().fold(0, |flags, (enabled, flag)| flags | if enabled { flag } else { 0 });
        let snapshot = abi::LocalPlayerSnapshot {
            entity_id: player.entity_id, uuid: player.uuid.to_be_bytes(), pos: player.pos,
            yaw: player.yaw, pitch: player.pitch, head_yaw: player.head_yaw, body_yaw: player.body_yaw,
            flags, pose: player.pose.id() as u32,
            swing_hand: player.swing.map_or(abi::HAND_NONE, |(value, _)| hand(value)),
            swing_ticks: player.swing.map_or(0, |(_, ticks)| ticks),
            using_hand: player.using_item.map_or(abi::HAND_NONE, hand), use_ticks: player.use_ticks,
            equipment: std::array::from_fn(|index| abi::ByteView::new(&equipment[index])),
            vehicle: player.vehicle.unwrap_or_default(), skin_parts: u32::from(player.skin_parts),
            dimension: abi::StrView::new(&dimension), game_mode: u32::from(player.game_mode),
            health: player.health, max_health: player.max_health.unwrap_or_default(), food_level: player.food_level,
        };
        unsafe { callback(user, &snapshot) }
    })
}

unsafe extern "C" fn host_snapshot(handle: abi::Handle, frame: abi::SnapshotFrameCallback, report: abi::SnapshotReportCallback, user: *mut c_void) -> abi::Status {
    service(handle, |context| {
        let (Some(frame), Some(report)) = (frame, report) else { return abi::Status::INVALID };
        let Some(host) = context.host.write() else { return abi::Status::UNSUPPORTED };
        let mut status = abi::Status::OK;
        let result = host.snapshot_client_world(&mut |state, packet_id, bytes| {
            if status == abi::Status::OK {
                status = if bytes.len() > abi::MAX_FRAME_BYTES { abi::Status::INVALID }
                    else { unsafe { frame(user, net_state(state), packet_id, abi::ByteView::new(bytes)) } };
            }
        });
        if status != abi::Status::OK { return status; }
        if result.missing.len() > abi::MAX_SETTINGS_BUTTONS { return abi::Status::INVALID; }
        let missing: Vec<_> = result.missing.iter().map(|value| abi::StrView::new(value)).collect();
        unsafe { report(user, &abi::SnapshotReport { written: result.written, missing: missing.as_ptr(), missing_len: missing.len() }) }
    })
}

unsafe extern "C" fn host_playing(handle: abi::Handle, output: *mut u32) -> abi::Status {
    service(handle, |context| {
        if output.is_null() || !output.is_aligned() { return abi::Status::INVALID; }
        let Some(host) = context.host.read() else { return abi::Status::UNSUPPORTED };
        unsafe { *output = u32::from(host.playing()) };
        abi::Status::OK
    })
}

unsafe extern "C" fn host_instance_dir(handle: abi::Handle, output: *mut abi::OutputBuffer) -> abi::Status {
    service(handle, |context| {
        let Ok(output) = (unsafe { output_buffer(output) }) else { return abi::Status::INVALID };
        let Some(host) = context.host.read() else { return abi::Status::UNSUPPORTED };
        let path = host.instance_dir();
        let Some(path) = path.to_str() else { return abi::Status::UNSUPPORTED };
        unsafe { output.write(path.as_bytes()) }
    })
}

unsafe extern "C" fn host_account(handle: abi::Handle, callback: abi::AccountCallback, user: *mut c_void) -> abi::Status {
    service(handle, |context| {
        let Some(callback) = callback else { return abi::Status::INVALID };
        let Some(host) = context.host.read() else { return abi::Status::UNSUPPORTED };
        let Some(account) = host.account() else { return abi::Status::NOT_FOUND };
        unsafe { callback(user, &abi::Account { name: abi::StrView::new(&account.name), uuid: account.uuid.to_be_bytes(), online: u32::from(account.online) }) }
    })
}

unsafe extern "C" fn host_screen_open(handle: abi::Handle, id: abi::StrView, output: *mut u32) -> abi::Status {
    screen_open(handle, id, output, false)
}

unsafe extern "C" fn host_screen_open_anywhere(handle: abi::Handle, id: abi::StrView, output: *mut u32) -> abi::Status {
    screen_open(handle, id, output, true)
}

fn screen_open(handle: abi::Handle, id: abi::StrView, output: *mut u32, anywhere: bool) -> abi::Status {
    service(handle, |context| {
        if output.is_null() || !output.is_aligned() { return abi::Status::INVALID; }
        let Ok(id) = (unsafe { id.as_str(256) }) else { return abi::Status::INVALID };
        let Some(host) = context.host.write() else { return abi::Status::UNSUPPORTED };
        let opened = if anywhere { host.mod_screen_open_anywhere(id) } else { host.mod_screen_open(id) };
        unsafe { *output = u32::from(opened) };
        abi::Status::OK
    })
}

unsafe extern "C" fn host_screen_close(handle: abi::Handle, id: abi::StrView) -> abi::Status {
    service(handle, |context| {
        let Ok(id) = (unsafe { id.as_str(256) }) else { return abi::Status::INVALID };
        let Some(host) = context.host.write() else { return abi::Status::UNSUPPORTED };
        host.mod_screen_close(id);
        abi::Status::OK
    })
}

unsafe extern "C" fn host_screen(handle: abi::Handle, output: *mut abi::OutputBuffer) -> abi::Status {
    service(handle, |context| {
        let Ok(output) = (unsafe { output_buffer(output) }) else { return abi::Status::INVALID };
        let Some(host) = context.host.read() else { return abi::Status::UNSUPPORTED };
        match host.mod_screen() { Some(id) => unsafe { output.write(id.as_bytes()) }, None => abi::Status::NOT_FOUND }
    })
}

fn rect_values(rect: abi::Rect) -> [f32; 4] { [rect.x, rect.y, rect.width, rect.height] }

unsafe extern "C" fn host_batch_len(handle: abi::Handle, batch: abi::Handle, output: *mut usize) -> abi::Status {
    service(handle, |context| {
        if batch != hud_handle(handle) || output.is_null() || !output.is_aligned() { return abi::Status::INVALID; }
        let Some(batch) = context.batch.as_deref() else { return abi::Status::INVALID };
        unsafe { *output = batch.vertices.len() - context.batch_start };
        abi::Status::OK
    })
}

fn batch_range<'borrow>(context: &'borrow mut Invocation<'_, '_>, start: usize) -> Result<(&'borrow mut HudBatch, usize), abi::Status> {
    let Some(batch) = context.batch.as_deref_mut() else { return Err(abi::Status::INVALID) };
    let Some(start) = context.batch_start.checked_add(start).filter(|start| *start <= batch.vertices.len()) else { return Err(abi::Status::INVALID) };
    Ok((batch, start))
}

unsafe extern "C" fn host_batch_scale(handle: abi::Handle, batch: abi::Handle, start: usize, center: abi::Point, scale: abi::Point) -> abi::Status {
    service(handle, |context| {
        if batch != hud_handle(handle) || ![center.x, center.y, scale.x, scale.y].iter().all(|value| value.is_finite()) { return abi::Status::INVALID; }
        let Ok((batch, start)) = batch_range(context, start) else { return abi::Status::INVALID };
        batch.scale_since(start, center.x, center.y, scale.x, scale.y);
        abi::Status::OK
    })
}

unsafe extern "C" fn host_batch_clip(handle: abi::Handle, batch: abi::Handle, start: usize, rect: abi::Rect) -> abi::Status {
    service(handle, |context| {
        if batch != hud_handle(handle) || !rect_values(rect).iter().all(|value| value.is_finite()) { return abi::Status::INVALID; }
        let Ok((batch, start)) = batch_range(context, start) else { return abi::Status::INVALID };
        batch.clip_since(start, rect.x, rect.y, rect.x + rect.width, rect.y + rect.height);
        abi::Status::OK
    })
}

unsafe extern "C" fn host_batch_tint(handle: abi::Handle, batch: abi::Handle, start: usize, opacity: f32) -> abi::Status {
    service(handle, |context| {
        if batch != hud_handle(handle) || !opacity.is_finite() || !(0.0..=1.0).contains(&opacity) { return abi::Status::INVALID; }
        let Ok((batch, start)) = batch_range(context, start) else { return abi::Status::INVALID };
        for vertex in &mut batch.vertices[start..] {
            let alpha = ((vertex.color >> 24) as f32 * opacity).round() as u32;
            vertex.color = (vertex.color & 0x00ff_ffff) | (alpha << 24);
        }
        abi::Status::OK
    })
}

unsafe extern "C" fn host_texture_create(handle: abi::Handle, width: u32, height: u32, format: u32, filter: u32, output: *mut abi::Handle) -> abi::Status {
    service(handle, |context| {
        if output.is_null() || !output.is_aligned() || width == 0 || height == 0 || width > 16384 || height > 16384 || context.textures.len() >= 256 { return abi::Status::INVALID; }
        let format = match format { abi::TEXTURE_RGBA8 => wgpu::TextureFormat::Rgba8Unorm, abi::TEXTURE_BGRA8 => wgpu::TextureFormat::Bgra8Unorm, abi::TEXTURE_RGBA16_FLOAT => wgpu::TextureFormat::Rgba16Float, _ => return abi::Status::INVALID };
        let filter = match filter { abi::FILTER_NEAREST => NativeGuiTextureFilter::Nearest, abi::FILTER_LINEAR => NativeGuiTextureFilter::Linear, _ => return abi::Status::INVALID };
        let Some(host) = context.host.read() else { return abi::Status::UNSUPPORTED };
        let Ok(texture) = host.gui_texture_create([width, height], format, filter) else { return abi::Status::HOST_ERROR };
        let texture_handle = abi::Handle { id: *context.next_texture, generation: context.texture_owner };
        let Some(next) = context.next_texture.checked_add(1) else { return abi::Status::DISABLED };
        *context.next_texture = next;
        context.textures.push((texture_handle, texture));
        unsafe { *output = texture_handle };
        abi::Status::OK
    })
}

unsafe extern "C" fn host_texture_release(handle: abi::Handle, texture: abi::Handle) -> abi::Status {
    service(handle, |context| {
        let Some(index) = context.textures.iter().position(|(stored, _)| *stored == texture) else { return abi::Status::INVALID };
        context.textures.swap_remove(index);
        abi::Status::OK
    })
}

unsafe extern "C" fn host_texture(handle: abi::Handle, batch: abi::Handle, texture: abi::Handle, rect: abi::Rect, uv: abi::Rect, argb: u32, nine: *const abi::GuiNineSlice, output: *mut u32) -> abi::Status {
    service(handle, |context| {
        if batch != hud_handle(handle) || output.is_null() || !output.is_aligned() || !rect_values(rect).iter().chain(rect_values(uv).iter()).all(|value| value.is_finite()) { return abi::Status::INVALID; }
        let nine = if nine.is_null() { None } else {
            if !nine.is_aligned() { return abi::Status::INVALID; }
            let nine = unsafe { nine.read() };
            Some(NativeGuiNineSlice { source_size: nine.source_size, border: nine.border })
        };
        let Some((_, texture)) = context.textures.iter().find(|(stored, _)| *stored == texture) else { return abi::Status::INVALID };
        let Some(batch) = context.batch.as_deref_mut() else { return abi::Status::INVALID };
        let Some(host) = context.host.read() else { return abi::Status::UNSUPPORTED };
        unsafe { *output = u32::from(host.gui_texture(batch, texture, rect_values(rect), rect_values(uv), argb, nine)) };
        abi::Status::OK
    })
}

static HOST_TABLE: abi::HostTableV1 = abi::HostTableV1 {
    header: abi::TableHeader::for_type::<abi::HostTableV1>(),
    services: abi::ALL_SERVICES,
    log: Some(host_log),
    feedback: Some(host_feedback),
    client_world_key: Some(host_world_key),
    hud_text: Some(host_hud_text),
    hud_rect: Some(host_hud_rect),
    mod_net_send: Some(host_net_send),
    setting_get: Some(host_setting_get),
    setting_set: Some(host_setting_set),
    text_width: Some(host_text_width),
    local_player_snapshot: Some(host_player),
    snapshot_client_world: Some(host_snapshot),
    playing: Some(host_playing),
    instance_dir: Some(host_instance_dir),
    account: Some(host_account),
    mod_screen_open: Some(host_screen_open),
    mod_screen_close: Some(host_screen_close),
    mod_screen: Some(host_screen),
    mod_screen_open_anywhere: Some(host_screen_open_anywhere),
    gui_texture: Some(host_texture),
    batch_len: Some(host_batch_len),
    batch_scale_since: Some(host_batch_scale),
    batch_clip_since: Some(host_batch_clip),
    batch_tint_since: Some(host_batch_tint),
    gui_texture_create: Some(host_texture_create),
    gui_texture_release: Some(host_texture_release),
};

include!(concat!(env!("OUT_DIR"), "/dll_mod_adapters.rs"));

#[cfg(all(test, windows))]
pub(super) fn from_library(
    mods: Mods,
    identities: &'static [super::NativeMod],
    path: &Path,
    unsigned_flag: Option<&OsStr>,
    symbol: &[u8],
) -> Result<(Mods, &'static [super::NativeMod]), String> {
    let result = (|| {
        if !path.is_absolute() || !path.extension().and_then(OsStr::to_str).is_some_and(|extension| extension.eq_ignore_ascii_case("dll")) {
            return Err(LoadError::Invalid("DLL path must be absolute with .dll extension"));
        }
        let allow_unsigned = unsigned_flag == Some(OsStr::new("1"));
        verify(path, allow_unsigned)?;
        let path = path.canonicalize().map_err(|error| LoadError::Io(error.to_string()))?;
        let metadata = load_symbol(&path, allow_unsigned, symbol)?;
        if identities.len() >= MAX_MODS || identities.iter().any(|identity| identity.id == metadata.id) {
            return Err(LoadError::Invalid("duplicate or too many native identities"));
        }
        Ok(append(mods, identities, vec![metadata]))
    })();
    result.map_err(|error| format!("{error:?}"))
}

#[cfg(test)]
pub(super) unsafe fn in_process(
    mods: Mods,
    identities: &'static [super::NativeMod],
    entries: &[abi::ModEntry],
) -> Result<(Mods, &'static [super::NativeMod]), String> {
    let accepted = entries.iter().map(|entry| unsafe {
        descriptor(entry(abi::ABI_VERSION, &HOST_TABLE)).map_err(|error| format!("{error:?}"))
    }).collect::<Result<Vec<_>, _>>()?;
    Ok(append(mods, identities, accepted))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn settings_components_reject_windows_path_aliases() {
        for value in ["", ".", "..", "...", "value.", "../value", "C:value", "NUL", "con.txt", "COM1", "lpt9.data"] {
            assert!(!safe_component(value), "{value}");
        }
        for value in ["dll-hello", "display.visible", "value_1", "COM10"] {
            assert!(safe_component(value), "{value}");
        }
    }

    #[test]
    fn unsigned_and_missing_entry_are_rejected_before_execution() {
        assert_eq!(verify(Path::new("missing.dll"), false), Err(LoadError::Unsigned));
        assert_eq!(verify(Path::new("missing.dll"), true), Ok(()));
        assert!(matches!(entry_address(None), Err(LoadError::MissingEntry)));
    }

    #[test]
    fn wasm_only_switch_does_not_authorize_external_dll_paths() {
        let switches = crate::yak_mods::UnsignedSwitches::from_values(
            None,
            Some(OsStr::new("1")),
        );
        assert!(switches.wasm);
        assert!(!switches.dll);

        let path = std::env::temp_dir().join("yoake-wasm-switch-must-not-load.dll");
        assert_eq!(verify(&path, switches.dll), Err(LoadError::Unsigned));

        let paths = std::env::join_paths([path]).unwrap();
        let mods: Mods = Arc::from(Vec::<Box<dyn Hooks>>::new());
        let identities: &'static [super::super::NativeMod] = &[];
        let (result, result_ids) = extend_paths(mods.clone(), identities, Some(&paths), switches.dll);
        assert!(Arc::ptr_eq(&mods, &result));
        assert!(std::ptr::eq(identities, result_ids));
    }

    #[test]
    fn absent_or_empty_paths_preserve_the_original_arc_and_identities() {
        let mods: Mods = Arc::from(Vec::<Box<dyn Hooks>>::new());
        let identities: &'static [super::super::NativeMod] = &[];
        for paths in [None, Some(OsStr::new(""))] {
            let (result, result_ids) = extend_paths(mods.clone(), identities, paths, false);
            assert!(Arc::ptr_eq(&mods, &result));
            assert!(std::ptr::eq(identities, result_ids));
        }
    }

    #[test]
    fn absent_installed_binaries_preserve_the_original_arc_and_identities() {
        let mods: Mods = Arc::from(Vec::<Box<dyn Hooks>>::new());
        let identities: &'static [super::super::NativeMod] = &[];
        let (result, result_ids) = extend_yaks(mods.clone(), identities, &[], false);
        assert!(Arc::ptr_eq(&mods, &result));
        assert!(std::ptr::eq(identities, result_ids));
    }

    #[test]
    fn required_yak_identity_includes_the_verified_binary_digest() {
        let digest = "ab".repeat(32);
        let (version, side) = native_identity("1.2.3", Some(&digest));
        assert!(version.starts_with("1.2.3#"), "{version}");
        assert_eq!(version.chars().count(), "1.2.3#".len() + 32);
        assert!(matches!(side, super::super::NativeSide::Both));
        // The digest and the whole version both count.
        assert_ne!(version, native_identity("1.2.3", Some(&"cd".repeat(32))).0);
        assert_ne!(version, native_identity("1.2.4", Some(&digest)).0);
        let (version, side) = native_identity("1.2.3", None);
        assert_eq!(version, "1.2.3");
        assert!(matches!(side, super::super::NativeSide::Client));
    }

    #[test]
    fn identities_of_any_version_length_fit_the_wire_limit() {
        let digest = "ab".repeat(32);
        for length in [1usize, 31, 32, 55, 56, 64, 65, 200, 5000] {
            let version = format!("1.0.0-{}", "x".repeat(length));
            for digest in [Some(digest.as_str()), None] {
                let (text, _) = native_identity(&version, digest);
                assert!(text.chars().count() <= 64, "{length}: {}", text.len());
                // And it survives the identity list's encoder and decoder unchanged.
                let entry = horizon_protocol::mod_list::Entry {
                    id: "mod".into(), version: text.clone(), side: horizon_protocol::mod_list::Side::Both, sha256: None,
                };
                let decoded = horizon_protocol::mod_list::decode(&horizon_protocol::mod_list::encode(&[entry])).unwrap();
                assert_eq!(decoded[0].version, text);
            }
        }
        // Two long versions that differ only past the shown prefix keep different identities.
        let a = native_identity(&format!("1.0.0-{}a", "x".repeat(60)), Some(&digest)).0;
        let b = native_identity(&format!("1.0.0-{}b", "x".repeat(60)), Some(&digest)).0;
        assert_ne!(a, b);
    }

    #[test]
    fn descriptor_header_fails_closed_before_reading_the_payload() {
        let mut header = abi::TableHeader::for_type::<abi::ModDescriptor>();
        header.abi_version += 1;
        assert!(matches!(unsafe { descriptor((&header as *const abi::TableHeader).cast()) }, Err(LoadError::Invalid(_))));
        header = abi::TableHeader::for_type::<abi::ModDescriptor>();
        header.schema_hash ^= 1;
        assert!(unsafe { descriptor((&header as *const abi::TableHeader).cast()) }.is_err());
        header = abi::TableHeader::for_type::<abi::ModDescriptor>();
        header.size -= 1;
        assert!(unsafe { descriptor((&header as *const abi::TableHeader).cast()) }.is_err());
        assert!(unsafe { descriptor(std::ptr::null()) }.is_err());
    }

    #[test]
    fn stale_and_wrong_thread_handles_never_reach_host_memory() {
        let handle = abi::Handle { id: 1, generation: 1 };
        assert_eq!(unsafe { host_feedback(handle, abi::StrView::new("stale")) }, abi::Status::INVALID);
        assert_eq!(std::thread::spawn(move || unsafe {
            host_feedback(handle, abi::StrView::new("wrong thread"))
        }).join().unwrap(), abi::Status::INVALID);
    }
}
