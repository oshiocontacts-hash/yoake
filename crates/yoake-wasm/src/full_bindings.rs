//! Compile check for the full-catalog worlds of `wit/yoake-full.wit`: the generated WIT must be
//! usable from Rust. No runtime wiring yet; the frozen `client-mod` world stays in `lib.rs`.
#![allow(dead_code)]

/// Host bindings for `client-full`: every client-side hook as an export.
pub mod client {
    wasmtime::component::bindgen!({
        path: "wit",
        world: "client-full",
    });
}

/// Host bindings for `server-full`: every server-side hook as an export, plus the draft
/// `server-services` imports.
pub mod server {
    wasmtime::component::bindgen!({
        path: "wit",
        world: "server-full",
    });
}
