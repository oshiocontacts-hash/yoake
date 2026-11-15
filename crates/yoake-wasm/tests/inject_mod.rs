use std::collections::BTreeMap;
use std::sync::Mutex;

use wit_parser::{ManglingAndAbi, Resolve};
use yoake_inject_rt::{Side, catalog, enter_side};
use yoake_value::Value;
use yoake_wasm::{Sandbox, inject::Injection};

const TARGET: &str =
    "game::tick_sleep_timer#198a5917f750358ff70ce3f80ede1aa2c142d914cbe7bed985f5b6f4ed29a7b7";

static INJECTION_TEST: Mutex<()> = Mutex::new(());

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
    let mut found = Vec::new();
    while position < module.len() {
        let id = module[position];
        position += 1;
        let length = read_leb(module, &mut position) as usize;
        found.push((id, module[position..position + length].to_vec()));
        position += length;
    }
    found
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

fn function_indices(module: &[u8]) -> (u32, BTreeMap<String, u32>) {
    let mut imported = 0;
    let mut exports = BTreeMap::new();
    for (id, section) in sections(module) {
        let mut position = 0;
        if id == 2 {
            imported = read_leb(&section, &mut position);
            for _ in 0..imported {
                read_name(&section, &mut position);
                read_name(&section, &mut position);
                assert_eq!(section[position], 0);
                position += 1;
                read_leb(&section, &mut position);
            }
        } else if id == 7 {
            let count = read_leb(&section, &mut position);
            for _ in 0..count {
                let name = read_name(&section, &mut position).to_owned();
                let kind = section[position];
                position += 1;
                let index = read_leb(&section, &mut position);
                if kind == 0 {
                    exports.insert(name, index);
                }
            }
        }
    }
    (imported, exports)
}

