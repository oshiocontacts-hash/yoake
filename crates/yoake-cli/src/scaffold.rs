use crate::{Result, reserved_name};
use horizon_mods::yak::check_plain_path;
use std::env;
use std::fs;
use std::path::{Path, PathBuf};
use std::time::{SystemTime, UNIX_EPOCH};

pub(super) fn write_bundled_sdk(root: &Path) -> Result<()> {
    let files = [
        ("yoake-wasm-sdk/Cargo.toml", include_str!("../../yoake-wasm-sdk/Cargo.toml")),
        ("yoake-wasm-sdk/src/lib.rs", include_str!("../../yoake-wasm-sdk/src/lib.rs")),
        ("yoake-wasm-sdk/src/heap.rs", include_str!("../../yoake-wasm-sdk/src/heap.rs")),
        ("yoake-wasm-sdk/src/nbt.rs", include_str!("../../yoake-wasm-sdk/src/nbt.rs")),
        ("yoake-wasm-sdk/src/testing.rs", include_str!("../../yoake-wasm-sdk/src/testing.rs")),
        ("yoake-wasm-sdk/src/testing/engine.rs", include_str!("../../yoake-wasm-sdk/src/testing/engine.rs")),
        ("yoake-wasm-sdk/src/full.rs", include_str!("../../yoake-wasm-sdk/src/full.rs")),
        ("yoake-wasm-sdk/src/full_client_generated.rs", include_str!("../../yoake-wasm-sdk/src/full_client_generated.rs")),
        ("yoake-wasm-sdk/src/full_server_generated.rs", include_str!("../../yoake-wasm-sdk/src/full_server_generated.rs")),
        ("yoake-wasm-sdk/src/full_hooks_generated.rs", include_str!("../../yoake-wasm-sdk/src/full_hooks_generated.rs")),
        ("yoake-wasm-sdk/test-macros/Cargo.toml", include_str!("../../yoake-wasm-sdk/test-macros/Cargo.toml")),
        ("yoake-wasm-sdk/test-macros/src/lib.rs", include_str!("../../yoake-wasm-sdk/test-macros/src/lib.rs")),
        ("yoake-wasm/wit/yoake-mod.wit", include_str!("../../yoake-wasm/wit/yoake-mod.wit")),
        ("yoake-wasm/wit/yoake-full.wit", include_str!("../../yoake-wasm/wit/yoake-full.wit")),
        ("yoake-wasm/wit/yoake-test.wit", include_str!("../../yoake-wasm/wit/yoake-test.wit")),
    ];
    for (name, content) in files {
        let path = root.join(".yoake").join(name);
        let parent = path.parent().ok_or("bundled SDK file has no parent directory")?;
        fs::create_dir_all(parent).map_err(|error| format!("{}: {error}", parent.display()))?;
        fs::write(&path, content).map_err(|error| format!("{}: {error}", path.display()))?;
    }
    Ok(())
}

