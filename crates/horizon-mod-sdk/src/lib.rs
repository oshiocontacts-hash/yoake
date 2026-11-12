//! Rust-side DLL facade. Implement YoakeMod and call `yoake_mod!(MyMod)` in a
//! cdylib. Only the generated C tables cross the library boundary. Build with
//! panic=unwind; aborting panics and native memory faults are not recoverable.

pub use horizon_mod_abi as abi;
use abi::{Handle, HookResult, HostTableV1, Status, StrView};
use std::{ffi::c_void, marker::PhantomData, panic::AssertUnwindSafe, rc::Rc};

pub trait YoakeMod: Default + Send + 'static {
    const ID: &'static str;
    const VERSION: &'static str;
    const REQUIRED_SERVICES: u64 = abi::ALL_SERVICES;

    fn client_tick(&mut self, _host: &mut Host<'_>) {}
    fn key_input(&mut self, _host: &mut Host<'_>, _key: &str, _pressed: bool) {}
    fn hud(&mut self, _host: &mut Host<'_>, _batch: HudBatch<'_>) {}
    fn mod_net_receive(&mut self, _host: &mut Host<'_>, _channel: &str, _data: &[u8]) {}
}

pub struct Host<'call> {
    table: HostTableV1,
    handle: Handle,
    lifetime: PhantomData<&'call mut ()>,
    thread: PhantomData<Rc<()>>,
}

pub struct HudBatch<'call> {
    handle: Handle,
    lifetime: PhantomData<&'call mut ()>,
    thread: PhantomData<Rc<()>>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum WorldKey {
    Singleplayer(String),
    Server(String),
    Lan(String),
    Friend,
    Replay(String),
}

#[derive(Debug, PartialEq, Eq)]
pub struct SendError {
    pub status: Status,
    pub message: String,
}

fn ok(status: Status) -> Result<(), Status> {
    if status == Status::OK { Ok(()) } else { Err(status) }
}

fn read_output(mut service: impl FnMut(&mut abi::OutputBuffer) -> Status) -> Result<Option<String>, Status> {
    let mut bytes = vec![0; 256];
    for _attempt in 0..3 {
        let mut output = abi::OutputBuffer::new(&mut bytes);
        let status = service(&mut output);
        if status == Status::NOT_FOUND {
            return Ok(None);
        }
        if output.len > abi::MAX_OUTPUT_BYTES {
            return Err(Status::INVALID);
        }
        if status == Status::BUFFER_TOO_SMALL && output.len > bytes.len() {
            bytes.resize(output.len, 0);
            continue;
        }
        ok(status)?;
        if output.len > bytes.len() {
            return Err(Status::INVALID);
        }
        bytes.truncate(output.len);
        return String::from_utf8(bytes).map(Some).map_err(|_| Status::INVALID);
    }
    Err(Status::BUSY)
}

impl Host<'_> {
    pub fn feedback(&mut self, text: &str) -> Result<(), Status> {
        let service = self.table.feedback.ok_or(Status::UNSUPPORTED)?;
        ok(unsafe { service(self.handle, StrView::new(text)) })
    }

    pub fn client_world_key(&mut self) -> Result<Option<WorldKey>, Status> {
        let service = self.table.client_world_key.ok_or(Status::UNSUPPORTED)?;
        let mut kind = 0;
        let value = read_output(|output| unsafe { service(self.handle, &mut kind, output) })?;
        let Some(value) = value else { return Ok(None) };
        Ok(Some(match kind {
            abi::WORLD_SINGLEPLAYER => WorldKey::Singleplayer(value),
            abi::WORLD_SERVER => WorldKey::Server(value),
            abi::WORLD_LAN => WorldKey::Lan(value),
            abi::WORLD_FRIEND => WorldKey::Friend,
            abi::WORLD_REPLAY => WorldKey::Replay(value),
            _ => return Err(Status::UNSUPPORTED),
        }))
    }

    pub fn setting(&mut self, key: &str) -> Result<Option<String>, Status> {
        let service = self.table.setting_get.ok_or(Status::UNSUPPORTED)?;
        read_output(|output| unsafe { service(self.handle, StrView::new(key), output) })
    }

    pub fn set_setting(&mut self, key: &str, value: &str) -> Result<(), Status> {
        let service = self.table.setting_set.ok_or(Status::UNSUPPORTED)?;
        ok(unsafe { service(self.handle, StrView::new(key), StrView::new(value)) })
    }

    pub fn mod_net_send(&mut self, channel: &str, bytes: &[u8]) -> Result<(), SendError> {
        let Some(service) = self.table.mod_net_send else {
            return Err(SendError { status: Status::UNSUPPORTED, message: String::new() });
        };
        let mut error_bytes = [0; 4096];
        let mut output = abi::OutputBuffer::new(&mut error_bytes);
        let status = unsafe { service(self.handle, StrView::new(channel), abi::ByteView::new(bytes), &mut output) };
        if status == Status::OK {
            return Ok(());
        }
        let message = if output.len <= error_bytes.len() {
            String::from_utf8_lossy(&error_bytes[..output.len]).into_owned()
        } else {
            String::from("Host error message exceeds the supplied output buffer")
        };
        Err(SendError { status, message })
    }

    pub fn hud_text(&mut self, batch: &HudBatch<'_>, text: &str, at: [f32; 2], rgb: u32, shadow: bool) -> Result<(), Status> {
        let service = self.table.hud_text.ok_or(Status::UNSUPPORTED)?;
        ok(unsafe { service(self.handle, batch.handle, StrView::new(text), abi::Point { x: at[0], y: at[1] }, rgb, u32::from(shadow)) })
    }

    pub fn hud_rect(&mut self, batch: &HudBatch<'_>, rect: [f32; 4], argb: u32) -> Result<(), Status> {
        let service = self.table.hud_rect.ok_or(Status::UNSUPPORTED)?;
        ok(unsafe { service(self.handle, batch.handle, abi::Rect { x: rect[0], y: rect[1], width: rect[2], height: rect[3] }, argb) })
    }
}

