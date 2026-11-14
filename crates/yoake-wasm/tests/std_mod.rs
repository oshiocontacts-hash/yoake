//! A normal `std` guest (`hello-std`) in the sandboxed WASI subset.

use std::sync::{Arc, Mutex};

use yoake_wasm::bindings::yoake::mods::types::HostError;
use yoake_wasm::{ClientHost, ClientMod, Sandbox};

const HELLO_STD: &str =
    "C:/Users/USER/NestgateHorizon/target-wasm/wasm32-wasip2/release/yoake_wasm_hello_std.wasm";

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
    let Ok(bytes) = std::fs::read(HELLO_STD) else {
        eprintln!("skipped: {HELLO_STD} is not built");
        return;
    };
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
