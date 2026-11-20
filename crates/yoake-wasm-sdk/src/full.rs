//! Typed guest bindings of the full hook catalog (feature `full`): the `client-full` and `server-full`
//! worlds of `crates/yoake-wasm/wit/yoake-full.wit`.
//!
//! ```ignore
//! use yoake_wasm_sdk::full::{client::ClientMod, hook, client::et};
//!
//! #[derive(Default)]
//! struct Demo;
//! impl ClientMod for Demo {
//!     fn subscriptions() -> &'static [&'static str] { &[hook::HUD, hook::GUI_LAYER_PRE] }
//!     fn hud(&mut self) -> et::HudOutcome { /* draw with `commands` */ todo!() }
//!     fn gui_layer_pre(&mut self, layer: String) -> et::GuiLayerPreOutcome { /* cancel one layer */ todo!() }
//! }
//! yoake_wasm_sdk::export_client_full!(Demo);
//! ```
//!
//! A hook with a `&mut` draw object in the engine (`HudBatch`, `BlockEntityDraw`, ...) returns the
//! commands to apply instead; a hook that can cancel returns `true` (or a `cancel` field); a hook that
//! replaces or collects a value returns it. Every hook of a world has a default, so a mod overrides only
//! the hooks it lists in `subscriptions()`; the host calls only those.

/// Catalog hook names, as `init` returns them.
pub mod hook {
    include!("full_hooks_generated.rs");
}

/// The `client-full` world: every client-side hook, with the services of the `client-mod` world.
pub mod client {
    #[doc(hidden)]
    pub mod bindings {
        wit_bindgen::generate!({
            path: "../yoake-wasm/wit",
            world: "client-full",
            generate_all,
            pub_export_macro: true,
            default_bindings_module: "yoake_wasm_sdk::full::client::bindings",
        });
    }

    pub use bindings::yoake::mods::client_services as services;
    /// The engine value types of the hooks (records, enums, variants).
    pub use bindings::yoake::mods::engine_types as et;
    pub mod types {
        pub use super::bindings::yoake::mods::client_types::*;
        pub use super::bindings::yoake::mods::server_types::{
            ConfigValue, ConfigValueKind, StorageScope,
        };
    }
    pub use bindings::yoake::mods::{feedback, hud, logging, network, settings};
    pub use bindings::yoake::mods::{gpu_services as gpu_raw, gpu_types};

    /// Current scaled GUI width and height, in GUI pixels.
    pub fn gui_size() -> Result<types::Vec2, String> {
        services::gui_size()
    }

    /// Reserve a HUD position using the shared layout, including the top center.
    #[doc(alias = "hud_place")]
    pub fn hud_place_v2(
        corner: types::HudCornerV2,
        size: types::Vec2,
    ) -> Result<Option<types::Vec2>, String> {
        services::hud_place_v2(corner, size)
    }

    /// Enable or disable the OS input method on this mod's open screen.
    pub fn ime_set_allowed(allowed: bool) -> Result<(), String> {
        services::ime_set_allowed(allowed)
    }

    /// Place the IME candidate window using a GUI-scaled rectangle.
    pub fn ime_set_cursor_area(x: f32, y: f32, w: f32, h: f32) -> Result<(), String> {
        services::ime_set_cursor_area(h, w, x, y)
    }

    /// The stack on the cursor in the open screen, or `None` when it is empty. Needs
    /// `service.screen`. (Service 221.)
    pub fn screen_carried() -> Result<Option<types::ItemStack>, String> {
        services::screen_carried()
    }

    /// The local player's stacks as this client sees them: the main inventory (hotbar first),
    /// the armor and the off hand, without empty slots and without the cursor. Needs
    /// `service.client-view`. (Service 222.)
    pub fn local_player_stacks() -> Result<Vec<types::ItemStack>, String> {
        services::local_player_stacks()
    }

    /// A UI sound for this client alone: heard as it is, with no position (`sound_id` is
    /// `namespace:path`, `volume` 0 to 16, `pitch` 0 to 8). It plays when the next frame
    /// starts, so any hook may call it. Needs `service.audio`. (Service 223.)
    pub fn play_local_sound(sound_id: &str, volume: f32, pitch: f32) -> Result<(), String> {
        services::play_local_sound(pitch, sound_id, volume)
    }

    /// `play_local_sound` at a place in the world (attenuated, this client only). Needs
    /// `service.audio`. (Service 224.)
    pub fn play_local_sound_at(
        position: types::Vec3,
        sound_id: &str,
        volume: f32,
        pitch: f32,
    ) -> Result<(), String> {
        services::play_local_sound_at(pitch, position, sound_id, volume)
    }

