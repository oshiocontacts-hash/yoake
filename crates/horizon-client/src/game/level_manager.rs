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
        let guests_in = self
            .lan
            .iter()
            .flat_map(|h| h.hub.guests.iter())
            .any(|g| g.placed && g.dimension == id)
            || self.level_teleports.iter().any(|t| t.dim == dim)
            || self.server_trips.iter().any(|t| t.dim == dim);
        if self.dim == dim || guests_in {
            return Err(LevelManagerError::Occupied(id.to_owned()));
        }
        let dir = self.level_registry.save_dir(&self.world_root, id);
        // Closing the level releases its region files before they are removed.
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
