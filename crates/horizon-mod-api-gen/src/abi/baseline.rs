use super::{generate_full_wit, generate_wit};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use std::{collections::{BTreeMap, BTreeSet, HashMap}, fs, path::{Path, PathBuf}};
use wit_parser::{Function, FunctionKind, Handle, InterfaceId, Resolve, Type, TypeDefKind, TypeId, TypeOwner, WorldId, WorldItem, WorldKey};

const HEADER: &str = "// Yoake 1.0 ABI baseline. Refresh deliberately BEFORE release only.\n// Comment-enveloped WIT: kept inert when bindings parse this directory.\n// Decode by removing this header and the leading `// ` from every remaining line.\n";
const PACKAGE: &str = "package yoake:mods@1.0.0;";

struct Abi<'a> {
    resolve: &'a Resolve,
    interfaces: HashMap<InterfaceId, String>,
}

impl<'a> Abi<'a> {
    fn new(resolve: &'a Resolve) -> Self {
        let mut abi = Self { resolve, interfaces: HashMap::new() };
        for (package_id, package) in &resolve.packages {
            for (name, interface_id) in &package.interfaces {
                abi.interfaces.insert(*interface_id, format!("{}/interface/{name}", resolve.packages[package_id].name));
            }
        }
        for (world_id, world) in &resolve.worlds {
            for (direction, items) in [("import", &world.imports), ("export", &world.exports)] {
                for (key, item) in items {
                    if let (WorldKey::Name(name), WorldItem::Interface { id, .. }) = (key, item) {
                        let name = format!("{}/{direction}/{name}", abi.world_name(world_id));
                        abi.interfaces.entry(*id).or_insert(name);
                    }
                }
            }
        }
        abi
    }

    fn world_name(&self, world_id: WorldId) -> String {
        let world = &self.resolve.worlds[world_id];
        format!("{}/world/{}", self.resolve.packages[world.package.unwrap()].name, world.name)
    }

    fn type_name(&self, type_id: TypeId) -> Option<String> {
        let ty = &self.resolve.types[type_id];
        let name = ty.name.as_ref()?;
        let owner = match ty.owner {
            TypeOwner::Interface(id) => self.interfaces[&id].clone(),
            TypeOwner::World(id) => self.world_name(id),
            TypeOwner::None => return None,
        };
        Some(format!("{owner}/type/{name}"))
    }

    fn ty(&self, ty: &Type) -> Value {
        match ty {
            Type::Id(id) => match self.type_name(*id) {
                Some(name) => json!({"ref": name}),
                None => self.definition(*id),
            },
            primitive => json!(format!("{primitive:?}")),
        }
    }

    fn definition(&self, type_id: TypeId) -> Value {
        let definition = &self.resolve.types[type_id];
        let shape = match &definition.kind {
            TypeDefKind::Record(record) => json!({"record": record.fields.iter().map(|field| json!([field.name, self.ty(&field.ty)])).collect::<Vec<_>>()}),
            TypeDefKind::Resource => json!("resource"),
            TypeDefKind::Handle(Handle::Own(id)) => json!({"own": self.ty(&Type::Id(*id))}),
            TypeDefKind::Handle(Handle::Borrow(id)) => json!({"borrow": self.ty(&Type::Id(*id))}),
            TypeDefKind::Flags(flags) => json!({"flags": flags.flags.iter().map(|flag| &flag.name).collect::<Vec<_>>()}),
            TypeDefKind::Tuple(tuple) => json!({"tuple": tuple.types.iter().map(|ty| self.ty(ty)).collect::<Vec<_>>()}),
            TypeDefKind::Variant(variant) => json!({"variant": variant.cases.iter().map(|case| json!([case.name, case.ty.as_ref().map(|ty| self.ty(ty))])).collect::<Vec<_>>()}),
            TypeDefKind::Enum(enumeration) => json!({"enum": enumeration.cases.iter().map(|case| &case.name).collect::<Vec<_>>()}),
            TypeDefKind::Option(ty) => json!({"option": self.ty(ty)}),
            TypeDefKind::Result(result) => json!({"result": [result.ok.as_ref().map(|ty| self.ty(ty)), result.err.as_ref().map(|ty| self.ty(ty))]}),
            TypeDefKind::List(ty) => json!({"list": self.ty(ty)}),
            TypeDefKind::Map(key, value) => json!({"map": [self.ty(key), self.ty(value)]}),
            TypeDefKind::FixedLengthList(ty, length) => json!({"fixed-list": [self.ty(ty), json!(length)]}),
            TypeDefKind::Future(ty) => json!({"future": ty.as_ref().map(|ty| self.ty(ty))}),
            TypeDefKind::Stream(ty) => json!({"stream": ty.as_ref().map(|ty| self.ty(ty))}),
            TypeDefKind::Type(ty) => json!({"alias": self.ty(ty)}),
            TypeDefKind::Unknown => panic!("unresolved ABI type"),
        };
        json!({"shape": shape, "external-id": definition.external_id})
    }

