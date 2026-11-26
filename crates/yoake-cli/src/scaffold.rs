use crate::{reserved_name, Result};
use horizon_mods::yak::check_plain_path;
use std::env;
use std::fs;
use std::path::{Path, PathBuf};
use std::time::{SystemTime, UNIX_EPOCH};

pub(crate) const TEMPLATES: [&str; 9] = [
    "minigame",
    "hud-only",
    "custom-mob",
    "utility",
    "script",
    "client-full",
    "server-full",
    "worker",
    "full-access",
];

pub(super) fn write_bundled_sdk(root: &Path) -> Result<()> {
    let files: &[(&str, &[u8])] = include!(concat!(env!("OUT_DIR"), "/bundled_sdk.rs"));
    for (name, content) in files {
        let path = root.join(".yoake").join(name);
        let parent = path
            .parent()
            .ok_or("bundled SDK file has no parent directory")?;
        fs::create_dir_all(parent).map_err(|error| format!("{}: {error}", parent.display()))?;
        fs::write(&path, content).map_err(|error| format!("{}: {error}", path.display()))?;
    }
    Ok(())
}

pub fn create(template: &str, name: &str) -> Result<PathBuf> {
    let parent = env::current_dir().map_err(|error| error.to_string())?;
    create_in(&parent, template, name)
}

