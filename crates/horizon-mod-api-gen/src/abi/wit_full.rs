//! The full-catalog WIT (`yoake-full.wit`): every native hook, side-aware, typed from the Rust
//! hook signatures. The frozen 19-hook `client-mod` world is generated elsewhere (`wit.rs`).
//!
//! Side rule (see also the header of the generated file):
//! 1. an explicit catalog `side` (`client`, `server`, `both`) wins;
//! 2. otherwise the Rust target path decides: `horizon_client_render::` and `horizon_client_lib::`
//!    are client code, `horizon_world::` and `horizon_server*::` are server code, and a hook on
//!    `GameLevel` (level simulation) is server;
//! 3. the remaining `Game` methods in `horizon_client::game::native_hooks` run on the client and
//!    in the integrated server alike, so they are undecidable and appear in both interfaces.
//!
//! Type rule: primitives, strings, lists, options, tuples and results map directly; plain structs
//! and enums found in the workspace sources become records, enums and variants (recursively,
//! deduplicated by name). Anything else (live engine objects, trait objects, generics,
//! encapsulated structs, recursion) becomes a `u64` opaque handle, except the engine types of the
//! table in `types.rs` (NBT, item stacks, text, JSON, mod block entities, `&mut` draw objects).
use serde::Serialize;
use std::{
    collections::{BTreeMap, BTreeSet, HashMap, HashSet},
    fs,
    path::{Path, PathBuf},
};
use std::rc::Rc;
use syn::{FnArg, GenericArgument, Item, Pat, PathArguments, Type};

mod types;
use types::{Carrier, Effect, ITEM_STACK_DOC, JSON_DOC, NBT_DOC, TABLE, TEXT_DOC, effect_of};

const SERVER_DRAFT: &str = include_str!("../../abi-subset-server.json");

/// Existing service interfaces the client world imports (they live in `yoake-mod.wit`).
const CLIENT_IMPORTS: &[&str] = &[
    "types",
    "logging",
    "feedback",
    "hud",
    "network",
    "settings",
    "client-world",
    "player",
    "instance",
    "accounts",
    "screen",
];
/// Existing service interfaces the server world imports.
const SERVER_IMPORTS: &[&str] = &["types", "logging"];

const KEYWORDS: &[&str] = &[
    "as", "async", "bool", "borrow", "char", "constructor", "enum", "export", "f32", "f64",
    "flags", "from", "func", "future", "import", "include", "interface", "list", "option", "own",
    "package", "record", "resource", "result", "s8", "s16", "s32", "s64", "static", "stream",
    "string", "tuple", "type", "u8", "u16", "u32", "u64", "use", "variant", "with", "world",
    "error-context", "u128", "s128", "map",
];

const MAX_NAMED_FIELDS: usize = 40;

// ---------------------------------------------------------------------------------------------
// Identifiers

/// WIT kebab-case: words never start with a digit; keywords are escaped with `%`.
fn kebab(name: &str) -> String {
    let name = name.strip_prefix("r#").unwrap_or(name);
    let chars: Vec<char> = name.chars().collect();
    let mut words: Vec<String> = Vec::new();
    let mut current = String::new();
    for (index, &c) in chars.iter().enumerate() {
        if !c.is_ascii_alphanumeric() {
            if !current.is_empty() {
                words.push(std::mem::take(&mut current));
            }
            continue;
        }
        if c.is_ascii_uppercase() && !current.is_empty() {
            let previous = chars[index - 1];
            let next = chars.get(index + 1).copied();
            let boundary = previous.is_ascii_lowercase()
                || previous.is_ascii_digit()
                || (previous.is_ascii_uppercase() && next.is_some_and(|n| n.is_ascii_lowercase()));
            if boundary {
                words.push(std::mem::take(&mut current));
            }
        }
        current.push(c.to_ascii_lowercase());
    }
    if !current.is_empty() {
        words.push(current);
    }
    let mut merged: Vec<String> = Vec::new();
    for word in words {
        if word.starts_with(|c: char| c.is_ascii_digit()) {
            match merged.last_mut() {
                Some(last) => last.push_str(&word),
                None => merged.push(format!("n{word}")),
            }
        } else {
            merged.push(word);
        }
    }
    if merged.is_empty() {
        merged.push("x".into());
    }
    merged.join("-")
}

fn escape(name: &str) -> String {
    if KEYWORDS.contains(&name) {
        format!("%{name}")
    } else {
        name.to_owned()
    }
}

fn ident(name: &str) -> String {
    escape(&kebab(name))
}

/// `base` made unique inside `used` (WIT words cannot start with a digit, so the suffix is a word).
fn unique(base: String, used: &mut HashSet<String>) -> String {
    let mut name = base.clone();
    let mut n = 2;
    while !used.insert(name.clone()) {
        name = format!("{base}-n{n}");
        n += 1;
    }
    name
}

fn sanitize_doc(text: &str) -> String {
    text.chars()
        .map(|c| if c.is_control() && c != '\n' && c != '\t' { ' ' } else { c })
        .filter(|c| !matches!(*c, '\u{202a}'..='\u{202e}' | '\u{2066}'..='\u{2069}'))
        .collect()
}

fn doc_lines(text: &str, indent: &str) -> String {
    text.lines()
        .map(|line| format!("{indent}///{}\n", sanitize_doc(line).trim_end()))
        .collect()
}

// ---------------------------------------------------------------------------------------------
// Source index of plain types

#[derive(Clone)]
enum Def {
    Struct(syn::ItemStruct),
    Enum(syn::ItemEnum),
    Alias(syn::ItemType),
}

impl Def {
    fn fingerprint(&self) -> String {
        use quote::ToTokens;
        match self {
            Def::Struct(item) => item.to_token_stream().to_string(),
            Def::Enum(item) => item.to_token_stream().to_string(),
            Def::Alias(item) => item.to_token_stream().to_string(),
        }
    }

    fn name(&self) -> String {
        match self {
            Def::Struct(item) => item.ident.to_string(),
            Def::Enum(item) => item.ident.to_string(),
            Def::Alias(item) => item.ident.to_string(),
        }
    }
}

/// What a source file contributes: its type definitions and the names it imports.
#[derive(Default)]
struct ParsedFile {
    defs: Vec<Def>,
    /// Local name -> full path of the `use` that introduced it.
    imports: HashMap<String, Vec<String>>,
}

/// Where a name is looked up from: a source file and its crate.
#[derive(Clone)]
struct Context {
    file: PathBuf,
    krate: String,
    parsed: Rc<ParsedFile>,
}

fn is_test_item(attrs: &[syn::Attribute]) -> bool {
    use quote::ToTokens;
    attrs.iter().any(|attr| {
        attr.path().is_ident("cfg") && attr.to_token_stream().to_string().contains("test")
    })
}

fn flatten_use(tree: &syn::UseTree, prefix: &mut Vec<String>, out: &mut HashMap<String, Vec<String>>) {
    match tree {
        syn::UseTree::Path(path) => {
            prefix.push(path.ident.to_string());
            flatten_use(&path.tree, prefix, out);
            prefix.pop();
        }
        syn::UseTree::Name(name) => {
            let mut full = prefix.clone();
            full.push(name.ident.to_string());
            out.entry(name.ident.to_string()).or_insert(full);
        }
        syn::UseTree::Rename(rename) => {
            let mut full = prefix.clone();
            full.push(rename.ident.to_string());
            out.entry(rename.rename.to_string()).or_insert(full);
        }
        syn::UseTree::Group(group) => {
            for item in &group.items {
                flatten_use(item, prefix, out);
            }
        }
        syn::UseTree::Glob(_) => {}
    }
}

