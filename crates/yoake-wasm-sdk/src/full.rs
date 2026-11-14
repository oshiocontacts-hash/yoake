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
}
