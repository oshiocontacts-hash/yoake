//! Minecraft `options.txt` defaults and per-instance overrides applied before launch.
//!
//! Players keep a global profile ("Usual settings" / `game-options.json` in the launcher data
//! folder) and optional per-instance overrides (`Meta::game_options` in `instances.json`). Each
//! entry specifies a vanilla `options.txt` key, its text-encoded value, and whether it applies
//! `Always` (every launch) or `Once` (until the value changes in the launcher).
use crate::instances;
use serde::{Deserialize, Serialize};
use std::collections::{BTreeMap, HashMap, HashSet};
use std::fs;
use std::path::Path;
use std::sync::Mutex;

const GLOBAL_FILE: &str = "game-options.json";
pub const OPTIONS_TXT: &str = "options.txt";

/// Whether an entry overwrites `options.txt` on every launch or only once per configured value.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum ApplyMode {
    #[default]
    Always,
    Once,
}

/// One configured `options.txt` entry.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct OptionEntry {
    pub key: String,
    pub value: String,
    #[serde(default)]
    pub mode: ApplyMode,
}

/// Global "Usual settings" profile stored in `<launcher_data>/game-options.json`.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct GlobalProfile {
    #[serde(default)]
    pub entries: Vec<OptionEntry>,
}

/// Per-instance game options state stored in `instances.json` (`Meta::game_options`).
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct InstanceConfig {
    #[serde(default = "default_true", skip_serializing_if = "is_true")]
    pub use_usual: bool,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub entries: Vec<OptionEntry>,
    /// Last `Once` value written to this instance's `options.txt`, keyed by option name.
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub applied: BTreeMap<String, String>,
}

fn default_true() -> bool {
    true
}

fn is_true(v: &bool) -> bool {
    *v
}

impl Default for InstanceConfig {
    fn default() -> Self {
        Self {
            use_usual: true,
            entries: Vec::new(),
            applied: BTreeMap::new(),
        }
    }
}

impl InstanceConfig {
    pub fn is_default(&self) -> bool {
        self.use_usual && self.entries.is_empty() && self.applied.is_empty()
    }
}

/// Inserts or updates an entry by key, preserving existing order when replacing.
pub fn upsert_entry(entries: &mut Vec<OptionEntry>, key: String, value: String, mode: ApplyMode) {
    if let Some(existing) = entries.iter_mut().find(|e| e.key == key) {
        existing.value = value;
        existing.mode = mode;
    } else {
        entries.push(OptionEntry { key, value, mode });
    }
}

/// Removes an entry by key; returns whether one was removed.
pub fn remove_entry(entries: &mut Vec<OptionEntry>, key: &str) -> bool {
    let before = entries.len();
    entries.retain(|e| e.key != key);
    entries.len() != before
}

/// Returns the entry for `key`, if present.
pub fn get_entry<'a>(entries: &'a [OptionEntry], key: &str) -> Option<&'a OptionEntry> {
    entries.iter().find(|e| e.key == key)
}

/// Loads the global profile from the launcher's data folder.
pub fn load_global() -> GlobalProfile {
    load_global_in(&crate::backend::data_dir())
}

pub fn load_global_in(data_dir: &Path) -> GlobalProfile {
    let path = data_dir.join(GLOBAL_FILE);
    let Ok(text) = fs::read_to_string(&path) else {
        return GlobalProfile::default();
    };
    serde_json::from_str(&text).unwrap_or_default()
}

/// Saves the global profile atomically into the launcher's data folder.
pub fn save_global(profile: &GlobalProfile) -> Result<(), String> {
    save_global_in(&crate::backend::data_dir(), profile)
}