#[doc(hidden)]
pub mod exports {
    use super::*;

    pub struct DescriptorStorage(pub abi::ModDescriptor);

    /// The export macro only stores immutable descriptors backed by static data.
    unsafe impl Sync for DescriptorStorage {}

    struct Instance<Mod> {
        value: Mod,
        table: HostTableV1,
        disabled: bool,
    }

    fn caught<ResultValue>(operation: impl FnOnce() -> ResultValue) -> Result<ResultValue, ()> {
        match std::panic::catch_unwind(AssertUnwindSafe(operation)) {
            Ok(result) => Ok(result),
            Err(payload) => {
                std::mem::forget(payload);
                Err(())
            }
        }
    }

    fn log_disabled(table: HostTableV1, id: &str) {
        if let Some(log) = table.log {
            let _ = caught(|| unsafe { log(StrView::new(id), StrView::new("DLL mod disabled: Rust panic at ABI boundary")) });
        }
    }

    unsafe fn checked_host(table: *const HostTableV1) -> Result<HostTableV1, Status> {
        if table.is_null() || !table.is_aligned() {
            return Err(Status::INVALID);
        }
        let header = unsafe { table.cast::<abi::TableHeader>().read() };
        if !header.valid_for::<HostTableV1>() {
            return Err(Status::INVALID);
        }
        let table = unsafe { table.read() };
        if table.services & !abi::ALL_SERVICES != 0 {
            return Err(Status::UNSUPPORTED);
        }
        for (mask, present) in [
            (abi::SERVICE_LOG, table.log.is_some()),
            (abi::SERVICE_FEEDBACK, table.feedback.is_some()),
            (abi::SERVICE_CLIENT_WORLD_KEY, table.client_world_key.is_some()),
            (abi::SERVICE_HUD_TEXT, table.hud_text.is_some()),
            (abi::SERVICE_HUD_RECT, table.hud_rect.is_some()),
            (abi::SERVICE_MOD_NET_SEND, table.mod_net_send.is_some()),
            (abi::SERVICE_SETTING_GET, table.setting_get.is_some()),
            (abi::SERVICE_SETTING_SET, table.setting_set.is_some()),
        ] {
            if (table.services & mask != 0) != present {
                return Err(Status::INVALID);
            }
        }
        Ok(table)
    }

