//! A full-access mod written with Rust `std` for `wasm32-wasip1` (FR-F2). It does not use the SDK (the SDK is
//! `no_std` with its own allocator); it declares the few engine imports itself. Hook `SERVER_TICK` uses files, log
//! lines, clocks and random; hook `CLIENT_TICK` sleeps (a wait that must trap).

#[link(wasm_import_module = "env")]
unsafe extern "C" {
    fn hook_register(id: i32, priority: i32) -> i32;
    fn report(slot: i32, value: i64);
}

const SERVER_TICK: u32 = 0x8000_0001;
const CLIENT_TICK: u32 = 0x8000_0002;

#[unsafe(no_mangle)]
pub extern "C" fn yoake_init() {
    unsafe {
        hook_register(SERVER_TICK as i32, 0);
        hook_register(CLIENT_TICK as i32, 0);
    }
}

#[unsafe(no_mangle)]
pub extern "C" fn yoake_hook(id: i32, _handle: i64) -> i64 {
    let r = |slot: i32, value: i64| unsafe { report(slot, value) };
    if id as u32 == CLIENT_TICK {
        std::thread::sleep(std::time::Duration::from_millis(50));
        r(100, 1); // not reached
        return 0;
    }
    r(1, i64::from(std::fs::write("/data/std.txt", "from std").is_ok()));
    r(2, std::fs::read_to_string("/data/std.txt").map_or(-1, |text| text.len() as i64));
    r(3, i64::from(std::fs::write("/game/std-probe.txt", "g").is_ok()));
    println!("std guest says hi");
    eprintln!("std guest stderr line");
    let wall = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map_or(0, |d| d.as_secs());
    r(4, i64::from(wall > 1_700_000_000));
    let began = std::time::Instant::now();
    r(5, i64::from(began.elapsed() < std::time::Duration::from_secs(5)));
    // a short wait is allowed
    std::thread::sleep(std::time::Duration::from_micros(500));
    use std::hash::{BuildHasher, Hasher};
    let mut hasher = std::collections::hash_map::RandomState::new().build_hasher();
    hasher.write_u8(1);
    r(6, i64::from(hasher.finish() != 0));
    0
}