fn collect_items(items: Vec<Item>, parsed: &mut ParsedFile) {
    for item in items {
        match item {
            Item::Struct(item) if !is_test_item(&item.attrs) => parsed.defs.push(Def::Struct(item)),
            Item::Enum(item) if !is_test_item(&item.attrs) => parsed.defs.push(Def::Enum(item)),
            Item::Type(item) if !is_test_item(&item.attrs) => parsed.defs.push(Def::Alias(item)),
            Item::Use(item) if !is_test_item(&item.attrs) => {
                flatten_use(&item.tree, &mut Vec::new(), &mut parsed.imports);
            }
            Item::Mod(item) if !is_test_item(&item.attrs) => {
                if let Some((_, items)) = item.content {
                    collect_items(items, parsed);
                }
            }
            _ => {}
        }
    }
}

fn parse_file_context(path: &Path) -> Rc<ParsedFile> {
    // An unreadable or unparsable file only shrinks the index; its types become handles.
    let mut parsed = ParsedFile::default();
    if let Ok(source) = fs::read_to_string(path)
        && let Ok(file) = syn::parse_file(&source)
    {
        collect_items(file.items, &mut parsed);
    }
    Rc::new(parsed)
}

fn identifier_after(text: &str) -> String {
    text.chars().take_while(|c| c.is_alphanumeric() || *c == '_').collect()
}

/// Workspace type definitions, found by a cheap text scan and parsed only when a hook needs them.
#[derive(Default)]
struct TypeIndex {
    /// Type name -> files that appear to define it.
    files: BTreeMap<String, Vec<PathBuf>>,
    /// File -> (crate directory name, module path below `src`).
    origins: HashMap<PathBuf, (String, String)>,
    /// `Original as Local` imports seen anywhere: local -> (original, file of the import).
    aliases: HashMap<String, (String, PathBuf)>,
    parsed: HashMap<PathBuf, Rc<ParsedFile>>,
    crates_dir: PathBuf,
}

type Candidate = (PathBuf, Def);

fn single(found: &[Candidate]) -> bool {
    let first = found[0].1.fingerprint();
    found.iter().all(|(_, def)| def.fingerprint() == first)
}

impl TypeIndex {
    fn load(root: &Path) -> Result<Self, String> {
        let mut index = Self::default();
        let crates = root.join("crates");
        index.crates_dir = crates.clone();
        let mut dirs: Vec<_> = fs::read_dir(&crates)
            .map_err(|e| format!("{}: {e}", crates.display()))?
            .filter_map(|entry| entry.ok().map(|entry| entry.path()))
            .filter(|path| path.join("src").is_dir())
            .collect();
        dirs.sort();
        for dir in dirs {
            let krate = dir.file_name().and_then(|n| n.to_str()).unwrap_or_default().to_owned();
            index.walk(&dir.join("src"), &dir.join("src"), &krate);
        }
        Ok(index)
    }

    fn walk(&mut self, dir: &Path, src: &Path, krate: &str) {
        let Ok(entries) = fs::read_dir(dir) else { return };
        let mut paths: Vec<_> = entries.filter_map(|entry| entry.ok().map(|e| e.path())).collect();
        paths.sort();
        for path in paths {
            if path.is_dir() {
                self.walk(&path, src, krate);
            } else if path.extension().is_some_and(|ext| ext == "rs") {
                let Ok(source) = fs::read_to_string(&path) else { continue };
                let relative = path.strip_prefix(src).unwrap_or(&path).with_extension("");
                let mut module: Vec<String> =
                    relative.components().map(|c| c.as_os_str().to_string_lossy().into_owned()).collect();
                if matches!(module.last().map(String::as_str), Some("mod" | "lib" | "main")) {
                    module.pop();
                }
                self.origins.insert(path.clone(), (krate.to_owned(), module.join("::")));
                for line in source.lines() {
                    let mut rest = line.trim_start();
                    if let Some(after) = rest.strip_prefix("pub") {
                        rest = after.trim_start();
                        if rest.starts_with('(') {
                            rest = rest.split_once(')').map_or("", |(_, tail)| tail).trim_start();
                        }
                    }
                    if let Some(rest) = ["struct ", "enum ", "type "].iter().find_map(|k| rest.strip_prefix(k)) {
                        let name = identifier_after(rest);
                        if !name.is_empty() {
                            let files = self.files.entry(name).or_default();
                            if !files.contains(&path) {
                                files.push(path.clone());
                            }
                        }
                    }
                    // `Original as Local` inside a `use`, never a cast or an expression.
                    if line.contains(" as ")
                        && !line.contains(['(', '=', '.', '<'])
                        && let Some((left, right)) = line.split_once(" as ")
                    {
                        let reversed: String = left
                            .chars()
                            .rev()
                            .take_while(|c| c.is_alphanumeric() || *c == '_')
                            .collect();
                        let original: String = reversed.chars().rev().collect();
                        let local = identifier_after(right.trim_start());
                        if !original.is_empty() && !local.is_empty() {
                            self.aliases.entry(local).or_insert((original, path.clone()));
                        }
                    }
                }
            }
        }
    }

    fn parsed(&mut self, path: &Path) -> Rc<ParsedFile> {
        if let Some(parsed) = self.parsed.get(path) {
            return parsed.clone();
        }
        let parsed = parse_file_context(path);
        self.parsed.insert(path.to_owned(), parsed.clone());
        parsed
    }

    fn context(&mut self, path: &Path) -> Context {
        let krate = self.origins.get(path).map(|(krate, _)| krate.clone()).unwrap_or_default();
        Context { file: path.to_owned(), krate, parsed: self.parsed(path) }
    }

    /// The workspace crates `krate` depends on, read from its manifest.
    fn dependencies(&self, krate: &str) -> HashSet<String> {
        let manifest = fs::read_to_string(self.crates_dir.join(krate).join("Cargo.toml")).unwrap_or_default();
        manifest
            .lines()
            .filter_map(|line| line.split(['=', '.', ' ']).next())
            .filter(|key| key.starts_with("horizon-") || key.starts_with("yoake-"))
            .map(str::to_owned)
            .collect()
    }

    fn crate_of(&self, file: &Path) -> Option<&str> {
        self.origins.get(file).map(|(krate, _)| krate.as_str())
    }

