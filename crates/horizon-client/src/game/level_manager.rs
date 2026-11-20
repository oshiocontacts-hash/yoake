//! Creating and deleting levels while the server runs (F3-6).
//!
//! A created level is a stem like a data pack's or a mod's (`{"type": ..., "generator": ...}`) with
//! an id of its own. Persistent ones are listed in `data/yoake_levels.json` and come back when the
//! world opens; ephemeral ones save under `yoake_temp/` and are gone with the level or the world's
//! next opening. A level has its own folder (`dimensions/<ns>/<path>/`); a deleted one loses it.
//!
//! The protocol cannot add a level to a client that is already connected: the login packet's level
//! list and the dimension type registry are sent when it joins. So a created level is announced to
//! the hub (new joiners get its type in their registry and its key in the login list), clients
//! connected earlier can enter it when its type is one they have (a vanilla or pack type), and
//! the hub refuses to send one into a level whose type they lack until they reconnect
//! (`Hub::guest_knows_level`).
use super::{Dim, Game};
use horizon_client::level_registry::{LevelError, LevelSource};
use serde_json::Value;

/// Why a level could not be created or deleted.
#[derive(Clone, Debug, PartialEq)]
pub(super) enum LevelManagerError {
    Registry(LevelError),
    /// Players are in the level.
    Occupied(String),
    Io(String),
}
impl std::fmt::Display for LevelManagerError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Registry(e) => write!(f, "{e}"),
            Self::Occupied(id) => write!(f, "level `{id}` has players in it"),
            Self::Io(e) => write!(f, "{e}"),
        }
    }
}
impl From<LevelError> for LevelManagerError {
    fn from(e: LevelError) -> Self {
        Self::Registry(e)
    }
}

/// What `list_levels` says of a level.
#[derive(Clone, Debug, PartialEq)]
#[allow(dead_code)] // The mod API reaches it with F3-8.
pub(super) struct LevelInfo {
    pub id: String,
    pub source: LevelSource,
    pub ephemeral: bool,
    /// The level is loaded (in place or parked).
    pub open: bool,
}

impl Game<'_> {
    /// `horizon-ctl level ...` (the control socket, for tests and tools; players have no command for it):
    ///   level list
    ///   level create ID STEM_JSON [temp]     a stem without spaces, `temp` for an ephemeral level
    ///   level delete ID | unload ID | regenerate ID [SEED]
    /// An integrated server's levels are its server thread's: the call runs there.
    pub(super) fn level_control(
        &mut self,
        words: &[&str],
    ) -> horizon_client::Result<Option<serde_json::Value>> {
        if words.first() != Some(&"level") {
            return Ok(None);
        }
        let owned: Vec<String> = words.iter().map(|w| (*w).to_owned()).collect();
        let reply = match self.integrated.as_ref() {
            Some(server) => server
                .call(move |g| g.run_level_command(&owned))
                .ok_or(super::integrated::NO_ANSWER)?,
            None => self.run_level_command(&owned),
        };
        Ok(Some(reply?))
    }

    fn run_level_command(&mut self, words: &[String]) -> std::result::Result<serde_json::Value, String> {
        use serde_json::json;
        let arg = |i: usize| words.get(i).map(String::as_str).ok_or("level list|create|delete|unload|regenerate");
        let failed = |e: LevelManagerError| e.to_string();
        match arg(1)? {
            "list" => Ok(json!({"levels": self.list_levels().iter().map(|l| json!({
                "id": l.id, "source": format!("{:?}", l.source), "ephemeral": l.ephemeral, "open": l.open,
            })).collect::<Vec<_>>()})),
            "create" => {
                let stem: Value = serde_json::from_str(arg(3)?).map_err(|e| format!("the stem: {e}"))?;
                self.create_level(arg(2)?, stem, words.get(4).map(String::as_str) != Some("temp"))
                    .map(|d| json!({"ok": true, "level": d.id()}))
                    .map_err(failed)
            }
            "delete" => self.delete_level(arg(2)?).map(|()| json!({"ok": true})).map_err(failed),
            "unload" => self.unload_level(arg(2)?).map(|()| json!({"ok": true})).map_err(failed),
            "regenerate" => {
                let seed = words.get(3).map(|s| s.parse::<i64>()).transpose().map_err(|e| e.to_string())?;
                self.regenerate_level(arg(2)?, seed).map(|()| json!({"ok": true})).map_err(failed)
            }
            _ => Err("level list|create|delete|unload|regenerate".into()),
        }
    }
}

