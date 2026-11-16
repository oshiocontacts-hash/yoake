//! The game's installed-.yak side as the DLL loader sees it (`horizon-client/src/yak_mods.rs`):
//! no `yoake-mods.txt` in these tests, so no entries; the loader's .yak path is tested in
//! horizon-client itself.
use horizon_mods::yak::{self, Manifest, PublicKey, Target, Version};
use std::path::PathBuf;

pub(crate) struct Installed {
    pub(crate) root: PathBuf,
    pub(crate) manifest: Manifest,
}

impl Installed {
    pub(crate) fn new_for_test(root: PathBuf, manifest: Manifest) -> Self {
        Self { root, manifest }
    }
}

/// Same fields as the real `Build`, filled the same way.
pub(crate) struct Build {
    pub(crate) version: Version,
    pub(crate) engine: String,
    pub(crate) abi: u32,
    pub(crate) target: Option<Target>,
}

impl Build {
    pub(crate) fn current() -> Result<Self, String> {
        Ok(Self {
            version: Version::parse(horizon_mod_abi::YOAKE_VERSION).map_err(|error| error.to_string())?,
            engine: format!(
                "{}+build.{:016x}",
                horizon_mod_abi::YOAKE_VERSION,
                horizon_mod_abi::BUILD_FINGERPRINT
            ),
            abi: horizon_mod_abi::ABI_VERSION,
            target: Target::host().ok(),
        })
    }

    fn check(&self, manifest: &Manifest) -> Result<(), String> {
        let issues = if manifest.bin.is_some() {
            yak::check_compatibility(
                manifest,
                &yak::Environment {
                    yoake: &self.version,
                    engine: &self.engine,
                    abi: self.abi,
                    target: self.target.ok_or("unsupported native target")?,
                },
            )
        } else {
            yak::check_data_compatibility(manifest, &self.version)
        };
        if issues.is_empty() {
            Ok(())
        } else {
            Err(format!("incompatible build: {issues:?}"))
        }
    }
}

/// Same shape and rule as the real `UnsignedSwitches`.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) struct UnsignedSwitches {
    pub(crate) dll: bool,
    pub(crate) wasm: bool,
}

impl UnsignedSwitches {
    pub(crate) fn from_env() -> Self {
        let dll = std::env::var_os("YOAKE_DLL_MODS_UNSIGNED");
        let wasm = std::env::var_os("YOAKE_WASM_UNSIGNED");
        Self::from_values(dll.as_deref(), wasm.as_deref())
    }

    pub(crate) fn from_values(dll: Option<&std::ffi::OsStr>, wasm: Option<&std::ffi::OsStr>) -> Self {
        let dll = dll == Some(std::ffi::OsStr::new("1"));
        let wasm = wasm == Some(std::ffi::OsStr::new("1"));
        Self { dll, wasm: dll || wasm }
    }
}

/// Not a dedicated server in these tests.
pub(crate) fn dedicated() -> bool {
    false
}

pub(crate) fn initialize() {}

pub(crate) fn entries() -> &'static [Installed] {
    &[]
}

pub(crate) fn refuse_binary(_entry: &Installed, _reason: &str) {}

/// The real preflight (`horizon-client/src/yak_mods.rs`), so the loader tests see the same refusals.
pub(crate) fn binary_preflight(
    entry: &Installed,
    build: &Build,
    keys: &[PublicKey],
    allow_unsigned: bool,
) -> Result<(PathBuf, String), String> {
    yak::check_installed_tree(&entry.root)?;
    let manifest = Manifest::read(&entry.root.join("yak.toml"))?;
    if manifest.id != entry.manifest.id
        || manifest.version != entry.manifest.version
        || manifest.side.required_on_join != entry.manifest.side.required_on_join
    {
        return Err("YAK identity changed after discovery".into());
    }
    build.check(&manifest)?;
    if !manifest.side.client {
        return Err("server-only YAK".into());
    }
    let bin = manifest.bin.as_ref().ok_or("no binary declared")?;
    let target = build.target.ok_or("unsupported native target")?;
    let name = format!("bin/{target}/{}", target.library_filename(&bin.library));
    let digest = yak::hash_installed_file(&entry.root, &name)?;
    if let Err(error) = yak::verify_installed(&entry.root, &[&name], keys) {
        if !allow_unsigned {
            return Err(error);
        }
        log::warn!("YAK {}: developer unsigned override: {error}", manifest.id);
    }
    Ok((entry.root.join(name), digest))
}
