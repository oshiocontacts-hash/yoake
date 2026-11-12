//! Startup-only, trusted native libraries. No DLL means no dispatch wrappers.
//! DLL memory must obey horizon_mod_abi's contract; this is not a sandbox.

use crate::mod_api::*;
use horizon_mod_abi as abi;
use std::{
    cell::Cell,
    ffi::{OsStr, c_void},
    marker::PhantomData,
    path::{Path, PathBuf},
    rc::Rc,
    sync::{Arc, Mutex, TryLockError, atomic::{AtomicU64, Ordering}},
};

const MAX_MODS: usize = 128;
const HUD_BIT: u64 = 1 << 63;
static GENERATION: AtomicU64 = AtomicU64::new(1);

#[derive(Debug, PartialEq, Eq)]
enum LoadError {
    Unsigned,
    #[cfg_attr(windows, allow(dead_code))]
    Platform,
    MissingEntry,
    Invalid(&'static str),
    Io(String),
}

/// TODO(phase 3): authenticate the file and pin its bytes before executing it.
/// Until signing exists, only the explicit developer override can pass this gate.
fn verify(_path: &Path, allow_unsigned: bool) -> Result<(), LoadError> {
    if allow_unsigned { Ok(()) } else { Err(LoadError::Unsigned) }
}

struct Metadata {
    id: String,
    version: String,
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
        hooks: unsafe { value.hooks.read() },
        create: value.create.ok_or(LoadError::Invalid("missing create"))?,
        destroy: value.destroy.ok_or(LoadError::Invalid("missing destroy"))?,
    })
}

#[cfg(windows)]
fn load(path: &Path, allow_unsigned: bool) -> Result<Metadata, LoadError> {
    verify(path, allow_unsigned)?;
    let library = unsafe { libloading::Library::new(path) }.map_err(|error| LoadError::Io(error.to_string()))?;
    let entry = unsafe { library.get::<abi::ModEntry>(b"yoake_mod_entry\0") }.ok().map(|symbol| *symbol);
    let entry = entry_address(entry)?;
    let metadata = unsafe { descriptor(entry(abi::ABI_VERSION, &HOST_TABLE))? };
    let _retained_until_exit = Box::leak(Box::new(library));
    Ok(metadata)
}

#[cfg(not(windows))]
fn load(path: &Path, allow_unsigned: bool) -> Result<Metadata, LoadError> {
    verify(path, allow_unsigned)?;
    Err(LoadError::Platform)
}

pub(super) fn extend(mods: Mods, identities: &'static [super::NativeMod]) -> (Mods, &'static [super::NativeMod]) {
    extend_paths(mods, identities, std::env::var_os("YOAKE_DLL_MODS").as_deref(), std::env::var_os("YOAKE_DLL_MODS_UNSIGNED").as_deref() == Some(OsStr::new("1")))
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
        let version: &'static str = Box::leak(metadata.version.clone().into_boxed_str());
        owned.push(super::NativeMod { id, version, side: super::NativeSide::Client });
        combined.push(Box::new(DllMod::new(metadata, combined.len() as u64 + 1)));
    }
    (Arc::from(combined), Box::leak(owned.into_boxed_slice()))
}

struct StaticMod {
    original: Mods,
    index: usize,
}

