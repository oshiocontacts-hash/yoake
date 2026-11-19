use yoake_wasm_sdk::full::client::bindings::yoake::mods::types::{TextureFilter, TextureFormat};
use yoake_wasm_sdk::full::client::{ClientMod, et, hud, services, types};
use yoake_wasm_sdk::full::hook;

#[derive(Default)]
struct Demo;

impl ClientMod for Demo {
    fn subscriptions() -> &'static [&'static str] {
        &[hook::HUD]
    }

    fn hud(&mut self) -> et::HudOutcome {
        services::register_key_binding("demo", "key.keyboard.g", "demo:action", "demo action")
            .unwrap();
        assert!(services::key_binding_state("demo:action").unwrap().down);
        services::draw_world_line(
            0xff12_3456,
            types::Vec3 {
                x: 4.0,
                y: 5.0,
                z: 6.0,
            },
            types::Vec3 {
                x: 1.0,
                y: 2.0,
                z: 3.0,
            },
            2.0,
        )
        .unwrap();
        services::register_post_shader("demo:tint", 16, "valid shader").unwrap();
        assert!(services::register_post_shader("other:invalid", 16, "invalid shader").is_err());
        let texture =
            hud::GuiTexture::create(2, 2, TextureFormat::Rgba8, TextureFilter::Nearest).unwrap();
        services::gui_texture_write(&[255; 16], (0, 0), (2, 2), &texture).unwrap();
        assert!(services::gui_texture_write(&[255; 16], (1, 1), (2, 2), &texture).is_err());
        services::ime_set_allowed(true).unwrap();
        yoake_wasm_sdk::full::client::ime_set_cursor_area(10.0, 20.0, 30.0, 40.0).unwrap();
        println!("client services: real imports passed");
        et::HudOutcome {
            corners_commands: Vec::new(),
            batch_commands: Vec::new(),
        }
    }
}

yoake_wasm_sdk::export_client_full!(Demo);
