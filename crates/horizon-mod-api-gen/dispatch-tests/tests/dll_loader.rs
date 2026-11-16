use horizon_mod_abi as abi;
use horizon_mod_sdk::{Host, HudBatch, WorldKey, YoakeMod, exports};
use std::sync::Arc;

extern crate self as wgpu;

pub enum TextureFormat { Rgba8Unorm, Bgra8Unorm, Rgba16Float }

#[path = "support/dll_host.rs"]
mod mod_api;

#[derive(Clone, Copy)]
pub enum NativeSide {
    Client,
    Both,
}

#[derive(Clone, Copy)]
pub struct NativeMod {
    pub id: &'static str,
    pub version: &'static str,
    pub side: NativeSide,
}

#[path = "support/yak_stub.rs"]
mod yak_mods;

/// The loader asks whether this is a verified test worker; never, here.
mod yoake_test {
    pub(crate) fn is_worker() -> bool {
        false
    }
}

#[path = "../../../horizon-client/src/dll_mods.rs"]
mod dll_mods;

#[path = "support/dll_phase2.rs"]
mod phase2;

#[cfg(windows)]
#[path = "support/dll_windows.rs"]
mod windows;

unsafe extern "C" fn entry<Mod: YoakeMod>(
    version: u32,
    host: *const abi::HostTableV1,
) -> *const abi::ModDescriptor {
    let hooks = Box::leak(Box::new(exports::hooks::<Mod>()));
    let descriptor = Box::leak(Box::new(exports::DescriptorStorage(abi::ModDescriptor {
        header: abi::TableHeader::for_type::<abi::ModDescriptor>(),
        build_fingerprint: abi::BUILD_FINGERPRINT,
        yoake_version: abi::StrView::new(abi::YOAKE_VERSION),
        nightly: abi::StrView::new(abi::YOAKE_NIGHTLY),
        target: abi::StrView::new(abi::TARGET),
        id: abi::StrView::new(Mod::ID),
        version: abi::StrView::new(Mod::VERSION),
        required_services: Mod::REQUIRED_SERVICES,
        side: abi::SIDE_CLIENT,
        flags: 0,
        hooks,
        create: Some(exports::create::<Mod>),
        destroy: Some(exports::destroy::<Mod>),
    })));
    unsafe { exports::entry(version, host, descriptor) }
}

struct Static(&'static str);

impl mod_api::Hooks for Static {
    fn tick(&self, host: &mut dyn mod_api::Host, _event: &mut mod_api::TickEvent) {
        host.feedback(mod_api::Text::literal(self.0));
    }

    fn cancel(&self, visits: &mut Vec<&'static str>) -> bool {
        visits.push(self.0);
        self.0 == "static-b"
    }

    fn replace(&self) -> Option<&'static str> {
        Some(self.0)
    }

    fn collect(&self, values: &mut Vec<&'static str>) {
        values.push(self.0);
    }
}

#[derive(Default)]
struct First {
    ticks: u64,
}

impl YoakeMod for First {
    const ID: &'static str = "dll-a";
    const VERSION: &'static str = "1";

    fn client_tick(&mut self, host: &mut Host<'_>) {
        self.ticks += 1;
        host.feedback(&format!("dll-a:{}", self.ticks)).unwrap();
        assert_eq!(host.client_world_key(), Ok(Some(WorldKey::Server("a".repeat(512)))));
        assert_eq!(host.setting("visible"), Ok((self.ticks > 1).then(|| "true".into())));
        host.set_setting("visible", "true").unwrap();
        assert_eq!(host.set_setting("../escape", "true"), Err(abi::Status::INVALID));
    }

    fn key_input(&mut self, host: &mut Host<'_>, key: &str, pressed: bool) {
        host.feedback(&format!("{key}:{pressed}")).unwrap();
    }

    fn hud(&mut self, host: &mut Host<'_>, batch: HudBatch<'_>) {
        host.hud_text(&batch, "tick HUD", [2.0, 3.0], 0xffffff, true).unwrap();
        host.hud_rect(&batch, [1.0, 2.0, 60.0, 18.0], 0x90000000).unwrap();
    }