    /// The definition `name` means when written in `from`: the file's own definition, else the one
    /// its imports point at, else the one in its crate. Identical copies count as one.
    fn resolve(&mut self, name: &str, from: &Context) -> Result<Candidate, &'static str> {
        let mut name = name.to_owned();
        let mut hint = from.parsed.imports.get(&name).cloned();
        if !self.files.contains_key(&name) {
            if let Some(last) = hint.as_ref().and_then(|path| path.last()).filter(|last| self.files.contains_key(*last)) {
                name = last.clone();
            } else if let Some((original, file)) = self.aliases.get(&name).cloned() {
                hint = self.parsed(&file).imports.get(&name).cloned();
                name = original;
            }
        } else if let Some(last) = hint.as_ref().and_then(|path| path.last()).filter(|last| **last != name && self.files.contains_key(*last)) {
            name = last.clone();
        }
        let files = self.files.get(&name).ok_or("not found in workspace sources")?.clone();
        let mut found: Vec<Candidate> = Vec::new();
        for file in files {
            let parsed = self.parsed(&file);
            found.extend(parsed.defs.iter().filter(|d| d.name() == name).map(|d| (file.clone(), d.clone())));
        }
        if found.is_empty() {
            return Err("not found in workspace sources");
        }
        if single(&found) {
            return Ok(found.swap_remove(0));
        }
        let choose = |found: Vec<Candidate>| -> Result<Candidate, &'static str> {
            if single(&found) {
                Ok(found.into_iter().next().unwrap())
            } else {
                Err("ambiguous definition")
            }
        };
        // 1. defined in the same file
        let local: Vec<Candidate> = found.iter().filter(|(file, _)| *file == from.file).cloned().collect();
        if !local.is_empty() {
            return choose(local);
        }
        // 2. the import's crate and module
        if let Some(path) = hint {
            let first = path.first().map(String::as_str);
            let krate = match first {
                Some("crate" | "super" | "self") | None => from.krate.clone(),
                Some(first) => first.replace('_', "-"),
            };
            let middle = if path.len() > 2 { path[1..path.len() - 1].join("::") } else { String::new() };
            let mut by_crate: Vec<Candidate> =
                found.iter().filter(|(file, _)| self.crate_of(file) == Some(krate.as_str())).cloned().collect();
            if by_crate.len() > 1 && !middle.is_empty() && !matches!(first, Some("super" | "self")) {
                let by_module: Vec<Candidate> = by_crate
                    .iter()
                    .filter(|(file, _)| {
                        self.origins.get(file).is_some_and(|(_, module)| {
                            *module == middle
                                || module.starts_with(&format!("{middle}::"))
                                || middle.starts_with(&format!("{module}::"))
                        })
                    })
                    .cloned()
                    .collect();
                if !by_module.is_empty() {
                    by_crate = by_module;
                }
            }
            if !by_crate.is_empty() {
                return choose(by_crate);
            }
            // The import's crate has no such definition: a re-export of a dependency's type, found
            // by the module name among the dependencies, or generated code (found nowhere).
            if !middle.is_empty() {
                let dependencies = self.dependencies(&from.krate);
                let by_module: Vec<Candidate> = found
                    .iter()
                    .filter(|(file, _)| {
                        self.crate_of(file).is_some_and(|k| dependencies.contains(k))
                            && self.origins.get(file).is_some_and(|(_, module)| {
                                *module == middle || module.ends_with(&format!("::{middle}"))
                            })
                    })
                    .cloned()
                    .collect();
                if !by_module.is_empty() {
                    return choose(by_module);
                }
            }
        }
        // 3. the file's own crate
        let by_crate: Vec<Candidate> =
            found.iter().filter(|(file, _)| self.crate_of(file) == Some(from.krate.as_str())).cloned().collect();
        if !by_crate.is_empty() {
            return choose(by_crate);
        }
        // 4. the one definition in a crate this file's crate depends on
        let dependencies = self.dependencies(&from.krate);
        let by_dependency: Vec<Candidate> = found
            .iter()
            .filter(|(file, _)| self.crate_of(file).is_some_and(|krate| dependencies.contains(krate)))
            .cloned()
            .collect();
        if !by_dependency.is_empty() {
            return choose(by_dependency);
        }
        Err("ambiguous definition, or defined by generated code")
    }
}

// ---------------------------------------------------------------------------------------------
// Type mapping

#[derive(Default, Clone)]
struct Mapped {
    wit: String,
    /// Named types referenced by name (not transitively).
    uses: BTreeSet<String>,
    /// Handles introduced right here: rust type labels.
    direct: BTreeSet<String>,
    /// Handles here and inside every named type used.
    closure: BTreeSet<String>,
}

impl Mapped {
    fn plain(wit: &str) -> Self {
        Self { wit: wit.into(), ..Self::default() }
    }

    fn handle(label: String) -> Self {
        Self {
            wit: "u64".into(),
            direct: BTreeSet::from([label.clone()]),
            closure: BTreeSet::from([label]),
            ..Self::default()
        }
    }

    fn wrap(wit: String, inner: Vec<Mapped>) -> Self {
        let mut mapped = Self { wit, ..Self::default() };
        for inner in inner {
            mapped.uses.extend(inner.uses);
            mapped.direct.extend(inner.direct);
            mapped.closure.extend(inner.closure);
        }
        mapped
    }
}

struct Named {
    declaration: String,
}

#[derive(Default)]
struct Mapper {
    index: TypeIndex,
    /// Where names are resolved from; the last entry is the type being built (or the hook).
    context: Vec<Context>,
    /// WIT name -> declaration.
    named: BTreeMap<String, Named>,
    by_rust: HashMap<String, Mapped>,
    building: Vec<String>,
    names: HashSet<String>,
    /// Hook function names: a type of the same name would clash inside the hooks interface.
    reserved: HashSet<String>,
    /// Why a path type became a handle, for the report: label -> reason.
    reasons: BTreeMap<String, String>,
}

const SCALARS: &[(&str, &str)] = &[
    ("bool", "bool"),
    ("char", "char"),
    ("i8", "s8"),
    ("i16", "s16"),
    ("i32", "s32"),
    ("i64", "s64"),
    ("isize", "s64"),
    ("u8", "u8"),
    ("u16", "u16"),
    ("u32", "u32"),
    ("u64", "u64"),
    ("usize", "u64"),
    ("f32", "f32"),
    ("f64", "f64"),
];

fn scalar(ty: &Type) -> Option<&'static str> {
    if let Type::Path(path) = ty
        && path.qself.is_none()
        && let Some(name) = path.path.get_ident()
    {
        return SCALARS.iter().find(|(rust, _)| name == rust).map(|(_, wit)| *wit);
    }
    None
}

fn label(ty: &Type) -> String {
    use quote::ToTokens;
    match ty {
        Type::Path(path) if path.qself.is_none() => {
            path.path.segments.last().map(|s| s.ident.to_string()).unwrap_or_default()
        }
        Type::Reference(reference) => {
            let inner = label(&reference.elem);
            if reference.mutability.is_some() { format!("&mut {inner}") } else { inner }
        }
        Type::Slice(slice) => format!("[{}]", label(&slice.elem)),
        other => {
            let text = other.to_token_stream().to_string();
            // Lifetimes carry no information for the report.
            text.replace("'_ ", "").replace("' a ", "").replace(" < '_ >", "")
        }
    }
}

fn type_args(arguments: &PathArguments) -> Vec<&Type> {
    match arguments {
        PathArguments::AngleBracketed(args) => args
            .args
            .iter()
            .filter_map(|arg| match arg {
                GenericArgument::Type(ty) => Some(ty),
                _ => None,
            })
            .collect(),
        _ => Vec::new(),
    }
}

/// glam's plain vector types, which the engine passes around by value.
fn glam(name: &str) -> Option<String> {
    let (element, count) = match name {
        "Vec2" => ("f32", 2),
        "Vec3" | "Vec3A" => ("f32", 3),
        "Vec4" | "Quat" => ("f32", 4),
        "DVec2" => ("f64", 2),
        "DVec3" => ("f64", 3),
        "DVec4" | "DQuat" => ("f64", 4),
        "IVec2" => ("s32", 2),
        "IVec3" => ("s32", 3),
        "IVec4" => ("s32", 4),
        "UVec2" => ("u32", 2),
        "UVec3" => ("u32", 3),
        "UVec4" => ("u32", 4),
        _ => return None,
    };
    Some(format!("tuple<{}>", vec![element; count].join(", ")))
}

impl Mapper {
    fn new(index: TypeIndex) -> Self {
        Self { index, ..Self::default() }
    }

    fn handle(&mut self, ty: &Type, reason: &str) -> Mapped {
        let label = label(ty);
        self.reasons.entry(label.clone()).or_insert_with(|| reason.to_owned());
        Mapped::handle(label)
    }

    fn map(&mut self, ty: &Type) -> Mapped {
        match ty {
            Type::Paren(inner) => self.map(&inner.elem),
            Type::Group(inner) => self.map(&inner.elem),
            Type::Reference(reference) => {
                if reference.mutability.is_some() && scalar(&reference.elem).is_none() {
                    // A live mutable object, not a value.
                    self.handle(ty, "mutable reference to a live object")
                } else {
                    self.map(&reference.elem)
                }
            }
            Type::Slice(slice) => {
                let inner = self.map(&slice.elem);
                Mapped::wrap(format!("list<{}>", inner.wit), vec![inner])
            }
            Type::Array(array) => {
                let inner = self.map(&array.elem);
                Mapped::wrap(format!("list<{}>", inner.wit), vec![inner])
            }
            Type::Tuple(tuple) => {
                if tuple.elems.is_empty() {
                    return self.handle(ty, "unit type");
                }
                let inner: Vec<_> = tuple.elems.iter().map(|elem| self.map(elem)).collect();
                let wit = format!("tuple<{}>", inner.iter().map(|m| m.wit.as_str()).collect::<Vec<_>>().join(", "));
                Mapped::wrap(wit, inner)
            }
            Type::Path(path) => self.map_path(ty, path),
            Type::TraitObject(_) => self.handle(ty, "trait object"),
            Type::ImplTrait(_) => self.handle(ty, "impl trait"),
            Type::BareFn(_) => self.handle(ty, "function pointer"),
            Type::Ptr(_) => self.handle(ty, "raw pointer"),
            _ => self.handle(ty, "unsupported type form"),
        }
    }