pub fn save_global_in(data_dir: &Path, profile: &GlobalProfile) -> Result<(), String> {
    static SAVE_LOCK: Mutex<()> = Mutex::new(());
    let _guard = SAVE_LOCK.lock().unwrap_or_else(|e| e.into_inner());
    let path = data_dir.join(GLOBAL_FILE);
    let tmp = data_dir.join(format!("{GLOBAL_FILE}.{}.tmp", std::process::id()));
    let json =
        serde_json::to_vec_pretty(profile).map_err(|e| format!("{}: {e}", path.display()))?;
    let written = fs::create_dir_all(data_dir)
        .and_then(|()| fs::write(&tmp, json))
        .and_then(|()| fs::rename(&tmp, &path));
    if let Err(e) = written {
        let _ = fs::remove_file(&tmp);
        return Err(format!("{}: {e}", path.display()));
    }
    Ok(())
}

/// Resolves the effective option list for an instance (global usual settings overridden key-by-key
/// by the instance's own entries when `use_usual` is enabled).
pub fn resolve_entries(global: &GlobalProfile, instance: &InstanceConfig) -> Vec<OptionEntry> {
    if !instance.use_usual {
        return deduplicated(&instance.entries);
    }
    let mut out = deduplicated(&global.entries);
    for entry in &instance.entries {
        upsert_entry(&mut out, entry.key.clone(), entry.value.clone(), entry.mode);
    }
    out
}

fn deduplicated(entries: &[OptionEntry]) -> Vec<OptionEntry> {
    let mut out = Vec::with_capacity(entries.len());
    for entry in entries {
        upsert_entry(&mut out, entry.key.clone(), entry.value.clone(), entry.mode);
    }
    out
}

/// What happened when planning and applying game options to an instance folder.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct ApplyReport {
    pub written_count: usize,
    pub applied_changed: bool,
}

/// Applies effective `Always` and unapplied `Once` entries to `<home>/options.txt`, updating
/// `instance.applied` only after `options.txt` has been written successfully.
pub fn plan_and_apply(
    home: &Path,
    global: &GlobalProfile,
    instance: &mut InstanceConfig,
) -> Result<ApplyReport, String> {
    let effective = resolve_entries(global, instance);
    let active_once: HashSet<&str> = effective
        .iter()
        .filter(|e| e.mode == ApplyMode::Once)
        .map(|e| e.key.as_str())
        .collect();

    let mut next_applied = instance.applied.clone();
    next_applied.retain(|k, _| active_once.contains(k.as_str()));

    let mut updates: Vec<(String, String)> = Vec::new();
    let mut newly_applied_once: Vec<(String, String)> = Vec::new();
    for entry in &effective {
        match entry.mode {
            ApplyMode::Always => {
                updates.push((entry.key.clone(), entry.value.clone()));
            }
            ApplyMode::Once => {
                if next_applied.get(&entry.key) != Some(&entry.value) {
                    updates.push((entry.key.clone(), entry.value.clone()));
                    newly_applied_once.push((entry.key.clone(), entry.value.clone()));
                }
            }
        }
    }

    if !updates.is_empty() {
        let options_path = home.join(OPTIONS_TXT);
        let existing = match fs::read_to_string(&options_path) {
            Ok(text) => text,
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => String::new(),
            Err(e) => return Err(format!("{}: {e}", options_path.display())),
        };
        let updated = apply_entries_to_text(&existing, &updates);
        if !options_path.exists() || updated != existing {
            fs::create_dir_all(home).map_err(|e| format!("{}: {e}", home.display()))?;
            write_atomic(&options_path, updated.as_bytes())?;
        }
        for (k, v) in newly_applied_once {
            next_applied.insert(k, v);
        }
    }

    let applied_changed = next_applied != instance.applied;
    instance.applied = next_applied;
    Ok(ApplyReport {
        written_count: updates.len(),
        applied_changed,
    })
}

fn write_atomic(path: &Path, bytes: &[u8]) -> Result<(), String> {
    let name = path
        .file_name()
        .and_then(|n| n.to_str())
        .unwrap_or(OPTIONS_TXT);
    let tmp = path.with_file_name(format!("{name}.{}.tmp", std::process::id()));
    let res = fs::write(&tmp, bytes).and_then(|()| fs::rename(&tmp, path));
    if let Err(e) = res {
        let _ = fs::remove_file(&tmp);
        return Err(format!("{}: {e}", path.display()));
    }
    Ok(())
}

