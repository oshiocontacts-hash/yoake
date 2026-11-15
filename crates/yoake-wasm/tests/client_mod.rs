use std::sync::{Arc, Mutex};

use wit_parser::{ManglingAndAbi, Resolve};
use yoake_wasm::bindings::yoake::mods::types::HostError;
use yoake_wasm::{ClientHost, ClientMod, Sandbox};

#[derive(Default)]
struct FakeHost {
    calls: Arc<Mutex<Vec<String>>>,
}

impl ClientHost for FakeHost {
    fn mod_screen_open(&mut self, mod_id: &str) -> Result<bool, HostError> {
        self.calls.lock().unwrap().push(mod_id.to_owned());
        Ok(true)
    }
}

struct SlowHost {
    calls: Arc<Mutex<Vec<String>>>,
}

impl ClientHost for SlowHost {
    fn mod_screen_open(&mut self, mod_id: &str) -> Result<bool, HostError> {
        std::thread::sleep(std::time::Duration::from_millis(60));
        self.calls.lock().unwrap().push(mod_id.to_owned());
        Ok(true)
    }
}

fn read_leb(bytes: &[u8], position: &mut usize) -> u32 {
    let mut value = 0;
    let mut shift = 0;
    loop {
        let byte = bytes[*position];
        *position += 1;
        value |= u32::from(byte & 0x7f) << shift;
        if byte & 0x80 == 0 {
            return value;
        }
        shift += 7;
    }
}

fn write_leb(mut value: u32, bytes: &mut Vec<u8>) {
    loop {
        let mut byte = (value & 0x7f) as u8;
        value >>= 7;
        if value != 0 {
            byte |= 0x80;
        }
        bytes.push(byte);
        if value == 0 {
            return;
        }
    }
}

fn sections(module: &[u8]) -> Vec<(u8, Vec<u8>)> {
    let mut position = 8;
    let mut sections = Vec::new();
    while position < module.len() {
        let id = module[position];
        position += 1;
        let length = read_leb(module, &mut position) as usize;
        sections.push((id, module[position..position + length].to_vec()));
        position += length;
    }
    sections
}

fn replace_section(module: &[u8], target: u8, replacement: &[u8]) -> Vec<u8> {
    let mut updated = module[..8].to_vec();
    for (id, section) in sections(module) {
        let contents = if id == target { replacement } else { &section };
        updated.push(id);
        write_leb(contents.len() as u32, &mut updated);
        updated.extend_from_slice(contents);
    }
    updated
}

fn read_name<'a>(bytes: &'a [u8], position: &mut usize) -> &'a str {
    let length = read_leb(bytes, position) as usize;
    let name = std::str::from_utf8(&bytes[*position..*position + length]).unwrap();
    *position += length;
    name
}

fn function_indices(module: &[u8]) -> (u32, u32, u32, u32, u32) {
    let mut imported = 0;
    let mut open = None;
    let mut init = None;
    let mut tick = None;
    let mut frames_listened = None;
    for (id, section) in sections(module) {
        let mut position = 0;
        if id == 2 {
            let count = read_leb(&section, &mut position);
            for index in 0..count {
                let namespace = read_name(&section, &mut position);
                let name = read_name(&section, &mut position);
                assert_eq!(section[position], 0);
                position += 1;
                read_leb(&section, &mut position);
                if namespace.contains("/screen") && name == "mod-screen-open" {
                    open = Some(index);
                }
            }
            imported = count;
        } else if id == 7 {
            let count = read_leb(&section, &mut position);
            for _ in 0..count {
                let name = read_name(&section, &mut position);
                let kind = section[position];
                position += 1;
                let index = read_leb(&section, &mut position);
                if kind == 0 && name == "cm32p2||init" {
                    init = Some(index);
                } else if kind == 0 && name.ends_with("|tick") {
                    tick = Some(index);
                } else if kind == 0 && name.ends_with("|net-frames-listened") {
                    frames_listened = Some(index);
                }
            }
        }
    }
    (imported, open.unwrap(), init.unwrap(), tick.unwrap(), frames_listened.unwrap())
}

