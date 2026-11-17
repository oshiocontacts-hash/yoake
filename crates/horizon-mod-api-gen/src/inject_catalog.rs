use std::{
    collections::HashSet,
    fs,
    path::{Path, PathBuf},
};

use serde::{Deserialize, Serialize};
use syn::{Attribute, Meta, visit::Visit};
use yoake_inject_spec::{Options, Rule, parse_options, site_policy};

#[derive(Deserialize, Serialize)]
struct CatalogSite {
    id: String,
    path: String,
    signature_hash: String,
    arguments: Vec<String>,
    result: String,
    side: String,
    allow: Vec<String>,
    source: String,
    argument_rules: Vec<serde_json::Value>,
    result_rule: serde_json::Value,
    required_capabilities: Vec<String>,
    client_required_capabilities: Vec<String>,
}

fn validation_rule(rule: Rule) -> serde_json::Value {
    match rule {
        Rule::Unrestricted => serde_json::json!({ "kind": "unrestricted" }),
        Rule::Finite => serde_json::json!({ "kind": "finite" }),
        Rule::Number(minimum, maximum) => {
            serde_json::json!({ "kind": "number", "minimum": minimum, "maximum": maximum })
        }
        Rule::ClampedNumber(minimum, maximum) => {
            serde_json::json!({ "kind": "clamped-number", "minimum": minimum, "maximum": maximum })
        }
        Rule::XpMerge => serde_json::json!({
            "kind": "xp-merge", "minimum": 1, "preserve_sum": true, "maximum_count": "vanilla-result"
        }),
        Rule::Subset => serde_json::json!({ "kind": "subset", "of": "vanilla-result" }),
        Rule::Vector(minimum, maximum) => {
            serde_json::json!({ "kind": "vector", "minimum": minimum, "maximum": maximum })
        }
        Rule::BlockState => serde_json::json!({ "kind": "block-state", "registry": "live" }),
    }
}

fn annotation(attributes: &[Attribute]) -> Result<Option<Options>, String> {
    let mut found = None;
    for attribute in attributes {
        if attribute.path().is_ident("cfg_attr")
            && quote::ToTokens::to_token_stream(attribute)
                .to_string()
                .contains("yoake_inject")
        {
            return Err("conditional yoake_inject attributes are not supported".into());
        }
        if attribute
            .path()
            .segments
            .last()
            .is_some_and(|segment| segment.ident == "yoake_inject")
        {
            if found.is_some() {
                return Err("use one yoake_inject attribute".into());
            }
            let tokens = match &attribute.meta {
                Meta::Path(_) => Default::default(),
                Meta::List(list) => list.tokens.clone(),
                Meta::NameValue(_) => return Err("invalid yoake_inject attribute".into()),
            };
            found = Some(parse_options(tokens).map_err(|error| error.to_string())?);
        }
    }
    Ok(found)
}

fn is_conditional(attributes: &[Attribute]) -> bool {
    attributes
        .iter()
        .any(|attribute| attribute.path().is_ident("cfg") || attribute.path().is_ident("cfg_attr"))
}

struct SourceScanner<'a> {
    source: &'a str,
    sites: Vec<CatalogSite>,
    errors: Vec<String>,
    conditional_depth: usize,
    function_depth: usize,
}

impl<'ast> Visit<'ast> for SourceScanner<'_> {
    fn visit_item_mod(&mut self, item: &'ast syn::ItemMod) {
        let conditional = usize::from(is_conditional(&item.attrs));
        self.conditional_depth += conditional;
        syn::visit::visit_item_mod(self, item);
        self.conditional_depth -= conditional;
    }

    fn visit_item_fn(&mut self, item: &'ast syn::ItemFn) {
        match annotation(&item.attrs) {
            Ok(Some(options)) => {
                let result = self.site(item, options);
                match result {
                    Ok(site) => self.sites.push(site),
                    Err(error) => self.errors.push(error),
                }
            }
            Ok(None) => {}
            Err(error) => self.errors.push(error),
        }
        self.function_depth += 1;
        syn::visit::visit_item_fn(self, item);
        self.function_depth -= 1;
    }

    fn visit_impl_item_fn(&mut self, item: &'ast syn::ImplItemFn) {
        match annotation(&item.attrs) {
            Ok(Some(options)) => {
                let function = syn::ItemFn {
                    attrs: item.attrs.clone(),
                    vis: item.vis.clone(),
                    sig: item.sig.clone(),
                    block: Box::new(item.block.clone()),
                };
                match self.site(&function, options) {
                    Ok(site) => self.sites.push(site),
                    Err(error) => self.errors.push(error),
                }
            }
            Ok(None) => {}
            Err(error) => self.errors.push(error),
        }
        self.function_depth += 1;
        syn::visit::visit_impl_item_fn(self, item);
        self.function_depth -= 1;
    }

    fn visit_trait_item_fn(&mut self, item: &'ast syn::TraitItemFn) {
        match annotation(&item.attrs) {
            Ok(Some(_)) => self
                .errors
                .push("injected trait methods are not supported".into()),
            Ok(None) => {}
            Err(error) => self.errors.push(error),
        }
        self.function_depth += 1;
        syn::visit::visit_trait_item_fn(self, item);
        self.function_depth -= 1;
    }
}

