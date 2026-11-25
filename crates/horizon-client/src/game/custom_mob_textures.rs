//! Client game wiring for F4M-7 runtime custom mob textures.

use super::Game;
use horizon_client::mobs::custom::{
    registry,
    textures::{self, MobTextureOverrides},
};

impl Game<'_> {
    /// Collects pending dirty-rect uploads for runtime custom mob textures into `self.sprite_patches`.
    pub(super) fn flush_custom_mob_textures(&mut self) {
        let Some(region) = self.gui.atlas.mob_texture_region() else {
            return;
        };
        let atlas_size = [self.gui.atlas.width, self.gui.atlas.height];
        textures::flush_patches(region, atlas_size, &mut self.sprite_patches);
    }

    /// Marks every active runtime custom mob texture dirty so the rebuilt atlas receives them again.
    pub(super) fn reapply_custom_mob_textures(&mut self) {
        textures::mark_all_dirty();
        self.flush_custom_mob_textures();
    }

    /// Returns the atlas UV overrides currently bound to custom mob entity `entity`.
    pub(super) fn custom_mob_texture_overrides(&self, entity: u32) -> Option<MobTextureOverrides> {
        let region = self.gui.atlas.mob_texture_region()?;
        let atlas_size = [self.gui.atlas.width, self.gui.atlas.height];
        textures::overrides_for(entity, region, atlas_size)
    }
}

pub(super) fn should_reserve_mob_textures() -> bool {
    custom_mob_textures_enabled()
}

fn custom_mob_textures_enabled() -> bool {
    let has_custom_mobs = registry::registry()
        .read()
        .map(|r| !r.is_empty())
        .unwrap_or(false);
    #[cfg(feature = "yoake2-slice")]
    if crate::yoake2_mods::client_mods_loaded() {
        return true;
    }
    has_custom_mobs || crate::entry::wasm_mods::mob_services::any_granted() || !textures::is_empty()
}
