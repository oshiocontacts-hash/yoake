//! A server mod that adds a level with a generator of its own (F3-8), the sample of the level services:
//!
//! * `init` registers a dimension type (`levelgen:terrace_type`: 64 blocks high from y 0, a blue sky, a
//!   little ambient light), the generator `levelgen:terraces` and the level `levelgen:terraces`
//!   whose stem names that type and generator, with a `step` setting;
//! * the engine calls `handle_call` with `generator::METHOD` for every chunk of the level and the mod
//!   answers with a `Fill`: bedrock, stone, a grass top on terraces `step` blocks wide, one biome.
//!
//! The terrain is a pure function of the chunk, the seed and the settings, as a generator must be
//! (the same chunk asked twice, or on another machine, gives the same blocks).
use yoake_wasm_sdk::full::generator::{self, Fill, Request};
use yoake_wasm_sdk::full::server::{ServerMod, ext};

const TYPE: &str = r##"{
  "min_y": 0, "height": 64, "logical_height": 64, "has_skylight": true, "has_ceiling": false,
  "ambient_light": 0.2, "coordinate_scale": 1.0, "skybox": "overworld", "cardinal_light": "default",
  "attributes": {"minecraft:visual/sky_color": "#4466aa", "minecraft:visual/fog_color": "#8899cc"}
}"##;

const STEM: &str = r#"{
  "type": "levelgen:terrace_type",
  "generator": {"type": "yoake:mod", "id": "levelgen:terraces", "settings": {"step": 8}}
}"#;

const BASED_STEM: &str = r#"{
  "type": "minecraft:overworld",
  "generator": {
    "type": "yoake:mod", "id": "levelgen:terraces", "base": "minecraft:overworld",
    "biome_source": {"type": "minecraft:fixed", "biome": "levelgen:decorated"},
    "decorate": true
  }
}"#;

/// The surface height (the first free y) of the column at block (x, z).
pub fn surface(x: i32, z: i32, step: i32, seed: i64) -> i32 {
    let step = step.max(1);
    let terrace = (x.div_euclid(step) + z.div_euclid(step)).rem_euclid(4);
    8 + 4 * terrace + (seed & 1) as i32
}

#[derive(Default)]
struct Guest;

impl ServerMod for Guest {
    fn subscriptions() -> &'static [&'static str] {
        &[]
    }

    fn init(&mut self) {
        for (what, result) in [
            ("type", ext::register_type("levelgen:terrace_type", TYPE)),
            ("generator", ext::register_generator("levelgen:terraces")),
            ("level", ext::register_level("levelgen:terraces", STEM)),
            ("based level", ext::register_level("levelgen:based", BASED_STEM)),
        ] {
            if let Err(e) = result {
                println!("level-generator: registering the {what}: {e}");
            }
        }
    }

    fn handle_call(&mut self, _caller: String, method: String, payload: Vec<u8>) -> Result<Vec<u8>, String> {
        if method != generator::METHOD {
            return Err("unknown method".into());
        }
        let request = Request::parse(&payload).ok_or("a request this mod cannot read")?;
        if request.level == "levelgen:based" {
            let mut fill = Fill::new();
            let section = ((48 - request.min_y) / 16) as u16;
            let mut indices = vec![0u16; 4096];
            indices[2] = 1;
            fill.palette(section, &["minecraft:air", "minecraft:diamond_block"], indices);
            return Ok(fill.to_bytes());
        }
        let step = request.setting_int("step").unwrap_or(8) as i32;
        let sections = (request.height / 16) as u16;
        let mut fill = Fill::new();
        fill.biome("minecraft:plains");
        for index in 0..sections {
            let bottom = request.min_y + i32::from(index) * 16;
            let mut indices = vec![0u16; 4096];
            let mut any = false;
            for z in 0..16 {
                for x in 0..16 {
                    let top = surface(request.x * 16 + x, request.z * 16 + z, step, request.seed);
                    for y in 0..16 {
                        let wy = bottom + y;
                        let block = if wy == request.min_y {
                            3 // bedrock
                        } else if wy < top - 1 {
                            1 // stone
                        } else if wy == top - 1 {
                            2 // grass
                        } else {
                            0 // air
                        };
                        if block != 0 {
                            any = true;
                        }
                        indices[((y * 16 + z) * 16 + x) as usize] = block;
                    }
                }
            }
            if any {
                fill.palette(
                    index,
                    &["minecraft:air", "minecraft:stone", "minecraft:grass_block", "minecraft:bedrock"],
                    indices,
                );
            }
        }
        Ok(fill.to_bytes())
    }
}

yoake_wasm_sdk::export_server_full!(Guest);