fn i32_const(body: &mut Vec<u8>, mut value: i32) {
    body.push(0x41);
    loop {
        let mut byte = (value & 0x7f) as u8;
        value >>= 7;
        let finished = (value == 0 && byte & 0x40 == 0)
            || (value == -1 && byte & 0x40 != 0);
        if !finished {
            byte |= 0x80;
        }
        body.push(byte);
        if finished {
            break;
        }
    }
}

fn store_i32(body: &mut Vec<u8>, address: i32, value: i32) {
    i32_const(body, address);
    i32_const(body, value);
    body.extend_from_slice(&[0x36, 2, 0]);
}

fn store_bytes(body: &mut Vec<u8>, address: i32, bytes: &[u8]) {
    for (offset, byte) in bytes.iter().enumerate() {
        i32_const(body, address + offset as i32);
        i32_const(body, i32::from(*byte));
        body.extend_from_slice(&[0x3a, 0, 0]);
    }
}

fn init_body(subscription: &str) -> Vec<u8> {
    let mut body = vec![0];
    store_i32(&mut body, 0, 16);
    store_i32(&mut body, 4, 1);
    store_i32(&mut body, 16, 32);
    store_i32(&mut body, 20, subscription.len() as i32);
    store_bytes(&mut body, 32, subscription.as_bytes());
    i32_const(&mut body, 0);
    body.push(0x0b);
    body
}

fn expect_byte(body: &mut Vec<u8>, address: i32, expected: i32) {
    i32_const(body, address);
    body.extend_from_slice(&[0x2d, 0, 0]);
    i32_const(body, expected);
    body.extend_from_slice(&[0x47, 0x04, 0x40, 0x00, 0x0b]);
}

fn screen_call(body: &mut Vec<u8>, function: u32, address: i32, length: i32) {
    i32_const(body, address);
    i32_const(body, length);
    i32_const(body, 128);
    body.push(0x10);
    write_leb(function, body);
}

fn screen_tick(function: u32, try_other_mod: bool) -> Vec<u8> {
    let mut body = vec![0];
    store_bytes(&mut body, 64, b"mine");
    if try_other_mod {
        store_bytes(&mut body, 80, b"other");
        screen_call(&mut body, function, 80, 5);
        expect_byte(&mut body, 128, 1);
        expect_byte(&mut body, 129, 0);
    }
    screen_call(&mut body, function, 64, 4);
    expect_byte(&mut body, 128, 0);
    expect_byte(&mut body, 129, 1);
    body.push(0x0b);
    body
}

fn component(subscription: &str, tick: impl FnOnce(u32) -> Vec<u8>) -> Vec<u8> {
    let mut resolve = Resolve::default();
    let package = resolve
        .push_str("yoake-mod.wit", include_str!("../wit/yoake-mod.wit"))
        .unwrap();
    let world = resolve.select_world(&[package], Some("client-mod")).unwrap();
    let module = wit_component::dummy_module(&resolve, world, ManglingAndAbi::Standard32);
    let (imported, open, init, tick_index, frames_listened) = function_indices(&module);
    let init_body = init_body(subscription);
    let tick_body = tick(open);
    let code = sections(&module)
        .into_iter()
        .find(|(id, _)| *id == 10)
        .unwrap()
        .1;
    let mut position = 0;
    let count = read_leb(&code, &mut position);
    let mut replacement = Vec::new();
    write_leb(count, &mut replacement);
    for index in 0..count {
        let length = read_leb(&code, &mut position) as usize;
        let original = &code[position..position + length];
        position += length;
        let body: &[u8] = if index + imported == init {
            &init_body
        } else if index + imported == tick_index {
            &tick_body
        } else if index + imported == frames_listened {
            &[0, 0x41, 0, 0x0b]
        } else {
            original
        };
        write_leb(body.len() as u32, &mut replacement);
        replacement.extend_from_slice(body);
    }
    let module = replace_section(&module, 10, &replacement);
    let mut module = replace_section(&module, 5, &[1, 0, 1]);
    wit_component::embed_component_metadata(
        &mut module,
        &resolve,
        world,
        wit_component::StringEncoding::UTF8,
    )
    .unwrap();
    wit_component::ComponentEncoder::default()
        .module(&module)
        .unwrap()
        .encode()
        .unwrap()
}