    fn function(&self, function: &Function) -> Value {
        let kind = match function.kind {
            FunctionKind::Freestanding => json!("freestanding"),
            FunctionKind::AsyncFreestanding => json!("async-freestanding"),
            FunctionKind::Method(id) => json!({"method": self.ty(&Type::Id(id))}),
            FunctionKind::AsyncMethod(id) => json!({"async-method": self.ty(&Type::Id(id))}),
            FunctionKind::Static(id) => json!({"static": self.ty(&Type::Id(id))}),
            FunctionKind::AsyncStatic(id) => json!({"async-static": self.ty(&Type::Id(id))}),
            FunctionKind::Constructor(id) => json!({"constructor": self.ty(&Type::Id(id))}),
        };
        json!({"kind": kind, "params": function.params.iter().map(|param| json!([param.name, self.ty(&param.ty)])).collect::<Vec<_>>(), "result": function.result.as_ref().map(|ty| self.ty(ty)), "external-id": function.external_id})
    }

    fn items(&self) -> BTreeMap<String, Value> {
        let mut items = BTreeMap::new();
        for (_, package) in &self.resolve.packages {
            items.insert(package.name.to_string(), Value::Null);
        }
        for (interface_id, interface) in &self.resolve.interfaces {
            let owner = &self.interfaces[&interface_id];
            items.insert(owner.clone(), Value::Null);
            for (name, type_id) in &interface.types {
                items.insert(format!("{owner}/type/{name}"), self.definition(*type_id));
            }
            for (name, function) in &interface.functions {
                items.insert(format!("{owner}/function/{name}"), self.function(function));
            }
        }
        for (world_id, world) in &self.resolve.worlds {
            let owner = self.world_name(world_id);
            items.insert(owner.clone(), Value::Null);
            for (direction, bindings) in [("import", &world.imports), ("export", &world.exports)] {
                for (key, item) in bindings {
                    let name = match key {
                        WorldKey::Name(name) => name.clone(),
                        WorldKey::Interface(id) => self.interfaces[id].clone(),
                    };
                    let value = match item {
                        WorldItem::Interface { id, external_id, .. } => json!({"interface": self.interfaces[id], "external-id": external_id}),
                        WorldItem::Function(function) => self.function(function),
                        WorldItem::Type { id, .. } => self.definition(*id),
                    };
                    items.insert(format!("{owner}/{direction}/{name}"), value);
                }
            }
        }
        items
    }
}

fn wit_items(source: &str) -> BTreeMap<String, Value> {
    let mut resolve = Resolve::default();
    resolve.push_str("abi.wit", source).expect("ABI WIT must parse");
    Abi::new(&resolve).items()
}

fn changes<T: PartialEq>(baseline: &BTreeMap<String, T>, current: &BTreeMap<String, T>) -> Vec<String> {
    baseline.iter().filter(|(name, value)| current.get(*name) != Some(*value)).map(|(name, _)| name.clone()).collect()
}

#[derive(Clone, Debug, Deserialize, PartialEq, Serialize)]
struct IdEntry {
    id: u64,
    capability: Option<String>,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
struct IdTables {
    services: BTreeMap<String, IdEntry>,
    hooks: BTreeMap<String, IdEntry>,
    capabilities: BTreeSet<String>,
    manifest_permissions: BTreeSet<String>,
}

fn id_changes(baseline: &IdTables, current: &IdTables) -> Vec<String> {
    let mut errors = Vec::new();
    for (kind, before, after) in [("service", &baseline.services, &current.services), ("hook", &baseline.hooks, &current.hooks)] {
        errors.extend(changes(before, after).into_iter().map(|name| format!("{kind} {name}")));
        let mut ids = BTreeSet::new();
        for (name, entry) in after {
            if !ids.insert(entry.id) {
                errors.push(format!("duplicate {kind} id {} ({name})", entry.id));
            }
        }
    }
    errors.extend(baseline.capabilities.difference(&current.capabilities).map(|name| format!("capability {name}")));
    errors.extend(baseline.manifest_permissions.difference(&current.manifest_permissions).map(|name| format!("manifest permission {name}")));
    errors
}

fn root() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("../..")
}

