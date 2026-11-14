use std::{collections::HashSet, fs, path::{Path, PathBuf}};

use serde::Serialize;
use syn::{visit::Visit, Attribute, Meta};
use yoake_inject_spec::{inspect_signature, parse_options, Options};

#[derive(Serialize)]
struct CatalogSite {
    id: String,
    path: String,
    signature_hash: String,
    arguments: Vec<&'static str>,
    result: &'static str,
    side: String,
    allow: Vec<&'static str>,
    source: String,
}

fn annotation(attributes: &[Attribute]) -> Result<Option<Options>, String> {
    let mut found = None;
    for attribute in attributes {
        if attribute.path().is_ident("cfg_attr")
            && quote::ToTokens::to_token_stream(attribute).to_string().contains("yoake_inject")
        {
            return Err("conditional yoake_inject attributes are not supported".into());
        }
        if attribute.path().segments.last().is_some_and(|segment| segment.ident == "yoake_inject") {
            if !attribute.path().is_ident("yoake_inject") || found.is_some() {
                return Err("use one unqualified yoake_inject attribute".into());
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
    attributes.iter().any(|attribute| attribute.path().is_ident("cfg") || attribute.path().is_ident("cfg_attr"))
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
            Ok(Some(_)) => self.errors.push("injected methods and associated functions are not supported".into()),
            Ok(None) => {}
            Err(error) => self.errors.push(error),
        }
        self.function_depth += 1;
        syn::visit::visit_impl_item_fn(self, item);
        self.function_depth -= 1;
    }

    fn visit_trait_item_fn(&mut self, item: &'ast syn::TraitItemFn) {
        match annotation(&item.attrs) {
            Ok(Some(_)) => self.errors.push("injected trait methods are not supported".into()),
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
        let path = options.id.ok_or("cataloged sites require an explicit id")?;
        let side = options.side.ok_or("cataloged sites require an explicit side")?;
        let info = inspect_signature(&item.sig).map_err(|error| error.to_string())?;
        let signature_hash = info.hash();
        let mask = options.allowed.unwrap_or(7);
        let allow = [("head", 1), ("return", 2), ("overwrite", 4)]
            .into_iter().filter_map(|(name, bit)| (mask & bit != 0).then_some(name)).collect();
        Ok(CatalogSite {
            id: format!("{path}#{signature_hash}"),
            path,
            signature_hash,
            arguments: info.arguments.iter().map(|shape| shape.wire).collect(),
            result: info.result.wire,
            side,
            allow,
            source: self.source.to_owned(),
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
    for entry in fs::read_dir(directory).map_err(|error| format!("{}: {error}", directory.display()))? {
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
    let mut files = vec![root.join("crates/horizon-client/src/game.rs")];
    for directory in ["crates/horizon-world/src", "crates/horizon-items/src", "crates/horizon-client/src/game"] {
        collect_rust_files(&root.join(directory), &mut files)?;
    }
    files.sort();
    let mut sites = Vec::new();
    for path in files {
        let name = path.strip_prefix(root).map_err(|error| error.to_string())?
            .to_string_lossy().replace(std::path::MAIN_SEPARATOR, "/");
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn catalog_uses_the_shared_signature_contract() {
        let source = r#"#[yoake_inject(id = "demo::hit", side = "server", allow = "head,return")]
            fn hit(amount: i32, critical: bool) -> i32 { amount }"#;
        let sites = scan_source(source, "demo.rs").unwrap();
        assert_eq!(sites.len(), 1);
        assert_eq!(sites[0].id, "demo::hit#b4d3a485bf2cb6dda7bd066e50eb91ee5a7fdc8883530bbed800ef41e673daa9");
        assert_eq!(sites[0].arguments, ["s32", "bool"]);
        assert_eq!(sites[0].allow, ["head", "return"]);
        assert_eq!(sites[0].side, "server");
    }

    #[test]
    fn catalog_refuses_sites_it_cannot_represent() {
        assert!(scan_source("#[yoake_inject] fn f() {}", "demo.rs").is_err());
        assert!(scan_source(r#"#[yoake_inject(id = "f", side = "server")] fn f(x: &str) {}"#, "demo.rs").is_err());
        assert!(scan_source(r#"impl Game { #[yoake_inject(id = "f", side = "server")] fn f(&self) {} }"#, "demo.rs").is_err());
        assert!(scan_source(r#"#[cfg(test)] mod test { #[yoake_inject(id = "f", side = "server")] fn f() {} }"#, "demo.rs").is_err());
    }

    #[test]
    fn committed_injection_catalog_is_current() {
        let root = Path::new(env!("CARGO_MANIFEST_DIR")).join("../..");
        let generated = generate_inject_catalog(&root).unwrap();
        assert_eq!(generated, include_str!("../../yoake-wasm/wit/inject-sites.json"));
    }
}