#[allow(dead_code)] // The mod API reaches these with F3-8.
impl Game<'_> {
    /// Creates a level from its stem while the world runs. `persistent` levels are written to
    /// `data/yoake_levels.json`; the others save under `yoake_temp/` and vanish with the world.
    pub(super) fn create_level(
        &mut self,
        id: &str,
        stem: Value,
        persistent: bool,
    ) -> Result<Dim, LevelManagerError> {
        let overrides = super::world_loading::world_worldgen_overrides(
            &self.world_root,
            &self.options.resources,
            self.res.biomes(),
            &self.level.enabled_features,
        );
        let data = horizon_client::vanillagen::shared_data_with(
            &self.options.resources.join("worldgen"),
            &overrides,
            self.pack.assets.registry,
        )
        .map_err(|e| LevelError::Unsupported {
            id: id.to_owned(),
            reason: format!("the worldgen data is not available: {e}"),
        })?;
        self.level_registry
            .create_level(id, stem, data.source_catalog(), !persistent)?;
        if persistent && let Err(e) = self.level_registry.write_created(&self.world_root) {
            let _ = self.level_registry.remove_level(id);
            return Err(LevelManagerError::Io(format!("data/yoake_levels.json: {e}")));
        }
        // New joiners are sent the level's type and key; connected clients see it when they can.
        if let Some(net) = self
            .level_registry
            .network_levels()
            .into_iter()
            .find(|l| l.id == id)
            && let Some(host) = self.lan.as_ref()
        {
            host.hub.sync.announce_level(net);
        }
        Ok(Dim::intern(id))
    }

    /// Deletes a created level: closes it, removes its folder, takes it out of the registry and of
    /// `data/yoake_levels.json`, and tells the hub. Refused while anyone is in it; the three
    /// vanilla levels and the levels of packs, the save and mods are not deletable.
    pub(super) fn delete_level(&mut self, id: &str) -> Result<(), LevelManagerError> {
        let Some(entry) = self.level_registry.get(id) else {
            return Err(LevelError::Unknown(id.to_owned()).into());
        };
        match entry.source {
            LevelSource::Vanilla => return Err(LevelError::Vanilla(id.to_owned()).into()),
            LevelSource::Created => {}
            _ => return Err(LevelError::NotCreated(id.to_owned()).into()),
        }
        let dim = Dim::intern(id);
        if self.is_occupied(dim, id) {
            return Err(LevelManagerError::Occupied(id.to_owned()));
        }
        let dir = self.level_registry.save_dir(&self.world_root, id);
        // Closing the level releases its region files before they are removed.
        self.level_before_close(id);
        self.levels.drop_parked(dim);
        self.level_ctxs.remove(&dim);
        match std::fs::remove_dir_all(&dir) {
            Err(e) if e.kind() != std::io::ErrorKind::NotFound => {
                return Err(LevelManagerError::Io(format!("{}: {e}", dir.display())));
            }
            _ => {}
        }
        self.level_registry.remove_level(id)?;
        self.level_registry
            .write_created(&self.world_root)
            .map_err(|e| LevelManagerError::Io(format!("data/yoake_levels.json: {e}")))?;
        if let Some(host) = self.lan.as_ref() {
            host.hub.sync.withdraw_level(id);
        }
        Ok(())
    }

    /// Saves a level and closes it, freeing its chunks and files, while it stays registered: it
    /// opens again from its folder the next time someone enters it (F3-7). Refused for the level in
    /// place and while anyone is in or headed for it; a level that is not open is already unloaded.
    pub(super) fn unload_level(&mut self, id: &str) -> Result<(), LevelManagerError> {
        let dim = self.unloadable(id)?;
        if self.levels.parked_world(dim).is_none() {
            return Ok(());
        }
        let back = self.dim;
        self.enter_level(None, dim)
            .and_then(|()| self.save_level(true))
            .and_then(|()| self.enter_level(None, back))
            .map_err(|e| LevelManagerError::Io(e.to_string()))?;
        self.level_before_close(id);
        self.levels.drop_parked(dim);
        Ok(())
    }

    /// Throws a level's terrain away and starts it again, with `new_seed` for a created level
    /// (`None` keeps its seed): closes it, removes its `region`, `entities` and `poi` folders, and
    /// leaves it registered under the same id, so references to it stay good. Refused while anyone
    /// is in it (F3-7).
    pub(super) fn regenerate_level(
        &mut self,
        id: &str,
        new_seed: Option<i64>,
    ) -> Result<(), LevelManagerError> {
        let dim = self.unloadable(id)?;
        if new_seed.is_some() {
            // Only a created level has a seed of its own to change (and to persist).
            self.level_registry.set_seed(id, new_seed)?;
            self.level_registry
                .write_created(&self.world_root)
                .map_err(|e| LevelManagerError::Io(format!("data/yoake_levels.json: {e}")))?;
        }
        self.level_before_close(id);
        self.levels.drop_parked(dim);
        let dir = self.level_registry.save_dir(&self.world_root, id);
        for folder in ["region", "entities", "poi"] {
            match std::fs::remove_dir_all(dir.join(folder)) {
                Err(e) if e.kind() != std::io::ErrorKind::NotFound => {
                    return Err(LevelManagerError::Io(format!("{}: {e}", dir.join(folder).display())));
                }
                _ => {}
            }
        }
        Ok(())
    }

    /// The level `id` when it may be closed: registered, not vanilla, not in place and empty.
    fn unloadable(&self, id: &str) -> Result<Dim, LevelManagerError> {
        let Some(dim) = self.level_dim(id) else {
            return Err(LevelError::Unknown(id.to_owned()).into());
        };
        if self.level_registry.get(id).is_some_and(|e| e.source == LevelSource::Vanilla) {
            return Err(LevelError::Vanilla(id.to_owned()).into());
        }
        if self.is_occupied(dim, id) {
            return Err(LevelManagerError::Occupied(id.to_owned()));
        }
        Ok(dim)
    }

    /// Whether the local player, a guest or a pending teleport is in or headed for the level.
    fn is_occupied(&self, dim: Dim, id: &str) -> bool {
        self.dim == dim
            || self
                .lan
                .iter()
                .flat_map(|h| h.hub.guests.iter())
                .any(|g| g.placed && g.dimension == id)
            || self.level_teleports.iter().any(|t| t.dim == dim)
            || self.server_trips.iter().any(|t| t.dim == dim)
    }

    /// The world's levels, the three vanilla ones first.
    pub(super) fn list_levels(&self) -> Vec<LevelInfo> {
        self.level_registry
            .entries()
            .iter()
            .map(|e| {
                let dim = Dim::lookup(&e.id);
                LevelInfo {
                    id: e.id.clone(),
                    source: e.source,
                    ephemeral: e.ephemeral,
                    open: dim.is_some_and(|d| d == self.dim || self.levels.parked_world(d).is_some()),
                }
            })
            .collect()
    }

    /// How coordinates scale going from level `from` to level `to`: the ratio of their types'
    /// `coordinate_scale` (the Nether's is 8): 1/8 from the Overworld to the Nether, 8 back, 1
    /// between levels of the same scale. The vanilla levels give exactly the factors portals have
    /// always used.
    pub(super) fn travel_scale(&self, from: Dim, to: Dim) -> f64 {
        self.level_registry.coordinate_scale(from.id()) / self.level_registry.coordinate_scale(to.id())
    }
}