    fn map_path(&mut self, ty: &Type, path: &syn::TypePath) -> Mapped {
        if path.qself.is_some() {
            return self.handle(ty, "qualified path");
        }
        let Some(last) = path.path.segments.last() else {
            return self.handle(ty, "empty path");
        };
        let name = last.ident.to_string();
        let args = type_args(&last.arguments);
        if let Some(wit) = scalar(ty) {
            return Mapped::plain(wit);
        }
        if matches!(name.as_str(), "u128" | "i128") && args.is_empty() {
            // 16 bytes in big-endian order, like the UUIDs of the frozen client ABI.
            return Mapped::plain("list<u8>");
        }
        if args.is_empty()
            && (self.context.last().and_then(|c| c.parsed.imports.get(&name)).is_some_and(|p| p.first().is_some_and(|f| f == "glam"))
                || (path.path.segments.len() > 1 && path.path.segments.first().is_some_and(|s| s.ident == "glam")))
            && let Some(wit) = glam(&name)
        {
            return Mapped::plain(&wit);
        }
        match (name.as_str(), args.as_slice()) {
            ("String" | "str" | "PathBuf" | "Path" | "OsString" | "Uuid", []) => {
                return Mapped::plain("string");
            }
            ("Box", [inner]) => return self.map(inner),
            ("Option", [inner]) => {
                let inner = self.map(inner);
                return Mapped::wrap(format!("option<{}>", inner.wit), vec![inner]);
            }
            ("Vec" | "VecDeque" | "HashSet" | "BTreeSet" | "BinaryHeap", [inner]) => {
                let inner = self.map(inner);
                return Mapped::wrap(format!("list<{}>", inner.wit), vec![inner]);
            }
            ("Cow", [inner]) => return self.map(inner),
            ("HashMap" | "BTreeMap" | "FxHashMap" | "IndexMap", [key, value, ..]) => {
                let key = self.map(key);
                let value = self.map(value);
                let wit = format!("list<tuple<{}, {}>>", key.wit, value.wit);
                return Mapped::wrap(wit, vec![key, value]);
            }
            ("Result", [ok, err]) => {
                let (ok, err) = (self.map(ok), self.map(err));
                let wit = format!("result<{}, {}>", ok.wit, err.wit);
                return Mapped::wrap(wit, vec![ok, err]);
            }
            // Shared immutable data is read as its value.
            ("Rc" | "Arc", [inner]) => return self.map(inner),
            ("RefCell" | "Cell" | "Mutex" | "RwLock" | "PhantomData" | "Instant" | "Rc" | "Arc", _) => {
                return self.handle(ty, "shared or runtime-only type");
            }
            _ => {}
        }
        if args.is_empty()
            && let Some(mapped) = self.table(path, &name)
        {
            return mapped;
        }
        self.named(ty, &name)
    }

    /// Whether `name`, as written in `path` or imported by the current file, comes from one of
    /// the `origin` modules or crates.
    fn comes_from(&self, path: &syn::TypePath, name: &str, origin: &[&str]) -> bool {
        if origin.is_empty() {
            return true;
        }
        let written = path.path.segments.iter().map(|s| s.ident.to_string());
        let imported = self
            .context
            .last()
            .and_then(|c| c.parsed.imports.get(name))
            .into_iter()
            .flat_map(|p| p.iter().cloned());
        written.chain(imported).any(|segment| origin.iter().any(|o| segment == *o))
    }

    fn declare(&mut self, name: &str, declaration: &str) {
        self.named.entry(name.to_owned()).or_insert_with(|| Named { declaration: declaration.to_owned() });
        self.names.insert(name.to_owned());
    }

    /// The type-table carrier for `name`, when it is the engine's type of that name.
    fn table(&mut self, path: &syn::TypePath, name: &str) -> Option<Mapped> {
        let row = TABLE.iter().find(|row| row.rust == name)?;
        if !self.comes_from(path, name, row.origin) {
            // No hint in the file (a type of the same crate): the definition decides.
            let from = self.context.last().cloned()?;
            let (file, _) = self.index.resolve(name, &from).ok()?;
            let krate = self.index.crate_of(&file)?;
            if !matches!(krate, "horizon-world" | "horizon-items" | "horizon-client-core") {
                return None;
            }
        }
        let referenced = |wit: &str| Mapped { wit: wit.to_owned(), uses: BTreeSet::from([wit.to_owned()]), ..Mapped::default() };
        Some(match row.carrier {
            Carrier::Alias(alias) => {
                let doc = match alias {
                    "nbt" => NBT_DOC,
                    "text-json" => TEXT_DOC,
                    _ => JSON_DOC,
                };
                self.declare(alias, doc);
                referenced(alias)
            }
            Carrier::Record(record) => {
                self.declare("nbt", NBT_DOC);
                self.declare(record, ITEM_STACK_DOC);
                referenced(record)
            }
            Carrier::Struct(target) => {
                let ty: Type = syn::parse_str(target).ok()?;
                let file = self.index.files.get(target)?.first()?.clone();
                let inside = self.index.context(&file);
                self.context.push(inside);
                let mapped = self.named(&ty, target);
                self.context.pop();
                mapped
            }
        })
    }

    /// Declares the commands of `effect` and returns the list type that carries them.
    fn effect(&mut self, effect: &Effect) -> Mapped {
        self.declare("nbt", NBT_DOC);
        self.declare("text-json", TEXT_DOC);
        self.declare("item-stack", ITEM_STACK_DOC);
        for (index, name) in effect.names.iter().enumerate() {
            // The declaration is one text block shared by the names; it is stored once under the first.
            let declaration = if index == 0 { effect.declaration } else { "" };
            self.named.entry((*name).to_owned()).or_insert_with(|| Named { declaration: declaration.to_owned() });
            self.names.insert((*name).to_owned());
        }
        Mapped { wit: format!("list<{}>", effect.wit), uses: BTreeSet::from([effect.wit.to_owned()]), ..Mapped::default() }
    }

    /// The generated named type for the workspace type `name`, or a handle.
    fn named(&mut self, ty: &Type, name: &str) -> Mapped {
        let Some(from) = self.context.last().cloned() else {
            return self.handle(ty, "no source context");
        };
        let (file, def) = match self.index.resolve(name, &from) {
            Ok(found) => found,
            Err(reason) => return self.handle(ty, reason),
        };
        let key = format!("{}@{}", file.display(), def.name());
        if let Some(mapped) = self.by_rust.get(&key) {
            return mapped.clone();
        }
        if self.building.contains(&key) {
            return self.handle(ty, "recursive type");
        }
        // The definition's own file decides what its field types mean.
        let inside = self.index.context(&file);
        self.context.push(inside);
        self.building.push(key.clone());
        let mapped = self.build(ty, name, &def);
        self.building.pop();
        self.context.pop();
        self.by_rust.insert(key, mapped.clone());
        mapped
    }

    /// Register a generated named type built from `inner` and refer to it.
    fn reference(&mut self, declaration: (String, String), inner: Vec<Mapped>) -> Mapped {
        let (wit_name, text) = declaration;
        let mut closure = BTreeSet::new();
        let mut uses = BTreeSet::new();
        for inner in &inner {
            closure.extend(inner.closure.iter().cloned());
            uses.extend(inner.uses.iter().cloned());
        }
        self.named.insert(wit_name.clone(), Named { declaration: text });
        let mut set = BTreeSet::new();
        set.insert(wit_name.clone());
        Mapped { wit: escape(&wit_name), uses: set, direct: BTreeSet::new(), closure }
    }

