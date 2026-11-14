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

// ---------------------------------------------------------------------------------------------
// Limits: tables, time

const TICK_WIT: &str = "package yoake:mods@1.0.0; world client-mod { export tick: func(); }";

fn sleb(mut value: i64) -> Vec<u8> {
    let mut out = Vec::new();
    loop {
        let byte = (value & 0x7f) as u8;
        value >>= 7;
        if (value == 0 && byte & 0x40 == 0) || (value == -1 && byte & 0x40 != 0) {
            out.push(byte);
            return out;
        }
        out.push(byte | 0x80);
    }
}

/// Inserts (or replaces) the table section of a core module.
fn with_tables(module: Vec<u8>, tables: usize) -> Vec<u8> {
    let mut content = Vec::new();
    write_leb(tables, &mut content);
    for _ in 0..tables {
        content.extend_from_slice(&[0x70, 0x00, 0x01]);
    }
    if sections(&module).iter().any(|(id, _)| *id == 4) {
        return replace_section(&module, 4, &content);
    }
    let mut output = module[..8].to_vec();
    let mut inserted = false;
    for (id, range) in sections(&module) {
        if id > 4 && !inserted {
            inserted = true;
            output.push(4);
            write_leb(content.len(), &mut output);
            output.extend_from_slice(&content);
        }
        output.push(id);
        write_leb(range.len(), &mut output);
        output.extend_from_slice(&module[range]);
    }
    assert!(inserted);
    output
}

fn run_tick(guest: &mut yoake_wasm::ModInstance<()>) -> Option<()> {
    guest.call("tick", |store, instance| {
        let tick = instance.get_typed_func::<(), ()>(&mut *store, "tick").map_err(|error| error.to_string())?;
        tick.call(&mut *store, ()).map_err(|error| error.to_string())?;
        tick.post_return(&mut *store).map_err(|error| error.to_string())
    })
}

/// A tick that grows its table by `by` elements and traps when the growth did (`trap_on_success`)
/// or did not succeed.
fn grow_body(by: i64, trap_on_success: bool) -> Vec<u8> {
    let mut body = vec![0, 0xd0, 0x70, 0x41];
    body.extend(sleb(by));
    body.extend_from_slice(&[0xfc, 0x0f, 0x00, 0x41, 0x7f]);
    // `table.grow` answers -1 when the store's limiter refuses: ne(-1) is "granted".
    body.push(if trap_on_success { 0x47 } else { 0x46 });
    body.extend_from_slice(&[0x04, 0x40, 0x00, 0x0b, 0x0b]);
    body
}

fn table_guest(tables: usize, body: Vec<u8>) -> Vec<u8> {
    component_with(TICK_WIT, |module| replace_tick_body(with_tables(module, tables), &body))
}

#[test]
fn a_table_cannot_grow_past_the_element_limit() {
    // Within the limit the growth works (this guest traps if it was refused: `eq -1`).
    let bytes = table_guest(1, grow_body(100, false));
    let mut guest = Sandbox::new().compile(&bytes).unwrap().instantiate("table-ok", (), |_| Ok(())).unwrap();
    assert_eq!(run_tick(&mut guest), Some(()));
    // Past it the limiter refuses (this guest traps if it was granted: `ne -1`).
    let over = yoake_wasm::TABLE_ELEMENT_LIMIT as i64 + 1;
    let bytes = table_guest(1, grow_body(over, true));
    let mut guest = Sandbox::new().compile(&bytes).unwrap().instantiate("table-over", (), |_| Ok(())).unwrap();
    assert_eq!(run_tick(&mut guest), Some(()), "table.grow past the limit must fail, not succeed");
    // Control: the same guest asking for a small growth is granted, so it traps, which shows the
    // check above can fail.
    let bytes = table_guest(1, grow_body(100, true));
    let mut guest = Sandbox::new().compile(&bytes).unwrap().instantiate("table-control", (), |_| Ok(())).unwrap();
    assert_eq!(run_tick(&mut guest), None);
}

#[test]
fn too_many_tables_are_refused_at_instantiation() {
    let bytes = table_guest(yoake_wasm::TABLE_LIMIT + 1, vec![0, 0x0b]);
    assert!(Sandbox::new().compile(&bytes).unwrap().instantiate("many-tables", (), |_| Ok(())).is_err());
    let bytes = table_guest(yoake_wasm::TABLE_LIMIT, vec![0, 0x0b]);
    assert!(Sandbox::new().compile(&bytes).unwrap().instantiate("some-tables", (), |_| Ok(())).is_ok());
}

#[test]
fn an_initial_table_over_the_limit_is_refused() {
    let bytes = component_with(TICK_WIT, |module| {
        let module = with_tables(module, 1);
        let mut content = vec![1, 0x70, 0x00];
        write_leb(yoake_wasm::TABLE_ELEMENT_LIMIT + 1, &mut content);
        replace_section(&module, 4, &content)
    });
    assert!(Sandbox::new().compile(&bytes).unwrap().instantiate("big-table", (), |_| Ok(())).is_err());
}

/// A tick that counts `iterations` down (a bounded busy loop).
fn spin_body(iterations: i64) -> Vec<u8> {
    let mut body = vec![0x01, 0x01, 0x7f, 0x41];
    body.extend(sleb(iterations));
    body.extend_from_slice(&[0x21, 0x00, 0x03, 0x40, 0x20, 0x00, 0x41, 0x01, 0x6b, 0x22, 0x00, 0x0d, 0x00, 0x0b, 0x0b]);
    body
}

