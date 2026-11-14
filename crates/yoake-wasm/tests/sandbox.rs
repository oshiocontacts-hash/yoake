use std::cell::Cell;
use wit_parser::{ManglingAndAbi, Resolve};
use yoake_wasm::Sandbox;

fn component(wit: &str) -> Vec<u8> {
    component_with(wit, |module| module)
}

fn component_with(wit: &str, edit: impl FnOnce(Vec<u8>) -> Vec<u8>) -> Vec<u8> {
    let mut resolve = Resolve::default();
    let package = resolve.push_str("test.wit", wit).unwrap();
    let world = resolve.select_world(&[package], Some("client-mod")).unwrap();
    let mut module = edit(wit_component::dummy_module(
        &resolve,
        world,
        ManglingAndAbi::Standard32,
    ));
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

fn read_leb(bytes: &[u8], position: &mut usize) -> usize {
    let mut value = 0;
    let mut shift = 0;
    loop {
        let byte = bytes[*position];
        *position += 1;
        value |= usize::from(byte & 0x7f) << shift;
        if byte & 0x80 == 0 {
            return value;
        }
        shift += 7;
    }
}

fn write_leb(mut value: usize, output: &mut Vec<u8>) {
    loop {
        let mut byte = (value & 0x7f) as u8;
        value >>= 7;
        if value != 0 {
            byte |= 0x80;
        }
        output.push(byte);
        if value == 0 {
            break;
        }
    }
}

fn sections(module: &[u8]) -> Vec<(u8, std::ops::Range<usize>)> {
    assert_eq!(&module[..8], b"\0asm\x01\0\0\0");
    let mut position = 8;
    let mut result = Vec::new();
    while position < module.len() {
        let id = module[position];
        position += 1;
        let length = read_leb(module, &mut position);
        result.push((id, position..position + length));
        position += length;
    }
    result
}

fn replace_section(module: &[u8], section_id: u8, replacement: &[u8]) -> Vec<u8> {
    let mut output = module[..8].to_vec();
    let mut replaced = false;
    for (id, range) in sections(module) {
        let data = if id == section_id {
            assert!(!replaced);
            replaced = true;
            replacement
        } else {
            &module[range]
        };
        output.push(id);
        write_leb(data.len(), &mut output);
        output.extend_from_slice(data);
    }
    assert!(replaced, "the dummy module lacks the expected section");
    output
}

fn replace_tick_body(module: Vec<u8>, body: &[u8]) -> Vec<u8> {
    let sections = sections(&module);
    if let Some((_, range)) = sections.iter().find(|(id, _)| *id == 2) {
        let mut position = range.start;
        assert_eq!(read_leb(&module, &mut position), 0);
    }
    let (_, exports) = sections.iter().find(|(id, _)| *id == 7).unwrap();
    let mut position = exports.start;
    let count = read_leb(&module, &mut position);
    let mut function_index = None;
    for _ in 0..count {
        let length = read_leb(&module, &mut position);
        let name = std::str::from_utf8(&module[position..position + length]).unwrap();
        position += length;
        let kind = module[position];
        position += 1;
        let index = read_leb(&module, &mut position);
        if kind == 0 && name.ends_with("tick") {
            assert!(function_index.replace(index).is_none());
        }
    }
    let function_index = function_index.expect("missing tick export");
    let (_, code) = sections.iter().find(|(id, _)| *id == 10).unwrap();
    let mut position = code.start;
    let count = read_leb(&module, &mut position);
    assert!(function_index < count);
    let mut output = Vec::new();
    write_leb(count, &mut output);
    for index in 0..count {
        let length = read_leb(&module, &mut position);
        let original = &module[position..position + length];
        position += length;
        let selected = if index == function_index { body } else { original };
        write_leb(selected.len(), &mut output);
        output.extend_from_slice(selected);
    }
    replace_section(&module, 10, &output)
}

#[test]
fn empty_path_does_not_create_an_engine() {
    let sandbox = Sandbox::new();
    assert!(!sandbox.engine_started());
}

#[test]
fn import_outside_yoake_is_rejected() {
    let bytes = component(
        "package forbidden:system@1.0.0; interface host { use-it: func(); } world client-mod { import host; }",
    );
    let error = match Sandbox::new().compile(&bytes) {
        Ok(_) => panic!("forbidden import was accepted"),
        Err(error) => error,
    };
    assert!(error.contains("forbidden component import"), "{error}");
}

#[test]
fn wrong_imported_interface_major_is_rejected() {
    let bytes = component(
        "package yoake:mods@2.0.0; interface host { use-it: func(); } world client-mod { import host; }",
    );
    let error = match Sandbox::new().compile(&bytes) {
        Ok(_) => panic!("incompatible interface was accepted"),
        Err(error) => error,
    };
    assert!(
        error.contains("unsupported component interface version"),
        "{error}"
    );
}

#[test]
fn oversized_initial_memory_is_rejected() {
    let bytes = component_with(
        "package yoake:mods@1.0.0; world client-mod { export tick: func(value: list<u8>); }",
        |module| replace_section(&module, 5, &[1, 0, 0x81, 0x08]),
    );
    let compiled = Sandbox::new().compile(&bytes).unwrap();
    assert!(compiled.instantiate("oversized", (), |_| Ok(())).is_err());
}

#[test]
fn infinite_loop_is_stopped_and_disabled() {
    let bytes = component_with(
        "package yoake:mods@1.0.0; world client-mod { export tick: func(); }",
        |module| replace_tick_body(module, &[0, 0x03, 0x40, 0x0c, 0, 0x0b, 0x0b]),
    );
    let compiled = Sandbox::new().compile(&bytes).unwrap();
    let mut guest = compiled.instantiate("loop", (), |_| Ok(())).unwrap();
    assert!(guest.call("tick", |store, instance| {
        let tick = instance
            .get_typed_func::<(), ()>(&mut *store, "tick")
            .map_err(|error| error.to_string())?;
        tick.call(&mut *store, ()).map_err(|error| error.to_string())
    }).is_none());
    assert!(guest.is_disabled());
}

#[test]
fn successful_guest_call_keeps_the_instance_enabled() {
    let bytes = component_with(
        "package yoake:mods@1.0.0; world client-mod { export tick: func(); }",
        |module| replace_tick_body(module, &[0, 0x0b]),
    );
    let compiled = Sandbox::new().compile(&bytes).unwrap();
    let mut guest = compiled.instantiate("success", (), |_| Ok(())).unwrap();
    assert!(guest.call("tick", |store, instance| {
        let tick = instance
            .get_typed_func::<(), ()>(&mut *store, "tick")
            .map_err(|error| error.to_string())?;
        tick.call(&mut *store, ()).map_err(|error| error.to_string())?;
        tick.post_return(&mut *store).map_err(|error| error.to_string())
    }).is_some());
    assert!(!guest.is_disabled());
}

#[test]
fn guest_trap_disables_only_its_instance() {
    let bytes = component("package yoake:mods@1.0.0; world client-mod { export tick: func(); }");
    let mut sandbox = Sandbox::new();
    let compiled = sandbox.compile(&bytes).unwrap();
    assert!(sandbox.engine_started());
    let mut guest = compiled.instantiate("test", (), |_| Ok(())).unwrap();
    let other = sandbox
        .compile(&bytes)
        .unwrap()
        .instantiate("other", (), |_| Ok(()))
        .unwrap();
    let result = guest.call("tick", |store, instance| {
        let tick = instance
            .get_typed_func::<(), ()>(&mut *store, "tick")
            .map_err(|error| error.to_string())?;
        tick.call(&mut *store, ())
            .map_err(|error| error.to_string())
    });
    assert!(result.is_none());
    assert!(guest.is_disabled());
    assert!(!other.is_disabled());
    let called = Cell::new(false);
    assert!(
        guest
            .call("tick", |_, _| {
                called.set(true);
                Ok(())
            })
            .is_none()
    );
    assert!(!called.get());
}
