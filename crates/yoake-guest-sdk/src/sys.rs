//! The raw imports of module `env` (see `docs/yoake2/ENGINE_CALL_ABI.md`). On wasm32 they are the host's
//! functions; on any other target (host unit tests, `cargo check`) they are inert stand-ins that fail.

#[cfg(target_arch = "wasm32")]
#[link(wasm_import_module = "env")]
unsafe extern "C" {
    pub fn engine_call(handle: i64, id: i32, ptr: i32, len: i32, out_ptr: i32, out_cap: i32) -> i32;
    pub fn engine_take(ticket: i32, out_ptr: i32, out_cap: i32) -> i32;
    pub fn original(handle: i64, out: i32) -> i32;
    pub fn get(handle: i64, field: i32, out: i32) -> i32;
    pub fn set(handle: i64, field: i32, bits: i64) -> i32;
    pub fn report(slot: i32, value: i64);
    pub fn hook_register(id: i32, priority: i32) -> i32;
}

#[cfg(not(target_arch = "wasm32"))]
mod inert {
    pub unsafe fn engine_call(_: i64, _: i32, _: i32, _: i32, _: i32, _: i32) -> i32 {
        -6
    }
    pub unsafe fn engine_take(_: i32, _: i32, _: i32) -> i32 {
        -7
    }
    pub unsafe fn original(_: i64, _: i32) -> i32 {
        1
    }
    pub unsafe fn get(_: i64, _: i32, _: i32) -> i32 {
        1
    }
    pub unsafe fn set(_: i64, _: i32, _: i64) -> i32 {
        1
    }
    pub unsafe fn report(_: i32, _: i64) {}
    pub unsafe fn hook_register(_: i32, _: i32) -> i32 {
        0
    }
}
#[cfg(not(target_arch = "wasm32"))]
pub use inert::*;