    fn fresh_name(&mut self, rust: &str) -> String {
        let mut base = kebab(rust);
        if self.reserved.contains(&base) {
            base.push_str("-type");
        }
        unique(base, &mut self.names)
    }

    fn build(&mut self, ty: &Type, name: &str, def: &Def) -> Mapped {
        match def {
            Def::Alias(item) => {
                if item.generics.params.iter().any(|p| !matches!(p, syn::GenericParam::Lifetime(_))) {
                    return self.handle(ty, "generic type alias");
                }
                let target = (*item.ty).clone();
                self.map(&target)
            }
            Def::Struct(item) => self.build_struct(ty, name, item),
            Def::Enum(item) => self.build_enum(ty, name, item),
        }
    }

    fn generic(generics: &syn::Generics) -> bool {
        generics.params.iter().any(|p| !matches!(p, syn::GenericParam::Lifetime(_)))
    }

    fn build_struct(&mut self, ty: &Type, name: &str, item: &syn::ItemStruct) -> Mapped {
        if Self::generic(&item.generics) {
            return self.handle(ty, "generic type");
        }
        match &item.fields {
            syn::Fields::Unit => {
                let wit_name = self.fresh_name(name);
                let text = format!("    enum {} {{\n        unit,\n    }}\n", escape(&wit_name));
                self.reference((wit_name, text), Vec::new())
            }
            syn::Fields::Unnamed(fields) if fields.unnamed.len() == 1 => {
                // A newtype is its inner value.
                let inner = fields.unnamed[0].ty.clone();
                self.map(&inner)
            }
            fields => {
                let list: Vec<&syn::Field> = fields.iter().collect();
                if list.len() > MAX_NAMED_FIELDS {
                    return self.handle(ty, "too many fields for a plain record");
                }
                if list.iter().any(|f| matches!(f.vis, syn::Visibility::Inherited)) {
                    return self.handle(ty, "struct with private fields");
                }
                let list: Vec<&syn::Field> = list
                    .into_iter()
                    .filter(|f| !matches!(&f.ty, Type::Path(p) if p.path.segments.last().is_some_and(|s| s.ident == "PhantomData")))
                    .collect();
                if list.is_empty() {
                    let wit_name = self.fresh_name(name);
                    let text = format!("    enum {} {{\n        unit,\n    }}\n", escape(&wit_name));
                    return self.reference((wit_name, text), Vec::new());
                }
                let wit_name = self.fresh_name(name);
                let mut used = HashSet::new();
                let mut inner = Vec::new();
                let mut text = format!("    record {} {{\n", escape(&wit_name));
                for (index, field) in list.iter().enumerate() {
                    let field_name = match &field.ident {
                        Some(ident) => ident.to_string(),
                        None => format!("f{index}"),
                    };
                    let mapped = self.map(&field.ty);
                    let field_name = unique(kebab(&field_name), &mut used);
                    for handle in &mapped.direct {
                        text.push_str(&format!("        /// opaque engine handle ({handle})\n"));
                    }
                    text.push_str(&format!("        {}: {},\n", escape(&field_name), mapped.wit));
                    inner.push(mapped);
                }
                text.push_str("    }\n");
                self.reference((wit_name, text), inner)
            }
        }
    }

    fn build_enum(&mut self, ty: &Type, name: &str, item: &syn::ItemEnum) -> Mapped {
        if Self::generic(&item.generics) {
            return self.handle(ty, "generic type");
        }
        if item.variants.is_empty() {
            return self.handle(ty, "empty enum");
        }
        let wit_name = self.fresh_name(name);
        let mut used = HashSet::new();
        let all_unit = item.variants.iter().all(|v| matches!(v.fields, syn::Fields::Unit));
        let mut inner = Vec::new();
        let mut text;
        if all_unit {
            text = format!("    enum {} {{\n", escape(&wit_name));
            for variant in &item.variants {
                let case = unique(kebab(&variant.ident.to_string()), &mut used);
                text.push_str(&format!("        {},\n", escape(&case)));
            }
        } else {
            text = format!("    variant {} {{\n", escape(&wit_name));
            for variant in &item.variants {
                let case = unique(kebab(&variant.ident.to_string()), &mut used);
                let payload = match &variant.fields {
                    syn::Fields::Unit => None,
                    syn::Fields::Unnamed(fields) if fields.unnamed.len() == 1 => {
                        let mapped = self.map(&fields.unnamed[0].ty);
                        Some(mapped)
                    }
                    syn::Fields::Unnamed(fields) => {
                        let parts: Vec<_> = fields.unnamed.iter().map(|f| self.map(&f.ty)).collect();
                        let wit = format!("tuple<{}>", parts.iter().map(|m| m.wit.as_str()).collect::<Vec<_>>().join(", "));
                        Some(Mapped::wrap(wit, parts))
                    }
                    syn::Fields::Named(fields) => {
                        // A struct variant gets its own record.
                        let record = self.fresh_name(&format!("{name}-{}", variant.ident));
                        let mut field_names = HashSet::new();
                        let mut parts = Vec::new();
                        let mut body = format!("    record {} {{\n", escape(&record));
                        for field in &fields.named {
                            let mapped = self.map(&field.ty);
                            let field_name = unique(kebab(&field.ident.as_ref().unwrap().to_string()), &mut field_names);
                            for handle in &mapped.direct {
                                body.push_str(&format!("        /// opaque engine handle ({handle})\n"));
                            }
                            body.push_str(&format!("        {}: {},\n", escape(&field_name), mapped.wit));
                            parts.push(mapped);
                        }
                        if parts.is_empty() {
                            None
                        } else {
                            body.push_str("    }\n");
                            let mut closure = BTreeSet::new();
                            let mut uses = BTreeSet::new();
                            for part in &parts {
                                closure.extend(part.closure.iter().cloned());
                                uses.extend(part.uses.iter().cloned());
                            }
                            self.named.insert(record.clone(), Named { declaration: body });
                            uses.insert(record.clone());
                            Some(Mapped { wit: escape(&record), uses, direct: BTreeSet::new(), closure })
                        }
                    }
                };
                match payload {
                    Some(mapped) => {
                        for handle in &mapped.direct {
                            text.push_str(&format!("        /// opaque engine handle ({handle})\n"));
                        }
                        text.push_str(&format!("        {}({}),\n", escape(&case), mapped.wit));
                        inner.push(mapped);
                    }
                    None => text.push_str(&format!("        {},\n", escape(&case))),
                }
            }
        }
        text.push_str("    }\n");
        self.reference((wit_name, text), inner)
    }
}

// ---------------------------------------------------------------------------------------------
// Hook signatures

#[derive(Clone, Copy, PartialEq, Eq, Debug, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum Side {
    Client,
    Server,
    Both,
}

struct HookSig {
    name: String,
    side: Side,
    side_source: &'static str,
    contract: String,
    docs: String,
    params: Vec<(String, Type)>,
    out: Option<(String, Type)>,
    /// The file that defines the hook function, for resolving its type names.
    source: PathBuf,
}

fn side_of(hook: &serde_json::Value, target: &str, is_level: bool) -> (Side, &'static str) {
    match hook["side"].as_str() {
        Some("client") => return (Side::Client, "explicit"),
        Some("server") => return (Side::Server, "explicit"),
        Some("both") => return (Side::Both, "explicit"),
        _ => {}
    }
    if target.starts_with("horizon_client_render::") || target.starts_with("horizon_client_lib::") {
        (Side::Client, "path")
    } else if target.starts_with("horizon_world::") || target.starts_with("horizon_server") {
        (Side::Server, "path")
    } else if is_level {
        (Side::Server, "path")
    } else {
        (Side::Both, "undecidable")
    }
}

