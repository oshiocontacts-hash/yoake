//! A normal `std` guest (`hello-std`) in the sandboxed WASI subset.

use std::sync::{Arc, Mutex};

use yoake_wasm::bindings::yoake::mods::types::HostError;
use yoake_wasm::{ClientHost, ClientMod, Sandbox};

#[derive(Default, Clone)]
struct Calls(Arc<Mutex<Vec<String>>>);

struct FakeHost(Calls);

impl ClientHost for FakeHost {
    fn log(&mut self, target: &str, message: &str) -> Result<(), HostError> {
        self.0.0.lock().unwrap().push(format!("{target}:{message}"));
        Ok(())
    }
}

#[test]
fn std_guest_prints_to_the_mod_log_and_has_no_filesystem() {
    let Some(bytes) = yoake_wasm::test_guests::load(&yoake_wasm::test_guests::HELLO_STD) else { return };
    let calls = Calls::default();
    let mut guest = ClientMod::instantiate(&mut Sandbox::new(), "hello-std", &bytes, Box::new(FakeHost(calls.clone())))
        .expect("std component loads");
    assert!(guest.subscribed("hud"));
    assert_eq!(guest.tick(0), Some(()));
    assert_eq!(guest.hud(), Some(()));
    {
        let calls = calls.0.lock().unwrap();
        assert!(calls.contains(&"stdout:hello-std: println works, args=0, env=0".to_string()), "{calls:?}");
        assert!(calls.contains(&"stderr:hello-std: stderr works".to_string()), "{calls:?}");
        assert!(calls.contains(&"stdout:hello-std: fs denied=true".to_string()), "{calls:?}");
        assert!(calls.contains(&"stdout:hello-std: map ok=true elapsed_ok=true".to_string()), "{calls:?}");
        assert!(calls.contains(&"wasm-hello-std:hello from a std WebAssembly mod".to_string()), "{calls:?}");
    }
    // process::exit is a trap: only this mod is disabled.
    assert_eq!(guest.key("EXIT", true), None);
    assert!(guest.is_disabled());
}

#[test]
fn a_guest_cannot_block_the_host_thread_by_sleeping() {
    let Some(bytes) = yoake_wasm::test_guests::load(&yoake_wasm::test_guests::HELLO_STD) else { return };
    // A nap within the call budget is fine: the clock pollable is already ready.
    let mut guest = ClientMod::instantiate(&mut Sandbox::new(), "nap", &bytes, Box::new(FakeHost(Calls::default()))).unwrap();
    assert_eq!(guest.key("NAP", true), Some(()));
    assert!(!guest.is_disabled());
    // `thread::sleep(10s)` traps at once instead of sleeping the host thread.
    let mut guest = ClientMod::instantiate(&mut Sandbox::new(), "sleeper", &bytes, Box::new(FakeHost(Calls::default()))).unwrap();
    let started = std::time::Instant::now();
    assert_eq!(guest.key("SLEEP", true), None);
    assert!(started.elapsed() < std::time::Duration::from_millis(500), "{:?}", started.elapsed());
    assert!(guest.is_disabled());
}
