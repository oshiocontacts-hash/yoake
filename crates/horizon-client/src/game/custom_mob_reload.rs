//! Hot reload of custom mobs in the game (F4M-13): the R1 file watcher (once a second), R2
//! `/yoake mob reload [type]`, and bringing the live mobs, their render state and the animation
//! stores to the reloaded types. What is decided is in `mobs/custom/reload.rs` (no window needed).
//!
//! Developer mode only (`YOAKE_WASM_UNSIGNED=1`, an unsigned package folder). Without it nothing is
//! registered, so every entry point here returns on its first line. The swap itself happens in the
//! game tick, which runs between two frames: a frame never sees half a type.
use super::Game;
use horizon_client::mobs::custom::{self, reload};

/// What the game keeps of hot reload.
#[derive(Default)]
pub(super) struct CustomReload {
    /// The registry generation the live mobs were last brought to.
    synced: u64,
    /// A reload of ours is being applied: its mobs are announced in the log (a table another reload
    /// replaced is brought over quietly).
    announce: bool,
}

impl Game<'_> {
    /// Once per game tick: the watcher's poll once a second, then the mobs that still hold an older
    /// version of their type are brought to the registered one. Free in a game with no reloadable type
    /// that never reloaded.
    pub(super) fn tick_custom_reload(&mut self) {
        // Development only: no source is registered, and nothing is brought over, without the switch.
        if !reload::dev_enabled() {
            return;
        }
        let generation = reload::generation();
        let watching = reload::has_sources();
        if !watching && generation == self.custom_reload.synced {
            return;
        }
        // The window's game watches the files (one watcher for the process: the integrated server's game
        // only brings its mobs over); a texture change needs its renderer.
        if watching && self.renderer.is_some() && self.ticks.is_multiple_of(20) {
            for source in reload::watch_new_sources() {
                log::info!("Horizon custom-mob reload: watching {} in {}", source.id, source.dir.display());
            }
            for change in reload::watch_poll(false) {
                self.reload_changed(&change);
            }
        }
        // A reload (this one, or a resource reload that rebuilt the table) put new types in the
        // registry; and a mob spawned from the old table since is brought over too.
        if generation > 0 {
            self.bring_custom_mobs_current();
        }
        self.custom_reload.synced = reload::generation();
    }

    /// One poll's change of one type.
    fn reload_changed(&mut self, change: &reload::Change) {
        let Some(source) = reload::source_of(&change.id) else { return };
        let what = change.files.join(", ");
        if change.kinds & (reload::DATA | reload::MODEL) != 0 {
            match self.reload_source(&source) {
                Ok(()) => log::info!("Horizon custom-mob reload: {} reloaded ({what})", change.id),
                Err(why) => log::warn!(
                    "Horizon custom-mob reload: {} not reloaded, the old version keeps running ({what}): {why}",
                    change.id
                ),
            }
        }
        if change.kinds & reload::TEXTURE != 0 {
            // The textures are in the entity atlas: the resource reload (F3+T) stages the folder again.
            log::info!("Horizon custom-mob reload: {} textures changed ({what}), reloading resources", change.id);
            self.reload_packs();
        }
    }

    /// Rebuilds one type from its source and installs it; the watcher follows the new file list.
    fn reload_source(&mut self, source: &reload::Source) -> Result<(), String> {
        let new = reload::reload_type(source)?;
        reload::watch_again(source, &new.file);
        self.custom_reload.announce = true;
        self.bring_custom_mobs_current();
        self.custom_reload.announce = false;
        Ok(())
    }

    /// Brings the live mobs of every replaced type to the registered one: their kind, attributes, goals,
    /// hitbox switches, their render state (the pose buffers; the animation phase comes from the anchors)
    /// and the animation stores that hold the type.
    fn bring_custom_mobs_current(&mut self) {
        let synced = reload::sync_mobs(&mut self.mobs);
        for why in synced.refused {
            log::warn!("Horizon custom-mob reload: {why}");
            self.system_message(horizon_client::text::Text::literal(why.as_str()));
        }
        for (id, n) in synced.applied {
            let Some(ct) = custom::get(&id) else { continue };
            let ids: Vec<u32> = self.mobs.iter().filter(|m| m.kind.custom.as_ref().is_some_and(|c| c.id == id)).map(|m| m.id).collect();
            let render = self.custom_mob_render.get_mut();
            if let Some(tr) = ct.render.as_ref() {
                render.rebind(ids.iter().copied(), tr);
            }
            render.mirror.retype(&ct);
            if let Some(r) = self.remote.as_mut() {
                r.custom_mobs.retype(&ct);
            }
            if self.custom_reload.announce {
                log::info!("Horizon custom-mob reload: {id}: {n} mob(s) now use the new version");
            } else {
                log::debug!("Horizon custom-mob reload: {id}: {n} mob(s) brought to the registered type");
            }
        }
    }

    /// `/yoake mob reload [type]`: rebuilds the named type (every reloadable one without a name) from its
    /// folder now, and tells what happened to each.
    pub(super) fn yoake_mob(&mut self, args: &str) -> Vec<String> {
        let mut words = args.split_whitespace();
        if words.next() != Some("reload") {
            return vec![format!("Unknown option {args:?}: use /yoake mob reload [type]")];
        }
        let filter = words.next().map(|t| if t.contains(':') { t.to_owned() } else { format!("minecraft:{t}") });
        if words.next().is_some() {
            return vec!["Too many words: /yoake mob reload [type]".to_owned()];
        }
        if !reload::dev_enabled() {
            return vec!["Hot reload is off: start the game with YOAKE_WASM_UNSIGNED=1 and a custom mob package folder without a SIGNATURE".to_owned()];
        }
        let sources: Vec<reload::Source> = reload::sources().into_iter().filter(|s| filter.as_ref().is_none_or(|f| &s.id == f)).collect();
        if sources.is_empty() {
            return vec![match filter {
                Some(f) => format!("{f} has no folder to reload from (an installed signed package, or not a custom mob)"),
                None => "No custom mob type of an unsigned package folder is loaded".to_owned(),
            }];
        }
        for source in reload::watch_new_sources() {
            log::info!("Horizon custom-mob reload: watching {} in {}", source.id, source.dir.display());
        }
        // What changed since the last look (a texture goes through the resource reload).
        let changes = reload::watch_poll(true);
        let mut lines = Vec::new();
        for source in &sources {
            match self.reload_source(source) {
                Ok(()) => {
                    let n = self.mobs.iter().filter(|m| m.kind.custom.as_ref().is_some_and(|c| c.id == source.id)).count();
                    log::info!("Horizon custom-mob reload: {} reloaded by command", source.id);
                    lines.push(format!("Reloaded {} ({n} mob(s) now use it)", source.id));
                }
                Err(why) => {
                    log::warn!("Horizon custom-mob reload: {} not reloaded, the old version keeps running: {why}", source.id);
                    lines.push(format!("{} not reloaded, the old version keeps running: {why}", source.id));
                }
            }
        }
        if changes.iter().any(|c| c.kinds & reload::TEXTURE != 0 && sources.iter().any(|s| s.id == c.id)) {
            log::info!("Horizon custom-mob reload: textures changed, reloading resources");
            lines.push("Textures changed: reloading the resources (like F3+T)".to_owned());
            self.reload_packs();
        }
        self.custom_reload.synced = reload::generation();
        lines
    }
}