fn spin_guest(iterations: i64) -> Vec<u8> {
    component_with(TICK_WIT, |module| replace_tick_body(module, &spin_body(iterations)))
}

fn endless_guest() -> Vec<u8> {
    component_with(TICK_WIT, |module| replace_tick_body(module, &[0, 0x03, 0x40, 0x0c, 0, 0x0b, 0x0b]))
}

#[test]
fn an_endless_loop_is_stopped_within_the_budget_not_a_scheduler_tick_later() {
    let bytes = endless_guest();
    let mut sandbox = Sandbox::new();
    let mut best = std::time::Duration::MAX;
    for _ in 0..8 {
        let mut guest = sandbox.compile(&bytes).unwrap().instantiate("loop", (), |_| Ok(())).unwrap();
        guest.set_fuel_per_call(u64::MAX / 2);
        guest.set_limits(yoake_wasm::TimeLimits {
            call: std::time::Duration::from_millis(7),
            aggregate: std::time::Duration::from_millis(50),
        });
        let started = std::time::Instant::now();
        assert_eq!(run_tick(&mut guest), None);
        best = best.min(started.elapsed());
        assert!(guest.is_disabled());
    }
    // Never before the limit (a late or bunched epoch tick must not stop a call early), and soon
    // after it (a tick every millisecond where the OS allows; the Windows scheduler rounds sleeps
    // up to 15.6 ms, hence the margin).
    assert!(best >= std::time::Duration::from_millis(7), "{best:?}");
    assert!(best < std::time::Duration::from_millis(7 + 25), "{best:?}");
}

#[test]
fn the_default_limits_are_the_client_limits() {
    let bytes = endless_guest();
    let mut guest = Sandbox::new().compile(&bytes).unwrap().instantiate("client-loop", (), |_| Ok(())).unwrap();
    guest.set_fuel_per_call(u64::MAX / 2);
    let started = std::time::Instant::now();
    assert_eq!(run_tick(&mut guest), None);
    let elapsed = started.elapsed();
    assert!(elapsed >= yoake_wasm::CLIENT_HOOK_BUDGET, "{elapsed:?}");
    assert!(elapsed < yoake_wasm::CLIENT_HOOK_BUDGET + std::time::Duration::from_millis(30), "{elapsed:?}");
}

#[test]
fn a_server_module_gets_the_server_hook_budget() {
    let bytes = endless_guest();
    let mut guest = Sandbox::new().compile(&bytes).unwrap().instantiate("server-loop", (), |_| Ok(())).unwrap();
    guest.set_fuel_per_call(u64::MAX / 2);
    guest.set_limits(yoake_wasm::TimeLimits::SERVER);
    let started = std::time::Instant::now();
    assert_eq!(run_tick(&mut guest), None);
    let elapsed = started.elapsed();
    assert!(elapsed >= yoake_wasm::SERVER_HOOK_BUDGET, "{elapsed:?}");
    assert!(elapsed < yoake_wasm::SERVER_HOOK_BUDGET + std::time::Duration::from_millis(30), "{elapsed:?}");
}

#[test]
fn a_mod_that_uses_its_aggregate_budget_is_skipped_until_the_window_ends() {
    let mut sandbox = Sandbox::new();
    // Calibrate a busy loop of about half a millisecond.
    let mut iterations = 20_000i64;
    for _ in 0..6 {
        let mut guest = sandbox.compile(&spin_guest(iterations)).unwrap().instantiate("calibrate", (), |_| Ok(())).unwrap();
        let started = std::time::Instant::now();
        assert_eq!(run_tick(&mut guest), Some(()));
        let took = started.elapsed().as_secs_f64();
        if took > 0.0 {
            iterations = ((iterations as f64) * 0.0005 / took).clamp(1_000.0, 1_000_000.0) as i64;
        }
    }
    let mut guest = sandbox.compile(&spin_guest(iterations)).unwrap().instantiate("budget", (), |_| Ok(())).unwrap();
    guest.set_limits(yoake_wasm::TimeLimits {
        call: std::time::Duration::from_millis(50),
        aggregate: std::time::Duration::from_millis(5),
    });
    let mut ran = 0;
    for _ in 0..200 {
        if run_tick(&mut guest).is_some() {
            ran += 1;
        }
    }
    assert!((1..200).contains(&ran), "ran {ran} of 200");
    assert!(guest.skipped_calls() > 0);
    assert!(!guest.is_disabled(), "skipped calls are not a trap");
    // The next window starts afresh.
    std::thread::sleep(yoake_wasm::AGGREGATE_WINDOW + std::time::Duration::from_millis(10));
    assert_eq!(run_tick(&mut guest), Some(()));
}

#[test]
fn fuel_stops_a_busy_guest_before_the_clock_does() {
    // The default fuel (10 million instructions) bounds a call's CPU time well inside the limits.
    let bytes = endless_guest();
    let mut guest = Sandbox::new().compile(&bytes).unwrap().instantiate("fuel", (), |_| Ok(())).unwrap();
    let started = std::time::Instant::now();
    assert_eq!(run_tick(&mut guest), None);
    assert!(started.elapsed() < yoake_wasm::CLIENT_HOOK_BUDGET + std::time::Duration::from_millis(5), "{:?}", started.elapsed());
    assert!(guest.is_disabled());
}