pub(crate) fn create_in(parent: &Path, template: &str, name: &str) -> Result<PathBuf> {
    let description = match template {
        "minigame" => {
            "A client-side starting point for a minigame; server gameplay needs a server mod."
        }
        "hud-only" => "A client-side HUD starter.",
        "custom-mob" => "A custom F4M mob with Bedrock assets and a server-full ScriptGoal AI.",
        "utility" => "A client-side utility starter.",
        "script" => "A script placeholder; integration with a yoake-script mod is not implemented.",
        "client-full" => "A full-catalog client mod (client-full world).",
        "server-full" => "A full-catalog server mod (server-full world).",
        "worker" => "A server-full mod paired with a WASI p2 worker component for blocking HTTP/TCP I/O.",
        "full-access" => "A full-access core Wasm mod built with yoake-guest-sdk.",
        _ => {
            return Err(format!(
                "unknown template {template:?}; choose {}",
                TEMPLATES.join(", ")
            ));
        }
    };
    if reserved_name(name) {
        return Err(format!("name {name:?} is a reserved Windows name"));
    }
    if !(3..=48).contains(&name.len())
        || !name
            .bytes()
            .next()
            .is_some_and(|byte| byte.is_ascii_lowercase())
        || !name
            .bytes()
            .all(|byte| byte.is_ascii_lowercase() || byte.is_ascii_digit() || byte == b'-')
        || name.ends_with('-')
        || name.contains("--")
    {
        return Err("name must be 3..48 lowercase letters, digits, or single hyphens and start with a letter".into());
    }
    let sdk = if let Some(path) = env::var_os("YOAKE_SDK_PATH") {
        let sdk = PathBuf::from(path);
        let sdk =
            fs::canonicalize(&sdk).map_err(|error| format!("SDK {}: {error}", sdk.display()))?;
        let sdk = match sdk.to_str().and_then(|path| path.strip_prefix(r"\\?\")) {
            Some(plain) if !plain.starts_with("UNC") => PathBuf::from(plain),
            _ => sdk,
        };
        if !sdk.join("Cargo.toml").is_file() {
            return Err(format!("{} is not an SDK crate", sdk.display()));
        }
        Some(sdk)
    } else {
        None
    };
    stage_project(parent, name, |target| {
        create_project_files(target, template, name, description, sdk)
    })
}

fn create_project_files(
    target: &Path,
    template: &str,
    name: &str,
    description: &str,
    sdk: Option<PathBuf>,
) -> Result<()> {
    fs::create_dir(target.join("src")).map_err(|error| error.to_string())?;
    let wasm_sdk_path = if let Some(sdk) = sdk.as_ref() {
        serde_json::to_string(&sdk.to_string_lossy()).map_err(|error| error.to_string())?
    } else {
        write_bundled_sdk(target)?;
        serde_json::to_string(".yoake/yoake-wasm-sdk").map_err(|error| error.to_string())?
    };
    let guest_sdk_path = if let Some(sdk) = sdk.as_ref() {
        let guest = sdk
            .parent()
            .unwrap_or(sdk)
            .join("yoake-guest-sdk");
        serde_json::to_string(&guest.to_string_lossy()).map_err(|error| error.to_string())?
    } else {
        serde_json::to_string(".yoake/yoake-guest-sdk").map_err(|error| error.to_string())?
    };

    let ns = name.replace('-', "_");
    let cargo = match template {
        "custom-mob" | "client-full" | "server-full" => format!(
            "[package]\nname = \"{name}\"\nversion = \"0.1.0\"\nedition = \"2024\"\npublish = false\n\n[lib]\ncrate-type = [\"cdylib\", \"rlib\"]\n\n[dependencies]\nyoake-wasm-sdk = {{ path = {wasm_sdk_path}, features = [\"full\"] }}\n\n[features]\ntests = [\"yoake-wasm-sdk/test\"]\n\n[profile.release]\nopt-level = \"s\"\nlto = true\nstrip = true\npanic = \"abort\"\n\n[workspace]\nexclude = [\".yoake/yoake-wasm-sdk\", \".yoake/yoake-guest-sdk\"]\n"
        ),
        "worker" => format!(
            "[package]\nname = \"{name}\"\nversion = \"0.1.0\"\nedition = \"2024\"\npublish = false\n\n[lib]\ncrate-type = [\"cdylib\", \"rlib\"]\n\n[dependencies]\nyoake-wasm-sdk = {{ path = {wasm_sdk_path}, default-features = false, features = [\"std\"] }}\n\n[features]\nserver = [\"yoake-wasm-sdk/full\"]\nworker = [\"yoake-wasm-sdk/worker\"]\ntests = [\"yoake-wasm-sdk/test\"]\n\n[profile.release]\nopt-level = \"s\"\nlto = true\nstrip = true\npanic = \"abort\"\n\n[workspace]\nexclude = [\".yoake/yoake-wasm-sdk\", \".yoake/yoake-guest-sdk\"]\n"
        ),
        "full-access" => format!(
            "[package]\nname = \"{name}\"\nversion = \"0.1.0\"\nedition = \"2024\"\npublish = false\n\n[lib]\ncrate-type = [\"cdylib\"]\n\n[dependencies]\nyoake-guest-sdk = {{ path = {guest_sdk_path} }}\n\n[profile.release]\nopt-level = \"s\"\nlto = true\nstrip = true\npanic = \"abort\"\n\n[workspace]\nexclude = [\".yoake/yoake-wasm-sdk\", \".yoake/yoake-guest-sdk\"]\n"
        ),
        _ => format!(
            "[package]\nname = \"{name}\"\nversion = \"0.1.0\"\nedition = \"2024\"\npublish = false\n\n[lib]\ncrate-type = [\"cdylib\", \"rlib\"]\n\n[dependencies]\nyoake-wasm-sdk = {{ path = {wasm_sdk_path} }}\n\n[features]\n# `yoake test` builds the test module with this feature (it exports the tests instead of the mod).\ntests = [\"yoake-wasm-sdk/test\"]\n\n[profile.release]\nopt-level = \"s\"\nlto = true\nstrip = true\npanic = \"abort\"\n\n[workspace]\nexclude = [\".yoake/yoake-wasm-sdk\", \".yoake/yoake-guest-sdk\"]\n"
        ),
    };

    let manifest = match template {
        "custom-mob" => format!(
            "format = 1\nid = \"{ns}\"\nversion = \"0.1.0\"\nname = \"{name}\"\nauthors = [\"Replace with your name\"]\ndescription = \"{description}\"\n\n[yoake]\nversion = \"*\"\n\n[side]\nclient = true\nserver = true\nrequired_on_join = false\n\n[wasm]\ninterface = \"1.7\"\nserver = \"bin/wasm/{name}.wasm\"\npermissions = [\"service.mobs\"]\n"
        ),
        "client-full" => format!(
            "format = 1\nid = \"{name}\"\nversion = \"0.1.0\"\nname = \"{name}\"\nauthors = [\"Replace with your name\"]\ndescription = \"{description}\"\n\n[yoake]\nversion = \"*\"\n\n[side]\nclient = true\nserver = false\nrequired_on_join = false\n\n[wasm]\ninterface = \"1.7\"\nworld = \"client-full\"\nclient = \"bin/wasm/{name}.wasm\"\n"
        ),
        "server-full" => format!(
            "format = 1\nid = \"{name}\"\nversion = \"0.1.0\"\nname = \"{name}\"\nauthors = [\"Replace with your name\"]\ndescription = \"{description}\"\n\n[yoake]\nversion = \"*\"\n\n[side]\nclient = false\nserver = true\nrequired_on_join = false\n\n[wasm]\ninterface = \"1.7\"\nserver = \"bin/wasm/{name}.wasm\"\n"
        ),
        "worker" => format!(
            "format = 1\nid = \"{name}\"\nversion = \"0.1.0\"\nname = \"{name}\"\nauthors = [\"Replace with your name\"]\ndescription = \"{description}\"\n\n[yoake]\nversion = \"*\"\n\n[side]\nclient = false\nserver = true\nrequired_on_join = false\n\n[wasm]\ninterface = \"1.7\"\nserver = \"bin/wasm/{name}.wasm\"\nworker = \"bin/wasm/{name}-worker.wasm\"\n"
        ),
        "full-access" => format!(
            "format = 1\nid = \"{name}\"\nversion = \"0.1.0\"\nname = \"{name}\"\nauthors = [\"Replace with your name\"]\ndescription = \"{description}\"\n\n[yoake]\nversion = \"*\"\n\n[side]\nclient = false\nserver = true\nrequired_on_join = false\n\n[wasm]\ninterface = \"1.7\"\nworld = \"full-access\"\nserver = \"bin/wasm/{name}.wasm\"\n"
        ),
        _ => format!(
            "format = 1\nid = \"{name}\"\nversion = \"0.1.0\"\nname = \"{name}\"\nauthors = [\"Replace with your name\"]\ndescription = \"{description}\"\n\n[yoake]\nversion = \"*\"\n\n[side]\nclient = true\nserver = false\nrequired_on_join = false\n\n[wasm]\ninterface = \"1.7\"\nclient = \"bin/wasm/{name}.wasm\"\n"
        ),
    };

    let readme = format!(
        "# {name}\n\n{description}\n\nEdit the author in `yak.toml`. Install Rust and the `wasm32-wasip2` target; no `wasm-tools` installation is needed. `yoake build` compiles the modules and `yoake check` validates them offline. `yoake pack` builds, checks and writes `dist/{name}-0.1.0.yak`.\n"
    );

    let source = match template {
        "custom-mob" => CUSTOM_MOB.replace("__NS__", &ns).replace("__ID__", name),
        "client-full" => CLIENT_FULL.replace("__ID__", name),
        "server-full" => SERVER_FULL.replace("__ID__", name),
        "worker" => WORKER_TEMPLATE.replace("__ID__", name),
        "full-access" => FULL_ACCESS_TEMPLATE.replace("__ID__", name),
        "minigame" => STARTER.replace("__STARTER__", MINIGAME).replace("__ID__", name),
        "hud-only" => STARTER.replace("__STARTER__", HUD_ONLY).replace("__ID__", name),
        "utility" => STARTER.replace("__STARTER__", UTILITY).replace("__ID__", name),
        "script" => STARTER.replace("__STARTER__", SCRIPT).replace("__ID__", name),
        _ => return Err("unknown template".into()),
    };

    fs::write(target.join("Cargo.toml"), cargo).map_err(|error| error.to_string())?;
    fs::write(target.join("yak.toml"), manifest).map_err(|error| error.to_string())?;
    fs::write(target.join("README.md"), readme).map_err(|error| error.to_string())?;
    fs::write(target.join("src/lib.rs"), source).map_err(|error| error.to_string())?;

    if template == "custom-mob" {
        write_custom_mob_content(target, &ns, name)?;
    }
    if template == "script" {
        fs::create_dir_all(target.join("assets/scripts")).map_err(|error| error.to_string())?;
        fs::write(
            target.join("assets/scripts/README.md"),
            "Script placeholder. Confirm yoake-script's format and loader before adding scripts.\n",
        )
        .map_err(|error| error.to_string())?;
    }
    fs::write(target.join(".gitignore"), "target/\ndist/\n.yoake/\n")
        .map_err(|error| error.to_string())?;
    Ok(())
}

fn write_custom_mob_content(target: &Path, ns: &str, name: &str) -> Result<()> {
    let content = target.join("content");
    let entity_dir = content.join("data").join(ns).join("horizon/entity_type");
    let assets_ns = content.join("assets").join(ns);
    let geo_dir = assets_ns.join("geo");
    let anim_dir = assets_ns.join("animations");
    let ctrl_dir = assets_ns.join("controllers");
    let tex_dir = assets_ns.join("textures/entity");
    for dir in [&entity_dir, &geo_dir, &anim_dir, &ctrl_dir, &tex_dir] {
        fs::create_dir_all(dir).map_err(|error| error.to_string())?;
    }
    fs::write(
        content.join("horizon-mod.toml"),
        format!(
            "id = \"{ns}\"\nname = \"{name}\"\nversion = \"0.1.0\"\napi = \"0.2\"\nside = \"both\"\ncapabilities = [\"registries\"]\n"
        ),
    )
    .map_err(|error| error.to_string())?;

    let entity_json = format!(
        r#"{{
  "template": "yoake:mob",
  "format": 1,
  "category": "creature",
  "size": {{ "width": 0.6, "height": 0.7, "eye_height": 0.5 }},
  "hitboxes": [
    {{ "name": "body", "size": [0.6, 0.7] }}
  ],
  "attributes": {{
    "max_health": 14,
    "movement_speed": 0.3
  }},
  "model": {{
    "geo": "{ns}:geo/starter_mob",
    "textures": {{ "default": "{ns}:entity/starter_mob" }},
    "animations": ["{ns}:animations/starter_mob"],
    "controllers": ["{ns}:controllers/starter_mob"],
    "scale": 1.0,
    "shadow": {{ "radius": 0.4, "strength": 0.8 }}
  }},
  "synced": {{
    "state": {{ "type": "enum", "values": ["idle", "pounce"], "default": "idle", "persistent": true }}
  }},
  "ai": {{
    "navigator": "ground",
    "memories": {{
      "pounced": {{ "type": "bool" }}
    }},
    "goals": [
      {{ "type": "swim", "priority": 0 }},
      {{ "type": "script", "id": "pounce", "priority": 2 }},
      {{ "type": "wander", "priority": 6, "speed": 1.0 }},
      {{ "type": "look_at_player", "priority": 7 }},
      {{ "type": "random_look", "priority": 8 }}
    ],
    "script_goals": [
      {{ "id": "pounce", "priority": 2, "flags": ["move", "look"], "interval": 10 }}
    ]
  }},
  "spawn_egg": {{ "primary": "0xD2691E", "secondary": "0xFFF0A0" }},
  "sounds": {{ "ambient": "minecraft:entity.fox.ambient", "hurt": "minecraft:entity.fox.hurt", "death": "minecraft:entity.fox.death" }}
}}
"#
    );
    fs::write(entity_dir.join("starter_mob.json"), entity_json)
        .map_err(|error| error.to_string())?;

    let geo_json = r#"{
  "format_version": "1.12.0",
  "minecraft:geometry": [
    {
      "description": {
        "identifier": "geometry.starter_mob",
        "texture_width": 64,
        "texture_height": 32,
        "visible_bounds_width": 2,
        "visible_bounds_height": 2,
        "visible_bounds_offset": [0, 0.5, 0]
      },
      "bones": [
        { "name": "root", "pivot": [0, 0, 0] },
        {
          "name": "body",
          "parent": "root",
          "pivot": [0, 5, 0],
          "cubes": [{ "origin": [-3, 3, -5], "size": [6, 5, 10], "uv": [0, 0] }]
        }
      ]
    }
  ]
}
"#;
    fs::write(geo_dir.join("starter_mob.geo.json"), geo_json)
        .map_err(|error| error.to_string())?;

    let anim_json = r#"{
  "format_version": "1.8.0",
  "animations": {
    "animation.starter_mob.idle": {
      "loop": true,
      "animation_length": 1.0,
      "bones": {}
    },
    "animation.starter_mob.pounce": {
      "loop": false,
      "animation_length": 0.6,
      "bones": {}
    }
  }
}
"#;
    fs::write(anim_dir.join("starter_mob.animation.json"), anim_json)
        .map_err(|error| error.to_string())?;

    let ctrl_json = r#"{
  "controllers": {
    "locomotion": {
      "initial": "idle",
      "states": {
        "idle": {
          "animations": ["idle"]
        }
      }
    },
    "action": {
      "weight": 1,
      "triggers": {
        "pounce": "pounce"
      }
    }
  }
}
"#;
    fs::write(ctrl_dir.join("starter_mob.json"), ctrl_json).map_err(|error| error.to_string())?;
    fs::write(tex_dir.join("starter_mob.png"), solid_rgba_png(64, 32, [0xD2, 0x69, 0x1E, 0xFF]))
        .map_err(|error| error.to_string())?;
    Ok(())
}

