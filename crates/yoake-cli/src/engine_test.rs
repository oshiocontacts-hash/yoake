//! `yoake test` and the `tests` of `yoake dev`: run the project's test module in the game
//! (`horizon-client --yoake-test`, `docs/yoake-wasm-dev.md`, "Testing a mod in the engine").
//!
//! The test module is a second component built from the same crate with the Cargo feature `tests`
//! (the crate calls `yoake_wasm_sdk::export_tests!` under `#[cfg(feature = "tests")]`) and placed at
//! `bin/wasm/<id>-tests.wasm`. It is never declared in `yak.toml` and never packed.
use super::*;
use serde_json::Value;

/// The Cargo feature that builds the test module.
const TESTS_FEATURE: &str = "tests";

#[derive(Debug, Default, PartialEq)]
pub(crate) struct TestSettings {
    pub game: Option<PathBuf>,
    pub module: Option<PathBuf>,
    pub filter: Option<String>,
    pub timeout_ms: Option<String>,
    pub keep: bool,
    /// Arguments after `--`, handed to the game as they are (`--resources PATH`, `--headless`, ...).
    pub extra: Vec<String>,
}

pub(crate) fn parse(arguments: &[String]) -> Result<TestSettings> {
    let mut settings = TestSettings::default();
    let mut iterator = arguments.iter();
    while let Some(argument) = iterator.next() {
        let mut value = |flag: &str| iterator.next().cloned().ok_or_else(|| format!("missing value after {flag}"));
        match argument.as_str() {
            "--game" => settings.game = Some(PathBuf::from(value("--game")?)),
            "--module" => settings.module = Some(PathBuf::from(value("--module")?)),
            "--filter" => settings.filter = Some(value("--filter")?),
            "--timeout-ms" => settings.timeout_ms = Some(value("--timeout-ms")?),
            "--keep" => settings.keep = true,
            "--" => {
                settings.extra = iterator.by_ref().cloned().collect();
                break;
            }
            other => return Err(format!("unknown option {other}; {USAGE}")),
        }
    }
    Ok(settings)
}

/// The game to run: `--game`, else the `YOAKE_GAME` environment variable.
pub(crate) fn game_executable(settings: &TestSettings, environment: Option<&OsStr>) -> Option<PathBuf> {
    settings.game.clone().or_else(|| environment.filter(|value| !value.is_empty()).map(PathBuf::from))
}

/// Where the test module of `manifest`'s project is: `bin/wasm/<id>-tests.wasm`.
pub(crate) fn module_path(root: &Path, manifest: &Manifest) -> PathBuf {
    root.join("bin").join("wasm").join(format!("{}-tests.wasm", manifest.id))
}

/// Builds the test module when the crate has the `tests` feature; otherwise a module already at the
/// default path (built some other way) is used. `None`: the project has no tests.
pub(crate) fn build_module(root: &Path, manifest: &Manifest, given: Option<&Path>) -> Result<Option<PathBuf>> {
    if let Some(given) = given {
        return if given.is_file() { Ok(Some(given.to_path_buf())) } else { Err(format!("{} is not a file", given.display())) };
    }
    let destination = module_path(root, manifest);
    check_plain_path(&destination)?;
    let cargo_path = root.join("Cargo.toml");
    let has_feature = match fs::read_to_string(&cargo_path) {
        Ok(text) => {
            let cargo: toml::Table = text.parse().map_err(|error| format!("Cargo.toml: {error}"))?;
            let name = cargo.get("package").and_then(|package| package.get("name")).and_then(toml::Value::as_str).ok_or("Cargo.toml needs package.name")?;
            let library = cargo.get("lib").and_then(|library| library.get("name")).and_then(toml::Value::as_str).unwrap_or(name).replace('-', "_");
            let features = cargo.get("features").and_then(toml::Value::as_table);
            if features.is_some_and(|features| features.contains_key(TESTS_FEATURE)) {
                let target = target_directory(root, env::var_os("CARGO_TARGET_DIR").as_deref());
                let artifact = target.join("wasm32-wasip2").join("release").join(format!("{library}.wasm"));
                cargo_wasm(root, &target, Some(TESTS_FEATURE), "the tests", &artifact, &destination)?;
                true
            } else {
                false
            }
        }
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => false,
        Err(error) => return Err(format!("Cargo.toml: {error}")),
    };
    Ok((has_feature || destination.is_file()).then_some(destination))
}

/// The report line of the game: its last standard-output line that is a JSON object.
pub(crate) fn parse_report(stdout: &str) -> Result<Value> {
    stdout
        .lines()
        .rev()
        .map(str::trim)
        .find(|line| line.starts_with('{'))
        .ok_or_else(|| "the game printed no report".to_owned())
        .and_then(|line| serde_json::from_str(line).map_err(|error| format!("the game's report is not JSON: {error}")))
}

