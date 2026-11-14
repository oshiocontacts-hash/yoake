//! Host bindings for the full-catalog worlds of `wit/yoake-full.wit`: `client-full` (every client
//! side hook as an export) and `server-full` (every server side hook, plus the server services).
//! The service interfaces of the frozen `client-mod` world and the engine types are shared with
//! [`crate::bindings`], so one host context implements them once.
#![allow(dead_code)]

/// Host bindings for `client-full`.
pub mod client {
    wasmtime::component::bindgen!({
        path: "wit",
        world: "client-full",
        with: {
            "yoake:mods/types": crate::bindings::yoake::mods::types,
            "yoake:mods/logging": crate::bindings::yoake::mods::logging,
            "yoake:mods/feedback": crate::bindings::yoake::mods::feedback,
            "yoake:mods/hud": crate::bindings::yoake::mods::hud,
            "yoake:mods/network": crate::bindings::yoake::mods::network,
            "yoake:mods/settings": crate::bindings::yoake::mods::settings,
            "yoake:mods/client-world": crate::bindings::yoake::mods::client_world,
            "yoake:mods/player": crate::bindings::yoake::mods::player,
            "yoake:mods/instance": crate::bindings::yoake::mods::instance,
            "yoake:mods/accounts": crate::bindings::yoake::mods::accounts,
            "yoake:mods/screen": crate::bindings::yoake::mods::screen,
        },
    });
}

/// Host bindings for `server-full`.
pub mod server {
    wasmtime::component::bindgen!({
        path: "wit",
        world: "server-full",
        with: {
            "yoake:mods/types": crate::bindings::yoake::mods::types,
            "yoake:mods/logging": crate::bindings::yoake::mods::logging,
            "yoake:mods/engine-types": super::client::yoake::mods::engine_types,
        },
    });
}
