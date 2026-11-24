//! Hot reload of custom mobs in development (FR-F4-MOBS.md section 3.11, F4M-13).
//!
//! * **R1, assets**: [`Watcher`] polls the files of a dev folder or an unsigned package (the type's
//!   JSON, the model, the animations, the controllers, the textures and the glow mask). A change
//!   counts once the files have stayed the same for a whole poll (a build may still be writing).
//! * **R2, definitions**: [`rebuild`] reads the type again from its source folder through the same
//!   loaders as the first load; [`install`] puts the result in the registry, and [`sync_mobs`]
//!   brings every live mob of the type to it (attributes recomputed, the goal list rebuilt when the
//!   `ai` changed, hitbox parts replaced, the model and the animations at the next draw).
//! * **Failure handling**: a file that does not load keeps the old type running; [`rebuild`]
//!   returns why, naming the file, and the caller logs it once. A change of the synced values or the
//!   animation layers (what the wire format depends on) is refused with a message: those need a restart.
//!
//! Only types registered with [`register_source`] reload, and sources are registered only in
//! developer mode ([`dev_enabled`], the switch of WASM dev reload) for a folder without a
//! `SIGNATURE`. Nothing here runs, or costs anything, without one.
//!
//! The animation phase of a running mob lives in the clocks the server anchored, not in the type, so
//! a swap keeps it: [`super::render_state::MobRender::rebind`] only drops what is shaped by the old
//! model (the pose buffers and the layer states, which the next update takes from the anchors).
use super::{CustomType, attrs::AttrSet, build, registry};
use crate::mobs::{Mob, MobType};
use horizon_mods::custom_mob::CustomMobFile;
use std::{
    collections::{BTreeMap, BTreeSet, HashMap, HashSet},
    path::{Path, PathBuf},
    sync::{
        Arc, Mutex,
        atomic::{AtomicU64, Ordering},
    },
    time::SystemTime,
};

/// What kind of file changed.
pub const DATA: u8 = 1;
pub const MODEL: u8 = 2;
pub const TEXTURE: u8 = 4;

/// Whether developer mode is on: `YOAKE_WASM_UNSIGNED=1`, or the legacy `YOAKE_DLL_MODS_UNSIGNED=1`
/// superset (the same switch as the WASM dev reload).
pub fn dev_enabled() -> bool {
    static ON: std::sync::OnceLock<bool> = std::sync::OnceLock::new();
    *ON.get_or_init(|| {
        let on = |name| std::env::var_os(name).is_some_and(|v| v == "1");
        on("YOAKE_WASM_UNSIGNED") || on("YOAKE_DLL_MODS_UNSIGNED")
    })
}

/// Whether the server-module swap (R3) is on too: developer mode and `YOAKE_WASM_SERVER_RELOAD=1`.
/// Server modules keep their state in their instance, so a swap starts them afresh (design risk 11);
/// that is why it is a switch of its own.
pub fn server_reload_enabled() -> bool {
    dev_enabled() && std::env::var_os("YOAKE_WASM_SERVER_RELOAD").is_some_and(|v| v == "1")
}

/// Where a type's files are read from again: its package's `content` folder.
#[derive(Clone, Debug, PartialEq)]
pub struct Source {
    pub id: String,
    pub dir: PathBuf,
}

fn sources_table() -> &'static Mutex<BTreeMap<String, Source>> {
    static SOURCES: Mutex<BTreeMap<String, Source>> = Mutex::new(BTreeMap::new());
    &SOURCES
}

/// Registers the source of type `id` (developer mode and an unsigned folder only; the caller
/// decides). A second call for the same id replaces the first.
pub fn register_source(id: &str, dir: &Path) {
    if let Ok(mut table) = sources_table().lock() {
        table.insert(id.to_owned(), Source { id: id.to_owned(), dir: dir.to_owned() });
    }
}

/// The registered sources, by type id.
pub fn sources() -> Vec<Source> {
    sources_table().lock().map(|t| t.values().cloned().collect()).unwrap_or_default()
}

/// Whether any source is registered (cheap: for the per-tick check).
pub fn has_sources() -> bool {
    sources_table().lock().is_ok_and(|t| !t.is_empty())
}