/// What a run came to, for `yoake test` and `yoake dev`.
#[derive(Debug, Default, PartialEq)]
pub(crate) struct TestRun {
    pub tests: Vec<Value>,
    pub errors: Vec<String>,
    pub notes: Vec<String>,
}

/// The tests of a game report and what to say about the failures.
pub(crate) fn summarize(report: &Value) -> TestRun {
    let tests = report.get("tests").and_then(Value::as_array).cloned().unwrap_or_default();
    let mut errors: Vec<String> = report
        .get("errors")
        .and_then(Value::as_array)
        .map(|errors| errors.iter().filter_map(Value::as_str).map(str::to_owned).collect())
        .unwrap_or_default();
    for test in &tests {
        if test.get("ok").and_then(Value::as_bool) != Some(true) {
            let name = test.get("name").and_then(Value::as_str).unwrap_or("?");
            let why = test.get("error").and_then(Value::as_str).unwrap_or("failed");
            errors.push(format!("test {name} failed: {why}"));
        }
    }
    if errors.is_empty() && report.get("ok").and_then(Value::as_bool) != Some(true) {
        errors.push("the game reported a failed run".to_owned());
    }
    let notes = report.get("note").and_then(Value::as_str).map(|note| vec![note.to_owned()]).unwrap_or_default();
    TestRun { tests, errors, notes }
}

/// Runs the project's tests in the game and returns its report. `lead` are arguments before the game's own
/// (empty, except in the tests of this module).
pub(crate) fn run_game(root: &Path, game: &Path, lead: &[String], module: &Path, settings: &TestSettings) -> Result<Value> {
    let mut command = Command::new(game);
    command.args(lead).arg("--yoake-test").arg(root).arg("--yoake-test-module").arg(module);
    if let Some(filter) = &settings.filter {
        command.arg("--yoake-test-filter").arg(filter);
    }
    if let Some(timeout) = &settings.timeout_ms {
        command.arg("--yoake-test-timeout-ms").arg(timeout);
    }
    if settings.keep {
        command.arg("--yoake-test-keep");
    }
    command.args(&settings.extra).stdin(std::process::Stdio::null()).stderr(std::process::Stdio::inherit());
    let output = command.output().map_err(|error| format!("could not start {}: {error}", game.display()))?;
    let stdout = String::from_utf8_lossy(&output.stdout);
    parse_report(&stdout).map_err(|error| format!("{error} (exit {:?})", output.status.code()))
}

/// The tests of `yoake dev`: nothing to do (a note) without a test module or a game, else the game's run.
pub(crate) fn dev_tests(root: &Path, manifest: &Manifest) -> Result<TestRun> {
    let Some(module) = build_module(root, manifest, None)? else {
        return Ok(TestRun {
            notes: vec![format!("no tests: the project has no `{TESTS_FEATURE}` Cargo feature and no {}", module_path(root, manifest).display())],
            ..TestRun::default()
        });
    };
    let Some(game) = game_executable(&TestSettings::default(), env::var_os("YOAKE_GAME").as_deref()) else {
        return Ok(TestRun { notes: vec!["YOAKE_GAME is unset; the tests were built but not run".to_owned()], ..TestRun::default() });
    };
    Ok(summarize(&run_game(root, &game, &[], &module, &TestSettings::default())?))
}

/// `yoake test`: build, check and run the tests in the game.
pub(crate) fn test_command(arguments: &[String]) -> Result<Value> {
    let settings = parse(arguments)?;
    let game = game_executable(&settings, env::var_os("YOAKE_GAME").as_deref())
        .ok_or("yoake test needs the game: pass --game <executable> or set YOAKE_GAME")?;
    let root = env::current_dir().map_err(|error| error.to_string())?;
    let manifest = read_manifest(&root)?;
    if manifest.wasm.is_some() {
        build(&root, &manifest)?;
    }
    let checked = check(&root, &manifest)?;
    let module = build_module(&root, &manifest, settings.module.as_deref())?.ok_or_else(|| {
        format!("the project has no tests: add a `{TESTS_FEATURE}` Cargo feature that calls export_tests!, or pass --module")
    })?;
    let report = run_game(&root, &game, &[], &module, &settings)?;
    let run = summarize(&report);
    Ok(json!({
        "ok": run.errors.is_empty(), "built": true, "checked": true, "errors": run.errors, "notes": run.notes,
        "tests": run.tests, "id": manifest.id, "version": manifest.version.to_string(),
        "game": game.display().to_string(), "wasm_needs": checked["wasm_needs"],
    }))
}
