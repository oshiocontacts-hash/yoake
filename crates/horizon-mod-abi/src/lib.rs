//! Phase-one, native-width C ABI. There are no engine dependencies or Rust trait
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
//! valid in HUD callbacks. Byte/string views returned by a mod are copied by the
//! host before returning to engine dispatch.
//!
//! Read and validate the fixed TableHeader prefix before any larger table. V1
//! requires exact sizes, schema, target, release/nightly strings and engine source
//! fingerprint; it does not promise compatibility across engine versions or
//! architectures. The fingerprint detects mismatches, not tampering or signatures.
//! Descriptor and hook table memory must remain immutable for the library's whole
//! lifetime. `create` transfers a mod-owned opaque instance to the host; only that
//! mod's `destroy` may free it. The host serializes calls to each instance and
//! retains loaded libraries until process exit; there is no hot reload.
//!
//! Extern functions must never unwind. The SDK catches Rust unwinding panics and
//! logs/disables that instance; panic=abort, access violations, foreign exceptions
//! and malicious pointers cannot be recovered by catch_unwind. Mods must use an
//! unwinding panic strategy. Unknown status/flow values fail closed. All four V1
//! hooks listen; CANCEL/REPLACE are reserved for future marked catalog contracts.

use core::{ffi::c_void, mem::size_of, ptr, slice, str};

pub const MAX_TEXT_BYTES: usize = 65_536;
pub const MAX_DATA_BYTES: usize = 30_000;
pub const MAX_OUTPUT_BYTES: usize = 1_048_576;
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
        assert_eq!(ABI_VERSION, 1);
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
