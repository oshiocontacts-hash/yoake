use quote::ToTokens;
use serde::Deserialize;
use std::{collections::HashSet, fs, path::Path};

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Subset {
    abi_version: u32,
    hooks: Vec<Hook>,
    services: Vec<Service>,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Hook {
    name: String,
    abi: bool,
    id: u32,
    slot: String,
    event: String,
    contract: String,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Service {
    name: String,
    abi: bool,
    id: u32,
    kind: String,
    slot: Option<String>,
}

fn subset() -> Result<Subset, String> {
    serde_json::from_str(include_str!("../abi-subset.json")).map_err(|error| error.to_string())
}

fn service_signature(kind: &str) -> Result<(&'static str, &'static str), String> {
    Ok(match kind {
        "log" => ("StrView, StrView", "YoakeStrView, YoakeStrView"),
        "feedback" => ("Handle, StrView", "YoakeHandle, YoakeStrView"),
        "world" => ("Handle, *mut u32, *mut OutputBuffer", "YoakeHandle, uint32_t *, YoakeOutputBuffer *"),
        "text" => ("Handle, Handle, StrView, Point, u32, u32", "YoakeHandle, YoakeHandle, YoakeStrView, YoakePoint, uint32_t, uint32_t"),
        "rect" => ("Handle, Handle, Rect, u32", "YoakeHandle, YoakeHandle, YoakeRect, uint32_t"),
        "net" => ("Handle, StrView, ByteView, *mut OutputBuffer", "YoakeHandle, YoakeStrView, YoakeByteView, YoakeOutputBuffer *"),
        "get" => ("Handle, StrView, *mut OutputBuffer", "YoakeHandle, YoakeStrView, YoakeOutputBuffer *"),
        "set" => ("Handle, StrView, StrView", "YoakeHandle, YoakeStrView, YoakeStrView"),
        other => return Err(format!("unknown ABI service kind {other}")),
    })
}

fn fingerprint(hash: &mut u64, bytes: &[u8]) {
    for byte in bytes {
        *hash = (*hash ^ u64::from(*byte)).wrapping_mul(0x100000001b3);
    }
    *hash = (*hash ^ 255).wrapping_mul(0x100000001b3);
}

fn package_version(manifest: &str) -> Result<String, String> {
    let mut in_package = false;
    for line in manifest.lines().map(str::trim) {
        if line.starts_with('[') {
            in_package = line == "[package]";
        } else if in_package {
            if let Some((name, value)) = line.split_once('=') {
                if name.trim() == "version" {
                    return serde_json::from_str(value.trim())
                        .map_err(|error| format!("invalid literal horizon-core package version: {error}"));
                }
            }
        }
    }
    Err("horizon-core must declare a literal package version".into())
}

fn engine_fingerprint(root: &Path) -> Result<u64, String> {
    fn visit(root: &Path, path: &Path, hash: &mut u64) -> Result<(), String> {
        println!("cargo:rerun-if-changed={}", path.display());
        let mut entries = fs::read_dir(path)
            .map_err(|error| error.to_string())?
            .map(|entry| entry.map(|entry| entry.path()).map_err(|error| error.to_string()))
            .collect::<Result<Vec<_>, _>>()?;
        entries.sort();
        for path in entries {
            if fs::symlink_metadata(&path).map_err(|error| error.to_string())?.file_type().is_symlink() {
                continue;
            }
            if path.is_dir() {
                if !matches!(path.file_name().and_then(|name| name.to_str()), Some("target" | ".git" | ".local")) {
                    visit(root, &path, hash)?;
                }
            } else if path.extension().is_some_and(|extension| extension == "rs" || extension == "toml" || extension == "json" || extension == "in" || extension == "h") {
                println!("cargo:rerun-if-changed={}", path.display());
                let name = path.strip_prefix(root).map_err(|error| error.to_string())?.to_string_lossy().replace('\\', "/");
                fingerprint(hash, name.as_bytes());
                fingerprint(hash, &fs::read(&path).map_err(|error| error.to_string())?);
            }
        }
        Ok(())
    }
    let mut hash = 0xcbf29ce484222325;
    fingerprint(&mut hash, &fs::read(root.join("Cargo.toml")).map_err(|error| error.to_string())?);
    visit(root, &root.join("crates"), &mut hash)?;
    Ok(hash)
}

fn selected_schema(root: &Path, subset: &Subset) -> Result<String, String> {
    let generated = super::sources(root, super::FillMode::Require)?;
    let hooks: serde_json::Value = serde_json::from_str(&fs::read_to_string(root.join("native-hooks.json")).map_err(|error| error.to_string())?).map_err(|error| error.to_string())?;
    let catalog = super::catalog_hooks(root, &hooks)?;
    let mut schema = include_str!("../abi-subset.json").to_owned();
    for (source, names) in [
        (generated.hooks, subset.hooks.iter().filter(|hook| hook.abi).map(|hook| hook.name.as_str()).collect::<Vec<_>>()),
        (generated.host, subset.services.iter().filter(|service| service.abi && service.kind != "log").map(|service| service.name.as_str()).chain(["window"]).collect::<Vec<_>>()),
    ] {
        let parsed = syn::parse_file(&source).map_err(|error| error.to_string())?;
        for name in names {
            let method = parsed.items.iter().filter_map(|item| if let syn::Item::Trait(item) = item { Some(item) } else { None })
                .flat_map(|item| &item.items).find_map(|item| match item {
                    syn::TraitItem::Fn(method) if method.sig.ident == name => Some(method),
                    _ => None,
                }).ok_or_else(|| format!("ABI subset entry {name} is not in the generated catalog API"))?;
            schema.push_str(&method.sig.to_token_stream().to_string());
        }
    }
    for hook in subset.hooks.iter().filter(|hook| hook.abi) {
        let catalog_hook = catalog.iter().find(|entry| entry["name"].as_str() == Some(hook.name.as_str())).ok_or_else(|| format!("missing ABI hook {}", hook.name))?;
        if catalog_hook["mod_api"]["contract"].as_str() != Some(hook.contract.as_str()) {
            return Err(format!("ABI hook {} changed contract; revise the ABI explicitly", hook.name));
        }
        schema.push_str(&catalog_hook.to_string());
    }
    Ok(schema)
}

/// Emit the frozen, explicitly opted-in C ABI into the consumer's build output.
/// The overlay marks existing catalog entries, never hooks absent from the catalog/fragments.
pub fn generate_abi(root: &Path, out_dir: &Path) -> Result<(), String> {
    super::build::track(root)?;
    let subset = subset()?;
    if subset.abi_version != 1 {
        return Err("this generator implements ABI version 1 only".into());
    }
    let mut hook_ids = HashSet::new();
    let mut service_ids = HashSet::new();
    let mut schema = selected_schema(root, &subset)?;
    schema.push_str(include_str!("../../horizon-mod-abi/src/lib.rs"));
    schema.push_str(include_str!("abi_header.h"));
    let mut schema_hash = 0xcbf29ce484222325;
    fingerprint(&mut schema_hash, schema.as_bytes());
    let build_fingerprint = engine_fingerprint(root)?;
    let target = std::env::var("TARGET").map_err(|error| error.to_string())?;
    let version = match std::env::var("HORIZON_VERSION").ok().filter(|version| !version.is_empty()) {
        Some(version) => version,
        None => package_version(&fs::read_to_string(root.join("crates/horizon-core/Cargo.toml"))
            .map_err(|error| error.to_string())?)?,
    };
    let nightly = std::env::var("HORIZON_NIGHTLY").unwrap_or_default();
    for name in ["HORIZON_VERSION", "HORIZON_NIGHTLY"] {
        println!("cargo:rerun-if-env-changed={name}");
    }
    let mut rust = format!("pub const ABI_VERSION: u32 = 1;\npub const SCHEMA_HASH: u64 = {schema_hash};\npub const BUILD_FINGERPRINT: u64 = {build_fingerprint};\npub const YOAKE_VERSION: &str = {version:?};\npub const YOAKE_NIGHTLY: &str = {nightly:?};\npub const TARGET: &str = {target:?};\n");
    let mut header = include_str!("abi_header.h").to_owned();
    header.push_str(&format!("\n#define YOAKE_ABI_VERSION 1u\n#define YOAKE_SCHEMA_HASH UINT64_C({schema_hash})\n#define YOAKE_BUILD_FINGERPRINT UINT64_C({build_fingerprint})\n#define YOAKE_VERSION {version:?}\n#define YOAKE_NIGHTLY {nightly:?}\n#define YOAKE_TARGET {target:?}\n"));
    let mut hooks = String::from("#[repr(C)]\n#[derive(Clone, Copy, Default)]\npub struct HookTableV1 { pub header: TableHeader,");
    let mut c_hooks = String::from("typedef struct YoakeHookTableV1 { YoakeTableHeader header;\n");
    let mut hook_info = Vec::new();
    for hook in subset.hooks.iter().filter(|hook| hook.abi) {
        if hook.id == 0 || !hook_ids.insert(hook.id) || hook.contract != "listen" {
            return Err("phase 1 requires unique nonzero hook ids and listen adapters".into());
        }
        let constant = format!("HOOK_{}", hook.slot.to_uppercase());
        rust.push_str(&format!("pub const {constant}: u32 = {};\n", hook.id));
        header.push_str(&format!("#define YOAKE_{constant} {}u\n", hook.id));
        hooks.push_str(&format!(r#"pub {}: Option<unsafe extern "C" fn(*mut core::ffi::c_void, Handle, *const {}) -> HookResult>,"#, hook.slot, hook.event));
        c_hooks.push_str(&format!("YoakeHookResult (*{})(void *, YoakeHandle, const Yoake{} *);\n", hook.slot, hook.event));
        hook_info.push(format!("({}, {:?}, Contract::Listen)", hook.id, hook.name));
    }
    hooks.push_str("}\n");
    c_hooks.push_str("} YoakeHookTableV1;\n");
    rust.push_str(&hooks);
    rust.push_str(&format!("pub const HOOKS: &[(u32, &str, Contract)] = &[{}];\n", hook_info.join(",")));
    let mut host = String::from("#[repr(C)]\n#[derive(Clone, Copy, Default)]\npub struct HostTableV1 { pub header: TableHeader, pub services: u64,");
    let mut c_host = String::from("typedef struct YoakeHostTableV1 { YoakeTableHeader header; uint64_t services;\n");
    let mut all_services = 0u64;
    for service in subset.services.iter().filter(|service| service.abi) {
        if service.id == 0 || service.id > 64 || !service_ids.insert(service.id) {
            return Err("phase 1 requires unique service ids in 1..=64".into());
        }
        let slot = service.slot.as_deref().unwrap_or(&service.name);
        let mask = 1u64 << (service.id - 1);
        all_services |= mask;
        rust.push_str(&format!("pub const SERVICE_{}: u64 = {mask};\n", slot.to_uppercase()));
        header.push_str(&format!("#define YOAKE_SERVICE_{} UINT64_C({mask})\n", slot.to_uppercase()));
        let (signature, c_signature) = service_signature(&service.kind)?;
        host.push_str(&format!(r#"pub {slot}: Option<unsafe extern "C" fn({signature}) -> Status>,"#));
        c_host.push_str(&format!("YoakeStatus (*{slot})({c_signature});\n"));
    }
    host.push_str("}\n");
    c_host.push_str("} YoakeHostTableV1;\n");
    rust.push_str(&host);
    rust.push_str(&format!("pub const ALL_SERVICES: u64 = {all_services};\n"));
    header.push_str(&c_hooks);
    header.push_str(&c_host);
    header.push_str("typedef struct YoakeModDescriptor { YoakeTableHeader header; uint64_t build_fingerprint; YoakeStrView yoake_version, nightly, target, id, version; uint64_t required_services; uint32_t side, flags; const YoakeHookTableV1 *hooks; YoakeStatus (*create)(const YoakeHostTableV1 *, YoakeHandle, void **); YoakeStatus (*destroy)(void *); } YoakeModDescriptor;\nconst YoakeModDescriptor *yoake_mod_entry(uint32_t abi_version, const YoakeHostTableV1 *host);\n#ifdef __cplusplus\n}\n#endif\n#endif\n");
    syn::parse_file(&rust).map_err(|error| error.to_string())?;
    fs::write(out_dir.join("abi_generated.rs"), rust).map_err(|error| error.to_string())?;
    fs::write(out_dir.join("yoake_mod_abi.h"), header).map_err(|error| error.to_string())?;
    Ok(())
}

/// Generate the static forwarding and opted-in DLL adapters for a Hooks trait.
pub fn generate_dll_adapters(source: &str) -> Result<String, String> {
    let parsed = syn::parse_file(source).map_err(|error| error.to_string())?;
    let hooks = parsed.items.iter().find_map(|item| match item {
        syn::Item::Trait(item) if item.ident == "Hooks" => Some(item),
        _ => None,
    }).ok_or("generated Hooks trait missing")?;
    let mut forwarding = String::from("impl crate::mod_api::Hooks for StaticMod {\n");
    for item in &hooks.items {
        let syn::TraitItem::Fn(method) = item else { continue };
        let arguments = method.sig.inputs.iter().filter_map(|argument| match argument {
            syn::FnArg::Typed(argument) => Some(match &*argument.pat {
                syn::Pat::Ident(name) => Ok(name.ident.to_string()),
                _ => Err("non-identifier Hooks argument"),
            }),
            _ => None,
        }).collect::<Result<Vec<_>, _>>()?;
        forwarding.push_str(&format!("{} {{ self.original[self.index].{}({}) }}\n", method.sig.to_token_stream(), method.sig.ident, arguments.join(",")));
    }
    forwarding.push_str("}\n");
    let adapters = include_str!("dll_adapter.rs.in");
    for hook in subset()?.hooks.iter().filter(|hook| hook.abi) {
        if !hooks.items.iter().any(|item| matches!(item, syn::TraitItem::Fn(method) if method.sig.ident == hook.name)) {
            return Err(format!("missing DLL adapter hook {}", hook.name));
        }
    }
    forwarding.push_str(adapters);
    Ok(forwarding)
}

#[cfg(test)]
mod tests {
    #[test]
    fn source_version_comes_from_core_not_generator_or_sdk() {
        let manifest = r#"[workspace.package]
version = "9.0.0"
[package]
version = "0.2.3"
"#;
        assert_eq!(super::package_version(manifest).unwrap(), "0.2.3");
        assert!(super::package_version("[package]\nversion.workspace = true\n").is_err());
    }

    #[test]
    fn subset_ids_are_frozen_not_catalog_positions() {
        let subset = super::subset().unwrap();
        assert_eq!(subset.hooks.iter().map(|hook| (hook.id, hook.slot.as_str())).collect::<Vec<_>>(), [(1, "client_tick"), (2, "key_input"), (3, "hud"), (4, "mod_net_receive")]);
        assert!(subset.hooks.iter().all(|hook| hook.abi && hook.contract == "listen"));
        assert!(subset.services.iter().all(|service| service.abi));
    }

    #[test]
    fn static_forwarder_preserves_return_and_arguments() {
        let source = "pub trait Hooks: Send + Sync { fn tick(&self, host: &mut dyn Host, event: &mut TickEvent) {} fn key(&self, event: &mut KeyEvent) {} fn hud(&self) {} fn net_from_server(&self) {} fn replace(&self, value: &str) -> Option<String> { None } }";
        let result = super::generate_dll_adapters(source).unwrap();
        syn::parse_file(&result).unwrap();
        assert!(result.contains("self.original[self.index].replace(value)"));
    }
}