fn load_hooks(root: &Path) -> Result<Vec<HookSig>, String> {
    let catalog: serde_json::Value = serde_json::from_str(
        &fs::read_to_string(root.join("native-hooks.json")).map_err(|e| e.to_string())?,
    )
    .map_err(|e| e.to_string())?;
    let source = fs::read_to_string(root.join("crates/horizon-client/src/game/native_hooks.rs"))
        .map_err(|e| e.to_string())?;
    let file = syn::parse_file(&source).map_err(|e| e.to_string())?;
    let mut methods = Vec::new();
    for item in file.items {
        if let Item::Impl(item) = item {
            methods.extend(crate::impl_functions(item));
        }
    }
    let mut result = Vec::new();
    let game_source = root.join("crates/horizon-client/src/game/native_hooks.rs");
    for hook in crate::catalog_hooks(root, &catalog)? {
        let name = hook["name"].as_str().ok_or("hook name missing")?.to_owned();
        let target = hook["target"].as_str().ok_or("target missing")?;
        let native = format!("native_{name}");
        let mut source_path = game_source.clone();
        let method = if let Some((krate, path)) = crate::free_function_source(target) {
            let (module, _) = path.rsplit_once("::").ok_or("target missing module")?;
            let path = root
                .join("crates")
                .join(krate)
                .join("src")
                .join(format!("{}.rs", module.replace("::", "/")));
            let source = fs::read_to_string(&path).map_err(|e| format!("{}: {e}", path.display()))?;
            source_path = path.clone();
            let file = syn::parse_file(&source).map_err(|e| e.to_string())?;
            let functions: Vec<_> = file
                .items
                .into_iter()
                .filter_map(|item| match item {
                    Item::Fn(function) => Some(function),
                    _ => None,
                })
                .collect();
            crate::one_function(functions.iter(), &native)?.clone()
        } else {
            crate::one_function(methods.iter(), &native)?.clone()
        };
        let is_level = target.contains("::GameLevel<");
        let api = &hook["mod_api"];
        let contract = api["contract"].as_str().ok_or("contract missing")?.to_owned();
        let out_name = api["out"].as_str();
        let mut params = Vec::new();
        for arg in &method.sig.inputs {
            if let FnArg::Typed(arg) = arg {
                let Pat::Ident(pat) = &*arg.pat else {
                    return Err(format!("unsupported argument in {name}"));
                };
                let text = pat.ident.to_string();
                if is_level && text == "ticks" {
                    continue;
                }
                params.push((text, (*arg.ty).clone()));
            }
        }
        let out = match out_name {
            Some(out) => {
                let index = params
                    .iter()
                    .position(|(n, _)| n == out)
                    .ok_or_else(|| format!("missing output {out} in {name}"))?;
                Some(params.remove(index))
            }
            None => None,
        };
        let (side, side_source) = side_of(&hook, target, is_level);
        let docs = crate::doc_comments(&method.attrs, &name, false).unwrap_or_default();
        let docs = docs.lines().map(|line| line.strip_prefix("///").unwrap_or(line)).collect::<Vec<_>>().join("\n");
        result.push(HookSig { name, side, side_source, contract, docs, params, out, source: source_path });
    }
    Ok(result)
}

// ---------------------------------------------------------------------------------------------
// Rendering

#[derive(Serialize)]
struct HandleParam {
    param: String,
    rust_types: Vec<String>,
}

#[derive(Serialize)]
struct CoverageEntry {
    name: String,
    side: Side,
    side_source: &'static str,
    contract: String,
    /// No handles and nothing left out.
    fully_typed: bool,
    /// Typed, but some engine methods have no command (see `partial_notes`).
    partial: bool,
    partial_notes: Vec<String>,
    handle_params: Vec<HandleParam>,
}

#[derive(Serialize)]
struct HandleType {
    rust_type: String,
    hooks: usize,
    reason: String,
}

#[derive(Serialize)]
struct Coverage {
    generated_by: &'static str,
    hook_count: usize,
    fully_typed: usize,
    partial: usize,
    with_handles: usize,
    client_hooks: usize,
    server_hooks: usize,
    named_types: usize,
    handle_types: Vec<HandleType>,
    hooks: Vec<CoverageEntry>,
}

/// The generated full-catalog WIT and its coverage report.
pub struct FullWit {
    pub wit: String,
    pub coverage_json: String,
    pub hook_count: usize,
    pub client_hooks: usize,
    pub server_hooks: usize,
    pub fully_typed: usize,
    pub partial: usize,
    pub with_handles: usize,
    /// Handle rust type -> number of hooks that have it, most common first.
    pub handle_types: Vec<(String, usize)>,
}

struct Rendered {
    /// The doc comment and function line.
    text: String,
    uses: BTreeSet<String>,
}

