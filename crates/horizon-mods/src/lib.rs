//! Horizon's mod files (`docs/mods.md`): content mods (`.hmod`: blocks, items, data and
//! assets, no code), modpacks and, with `index`, the mod registry's index. Code mods are
//! native (docs/native-mods.md).
pub mod api_level;
#[cfg(test)]
#[path = "../../../tests/support/temp_dir.rs"]
mod test_support;
#[cfg(test)]
mod temp_dir_cleanup_tests;
pub mod content;
#[cfg(feature = "index")]
pub mod index;
pub mod manifest;
pub mod modpack;
#[cfg(feature = "yak")]
pub mod yak;

pub use content::Content;
pub use manifest::{Capability, Manifest, Package, Side, find};