fn i32_const(body: &mut Vec<u8>, mut value: i32) {
    body.push(0x41);
    loop {
        let mut byte = (value & 0x7f) as u8;
        value >>= 7;
        let finished = (value == 0 && byte & 0x40 == 0) || (value == -1 && byte & 0x40 != 0);
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

fn finish(mut body: Vec<u8>) -> Vec<u8> {
    i32_const(&mut body, 0);
    body.push(0x0b);
    body
}

fn registrations_body(registrations: &[(&str, u8)]) -> Vec<u8> {
    let mut body = vec![0];
    store_i32(&mut body, 0, 128);
    store_i32(&mut body, 4, registrations.len() as i32);
    let mut address = 1024;
    for (index, (target, points)) in registrations.iter().enumerate() {
        let entry = 128 + index as i32 * 12;
        store_i32(&mut body, entry, address);
        store_i32(&mut body, entry + 4, target.len() as i32);
        store_i32(&mut body, entry + 8, i32::from(*points));
        for byte in target.bytes() {
            i32_const(&mut body, address);
            i32_const(&mut body, i32::from(byte));
            body.extend_from_slice(&[0x3a, 0, 0]);
            address += 1;
        }
    }
    finish(body)
}

fn realloc_body() -> Vec<u8> {
    let mut body = vec![1, 1, 0x7f];
    i32_const(&mut body, 4096);
    body.extend_from_slice(&[0x28, 2, 0]);
    i32_const(&mut body, 8192);
    body.extend_from_slice(&[0x6a, 0x21, 4]);
    i32_const(&mut body, 4096);
    i32_const(&mut body, 4096);
    body.extend_from_slice(&[0x28, 2, 0]);
    i32_const(&mut body, 512);
    body.extend_from_slice(&[0x6a, 0x36, 2, 0, 0x20, 4, 0x0b]);
    body
}

fn unsigned(body: &mut Vec<u8>, address: i32, value: i32) {
    store_i32(body, address, 3);
    store_i32(body, address + 8, value);
    store_i32(body, address + 12, 0);
}

fn replace_arguments(value: i32) -> Vec<u8> {
    let mut body = vec![0];
    store_i32(&mut body, 0, 1);
    store_i32(&mut body, 8, 64);
    store_i32(&mut body, 12, 2);
    unsigned(&mut body, 64, value);
    store_i32(&mut body, 88, 1);
    store_i32(&mut body, 96, 0);
    finish(body)
}

fn return_value(value: i32) -> Vec<u8> {
    let mut body = vec![0];
    store_i32(&mut body, 0, 1);
    unsigned(&mut body, 8, value);
    finish(body)
}

fn overwrite_value(value: i32) -> Vec<u8> {
    let mut body = vec![0];
    unsigned(&mut body, 0, value);
    finish(body)
}

fn component(
    registrations: &[(&str, u8)],
    head: Vec<u8>,
    returned: Vec<u8>,
    overwrite: Vec<u8>,
) -> Vec<u8> {
    let mut resolve = Resolve::default();
    let package = resolve
        .push_str(
            "yoake-inject.wit",
            include_str!("../wit-inject/yoake-inject.wit"),
        )
        .unwrap();
    let world = resolve
        .select_world(&[package], Some("inject-mod"))
        .unwrap();
    let module = wit_component::dummy_module(&resolve, world, ManglingAndAbi::Standard32);
    let (imported, exports) = function_indices(&module);
    let bodies = [
        ("|registrations", registrations_body(registrations)),
        ("|inject-head", head),
        ("|inject-return", returned),
        ("|inject-overwrite", overwrite),
        ("realloc", realloc_body()),
    ]
    .into_iter()
    .map(|(suffix, body)| {
        let index = exports
            .iter()
            .find(|(name, _)| name.ends_with(suffix))
            .unwrap()
            .1;
        (*index, body)
    })
    .collect::<BTreeMap<_, _>>();
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
        let body = bodies
            .get(&(index + imported))
            .map(Vec::as_slice)
            .unwrap_or(original);
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
fn prefix_mod_ids_run_in_bare_id_order() {
    let _lock = INJECTION_TEST
        .lock()
        .unwrap_or_else(|error| error.into_inner());
    let mut sandbox = Sandbox::new();
    let trap = vec![0, 0x00, 0x0b];
    let extras = component(
        &[(TARGET, 1)],
        replace_arguments(20),
        trap.clone(),
        trap.clone(),
    );
    let base = component(&[(TARGET, 1)], replace_arguments(10), trap.clone(), trap);
    let _extras = Injection::instantiate(
        &mut sandbox,
        "foo-extras",
        &extras,
        &[TARGET.to_owned()],
        Side::Server,
    )
    .unwrap();
    let _base = Injection::instantiate(
        &mut sandbox,
        "foo",
        &base,
        &[TARGET.to_owned()],
        Side::Server,
    )
    .unwrap();
    let _side = enter_side(Side::Server);
    let site = catalog::SITES
        .iter()
        .find(|site| site.id() == TARGET)
        .unwrap();
    assert_eq!(
        site.invoke(
            vec![Value::Unsigned(2), Value::Boolean(false)],
            |arguments| arguments[0].clone()
        ),
        Value::Unsigned(20),
    );
}

#[test]
fn modules_with_only_opposite_side_targets_are_noops() {
    let _lock = INJECTION_TEST
        .lock()
        .unwrap_or_else(|error| error.into_inner());
    let mut sandbox = Sandbox::new();
    for (target, side) in [
        (TARGET, Side::Client),
        (
            "game::attack_strength_scale#e28527bc3c45193c7ad0406144863440989c9d229dac40a38e43a7c54768e6d3",
            Side::Server,
        ),
    ] {
        let trap = vec![0, 0x00, 0x0b];
        let bytes = component(&[(target, 1)], trap.clone(), trap.clone(), trap);
        let result =
            Injection::instantiate(&mut sandbox, "opposite", &bytes, &[target.to_owned()], side);
        assert!(result.is_ok(), "opposite-side targets rejected on {side:?}");
    }
}

#[test]
fn oversized_engine_snapshots_skip_without_disabling_handlers() {
    let _lock = INJECTION_TEST
        .lock()
        .unwrap_or_else(|error| error.into_inner());
    let mut sandbox = Sandbox::new();
    let target = "world::food::add_exhaustion#ca3064943a7660df0f8bb96751c0c58f9df0074453c066cff57d287963b4ef4e";
    let site = catalog::SITES
        .iter()
        .find(|site| site.id() == target)
        .unwrap();
    let _side = enter_side(Side::Server);
    for point in [1, 4] {
        let trap = vec![0, 0x00, 0x0b];
        let mut cancel = vec![0];
        store_i32(&mut cancel, 0, 2);
        store_i32(&mut cancel, 8, 0);
        let unit = finish(vec![0, 0x41, 0, 0x41, 0, 0x36, 2, 0]);
        let bytes = component(&[(target, point)], finish(cancel), trap, unit);
        let injection = Injection::instantiate(
            &mut sandbox,
            "oversized-engine",
            &bytes,
            &[target.to_owned()],
            Side::Server,
        )
        .unwrap();
        let mut scope = yoake_value::Scope::default();
        let receiver = scope.handle("Self", Some(Value::Text("x".repeat(256 * 1024 + 1))));
        assert_eq!(
            site.invoke(vec![receiver, Value::Float(1.0)], |_| Value::Unit),
            Value::Unit
        );
        assert_eq!(
            site.mask(),
            point,
            "host encoding failure disabled an innocent handler"
        );
        let receiver = scope.handle("Self", None);
        assert_eq!(
            site.invoke(vec![receiver, Value::Float(1.0)], |_| panic!(
                "host encoding failure disabled an innocent handler"
            )),
            Value::Unit
        );
        drop(injection);
    }
}

#[test]
fn component_injections_validate_dispatch_and_recover() {
    let _lock = INJECTION_TEST
        .lock()
        .unwrap_or_else(|error| error.into_inner());
    let mut sandbox = Sandbox::new();
    let site = catalog::SITES
        .iter()
        .find(|site| site.path() == "game::tick_sleep_timer")
        .unwrap();
    assert_eq!(site.id(), TARGET);
    let _side = enter_side(Side::Server);
    let targets = vec![TARGET.to_owned()];
    let trap = vec![0, 0x00, 0x0b];
    let proceed = finish(vec![0, 0x41, 0, 0x41, 0, 0x36, 2, 0]);
    let bytes = component(
        &[(TARGET, 3)],
        replace_arguments(10),
        trap.clone(),
        trap.clone(),
    );
    {
        let _loaded =
            Injection::instantiate(&mut sandbox, "head-return", &bytes, &targets, Side::Server)
                .unwrap();
        let mut native_calls = 0;
        for _ in 0..2 {
            let result = site.invoke(
                vec![Value::Unsigned(2), Value::Boolean(false)],
                |arguments| {
                    native_calls += 1;
                    let Value::Unsigned(value) = arguments[0] else {
                        panic!("unsigned argument required")
                    };
                    Value::Unsigned(value + 1)
                },
            );
            assert_eq!(result, Value::Unsigned(11));
        }
        assert_eq!(native_calls, 2);
    }
    assert_eq!(site.mask(), 0);
    let bytes = component(
        &[(TARGET, 6)],
        proceed.clone(),
        return_value(9),
        overwrite_value(7),
    );
    {
        let _loaded =
            Injection::instantiate(&mut sandbox, "overwrite", &bytes, &targets, Side::Server)
                .unwrap();
        assert_eq!(
            site.invoke(vec![Value::Unsigned(2), Value::Boolean(false)], |_| panic!(
                "overwrite must skip native"
            )),
            Value::Unsigned(9)
        );
        let conflict =
            Injection::instantiate(&mut sandbox, "conflict", &bytes, &targets, Side::Server)
                .err()
                .unwrap();
        assert!(
            conflict.to_string().contains("ConflictingOverwrite"),
            "{conflict}"
        );
    }
    let bytes = component(&[(TARGET, 4)], proceed.clone(), proceed.clone(), trap);
    {
        let _loaded = Injection::instantiate(
            &mut sandbox,
            "trapping-overwrite",
            &bytes,
            &targets,
            Side::Server,
        )
        .unwrap();
        for _ in 0..2 {
            assert_eq!(
                site.invoke(vec![Value::Unsigned(2), Value::Boolean(false)], |_| {
                    Value::Unsigned(3)
                }),
                Value::Unsigned(3)
            );
        }
    }
    assert_eq!(site.mask(), 0);
    for (registrations, expected) in [
        (vec![], "cover every manifest"),
        (vec![(TARGET, 0)], "unsupported mixin injection points"),
        (
            vec![(TARGET, 1), (TARGET, 2)],
            "duplicate mixin registration",
        ),
        (vec![("unknown#signature", 1)], "undeclared mixin target"),
    ] {
        let bytes = component(
            &registrations,
            proceed.clone(),
            proceed.clone(),
            proceed.clone(),
        );
        let error = Injection::instantiate(&mut sandbox, "invalid", &bytes, &targets, Side::Server)
            .err()
            .unwrap();
        assert!(error.to_string().contains(expected), "{error}");
        assert_eq!(site.mask(), 0);
    }
    for (targets, expected) in [
        (vec![], "between 1 and 1024"),
        (
            vec![TARGET.to_owned(), TARGET.to_owned()],
            "duplicate manifest",
        ),
        (vec!["game::tick_sleep_timer#bad".to_owned()], "signature"),
        (vec!["unknown#bad".to_owned()], "unknown"),
    ] {
        let error = Injection::instantiate(&mut sandbox, "invalid", &[], &targets, Side::Server)
            .err()
            .unwrap();
        assert!(error.to_string().contains(expected), "{error}");
    }
    let inactive =
        Injection::instantiate(&mut sandbox, "wrong-side", &[], &targets, Side::Client).unwrap();
    drop(inactive);
    let client_site = catalog::SITES
        .iter()
        .find(|site| site.path() == "game::attack_strength_scale")
        .unwrap();
    drop(
        Injection::instantiate(
            &mut sandbox,
            "server-with-client-targets",
            &[],
            &[client_site.id().to_owned()],
            Side::Server,
        )
        .unwrap(),
    );
    let chat_site = catalog::SITES
        .iter()
        .find(|site| site.path() == "game::chat::handle_chat_input")
        .unwrap();
    for point in [1, 2, 4] {
        let trap = vec![0, 0x00, 0x0b];
        let bytes = component(
            &[(&chat_site.id(), point)],
            trap.clone(),
            trap.clone(),
            trap,
        );
        let _loaded = Injection::instantiate_with_permissions(
            &mut sandbox,
            "host-value-over-budget",
            &bytes,
            &[chat_site.id().to_owned()],
            Side::Server,
            &["service.commands".to_owned()],
        )
        .unwrap();
        let mut scope = yoake_value::Scope::default();
        let receiver = scope.handle("Self", None);
        let oversized = Value::Text("x".repeat(256 * 1024 + 1));
        let arguments = vec![
            receiver.clone(),
            if point == 2 {
                Value::Text("id".to_owned())
            } else {
                oversized.clone()
            },
        ];
        let native_result = if point == 2 { oversized } else { Value::Unit };
        assert_eq!(
            chat_site.invoke(arguments, |_| native_result.clone()),
            native_result,
        );
        assert_eq!(chat_site.mask(), point);
        assert_eq!(
            chat_site.invoke(vec![receiver, Value::Text("id".to_owned())], |_| {
                Value::Unit
            }),
            Value::Unit,
        );
        assert_eq!(chat_site.mask(), 0);
    }
    let mut resolve = Resolve::default();
    let package = resolve
        .push_str("empty.wit", "package test:empty; world empty {}")
        .unwrap();
    let world = resolve.select_world(&[package], Some("empty")).unwrap();
    let mut module = wit_component::dummy_module(&resolve, world, ManglingAndAbi::Standard32);
    wit_component::embed_component_metadata(
        &mut module,
        &resolve,
        world,
        wit_component::StringEncoding::UTF8,
    )
    .unwrap();
    let bytes = wit_component::ComponentEncoder::default()
        .module(&module)
        .unwrap()
        .encode()
        .unwrap();
    let error = Injection::instantiate(
        &mut sandbox,
        "missing-exports",
        &bytes,
        &targets,
        Side::Server,
    )
    .err()
    .unwrap();
    assert!(
        error
            .to_string()
            .contains("must implement yoake:mods/inject@1.1.0"),
        "{error}"
    );
}
