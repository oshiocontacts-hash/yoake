//! Client-side F4M-7 example for the Cinder Fox: repaints a runtime custom mob texture when
//! synced `warmth` changes (updated by `fox_ai` on damage) and binds it to the fox locally.
//! REM-17: also draws a gold cube and an apple floating over the fox through `mob-layer`'s
//! model commands (`cube` and `item`).
use yoake_wasm_sdk::full::client::{self, ClientMod, et};
use yoake_wasm_sdk::full::hook;

#[derive(Default)]
struct FoxTexture {
    textures: Vec<(u32, u32)>,
}

impl FoxTexture {
    fn texture_for(&mut self, mob: u32) -> Option<u32> {
        if let Some((_, id)) = self.textures.iter().find(|(m, _)| *m == mob) {
            return Some(*id);
        }
        let id = client::mob_texture_create(64, 32).ok()?;
        self.textures.push((mob, id));
        Some(id)
    }
}

impl ClientMod for FoxTexture {
    fn subscriptions() -> &'static [&'static str] {
        &[
            hook::SYNCED_CHANGED_CLIENT,
            hook::PLAYER_RENDER_LISTENED,
            hook::MOB_LAYER,
        ]
    }

    /// Mob layers are only asked for while a mod listens.
    fn player_render_listened(&mut self, _listened: bool) -> Option<bool> {
        Some(true)
    }

    fn mob_layer(
        &mut self,
        mob: et::NativeMobRender,
        _who: Option<et::NativePlayerRender>,
    ) -> Vec<et::ModelCommand> {
        if mob.kind != "sample_mobs:cinder_fox" {
            return Vec::new();
        }
        let face = |sprite: &str| {
            Some(et::ModelCubeFace {
                sprite: sprite.into(),
                uv: None,
            })
        };
        vec![
            et::ModelCommand::Push,
            et::ModelCommand::Translate((0.0, 1.0, 0.0)),
            et::ModelCommand::Cube(et::ModelCube {
                min: (-2.0, 0.0, -2.0),
                max: (2.0, 4.0, 2.0),
                faces: vec![
                    face("block/gold_block"),
                    face("block/gold_block"),
                    face("block/gold_block"),
                    face("block/gold_block"),
                    face("block/gold_block"),
                    face("block/gold_block"),
                ],
                tint: 0x00ff_ffff,
            }),
            et::ModelCommand::Translate((0.0, 0.6, 0.0)),
            et::ModelCommand::Item(et::ModelItem {
                stack: et::ItemStack {
                    id: "minecraft:apple".into(),
                    count: 1,
                    components: Vec::new(),
                },
                display: et::DisplayContext::Ground,
                owner_yaw: None,
                seed: 0,
            }),
            et::ModelCommand::Pop,
        ]
    }

    fn synced_changed_client(
        &mut self,
        mob: u32,
        key: String,
        _old: et::MobScriptValue,
        new: et::MobScriptValue,
    ) {
        if key != "warmth" {
            return;
        }
        let et::MobScriptValue::Int(warmth) = new else {
            return;
        };
        let Some(tex_id) = self.texture_for(mob) else {
            return;
        };
        let r = ((warmth.clamp(0, 10) * 20) as u8).saturating_add(40);
        let base_pixel = [r, 220, 255, 255];
        let full: Vec<u8> = base_pixel
            .iter()
            .copied()
            .cycle()
            .take(64 * 32 * 4)
            .collect();
        let _ = client::mob_texture_write(tex_id, (0, 0), (64, 32), &full);

        // Exercise partial sub-region upload as well: bright gold band on head/upper UVs.
        let sub_pixel = [255, 220, 40, 255];
        let sub: Vec<u8> = sub_pixel
            .iter()
            .copied()
            .cycle()
            .take(32 * 16 * 4)
            .collect();
        let _ = client::mob_texture_write(tex_id, (0, 0), (32, 16), &sub);
        let _ = client::mob_texture_bind(mob, "default", Some(tex_id));
        println!("fox_texture: mob {mob} warmth {warmth} bound texture {tex_id}");
    }
}

yoake_wasm_sdk::export_client_full!(FoxTexture);
