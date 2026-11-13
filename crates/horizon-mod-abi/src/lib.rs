//! Version-two, native-width C ABI. There are no engine dependencies or Rust trait
//! objects on this boundary. `yoake_mod_abi.h` and the tables are generated into
//! this crate's OUT_DIR from the same catalog and fragments as static dispatch.
//!
//! # Ownership and safety
//! All pointers refer to caller-owned ABI memory, never engine objects. Views are
//! borrowed only until the synchronous call returns; strings are UTF-8 without a
//! terminator. Output buffers stay owned and allocated by the caller. Callees set
//! `len` to the required byte count, including on BUFFER_TOO_SMALL, and never
//! allocate memory for the caller to free. Empty views and zero-capacity queries
//! may use null pointers. Nonempty memory must be aligned, live, accessible and
//! nonoverlapping for the documented read/write; numeric checks cannot validate
//! arbitrary native memory. Loading native code is not a sandbox.
//!
//! Handles are checked ids plus generations, not addresses. Host handles and HUD
//! handles expire when the hook returns, work only on its thread, and cannot be
//! retained or used recursively from inside a host service. HUD handles are only
//! valid in drawing callbacks. Byte/string views returned by a mod are copied by the
//! host before returning to engine dispatch.
//!
//! Read and validate the fixed TableHeader prefix before any larger table. V2
//! requires exact sizes, schema, target, release/nightly strings and the build
//! fingerprint (release + nightly + ABI schema hash); it does not promise compatibility
//! across releases or architectures. The fingerprint detects mismatches, not tampering.
//! Descriptor and hook table memory must remain immutable for the library's whole
//! lifetime. `create` transfers a mod-owned opaque instance to the host; only that
//! mod's `destroy` may free it. The host serializes calls to each instance and
//! retains loaded libraries until process exit; there is no hot reload.
//!
//! Extern functions must never unwind. The SDK catches Rust unwinding panics and
//! logs/disables that instance; panic=abort, access violations, foreign exceptions
//! and malicious pointers cannot be recovered by catch_unwind. Mods must use an
//! unwinding panic strategy. Unknown status/flow values fail closed. Only hooks
//! marked cancel may return CANCEL; cancellation skips subsequent mods. Existing
//! table type names retain V1 for source compatibility, not binary compatibility.

use core::{ffi::c_void, mem::size_of, ptr, slice, str};

pub const MAX_TEXT_BYTES: usize = 65_536;
pub const MAX_DATA_BYTES: usize = 30_000;
pub const MAX_OUTPUT_BYTES: usize = 1_048_576;
pub const MAX_FRAME_BYTES: usize = 8_388_608;
pub const MAX_SETTINGS_BUTTONS: usize = 1024;
pub const NET_LOGIN: u32 = 1;
pub const NET_CONFIGURATION: u32 = 2;
pub const NET_PLAY: u32 = 3;
pub const HAND_NONE: u32 = 0;
pub const HAND_MAIN: u32 = 1;
pub const HAND_OFF: u32 = 2;
pub const PLAYER_ON_GROUND: u32 = 1 << 0;
pub const PLAYER_SPRINTING: u32 = 1 << 1;
pub const PLAYER_SWIMMING: u32 = 1 << 2;
pub const PLAYER_FALL_FLYING: u32 = 1 << 3;
pub const PLAYER_HAS_VEHICLE: u32 = 1 << 4;
pub const PLAYER_MAIN_HAND_RIGHT: u32 = 1 << 5;
pub const PLAYER_IN_WATER: u32 = 1 << 6;
pub const PLAYER_EYE_IN_WATER: u32 = 1 << 7;
pub const PLAYER_FLYING: u32 = 1 << 8;
pub const PLAYER_HAS_MAX_HEALTH: u32 = 1 << 9;
pub const TEXTURE_RGBA8: u32 = 1;
pub const TEXTURE_BGRA8: u32 = 2;
pub const TEXTURE_RGBA16_FLOAT: u32 = 3;
pub const FILTER_NEAREST: u32 = 1;
pub const FILTER_LINEAR: u32 = 2;
pub const SIDE_CLIENT: u32 = 2;
pub const WORLD_SINGLEPLAYER: u32 = 1;
pub const WORLD_SERVER: u32 = 2;
pub const WORLD_LAN: u32 = 3;
pub const WORLD_FRIEND: u32 = 4;
pub const WORLD_REPLAY: u32 = 5;
pub const FLOW_CONTINUE: u32 = 0;
pub const FLOW_CANCEL: u32 = 1;
pub const FLOW_REPLACE: u32 = 2;