/// Updates `options.txt` text with `updates`, preserving all untouched lines and original line order,
/// and appending any new keys at the end.
pub fn apply_entries_to_text(existing: &str, updates: &[(String, String)]) -> String {
    if updates.is_empty() {
        return existing.to_string();
    }
    let mut map: HashMap<&str, &str> = HashMap::new();
    for (k, v) in updates {
        map.insert(k.as_str(), v.as_str());
    }
    let mut written: HashSet<&str> = HashSet::new();
    let mut lines: Vec<String> = Vec::new();
    for line in split_lines(existing) {
        if let Some((key, _)) = line.split_once(':') {
            if let Some(val) = map.get(key) {
                if written.insert(key) {
                    lines.push(format!("{key}:{val}"));
                }
                continue;
            }
        }
        lines.push(line.to_string());
    }
    for (k, v) in updates {
        if written.insert(k.as_str()) {
            lines.push(format!("{k}:{v}"));
        }
    }
    if lines.is_empty() {
        String::new()
    } else {
        let mut out = lines.join("\n");
        out.push('\n');
        out
    }
}

fn split_lines(text: &str) -> Vec<&str> {
    let mut lines = Vec::new();
    let bytes = text.as_bytes();
    let mut start = 0;
    let mut i = 0;
    while i < bytes.len() {
        match bytes[i] {
            b'\n' => {
                lines.push(&text[start..i]);
                start = i + 1;
            }
            b'\r' => {
                lines.push(&text[start..i]);
                if bytes.get(i + 1) == Some(&b'\n') {
                    i += 1;
                }
                start = i + 1;
            }
            _ => {}
        }
        i += 1;
    }
    if start < bytes.len() {
        lines.push(&text[start..]);
    }
    lines
}

/// Applies game options before launching an instance. Never fails the launch: errors are logged
/// and ignored.
pub fn apply_for_instance(instance_id: &str, home: &Path, log: Option<&dyn Fn(String)>) {
    let emit = |msg: String| {
        eprintln!("{msg}");
        if let Some(f) = log {
            f(msg);
        }
    };
    let global = load_global();
    let Some(instance) = instances::list().into_iter().find(|i| i.id == instance_id) else {
        // Still apply global usual settings if instance isn't in the list for any reason.
        if global.entries.is_empty() {
            return;
        }
        let mut fallback = InstanceConfig::default();
        if let Err(e) = plan_and_apply(home, &global, &mut fallback) {
            emit(format!("Game options were not applied: {e}"));
        }
        return;
    };
    let mut cfg = instance.game_options.clone();
    match plan_and_apply(home, &global, &mut cfg) {
        Ok(report) => {
            if report.applied_changed {
                if let Err(e) = instances::edit(instance_id, |meta| {
                    meta.game_options.applied = cfg.applied.clone();
                }) {
                    emit(format!("Game options state was not saved: {e}"));
                }
            }
        }
        Err(e) => {
            emit(format!("Game options were not applied: {e}"));
        }
    }
}

// ----- Vanilla options.txt codecs, validation and friendly catalog -------------------------------