/// The source of type `id`.
pub fn source_of(id: &str) -> Option<Source> {
    sources_table().lock().ok()?.get(id).cloned()
}

/// Forgets every source (a new set of mods is being loaded).
pub fn clear_sources() {
    if let Ok(mut table) = sources_table().lock() {
        table.clear();
    }
}

/// Whether a content folder is an unsigned development package: the folder itself or its package
/// root (one up) carries no `SIGNATURE`.
pub fn is_unsigned_dir(dir: &Path) -> bool {
    dir.is_dir()
        && !dir.join("SIGNATURE").exists()
        && !dir.parent().is_some_and(|p| p.join("SIGNATURE").exists())
}

// ----------------------------------------------------------------------------------------------
// What is watched
// ----------------------------------------------------------------------------------------------

/// `<ns>:<path>` and an extension to the content file `assets/<ns>/<path><ext>`.
fn asset(id: &str, ext: &str) -> Option<String> {
    let (ns, rest) = id.split_once(':')?;
    Some(format!("assets/{ns}/{rest}{ext}"))
}

/// A texture id (`<ns>:entity/x`) to its file `assets/<ns>/textures/entity/x.png`.
fn texture(id: &str, suffix: &str) -> Option<String> {
    let (ns, rest) = id.split_once(':')?;
    Some(format!("assets/{ns}/textures/{rest}{suffix}.png"))
}

/// The relative paths a type is made from, with what a change of each means ([`DATA`], [`MODEL`], [`TEXTURE`]).
pub fn files_of(file: &CustomMobFile) -> Vec<(String, u8)> {
    let mut out = Vec::new();
    if let Some((ns, path)) = file.id.split_once(':') {
        out.push((format!("data/{ns}/horizon/entity_type/{path}.json"), DATA));
    }
    let m = &file.model;
    if let Some(geo) = &m.geo {
        out.extend(asset(geo, ".geo.json").map(|p| (p, MODEL)));
        out.extend(asset(geo, ".bbmodel").map(|p| (p, MODEL)));
    }
    out.extend(m.animations.iter().filter_map(|a| asset(a, ".animation.json")).map(|p| (p, MODEL)));
    out.extend(m.controllers.iter().filter_map(|c| asset(c, ".json")).map(|p| (p, MODEL)));
    let mut textures: Vec<&String> = m.textures.default.iter().chain(&m.textures.variants).collect();
    textures.extend(m.layers.iter().filter_map(|l| l.texture.as_ref()));
    for t in textures {
        out.extend(texture(t, "").map(|p| (p, TEXTURE)));
        out.extend(texture(t, "_glowmask").map(|p| (p, TEXTURE)));
    }
    out.sort();
    out.dedup();
    out
}

type Stamp = Option<(SystemTime, u64)>;

fn stat(path: &Path) -> Stamp {
    let meta = std::fs::metadata(path).ok()?;
    meta.is_file().then(|| (meta.modified().ok().unwrap_or(SystemTime::UNIX_EPOCH), meta.len()))
}

struct Entry {
    dir: PathBuf,
    files: Vec<(String, u8)>,
    /// The files as last seen, and as last handed over.
    seen: Vec<Stamp>,
    applied: Vec<Stamp>,
}

/// One type's change found by a poll.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Change {
    pub id: String,
    /// [`DATA`] | [`MODEL`] | [`TEXTURE`].
    pub kinds: u8,
    /// The changed files, relative to the source folder.
    pub files: Vec<String>,
}

/// Polls the files of the registered types.
#[derive(Default)]
pub struct Watcher {
    types: BTreeMap<String, Entry>,
}

impl Watcher {
    /// Starts (or restarts) watching what `file` is made from in `dir`. The files as they are now are
    /// the baseline: only a later change is reported.
    pub fn track(&mut self, source: &Source, file: &CustomMobFile) {
        let files = files_of(file);
        let stamps: Vec<Stamp> = files.iter().map(|(p, _)| stat(&source.dir.join(p))).collect();
        self.types.insert(
            source.id.clone(),
            Entry { dir: source.dir.clone(), files, seen: stamps.clone(), applied: stamps },
        );
    }

