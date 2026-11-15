use crate::Result;
use std::env;
use std::fs;
use std::path::PathBuf;

pub fn create(template: &str, name: &str) -> Result<()> {
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
    if !(3..=48).contains(&name.len())
        || !name.bytes().next().is_some_and(|byte| byte.is_ascii_lowercase())
        || !name.bytes().all(|byte| byte.is_ascii_lowercase() || byte.is_ascii_digit() || byte == b'-')
        || name.ends_with('-') || name.contains("--")
    {
        return Err("name must be 3..48 lowercase letters, digits, or single hyphens and start with a letter".into());
    }
    let sdk = env::var_os("YOAKE_SDK_PATH").map(PathBuf::from).unwrap_or_else(|| {
        PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../yoake-wasm-sdk")
    });
    let sdk = fs::canonicalize(&sdk).map_err(|error| format!("SDK {}: {error}", sdk.display()))?;
    // Windows canonical paths carry the verbatim `\\?\` prefix; Cargo accepts it but other tools and readers do not.
    let sdk = match sdk.to_str().and_then(|path| path.strip_prefix(r"\\?\")) {
        Some(plain) if !plain.starts_with("UNC") => PathBuf::from(plain),
        _ => sdk,
    };
    if !sdk.join("Cargo.toml").is_file() {
        return Err(format!("{} is not an SDK crate", sdk.display()));
    }
    let target = PathBuf::from(name);
    fs::create_dir(&target).map_err(|error| format!("{}: {error}", target.display()))?;
    fs::create_dir(target.join("src")).map_err(|error| error.to_string())?;
    let sdk_path = serde_json::to_string(&sdk.to_string_lossy()).map_err(|error| error.to_string())?;
    let cargo = format!("[package]\nname = \"{name}\"\nversion = \"0.1.0\"\nedition = \"2024\"\npublish = false\n\n[lib]\ncrate-type = [\"cdylib\", \"rlib\"]\n\n[dependencies]\nyoake-wasm-sdk = {{ path = {sdk_path} }}\n\n[profile.release]\nopt-level = \"s\"\nlto = true\nstrip = true\npanic = \"abort\"\n\n[workspace]\n");
    let manifest = format!("format = 1\nid = \"{name}\"\nversion = \"0.1.0\"\nname = \"{name}\"\nauthors = [\"Replace with your name\"]\ndescription = \"{description}\"\n\n[yoake]\nversion = \"*\"\n\n[side]\nclient = true\nserver = false\nrequired_on_join = false\n\n[wasm]\ninterface = \"1.0\"\nclient = \"bin/wasm/{name}.wasm\"\n");
    let readme = format!("# {name}\n\n{description}\n\nEdit the author in `yak.toml`. `yoake build` compiles and packs an unsigned development package; signing is an owner operation. Set `YOAKE_SDK_PATH` when creating a project to use a different local SDK. The generated Cargo manifest stores that SDK's absolute path.\n\nFor hot reload, set `YOAKE_WASM_DEV_DIR` to the engine's raw-Wasm developer directory. The `{name}.wasm` file must already be present when the engine starts; enabling unsigned developer mode is an explicit owner decision. `yoake dev` builds and updates that file but cannot confirm the engine accepted it.\n\n`cargo test` runs the local mock-driver test, not an engine world. `YOAKE_TESTS` registers an example for a future engine runner; the current engine cannot discover or run it. `yoake dev` reports that missing runner instead of claiming success.\n");
    let readme = format!("{readme}\n`yoake check` runs preflight checks but exits unsuccessfully until full WIT ABI verification is available.\n");
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
    println!("created {}", target.display());
    Ok(())
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

#[cfg(target_arch = "wasm32")]
yoake_wasm_sdk::export_mod!(Starter);

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