/// Java `Double.toString` shortest round-trip formatting used by Minecraft's Gson/Options.
pub fn double_to_string(value: f64) -> String {
    if value.is_nan() {
        return "NaN".into();
    }
    if value.is_infinite() {
        return if value < 0.0 { "-Infinity" } else { "Infinity" }.into();
    }
    if value == 0.0 {
        return if value.is_sign_negative() {
            "-0.0"
        } else {
            "0.0"
        }
        .into();
    }
    let text = format!("{:e}", value.abs());
    let (mantissa, exponent) = text.split_once('e').unwrap_or((&text, "0"));
    let digits: String = mantissa.chars().filter(char::is_ascii_digit).collect();
    let exponent: i32 = exponent.parse().unwrap_or(0);
    let mut out = String::new();
    if value < 0.0 {
        out.push('-');
    }
    let digits = if digits.is_empty() { "0" } else { &digits };
    if (-3..7).contains(&exponent) {
        if exponent >= 0 {
            let int_len = exponent as usize + 1;
            if digits.len() <= int_len {
                out.push_str(digits);
                out.extend(std::iter::repeat_n('0', int_len - digits.len()));
                out.push_str(".0");
            } else {
                out.push_str(&digits[..int_len]);
                out.push('.');
                out.push_str(&digits[int_len..]);
            }
        } else {
            out.push_str("0.");
            out.extend(std::iter::repeat_n('0', (-exponent - 1) as usize));
            out.push_str(digits);
        }
    } else {
        out.push_str(&digits[..1]);
        out.push('.');
        if digits.len() > 1 {
            out.push_str(&digits[1..]);
        } else {
            out.push('0');
        }
        out.push('E');
        out.push_str(&exponent.to_string());
    }
    out
}

/// Categories for grouping friendly options in the picker.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Category {
    Sound,
    Video,
    Controls,
}

impl Category {
    pub const ALL: [Category; 3] = [Category::Sound, Category::Video, Category::Controls];

    pub fn label(self) -> &'static str {
        match self {
            Category::Sound => "Sound",
            Category::Video => "Video & Graphics",
            Category::Controls => "Controls & Language",
        }
    }
}

/// UI control type for a friendly option.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ControlKind {
    /// Percentage `0..=100` encoded as `0.0..=1.0`.
    UnitPercent,
    /// FOV degrees `30..=110` encoded as `(fov - 70.0) / 40.0`.
    Fov,
    /// Render distance chunks `2..=32` (validated up to `128`).
    RenderDistance,
    /// Simulation distance chunks `5..=32`.
    SimulationDistance,
    /// GUI scale `0..=6` (`0` = Auto, validated `>= 0`).
    GuiScale,
    /// Max FPS `10..=260` step `10` (`260` = Unlimited).
    MaxFps,
    /// Mouse sensitivity `0..=200` % encoded as `0.0..=1.0` (`100%` = `0.5`).
    MouseSensitivity,
    /// Boolean `"true"` / `"false"`.
    Bool,
    /// Language code plain string (e.g. `ja_jp`, `en_us`).
    Language,
}

/// Definition of a friendly option.
#[derive(Clone, Copy, Debug)]
pub struct FriendlyDef {
    pub key: &'static str,
    pub label: &'static str,
    pub category: Category,
    pub kind: ControlKind,
    pub default_raw: &'static str,
}

