use std::sync::{Arc, Mutex};

use yoake_wasm::bindings::yoake::mods::types::{HostError, Point};
use yoake_wasm::{ClientHost, ClientMod, Sandbox};

#[derive(Default, Clone)]
struct Calls(Arc<Mutex<Vec<String>>>);

struct FakeHost(Calls);

impl ClientHost for FakeHost {
    fn log(&mut self, target: &str, message: &str) -> Result<(), HostError> {
        self.0.0.lock().unwrap().push(format!("log:{target}:{message}"));
        Ok(())
    }

    fn text(&mut self, text: &str, _at: Point, _rgb: u32, _shadow: bool) -> Result<(), HostError> {
        self.0.0.lock().unwrap().push(format!("text:{text}"));
        Ok(())
    }
}

#[test]
fn built_hello_example_loads_and_draws() {
    let Some(bytes) = yoake_wasm::test_guests::load(&yoake_wasm::test_guests::HELLO) else { return };
    let calls = Calls::default();
    let mut guest =
        ClientMod::instantiate(&mut Sandbox::new(), "hello", &bytes, Box::new(FakeHost(calls.clone())))
            .expect("hello component loads");

    assert!(guest.subscribed("tick"));
    assert!(guest.subscribed("hud"));
    assert!(!guest.subscribed("key"));
    assert_eq!(guest.tick(0), Some(()));
    assert_eq!(guest.hud(), Some(()));
    assert_eq!(guest.key("K", true), None);

    let calls = calls.0.lock().unwrap();
    assert!(calls.iter().any(|c| c.starts_with("log:wasm-hello")), "{calls:?}");
    assert!(calls.contains(&"text:wasm-hello #1".to_string()), "{calls:?}");
}
