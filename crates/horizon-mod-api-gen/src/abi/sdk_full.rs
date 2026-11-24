//! The guest SDK's typed bindings of the full catalog (`yoake_wasm_sdk::full`): for each world a trait
//! with a default for every hook, the hook-name constants, and a macro that implements the generated
//! WIT exports by calling the trait. Committed under `crates/yoake-wasm-sdk/src/` and checked by a
//! test, since the SDK is a separate guest crate that cannot run this generator.
use super::wit_full::{FullWit, HookPlan, Main, ResultForm, Side, generate_full_wit, kebab};
use std::fmt::Write as _;
use std::path::Path;

fn camel(wit: &str) -> String {
    use heck::ToUpperCamelCase;
    wit.trim_start_matches('%').to_upper_camel_case()
}

const KEYWORDS: &[&str] = &[
    "as", "break", "const", "continue", "crate", "else", "enum", "extern", "false", "fn", "for", "if", "impl", "in",
    "let", "loop", "match", "mod", "move", "mut", "pub", "ref", "return", "self", "Self", "static", "struct", "super",
    "trait", "true", "type", "unsafe", "use", "where", "while", "async", "await", "dyn", "abstract", "become", "box",
    "do", "final", "macro", "override", "priv", "typeof", "unsized", "virtual", "yield", "try",
];

fn snake(wit: &str) -> String {
    let name = wit.trim_start_matches('%').replace('-', "_");
    if KEYWORDS.contains(&name.as_str()) { format!("r#{name}") } else { name }
}

/// Splits `a, b<c, d>, e` at the top-level commas.
fn split_args(text: &str) -> Vec<&str> {
    let mut parts = Vec::new();
    let (mut depth, mut start) = (0usize, 0usize);
    for (index, ch) in text.char_indices() {
        match ch {
            '<' => depth += 1,
            '>' => depth -= 1,
            ',' if depth == 0 => {
                parts.push(text[start..index].trim());
                start = index + 1;
            }
            _ => {}
        }
    }
    let last = text[start..].trim();
    if !last.is_empty() {
        parts.push(last);
    }
    parts
}

/// The Rust type wit-bindgen gives a WIT type in the guest.
fn guest_type(wit: &str) -> String {
    let wit = wit.trim();
    if let Some(inner) = wit.strip_prefix("list<").and_then(|r| r.strip_suffix('>')) {
        return format!("Vec<{}>", guest_type(inner));
    }
    if let Some(inner) = wit.strip_prefix("option<").and_then(|r| r.strip_suffix('>')) {
        return format!("Option<{}>", guest_type(inner));
    }
    if let Some(inner) = wit.strip_prefix("tuple<").and_then(|r| r.strip_suffix('>')) {
        let parts: Vec<String> = split_args(inner).into_iter().map(guest_type).collect();
        return format!("({},)", parts.join(", "));
    }
    if let Some(inner) = wit.strip_prefix("result<").and_then(|r| r.strip_suffix('>')) {
        let parts = split_args(inner);
        let ok = if parts[0] == "_" { "()".to_owned() } else { guest_type(parts[0]) };
        let err = parts.get(1).map(|p| guest_type(p)).unwrap_or_else(|| "()".to_owned());
        return format!("::core::result::Result<{ok}, {err}>");
    }
    match wit {
        "bool" | "char" | "u8" | "u16" | "u32" | "u64" | "f32" | "f64" => wit.to_owned(),
        "s8" => "i8".into(),
        "s16" => "i16".into(),
        "s32" => "i32".into(),
        "s64" => "i64".into(),
        "string" | "text-json" | "json" => "String".into(),
        "nbt" => "Vec<u8>".into(),
        other => format!("et::{}", camel(other)),
    }
}