impl SourceScanner<'_> {
    fn site(&self, item: &syn::ItemFn, options: Options) -> Result<CatalogSite, String> {
        if self.conditional_depth != 0 || is_conditional(&item.attrs) {
            return Err("conditional sites are not supported by the source catalog".into());
        }
        if self.function_depth != 0 {
            return Err("nested injected functions are not supported by the source catalog".into());
        }
        let info = yoake_inject_spec::inspect_signature_with_options(&item.sig, &options)
            .map_err(|error| error.to_string())?;
        let path = options.id.ok_or("cataloged sites require an explicit id")?;
        let side = options
            .side
            .ok_or("cataloged sites require an explicit side")?;
        let signature_hash = info.hash();
        let mask = options.allowed.unwrap_or(7);
        let policy = site_policy(&path);
        let argument_rules = info
            .arguments
            .iter()
            .enumerate()
            .map(|(index, _)| {
                validation_rule(
                    policy
                        .arguments
                        .iter()
                        .find(|(argument, _)| *argument == index)
                        .map_or(Rule::Finite, |(_, rule)| *rule),
                )
            })
            .collect();
        let allow = [("head", 1), ("return", 2), ("overwrite", 4)]
            .into_iter()
            .filter_map(|(name, bit)| (mask & bit != 0).then_some(name.to_owned()))
            .collect();
        Ok(CatalogSite {
            id: format!("{path}#{signature_hash}"),
            path,
            signature_hash,
            arguments: info
                .arguments
                .iter()
                .map(|shape| shape.wire.clone())
                .collect(),
            result: info.result.wire,
            side,
            allow,
            source: self.source.to_owned(),
            argument_rules,
            result_rule: validation_rule(policy.result),
            required_capabilities: if policy.commands {
                vec!["service.commands".into()]
            } else {
                vec![]
            },
            client_required_capabilities: if policy.client_chat {
                vec!["service.chat".into()]
            } else {
                vec![]
            },
        })
    }
}

fn scan_source(source: &str, name: &str) -> Result<Vec<CatalogSite>, String> {
    let file = syn::parse_file(source).map_err(|error| format!("{name}: {error}"))?;
    let mut scanner = SourceScanner {
        source: name,
        sites: Vec::new(),
        errors: Vec::new(),
        conditional_depth: usize::from(is_conditional(&file.attrs)),
        function_depth: 0,
    };
    scanner.visit_file(&file);
    if !scanner.errors.is_empty() {
        return Err(format!("{name}: {}", scanner.errors.join("; ")));
    }
    Ok(scanner.sites)
}

fn collect_rust_files(directory: &Path, files: &mut Vec<PathBuf>) -> Result<(), String> {
    for entry in
        fs::read_dir(directory).map_err(|error| format!("{}: {error}", directory.display()))?
    {
        let entry = entry.map_err(|error| error.to_string())?;
        let kind = entry.file_type().map_err(|error| error.to_string())?;
        let path = entry.path();
        if kind.is_dir() {
            collect_rust_files(&path, files)?;
        } else if kind.is_file() && path.extension().is_some_and(|extension| extension == "rs") {
            files.push(path);
        }
    }
    Ok(())
}