fn current_wit(root: &Path) -> String {
    let mut files = fs::read_dir(root.join("crates/yoake-wasm/wit")).unwrap().map(|entry| entry.unwrap().path()).filter(|path| path.extension().is_some_and(|extension| extension == "wit") && path.file_name().unwrap() != "abi-1.0-baseline.wit").collect::<Vec<_>>();
    files.sort();
    let mut source = format!("{PACKAGE}\n");
    for file in files {
        let content = match file.file_name().unwrap().to_str().unwrap() {
            "yoake-mod.wit" => generate_wit().unwrap(),
            "yoake-full.wit" => generate_full_wit(root).unwrap().wit,
            _ => fs::read_to_string(file).unwrap(),
        };
        for line in content.lines() {
            if line.starts_with("package ") {
                assert_eq!(line, PACKAGE);
            } else {
                source.push_str(line);
                source.push('\n');
            }
        }
    }
    source
}

fn current_ids(root: &Path) -> IdTables {
    let mut tables = IdTables { services: BTreeMap::new(), hooks: BTreeMap::new(), capabilities: BTreeSet::new(), manifest_permissions: BTreeSet::new() };
    for catalog in ["abi-subset.json", "abi-subset-server.json", "abi-subset-client.json", "abi-subset-gpu.json"] {
        let catalog: Value = serde_json::from_str(&fs::read_to_string(root.join("crates/horizon-mod-api-gen").join(catalog)).unwrap()).unwrap();
        for (kind, table) in [("services", &mut tables.services), ("hooks", &mut tables.hooks)] {
            for entry in catalog[kind].as_array().unwrap().iter().filter(|entry| entry["abi"] == true) {
                let name = entry["name"].as_str().unwrap().to_owned();
                let entry = IdEntry { id: entry["id"].as_u64().unwrap(), capability: entry["capability"].as_str().map(str::to_owned) };
                if let Some(capability) = &entry.capability {
                    tables.capabilities.insert(capability.clone());
                }
                assert!(table.insert(name.clone(), entry).is_none(), "duplicate ABI {kind} name {name}");
            }
        }
    }
    let manifest = fs::read_to_string(root.join("crates/horizon-mods/src/yak/manifest.rs")).unwrap();
    let manifest = syn::parse_file(&manifest).unwrap();
    let permissions = manifest.items.iter().find_map(|item| match item {
        syn::Item::Const(item) if item.ident == "WASM_PERMISSIONS" => Some(&*item.expr),
        _ => None,
    }).expect("manifest declares WASM_PERMISSIONS");
    let syn::Expr::Array(permissions) = permissions else { panic!("WASM_PERMISSIONS must be an array") };
    for permission in &permissions.elems {
        let syn::Expr::Lit(syn::ExprLit { lit: syn::Lit::Str(permission), .. }) = permission else { panic!("capability must be a string") };
        tables.capabilities.insert(permission.value());
        tables.manifest_permissions.insert(permission.value());
    }
    tables
}

#[test]
fn abi_1_0_is_additive() {
    let root = root();
    let directory = root.join("crates/yoake-wasm/wit");
    let current = current_wit(&root);
    let ids = current_ids(&root);
    let baseline_wit = directory.join("abi-1.0-baseline.wit");
    let baseline_ids = directory.join("abi-1.0-ids.json");
    if std::env::var("YOAKE_UPDATE_ABI_BASELINE").as_deref() == Ok("1") {
        let snapshot = format!("{HEADER}{}", current.lines().map(|line| format!("// {line}\n")).collect::<String>());
        fs::write(&baseline_wit, snapshot).unwrap();
        fs::write(&baseline_ids, format!("{}\n", serde_json::to_string_pretty(&ids).unwrap())).unwrap();
    }
    let snapshot = fs::read_to_string(baseline_wit).expect("committed 1.0 WIT baseline").replace("\r\n", "\n");
    let snapshot = snapshot.strip_prefix(HEADER).unwrap().lines().map(|line| format!("{}\n", line.strip_prefix("// ").expect("comment-enveloped baseline"))).collect::<String>();
    let errors = changes(&wit_items(&snapshot), &wit_items(&current));
    assert!(errors.is_empty(), "breaking 1.0 WIT changes: {errors:?}");
    let baseline: IdTables = serde_json::from_str(&fs::read_to_string(baseline_ids).expect("committed 1.0 id baseline")).unwrap();
    let errors = id_changes(&baseline, &ids);
    assert!(errors.is_empty(), "breaking 1.0 id/capability changes: {errors:?}");
}