    /// The id that names `texture` in the `hud-command`s a hook returns: use it as
    /// `HudTexture::id`, or as `texture:<id>` in `HudQuad::sprite`. It stays valid until the
    /// texture is dropped. Needs `service.hud`. (Service 225.)
    pub fn gui_texture_id(texture: &hud::GuiTexture) -> Result<String, String> {
        services::gui_texture_id(texture)
    }

    /// The four corners of the `width` x `height` rectangle with its top left at `(x, y)`,
    /// turned `degrees` clockwise about `pivot`, in the order a `HudQuad` takes them (round the
    /// shape from the top left). Pass them as `HudQuad::corners` to draw a turned sprite or
    /// texture.
    pub fn rotated_corners(
        x: f32,
        y: f32,
        width: f32,
        height: f32,
        degrees: f32,
        pivot: (f32, f32),
    ) -> Vec<(f32, f32)> {
        let (sin, cos) = degrees.to_radians().sin_cos();
        [(x, y), (x + width, y), (x + width, y + height), (x, y + height)]
            .into_iter()
            .map(|(px, py)| {
                let (dx, dy) = (px - pivot.0, py - pivot.1);
                (pivot.0 + dx * cos - dy * sin, pivot.1 + dx * sin + dy * cos)
            })
            .collect()
    }

    /// Owned GPU resources and checked command descriptors. Raw imports remain in `gpu_raw`.
    pub mod gpu {
        include!("gpu.rs");
    }

    #[allow(unused_imports)]
    use crate::prelude::*;

    include!("full_client_generated.rs");
}

/// Chunk generators written in a guest (F3-8): the request and the answer of the engine's
/// `yoake:generate-chunk` call.
#[path = "full_generator.rs"]
pub mod generator;

/// The `server-full` world: every server-side hook, plus the server services.
pub mod server {
    #[doc(hidden)]
    pub mod bindings {
        wit_bindgen::generate!({
            path: "../yoake-wasm/wit",
            world: "server-full",
            generate_all,
            pub_export_macro: true,
            default_bindings_module: "yoake_wasm_sdk::full::server::bindings",
        });
    }

    /// The engine value types of the hooks (records, enums, variants).
    pub use bindings::yoake::mods::engine_types as et;
    pub use bindings::yoake::mods::logging;
    /// The server services (`tell`, `run_command`, `player_teleport`, ...). Services the host does not
    /// implement return `Err("unsupported")`.
    pub use bindings::yoake::mods::server_services as services;
    pub use bindings::yoake::mods::server_types as types;

    #[allow(unused_imports)]
    use crate::prelude::*;

    include!("full_server_generated.rs");

    /// Friendlier wrappers over the server services (the raw ones are in [`services`]): plain
    /// arguments instead of records, `Err` strings as the host gives them. Every service of the
    /// host is validated host-side; these only save typing.
    pub mod ext {
        use super::{services, types};
        use crate::prelude::*;

        /// An item stack with default components.
        pub fn item(id: &str, count: u32) -> types::ItemStack {
            types::ItemStack {
                item: id.to_owned(),
                count,
                components: Vec::new(),
            }
        }

        pub fn block_pos(x: i32, y: i32, z: i32) -> types::BlockPos {
            types::BlockPos { x, y, z }
        }

        /// Set a player's food level (0 to 20) and saturation (0 up to the new food level).
        /// Needs `player-write`. (Service 226.)
        pub fn set_food(player: &str, food: i32, saturation: f32) -> Result<(), String> {
            services::player_set_food(food, player, saturation)
        }

        /// Registers a dimension type (a data pack's JSON object) for the world, from `init`.
        /// (Service 227.)
        pub fn register_type(id: &str, type_json: &str) -> Result<(), String> {
            services::level_register_type(id, type_json)
        }

        /// Registers a level (a level stem JSON object) for the world, from `init`: its own folder
        /// `dimensions/<ns>/<path>/`. (Service 228.)
        pub fn register_level(id: &str, stem_json: &str) -> Result<(), String> {
            services::level_register(id, stem_json)
        }

        /// Makes this package the generator of levels whose stem is
        /// `{"type":"yoake:mod","id":<id>}`, from `init`; answer `generator::METHOD` in `handle_call`.
        /// (Service 234.)
        pub fn register_generator(id: &str) -> Result<(), String> {
            services::generator_register(id)
        }

        /// Creates a level while the server runs; persistent ones come back at the next start.
        /// (Service 229.)
        pub fn create_level(id: &str, stem_json: &str, persistent: bool) -> Result<(), String> {
            services::level_create(id, persistent, stem_json)
        }

        /// Deletes a created level nobody is in. (Service 230.)
        pub fn delete_level(id: &str) -> Result<(), String> {
            services::level_delete(id)
        }