#[repr(transparent)]
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Status(pub u32);

impl Status {
    pub const OK: Self = Self(0);
    pub const INVALID: Self = Self(1);
    pub const BUFFER_TOO_SMALL: Self = Self(2);
    pub const NOT_FOUND: Self = Self(3);
    pub const UNSUPPORTED: Self = Self(4);
    pub const DISABLED: Self = Self(5);
    pub const PANIC: Self = Self(6);
    pub const HOST_ERROR: Self = Self(7);
    pub const BUSY: Self = Self(8);
}

#[repr(C)]
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct TableHeader {
    pub abi_version: u32,
    pub size: u32,
    pub schema_hash: u64,
}

impl TableHeader {
    pub const fn for_type<Table>() -> Self {
        Self { abi_version: ABI_VERSION, size: size_of::<Table>() as u32, schema_hash: SCHEMA_HASH }
    }

    pub fn valid_for<Table>(&self) -> bool {
        *self == Self::for_type::<Table>()
    }
}

#[repr(C)]
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Handle {
    pub id: u64,
    pub generation: u64,
}

impl Handle {
    pub fn is_valid(self) -> bool {
        self.id != 0 && self.generation != 0
    }
}

#[repr(C)]
#[derive(Clone, Copy, Debug)]
pub struct ByteView {
    pub ptr: *const u8,
    pub len: usize,
}

impl ByteView {
    pub const fn new(bytes: &[u8]) -> Self {
        Self { ptr: bytes.as_ptr(), len: bytes.len() }
    }

    /// # Safety
    /// The caller guarantees readable memory for the entire returned borrow.
    pub unsafe fn as_slice<'view>(self, maximum: usize) -> Result<&'view [u8], Status> {
        if self.len > maximum || self.len > isize::MAX as usize || (self.len != 0 && self.ptr.is_null()) {
            return Err(Status::INVALID);
        }
        if self.len == 0 {
            return Ok(&[]);
        }
        Ok(unsafe { slice::from_raw_parts(self.ptr, self.len) })
    }
}

#[repr(C)]
#[derive(Clone, Copy, Debug)]
pub struct StrView {
    pub ptr: *const u8,
    pub len: usize,
}

impl StrView {
    pub const fn new(text: &str) -> Self {
        Self { ptr: text.as_ptr(), len: text.len() }
    }

    /// # Safety
    /// The caller guarantees readable memory for the entire returned borrow.
    pub unsafe fn as_str<'view>(self, maximum: usize) -> Result<&'view str, Status> {
        let bytes = unsafe { ByteView { ptr: self.ptr, len: self.len }.as_slice(maximum)? };
        str::from_utf8(bytes).map_err(|_| Status::INVALID)
    }
}

#[repr(C)]
#[derive(Debug)]
pub struct OutputBuffer {
    pub ptr: *mut u8,
    pub capacity: usize,
    pub len: usize,
}

impl OutputBuffer {
    pub fn new(bytes: &mut [u8]) -> Self {
        Self { ptr: bytes.as_mut_ptr(), capacity: bytes.len(), len: 0 }
    }

    /// # Safety
    /// `ptr` must permit `capacity` writable bytes, disjoint from `bytes`.
    pub unsafe fn write(&mut self, bytes: &[u8]) -> Status {
        self.len = bytes.len();
        if self.capacity > MAX_OUTPUT_BYTES || (self.capacity != 0 && self.ptr.is_null()) {
            return Status::INVALID;
        }
        if bytes.len() > self.capacity {
            return Status::BUFFER_TOO_SMALL;
        }
        if !bytes.is_empty() {
            unsafe { ptr::copy_nonoverlapping(bytes.as_ptr(), self.ptr, bytes.len()) };
        }
        Status::OK
    }
}

#[repr(C)]
#[derive(Clone, Copy, Debug, Default)]
pub struct Point {
    pub x: f32,
    pub y: f32,
}

#[repr(C)]
#[derive(Clone, Copy, Debug, Default)]
pub struct Rect {
    pub x: f32,
    pub y: f32,
    pub width: f32,
    pub height: f32,
}

#[repr(C)]
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct HookResult {
    pub status: Status,
    pub flow: u32,
}