/// Scan the gameplay source roots and render the deterministic, fail-closed draft catalog.
pub fn generate_inject_catalog(root: &Path) -> Result<String, String> {
    let mut files = [
        "game.rs",
        "crafting.rs",
        "furnace.rs",
        "motion.rs",
        "mobs/animal_brain.rs",
        "lan/prediction.rs",
    ]
    .into_iter()
    .map(|file| root.join("crates/horizon-client/src").join(file))
    .collect::<Vec<_>>();
    for directory in [
        "crates/horizon-world/src",
        "crates/horizon-items/src",
        "crates/horizon-client/src/game",
    ] {
        collect_rust_files(&root.join(directory), &mut files)?;
    }
    files.sort();
    let mut sites = Vec::new();
    for path in files {
        let name = path
            .strip_prefix(root)
            .map_err(|error| error.to_string())?
            .to_string_lossy()
            .replace(std::path::MAIN_SEPARATOR, "/");
        let source = fs::read_to_string(&path).map_err(|error| format!("{name}: {error}"))?;
        sites.extend(scan_source(&source, &name)?);
    }
    sites.sort_by(|left, right| left.path.cmp(&right.path));
    let mut paths = HashSet::new();
    for site in &sites {
        if !paths.insert(&site.path) {
            return Err(format!("duplicate injection path: {}", site.path));
        }
    }
    let mut json = serde_json::to_string_pretty(&sites).map_err(|error| error.to_string())?;
    json.push('\n');
    Ok(json)
}

pub fn generate_inject_registry(catalog: &str) -> Result<String, String> {
    let sites: Vec<CatalogSite> =
        serde_json::from_str(catalog).map_err(|error| error.to_string())?;
    let mut output = String::from(
        "use crate::{Side, Site, ValueType};\n\npub const KEYS: &[(&str, &str)] = &[\n",
    );
    for site in &sites {
        output.push_str(&format!(
            "    ({:?}, {:?}),\n",
            site.path, site.signature_hash
        ));
    }
    output.push_str(&format!(
        "];\n\npub static SITES: [Site; {}] = [\n",
        sites.len()
    ));
    for site in &sites {
        let arguments = site
            .arguments
            .iter()
            .map(|wire| registry_type(wire))
            .collect::<Result<Vec<_>, _>>()?
            .join(", ");
        let result = registry_type(&site.result)?;
        let side = match site.side.as_str() {
            "client" => "Client",
            "server" => "Server",
            "both" => "Both",
            _ => return Err("invalid catalog side".into()),
        };
        let mask: u8 = site
            .allow
            .iter()
            .map(|point| match point.as_str() {
                "head" => 1,
                "return" => 2,
                "overwrite" => 4,
                _ => 0,
            })
            .sum();
        output.push_str(&format!(
            "    Site::new({:?}, {:?}, Side::{side}, &[{arguments}], {result}, {mask}),\n",
            site.path, site.signature_hash
        ));
    }
    output.push_str("];\n");
    Ok(output)
}

