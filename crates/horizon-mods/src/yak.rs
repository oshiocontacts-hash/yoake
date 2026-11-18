//! Yoake YAK v1 packages. Nothing here loads code or changes vanilla mod discovery.

mod catalog;
mod compatibility;
mod config;
mod fs;
mod http_policy;
mod installed;
mod manifest;
mod pack;
mod permissions;

pub use catalog::{CATALOG_PATH, Catalog, CatalogEntry, CatalogVersion, MAX_CATALOG_SIZE};
pub use compatibility::{
    CompatibilityIssue, Environment, Resolution, ResolutionIssue, check_compatibility,
    check_data_compatibility, resolve,
};
pub use config::{
    ConfigChange, ConfigField, ConfigFile, ConfigKind, ConfigRuntimeSide, ConfigSide, ConfigState,
    PermissionNode, has_permission, parse_config, resolve_config, serialize_config,
};
pub use fs::Dir as ServiceDirectory;
pub use horizon_release_sign::yak::{
    Entry, MAX_ARCHIVE_SIZE, MAX_COMPRESSION_RATIO, MAX_ENTRIES, MAX_FILE_SIZE, MAX_TOTAL_SIZE,
    Signature, canonical_hashes,
};
pub use horizon_release_sign::{PublicKey, parse_keys};
pub use http_policy::{HttpsPolicy, validate_https_addresses};
pub use installed::{
    check_installed_tree, check_plain_path, hash_installed_file, trusted_keys, verify_installed,
};
pub use manifest::{
    Bin, FULL_ACCESS_WORLD, Http, MAX_WASM_SIZE, Manifest, Mixin, OptionValue, SUPPORTED_WASM_INTERFACE, Service, Shader,
    ShaderOption, Side, Target, Wasm, Yoake,
};
pub use pack::pack_directory;
pub use permissions::{PermissionOverrides, normalize_uuid};
pub use semver::{Version, VersionReq};

use horizon_release_sign::yak as container;
use std::collections::BTreeMap;
use std::fs::OpenOptions;
use std::io::{Read, Seek, SeekFrom};
use std::path::Path;

#[derive(Clone, Debug, Eq, Ord, PartialEq, PartialOrd)]
pub enum Part {
    Manifest,
    Bin(Target),
    Wasm,
    Content,
    Assets,
    Shaders(String),
    Config,
    Metadata,
    Signature,
}

#[derive(Clone, Debug)]
pub struct Summary {
    pub manifest: Manifest,
    pub files: usize,
    pub uncompressed_size: u64,
    pub signed: bool,
}

pub struct Archive<Reader: Read + Seek> {
    container: container::Archive<Reader>,
    manifest: Manifest,
    parts: BTreeMap<String, Part>,
}

impl<Reader: Read + Seek> Archive<Reader> {
    /// Fully scans the bounded ZIP, validates the manifest and checks any embedded hash list.
    /// Authenticity is a separate, explicit caller-pinned signature or catalog check.
    pub fn new(reader: Reader) -> Result<Self, String> {
        let mut container = container::Archive::new(reader)?;
        let bytes = container.read_small(container::MANIFEST, container::MAX_MANIFEST_SIZE)?;
        let text = std::str::from_utf8(&bytes).map_err(|_| "yak.toml is not UTF-8")?;
        let manifest = Manifest::parse(text)?;
        let mut parts = BTreeMap::new();
        for entry in container.entries() {
            let part = classify(&entry.name, &manifest)?;
            parts.insert(entry.name.clone(), part);
        }
        if let Some(bin) = &manifest.bin {
            for target in &bin.targets {
                let library = format!("bin/{target}/{}", target.library_filename(&bin.library));
                if !parts.contains_key(&library) {
                    return Err(format!("missing declared binary: {library}"));
                }
            }
        }
        if let Some(wasm) = &manifest.wasm {
            for (side, path) in wasm.modules() {
                let entry = container.entries().iter().find(|entry| entry.name == path)
                    .ok_or_else(|| format!("missing declared wasm {side} module: {path}"))?;
                if entry.size > MAX_WASM_SIZE {
                    return Err(format!("wasm {side} module {path} exceeds 32 MiB"));
                }
            }
        }
        for shader in &manifest.shaders {
            if !parts.values().any(|part| part == &Part::Shaders(shader.id.clone())) {
                return Err(format!("missing declared shader pack: {}", shader.id));
            }
        }
        Ok(Self { container, manifest, parts })
    }

    /// Use only with a version obtained from a caller-pinned, verified catalog.
    /// Authenticates the complete download, including unsigned packages, before parsing it.
    pub fn from_catalog(mut reader: Reader, version: &CatalogVersion) -> Result<Self, String> {
        reader.seek(SeekFrom::Start(0)).map_err(|error| error.to_string())?;
        version.verify_download(&mut reader)?;
        reader.seek(SeekFrom::Start(0)).map_err(|error| error.to_string())?;
        let archive = Self::new(reader)?;
        let actual = serde_json::to_value(archive.manifest()).map_err(|error| error.to_string())?;
        let expected = serde_json::to_value(&version.manifest).map_err(|error| error.to_string())?;
        if actual != expected {
            return Err("catalog manifest summary differs from yak.toml".into());
        }
        Ok(archive)
    }