/// Generates a minimal valid uncompressed RGBA PNG (`width` x `height`).
fn solid_rgba_png(width: u32, height: u32, rgba: [u8; 4]) -> Vec<u8> {
    fn crc32(data: &[u8]) -> u32 {
        let mut crc = 0xffff_ffffu32;
        for &byte in data {
            crc ^= u32::from(byte);
            for _ in 0..8 {
                crc = if crc & 1 != 0 {
                    (crc >> 1) ^ 0xedb8_8320
                } else {
                    crc >> 1
                };
            }
        }
        !crc
    }
    fn adler32(data: &[u8]) -> u32 {
        let (mut a, mut b) = (1u32, 0u32);
        for &byte in data {
            a = (a + u32::from(byte)) % 65_521;
            b = (b + a) % 65_521;
        }
        (b << 16) | a
    }
    fn push_chunk(out: &mut Vec<u8>, tag: &[u8; 4], payload: &[u8]) {
        out.extend_from_slice(&(payload.len() as u32).to_be_bytes());
        out.extend_from_slice(tag);
        out.extend_from_slice(payload);
        let mut crc_input = Vec::with_capacity(4 + payload.len());
        crc_input.extend_from_slice(tag);
        crc_input.extend_from_slice(payload);
        out.extend_from_slice(&crc32(&crc_input).to_be_bytes());
    }

    let mut raw = Vec::with_capacity((1 + width as usize * 4) * height as usize);
    for _ in 0..height {
        raw.push(0); // filter: None
        for _ in 0..width {
            raw.extend_from_slice(&rgba);
        }
    }
    let mut zlib = Vec::with_capacity(raw.len() + 11);
    zlib.extend_from_slice(&[0x78, 0x01, 0x01]);
    let len = raw.len() as u16;
    zlib.extend_from_slice(&len.to_le_bytes());
    zlib.extend_from_slice(&(!len).to_le_bytes());
    zlib.extend_from_slice(&raw);
    zlib.extend_from_slice(&adler32(&raw).to_be_bytes());

    let mut png = Vec::new();
    png.extend_from_slice(b"\x89PNG\r\n\x1a\n");
    let mut ihdr = Vec::with_capacity(13);
    ihdr.extend_from_slice(&width.to_be_bytes());
    ihdr.extend_from_slice(&height.to_be_bytes());
    ihdr.extend_from_slice(&[8, 6, 0, 0, 0]);
    push_chunk(&mut png, b"IHDR", &ihdr);
    push_chunk(&mut png, b"IDAT", &zlib);
    push_chunk(&mut png, b"IEND", &[]);
    png
}