    fn mod_net_receive(&mut self, host: &mut Host<'_>, channel: &str, data: &[u8]) {
        host.mod_net_send(channel, data).unwrap();
    }
}

#[derive(Default)]
struct Second;

impl YoakeMod for Second {
    const ID: &'static str = "dll-b";
    const VERSION: &'static str = "2";

    fn client_tick(&mut self, host: &mut Host<'_>) {
        host.feedback("dll-b").unwrap();
    }
}

fn static_mods() -> mod_api::Mods {
    Arc::from(vec![
        Box::new(Static("static-a")) as Box<dyn mod_api::Hooks>,
        Box::new(Static("static-b")),
    ])
}

fn tick(mods: &mod_api::Mods, host: &mut mod_api::FakeHost) {
    for native_mod in mods.iter() {
        native_mod.tick(host, &mut mod_api::TickEvent);
    }
}

#[test]
fn production_loader_runs_static_then_dll_with_owned_identities() {
    let (mods, identities) = unsafe {
        dll_mods::in_process(static_mods(), &[], &[entry::<First>, entry::<Second>]).unwrap()
    };
    assert_eq!(identities.iter().map(|identity| identity.id).collect::<Vec<_>>(), ["dll-a", "dll-b"]);
    assert_eq!(identities[1].version, "2");
    assert!(identities.iter().all(|identity| matches!(identity.side, NativeSide::Client)));
    let mut host = mod_api::FakeHost { client: true, ..Default::default() };
    tick(&mods, &mut host);
    tick(&mods, &mut host);
    assert_eq!(host.feedback, ["static-a", "static-b", "dll-a:1", "dll-b", "static-a", "static-b", "dll-a:2", "dll-b"]);
    assert_eq!(host.settings.lock().unwrap().len(), 1);
}

#[test]
fn key_hud_bytes_and_settings_round_trip_through_real_adapters() {
    let (mods, _) = unsafe { dll_mods::in_process(Arc::from([]), &[], &[entry::<First>]).unwrap() };
    let mut host = mod_api::FakeHost { client: true, ..Default::default() };
    let mut batch = mod_api::HudBatch::default();
    for native_mod in mods.iter() {
        native_mod.key(&mut host, &mut mod_api::KeyEvent { key: "key.keyboard.f8", pressed: true });
        native_mod.hud(&mut host, &mut mod_api::HudEvent { batch: &mut batch });
        native_mod.net_from_server(&mut host, &mut mod_api::NetFromServerEvent { channel: "test:echo", data: &[0, 255, 1, 0] });
    }
    tick(&mods, &mut host);
    assert_eq!(host.feedback, ["key.keyboard.f8:true", "dll-a:1"]);
    assert_eq!(batch.text, ["tick HUD"]);
    assert_eq!(batch.rects, [[1.0, 2.0, 60.0, 18.0]]);
    assert_eq!(host.sends, [("test:echo".into(), vec![0, 255, 1, 0])]);
    assert_eq!(host.settings.lock().unwrap().get(&("dll-a".into(), "visible".into())).map(String::as_str), Some("true"));
}

#[test]
fn unoffered_contracts_keep_static_cancel_replace_and_collect_semantics() {
    let (mods, _) = unsafe {
        dll_mods::in_process(static_mods(), &[], &[entry::<Second>]).unwrap()
    };
    let mut visits = Vec::new();
    for native_mod in mods.iter() {
        if native_mod.cancel(&mut visits) { break; }
    }
    assert_eq!(visits, ["static-a", "static-b"]);
    assert_eq!(mods.iter().find_map(|native_mod| native_mod.replace()), Some("static-a"));
    let mut values = Vec::new();
    for native_mod in mods.iter() { native_mod.collect(&mut values); }
    assert_eq!(values, ["static-a", "static-b"]);
    let mut host = mod_api::FakeHost::default();
    tick(&mods, &mut host);
    assert_eq!(host.feedback, ["static-a", "static-b"]);
}

