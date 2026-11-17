use std::sync::atomic::{AtomicU64, Ordering};

use yoake_wasm_sdk::export_mixin;
use yoake_wasm_sdk::mixin::{ConfigValue, ConfigValueKind, Handler, Value, config_get};

const TARGET: &str =
    "game::tick_sleep_timer#198a5917f750358ff70ce3f80ede1aa2c142d914cbe7bed985f5b6f4ed29a7b7";
static COUNT: AtomicU64 = AtomicU64::new(u64::MAX);

fn config_changed(key: &str, value: ConfigValue) {
    if key == "count" && value.kind == ConfigValueKind::Int {
        COUNT.store(value.int_value as u64, Ordering::Relaxed);
    }
}

fn configured_timer(native: Value) -> Option<Value> {
    if COUNT.load(Ordering::Relaxed) == u64::MAX {
        match config_get("count") {
            Ok(value) => config_changed("count", value),
            Err(_) => return Some(native),
        }
    }
    Some(Value::Unsigned(COUNT.load(Ordering::Relaxed)))
}

static HANDLERS: &[Handler] = &[Handler {
    site: TARGET,
    head: None,
    returned: Some(configured_timer),
    overwrite: None,
}];

export_mixin!(HANDLERS, config_changed = config_changed);