struct Staging(Option<PathBuf>);

impl Drop for Staging {
    fn drop(&mut self) {
        if let Some(path) = &self.0 {
            if check_plain_path(path).is_ok() {
                let _ = fs::remove_dir_all(path);
            }
        }
    }
}

pub(super) fn stage_project(
    parent: &Path,
    name: &str,
    write: impl FnOnce(&Path) -> Result<()>,
) -> Result<PathBuf> {
    check_plain_path(parent)?;
    let target = parent.join(name);
    check_plain_path(&target)?;
    if target.exists() {
        return Err(format!("{} already exists", target.display()));
    }
    let stamp = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_err(|error| error.to_string())?
        .as_nanos();
    let path = parent.join(format!(".yoake-{name}-{}-{stamp}.tmp", std::process::id()));
    check_plain_path(&path)?;
    fs::create_dir(&path).map_err(|error| format!("{}: {error}", path.display()))?;
    let mut staging = Staging(Some(path.clone()));
    write(&path)?;
    if target.exists() {
        return Err(format!("{} already exists", target.display()));
    }
    fs::rename(&path, &target).map_err(|error| format!("{}: {error}", target.display()))?;
    staging.0 = None;
    Ok(target)
}

const MINIGAME: &str = r##"use yoake_wasm_sdk::{hook, hud, Mod};