        /// Saves and closes a level at the next tick boundary; it opens again when entered.
        /// (Service 231.)
        pub fn unload_level(id: &str) -> Result<(), String> {
            services::level_unload(id)
        }

        /// Throws a level's terrain away, with a new seed for a created level. (Service 232.)
        pub fn regenerate_level(id: &str, seed: Option<i64>) -> Result<(), String> {
            services::level_regenerate(id, seed)
        }

        /// The world's level ids, the three vanilla ones first. (Service 233.)
        pub fn levels() -> Result<Vec<String>, String> {
            services::level_list()
        }

        pub fn vec3(x: f64, y: f64, z: f64) -> types::Vec3 {
            types::Vec3 { x, y, z }
        }

        /// Breaks a block as the player with their tool: drops, durability, experience, statistics, events.
        pub fn break_block_as(
            dimension: &str,
            player: &str,
            at: (i32, i32, i32),
        ) -> Result<bool, String> {
            services::break_block_as(dimension, player, block_pos(at.0, at.1, at.2))
        }

        /// Gives `count` of an item (default components); the number that did not fit.
        pub fn give(player: &str, id: &str, count: u32) -> Result<u32, String> {
            let left = services::give_items(&[item(id, count)], player)?;
            Ok(left.iter().map(|stack| stack.count).sum())
        }

        /// Takes exactly `count` of an item from the player (any components), all or nothing.
        pub fn take(player: &str, id: &str, count: u32) -> Result<bool, String> {
            services::inventory_apply(
                &types::InventoryChange {
                    operation: types::InventoryOperation::Clear,
                    item: id.to_owned(),
                    count,
                    components: None,
                },
                player,
            )
        }

        /// Takes exactly this stack (item, count and components) from the player, all or nothing.
        pub fn take_exact(player: &str, stack: &types::ItemStack) -> Result<bool, String> {
            services::inventory_apply(
                &types::InventoryChange {
                    operation: types::InventoryOperation::Clear,
                    item: stack.item.clone(),
                    count: stack.count,
                    components: Some(stack.components.clone()),
                },
                player,
            )
        }

        /// Every slot of the container at the position (both halves of a double chest).
        pub fn container(
            dimension: &str,
            at: (i32, i32, i32),
        ) -> Result<Vec<Option<types::ItemStack>>, String> {
            services::container_slots(dimension, block_pos(at.0, at.1, at.2))
        }

        /// Rearranges the container: the same items in other slots (the host checks the totals).
        pub fn set_container(
            dimension: &str,
            at: (i32, i32, i32),
            slots: &[Option<types::ItemStack>],
        ) -> Result<(), String> {
            services::container_set_slots(dimension, block_pos(at.0, at.1, at.2), slots)
        }

        /// A transient `add-value` attribute modifier.
        pub fn add_attribute(
            player: &str,
            attribute: &str,
            id: &str,
            amount: f64,
        ) -> Result<(), String> {
            services::player_attribute_modifier_add(
                amount,
                attribute,
                id,
                types::AttributeOperation::AddValue,
                player,
            )
        }

        /// Calls another package's `handle_call`.
        pub fn call(target: &str, method: &str, payload: &[u8]) -> Result<Vec<u8>, String> {
            services::mod_call(method, payload, target)
        }

        /// This package's own per-world storage.
        pub fn store_get(key: &str) -> Result<Option<Vec<u8>>, String> {
            services::world_storage_read(key)
        }

        pub fn store_put(key: &str, data: &[u8]) -> Result<(), String> {
            services::world_storage_write(data, key)
        }

        /// Declares a data attachment of this package (`id` starts with the package id and a colon;
        /// `kind` is `player`, `entity`, `block-entity`, `chunk` or `level`). Call it in `init` or a hook;
        /// defining it again with the same policy is fine.
        pub fn attachment_define(id: &str, kind: &str, persistent: bool, synced: bool, copy_on_death: bool) -> Result<(), String> {
            services::attachment_define(copy_on_death, id, kind, persistent, synced)
        }

        /// Reads an attachment's bytes. `holder`: a player UUID, an entity id, `x,y,z`, `cx,cz`, or `""` for the level.
        pub fn attachment_get(id: &str, kind: &str, holder: &str) -> Result<Option<Vec<u8>>, String> {
            services::attachment_get(holder, id, kind)
        }

        /// Stores bytes (at most 64 KiB) on a holder; synced attachments reach the clients that see it.
        pub fn attachment_set(id: &str, kind: &str, holder: &str, data: &[u8]) -> Result<(), String> {
            services::attachment_set(data, holder, id, kind)
        }

        /// Removes the value; true when there was one.
        pub fn attachment_remove(id: &str, kind: &str, holder: &str) -> Result<bool, String> {
            services::attachment_remove(holder, id, kind)
        }
    }
}
