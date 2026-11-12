use super::*;
use std::cell::RefCell;

#[derive(Default)]
struct State {
    feedback: Vec<String>,
    setting: Option<String>,
    text: Vec<String>,
    rects: usize,
    sent: Vec<(String, Vec<u8>)>,
    logs: Vec<String>,
}

thread_local! {
    static STATE: RefCell<State> = RefCell::new(State::default());
}

const HANDLE: Handle = Handle { id: 1, generation: 7 };
const PHASE_ONE_SERVICES: u64 = (1 << 8) - 1;

fn reset() { STATE.with(|state| *state.borrow_mut() = State::default()); }

unsafe extern "C" fn log(_id: StrView, message: StrView) -> Status {
    let Ok(message) = (unsafe { message.as_str(abi::MAX_TEXT_BYTES) }) else { return Status::INVALID };
    STATE.with(|state| state.borrow_mut().logs.push(message.to_owned()));
    Status::OK
}

unsafe extern "C" fn feedback(handle: Handle, text: StrView) -> Status {
    if handle != HANDLE { return Status::INVALID; }
    let Ok(text) = (unsafe { text.as_str(abi::MAX_TEXT_BYTES) }) else { return Status::INVALID };
    STATE.with(|state| state.borrow_mut().feedback.push(text.to_owned()));
    Status::OK
}

unsafe extern "C" fn world(_handle: Handle, kind: *mut u32, output: *mut abi::OutputBuffer) -> Status {
    unsafe { *kind = abi::WORLD_SERVER; (*output).write("a".repeat(512).as_bytes()) }
}

unsafe extern "C" fn text(_handle: Handle, _batch: Handle, value: StrView, _at: abi::Point, _rgb: u32, _shadow: u32) -> Status {
    let Ok(value) = (unsafe { value.as_str(abi::MAX_TEXT_BYTES) }) else { return Status::INVALID };
    STATE.with(|state| state.borrow_mut().text.push(value.to_owned()));
    Status::OK
}

unsafe extern "C" fn rect(_handle: Handle, _batch: Handle, _rect: abi::Rect, _argb: u32) -> Status {
    STATE.with(|state| state.borrow_mut().rects += 1);
    Status::OK
}

unsafe extern "C" fn send(_handle: Handle, channel: StrView, data: abi::ByteView, output: *mut abi::OutputBuffer) -> Status {
    let Ok(channel) = (unsafe { channel.as_str(256) }) else { return Status::INVALID };
    let Ok(data) = (unsafe { data.as_slice(abi::MAX_DATA_BYTES) }) else { return Status::INVALID };
    STATE.with(|state| state.borrow_mut().sent.push((channel.to_owned(), data.to_vec())));
    unsafe { (*output).write(b"") }
}

unsafe extern "C" fn get(_handle: Handle, _key: StrView, output: *mut abi::OutputBuffer) -> Status {
    STATE.with(|state| match &state.borrow().setting {
        Some(value) => unsafe { (*output).write(value.as_bytes()) },
        None => Status::NOT_FOUND,
    })
}

unsafe extern "C" fn set(_handle: Handle, _key: StrView, value: StrView) -> Status {
    let Ok(value) = (unsafe { value.as_str(abi::MAX_TEXT_BYTES) }) else { return Status::INVALID };
    STATE.with(|state| state.borrow_mut().setting = Some(value.to_owned()));
    Status::OK
}

fn table() -> HostTableV1 {
    HostTableV1 {
        header: abi::TableHeader::for_type::<HostTableV1>(),
        services: PHASE_ONE_SERVICES,
        log: Some(log),
        feedback: Some(feedback),
        client_world_key: Some(world),
        hud_text: Some(text),
        hud_rect: Some(rect),
        mod_net_send: Some(send),
        setting_get: Some(get),
        setting_set: Some(set),
        ..Default::default()
    }
}

#[derive(Default)]
struct RoundTrip;