#[derive(Default)]
struct Starter {
    ticks: u64,
}

impl Mod for Starter {
    fn subscriptions() -> &'static [&'static str] { &[hook::TICK, hook::HUD] }

    fn tick(&mut self) { self.ticks = self.ticks.saturating_add(1); }

    fn hud(&mut self) {
        let _ = hud::text(&format!("__ID__: {}", self.ticks), (10.0, 10.0), 0xffffff, true);
    }
}
"##;

const HUD_ONLY: &str = r##"use yoake_wasm_sdk::{hook, hud, Mod};

#[derive(Default)]
struct Starter;

impl Mod for Starter {
    fn subscriptions() -> &'static [&'static str] { &[hook::HUD] }

    fn hud(&mut self) {
        let _ = hud::text("__ID__", (10.0, 10.0), 0xffffff, true);
    }
}
"##;

const CUSTOM_MOB: &str = r##"//! Starter custom mob AI for `__NS__:starter_mob` (F4M `service.mobs`).
use yoake_wasm_sdk::full::hook;
use yoake_wasm_sdk::full::server::{mobs, ServerMod};

const MOB_ID: &str = "__NS__:starter_mob";

#[derive(Default)]
struct Starter {
    pouncing: Vec<(u32, u32)>,
}

impl ServerMod for Starter {
    fn subscriptions() -> &'static [&'static str] {
        &[
            hook::MOB_GOAL_CAN_USE,
            hook::MOB_GOAL_CAN_CONTINUE,
            hook::MOB_GOAL_START,
            hook::MOB_GOAL_TICK,
            hook::MOB_GOAL_STOP,
        ]
    }

    fn mob_goal_can_use(&mut self, mob: u32, goal: String) -> bool {
        if goal != "pounce" {
            return false;
        }
        let Ok(Some(info)) = mobs::info(mob) else {
            return false;
        };
        info.type_id == MOB_ID && info.on_ground && info.target_visible
    }

    fn mob_goal_can_continue(&mut self, mob: u32, goal: String) -> bool {
        goal == "pounce"
            && self
                .pouncing
                .iter()
                .any(|(id, ticks)| *id == mob && *ticks < 12)
    }

    fn mob_goal_start(&mut self, mob: u32, goal: String) {
        if goal != "pounce" {
            return;
        }
        self.pouncing.retain(|(id, _)| *id != mob);
        self.pouncing.push((mob, 0));
        let _ = mobs::navigate_stop(mob);
        let _ = mobs::trigger_anim(mob, "", "pounce", 1.0);
    }

    fn mob_goal_tick(&mut self, mob: u32, goal: String) {
        if goal == "pounce" {
            if let Some((_, ticks)) = self.pouncing.iter_mut().find(|(id, _)| *id == mob) {
                *ticks += 1;
            }
        }
    }

    fn mob_goal_stop(&mut self, mob: u32, goal: String) {
        if goal == "pounce" {
            self.pouncing.retain(|(id, _)| *id != mob);
        }
    }
}