fn registry_type(wire: &str) -> Result<String, String> {
    let variant = match wire {
        "unit" => "Unit",
        "bool" => "Bool",
        "s8" => "S8",
        "s16" => "S16",
        "s32" => "S32",
        "s64" => "S64",
        "u8" => "U8",
        "u16" | "block-id" => "U16",
        "u32" | "block-state-id" => "U32",
        "u64" => "U64",
        "usize" => "Usize",
        "isize" => "Ssize",
        "f32" => "F32",
        "f64" => "F64",
        "string" => "String",
        "list<u8>" => "Bytes",
        "list<u32>" => "U32List",
        "list<block-pos>" => "BlockPosList",
        "list<string>" => "StringList",
        "block-pos" => "BlockPos",
        "vec3-f32" => "Vec3F32",
        "vec3" => "Vec3",
        value if value.starts_with("scoped<") => "Handle",
        _ => return Err(format!("unknown catalog value type: {wire}")),
    };
    Ok(format!("ValueType::{variant}"))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn catalog_uses_the_shared_signature_contract() {
        let source = r#"#[yoake_inject(id = "demo::hit", side = "server", allow = "head,return")]
            fn hit(amount: i32, critical: bool) -> i32 { amount }"#;
        let sites = scan_source(source, "demo.rs").unwrap();
        assert_eq!(sites.len(), 1);
        assert_eq!(
            sites[0].id,
            "demo::hit#b4d3a485bf2cb6dda7bd066e50eb91ee5a7fdc8883530bbed800ef41e673daa9"
        );
        assert_eq!(sites[0].arguments, ["s32", "bool"]);
        assert_eq!(sites[0].allow, ["head", "return"]);
        assert_eq!(sites[0].side, "server");
    }

    #[test]
    fn catalog_refuses_sites_it_cannot_represent() {
        assert!(scan_source("#[yoake_inject] fn f() {}", "demo.rs").is_err());
        assert!(
            scan_source(
                r#"#[yoake_inject(id = "f", side = "server")] fn f(x: *const u8) {}"#,
                "demo.rs"
            )
            .is_err()
        );
        assert!(
            scan_source(
                r#"impl Game { #[yoake_inject(id = "f", side = "server")] fn f(&self) {} }"#,
                "demo.rs"
            )
            .is_ok()
        );
        assert!(
            scan_source(
                r#"#[cfg(test)] mod test { #[yoake_inject(id = "f", side = "server")] fn f() {} }"#,
                "demo.rs"
            )
            .is_err()
        );
    }

    #[test]
    fn every_site_has_explicit_numeric_and_registry_boundaries() {
        let root = Path::new(env!("CARGO_MANIFEST_DIR")).join("../..");
        let generated = generate_inject_catalog(&root).unwrap();
        let sites: Vec<CatalogSite> = serde_json::from_str(&generated).unwrap();
        assert_eq!(sites.len(), 56);
        let mut block_state_slots = 0;
        let mut sensitive = Vec::new();
        for site in sites {
            assert_eq!(site.arguments.len(), site.argument_rules.len());
            for (wire, rule) in site
                .arguments
                .iter()
                .zip(&site.argument_rules)
                .chain(std::iter::once((&site.result, &site.result_rule)))
            {
                let kind = rule["kind"].as_str().unwrap();
                match wire.as_str() {
                    "block-state-id" => {
                        assert_eq!(kind, "block-state", "{}", site.path);
                        assert_eq!(rule["registry"], "live");
                        block_state_slots += 1;
                    }
                    "block-pos" | "vec3" | "vec3-f32" => {
                        assert_eq!(kind, "vector", "{}", site.path)
                    }
                    "unit" | "bool" | "string" => assert_eq!(kind, "finite"),
                    "list<u32>" => assert_eq!(kind, "xp-merge"),
                    "list<block-pos>" | "list<string>" => {
                        assert!(matches!(kind, "finite" | "subset"), "{}", site.path);
                    }
                    wire if wire.starts_with("scoped<") => assert_eq!(kind, "finite"),
                    "f32" if site.path == "world::food::exhaustion_amount" => {
                        assert!(matches!(kind, "number" | "clamped-number"));
                    }
                    _ => assert_eq!(kind, "number", "{}: {wire}", site.path),
                }
            }
            if !site.required_capabilities.is_empty() {
                assert_eq!(site.required_capabilities, ["service.commands"]);
                sensitive.push(site.path.clone());
            }
            assert_eq!(
                site.client_required_capabilities == ["service.chat"],
                site.path == "game::chat::handle_chat_input"
            );
        }
        assert_eq!(block_state_slots, 5);
        assert_eq!(
            sensitive,
            [
                "game::chat::handle_chat_input",
                "game::commands::execute::execute_line",
                "game::commands::execute_command"
            ]
        );
    }

    #[test]
    fn committed_injection_catalog_is_current() {
        let root = Path::new(env!("CARGO_MANIFEST_DIR")).join("../..");
        let generated = generate_inject_catalog(&root).unwrap();
        let registry = generate_inject_registry(&generated).unwrap();
        if std::env::var_os("YOAKE_UPDATE_WIT").is_some() {
            fs::write(
                root.join("crates/yoake-wasm/wit/inject-sites.json"),
                &generated,
            )
            .unwrap();
            fs::write(
                root.join("crates/yoake-inject-rt/src/catalog.rs"),
                &registry,
            )
            .unwrap();
        }
        assert_eq!(
            generated,
            fs::read_to_string(root.join("crates/yoake-wasm/wit/inject-sites.json"))
                .unwrap()
                .replace("\r\n", "\n")
        );
        assert_eq!(
            registry,
            fs::read_to_string(root.join("crates/yoake-inject-rt/src/catalog.rs"))
                .unwrap()
                .replace("\r\n", "\n")
        );
    }
}