const FIXTURE: &str = "package test:abi@1.0.0;
interface api {
    record view { title: string, visible: bool }
    enum mode { normal, fullscreen }
    variant event { click(u32), close }
    flags access { read, write }
    type views = list<view>;
    read: func(index: u32, mode: mode) -> option<view>;
}
world guest { import api; export run: func(); }
";

#[test]
fn existing_wit_items_are_immutable() {
    let before = wit_items(FIXTURE);
    for (old, new) in [
        ("title: string, visible: bool", "title: string, visible: bool, kind: string"),
        ("title: string, visible: bool", "title: string"),
        ("title: string, visible: bool", "visible: bool, title: string"),
        ("title: string", "title: u32"),
        ("visible: bool", "shown: bool"),
        ("normal, fullscreen", "normal, fullscreen, borderless"),
        ("normal, fullscreen", "fullscreen, normal"),
        ("normal, fullscreen", "normal"),
        ("click(u32), close", "click(u64), close"),
        ("click(u32), close", "click(u32), close, move"),
        ("click(u32), close", "close, click(u32)"),
        ("click(u32), close", "close"),
        ("read, write", "read, write, execute"),
        ("list<view>", "option<view>"),
        ("index: u32, mode: mode", "mode: mode, index: u32"),
        ("index: u32", "index: u64"),
        ("index: u32", "offset: u32"),
        ("index: u32, mode: mode", "index: u32"),
        ("-> option<view>", "-> view"),
        ("read: func(index: u32, mode: mode) -> option<view>;", ""),
        ("export run: func();", "export run: func() -> u32;"),
        ("import api;", "export api;"),
    ] {
        assert!(!changes(&before, &wit_items(&FIXTURE.replace(old, new))).is_empty(), "accepted breaking change: {old} => {new}");
    }
}

#[test]
fn independent_wit_additions_and_formatting_are_allowed() {
    let current = FIXTURE.replace("record view", "/// Documentation only.\nrecord view").replace("interface api {", "interface api {\nrecord added { count: u64 }\nadded-call: func();");
    let current = format!("{current}\ninterface extra {{ ping: func(); }}\nworld another {{ import extra; }}\n");
    assert!(changes(&wit_items(FIXTURE), &wit_items(&current)).is_empty());
    let inline = "package test:inline@1.0.0; world guest { import host: interface { run: func(); } }";
    assert!(changes(&wit_items(inline), &wit_items(&inline.replace("run: func();", "run: func(); added: func();"))).is_empty());
}

#[test]
fn ids_and_capability_names_are_immutable() {
    let before = current_ids(&root());
    for kind in ["service", "hook", "capability", "manifest-permission", "service-capability", "duplicate-service", "duplicate-hook", "removed-service", "removed-hook"] {
        let mut current = before.clone();
        match kind {
            "service" => current.services.get_mut("log").unwrap().id += 1,
            "hook" => current.hooks.get_mut("tick").unwrap().id += 1,
            "capability" => { current.capabilities.remove("service.chat"); }
            "manifest-permission" => { current.manifest_permissions.remove("service.chat"); }
            "service-capability" => current.services.get_mut("buffer").unwrap().capability = Some("other".into()),
            "duplicate-service" => { current.services.insert("added".into(), current.services["log"].clone()); }
            "duplicate-hook" => { current.hooks.insert("added".into(), current.hooks["tick"].clone()); }
            "removed-service" => { current.services.remove("log"); }
            "removed-hook" => { current.hooks.remove("tick"); }
            _ => unreachable!(),
        }
        assert!(!id_changes(&before, &current).is_empty(), "accepted breaking {kind}");
    }
}

#[test]
fn reserved_w2dn_ids_are_additions() {
    let before: IdTables = serde_json::from_str(&fs::read_to_string(root().join("crates/yoake-wasm/wit/abi-1.0-ids.json")).unwrap()).unwrap();
    let mut current = before.clone();
    for id in 127..=145 {
        current.services.insert(format!("w2dn-service-{id}"), IdEntry { id, capability: None });
    }
    for id in 43..=44 {
        current.hooks.insert(format!("w2dn-hook-{id}"), IdEntry { id, capability: None });
    }
    current.capabilities.insert("service.added".into());
    current.manifest_permissions.insert("service.added".into());
    assert!(id_changes(&before, &current).is_empty());
}