#[cfg(all(target_arch = "wasm32", not(feature = "tests")))]
yoake_wasm_sdk::export_server_full!(Starter);

#[cfg(all(target_arch = "wasm32", feature = "tests"))]
yoake_wasm_sdk::export_tests!(YOAKE_TESTS);

pub const YOAKE_TESTS: &[yoake_wasm_sdk::testing::TestCase] = &[];
"##;

const CLIENT_FULL: &str = r##"use yoake_wasm_sdk::full::client::{et, ClientMod};
use yoake_wasm_sdk::full::hook;

#[derive(Default)]
struct Starter {
    ticks: u64,
}

impl ClientMod for Starter {
    fn subscriptions() -> &'static [&'static str] {
        &[hook::TICK, hook::HUD]
    }

    fn tick(&mut self) {
        self.ticks = self.ticks.saturating_add(1);
    }

    fn hud(&mut self) -> et::HudOutcome {
        et::HudOutcome {
            batch_commands: vec![et::HudCommand::Text(et::HudText {
                text: format!("__ID__: {}", self.ticks),
                x: 8.0,
                y: 8.0,
                color: 0xffff_ffff,
                shadow: true,
            })],
            corners_commands: Vec::new(),
        }
    }
}

#[cfg(all(target_arch = "wasm32", not(feature = "tests")))]
yoake_wasm_sdk::export_client_full!(Starter);

