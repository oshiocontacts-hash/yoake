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

    /// The engine value types of the hooks (records, enums, variants).
    pub use bindings::yoake::mods::engine_types as et;
    pub use bindings::yoake::mods::{client_services as services, client_types as types};
    pub use bindings::yoake::mods::{gpu_services as gpu, gpu_types};
    pub use bindings::yoake::mods::{feedback, hud, logging, network, settings};

    #[allow(unused_imports)]
    use crate::prelude::*;

    include!("full_client_generated.rs");
}

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
            types::ItemStack { item: id.to_owned(), count, components: Vec::new() }
        }

        pub fn block_pos(x: i32, y: i32, z: i32) -> types::BlockPos {
            types::BlockPos { x, y, z }
        }

        pub fn vec3(x: f64, y: f64, z: f64) -> types::Vec3 {
            types::Vec3 { x, y, z }
        }

        /// Breaks a block as the player with their tool: drops, durability, experience, statistics, events.
        pub fn break_block_as(dimension: &str, player: &str, at: (i32, i32, i32)) -> Result<bool, String> {
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
                &types::InventoryChange { operation: types::InventoryOperation::Clear, item: id.to_owned(), count, components: None },
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
        pub fn container(dimension: &str, at: (i32, i32, i32)) -> Result<Vec<Option<types::ItemStack>>, String> {
            services::container_slots(dimension, block_pos(at.0, at.1, at.2))
        }

        /// Rearranges the container: the same items in other slots (the host checks the totals).
        pub fn set_container(dimension: &str, at: (i32, i32, i32), slots: &[Option<types::ItemStack>]) -> Result<(), String> {
            services::container_set_slots(dimension, block_pos(at.0, at.1, at.2), slots)
        }

        /// A transient `add-value` attribute modifier.
        pub fn add_attribute(player: &str, attribute: &str, id: &str, amount: f64) -> Result<(), String> {
            services::player_attribute_modifier_add(amount, attribute, id, types::AttributeOperation::AddValue, player)
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
    }
}