    pub fn is_empty(&self) -> bool {
        self.types.is_empty()
    }

    pub fn tracks(&self, id: &str) -> bool {
        self.types.contains_key(id)
    }

    /// Looks at the files once. A change is reported when the files have stayed as they were since the
    /// previous poll (or at once with `force`: `/yoake mob reload`), and again only after the next change.
    pub fn poll(&mut self, force: bool) -> Vec<Change> {
        let mut out = Vec::new();
        for (id, e) in &mut self.types {
            let now: Vec<Stamp> = e.files.iter().map(|(p, _)| stat(&e.dir.join(p))).collect();
            let moving = now != e.seen;
            e.seen.clone_from(&now);
            if (moving && !force) || now == e.applied {
                continue;
            }
            let mut kinds = 0;
            let mut files = Vec::new();
            for (i, (path, kind)) in e.files.iter().enumerate() {
                if now[i] != e.applied[i] {
                    kinds |= kind;
                    files.push(path.clone());
                }
            }
            e.applied = now;
            out.push(Change { id: id.clone(), kinds, files });
        }
        out
    }
}

fn shared_watcher() -> &'static Mutex<Watcher> {
    static WATCHER: Mutex<Watcher> = Mutex::new(Watcher { types: BTreeMap::new() });
    &WATCHER
}

/// Starts watching every registered source whose type is in the registry and is not watched yet; the ids
/// that began.
pub fn watch_new_sources() -> Vec<Source> {
    let mut began = Vec::new();
    let Ok(mut w) = shared_watcher().lock() else { return began };
    for source in sources() {
        if w.tracks(&source.id) {
            continue;
        }
        if let Some(ct) = registry::get(&source.id) {
            w.track(&source, &ct.file);
            began.push(source);
        }
    }
    began
}

/// [`Watcher::poll`] on the process's one watcher (the window's game and the integrated server's share it).
pub fn watch_poll(force: bool) -> Vec<Change> {
    shared_watcher().lock().map(|mut w| w.poll(force)).unwrap_or_default()
}

/// Watches `new`'s files from now on (after a reload).
pub fn watch_again(source: &Source, file: &CustomMobFile) {
    if let Ok(mut w) = shared_watcher().lock() {
        w.track(source, file);
    }
}

// ----------------------------------------------------------------------------------------------
// Rebuilding a type
// ----------------------------------------------------------------------------------------------

/// The type `source` names, read again from its folder through the loaders of the first load. The error
/// says which file failed and why.
pub fn rebuild(source: &Source) -> Result<CustomType, String> {
    let (ns, path) = source.id.split_once(':').ok_or_else(|| format!("{}: not an id", source.id))?;
    let rel = format!("data/{ns}/horizon/entity_type/{path}.json");
    let bytes = std::fs::read(source.dir.join(&rel)).map_err(|e| format!("{rel}: {e}"))?;
    let file = CustomMobFile::parse(&source.id, &bytes).map_err(|e| format!("{rel}: {e}"))?;
    let model = file.load_model(&mut |p| {
        std::fs::read(source.dir.join(p)).map_err(|e| format!("{p}: {e}"))
    })?;
    CustomType::new(file, Some(&model)).map_err(|e| format!("{rel}: {e}"))
}

/// Whether `new` can replace `old` under live mobs: what is sent to clients (the tracked values, the
/// animation layers and triggers) must be the same shape.
pub fn compatible(old: &CustomType, new: &CustomType) -> Result<(), String> {
    if old.sync_version == new.sync_version {
        return Ok(());
    }
    Err(format!(
        "{}: the synced values or the animation layers changed (the wire layout differs); restart the game to use them",
        new.id
    ))
}

/// What a reload did.
#[derive(Debug, Default, PartialEq, Eq)]
pub struct Outcome {
    /// The type was replaced.
    pub installed: bool,
    pub mobs: usize,
}

static GENERATION: AtomicU64 = AtomicU64::new(0);

/// Counts every replacement of a registry entry (a reload, or a whole table replaced by a resource
/// reload): the game looks at it to bring live mobs to the current types.
pub fn generation() -> u64 {
    GENERATION.load(Ordering::Acquire)
}