#[cfg(all(target_arch = "wasm32", feature = "tests"))]
yoake_wasm_sdk::export_tests!(YOAKE_TESTS);

pub const YOAKE_TESTS: &[yoake_wasm_sdk::testing::TestCase] = &[];
"##;

const SERVER_FULL: &str = r##"use yoake_wasm_sdk::full::hook;
use yoake_wasm_sdk::full::server::{services, ServerMod};

#[derive(Default)]
struct Starter {
    ticks: u64,
}

impl ServerMod for Starter {
    fn subscriptions() -> &'static [&'static str] {
        &[hook::TICK]
    }

    fn init(&mut self) {
        let _ = yoake_wasm_sdk::log("__ID__", "server-full initialized");
    }

    fn tick(&mut self) {
        self.ticks = self.ticks.saturating_add(1);
        if self.ticks == 1 {
            let _ = services::server_players();
        }
    }
}

#[cfg(all(target_arch = "wasm32", not(feature = "tests")))]
yoake_wasm_sdk::export_server_full!(Starter);

#[cfg(all(target_arch = "wasm32", feature = "tests"))]
yoake_wasm_sdk::export_tests!(YOAKE_TESTS);

pub const YOAKE_TESTS: &[yoake_wasm_sdk::testing::TestCase] = &[];
"##;

const WORKER_TEMPLATE: &str = r##"#[cfg(feature = "server")]
mod server_mod {
    use yoake_wasm_sdk::full::hook;
    use yoake_wasm_sdk::full::server::{worker, ServerMod};

    #[derive(Default)]
    struct Starter;