impl YoakeMod for RoundTrip {
    const ID: &'static str = "round-trip";
    const VERSION: &'static str = "1";
    const REQUIRED_SERVICES: u64 = PHASE_ONE_SERVICES;

    fn client_tick(&mut self, host: &mut Host<'_>) {
        assert_eq!(host.setting("key"), Ok(None));
        host.set_setting("key", "retained UTF-8: hello").unwrap();
        assert_eq!(host.setting("key"), Ok(Some("retained UTF-8: hello".into())));
        assert_eq!(host.client_world_key(), Ok(Some(WorldKey::Server("a".repeat(512)))));
        host.feedback("tick").unwrap();
    }

    fn key_input(&mut self, host: &mut Host<'_>, key: &str, pressed: bool) {
        host.feedback(&format!("{key}:{pressed}")).unwrap();
    }

    fn hud(&mut self, host: &mut Host<'_>, batch: HudBatch<'_>) {
        host.hud_text(&batch, "HUD", [1.0, 2.0], 0xffffff, true).unwrap();
        host.hud_rect(&batch, [1.0, 2.0, 3.0, 4.0], 0x78000000).unwrap();
    }

    fn mod_net_receive(&mut self, host: &mut Host<'_>, channel: &str, data: &[u8]) {
        host.mod_net_send(channel, data).unwrap();
    }
}

yoake_mod!(RoundTrip);

#[test]
fn exported_descriptor_round_trips_every_service_without_engine() {
    reset();
    let table = table();
    unsafe {
        let descriptor = &*yoake_mod_entry(abi::ABI_VERSION, &table);
        assert_eq!(descriptor.header.abi_version, abi::ABI_VERSION);
        assert_eq!(descriptor.id.as_str(64), Ok("round-trip"));
        let mut instance = std::ptr::null_mut();
        assert_eq!(descriptor.create.unwrap()(&table, HANDLE, &mut instance), Status::OK);
        let hooks = &*descriptor.hooks;
        assert_eq!(hooks.client_tick.unwrap()(instance, HANDLE, &abi::TickEvent::default()), HookResult::OK);
        let key = abi::KeyEvent { key: StrView::new("key.keyboard.f8"), pressed: 1 };
        assert_eq!(hooks.key_input.unwrap()(instance, HANDLE, &key), HookResult::OK);
        let hud = abi::HudEvent { batch: Handle { id: 2, generation: 7 } };
        assert_eq!(hooks.hud.unwrap()(instance, HANDLE, &hud), HookResult::OK);
        let message = abi::NetEvent { channel: StrView::new("test:echo"), data: abi::ByteView::new(&[0, 255, 1]) };
        assert_eq!(hooks.mod_net_receive.unwrap()(instance, HANDLE, &message), HookResult::OK);
        assert_eq!(descriptor.destroy.unwrap()(instance), Status::OK);
    }
    STATE.with(|state| {
        let state = state.borrow();
        assert_eq!(state.feedback, ["tick", "key.keyboard.f8:true"]);
        assert_eq!(state.text, ["HUD"]);
        assert_eq!(state.rects, 1);
        assert_eq!(state.sent, [("test:echo".into(), vec![0, 255, 1])]);
        assert!(state.logs.is_empty());
    });
}

#[test]
fn entry_refuses_version_schema_size_and_inconsistent_capabilities() {
    let mut table = table();
    unsafe {
        assert!(yoake_mod_entry(abi::ABI_VERSION + 1, &table).is_null());
        table.header.schema_hash ^= 1;
        assert!(yoake_mod_entry(abi::ABI_VERSION, &table).is_null());
        table = self::table();
        table.header.size -= 1;
        assert!(yoake_mod_entry(abi::ABI_VERSION, &table).is_null());
        table = self::table();
        table.feedback = None;
        assert!(yoake_mod_entry(abi::ABI_VERSION, &table).is_null());
        assert!(yoake_mod_entry(abi::ABI_VERSION, std::ptr::null()).is_null());
    }
}

#[derive(Default)]
struct Panics;