/// All friendly options offered in the UI, in display order.
pub const FRIENDLY: &[FriendlyDef] = &[
    FriendlyDef {
        key: "soundCategory_master",
        label: "Master volume",
        category: Category::Sound,
        kind: ControlKind::UnitPercent,
        default_raw: "1.0",
    },
    FriendlyDef {
        key: "soundCategory_music",
        label: "Music (BGM)",
        category: Category::Sound,
        kind: ControlKind::UnitPercent,
        default_raw: "1.0",
    },
    FriendlyDef {
        key: "soundCategory_record",
        label: "Jukebox / Note blocks",
        category: Category::Sound,
        kind: ControlKind::UnitPercent,
        default_raw: "1.0",
    },
    FriendlyDef {
        key: "soundCategory_weather",
        label: "Weather",
        category: Category::Sound,
        kind: ControlKind::UnitPercent,
        default_raw: "1.0",
    },
    FriendlyDef {
        key: "soundCategory_block",
        label: "Blocks",
        category: Category::Sound,
        kind: ControlKind::UnitPercent,
        default_raw: "1.0",
    },
    FriendlyDef {
        key: "soundCategory_hostile",
        label: "Hostile creatures",
        category: Category::Sound,
        kind: ControlKind::UnitPercent,
        default_raw: "1.0",
    },
    FriendlyDef {
        key: "soundCategory_neutral",
        label: "Friendly creatures",
        category: Category::Sound,
        kind: ControlKind::UnitPercent,
        default_raw: "1.0",
    },
    FriendlyDef {
        key: "soundCategory_player",
        label: "Players",
        category: Category::Sound,
        kind: ControlKind::UnitPercent,
        default_raw: "1.0",
    },
    FriendlyDef {
        key: "soundCategory_ambient",
        label: "Ambient / Environment",
        category: Category::Sound,
        kind: ControlKind::UnitPercent,
        default_raw: "1.0",
    },
    FriendlyDef {
        key: "soundCategory_voice",
        label: "Voice / Speech",
        category: Category::Sound,
        kind: ControlKind::UnitPercent,
        default_raw: "1.0",
    },
    FriendlyDef {
        key: "soundCategory_ui",
        label: "UI sounds",
        category: Category::Sound,
        kind: ControlKind::UnitPercent,
        default_raw: "1.0",
    },
    FriendlyDef {
        key: "fov",
        label: "FOV",
        category: Category::Video,
        kind: ControlKind::Fov,
        default_raw: "0.0",
    },
    FriendlyDef {
        key: "renderDistance",
        label: "Render distance",
        category: Category::Video,
        kind: ControlKind::RenderDistance,
        default_raw: "12",
    },
    FriendlyDef {
        key: "simulationDistance",
        label: "Simulation distance",
        category: Category::Video,
        kind: ControlKind::SimulationDistance,
        default_raw: "12",
    },
    FriendlyDef {
        key: "guiScale",
        label: "GUI scale",
        category: Category::Video,
        kind: ControlKind::GuiScale,
        default_raw: "0",
    },
    FriendlyDef {
        key: "gamma",
        label: "Brightness",
        category: Category::Video,
        kind: ControlKind::UnitPercent,
        default_raw: "0.5",
    },
    FriendlyDef {
        key: "maxFps",
        label: "Max FPS",
        category: Category::Video,
        kind: ControlKind::MaxFps,
        default_raw: "120",
    },
    FriendlyDef {
        key: "bobView",
        label: "View bobbing",
        category: Category::Video,
        kind: ControlKind::Bool,
        default_raw: "true",
    },
    FriendlyDef {
        key: "mouseSensitivity",
        label: "Mouse sensitivity",
        category: Category::Controls,
        kind: ControlKind::MouseSensitivity,
        default_raw: "0.5",
    },
    FriendlyDef {
        key: "lang",
        label: "Game language",
        category: Category::Controls,
        kind: ControlKind::Language,
        default_raw: "en_us",
    },
];

/// Common Minecraft language codes with native display names.
pub const LANGUAGES: &[(&str, &str)] = &[
    ("ja_jp", "日本語 (ja_jp)"),
    ("en_us", "English (US) (en_us)"),
    ("en_gb", "English (UK) (en_gb)"),
    ("zh_cn", "简体中文 (zh_cn)"),
    ("zh_tw", "繁體中文 (zh_tw)"),
    ("ko_kr", "한국어 (ko_kr)"),
    ("fr_fr", "Français (fr_fr)"),
    ("de_de", "Deutsch (de_de)"),
    ("es_es", "Español (es_es)"),
    ("pt_br", "Português (Brasil) (pt_br)"),
    ("ru_ru", "Русский (ru_ru)"),
];

/// Finds a friendly option definition by `options.txt` key.
pub fn find_friendly(key: &str) -> Option<&'static FriendlyDef> {
    FRIENDLY.iter().find(|d| d.key == key)
}

pub fn encode_unit_percent(pct: i32) -> Result<String, String> {
    if !(0..=100).contains(&pct) {
        return Err(format!("Percentage must be between 0 and 100 (got {pct})."));
    }
    Ok(double_to_string(f64::from(pct) / 100.0))
}