pub(super) fn note_replaced() {
    GENERATION.fetch_add(1, Ordering::AcqRel);
}

/// Replaces the registered type `new.id` by `new` (the caller checked [`compatible`]).
pub fn install(new: CustomType) -> Arc<CustomType> {
    let new = Arc::new(new);
    if let Ok(mut table) = registry::registry().write() {
        table.replace_one(new.clone());
    }
    new
}

/// The engine's type of `ct`, made once per registered version and shared by every mob of it.
pub fn kind_of(ct: &Arc<CustomType>) -> Arc<MobType> {
    static KINDS: Mutex<Option<HashMap<String, (Arc<CustomType>, Arc<MobType>)>>> = Mutex::new(None);
    let mut kinds = KINDS.lock().unwrap_or_else(|e| e.into_inner());
    let kinds = kinds.get_or_insert_with(HashMap::new);
    if let Some((c, k)) = kinds.get(&ct.id)
        && Arc::ptr_eq(c, ct)
    {
        return k.clone();
    }
    let kind = Arc::new(build::mob_type(ct));
    kinds.insert(ct.id.clone(), (ct.clone(), kind.clone()));
    kind
}

// ----------------------------------------------------------------------------------------------
// Bringing live mobs to a new type
// ----------------------------------------------------------------------------------------------

/// Carries a mob of `old` over to `new` (see the module docs). The mob's own values stay: health
/// (cut to the new maximum), tracked values, modifiers, what it carries and where it is.
pub fn apply_to_mob(m: &mut Mob, old: &CustomType, new: &Arc<CustomType>) {
    let was_kind = m.kind.clone();
    let kind = kind_of(new);
    let was_full = m.health >= m.attrs.max_health;
    m.kind = kind.clone();
    // The built-in numbers follow the file unless something changed them on this mob.
    let follow = |now: &mut f64, was: f64, to: f64| {
        if (*now - was).abs() < 1e-9 {
            *now = to;
        }
    };
    follow(&mut m.attrs.speed, was_kind.speed, kind.speed);
    follow(&mut m.attrs.attack_knockback, was_kind.attack_knockback, kind.attack_knockback);
    follow(&mut m.attrs.follow_range, was_kind.follow_range, kind.follow_range);
    follow(&mut m.attrs.knockback_resistance, was_kind.knockback_resistance, kind.knockback_resistance);
    follow(&mut m.attrs.jump_strength, was_kind.jump_strength, kind.jump_strength);
    let (mut damage, mut max_health) = (f64::from(m.attrs.attack_damage), f64::from(m.attrs.max_health));
    follow(&mut damage, f64::from(was_kind.attack_damage), f64::from(kind.attack_damage));
    follow(&mut max_health, f64::from(was_kind.max_health), f64::from(kind.max_health));
    m.attrs.attack_damage = damage as f32;
    m.attrs.max_health = max_health as f32;
    m.health = if was_full { m.attrs.max_health } else { m.health.min(m.attrs.max_health) };
    let ai_changed = old.file.ai != new.file.ai;
    if let Some(state) = m.extra.custom.as_mut() {
        state.attrs = rebase_attrs(&state.attrs, old, new);
        let mask = rebase_hitbox_mask(state.hitbox_mask, old, new);
        if mask != state.hitbox_mask {
            state.hitbox_mask = mask;
            state.hitbox_dirty = true;
        }
        if ai_changed {
            state.brain = super::ai::brain::BrainState::new(new);
        }
    }
    if ai_changed {
        m.ai.goals.clear();
        m.ai.targets.clear();
        super::ai::attach(m, new);
    }
}

/// The mob's attribute set on the new definitions: its bases and permanent modifiers carry over for
/// the attributes both have (an attribute still at the old default moves to the new default).
fn rebase_attrs(old_set: &AttrSet, old: &CustomType, new: &CustomType) -> AttrSet {
    let mut set = AttrSet::new(&new.attributes).unwrap_or_default();
    set.read_nbt(&old_set.to_nbt(&old.attributes));
    set
}