#[test]
fn subscribed_tick_runs_and_unsubscribed_key_is_skipped() {
    let calls = Arc::new(Mutex::new(Vec::new()));
    let bytes = component("tick", |open| screen_tick(open, false));
    let mut sandbox = Sandbox::new();
    let mut guest = ClientMod::instantiate(
        &mut sandbox,
        "mine",
        &bytes,
        Box::new(FakeHost { calls: calls.clone() }),
    )
    .unwrap();

    assert!(guest.subscribed("tick"));
    assert!(!guest.subscribed("key"));
    assert_eq!(guest.tick(0), Some(()));
    assert_eq!(guest.key("K", true), None);
    assert_eq!(*calls.lock().unwrap(), ["mine"]);
}

#[test]
fn slow_client_service_does_not_disable_the_guest() {
    let calls = Arc::new(Mutex::new(Vec::new()));
    let bytes = component("tick", |open| screen_tick(open, false));
    let mut guest = ClientMod::instantiate(
        &mut Sandbox::new(),
        "mine",
        &bytes,
        Box::new(SlowHost { calls: calls.clone() }),
    )
    .unwrap();

    assert_eq!(guest.tick(0), Some(()));
    assert!(!guest.is_disabled());
    assert_eq!(*calls.lock().unwrap(), ["mine"]);
}

#[test]
fn unknown_subscription_rejects_the_component() {
    let bytes = component("not_a_hook", |open| screen_tick(open, false));
    let result = ClientMod::instantiate(
        &mut Sandbox::new(),
        "mine",
        &bytes,
        Box::<FakeHost>::default(),
    );

    match result {
        Err(error) => assert!(error.to_string().contains("not_a_hook")),
        Ok(_) => panic!("an unknown subscription must fail to load"),
    }
}

#[test]
fn screen_open_rejects_another_mod_and_forwards_own_id() {
    let calls = Arc::new(Mutex::new(Vec::new()));
    let bytes = component("tick", |open| screen_tick(open, true));
    let mut guest = ClientMod::instantiate(
        &mut Sandbox::new(),
        "mine",
        &bytes,
        Box::new(FakeHost { calls: calls.clone() }),
    )
    .unwrap();

    assert_eq!(guest.tick(0), Some(()));
    assert_eq!(*calls.lock().unwrap(), ["mine"]);
}

#[test]
fn a_hook_trap_disables_the_mod() {
    let bytes = component("tick", |_| vec![0, 0x00, 0x0b]);
    let mut guest = ClientMod::instantiate(
        &mut Sandbox::new(),
        "mine",
        &bytes,
        Box::<FakeHost>::default(),
    )
    .unwrap();

    assert_eq!(guest.tick(0), None);
    assert!(guest.is_disabled());
    assert_eq!(guest.tick(0), None);
}

#[test]
fn net_frames_listened_preserves_an_existing_listener() {
    let bytes = component("net_frames_listened", |_| vec![0, 0x0b]);
    let mut guest = ClientMod::instantiate(
        &mut Sandbox::new(),
        "mine",
        &bytes,
        Box::<FakeHost>::default(),
    )
    .unwrap();

    assert_eq!(guest.net_frames_listened(true), Some(true));
    assert_eq!(guest.net_frames_listened(false), Some(false));
}