    /// # Safety
    /// The host table and descriptor must satisfy the ABI memory contract.
    pub unsafe fn entry(version: u32, table: *const HostTableV1, descriptor: &'static DescriptorStorage) -> *const abi::ModDescriptor {
        caught(|| {
            if version != abi::ABI_VERSION {
                return std::ptr::null();
            }
            let Ok(table) = (unsafe { checked_host(table) }) else { return std::ptr::null() };
            if descriptor.0.required_services & !table.services != 0 {
                return std::ptr::null();
            }
            &descriptor.0 as *const _
        }).unwrap_or(std::ptr::null())
    }

    /// # Safety
    /// Called once with valid host memory and an exclusively writable output.
    pub unsafe extern "C" fn create<Mod: YoakeMod>(table: *const HostTableV1, handle: Handle, output: *mut *mut c_void) -> Status {
        if output.is_null() || !output.is_aligned() {
            return Status::INVALID;
        }
        unsafe { output.write(std::ptr::null_mut()) };
        let result = caught(|| {
            let table = unsafe { checked_host(table)? };
            if !handle.is_valid() || Mod::REQUIRED_SERVICES & !table.services != 0 {
                return Err(Status::INVALID);
            }
            let instance = Box::new(Instance { value: Mod::default(), table, disabled: false });
            unsafe { output.write(Box::into_raw(instance).cast()) };
            Ok(Status::OK)
        });
        match result {
            Ok(Ok(status)) => status,
            Ok(Err(status)) => status,
            Err(()) => {
                if let Ok(table) = unsafe { checked_host(table) } {
                    log_disabled(table, Mod::ID);
                }
                Status::PANIC
            }
        }
    }