    pub fn manifest(&self) -> &Manifest {
        &self.manifest
    }

    pub fn entries(&self) -> &[Entry] {
        self.container.entries()
    }

    pub fn hashes(&self) -> &BTreeMap<String, String> {
        self.container.hashes()
    }

    pub fn canonical_hash_list(&self) -> String {
        canonical_hashes(self.hashes())
    }

    /// The bytes of a full-access mod's core module (`[wasm] world = "full-access"`), or `None` for any other
    /// package. Reading is bounded by [`MAX_WASM_SIZE`]; nothing is run or verified here.
    pub fn full_access_module(&mut self) -> Result<Option<Vec<u8>>, String> {
        let Some(path) = self
            .manifest
            .wasm
            .as_ref()
            .filter(|wasm| wasm.full_access())
            .and_then(|wasm| wasm.server.clone())
        else {
            return Ok(None);
        };
        self.container.read_small(&path, MAX_WASM_SIZE).map(Some)
    }

    pub fn signature(&self) -> Option<&Signature> {
        self.container.signature()
    }

    pub fn verify_signature(&self, keys: &[PublicKey]) -> Result<Option<PublicKey>, String> {
        self.container.verify_signature(keys)
    }

    pub fn files<'a>(&'a self, part: &'a Part) -> impl Iterator<Item = &'a Entry> + 'a {
        self.entries().iter().filter(move |entry| self.parts.get(&entry.name) == Some(part))
    }

    pub fn parts(&self) -> BTreeMap<Part, Vec<&Entry>> {
        let mut parts: BTreeMap<Part, Vec<&Entry>> = BTreeMap::new();
        for entry in self.entries() {
            parts.entry(self.parts[&entry.name].clone()).or_default().push(entry);
        }
        parts
    }

    pub fn summary(&self) -> Summary {
        Summary {
            manifest: self.manifest.clone(),
            files: self.entries().len(),
            uncompressed_size: self.entries().iter().map(|entry| entry.size).sum(),
            signed: self.signature().is_some(),
        }
    }

    /// An unsigned package is permitted, but is not authenticated by this operation.
    /// Signed packages require a caller-pinned key. The destination must not exist,
    /// and the caller must exclusively control its parent during this operation.
    pub fn extract_target(&mut self, destination: &Path, target: Target, keys: &[PublicKey]) -> Result<(), String> {
        self.verify_signature(keys)?;
        if self.manifest.bin.as_ref().is_some_and(|bin| !bin.targets.contains(&target)) {
            return Err(format!("YAK has no binary for {target}"));
        }
        let names: Vec<_> = self.entries().iter().filter(|entry| {
            !matches!(self.parts.get(&entry.name), Some(Part::Bin(other)) if *other != target)
        }).map(|entry| entry.name.clone()).collect();
        container::atomic_directory(destination, |staging| {
            for name in names {
                let path = staging.join(&name);
                let parent = path.parent().ok_or("extracted file has no parent")?;
                container::check_plain_path(parent)?;
                std::fs::create_dir_all(parent).map_err(|error| error.to_string())?;
                let mut file = OpenOptions::new().write(true).create_new(true).open(&path)
                    .map_err(|error| error.to_string())?;
                self.container.copy_file(&name, &mut file)?;
                file.sync_all().map_err(|error| error.to_string())?;
            }
            Ok(())
        })
    }
}

fn classify(name: &str, manifest: &Manifest) -> Result<Part, String> {
    match name {
        container::MANIFEST => return Ok(Part::Manifest),
        container::SIGNATURE => return Ok(Part::Signature),
        "LICENSE" | "README.md" => return Ok(Part::Metadata),
        _ => (),
    }
    let (root, rest) = name.split_once('/').ok_or_else(|| format!("unknown YAK root file: {name}"))?;
    match root {
        "content" => Ok(Part::Content),
        "assets" => Ok(Part::Assets),
        "config" => Ok(Part::Config),
        "bin" if rest.starts_with("wasm/") => {
            let wasm = manifest.wasm.as_ref().ok_or("bin/wasm files require [wasm]")?;
            if !wasm.modules().iter().any(|(_, path)| *path == name) {
                return Err(format!("undeclared wasm module: {name}"));
            }
            Ok(Part::Wasm)
        }
        "bin" => {
            let (target, _) = rest.split_once('/').ok_or("bin files need a target directory")?;
            let target: Target = target.parse()?;
            let bin = manifest.bin.as_ref().ok_or("bin files require [bin]")?;
            if !bin.targets.contains(&target) {
                return Err(format!("undeclared binary target: {target}"));
            }
            Ok(Part::Bin(target))
        }
        "shaders" => {
            let (id, _) = rest.split_once('/').ok_or("shader files need a pack-id directory")?;
            if !manifest.shaders.iter().any(|shader| shader.id == id) {
                return Err(format!("undeclared shader pack: {id}"));
            }
            Ok(Part::Shaders(id.to_owned()))
        }
        _ => Err(format!("unknown YAK part: {root}")),
    }
}