/// The default answer of a hook in the trait.
fn default_answer(plan: &HookPlan) -> String {
    match &plan.result {
        ResultForm::None => String::new(),
        ResultForm::Commands => "Vec::new()".to_owned(),
        ResultForm::Direct => match &plan.main {
            Main::Cancel => "false".to_owned(),
            Main::Replace(_) => "None".to_owned(),
            Main::Collect { list: true, .. } => {
                let aggregate = plan.params.last().map(|p| snake(&p.name)).unwrap_or_default();
                aggregate
            }
            Main::Collect { .. } => "None".to_owned(),
            Main::None => String::new(),
        },
        ResultForm::Outcome(record) => {
            let mut fields = Vec::new();
            match &plan.main {
                Main::Cancel => fields.push("cancel: false".to_owned()),
                Main::Replace(_) => fields.push("value: None".to_owned()),
                Main::Collect { list: true, .. } => {
                    let aggregate = plan.params.last().map(|p| snake(&p.name)).unwrap_or_default();
                    fields.push(format!("value: {aggregate}"));
                }
                Main::Collect { .. } => fields.push("value: None".to_owned()),
                Main::None => {}
            }
            for effect in &plan.effects {
                fields.push(format!("{}: Vec::new()", snake(&effect.wit_field)));
            }
            for extra in &plan.extras {
                fields.push(format!("{}: None", snake(&extra.wit_field)));
            }
            format!("et::{} {{ {} }}", camel(record), fields.join(", "))
        }
    }
}

fn on_side(plan: &HookPlan, client: bool) -> bool {
    match plan.side {
        Side::Both => true,
        Side::Client => client,
        Side::Server => !client,
    }
}