pub fn decode_unit_percent(raw: &str) -> Option<i32> {
    let v: f64 = raw.trim().parse().ok()?;
    if !v.is_finite() || !(0.0..=1.0).contains(&v) {
        return None;
    }
    Some((v * 100.0).round() as i32)
}

pub fn encode_fov(degrees: i32) -> Result<String, String> {
    if !(30..=110).contains(&degrees) {
        return Err(format!("FOV must be between 30 and 110 (got {degrees})."));
    }
    Ok(double_to_string((f64::from(degrees) - 70.0) / 40.0))
}

pub fn decode_fov(raw: &str) -> Option<i32> {
    let v: f64 = raw.trim().parse().ok()?;
    if !v.is_finite() {
        return None;
    }
    let deg = (v * 40.0 + 70.0).round() as i32;
    (30..=110).contains(&deg).then_some(deg)
}

pub fn encode_render_distance(chunks: i32) -> Result<String, String> {
    if !(2..=128).contains(&chunks) {
        return Err(format!(
            "Render distance must be between 2 and 128 (got {chunks})."
        ));
    }
    Ok(chunks.to_string())
}

pub fn decode_render_distance(raw: &str) -> Option<i32> {
    let v: i32 = raw.trim().parse().ok()?;
    (2..=128).contains(&v).then_some(v)
}

pub fn encode_simulation_distance(chunks: i32) -> Result<String, String> {
    if !(5..=32).contains(&chunks) {
        return Err(format!(
            "Simulation distance must be between 5 and 32 (got {chunks})."
        ));
    }
    Ok(chunks.to_string())
}

pub fn decode_simulation_distance(raw: &str) -> Option<i32> {
    let v: i32 = raw.trim().parse().ok()?;
    (5..=32).contains(&v).then_some(v)
}

pub fn encode_gui_scale(scale: i32) -> Result<String, String> {
    if scale < 0 {
        return Err(format!("GUI scale must be 0 or greater (got {scale})."));
    }
    Ok(scale.to_string())
}

pub fn decode_gui_scale(raw: &str) -> Option<i32> {
    let v: i32 = raw.trim().parse().ok()?;
    (v >= 0).then_some(v)
}

pub fn encode_max_fps(fps: i32) -> Result<String, String> {
    if !(10..=260).contains(&fps) || fps % 10 != 0 {
        return Err(format!(
            "Max FPS must be a multiple of 10 between 10 and 260 (got {fps})."
        ));
    }
    Ok(fps.to_string())
}

pub fn decode_max_fps(raw: &str) -> Option<i32> {
    let v: i32 = raw.trim().parse().ok()?;
    let snapped = (v / 10) * 10;
    (10..=260).contains(&snapped).then_some(snapped)
}

pub fn encode_mouse_sensitivity(pct: i32) -> Result<String, String> {
    if !(0..=200).contains(&pct) {
        return Err(format!(
            "Mouse sensitivity must be between 0 and 200 (got {pct})."
        ));
    }
    Ok(double_to_string(f64::from(pct) / 200.0))
}

pub fn decode_mouse_sensitivity(raw: &str) -> Option<i32> {
    let v: f64 = raw.trim().parse().ok()?;
    if !v.is_finite() || !(0.0..=1.0).contains(&v) {
        return None;
    }
    Some((v * 200.0).round() as i32)
}

pub fn encode_bool(on: bool) -> String {
    if on { "true" } else { "false" }.to_string()
}

pub fn decode_bool(raw: &str) -> Option<bool> {
    match raw.trim() {
        "true" => Some(true),
        "false" => Some(false),
        _ => None,
    }
}

pub fn validate_lang(code: &str) -> Result<String, String> {
    let trimmed = code.trim();
    if trimmed.is_empty()
        || trimmed.contains(':')
        || trimmed.contains('\r')
        || trimmed.contains('\n')
        || trimmed.contains('"')
    {
        return Err("Language code must be a plain code like ja_jp or en_us.".into());
    }
    Ok(trimmed.to_string())
}