pub fn create(template: &str, name: &str) -> Result<PathBuf> {
    let description = match template {
        "minigame" => "A client-side starting point for a minigame; server gameplay needs a server mod.",
        "hud-only" => "A client-side HUD starter.",
        "custom-mob" => "A client-side starter with data and model placeholders; it does not register a mob.",
        "utility" => "A client-side utility starter.",
        "script" => "A script placeholder; integration with a yoake-script mod is not implemented.",
        _ => return Err("unknown template; choose minigame, hud-only, custom-mob, utility, or script".into()),
    };
    let starter = match template {
        "minigame" => MINIGAME,
        "hud-only" => HUD_ONLY,
        "custom-mob" => CUSTOM_MOB,
        "utility" => UTILITY,
        "script" => SCRIPT,
        _ => return Err("unknown template".into()),
    };
    if reserved_name(name) {
        return Err(format!("name {name:?} is a reserved Windows name"));
    }
    if !(3..=48).contains(&name.len())
        || !name.bytes().next().is_some_and(|byte| byte.is_ascii_lowercase())
        || !name.bytes().all(|byte| byte.is_ascii_lowercase() || byte.is_ascii_digit() || byte == b'-')
        || name.ends_with('-') || name.contains("--")
    {
        return Err("name must be 3..48 lowercase letters, digits, or single hyphens and start with a letter".into());
    }
    let sdk = if let Some(path) = env::var_os("YOAKE_SDK_PATH") {
        let sdk = PathBuf::from(path);
        let sdk = fs::canonicalize(&sdk).map_err(|error| format!("SDK {}: {error}", sdk.display()))?;
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
    let parent = env::current_dir().map_err(|error| error.to_string())?;
    stage_project(&parent, name, |target| {
        create_project_files(target, template, name, description, starter, sdk)
    })
}

fn create_project_files(
    target: &Path,
    template: &str,
    name: &str,
    description: &str,
    starter: &str,
    sdk: Option<PathBuf>,
) -> Result<()> {
    fs::create_dir(target.join("src")).map_err(|error| error.to_string())?;
    let sdk_path = if let Some(sdk) = sdk {
        serde_json::to_string(&sdk.to_string_lossy()).map_err(|error| error.to_string())?
    } else {
        write_bundled_sdk(target)?;
        serde_json::to_string(".yoake/yoake-wasm-sdk").map_err(|error| error.to_string())?
    };
    let cargo = format!("[package]\nname = \"{name}\"\nversion = \"0.1.0\"\nedition = \"2024\"\npublish = false\n\n[lib]\ncrate-type = [\"cdylib\", \"rlib\"]\n\n[dependencies]\nyoake-wasm-sdk = {{ path = {sdk_path} }}\n\n[features]\n# `yoake test` builds the test module with this feature (it exports the tests instead of the mod).\ntests = [\"yoake-wasm-sdk/test\"]\n\n[profile.release]\nopt-level = \"s\"\nlto = true\nstrip = true\npanic = \"abort\"\n\n[workspace]\nexclude = [\".yoake/yoake-wasm-sdk\"]\n");
    let manifest = format!("format = 1\nid = \"{name}\"\nversion = \"0.1.0\"\nname = \"{name}\"\nauthors = [\"Replace with your name\"]\ndescription = \"{description}\"\n\n[yoake]\nversion = \"*\"\n\n[side]\nclient = true\nserver = false\nrequired_on_join = false\n\n[wasm]\ninterface = \"1.0\"\nclient = \"bin/wasm/{name}.wasm\"\n");
    let readme = format!("# {name}\n\n{description}\n\nEdit the author in `yak.toml`. Install Rust and the `wasm32-wasip2` target; no `wasm-tools` installation is needed. `yoake build` compiles and checks the component. `yoake check` calls `init` using stub hosts and validates its hook subscriptions; it does not run hooks in a game. `yoake pack` builds, checks and writes `dist/{name}-0.1.0.yak` (using the version in `yak.toml`), replacing the previous package of that version. Packages are unsigned; signing is an owner operation. The project includes a copy of the SDK and WIT definitions. Optionally set `YOAKE_SDK_PATH` when creating a project to use a local SDK checkout instead.\n\nFor development, set `YOAKE_WASM_DEV_DIR` to the engine's raw-Wasm developer directory. `yoake dev` builds, checks and installs `{name}.wasm` there, but cannot confirm the engine accepted it. Without that setting it succeeds without installing and reports a note. A newly installed file must be present when the engine starts for later hot reloads; enabling unsigned developer mode is an explicit owner decision.\n\n`cargo test` runs the local mock-driver test, not an engine world. `yoake test` builds the crate with the `tests` Cargo feature (which exports `YOAKE_TESTS` as a test module instead of the mod) and runs every test in the game, each in a fresh temporary world, offscreen: set `YOAKE_GAME` to the game executable. `yoake dev` fills its `tests` array the same way when `YOAKE_GAME` is set. See docs/yoake-wasm-dev.md, Testing a mod in the engine.\n");
    fs::write(target.join("Cargo.toml"), cargo).map_err(|error| error.to_string())?;
    fs::write(target.join("yak.toml"), manifest).map_err(|error| error.to_string())?;
    fs::write(target.join("README.md"), readme).map_err(|error| error.to_string())?;
    let source = STARTER.replace("__STARTER__", starter).replace("__ID__", name);
    fs::write(target.join("src/lib.rs"), source).map_err(|error| error.to_string())?;
    if template == "custom-mob" {
        fs::create_dir_all(target.join("assets/models")).map_err(|error| error.to_string())?;
        fs::create_dir_all(target.join("assets/data")).map_err(|error| error.to_string())?;
        fs::write(target.join("assets/models/README.md"), "Model placeholder; no model format or registration is implied.\n").map_err(|error| error.to_string())?;
        fs::write(target.join("assets/data/README.md"), "Mob data placeholder; no schema or registration is implied.\n").map_err(|error| error.to_string())?;
    }
    if template == "script" {
        fs::create_dir_all(target.join("assets/scripts")).map_err(|error| error.to_string())?;
        fs::write(target.join("assets/scripts/README.md"), "Script placeholder. Confirm yoake-script's format and loader before adding scripts.\n").map_err(|error| error.to_string())?;
    }
    fs::write(target.join(".gitignore"), "target/\ndist/\n.yoake/\n").map_err(|error| error.to_string())?;
    Ok(())
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

pub(super) fn stage_project(parent: &Path, name: &str, write: impl FnOnce(&Path) -> Result<()>) -> Result<PathBuf> {
    check_plain_path(parent)?;
    let target = parent.join(name);
    check_plain_path(&target)?;
    if target.exists() {
        return Err(format!("{} already exists", target.display()));
    }
    let stamp = SystemTime::now().duration_since(UNIX_EPOCH).map_err(|error| error.to_string())?.as_nanos();
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

const CUSTOM_MOB: &str = r##"use yoake_wasm_sdk::{hook, hud, Mod};

#[derive(Default)]
struct Starter;

impl Mod for Starter {
    fn subscriptions() -> &'static [&'static str] { &[hook::HUD] }

    fn hud(&mut self) {
        let _ = hud::text("__ID__: mob placeholder", (10.0, 10.0), 0xffffff, true);
    }
}
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