#[derive(Default)]
struct Panics;

impl YoakeMod for Panics {
    const ID: &'static str = "panics";
    const VERSION: &'static str = "1";

    fn client_tick(&mut self, _host: &mut Host<'_>) {
        panic!("isolated DLL hook panic");
    }
}

#[test]
fn panic_disables_only_one_dll_and_later_mods_keep_running() {
    let (mods, _) = unsafe {
        dll_mods::in_process(Arc::from([]), &[], &[entry::<Panics>, entry::<Second>]).unwrap()
    };
    let mut host = mod_api::FakeHost { client: true, ..Default::default() };
    tick(&mods, &mut host);
    tick(&mods, &mut host);
    assert_eq!(host.feedback, ["dll-b", "dll-b"]);
}

#[derive(Default)]
struct HostPanic;

impl YoakeMod for HostPanic {
    const ID: &'static str = "host-panic";
    const VERSION: &'static str = "1";

    fn client_tick(&mut self, host: &mut Host<'_>) {
        let _ignored_by_mod = host.feedback("panics in host");
    }
}

#[test]
fn host_panic_disables_the_caller_even_if_it_ignores_the_status() {
    let (mods, _) = unsafe {
        dll_mods::in_process(Arc::from([]), &[], &[entry::<HostPanic>, entry::<Second>]).unwrap()
    };
    let mut host = mod_api::FakeHost { client: true, panic_feedback: true, ..Default::default() };
    mods[0].tick(&mut host, &mut mod_api::TickEvent);
    host.panic_feedback = false;
    tick(&mods, &mut host);
    assert_eq!(host.feedback, ["dll-b"]);
}

thread_local! {
    static MUTATION: std::cell::Cell<u32> = const { std::cell::Cell::new(0) };
}

unsafe extern "C" fn incompatible_entry(
    version: u32,
    host: *const abi::HostTableV1,
) -> *const abi::ModDescriptor {
    let mut descriptor = unsafe { entry::<Second>(version, host).read() };
    MUTATION.with(|mutation| match mutation.get() {
        0 => descriptor.yoake_version = abi::StrView::new("wrong release"),
        1 => descriptor.nightly = abi::StrView::new("wrong nightly"),
        2 => descriptor.target = abi::StrView::new("wrong target"),
        3 => descriptor.build_fingerprint ^= 1,
        4 => descriptor.required_services = u64::MAX,
        5 => descriptor.flags = 1,
        6 => descriptor.create = None,
        7 => descriptor.hooks = std::ptr::null(),
        _ => descriptor.id = abi::StrView::new("../escape"),
    });
    Box::into_raw(Box::new(descriptor))
}

#[test]
fn descriptor_rejects_release_nightly_target_build_capabilities_and_invalid_exports() {
    for mutation in 0..9 {
        MUTATION.with(|value| value.set(mutation));
        let result = unsafe {
            dll_mods::in_process(Arc::from([]), &[], &[incompatible_entry])
        };
        assert!(result.is_err(), "accepted mutation {mutation}");
    }
}

thread_local! {
    static PROBE_TABLE: std::cell::Cell<Option<abi::HostTableV1>> = const { std::cell::Cell::new(None) };
    static PREVIOUS_HOST: std::cell::Cell<Option<abi::Handle>> = const { std::cell::Cell::new(None) };
    static PROBE_STATUSES: std::cell::RefCell<Vec<abi::Status>> = const { std::cell::RefCell::new(Vec::new()) };
}