fn world_text(full: &FullWit, client: bool) -> String {
    let (world, trait_name, macro_name, interface, bindings) = if client {
        ("client", "ClientMod", "export_client_full", "client_hooks", "client")
    } else {
        ("server", "ServerMod", "export_server_full", "server_hooks", "server")
    };
    let plans: Vec<&HookPlan> = full
        .plans
        .iter()
        .filter(|plan| on_side(plan, client))
        .collect();
    let mut text = format!(
        "// @generated by horizon-mod-api-gen; do not edit.\n// The {world} half of the full catalog: {} hooks.\n\n",
        plans.len() + 1
    );
    let _ = writeln!(
        text,
        "/// A `{world}-full` mod. Override the hooks you need and list them in [`{trait_name}::subscriptions`]."
    );
    let _ = writeln!(
        text,
        "/// The type must be `Default`; one instance lives for the whole session."
    );
    let _ = writeln!(
        text,
        "#[allow(unused_variables)]\npub trait {trait_name}: Default + 'static {{"
    );
    text.push_str("    /// Hook names (see [`hook`]) the host should call. Runs once, from `init`.\n    fn subscriptions() -> &'static [&'static str] {\n        &[]\n    }\n    /// Called once right before the subscription list is returned.\n    fn init(&mut self) {}\n");
    if client {
        text.push_str("    fn gpu_frame(&mut self) {}\n");
    }
    {
        text.push_str("    /// Another package's `mod_call` reaches this: `caller` is its package id, `method` and `payload` are the two\n    /// packages' own protocol (up to 1 MiB each way). The default refuses every call.\n    fn handle_call(&mut self, caller: String, method: String, payload: Vec<u8>) -> Result<Vec<u8>, String> {\n        Err(String::from(\"this package does not handle calls\"))\n    }\n");
    }
    for plan in &plans {
        let params: Vec<String> =
            plan.params.iter().map(|p| format!("{}: {}", snake(&p.name), guest_type(&p.wit))).collect();
        let sep = if params.is_empty() { "" } else { ", " };
        let result = plan.result_wit.as_deref().map(|r| format!(" -> {}", guest_type(r))).unwrap_or_default();
        let _ = writeln!(text, "    fn {}(&mut self{sep}{}){result} {{ {} }}", snake(&plan.name), params.join(", "), default_answer(plan));
    }
    text.push_str(
        "    fn config_changed(&mut self, key: String, value: types::ConfigValue) {}\n}\n\n",
    );
    let _ = writeln!(
        text,
        "/// Wires a [`{trait_name}`] type to the component exports (`init` and the hooks)."
    );
    let _ = writeln!(
        text,
        "#[macro_export]\nmacro_rules! {macro_name} {{\n    ($ty:ty) => {{\n        const _: () = {{"
    );
    let _ = writeln!(
        text,
        "            use $crate::full::{bindings}::{trait_name} as _;"
    );
    let _ = writeln!(
        text,
        "            use $crate::full::{bindings}::bindings::exports::yoake::mods::{interface}::Guest as HooksGuest;"
    );
    let _ = writeln!(
        text,
        "            #[allow(unused_imports)]\n            use $crate::full::{bindings}::bindings::yoake::mods::engine_types as et;"
    );
    let _ = writeln!(
        text,
        "            use $crate::full::{bindings}::bindings::yoake::mods::server_types as types;"
    );
    text.push_str("            $crate::__export_realloc!();\n            struct Shim;\n");
    let _ = writeln!(text, "            impl $crate::full::{bindings}::bindings::Guest for Shim {{");
    text.push_str("                fn init() -> $crate::__private::Vec<$crate::__private::String> {\n                    $crate::rt::install::<$ty>(<$ty as ::core::default::Default>::default());\n                    $crate::rt::with::<$ty, _>(|m| m.init());\n");
    let _ = writeln!(text, "                    <$ty as $crate::full::{bindings}::{trait_name}>::subscriptions().iter().map(|s| $crate::__private::ToString::to_string(*s)).collect()");
    text.push_str("                }\n");
    if client {
        text.push_str("                fn gpu_frame() {\n                    $crate::rt::with::<$ty, _>(|m| m.gpu_frame()).expect(\"mod instance\");\n                }\n");
    }
    {
        text.push_str("                fn handle_call(caller: $crate::__private::String, method: $crate::__private::String, payload: $crate::__private::Vec<u8>) -> ::core::result::Result<$crate::__private::Vec<u8>, $crate::__private::String> {\n                    $crate::rt::with::<$ty, _>(|m| m.handle_call(caller, method, payload)).expect(\"mod instance\")\n                }\n");
    }
    text.push_str("            }\n\n            #[allow(unused_variables)]\n            impl HooksGuest for Shim {\n");
    for plan in &plans {
        let params: Vec<String> = plan.params.iter().enumerate().map(|(i, p)| format!("a{i}: {}", guest_type(&p.wit))).collect();
        let args: Vec<String> = (0..plan.params.len()).map(|i| format!("a{i}")).collect();
        let result = plan.result_wit.as_deref().map(|r| format!(" -> {}", guest_type(r))).unwrap_or_default();
        let name = snake(&plan.name);
        let _ = writeln!(
            text,
            "                fn {name}({}){result} {{ $crate::rt::with::<$ty, _>(|m| m.{name}({})).expect(\"mod instance\") }}",
            params.join(", "),
            args.join(", ")
        );
    }
    text.push_str("                fn config_changed(key: $crate::__private::String, value: types::ConfigValue) { $crate::rt::with::<$ty, _>(|mod_instance| mod_instance.config_changed(key, value)).expect(\"mod instance\") }\n            }\n\n");
    let _ = writeln!(
        text,
        "            $crate::full::{bindings}::bindings::export!(Shim with_types_in $crate::full::{bindings}::bindings);"
    );
    text.push_str("        };\n    };\n}\n");
    text
}

fn hook_constants(full: &FullWit) -> String {
    let mut text = String::from("// @generated by horizon-mod-api-gen; do not edit.\n// Catalog hook names, as `init` returns them.\n");
    text.push_str("pub const GPU_FRAME: &str = \"gpu_frame\";\n");
    for plan in &full.plans {
        let _ = writeln!(text, "pub const {}: &str = {:?};", plan.name.to_uppercase(), plan.name);
    }
    text.push_str("pub const CONFIG_CHANGED: &str = \"config_changed\";\n");
    text
}

/// The three generated guest SDK files: client world, server world, hook constants.
pub struct SdkFull {
    pub client: String,
    pub server: String,
    pub hooks: String,
}

pub fn generate_sdk_full(root: &Path) -> Result<SdkFull, String> {
    let full = generate_full_wit(root)?;
    let _ = kebab;
    Ok(SdkFull { client: world_text(&full, true), server: world_text(&full, false), hooks: hook_constants(&full) })
}