struct InstanceState {
    pointer: *mut c_void,
    disabled: bool,
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
        Self { id: metadata.id, handle_id, hooks: metadata.hooks, create: metadata.create, destroy: metadata.destroy, state: Mutex::new(InstanceState { pointer: std::ptr::null_mut(), disabled: false }) }
    }

    fn invoke<'call>(&self, host: &'call mut dyn Host, batch: Option<&'call mut HudBatch>, callback: impl FnOnce(*mut c_void, abi::Handle) -> abi::HookResult) {
        let mut state = match self.state.try_lock() {
            Ok(state) => state,
            Err(TryLockError::WouldBlock) => return,
            Err(TryLockError::Poisoned(error)) => {
                error.into_inner().disabled = true;
                log::error!("DLL mod {} disabled: poisoned instance", self.id);
                return;
            }
        };
        if state.disabled { return; }
        let Ok(generation) = GENERATION.try_update(Ordering::Relaxed, Ordering::Relaxed, |generation| generation.checked_add(1)) else {
            state.disabled = true;
            log::error!("DLL mod {} disabled: handle generation exhausted", self.id);
            return;
        };
        let handle = abi::Handle { id: self.handle_id, generation };
        let mut context = Invocation { host, batch, id: &self.id, faulted: false };
        let result = {
            let _scope = Scope::enter(&mut context, handle);
            if state.pointer.is_null() {
                let status = unsafe { (self.create)(&HOST_TABLE, handle, &mut state.pointer) };
                if status != abi::Status::OK || state.pointer.is_null() {
                    abi::HookResult::failed(if status == abi::Status::OK { abi::Status::INVALID } else { status })
                } else {
                    callback(state.pointer, handle)
                }
            } else {
                callback(state.pointer, handle)
            }
        };
        if context.faulted || result.status != abi::Status::OK || result.flow != abi::FLOW_CONTINUE {
            state.disabled = true;
            log::error!("DLL mod {} disabled: status={} flow={} host_fault={}", self.id, result.status.0, result.flow, context.faulted);
        }
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

struct Invocation<'call> {
    host: &'call mut dyn Host,
    batch: Option<&'call mut HudBatch>,
    id: &'call str,
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
    fn enter(context: &'scope mut Invocation<'_>, handle: abi::Handle) -> Self {
        let previous = ACTIVE.with(|active| active.replace(Some(Active { pointer: (context as *mut Invocation<'_>).cast(), handle, busy: false })));
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

fn service(handle: abi::Handle, operation: impl FnOnce(&mut Invocation<'_>) -> abi::Status) -> abi::Status {
    ACTIVE.with(|slot| {
        let Some(active) = slot.get() else { return abi::Status::INVALID };
        if active.handle != handle || !handle.is_valid() { return abi::Status::INVALID; }
        if active.busy { return abi::Status::BUSY; }
        let _scope = ServiceScope(active);
        slot.set(Some(Active { busy: true, ..active }));
        let context = unsafe { &mut *active.pointer.cast::<Invocation<'_>>() };
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

fn safe_component(value: &str) -> bool {
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
        context.host.feedback(Text::literal(text));
        abi::Status::OK
    })
}

unsafe extern "C" fn host_world_key(handle: abi::Handle, kind: *mut u32, output: *mut abi::OutputBuffer) -> abi::Status {
    service(handle, |context| {
        if kind.is_null() || !kind.is_aligned() { return abi::Status::INVALID; }
        let Ok(output) = (unsafe { output_buffer(output) }) else { return abi::Status::INVALID };
        let Some(world) = context.host.client_world_key() else {
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
        context.host.text(batch, text, [at.x, at.y], rgb, shadow != 0);
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
        context.host.fill(batch, coordinates, argb);
        abi::Status::OK
    })
}

unsafe extern "C" fn host_net_send(handle: abi::Handle, channel: abi::StrView, bytes: abi::ByteView, error: *mut abi::OutputBuffer) -> abi::Status {
    service(handle, |context| {
        let Ok(error) = (unsafe { output_buffer(error) }) else { return abi::Status::INVALID };
        let Ok(channel) = (unsafe { channel.as_str(256) }) else { return abi::Status::INVALID };
        let Ok(bytes) = (unsafe { bytes.as_slice(abi::MAX_DATA_BYTES) }) else { return abi::Status::INVALID };
        match context.host.net_send_to_server(channel, bytes) {
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
        match context.host.setting(context.id, key) {
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
        context.host.set_setting(context.id, key, value);
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
};

include!(concat!(env!("OUT_DIR"), "/dll_mod_adapters.rs"));

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
