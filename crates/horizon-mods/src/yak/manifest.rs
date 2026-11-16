use semver::{Version, VersionReq};
use serde::{Deserialize, Serialize};
use std::collections::{BTreeMap, BTreeSet};
use std::fmt;
use std::fs::File;
use std::io::Read;
use std::path::Path;
use std::str::FromStr;
use url::Url;

use super::config::{ConfigField, PermissionNode, validate_declarations};
use super::http_policy::HttpsPolicy;

#[derive(Clone, Copy, Debug, Deserialize, Eq, Ord, PartialEq, PartialOrd, Serialize)]
pub enum Target {
    #[serde(rename = "windows-x64")]
    WindowsX64,
    #[serde(rename = "linux-x64")]
    LinuxX64,
    #[serde(rename = "macos-arm64")]
    MacosArm64,
    #[serde(rename = "macos-x64")]
    MacosX64,
}

impl Target {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::WindowsX64 => "windows-x64",
            Self::LinuxX64 => "linux-x64",
            Self::MacosArm64 => "macos-arm64",
            Self::MacosX64 => "macos-x64",
        }
    }

    pub fn library_filename(self, library: &str) -> String {
        match self {
            Self::WindowsX64 => format!("{library}.dll"),
            Self::LinuxX64 => format!("lib{library}.so"),
            Self::MacosArm64 | Self::MacosX64 => format!("lib{library}.dylib"),
        }
    }

    pub fn host() -> Result<Self, String> {
        match (std::env::consts::OS, std::env::consts::ARCH) {
            ("windows", "x86_64") => Ok(Self::WindowsX64),
            ("linux", "x86_64") => Ok(Self::LinuxX64),
            ("macos", "aarch64") => Ok(Self::MacosArm64),
            ("macos", "x86_64") => Ok(Self::MacosX64),
            _ => Err("this host has no YAK v1 binary target".into()),
        }
    }
}

impl fmt::Display for Target {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(self.as_str())
    }
}

impl FromStr for Target {
    type Err = String;

