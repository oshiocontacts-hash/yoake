//! The level services of WebAssembly (and native) mods (F3-8): registering levels, dimension types and
//! generators, creating, deleting, unloading and regenerating levels, listing them. They are the
//! `Game` helpers the sandboxed `level-*` services and `generator-register` call
//! (`tools/native-hook-fragments/f3_levels.json`); each forwards to the level manager of F3-6/F3-7 and
//! checks what a guest passes first.
use super::{Dim, Game};
use serde_json::Value;

/// Most levels mods may have created at once (registered ones are not counted).
const MAX_CREATED_LEVELS: usize = 64;
/// Largest stem or type a guest passes.
const MAX_JSON: usize = 1 << 16;

fn json_object(what: &str, text: &str) -> Result<Value, String> {
    if text.len() > MAX_JSON {
        return Err(format!("the {what} is larger than 64 KiB"));
    }
    match serde_json::from_str::<Value>(text) {
        Ok(v @ Value::Object(_)) => Ok(v),
        Ok(_) => Err(format!("the {what} is not a JSON object")),
        Err(e) => Err(format!("the {what}: {e}")),
    }
}

impl Game<'_> {
    /// Registers a level stem while the world has not ticked yet (the mods' normal way is
    /// `init`, which stages it for every world; this one is for a hook that runs first).
    pub(super) fn native_levels_register(&mut self, id: &str, stem_json: &str) -> Result<(), String> {
        let stem = json_object("level stem", stem_json)?;
        self.register_mod_level(id, stem)
            .map(|_| ())
            .map_err(|e| e.to_string())
    }

    /// Creates a level while the server runs (see `Game::create_level`).
    pub(super) fn native_levels_create(
        &mut self,
        id: &str,
        persistent: bool,
        stem_json: &str,
    ) -> Result<(), String> {
        let stem = json_object("level stem", stem_json)?;
        let created = self
            .list_levels()
            .iter()
            .filter(|l| l.source == horizon_client::level_registry::LevelSource::Created)
            .count();
        if created >= MAX_CREATED_LEVELS {
            return Err(format!("at most {MAX_CREATED_LEVELS} levels created while the server runs"));
        }
        self.create_level(id, stem, persistent)
            .map(|_| ())
            .map_err(|e| e.to_string())
    }

    pub(super) fn native_levels_delete(&mut self, id: &str) -> Result<(), String> {
        self.delete_level(id).map_err(|e| e.to_string())
    }

    /// Saving a level needs it in place, which a hook's level is not free to leave: the unload is
    /// queued and runs at the start of the next server tick. Refusals that can be known now are
    /// answered now.
    pub(super) fn native_levels_unload(&mut self, id: &str) -> Result<(), String> {
        let Some(entry) = self.level_registry.get(id) else {
            return Err(format!("level `{id}` is not one of this world's levels"));
        };
        if entry.source == horizon_client::level_registry::LevelSource::Vanilla {
            return Err(format!("`{id}` is one of the three vanilla levels"));
        }
        if !self.pending_unloads.iter().any(|l| l == id) {
            self.pending_unloads.push(id.to_owned());
        }
        Ok(())
    }

    pub(super) fn native_levels_regenerate(&mut self, id: &str, seed: Option<i64>) -> Result<(), String> {
        self.regenerate_level(id, seed).map_err(|e| e.to_string())
    }

    pub(super) fn native_levels_list(&self) -> Vec<String> {
        self.level_registry.ids().map(str::to_owned).collect()
    }

    /// Declares that package `mod_id` generates the chunks of levels naming generator `id`, for
    /// a world that has not ticked yet.
    pub(super) fn native_levels_generator(&mut self, id: &str, mod_id: &str) -> Result<(), String> {
        self.register_mod_generator(id, crate::entry::wasm_mods::generator_factory(mod_id))
    }

    /// The unloads mods asked for, at the start of a server tick (F3-8). A level that cannot be
    /// unloaded now (someone entered it meanwhile) is reported and dropped.
    pub(super) fn run_pending_level_ops(&mut self) {
        for id in std::mem::take(&mut self.pending_unloads) {
            if let Err(e) = self.unload_level(&id) {
                log::warn!("Horizon: unloading level {id} (asked by a mod): {e}");
            }
        }
    }

    /// The level `id` as a `Dim`, when it is one of this world's.
    #[allow(dead_code)]
    pub(super) fn native_levels_dim(&self, id: &str) -> Option<Dim> {
        self.level_dim(id)
    }
}