fn render_hook(mapper: &mut Mapper, hook: &HookSig, entry: &mut CoverageEntry) -> Rendered {
    let mut uses = BTreeSet::new();
    let context = mapper.index.context(&hook.source);
    mapper.context = vec![context];
    let mut doc = String::new();
    let mut used_params = HashSet::new();
    let mut params = Vec::new();
    let mut extras: Vec<(String, Mapped)> = Vec::new();
    let mut notes = Vec::new();
    let mut handle_params: Vec<HandleParam> = Vec::new();
    let mut all_handles: BTreeSet<String> = BTreeSet::new();
    // Live draw objects: (param, command list type).
    let mut effects: Vec<(String, Mapped)> = Vec::new();

    let record_handles = |name: &str, mapped: &Mapped, handle_params: &mut Vec<HandleParam>, all: &mut BTreeSet<String>| {
        if !mapped.closure.is_empty() {
            handle_params.push(HandleParam { param: name.to_owned(), rust_types: mapped.closure.iter().cloned().collect() });
            all.extend(mapped.closure.iter().cloned());
        }
    };

    for (name, ty) in &hook.params {
        let wit_name = unique(kebab(name), &mut used_params);
        if let Some(effect) = effect_of(ty) {
            let mapped = mapper.effect(effect);
            notes.push(format!("`{wit_name}` is applied by the host: return `{}` commands for it.", effect.wit));
            if let Some(note) = effect.partial {
                let note = note.to_owned();
                if !entry.partial_notes.contains(&note) {
                    entry.partial_notes.push(note);
                }
                entry.partial = true;
            }
            uses.extend(mapped.uses.iter().cloned());
            effects.push((wit_name, mapped));
            continue;
        }
        // `&mut` values the hook may change: scalars always, other types when fully typed.
        let mutable_elem = match ty {
            Type::Reference(reference) if reference.mutability.is_some() => Some(&*reference.elem),
            _ => None,
        };
        let mapped = match mutable_elem {
            Some(elem) if scalar(elem).is_some() => {
                let mapped = mapper.map(elem);
                extras.push((wit_name.clone(), mapped.clone()));
                mapped
            }
            Some(elem) => {
                let mapped = mapper.map(elem);
                if mapped.closure.is_empty() {
                    extras.push((wit_name.clone(), mapped.clone()));
                    mapped
                } else {
                    mapper.map(ty)
                }
            }
            None => mapper.map(ty),
        };
        for handle in &mapped.direct {
            notes.push(format!("`{wit_name}` is an opaque engine handle ({handle})."));
        }
        record_handles(&wit_name, &mapped, &mut handle_params, &mut all_handles);
        uses.extend(mapped.uses.iter().cloned());
        params.push(format!("{}: {}", escape(&wit_name), mapped.wit));
    }

    // The contract's answer.
    let mut main_field: Option<(&'static str, String)> = None;
    match (hook.contract.as_str(), &hook.out) {
        ("listen", _) => {}
        ("cancel", _) => {
            main_field = Some(("cancel", "bool".into()));
            notes.push("Returns true to cancel (take) the event; the first mod that cancels wins.".into());
        }
        ("replace", Some((name, ty))) => {
            let inner = match ty {
                Type::Reference(reference) => &*reference.elem,
                other => other,
            };
            let mapped = mapper.map(inner);
            record_handles(name, &mapped, &mut handle_params, &mut all_handles);
            uses.extend(mapped.uses.iter().cloned());
            let wit = if mapped.wit.starts_with("option<") { mapped.wit.clone() } else { format!("option<{}>", mapped.wit) };
            main_field = Some(("value", wit));
            notes.push(format!("Returns Some to set `{name}` (None keeps it); the first mod that answers wins."));
        }
        ("collect", Some((name, ty))) => {
            let inner = match ty {
                Type::Reference(reference) => &*reference.elem,
                other => other,
            };
            let mapped = mapper.map(inner);
            record_handles(name, &mapped, &mut handle_params, &mut all_handles);
            uses.extend(mapped.uses.iter().cloned());
            // The current aggregate goes in, the mod's version comes back.
            let param = unique(kebab(name), &mut used_params);
            params.push(format!("{}: {}", escape(&param), mapped.wit));
            let is_list = mapped.wit.starts_with("list<");
            let wit = if is_list { mapped.wit.clone() } else { format!("option<{}>", mapped.wit) };
            main_field = Some(("value", wit));
            notes.push(if is_list {
                format!("Collect: `{param}` is the aggregate so far; return the aggregate you want.")
            } else {
                format!("Collect: `{param}` is the current value; return Some to change it (None keeps it).")
            });
        }
        _ => {}
    }

    // Extra mutable values turn the answer into an outcome record.
    let wit_name = kebab(&hook.name);
    let result = if extras.is_empty() && effects.is_empty() {
        main_field.map(|(_, wit)| wit)
    } else if extras.is_empty() && effects.len() == 1 && main_field.is_none() {
        notes.push("Returns the commands to apply.".into());
        Some(effects[0].1.wit.clone())
    } else {
        let record = format!("{wit_name}-outcome");
        let record = if mapper.reserved.contains(&record) { format!("{record}-type") } else { record };
        let mut text = format!("    record {} {{\n", escape(&record));
        let mut used = HashSet::new();
        if let Some((field, wit)) = &main_field {
            used.insert((*field).to_owned());
            text.push_str(&format!("        {field}: {wit},\n"));
        }
        let single = effects.len() == 1;
        for (name, mapped) in &effects {
            let field = if single { unique("commands".into(), &mut used) } else { unique(format!("{name}-commands"), &mut used) };
            text.push_str(&format!("        /// Commands to apply to `{name}`, in order.\n"));
            text.push_str(&format!("        {}: {},\n", escape(&field), mapped.wit));
        }
        for (name, mapped) in &extras {
            let field = unique(name.clone(), &mut used);
            for handle in &mapped.direct {
                text.push_str(&format!("        /// opaque engine handle ({handle})\n"));
            }
            text.push_str(&format!("        /// The new `{name}`, or none to leave it unchanged.\n"));
            text.push_str(&format!("        {}: option<{}>,\n", escape(&field), mapped.wit));
        }
        text.push_str("    }\n");
        for (_, mapped) in &extras {
            uses.extend(mapped.uses.iter().cloned());
        }
        mapper.named.insert(record.clone(), Named { declaration: text });
        mapper.names.insert(record.clone());
        uses.insert(record.clone());
        if !extras.is_empty() {
            notes.push(format!("Also returns the changed values of {}.", extras.iter().map(|(n, _)| format!("`{n}`")).collect::<Vec<_>>().join(", ")));
        }
        Some(escape(&record))
    };

    if !hook.docs.trim().is_empty() {
        doc.push_str(&doc_lines(&hook.docs, "    "));
        if !notes.is_empty() {
            doc.push_str("    ///\n");
        }
    }
    for note in notes {
        doc.push_str(&format!("    /// {note}\n"));
    }
    let result = result.map(|r| format!(" -> {r}")).unwrap_or_default();
    let text = format!("{doc}    {}: func({}){result};\n", ident(&hook.name), params.join(", "));
    handle_params.sort_by(|a, b| a.param.cmp(&b.param));
    handle_params.dedup_by(|a, b| a.param == b.param);
    entry.fully_typed = all_handles.is_empty() && !entry.partial;
    entry.handle_params = handle_params;
    Rendered { text, uses }
}

fn use_line(names: &BTreeSet<String>) -> String {
    if names.is_empty() {
        return String::new();
    }
    let list: Vec<String> = names.iter().map(|n| escape(n)).collect();
    format!("    use engine-types.{{{}}};\n\n", list.join(", "))
}

fn draft_services() -> Result<String, String> {
    let draft: serde_json::Value = serde_json::from_str(SERVER_DRAFT).map_err(|e| e.to_string())?;
    let mut types = String::from("interface server-types {\n");
    let mut names = Vec::new();
    for (name, values) in draft["enums"].as_object().ok_or("draft enums missing")? {
        names.push(name.clone());
        types.push_str(&format!("    enum {} {{\n", escape(name)));
        for case in values.as_array().ok_or("enum cases")? {
            types.push_str(&format!("        {},\n", escape(case.as_str().ok_or("enum case")?)));
        }
        types.push_str("    }\n\n");
    }
    for (name, fields) in draft["types"].as_object().ok_or("draft types missing")? {
        names.push(name.clone());
        types.push_str(&format!("    record {} {{\n", escape(name)));
        let fields = fields.as_object().ok_or("draft fields")?;
        for (field, ty) in fields {
            types.push_str(&format!("        {}: {},\n", escape(field), ty.as_str().ok_or("draft field type")?));
        }
        types.push_str("    }\n\n");
    }
    types = types.trim_end().to_owned();
    types.push_str("\n}\n\n");
    let mut services = String::from("/// Draft server services (value types only): see `abi-subset-server.json`.\ninterface server-services {\n");
    services.push_str(&format!("    use server-types.{{{}}};\n\n", names.iter().map(|n| escape(n)).collect::<Vec<_>>().join(", ")));
    let type_names: HashSet<String> = names.iter().map(|n| kebab(n)).collect();
    for service in draft["services"].as_array().ok_or("draft services missing")? {
        let name = service["name"].as_str().ok_or("service name")?;
        let mut args: Vec<(&String, &serde_json::Value)> = service["arguments"].as_object().ok_or("service args")?.iter().collect();
        args.sort_by(|a, b| a.0.cmp(b.0));
        let args: Vec<String> = args
            .into_iter()
            .map(|(n, t)| Ok(format!("{}: {}", escape(n), t.as_str().ok_or("arg type")?)))
            .collect::<Result<_, String>>()?;
        let result = service["result"].as_str().map(|r| format!(" -> {r}")).unwrap_or_default();
        // A function may not share a name with a type of the interface.
        let mut function = kebab(name);
        if type_names.contains(&function) {
            function.push_str("-call");
        }
        services.push_str(&format!("    {}: func({}){result};\n", escape(&function), args.join(", ")));
    }
    services.push_str("}\n\n");
    Ok(format!("{types}{services}"))
}

/// Generate `yoake-full.wit` and `coverage.json` for the workspace at `root`.
pub fn generate_full_wit(root: &Path) -> Result<FullWit, String> {
    let hooks = load_hooks(root)?;
    let mut mapper = Mapper::new(TypeIndex::load(root)?);
    mapper.reserved = hooks.iter().map(|hook| kebab(&hook.name)).collect();
    let mut entries = Vec::new();
    let mut client = (String::new(), BTreeSet::new());
    let mut server = (String::new(), BTreeSet::new());
    let (mut client_count, mut server_count) = (0, 0);
    let mut handle_counts: BTreeMap<String, usize> = BTreeMap::new();
    for hook in &hooks {
        let mut entry = CoverageEntry {
            name: hook.name.clone(),
            side: hook.side,
            side_source: hook.side_source,
            contract: hook.contract.clone(),
            fully_typed: true,
            partial: false,
            partial_notes: Vec::new(),
            handle_params: Vec::new(),
        };
        let rendered = render_hook(&mut mapper, hook, &mut entry);
        let mut hook_handles = BTreeSet::new();
        for param in &entry.handle_params {
            hook_handles.extend(param.rust_types.iter().cloned());
        }
        for handle in hook_handles {
            *handle_counts.entry(handle).or_default() += 1;
        }
        if matches!(hook.side, Side::Client | Side::Both) {
            client.0.push_str(&rendered.text);
            client.1.extend(rendered.uses.iter().cloned());
            client_count += 1;
        }
        if matches!(hook.side, Side::Server | Side::Both) {
            server.0.push_str(&rendered.text);
            server.1.extend(rendered.uses.iter().cloned());
            server_count += 1;
        }
        entries.push(entry);
    }

    let mut wit = String::from("package yoake:mods@1.0.0;\n\n// @generated by horizon-mod-api-gen; do not edit.\n");
    wit.push_str("// Every native hook of the catalog, typed from the Rust hook signatures.\n");
    wit.push_str("// Side rule: an explicit catalog side wins; else the Rust target path decides\n");
    wit.push_str("// (render/lib crates are client, world/server crates and GameLevel hooks are server);\n");
    wit.push_str("// the remaining Game hooks run on both and appear in both interfaces.\n");
    wit.push_str("// Types: plain Rust structs/enums become records/enums/variants; NBT is `nbt` (network\n");
    wit.push_str("// bytes), item stacks `item-stack`, text `text-json`, JSON values `json`; `&mut` draw objects\n");
    wit.push_str("// become command lists the guest returns; anything else is a `u64` opaque engine handle\n");
    wit.push_str("// (documented per field). See coverage.json.\n");
    wit.push_str("\n");

    wit.push_str("interface engine-types {\n");
    let mut first = true;
    for (_, named) in mapper.named.iter().filter(|(_, named)| !named.declaration.is_empty()) {
        if !first {
            wit.push('\n');
        }
        first = false;
        wit.push_str(&named.declaration);
    }
    wit.push_str("}\n\n");

    wit.push_str(&draft_services()?);

    for (side, (text, uses), doc) in [
        ("client", &client, "Hooks that run in the client."),
        ("server", &server, "Hooks that run in the server (integrated or dedicated)."),
    ] {
        wit.push_str(&format!("/// {doc}\ninterface {side}-hooks {{\n"));
        wit.push_str(&use_line(uses));
        wit.push_str(text);
        wit.push_str("}\n\n");
    }

    for (side, imports, extra) in [
        ("client", CLIENT_IMPORTS, &[][..]),
        ("server", SERVER_IMPORTS, &["server-types", "server-services"][..]),
    ] {
        wit.push_str(&format!("world {side}-full {{\n    import engine-types;\n"));
        for import in imports.iter().chain(extra) {
            wit.push_str(&format!("    import {import};\n"));
        }
        wit.push_str(&format!("    export {side}-hooks;\n"));
        wit.push_str("    /// Returns catalog hook names with underscores, such as `net_frames_listened`.\n    export init: func() -> list<string>;\n}\n");
        if side == "client" {
            wit.push('\n');
        }
    }

    let fully_typed = entries.iter().filter(|e| e.fully_typed).count();
    let partial = entries.iter().filter(|e| e.partial && e.handle_params.is_empty()).count();
    let mut handle_types: Vec<(String, usize)> = handle_counts.into_iter().collect();
    handle_types.sort_by(|a, b| b.1.cmp(&a.1).then(a.0.cmp(&b.0)));
    let handle_report = handle_types
        .iter()
        .map(|(rust_type, hooks)| HandleType {
            rust_type: rust_type.clone(),
            hooks: *hooks,
            reason: mapper.reasons.get(rust_type).cloned().unwrap_or_else(|| "recursive or encapsulated type".into()),
        })
        .collect();
    let coverage = Coverage {
        generated_by: "horizon-mod-api-gen",
        hook_count: entries.len(),
        fully_typed,
        partial,
        with_handles: entries.iter().filter(|e| !e.handle_params.is_empty()).count(),
        client_hooks: client_count,
        server_hooks: server_count,
        named_types: mapper.named.len(),
        handle_types: handle_report,
        hooks: entries,
    };
    let mut coverage_json = serde_json::to_string_pretty(&coverage).map_err(|e| e.to_string())?;
    coverage_json.push('\n');
    Ok(FullWit {
        wit,
        coverage_json,
        hook_count: coverage.hook_count,
        client_hooks: client_count,
        server_hooks: server_count,
        fully_typed,
        partial,
        with_handles: coverage.with_handles,
        handle_types,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn root() -> std::path::PathBuf {
        Path::new(env!("CARGO_MANIFEST_DIR")).join("../..")
    }

    fn generated() -> &'static FullWit {
        static GENERATED: std::sync::OnceLock<FullWit> = std::sync::OnceLock::new();
        GENERATED.get_or_init(|| generate_full_wit(&root()).expect("generate full WIT"))
    }

    fn wit_dir() -> std::path::PathBuf {
        root().join("crates/yoake-wasm/wit")
    }

    #[test]
    fn kebab_follows_wit_words() {
        assert_eq!(kebab("BlockPos"), "block-pos");
        assert_eq!(kebab("AABB"), "aabb");
        assert_eq!(kebab("HTTPServer"), "http-server");
        assert_eq!(kebab("Vec3"), "vec3");
        assert_eq!(kebab("x_2d"), "x2d");
        assert_eq!(kebab("_0"), "n0");
        assert_eq!(kebab("r#type"), "type");
        assert_eq!(ident("type"), "%type");
        assert_eq!(ident("from"), "%from");
    }

    #[test]
    fn full_wit_matches_catalog_and_parses() {
        let generated = generated();
        let again = generate_full_wit(&root()).expect("regenerate full WIT");
        assert_eq!(generated.wit, again.wit);
        assert_eq!(generated.coverage_json, again.coverage_json);
        if std::env::var_os("YOAKE_UPDATE_WIT").is_some() {
            fs::write(wit_dir().join("yoake-full.wit"), &generated.wit).unwrap();
            fs::write(wit_dir().join("coverage.json"), &generated.coverage_json).unwrap();
        }
        let committed = fs::read_to_string(wit_dir().join("yoake-full.wit")).expect("read yoake-full.wit");
        assert_eq!(generated.wit, committed.replace("\r\n", "\n"), "regenerate with YOAKE_UPDATE_WIT=1");
        let committed = fs::read_to_string(wit_dir().join("coverage.json")).expect("read coverage.json");
        assert_eq!(generated.coverage_json, committed.replace("\r\n", "\n"));
        let mut resolve = wit_parser::Resolve::default();
        let (package, _) = resolve.push_dir(wit_dir()).expect("parse WIT package");
        for world in ["client-mod", "client-full", "server-full"] {
            resolve.select_world(&[package], Some(world)).unwrap_or_else(|e| panic!("{world}: {e}"));
        }
    }

    #[test]
    fn full_wit_covers_every_catalog_hook() {
        let generated = generated();
        let catalog: serde_json::Value =
            serde_json::from_str(&fs::read_to_string(root().join("native-hooks.json")).unwrap()).unwrap();
        let fragments = crate::catalog_hooks(&root(), &catalog).unwrap();
        assert_eq!(generated.hook_count, fragments.len());
        let report: serde_json::Value = serde_json::from_str(&generated.coverage_json).unwrap();
        assert_eq!(report["hooks"].as_array().unwrap().len(), fragments.len());
        assert_eq!(
            generated.fully_typed + generated.partial + generated.with_handles,
            generated.hook_count
        );
        for hook in &fragments {
            let name = ident(hook["name"].as_str().unwrap());
            assert!(generated.wit.contains(&format!("    {name}: func(")), "missing {name}");
        }
    }

    #[test]
    fn handles_and_contracts_are_documented() {
        let generated = generated();
        assert!(generated.with_handles > 0);
        assert!(generated.wit.contains("opaque engine handle ("));
        assert!(generated.wit.contains("world client-full {"));
        assert!(generated.wit.contains("world server-full {"));
    }
}

