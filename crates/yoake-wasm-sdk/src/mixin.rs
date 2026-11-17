use crate::prelude::*;

#[doc(hidden)]
pub mod bindings {
    #[cfg(not(feature = "mixin-config"))]
    wit_bindgen::generate!({
        path: "../yoake-wasm/wit-inject",
        world: "inject-mod",
        generate_all,
        pub_export_macro: true,
        default_bindings_module: "yoake_wasm_sdk::mixin::bindings",
    });
    #[cfg(feature = "mixin-config")]
    wit_bindgen::generate!({
        path: "../yoake-wasm/wit-inject-config",
        world: "inject-mod",
        generate_all,
        pub_export_macro: true,
        default_bindings_module: "yoake_wasm_sdk::mixin::bindings",
    });
}

#[cfg(not(feature = "mixin-config"))]
pub use bindings::exports::yoake::mods::inject::{
    Guest as InjectionGuest, HeadOutcome, RecordEntry, Registration, Value, ValueNode, ValueTree,
};
#[cfg(feature = "mixin-config")]
pub use bindings::exports::yoake::mods1_1_0::inject::{
    Guest as InjectionGuest, HeadOutcome, RecordEntry, Registration, Value, ValueNode, ValueTree,
};
#[cfg(feature = "mixin-config")]
pub use bindings::exports::yoake::mods1_2_0::config_events::Guest as ConfigEvents;
#[cfg(feature = "mixin-config")]
pub use bindings::yoake::mods1_2_0::config::{ConfigValue, ConfigValueKind, config_get};

pub struct Handler {
    pub site: &'static str,
    pub head: Option<fn(&[Value]) -> HeadOutcome>,
    pub returned: Option<fn(Value) -> Option<Value>>,
    pub overwrite: Option<fn(&[Value]) -> Value>,
}

impl Handler {
    pub const fn points(&self) -> u8 {
        (if self.head.is_some() { 1 } else { 0 })
            | (if self.returned.is_some() { 2 } else { 0 })
            | (if self.overwrite.is_some() { 4 } else { 0 })
    }
}

pub fn registrations(handlers: &[Handler]) -> Vec<Registration> {
    handlers
        .iter()
        .map(|handler| Registration {
            site: handler.site.into(),
            points: handler.points(),
        })
        .collect()
}

pub fn head(handlers: &[Handler], site: &str, arguments: &[Value]) -> HeadOutcome {
    handlers
        .iter()
        .find(|handler| handler.site == site)
        .and_then(|handler| handler.head)
        .map_or(HeadOutcome::Proceed, |callback| callback(arguments))
}

pub fn returned(handlers: &[Handler], site: &str, result: Value) -> Option<Value> {
    handlers
        .iter()
        .find(|handler| handler.site == site)
        .and_then(|handler| handler.returned)
        .and_then(|callback| callback(result))
}

pub fn overwrite(handlers: &[Handler], site: &str, arguments: &[Value]) -> Value {
    handlers
        .iter()
        .find(|handler| handler.site == site)
        .and_then(|handler| handler.overwrite)
        .map_or(Value::Unit, |callback| callback(arguments))
}

pub fn record_field<'value>(tree: &'value ValueTree, name: &str) -> Option<&'value ValueNode> {
    let ValueNode::Object(entries) = tree.nodes.get(tree.root as usize)? else {
        return None;
    };
    let entry = entries.iter().find(|entry| entry.key == name)?;
    tree.nodes.get(entry.node as usize)
}

#[macro_export]
macro_rules! export_mixin {
    ($handlers:expr) => {
        $crate::export_mixin!($handlers, config_changed = |_: &str, _| {});
    };
    ($handlers:expr, config_changed = $callback:expr) => {
        const _: () = {
            struct Mixin;
            impl $crate::mixin::InjectionGuest for Mixin {
                fn registrations() -> $crate::__private::Vec<$crate::mixin::Registration> {
                    $crate::mixin::registrations($handlers)
                }

                fn inject_head(
                    site: $crate::__private::String,
                    args: $crate::__private::Vec<$crate::mixin::Value>,
                ) -> $crate::mixin::HeadOutcome {
                    $crate::mixin::head($handlers, &site, &args)
                }

                fn inject_return(
                    site: $crate::__private::String,
                    result: $crate::mixin::Value,
                ) -> Option<$crate::mixin::Value> {
                    $crate::mixin::returned($handlers, &site, result)
                }

                fn inject_overwrite(
                    site: $crate::__private::String,
                    args: $crate::__private::Vec<$crate::mixin::Value>,
                ) -> $crate::mixin::Value {
                    $crate::mixin::overwrite($handlers, &site, &args)
                }
            }
            $crate::__export_mixin_config!(Mixin, $callback);
    $crate::mixin::bindings::export!(Mixin with_types_in $crate::mixin::bindings);
        };
    };
}

#[cfg(feature = "mixin-config")]
#[doc(hidden)]
#[macro_export]
macro_rules! __export_mixin_config {
    ($guest:ident, $callback:expr) => {
        impl $crate::mixin::ConfigEvents for $guest {
            fn config_changed(key: $crate::__private::String, value: $crate::mixin::ConfigValue) {
                ($callback)(&key, value);
            }
        }
    };
}

#[cfg(not(feature = "mixin-config"))]
#[doc(hidden)]
#[macro_export]
macro_rules! __export_mixin_config {
    ($guest:ident, $callback:expr) => {};
}