pub fn validate_raw_key(key: &str) -> Result<String, String> {
    let trimmed = key.trim();
    if trimmed.is_empty()
        || trimmed.contains(':')
        || trimmed.contains('\r')
        || trimmed.contains('\n')
    {
        return Err("Option key must not be empty or contain ':' or line breaks.".into());
    }
    Ok(trimmed.to_string())
}

pub fn validate_raw_value(value: &str) -> Result<String, String> {
    if value.contains('\r') || value.contains('\n') {
        return Err("Option value must not contain line breaks.".into());
    }
    Ok(value.to_string())
}

/// Validates a raw `key` and `value` pair against friendly codecs when known, or raw rules otherwise.
pub fn validate_entry(key: &str, value: &str) -> Result<(), String> {
    let key = validate_raw_key(key)?;
    let value = validate_raw_value(value)?;
    if let Some(def) = find_friendly(&key) {
        let valid = match def.kind {
            ControlKind::UnitPercent => decode_unit_percent(&value).is_some(),
            ControlKind::Fov => decode_fov(&value).is_some(),
            ControlKind::RenderDistance => decode_render_distance(&value).is_some(),
            ControlKind::SimulationDistance => decode_simulation_distance(&value).is_some(),
            ControlKind::GuiScale => decode_gui_scale(&value).is_some(),
            ControlKind::MaxFps => decode_max_fps(&value).is_some(),
            ControlKind::MouseSensitivity => decode_mouse_sensitivity(&value).is_some(),
            ControlKind::Bool => decode_bool(&value).is_some(),
            ControlKind::Language => validate_lang(&value).is_ok(),
        };
        if !valid {
            return Err(format!("Invalid value {value:?} for {key}."));
        }
    }
    Ok(())
}

/// Reads `<home>/options.txt` as ordered `(key, value)` pairs. Returns `Ok(None)` if the file
/// does not exist.
pub fn read_options_txt(home: &Path) -> Result<Option<Vec<(String, String)>>, String> {
    let path = home.join(OPTIONS_TXT);
    let text = match fs::read_to_string(&path) {
        Ok(text) => text,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(None),
        Err(e) => return Err(format!("{}: {e}", path.display())),
    };
    let mut pairs = Vec::new();
    for line in split_lines(&text) {
        if let Some((k, v)) = line.split_once(':') {
            if !k.is_empty() {
                pairs.push((k.to_string(), v.to_string()));
            }
        }
    }
    Ok(Some(pairs))
}

/// Loads friendly options (and updates any already-present custom keys) from `<home>/options.txt`,
/// keeping existing `ApplyMode` values for keys already in `existing`.
pub fn load_profile_from_options_txt(
    home: &Path,
    existing: &[OptionEntry],
) -> Result<Option<Vec<OptionEntry>>, String> {
    let Some(pairs) = read_options_txt(home)? else {
        return Ok(None);
    };
    let by_key: HashMap<&str, &str> = pairs
        .iter()
        .map(|(k, v)| (k.as_str(), v.as_str()))
        .collect();
    let existing_modes: HashMap<&str, ApplyMode> =
        existing.iter().map(|e| (e.key.as_str(), e.mode)).collect();
    let mut loaded = Vec::new();
    for def in FRIENDLY {
        if let Some(&raw) = by_key.get(def.key) {
            if validate_entry(def.key, raw).is_ok() {
                let mode = existing_modes.get(def.key).copied().unwrap_or_default();
                loaded.push(OptionEntry {
                    key: def.key.to_string(),
                    value: raw.to_string(),
                    mode,
                });
            }
        }
    }
    for old in existing {
        if find_friendly(&old.key).is_none() {
            if let Some(&raw) = by_key.get(old.key.as_str()) {
                if validate_raw_value(raw).is_ok() {
                    loaded.push(OptionEntry {
                        key: old.key.clone(),
                        value: raw.to_string(),
                        mode: old.mode,
                    });
                }
            }
        }
    }
    Ok(Some(loaded))
}

#[cfg(test)]
#[path = "game_options/tests.rs"]
mod tests;