    fn from_str(value: &str) -> Result<Self, String> {
        match value {
            "windows-x64" => Ok(Self::WindowsX64),
            "linux-x64" => Ok(Self::LinuxX64),
            "macos-arm64" => Ok(Self::MacosArm64),
            "macos-x64" => Ok(Self::MacosX64),
            _ => Err(format!("unsupported YAK target: {value}")),
        }
    }
}

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct Manifest {
    pub format: u32,
    pub id: String,
    pub version: Version,
    pub name: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub name_ja: Option<String>,
    pub authors: Vec<String>,
    pub description: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub license: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub homepage: Option<String>,
    pub yoake: Yoake,
    pub side: Side,
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub dependencies: BTreeMap<String, VersionReq>,
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub optional: BTreeMap<String, VersionReq>,
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub conflicts: BTreeMap<String, VersionReq>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub bin: Option<Bin>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub wasm: Option<Wasm>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub http: Option<Http>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub mixin: Option<Mixin>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub shaders: Vec<Shader>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub services: Vec<Service>,
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub config: BTreeMap<String, ConfigField>,
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub permissions: BTreeMap<String, PermissionNode>,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct Yoake {
    pub version: VersionReq,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct Side {
    pub client: bool,
    pub server: bool,
    pub required_on_join: bool,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct Bin {
    pub library: String,
    pub targets: Vec<Target>,
    pub engine: String,
    pub abi: u32,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct Http {
    pub hosts: Vec<String>,
}

#[derive(Clone, Debug, Deserialize, Serialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct Mixin {
    pub targets: Vec<String>,
}

/// Highest `yoake:mods` WebAssembly interface (major, minor) this build understands.
pub const SUPPORTED_WASM_INTERFACE: (u32, u32) = (1, 1);
/// Largest accepted `.wasm` module, in bytes.
pub const MAX_WASM_SIZE: u64 = 32 * 1024 * 1024;

#[derive(Clone, Debug, Deserialize, Serialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct Wasm {
    pub interface: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub client: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub server: Option<String>,
    /// The WIT world: `client-mod` (the frozen 19 hooks, the default), `client-full` (the
    /// client-side hook catalog), or `inject-mod` (injections on either side). Ordinary server
    /// hooks target `server-full`; hook worlds use interface 1.0 and injections use 1.1.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub world: Option<String>,
    /// Powers beyond the safe set that the package asks for; an installer shows them.
    /// `service.commands` permits command services and sensitive injections;
    /// `service.chat` additionally permits client chat/command injections.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub permissions: Vec<String>,
}

/// The `[wasm] permissions` this build knows.
pub const WASM_PERMISSIONS: [&str; 7] = [
    "service.chat",
    "service.commands",
    "service.config",
    "service.gpu",
    "service.http",
    "service.permissions",
    "service.storage",
];

/// The WIT worlds a client module may target.
pub const CLIENT_WASM_WORLDS: [&str; 3] = ["client-mod", "client-full", "inject-mod"];

/// Whether `[wasm] world` selects the full-catalog client world.
impl Wasm {
    pub fn client_full(&self) -> bool {
        self.world.as_deref() == Some("client-full")
    }

    /// Whether the package asked for `permission` (see [`WASM_PERMISSIONS`]).
    pub fn has_permission(&self, permission: &str) -> bool {
        self.permissions.iter().any(|asked| asked == permission)
    }
}

impl Wasm {
    /// Declared `(major, minor)` of the `yoake:mods` interface.
    pub fn interface_version(&self) -> Result<(u32, u32), String> {
        let (major, minor) = self
            .interface
            .split_once('.')
            .ok_or("wasm.interface must be major.minor, for example 1.0")?;
        let number = |part: &str| -> Result<u32, String> {
            if part.is_empty() || part.len() > 6 || !part.bytes().all(|byte| byte.is_ascii_digit())
            {
                return Err("wasm.interface must be major.minor, for example 1.0".into());
            }
            part.parse()
                .map_err(|_| "wasm.interface must be major.minor, for example 1.0".to_owned())
        };
        Ok((number(major)?, number(minor)?))
    }

    /// Declared module paths with their side label, client first.
    pub fn modules(&self) -> Vec<(&'static str, &str)> {
        let mut modules = Vec::new();
        if let Some(path) = &self.client {
            modules.push(("client", path.as_str()));
        }
        if let Some(path) = &self.server {
            modules.push(("server", path.as_str()));
        }
        modules
    }

    fn validate(&self) -> Result<(), String> {
        let (major, minor) = self.interface_version()?;
        if major != SUPPORTED_WASM_INTERFACE.0 {
            return Err(format!(
                "unsupported wasm interface major version {major} (this build supports {}.x)",
                SUPPORTED_WASM_INTERFACE.0
            ));
        }
        if minor > SUPPORTED_WASM_INTERFACE.1 {
            return Err(format!(
                "wasm interface {major}.{minor} is newer than the supported {}.{}",
                SUPPORTED_WASM_INTERFACE.0, SUPPORTED_WASM_INTERFACE.1
            ));
        }
        if minor == 1 && self.world.as_deref() != Some("inject-mod") {
            return Err("wasm interface 1.1 requires wasm.world = \"inject-mod\"; ordinary hook worlds use 1.0".into());
        }
        if let Some(world) = &self.world {
            if !CLIENT_WASM_WORLDS.contains(&world.as_str()) {
                return Err(format!(
                    "unknown wasm.world {world:?} (known: {})",
                    CLIENT_WASM_WORLDS.join(", ")
                ));
            }
            if self.client.is_none() && world != "inject-mod" {
                return Err("wasm.world applies to the client module; declare wasm.client".into());
            }
        }
        for permission in &self.permissions {
            if !WASM_PERMISSIONS.contains(&permission.as_str()) {
                return Err(format!(
                    "unknown wasm permission {permission:?} (known: {})",
                    WASM_PERMISSIONS.join(", ")
                ));
            }
        }
        if self
            .permissions
            .iter()
            .any(|permission| permission == "service.commands")
            && self.server.is_none()
            && self.world.as_deref() != Some("inject-mod")
        {
            return Err("wasm permission service.commands applies to the server module; declare wasm.server".into());
        }
        if self.has_permission("service.permissions") && self.server.is_none() {
            return Err("wasm permission service.permissions applies to the server module; declare wasm.server".into());
        }
        if self.has_permission("service.chat") && self.client.is_none() {
            return Err("wasm permission service.chat requires wasm.client".into());
        }
        let modules = self.modules();
        if modules.is_empty() {
            return Err("[wasm] needs at least one of client or server".into());
        }
        for (side, path) in &modules {
            horizon_release_sign::yak::check_path(path, false)
                .map_err(|error| format!("wasm.{side}: {error}"))?;
            let name = path.strip_prefix("bin/wasm/").filter(|name| {
                !name.is_empty()
                    && !name.contains('/')
                    && name.ends_with(".wasm")
                    && name.len() > ".wasm".len()
            });
            if name.is_none() {
                return Err(format!(
                    "wasm.{side} must be a file directly under bin/wasm/ ending in .wasm"
                ));
            }
        }
        Ok(())
    }
}

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct Shader {
    pub id: String,
    pub name: String,
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub options: BTreeMap<String, ShaderOption>,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct ShaderOption {
    pub values: Vec<OptionValue>,
    pub default: OptionValue,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub label_ja: Option<String>,
}

#[derive(Clone, Debug, Deserialize, PartialEq, Serialize)]
#[serde(untagged)]
pub enum OptionValue {
    Boolean(bool),
    Integer(i64),
    Float(f64),
    Text(String),
}

impl OptionValue {
    fn kind(&self) -> u8 {
        match self {
            Self::Boolean(_) => 0,
            Self::Integer(_) => 1,
            Self::Float(_) => 2,
            Self::Text(_) => 3,
        }
    }

    fn validate(&self) -> Result<(), String> {
        match self {
            Self::Float(value) if !value.is_finite() => Err("shader numbers must be finite".into()),
            Self::Text(value) => text(value, "shader option value", 1024),
            _ => Ok(()),
        }
    }
}

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct Service {
    pub id: String,
    pub name: String,
    pub required: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub default_url: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub help_url: Option<String>,
}

impl Manifest {
    pub fn read(path: &Path) -> Result<Self, String> {
        horizon_release_sign::yak::check_plain_path(path)?;
        let metadata = std::fs::symlink_metadata(path)
            .map_err(|error| format!("{}: {error}", path.display()))?;
        let limit = horizon_release_sign::yak::MAX_MANIFEST_SIZE;
        if !metadata.is_file() || metadata.len() > limit {
            return Err("yak.toml must be a regular file no larger than 1 MiB".into());
        }
        let file = File::open(path).map_err(|error| format!("yak.toml: {error}"))?;
        if !file
            .metadata()
            .map_err(|error| error.to_string())?
            .is_file()
        {
            return Err("yak.toml is not a regular file".into());
        }
        let mut source = String::new();
        file.take(limit + 1)
            .read_to_string(&mut source)
            .map_err(|error| format!("yak.toml: {error}"))?;
        Self::parse(&source)
    }

    pub fn parse(source: &str) -> Result<Self, String> {
        if source.len() as u64 > horizon_release_sign::yak::MAX_MANIFEST_SIZE {
            return Err("yak.toml exceeds 1 MiB".into());
        }
        let document: toml::Value =
            toml::from_str(source).map_err(|error| format!("yak.toml: {error}"))?;
        if let Some(services) = document.get("services") {
            reject_secrets(services)?;
        }
        let manifest: Self =
            toml::from_str(source).map_err(|error| format!("yak.toml: {error}"))?;
        manifest.validate()?;
        Ok(manifest)
    }

    pub fn validate(&self) -> Result<(), String> {
        if self.format != 1 {
            return Err(format!("unsupported YAK format {}", self.format));
        }
        identifier(&self.id)?;
        text(&self.name, "name", 256)?;
        text(&self.description, "description", 16384)?;
        if self.authors.is_empty() || self.authors.len() > 128 {
            return Err("authors must contain 1..128 names".into());
        }
        for author in &self.authors {
            text(author, "author", 256)?;
        }
        for (field, value) in [("name_ja", &self.name_ja), ("license", &self.license)] {
            if let Some(value) = value {
                text(value, field, 256)?;
            }
        }
        if let Some(homepage) = &self.homepage {
            web_url(homepage, false)?;
        }
        if !self.side.client && !self.side.server {
            return Err("at least one side must be enabled".into());
        }
        validate_declarations(&self.config, &self.permissions)?;
        let http_requested = self.wasm.as_ref().is_some_and(|wasm| wasm.has_permission("service.http"));
        match (&self.http, http_requested) {
            (Some(http), true) => { HttpsPolicy::new(&http.hosts)?; }
            (Some(_), false) => return Err("[http] requires wasm permission service.http".into()),
            (None, true) => return Err("service.http requires [http] hosts".into()),
            (None, false) => {}
        }
        for requirements in [&self.dependencies, &self.optional, &self.conflicts] {
            if requirements.len() > 1024 {
                return Err("too many dependency constraints".into());
            }
            for id in requirements.keys() {
                identifier(id)?;
                if id == &self.id {
                    return Err("a mod cannot depend on or conflict with itself".into());
                }
            }
        }
        if self
            .dependencies
            .keys()
            .any(|id| self.optional.contains_key(id))
        {
            return Err("a dependency cannot be both required and optional".into());
        }
        if let Some(bin) = &self.bin {
            if bin.library.is_empty()
                || bin.library.len() > 64
                || !bin
                    .library
                    .bytes()
                    .all(|byte| byte.is_ascii_alphanumeric() || byte == b'_' || byte == b'-')
            {
                return Err("bin.library must be a plain library basename".into());
            }
            if bin.targets.is_empty()
                || bin.targets.len() != bin.targets.iter().collect::<BTreeSet<_>>().len()
            {
                return Err("bin.targets must contain unique supported targets".into());
            }
            text(&bin.engine, "bin.engine", 256)?;
            Version::parse(&bin.engine)
                .map_err(|error| format!("bin.engine fingerprint: {error}"))?;
            if bin.abi == 0 {
                return Err("bin.abi must be positive".into());
            }
        }
        if let Some(mixin) = &self.mixin {
            if mixin.targets.is_empty() || mixin.targets.len() > 1024 {
                return Err("mixin.targets must contain 1..1024 site IDs".into());
            }
            let mut targets = BTreeSet::new();
            for target in &mixin.targets {
                let Some((path, hash)) = target.split_once('#') else {
                    return Err("mixin target must include a signature hash".into());
                };
                if path.is_empty()
                    || target.len() > 1024
                    || hash.len() != 64
                    || !hash
                        .bytes()
                        .all(|byte| byte.is_ascii_digit() || (b'a'..=b'f').contains(&byte))
                {
                    return Err("invalid mixin target or signature hash".into());
                }
                if !targets.insert(target) {
                    return Err("mixin.targets must be unique".into());
                }
                let site = yoake_inject_rt::catalog::SITES
                    .iter()
                    .find(|site| site.path() == path)
                    .ok_or_else(|| {
                        format!("unknown mixin target {path:?}; this engine does not expose it")
                    })?;
                if site.signature_hash() != hash {
                    return Err(format!(
                        "mixin signature mismatch for {path}: expected {}",
                        site.id()
                    ));
                }
                let wasm = self.wasm.as_ref().ok_or("mixin requires a [wasm] module")?;
                let has_module = match site.side() {
                    yoake_inject_rt::Side::Client => wasm.client.is_some(),
                    yoake_inject_rt::Side::Server => wasm.server.is_some(),
                    yoake_inject_rt::Side::Both => wasm.client.is_some() || wasm.server.is_some(),
                    yoake_inject_rt::Side::Unspecified => {
                        return Err(format!("mixin target {path} has no execution side"));
                    }
                };
                if !has_module {
                    return Err(format!(
                        "mixin target {path} requires a {:?} wasm module",
                        site.side()
                    ));
                }
            }
            let wasm = self.wasm.as_ref().ok_or("mixin requires a [wasm] module")?;
            if wasm.world.as_deref() != Some("inject-mod") || wasm.interface_version()? != (1, 1) {
                return Err(
                    "mixin requires wasm.world = \"inject-mod\" and wasm.interface = \"1.1\""
                        .into(),
                );
            }
        }
        if let Some(wasm) = &self.wasm {
            if self.bin.is_some() {
                return Err(
                    "a package may contain native code ([bin]) or WebAssembly ([wasm]), not both"
                        .into(),
                );
            }
            wasm.validate()?;
            if wasm.world.as_deref() == Some("inject-mod") && self.mixin.is_none() {
                return Err("wasm.world inject-mod requires [mixin] targets".into());
            }
        }
        if self.shaders.len() > 128 || self.services.len() > 128 {
            return Err("too many shaders or services".into());
        }
        let mut ids = BTreeSet::new();
        for shader in &self.shaders {
            identifier(&shader.id)?;
            text(&shader.name, "shader name", 256)?;
            if !ids.insert(&shader.id) {
                return Err(format!("duplicate shader {}", shader.id));
            }
            if shader.options.len() > 256 {
                return Err("too many shader options".into());
            }
            for (name, option) in &shader.options {
                text(name, "shader option name", 128)?;
                if option.values.is_empty() || option.values.len() > 256 {
                    return Err("shader options require 1..256 allowed values".into());
                }
                option.default.validate()?;
                for (position, value) in option.values.iter().enumerate() {
                    value.validate()?;
                    if value.kind() != option.default.kind()
                        || option.values[..position].contains(value)
                    {
                        return Err(
                            "shader option values must be unique and have the default's type"
                                .into(),
                        );
                    }
                }
                if !option.values.contains(&option.default) {
                    return Err("shader option default must be an allowed value".into());
                }
                if let Some(label) = &option.label_ja {
                    text(label, "shader option label_ja", 256)?;
                }
            }
        }
        ids.clear();
        for service in &self.services {
            identifier(&service.id)?;
            text(&service.name, "service name", 256)?;
            if !ids.insert(&service.id) {
                return Err(format!("duplicate service {}", service.id));
            }
            if let Some(url) = &service.default_url {
                web_url(url, false)?;
            }
            if let Some(url) = &service.help_url {
                web_url(url, false)?;
            }
        }
        Ok(())
    }
}

pub(crate) fn identifier(value: &str) -> Result<(), String> {
    if !(3..=48).contains(&value.len())
        || !value.bytes().all(|byte| {
            byte.is_ascii_lowercase() || byte.is_ascii_digit() || byte == b'_' || byte == b'-'
        })
    {
        return Err(format!("invalid YAK identifier: {value}"));
    }
    Ok(())
}

fn text(value: &str, field: &str, limit: usize) -> Result<(), String> {
    if value.trim().is_empty()
        || value.len() > limit
        || value
            .chars()
            .any(|character| character.is_control() && character != '\n' && character != '\t')
    {
        return Err(format!(
            "{field} is empty, too long, or contains control characters"
        ));
    }
    Ok(())
}

pub(crate) fn web_url(value: &str, https_only: bool) -> Result<Url, String> {
    if value.len() > 4096 {
        return Err("URL is too long".into());
    }
    let url = Url::parse(value).map_err(|error| format!("invalid URL: {error}"))?;
    if !(url.scheme() == "https" || (!https_only && url.scheme() == "http"))
        || url.host_str().is_none()
        || !url.username().is_empty()
        || url.password().is_some()
        || url.query().is_some()
        || url.fragment().is_some()
    {
        return Err(
            "URLs must be HTTP(S), with a host and no credentials, query, or fragment".into(),
        );
    }
    Ok(url)
}

fn reject_secrets(value: &toml::Value) -> Result<(), String> {
    match value {
        toml::Value::Table(table) => {
            for (key, value) in table {
                let normalized: String = key
                    .chars()
                    .filter(|character| character.is_ascii_alphanumeric())
                    .flat_map(char::to_lowercase)
                    .collect();
                if normalized == "key"
                    || [
                        "secret",
                        "token",
                        "password",
                        "passwd",
                        "passphrase",
                        "credential",
                        "authorization",
                        "apikey",
                        "privatekey",
                        "accesskey",
                        "bearer",
                    ]
                    .iter()
                    .any(|part| normalized.contains(*part))
                {
                    return Err(format!("secret-like service field is forbidden: {key}"));
                }
                reject_secrets(value)?;
            }
        }
        toml::Value::Array(values) => {
            for value in values {
                reject_secrets(value)?;
            }
        }
        _ => (),
    }
    Ok(())
}