impl YoakeMod for Panics {
    const ID: &'static str = "panics";
    const VERSION: &'static str = "1";
    const REQUIRED_SERVICES: u64 = PHASE_ONE_SERVICES;
    fn client_tick(&mut self, _host: &mut Host<'_>) { panic!("test DLL panic"); }
}

#[test]
fn panicking_hook_disables_only_its_instance_and_logs_once() {
    reset();
    let table = table();
    unsafe {
        let mut failed = std::ptr::null_mut();
        let mut healthy = std::ptr::null_mut();
        assert_eq!(exports::create::<Panics>(&table, HANDLE, &mut failed), Status::OK);
        assert_eq!(exports::create::<RoundTrip>(&table, HANDLE, &mut healthy), Status::OK);
        let event = abi::TickEvent::default();
        assert_eq!(exports::client_tick::<Panics>(failed, HANDLE, &event).status, Status::PANIC);
        assert_eq!(exports::client_tick::<Panics>(failed, HANDLE, &event).status, Status::DISABLED);
        assert_eq!(exports::client_tick::<RoundTrip>(healthy, HANDLE, &event), HookResult::OK);
        assert_eq!(exports::destroy::<Panics>(failed), Status::OK);
        assert_eq!(exports::destroy::<RoundTrip>(healthy), Status::OK);
    }
    STATE.with(|state| assert_eq!(state.borrow().logs.len(), 1));
}

#[test]
fn oversized_read_output_is_rejected_without_retrying() {
    let mut calls = 0;
    let result = read_output(|output| {
        calls += 1;
        output.len = abi::MAX_OUTPUT_BYTES + 1;
        Status::BUFFER_TOO_SMALL
    });
    assert_eq!(result, Err(Status::INVALID));
    assert_eq!(calls, 1);
}

#[test]
fn failed_send_is_not_retried_for_a_larger_error_buffer() {
    unsafe extern "C" fn send_large_error(_handle: Handle, _channel: StrView, _data: abi::ByteView, output: *mut abi::OutputBuffer) -> Status {
        STATE.with(|state| state.borrow_mut().sent.push((String::new(), Vec::new())));
        unsafe { (*output).write(&[b'x'; 5000]) }
    }
    reset();
    let mut table = table();
    table.mod_net_send = Some(send_large_error);
    let mut host = Host { table, handle: HANDLE, lifetime: PhantomData, thread: PhantomData };
    let error = host.mod_net_send("test:once", b"payload").unwrap_err();
    assert_eq!(error.status, Status::BUFFER_TOO_SMALL);
    STATE.with(|state| assert_eq!(state.borrow().sent.len(), 1));
}

struct ConstructorPanic;

impl Default for ConstructorPanic {
    fn default() -> Self { panic!("constructor panic") }
}

impl YoakeMod for ConstructorPanic {
    const ID: &'static str = "constructor-panic";
    const VERSION: &'static str = "1";
    const REQUIRED_SERVICES: u64 = PHASE_ONE_SERVICES;
}

#[derive(Default)]
struct DestructorPanic;

impl Drop for DestructorPanic {
    fn drop(&mut self) { panic!("destructor panic") }
}

impl YoakeMod for DestructorPanic {
    const ID: &'static str = "destructor-panic";
    const VERSION: &'static str = "1";
    const REQUIRED_SERVICES: u64 = PHASE_ONE_SERVICES;
}

#[test]
fn constructor_and_destructor_panics_stay_inside_the_abi() {
    reset();
    let table = table();
    let mut failed = std::ptr::null_mut();
    let mut destructing = std::ptr::null_mut();
    unsafe {
        assert_eq!(exports::create::<ConstructorPanic>(&table, HANDLE, &mut failed), Status::PANIC);
        assert!(failed.is_null());
        assert_eq!(exports::create::<DestructorPanic>(&table, HANDLE, &mut destructing), Status::OK);
        assert_eq!(exports::destroy::<DestructorPanic>(destructing), Status::PANIC);
    }
    STATE.with(|state| assert_eq!(state.borrow().logs.len(), 2));
}