    unsafe fn invoke<Mod: YoakeMod>(instance: *mut c_void, handle: Handle, operation: impl FnOnce(&mut Mod, &mut Host<'_>) -> Result<(), Status>) -> HookResult {
        let instance = instance.cast::<Instance<Mod>>();
        if instance.is_null() || !instance.is_aligned() || !handle.is_valid() {
            return HookResult::failed(Status::INVALID);
        }
        let instance = unsafe { &mut *instance };
        if instance.disabled {
            return HookResult::failed(Status::DISABLED);
        }
        let result = caught(|| {
            let mut host = Host { table: instance.table, handle, lifetime: PhantomData, thread: PhantomData };
            operation(&mut instance.value, &mut host)
        });
        match result {
            Ok(Ok(())) => HookResult::OK,
            Ok(Err(status)) => HookResult::failed(status),
            Err(()) => {
                instance.disabled = true;
                log_disabled(instance.table, Mod::ID);
                HookResult::failed(Status::PANIC)
            }
        }
    }

    unsafe fn event<'call, Event>(event: *const Event) -> Result<&'call Event, Status> {
        if event.is_null() || !event.is_aligned() {
            return Err(Status::INVALID);
        }
        Ok(unsafe { &*event })
    }

    /// # Safety
    /// The instance, handle and event obey the ABI contract; calls are serialized.
    pub unsafe extern "C" fn client_tick<Mod: YoakeMod>(instance: *mut c_void, handle: Handle, input: *const abi::TickEvent) -> HookResult {
        unsafe { invoke::<Mod>(instance, handle, |value, host| {
            if event(input)?.reserved != 0 { return Err(Status::INVALID) }
            value.client_tick(host);
            Ok(())
        }) }
    }

    /// # Safety
    /// The instance, handle and borrowed UTF-8 event obey the ABI contract.
    pub unsafe extern "C" fn key_input<Mod: YoakeMod>(instance: *mut c_void, handle: Handle, input: *const abi::KeyEvent) -> HookResult {
        unsafe { invoke::<Mod>(instance, handle, |value, host| {
            let input = event(input)?;
            if input.pressed > 1 { return Err(Status::INVALID) }
            value.key_input(host, input.key.as_str(abi::MAX_TEXT_BYTES)?, input.pressed != 0);
            Ok(())
        }) }
    }

    /// # Safety
    /// The instance, handle and invocation-scoped HUD event obey the ABI contract.
    pub unsafe extern "C" fn hud<Mod: YoakeMod>(instance: *mut c_void, handle: Handle, input: *const abi::HudEvent) -> HookResult {
        unsafe { invoke::<Mod>(instance, handle, |value, host| {
            let input = event(input)?;
            if !input.batch.is_valid() { return Err(Status::INVALID) }
            value.hud(host, HudBatch { handle: input.batch, lifetime: PhantomData, thread: PhantomData });
            Ok(())
        }) }
    }

    /// # Safety
    /// The instance, handle and borrowed byte event obey the ABI contract.
    pub unsafe extern "C" fn mod_net_receive<Mod: YoakeMod>(instance: *mut c_void, handle: Handle, input: *const abi::NetEvent) -> HookResult {
        unsafe { invoke::<Mod>(instance, handle, |value, host| {
            let input = event(input)?;
            value.mod_net_receive(host, input.channel.as_str(abi::MAX_TEXT_BYTES)?, input.data.as_slice(abi::MAX_DATA_BYTES)?);
            Ok(())
        }) }
    }

    /// # Safety
    /// Called exactly once for a created instance, after all callbacks have ended.
    pub unsafe extern "C" fn destroy<Mod: YoakeMod>(instance: *mut c_void) -> Status {
        let instance = instance.cast::<Instance<Mod>>();
        if instance.is_null() || !instance.is_aligned() {
            return Status::INVALID;
        }
        let table = unsafe { (*instance).table };
        if caught(|| unsafe { drop(Box::from_raw(instance)) }).is_err() {
            log_disabled(table, Mod::ID);
            Status::PANIC
        } else {
            Status::OK
        }
    }
}

#[macro_export]
macro_rules! yoake_mod {
    ($mod_type:ty) => {
        #[unsafe(no_mangle)]
        pub unsafe extern "C" fn yoake_mod_entry(
            abi_version: u32,
            host_table: *const $crate::abi::HostTableV1,
        ) -> *const $crate::abi::ModDescriptor {
            static HOOKS: $crate::abi::HookTableV1 = $crate::abi::HookTableV1 {
                header: $crate::abi::TableHeader::for_type::<$crate::abi::HookTableV1>(),
                client_tick: Some($crate::exports::client_tick::<$mod_type>),
                key_input: Some($crate::exports::key_input::<$mod_type>),
                hud: Some($crate::exports::hud::<$mod_type>),
                mod_net_receive: Some($crate::exports::mod_net_receive::<$mod_type>),
            };
            static DESCRIPTOR: $crate::exports::DescriptorStorage = $crate::exports::DescriptorStorage($crate::abi::ModDescriptor {
                header: $crate::abi::TableHeader::for_type::<$crate::abi::ModDescriptor>(),
                build_fingerprint: $crate::abi::BUILD_FINGERPRINT,
                yoake_version: $crate::abi::StrView::new($crate::abi::YOAKE_VERSION),
                nightly: $crate::abi::StrView::new($crate::abi::YOAKE_NIGHTLY),
                target: $crate::abi::StrView::new($crate::abi::TARGET),
                id: $crate::abi::StrView::new(<$mod_type as $crate::YoakeMod>::ID),
                version: $crate::abi::StrView::new(<$mod_type as $crate::YoakeMod>::VERSION),
                required_services: <$mod_type as $crate::YoakeMod>::REQUIRED_SERVICES,
                side: $crate::abi::SIDE_CLIENT,
                flags: 0,
                hooks: &HOOKS,
                create: Some($crate::exports::create::<$mod_type>),
                destroy: Some($crate::exports::destroy::<$mod_type>),
            });
            unsafe { $crate::exports::entry(abi_version, host_table, &DESCRIPTOR) }
        }
    };
}

#[cfg(test)]
mod tests;