impl HookResult {
    pub const OK: Self = Self { status: Status::OK, flow: FLOW_CONTINUE };

    pub const fn failed(status: Status) -> Self {
        Self { status, flow: FLOW_CONTINUE }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Contract {
    Listen,
    Cancel,
    Replace,
    Collect,
}

#[repr(C)]
#[derive(Clone, Copy, Debug, Default)]
pub struct TickEvent {
    pub reserved: u32,
}

#[repr(C)]
#[derive(Clone, Copy, Debug)]
pub struct KeyEvent {
    pub key: StrView,
    pub pressed: u32,
}

#[repr(C)]
#[derive(Clone, Copy, Debug)]
pub struct HudEvent {
    pub batch: Handle,
}

#[repr(C)]
#[derive(Clone, Copy, Debug)]
pub struct NetEvent {
    pub channel: StrView,
    pub data: ByteView,
}

#[repr(C)]
#[derive(Clone, Copy, Debug)]
pub struct FrameEvent {
    pub elapsed_ns: u64,
}

#[repr(C)]
#[derive(Clone, Copy, Debug)]
pub struct NetFramesListenedEvent {
    pub listened: *mut u32,
}

#[repr(C)]
#[derive(Clone, Copy, Debug)]
pub struct NetFrameReceived {
    pub state: u32,
    pub packet_id: i32,
    pub body: ByteView,
    pub monotonic_ns: u64,
    pub session: u64,
}

#[repr(C)]
#[derive(Clone, Copy, Debug)]
pub struct NetSessionEndedEvent {
    pub session: u64,
    pub reason: StrView,
}

#[repr(C)]
#[derive(Clone, Copy, Debug)]
pub struct GuiLayerEvent {
    pub layer: StrView,
    pub batch: Handle,
}

#[repr(C)]
#[derive(Clone, Copy, Debug)]
pub struct ScreenView {
    pub id: StrView,
    pub rect: Rect,
    pub mouse: Point,
    pub partial_tick: f32,
}

#[repr(C)]
#[derive(Clone, Copy, Debug)]
pub struct ScreenRenderEvent {
    pub screen: ScreenView,
    pub batch: Handle,
}

#[repr(C)]
#[derive(Clone, Copy, Debug)]
pub struct ScreenKeyEvent {
    pub screen: ScreenView,
    pub key: StrView,
    pub repeat: u32,
    pub modifiers: u32,
}

#[repr(C)]
#[derive(Clone, Copy, Debug)]
pub struct ScreenCharEvent {
    pub screen: ScreenView,
    pub character: u32,
    pub modifiers: u32,
}

#[repr(C)]
#[derive(Clone, Copy, Debug)]
pub struct ScreenMouseButtonEvent {
    pub screen: ScreenView,
    pub button: i32,
    pub modifiers: u32,
}

#[repr(C)]
#[derive(Clone, Copy, Debug)]
pub struct ScreenScrollEvent {
    pub screen: ScreenView,
    pub scroll: [f64; 2],
    pub modifiers: u32,
}

pub type SettingsPush = Option<unsafe extern "C" fn(*mut c_void, StrView) -> Status>;

#[repr(C)]
#[derive(Clone, Copy, Debug)]
pub struct SettingsEvent {
    pub mod_id: StrView,
    pub push: SettingsPush,
    pub user: *mut c_void,
}

#[repr(C)]
#[derive(Clone, Copy, Debug)]
pub struct SettingsClickEvent {
    pub mod_id: StrView,
    pub button: usize,
}

/// Equipment is six borrowed protocol Slot encodings; empty slots encode None.
/// UUID bytes are big-endian. Pose and game mode use their protocol numeric ids.
#[repr(C)]
#[derive(Clone, Copy, Debug)]
pub struct LocalPlayerSnapshot {
    pub entity_id: i32,
    pub uuid: [u8; 16],
    pub pos: [f64; 3],
    pub yaw: f32,
    pub pitch: f32,
    pub head_yaw: f32,
    pub body_yaw: f32,
    pub flags: u32,
    pub pose: u32,
    pub swing_hand: u32,
    pub swing_ticks: u32,
    pub using_hand: u32,
    pub use_ticks: i32,
    pub equipment: [ByteView; 6],
    pub vehicle: i32,
    pub skin_parts: u32,
    pub dimension: StrView,
    pub game_mode: u32,
    pub health: f32,
    pub max_health: f32,
    pub food_level: i32,
}

#[repr(C)]
#[derive(Clone, Copy, Debug)]
pub struct SnapshotReport {
    pub written: usize,
    pub missing: *const StrView,
    pub missing_len: usize,
}

#[repr(C)]
#[derive(Clone, Copy, Debug)]
pub struct Account {
    pub name: StrView,
    pub uuid: [u8; 16],
    pub online: u32,
}

#[repr(C)]
#[derive(Clone, Copy, Debug, Default)]
pub struct GuiNineSlice {
    pub source_size: [u32; 2],
    pub border: [u32; 4],
}

pub type PlayerCallback = Option<unsafe extern "C" fn(*mut c_void, *const LocalPlayerSnapshot) -> Status>;
pub type SnapshotFrameCallback = Option<unsafe extern "C" fn(*mut c_void, u32, i32, ByteView) -> Status>;
pub type SnapshotReportCallback = Option<unsafe extern "C" fn(*mut c_void, *const SnapshotReport) -> Status>;
pub type AccountCallback = Option<unsafe extern "C" fn(*mut c_void, *const Account) -> Status>;

include!(concat!(env!("OUT_DIR"), "/abi_generated.rs"));

#[repr(C)]
#[derive(Clone, Copy)]
pub struct ModDescriptor {
    pub header: TableHeader,
    pub build_fingerprint: u64,
    pub yoake_version: StrView,
    pub nightly: StrView,
    pub target: StrView,
    pub id: StrView,
    pub version: StrView,
    pub required_services: u64,
    pub side: u32,
    pub flags: u32,
    pub hooks: *const HookTableV1,
    pub create: Option<unsafe extern "C" fn(*const HostTableV1, Handle, *mut *mut c_void) -> Status>,
    pub destroy: Option<unsafe extern "C" fn(*mut c_void) -> Status>,
}

pub type ModDescriptorV2 = ModDescriptor;
pub type ModEntry = unsafe extern "C" fn(u32, *const HostTableV1) -> *const ModDescriptor;

#[cfg(test)]
mod tests {
    use super::*;
    use core::mem::{align_of, offset_of};

    #[test]
    fn layout_is_c_and_versioned() {
        assert_eq!(size_of::<TableHeader>(), 16);
        assert_eq!(size_of::<Handle>(), 16);
        assert_eq!(size_of::<HookResult>(), 8);
        assert_eq!(offset_of!(HookTableV1, client_tick), 16);
        assert_eq!(offset_of!(HostTableV1, services), 16);
        assert_eq!(offset_of!(HostTableV1, log), 24);
        assert_eq!(offset_of!(ModDescriptor, build_fingerprint), 16);
        assert_eq!(size_of::<ByteView>(), 2 * size_of::<usize>());
        assert_eq!(size_of::<OutputBuffer>(), 3 * size_of::<usize>());
        assert_eq!(align_of::<HookTableV1>(), align_of::<u64>());
        assert_eq!(ABI_VERSION, 2);
        let mut header = TableHeader::for_type::<HookTableV1>();
        assert!(header.valid_for::<HookTableV1>());
        header.abi_version += 1;
        assert!(!header.valid_for::<HookTableV1>());
        header = TableHeader::for_type::<HookTableV1>();
        header.schema_hash ^= 1;
        assert!(!header.valid_for::<HookTableV1>());
        header = TableHeader::for_type::<HookTableV1>();
        header.size += 1;
        assert!(!header.valid_for::<HookTableV1>());
    }

    #[test]
    fn views_and_caller_owned_buffers() {
        assert_eq!(unsafe { StrView::new("hello").as_str(5) }, Ok("hello"));
        assert_eq!(unsafe { StrView::new("hello").as_str(4) }, Err(Status::INVALID));
        let mut bytes = [0; 2];
        let mut output = OutputBuffer::new(&mut bytes);
        assert_eq!(unsafe { output.write(b"hello") }, Status::BUFFER_TOO_SMALL);
        assert_eq!(output.len, 5);
        assert_eq!(bytes, [0; 2]);
        assert_eq!(unsafe { output.write(b"ok") }, Status::OK);
        assert_eq!(bytes, *b"ok");
        assert_eq!(unsafe { ByteView { ptr: ptr::null(), len: 1 }.as_slice(1) }, Err(Status::INVALID));
        assert!(!Handle::default().is_valid());
    }
}
