use std::sync::{Arc, Mutex};

use yoake_wasm::bindings::yoake::mods::types::{HostError, TextureFilter, TextureFormat};
use yoake_wasm::client_types::{KeyState, Vec3};
use yoake_wasm::{ClientFullMod, ClientHost, ClientServices, Sandbox};

#[derive(Default)]
struct Observed {
    binding: Option<String>,
    line: Option<(u32, Vec3, Vec3, f32)>,
    shader: Option<(String, u32, String)>,
    invalid_shaders: usize,
    writes: usize,
    invalid_writes: usize,
    dropped: Vec<u64>,
    released: usize,
    output: Vec<String>,
}

struct Host(Arc<Mutex<Observed>>);

impl ClientHost for Host {
    fn client_services(&mut self) -> Option<&mut dyn ClientServices> {
        Some(self)
    }

    fn log(&mut self, _target: &str, message: &str) -> Result<(), HostError> {
        self.0.lock().unwrap().output.push(message.to_owned());
        Ok(())
    }

    fn gui_texture_create(
        &mut self,
        width: u32,
        height: u32,
        format: TextureFormat,
        filter: TextureFilter,
    ) -> Result<u64, HostError> {
        assert_eq!((width, height), (2, 2));
        assert!(matches!(format, TextureFormat::Rgba8));
        assert!(matches!(filter, TextureFilter::Nearest));
        Ok(73)
    }

    fn gui_texture_drop(&mut self, texture: u64) {
        self.0.lock().unwrap().dropped.push(texture);
    }

    fn release_resources(&mut self) {
        self.0.lock().unwrap().released += 1;
    }
}

impl ClientServices for Host {
    fn register_key_binding(
        &mut self,
        category: String,
        default_key: String,
        id: String,
        label: String,
    ) -> Result<(), String> {
        assert_eq!(
            (category.as_str(), default_key.as_str(), label.as_str()),
            ("demo", "key.keyboard.g", "demo action")
        );
        self.0.lock().unwrap().binding = Some(id);
        Ok(())
    }

    fn key_binding_state(&mut self, id: String) -> Result<KeyState, String> {
        assert_eq!(self.0.lock().unwrap().binding.as_deref(), Some(id.as_str()));
        Ok(KeyState { down: true })
    }

    fn draw_world_line(
        &mut self,
        color: u32,
        end: Vec3,
        start: Vec3,
        width: f32,
    ) -> Result<(), String> {
        self.0.lock().unwrap().line = Some((color, end, start, width));
        Ok(())
    }

    fn register_post_shader(
        &mut self,
        id: String,
        uniform_size: u32,
        wgsl: String,
    ) -> Result<(), String> {
        let mut observed = self.0.lock().unwrap();
        if !id.starts_with("demo:") {
            observed.invalid_shaders += 1;
            return Err("shader namespace does not belong to this mod".to_owned());
        }
        observed.shader = Some((id, uniform_size, wgsl));
        Ok(())
    }

    fn gui_texture_write(
        &mut self,
        bytes: Vec<u8>,
        origin: (u32, u32),
        size: (u32, u32),
        texture: u64,
    ) -> Result<(), String> {
        assert_eq!(texture, 73);
        assert_eq!(bytes, [255; 16]);
        let mut observed = self.0.lock().unwrap();
        if origin.0.checked_add(size.0).is_none_or(|end| end > 2)
            || origin.1.checked_add(size.1).is_none_or(|end| end > 2)
        {
            observed.invalid_writes += 1;
            return Err("texture write is out of bounds".to_owned());
        }
        observed.writes += 1;
        Ok(())
    }
}

#[test]
fn guest_calls_client_imports_and_receives_validation_errors() {
    let Some(bytes) = yoake_wasm::test_guests::load(&yoake_wasm::test_guests::CLIENT_SERVICES_DEMO)
    else {
        return;
    };
    let observed = Arc::new(Mutex::new(Observed::default()));
    let mut guest = ClientFullMod::instantiate(
        &mut Sandbox::new(),
        "demo",
        &bytes,
        Box::new(Host(observed.clone())),
        &[],
    )
    .unwrap();
    let outcome = guest.dispatch("hud", |exports, store| {
        exports.yoake_mods_client_hooks().call_hud(store)
    });
    assert!(
        outcome.is_some(),
        "real imports must return without trapping the guest"
    );
    assert!(!guest.is_disabled());
    drop(guest);
    let observed = observed.lock().unwrap();
    assert_eq!(observed.binding.as_deref(), Some("demo:action"));
    let (color, end, start, width) = observed.line.as_ref().unwrap();
    assert_eq!(
        (
            *color, end.x, end.y, end.z, start.x, start.y, start.z, *width
        ),
        (0xff12_3456, 4.0, 5.0, 6.0, 1.0, 2.0, 3.0, 2.0)
    );
    assert_eq!(
        observed.shader.as_ref().unwrap(),
        &("demo:tint".into(), 16, "valid shader".into())
    );
    assert_eq!(
        (
            observed.invalid_shaders,
            observed.writes,
            observed.invalid_writes
        ),
        (1, 1, 1)
    );
    assert_eq!(observed.dropped, [73]);
    assert_eq!(observed.released, 1);
    assert!(
        observed
            .output
            .iter()
            .any(|line| line.contains("client services: real imports passed"))
    );
}

#[test]
fn trapping_guest_releases_client_resources_before_unload() {
    let Some(bytes) = yoake_wasm::test_guests::load(&yoake_wasm::test_guests::CLIENT_SERVICES_DEMO)
    else {
        return;
    };
    let observed = Arc::new(Mutex::new(Observed::default()));
    let mut guest = ClientFullMod::instantiate(
        &mut Sandbox::new(),
        "demo",
        &bytes,
        Box::new(Host(observed.clone())),
        &[],
    )
    .unwrap();
    assert!(
        guest
            .dispatch("hud", |exports, store| {
                exports.yoake_mods_client_hooks().call_hud(store)
            })
            .is_some()
    );
    assert!(
        guest
            .dispatch("hud", |_, _| Err::<(), _>(wasmtime::Error::msg(
                "deliberate trap"
            )))
            .is_none()
    );
    assert!(guest.is_disabled());
    {
        let observed = observed.lock().unwrap();
        assert_eq!(observed.dropped, [73]);
        assert_eq!(observed.released, 1);
    }
    drop(guest);
    let observed = observed.lock().unwrap();
    assert_eq!(observed.dropped, [73]);
    assert_eq!(observed.released, 1);
}