/// Enabled hitbox parts by name: a part keeps its switch, a new one starts enabled.
pub fn rebase_hitbox_mask(mask: u32, old: &CustomType, new: &CustomType) -> u32 {
    let on = |i: usize| i >= 32 || mask & (1 << i) != 0;
    let mut out = 0u32;
    for (i, part) in new.hitboxes.iter().enumerate().take(32) {
        let enabled = old.hitboxes.iter().position(|p| p.name == part.name).is_none_or(on);
        if enabled {
            out |= 1 << i;
        }
    }
    out
}

/// What [`sync_mobs`] did.
#[derive(Debug, Default)]
pub struct Synced {
    /// `(type id, mobs carried over)` by type.
    pub applied: BTreeMap<String, usize>,
    /// Types a reload could not apply to live mobs, with why (each reported once).
    pub refused: Vec<String>,
}

/// Brings every mob whose custom type is no longer the registered one to the registered one. Nothing to
/// do (and nothing done but pointer comparisons) when every mob already holds the current types.
pub fn sync_mobs(mobs: &mut [Mob]) -> Synced {
    static REFUSED: Mutex<Option<HashSet<(String, usize, usize)>>> = Mutex::new(None);
    let mut out = Synced::default();
    let Ok(table) = registry::registry().read() else {
        return out;
    };
    for m in mobs.iter_mut() {
        let Some(have) = m.kind.custom.clone() else { continue };
        let Some(current) = table.get(&have.id) else { continue };
        if Arc::ptr_eq(&have, current) {
            continue;
        }
        if let Err(why) = compatible(&have, current) {
            let mut refused = REFUSED.lock().unwrap_or_else(|e| e.into_inner());
            let key = (have.id.clone(), Arc::as_ptr(&have) as usize, Arc::as_ptr(current) as usize);
            if refused.get_or_insert_with(HashSet::new).insert(key) {
                out.refused.push(why);
            }
            continue;
        }
        let current = current.clone();
        apply_to_mob(m, &have, &current);
        *out.applied.entry(current.id.clone()).or_default() += 1;
    }
    out
}

/// The ids of the types that have a source, and so can reload.
pub fn reloadable() -> BTreeSet<String> {
    sources().into_iter().map(|s| s.id).collect()
}