unsafe extern "C" fn probe_tick(
    _instance: *mut core::ffi::c_void,
    handle: abi::Handle,
    _event: *const abi::TickEvent,
) -> abi::HookResult {
    let Some(table) = PROBE_TABLE.get() else { return abi::HookResult::failed(abi::Status::INVALID) };
    let (Some(feedback), Some(text)) = (table.feedback, table.hud_text) else {
        return abi::HookResult::failed(abi::Status::UNSUPPORTED);
    };
    let mut statuses = vec![unsafe { feedback(handle, abi::StrView::new("valid handle")) }];
    if let Some(previous) = PREVIOUS_HOST.replace(Some(handle)) {
        statuses.push(unsafe { feedback(previous, abi::StrView::new("stale handle")) });
    }
    statuses.push(unsafe {
        feedback(abi::Handle { generation: handle.generation + 1, ..handle }, abi::StrView::new("wrong generation"))
    });
    statuses.push(unsafe {
        text(handle, abi::Handle { id: handle.id | (1 << 63), ..handle }, abi::StrView::new("outside HUD"), abi::Point { x: 1.0, y: 2.0 }, 0xffffff, 0)
    });
    statuses.push(std::thread::spawn(move || unsafe {
        feedback(handle, abi::StrView::new("wrong thread"))
    }).join().unwrap_or(abi::Status::PANIC));
    PROBE_STATUSES.with(|saved| saved.borrow_mut().extend(statuses));
    abi::HookResult::OK
}

unsafe extern "C" fn probe_hud(
    _instance: *mut core::ffi::c_void,
    handle: abi::Handle,
    event: *const abi::HudEvent,
) -> abi::HookResult {
    let Some(table) = PROBE_TABLE.get() else { return abi::HookResult::failed(abi::Status::INVALID) };
    let Some(text) = table.hud_text else { return abi::HookResult::failed(abi::Status::UNSUPPORTED) };
    let event = unsafe { event.read() };
    let statuses = unsafe {
        [
            text(handle, handle, abi::StrView::new("wrong batch"), abi::Point { x: 1.0, y: 2.0 }, 0xffffff, 0),
            text(handle, event.batch, abi::StrView::new("valid HUD handle"), abi::Point { x: 1.0, y: 2.0 }, 0xffffff, 0),
        ]
    };
    PROBE_STATUSES.with(|saved| saved.borrow_mut().extend(statuses));
    abi::HookResult::OK
}

unsafe extern "C" fn probe_entry(
    version: u32,
    host: *const abi::HostTableV1,
) -> *const abi::ModDescriptor {
    PROBE_TABLE.set(Some(unsafe { host.read() }));
    let mut descriptor = unsafe { entry::<Second>(version, host).read() };
    descriptor.id = abi::StrView::new("handle-probe");
    descriptor.hooks = Box::into_raw(Box::new(abi::HookTableV1 {
        header: abi::TableHeader::for_type::<abi::HookTableV1>(),
        client_tick: Some(probe_tick),
        hud: Some(probe_hud),
        ..Default::default()
    }));
    Box::into_raw(Box::new(descriptor))
}

#[test]
fn handles_expire_between_hooks_and_reject_wrong_threads_and_hud_phases() {
    PREVIOUS_HOST.set(None);
    PROBE_STATUSES.with(|statuses| statuses.borrow_mut().clear());
    let (mods, _) = unsafe {
        dll_mods::in_process(Arc::from([]), &[], &[probe_entry]).unwrap()
    };
    let mut host = mod_api::FakeHost { client: true, ..Default::default() };
    tick(&mods, &mut host);
    tick(&mods, &mut host);
    let mut batch = mod_api::HudBatch::default();
    mods[0].hud(&mut host, &mut mod_api::HudEvent { batch: &mut batch });
    PROBE_STATUSES.with(|statuses| assert_eq!(statuses.borrow().as_slice(), [
        abi::Status::OK, abi::Status::INVALID, abi::Status::INVALID, abi::Status::INVALID,
        abi::Status::OK, abi::Status::INVALID, abi::Status::INVALID, abi::Status::INVALID, abi::Status::INVALID,
        abi::Status::INVALID, abi::Status::OK,
    ]));
    assert_eq!(host.feedback, ["valid handle", "valid handle"]);
    assert_eq!(batch.text, ["valid HUD handle"]);
}