    impl ServerMod for Starter {
        fn subscriptions() -> &'static [&'static str] {
            &[hook::TICK]
        }

        fn init(&mut self) {
            let _ = worker::send(b"__ID__:ping".to_vec());
        }

        fn tick(&mut self) {
            while let Some(reply) = worker::recv() {
                let _ = yoake_wasm_sdk::log("__ID__", &String::from_utf8_lossy(&reply));
            }
        }
    }

    yoake_wasm_sdk::export_server_full!(Starter);
}

#[cfg(feature = "worker")]
mod worker_mod {
    use yoake_wasm_sdk::worker::{self, Worker};

    #[derive(Default)]
    struct StarterWorker;

    impl Worker for StarterWorker {
        fn run(&mut self) {
            while let Some(msg) = worker::recv() {
                let _ = worker::send(&msg);
            }
        }
    }

    yoake_wasm_sdk::export_worker!(StarterWorker);
}
"##;

const FULL_ACCESS_TEMPLATE: &str = r##"#![no_std]
extern crate alloc;

use yoake_guest_sdk::{export_guest, hooks, original, Handle, Registrar};

fn init(r: &mut Registrar) {
    let _ = r.hook(hooks::SERVER_TICK, 0);
}

fn hook(_id: u32, root: Handle) -> i64 {
    original(root).unwrap_or(0)
}

export_guest!(init, hook);
"##;

const UTILITY: &str = r##"use yoake_wasm_sdk::{hook, Mod};

#[derive(Default)]
struct Starter;

impl Mod for Starter {
    fn subscriptions() -> &'static [&'static str] { &[hook::KEY] }

    fn key(&mut self, key: &str, pressed: bool) {
        if pressed {
            let _ = yoake_wasm_sdk::log("__ID__", key);
        }
    }
}
"##;

const SCRIPT: &str = r##"use yoake_wasm_sdk::Mod;

#[derive(Default)]
struct Starter;

impl Mod for Starter {}
"##;

const STARTER: &str = r##"__STARTER__
use yoake_wasm_sdk::testing::{BlockPos, TestCase, TestResult, TestWorld};

#[cfg(all(target_arch = "wasm32", not(feature = "tests")))]
yoake_wasm_sdk::export_mod!(Starter);

#[cfg(all(target_arch = "wasm32", feature = "tests"))]
yoake_wasm_sdk::export_tests!(YOAKE_TESTS);

#[yoake_wasm_sdk::yoake_test]
fn example_world_test(world: &mut TestWorld<'_>) -> TestResult {
    let position = BlockPos { x: 0, y: 64, z: 0 };
    world.place_block(position, "minecraft:stone")?;
    world.run_ticks(1)?;
    world.assert_block(position, "minecraft:stone")
}

pub const YOAKE_TESTS: &[TestCase] = &[TestCase { name: "example_world_test", run: example_world_test }];

#[cfg(test)]
mod tests {
    use super::*;
    use yoake_wasm_sdk::testing::TestDriver;

    #[derive(Default)]
    struct MockWorld { block: Option<(BlockPos, String)>, ticks: u32 }

    impl TestDriver for MockWorld {
        fn place_block(&mut self, position: BlockPos, block: &str) -> TestResult {
            self.block = Some((position, block.to_owned()));
            Ok(())
        }
        fn block_at(&mut self, position: BlockPos) -> Result<String, String> {
            self.block.as_ref().filter(|(at, _)| *at == position).map(|(_, block)| block.clone()).ok_or_else(|| "missing block".to_owned())
        }
        fn spawn_entity(&mut self, _kind: &str, _position: BlockPos) -> Result<u64, String> { Ok(1) }
        fn run_ticks(&mut self, ticks: u32) -> TestResult { self.ticks += ticks; Ok(()) }
    }

    #[test]
    fn example_world_scenario() {
        let mut world = MockWorld::default();
        YOAKE_TESTS[0].run(&mut world).unwrap();
        assert_eq!(world.ticks, 1);
    }
}
"##;