/// Rebuilds type `source.id` and installs it when it loads and fits; the error is the log line's
/// reason. The caller brings the mobs over ([`sync_mobs`]) and tells the watcher ([`Watcher::track`]).
pub fn reload_type(source: &Source) -> Result<Arc<CustomType>, String> {
    let new = rebuild(source)?;
    if let Some(old) = registry::get(&source.id) {
        compatible(&old, &new)?;
    }
    Ok(install(new))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::mobs::custom::tests::fox_type;
    use horizon_core::geometry::Vec3;
    use serde_json::Value;
    use std::sync::atomic::AtomicU32;

    fn fixture() -> PathBuf {
        Path::new(env!("CARGO_MANIFEST_DIR")).join("../../tools/fixtures/cinder_fox/content")
    }

    fn copy_dir(from: &Path, to: &Path) {
        std::fs::create_dir_all(to).unwrap();
        for e in std::fs::read_dir(from).unwrap().flatten() {
            let (src, dst) = (e.path(), to.join(e.file_name()));
            if src.is_dir() {
                copy_dir(&src, &dst);
            } else {
                std::fs::copy(&src, &dst).unwrap();
            }
        }
    }

    /// A scratch copy of the fixture's content folder whose type is `rl<N><tag>:fox` (so no other test's
    /// registry entry is touched), as the source of a reload.
    fn scratch(tag: &str) -> Source {
        static N: AtomicU32 = AtomicU32::new(0);
        let n = N.fetch_add(1, Ordering::Relaxed);
        let dir = std::env::temp_dir().join(format!("hz-reload-{}-{tag}-{n}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        copy_dir(&fixture(), &dir);
        let id = format!("rl{n}{tag}:fox");
        let (ns, path) = id.split_once(':').unwrap();
        let old = dir.join("data/sample_mobs/horizon/entity_type/cinder_fox.json");
        let new = dir.join(format!("data/{ns}/horizon/entity_type/{path}.json"));
        std::fs::create_dir_all(new.parent().unwrap()).unwrap();
        std::fs::rename(old, new).unwrap();
        Source { id, dir }
    }

    fn type_path(s: &Source) -> PathBuf {
        let (ns, path) = s.id.split_once(':').unwrap();
        s.dir.join(format!("data/{ns}/horizon/entity_type/{path}.json"))
    }

    fn edit_json(path: &Path, f: impl FnOnce(&mut Value)) {
        let mut v: Value = serde_json::from_slice(&std::fs::read(path).unwrap()).unwrap();
        f(&mut v);
        std::fs::write(path, serde_json::to_vec_pretty(&v).unwrap()).unwrap();
    }

    #[test]
    fn a_type_names_every_file_it_is_made_from() {
        let files = files_of(&fox_type().file);
        let has = |p: &str, k: u8| files.iter().any(|(f, kind)| f == p && *kind == k);
        assert!(has("data/sample_mobs/horizon/entity_type/cinder_fox.json", DATA));
        assert!(has("assets/sample_mobs/geo/cinder_fox.geo.json", MODEL));
        assert!(has("assets/sample_mobs/geo/cinder_fox.bbmodel", MODEL));
        assert!(has("assets/sample_mobs/animations/cinder_fox.animation.json", MODEL));
        assert!(has("assets/sample_mobs/controllers/cinder_fox.json", MODEL));
        assert!(has("assets/sample_mobs/textures/entity/cinder_fox.png", TEXTURE));
        assert!(has("assets/sample_mobs/textures/entity/cinder_fox_glowmask.png", TEXTURE));
    }

    #[test]
    fn a_change_is_reported_once_after_the_files_settle() {
        let src = scratch("watch");
        let ct = rebuild(&src).unwrap();
        let mut w = Watcher::default();
        w.track(&src, &ct.file);
        assert!(w.poll(false).is_empty(), "nothing changed since the baseline");
        let anim = src.dir.join("assets/sample_mobs/animations/cinder_fox.animation.json");
        let tex = src.dir.join("assets/sample_mobs/textures/entity/cinder_fox.png");
        let mut text = std::fs::read_to_string(&anim).unwrap();
        text.push_str("\n\n");
        std::fs::write(&anim, text).unwrap();
        let mut png = std::fs::read(&tex).unwrap();
        png.push(0);
        std::fs::write(&tex, png).unwrap();
        // The first look sees the files move; the second sees them the same: the change counts.
        assert!(w.poll(false).is_empty(), "a file that just changed may still be written");
        let changes = w.poll(false);
        assert_eq!(changes.len(), 1);
        assert_eq!(changes[0].id, src.id);
        assert_eq!(changes[0].kinds, MODEL | TEXTURE);
        assert_eq!(changes[0].files.len(), 2);
        assert!(w.poll(false).is_empty(), "reported once");
        // A forced look (`/yoake mob reload`) does not wait for the settle.
        let mut data = std::fs::read(type_path(&src)).unwrap();
        data.push(b'\n');
        std::fs::write(type_path(&src), data).unwrap();
        assert_eq!(w.poll(true)[0].kinds, DATA);
    }

    /// Scenario 7's data half: move the ear, the new type has it; a malformed edit keeps the old type
    /// and says which file.
    #[test]
    fn a_moved_ear_loads_and_a_broken_file_keeps_the_old_type() {
        let src = scratch("ear");
        let first = reload_type(&src).unwrap();
        let ear = |t: &CustomType| {
            let tr = t.render.as_ref().unwrap();
            let i = tr.skeleton.find("ear_l").unwrap().index();
            tr.skeleton.bones[i].pivot
        };
        let geo = src.dir.join("assets/sample_mobs/geo/cinder_fox.geo.json");
        edit_json(&geo, |v| {
            for b in v["minecraft:geometry"][0]["bones"].as_array_mut().unwrap() {
                if b["name"] == "ear_l" {
                    b["pivot"][1] = Value::from(b["pivot"][1].as_f64().unwrap() + 2.0);
                }
            }
        });
        let second = reload_type(&src).unwrap();
        assert!((ear(&second).y - ear(&first).y - 2.0).abs() < 1e-4, "the ear moved up two pixels");
        assert!(Arc::ptr_eq(&second, &registry::get(&src.id).unwrap()), "the registry holds the new one");
        // A truncated file: nothing is replaced, and the reason names the file.
        std::fs::write(&geo, b"{ \"minecraft:geometry\": [ { \"bones\": [").unwrap();
        let why = reload_type(&src).unwrap_err();
        assert!(why.contains("geo"), "{why}");
        assert!(Arc::ptr_eq(&second, &registry::get(&src.id).unwrap()), "the old type keeps running");
        // The same for a type file that does not parse.
        std::fs::write(type_path(&src), b"{ not json").unwrap();
        assert!(reload_type(&src).unwrap_err().contains("fox.json"));
        assert!(Arc::ptr_eq(&second, &registry::get(&src.id).unwrap()));
    }

    /// R2: a new maximum health and a trimmed goal list reach a live mob, which keeps what is its own.
    #[test]
    fn a_definition_edit_reaches_a_live_mob_and_keeps_what_is_its_own() {
        let src = scratch("live");
        let old = reload_type(&src).unwrap();
        let mut mob = Mob::new(11, kind_of(&old), Vec3::new(0.5, 64.0, 0.5), 0.0);
        assert!(mob.ai.goals.has("Stroll"));
        mob.health = 5.0;
        mob.extra.custom.as_mut().unwrap().set_synced_slot(&old, 1, crate::mobs::custom::SyncedValue::I32(3));
        let full = Mob::new(12, kind_of(&old), Vec3::new(0.5, 64.0, 0.5), 0.0);
        assert_eq!(full.health, 14.0);
        edit_json(&type_path(&src), |v| {
            v["attributes"]["max_health"] = Value::from(20);
            v["ai"]["goals"] = serde_json::json!([{"type": "swim", "priority": 0}]);
        });
        let new = reload_type(&src).unwrap();
        let mut mobs = vec![mob, full];
        let done = sync_mobs(&mut mobs);
        assert_eq!(done.applied.get(&src.id), Some(&2));
        let mob = &mobs[0];
        assert!(Arc::ptr_eq(mob.kind.custom.as_ref().unwrap(), &new), "the mob holds the new type");
        assert_eq!(mob.attrs.max_health, 20.0, "the built-in number follows the file");
        assert_eq!(mob.health, 5.0, "a hurt mob stays hurt");
        assert!(mob.ai.goals.has("Float") && !mob.ai.goals.has("Stroll"), "the goal list was rebuilt");
        assert_eq!(
            mob.extra.custom.as_ref().unwrap().synced[1],
            crate::mobs::custom::SyncedValue::I32(3),
            "tracked values stay"
        );
        assert_eq!(mobs[1].health, 20.0, "a mob at full health follows the new maximum up");
        // Nothing left to do the second time.
        assert!(sync_mobs(&mut mobs).applied.is_empty());
    }

    #[test]
    fn a_change_of_the_wire_layout_is_refused_with_a_message() {
        let src = scratch("wire");
        let old = reload_type(&src).unwrap();
        edit_json(&type_path(&src), |v| {
            v["synced"]["hunger"] = serde_json::json!({"type": "i32", "default": 0, "min": 0, "max": 5, "sync": "all"});
        });
        let why = reload_type(&src).unwrap_err();
        assert!(why.contains("restart"), "{why}");
        assert!(Arc::ptr_eq(&old, &registry::get(&src.id).unwrap()), "the registry is untouched");
    }

    #[test]
    fn hitbox_switches_follow_the_part_names() {
        let t = fox_type();
        assert_eq!(rebase_hitbox_mask(0b01, &t, &t), 0b01);
        assert_eq!(rebase_hitbox_mask(0b10, &t, &t), 0b10);
        assert_eq!(rebase_hitbox_mask(0, &t, &t), 0);
    }

    #[test]
    fn a_folder_with_a_signature_is_not_a_dev_package() {
        let src = scratch("sig");
        assert!(is_unsigned_dir(&src.dir));
        std::fs::write(src.dir.join("SIGNATURE"), b"x").unwrap();
        assert!(!is_unsigned_dir(&src.dir));
        assert!(!is_unsigned_dir(&src.dir.join("missing")));
        let _ = std::fs::remove_dir_all(&src.dir);
    }
}
