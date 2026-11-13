//! The game's installed-.yak side as the DLL loader sees it (`horizon-client/src/yak_mods.rs`):
//! no `yoake-mods.txt` in these tests, so no entries; the loader's .yak path is tested in
//! horizon-client itself.
use horizon_mods::yak::{Manifest, PublicKey};
use std::path::PathBuf;

pub(crate) struct Installed {
    pub(crate) manifest: Manifest,
}

pub(crate) struct Build;

impl Build {
    pub(crate) fn current() -> Result<Self, String> {
        Ok(Self)
    }
}

pub(crate) fn initialize() {}

pub(crate) fn entries() -> &'static [Installed] {
    &[]
}

pub(crate) fn refuse_binary(_entry: &Installed, _reason: &str) {}

pub(crate) fn binary_preflight(
    _entry: &Installed,
    _build: &Build,
    _keys: &[PublicKey],
    _allow_unsigned: bool,
) -> Result<(PathBuf, String), String> {
    Err("no installed .yak in the loader tests".into())
}
