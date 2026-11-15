//! The coordinator: lists the tests of a project, runs each in a worker process with its own temporary
//! game folder and world, and builds the report.
use std::fs::{self, File};
use std::path::{Path, PathBuf};
use std::process::{Child, Command, Stdio};
use std::time::{Duration, Instant};

use serde_json::{Value, json};
use yoake_wasm::{Sandbox, TestHost, TestMod};

use crate::install::{self, Installed};
use crate::plan::Plan;
use crate::report::{self, TestRecord};

/// The folder name of the test world inside the worker's `saves/`.
pub const WORLD: &str = "yoake-test";
/// Time a worker gets after its test's own limit to write its result and exit, before it is killed.
pub const GRACE: Duration = Duration::from_secs(10);
/// The tail of a silent worker's console output that goes into the failure.
const OUTPUT_TAIL: usize = 1500;

/// The game process to start for a worker.
#[derive(Clone, Debug)]
pub struct GameCommand {
    pub exe: PathBuf,
    /// Arguments that come before everything else (tests use them to start a stand-in for the game).
    pub lead_args: Vec<String>,
    pub grace: Duration,
}

impl GameCommand {
    pub fn new(exe: impl Into<PathBuf>) -> Self {
        Self { exe: exe.into(), lead_args: Vec::new(), grace: GRACE }
    }
}

/// How a run ended: the report and the process exit code (0 passed, 1 a test failed, 2 not run).
pub struct Outcome {
    pub report: Value,
    pub code: i32,
}

/// A test host that offers nothing: enough to list the tests (`tests` calls no driver function).
struct Listing;
impl TestHost for Listing {}

/// Lists the test names of the module at `path`.
pub fn list_tests(path: &Path, limit: Duration) -> Result<Vec<String>, String> {
    let bytes = fs::read(path).map_err(|error| format!("{}: {error}", path.display()))?;
    let mut module = TestMod::instantiate(&mut Sandbox::new(), "yoake-test-list", &bytes, Box::new(Listing), limit)
        .map_err(|error| format!("{}: {error}", path.display()))?;
    module.tests()
}

fn failed(errors: Vec<String>, extra: Value) -> Outcome {
    Outcome { report: report::report(&[], &errors, extra), code: 2 }
}

/// Runs the plan. `prepare_world` makes the world folder it is given (`<game>/saves/yoake-test`) a fresh
/// superflat world with the fixed seed 0; the game supplies it (it writes the world's generator settings).
pub fn run(plan: &Plan, game: &GameCommand, prepare_world: &dyn Fn(&Path) -> Result<(), String>) -> Outcome {
    if plan.clients != 1 {
        return failed(
            vec![format!(
                "--yoake-test-clients={} is not implemented yet: the two-client mode is designed in docs/yoake-wasm-dev.md but only one client runs",
                plan.clients
            )],
            json!({"clients": plan.clients}),
        );
    }
    let project = match install::describe(&plan.target) {
        Ok(project) => project,
        Err(error) => return failed(vec![error], json!({})),
    };
    let extra = json!({"id": project.id, "version": project.version, "clients": 1});
    let module = match plan.module.clone() {
        Some(module) => module,
        None if plan.target.is_dir() => install::default_module(&plan.target, &project.id),
        None => return failed(vec!["a .yak carries no tests: pass the test module with --yoake-test-module".into()], extra),
    };
    if !module.is_file() {
        return failed(
            vec![format!("the test module {} does not exist: run yoake build (or give --yoake-test-module)", module.display())],
            extra,
        );
    }
    let mut names = match list_tests(&module, plan.timeout) {
        Ok(names) => names,
        Err(error) => return failed(vec![format!("could not list the tests: {error}")], extra),
    };
    let declared = names.len();
    if let Some(filter) = &plan.filter {
        names.retain(|name| name.contains(filter.as_str()));
    }
    let mut extra = extra;
    extra["module"] = json!(module.display().to_string());
    extra["declared"] = json!(declared);
    if names.is_empty() {
        extra["note"] = json!(if declared == 0 { "the test module declares no tests" } else { "no test matches the filter" });
        return Outcome { report: report::report(&[], &[], extra), code: 0 };
    }

    let root = std::env::temp_dir().join(format!(
        "yoake-test-{}-{}",
        std::process::id(),
        std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map_or(0, |elapsed| elapsed.as_nanos())
    ));
    let mut records = Vec::new();
    for (index, name) in names.iter().enumerate() {
        let record = run_one(plan, game, &module, &root.join(format!("t{index}")), name, prepare_world);
        eprintln!("yoake test: {} {} ({} ms)", if record.ok { "ok  " } else { "FAIL" }, record.name, record.ms);
        records.push(record);
    }
    if plan.keep {
        extra["kept"] = json!(root.display().to_string());
    } else {
        remove_tree(&root);
    }
    let code = report::exit_code(&records, &[]);
    Outcome { report: report::report(&records, &[], extra), code }
}

/// Runs one test in a worker and says how it went. Never panics: every failure is the test's record.
pub fn run_one(
    plan: &Plan,
    game: &GameCommand,
    module: &Path,
    dir: &Path,
    name: &str,
    prepare_world: &dyn Fn(&Path) -> Result<(), String>,
) -> TestRecord {
    let began = Instant::now();
    let fail = |error: String| TestRecord::fail(name, began.elapsed().as_millis() as u64, error);
    let game_dir = dir.join("game");
    let home = dir.join("home");
    let result = dir.join("result.json");
    let output = dir.join("worker.out");
    let prepared = (|| -> Result<(), String> {
        for folder in [&game_dir, &home] {
            fs::create_dir_all(folder).map_err(|error| format!("{}: {error}", folder.display()))?;
        }
        let mut installed: Vec<Installed> = Vec::new();
        for source in std::iter::once(&plan.target).chain(plan.with.iter()) {
            installed.push(install::install(source, &game_dir)?);
        }
        install::write_list(&game_dir, &installed)?;
        let world = game_dir.join("saves").join(WORLD);
        fs::create_dir_all(&world).map_err(|error| format!("{}: {error}", world.display()))?;
        prepare_world(&world)
    })();
    if let Err(error) = prepared {
        remove_unless_kept(plan, dir);
        return fail(format!("could not prepare the test world: {error}"));
    }
    let record = match spawn(plan, game, module, name, &game_dir, &home, &result, &output) {
        Ok(child) => finish(plan, game, child, name, &result, &output, began),
        Err(error) => fail(error),
    };
    remove_unless_kept(plan, dir);
    record
}

fn remove_unless_kept(plan: &Plan, dir: &Path) {
    if !plan.keep {
        remove_tree(dir);
    }
}

#[allow(clippy::too_many_arguments)]
fn spawn(
    plan: &Plan,
    game: &GameCommand,
    module: &Path,
    name: &str,
    game_dir: &Path,
    home: &Path,
    result: &Path,
    output: &Path,
) -> Result<Child, String> {
    let stdout = File::create(output).map_err(|error| format!("{}: {error}", output.display()))?;
    let stderr = stdout.try_clone().map_err(|error| error.to_string())?;
    let mut command = Command::new(&game.exe);
    command
        .args(&game.lead_args)
        // The window stays hidden and never takes the keyboard focus or the pointer.
        .args(["--offscreen", "--no-focus", "--no-pause"])
        .arg("--game-dir")
        .arg(game_dir)
        .arg("--home")
        .arg(home)
        .args(["--world", WORLD, "--seed", "0", "--creative", "--time", "6000", "--weather", "clear", "--render-distance", "4"])
        .args(["--yoake-test-run", name])
        .arg("--yoake-test-module")
        .arg(module)
        .arg("--yoake-test-result")
        .arg(result)
        .args(["--yoake-test-timeout-ms", &plan.timeout.as_millis().to_string()])
        .args(&plan.game_args)
        // Developer mode loads the unsigned package under test; it applies to this child only.
        .env("YOAKE_DLL_MODS_UNSIGNED", "1")
        .env("HORIZON_NO_AUDIO", "1")
        .env("HORIZON_NO_FOCUS", "1")
        .env("HORIZON_HOME", home)
        .stdin(Stdio::null())
        .stdout(stdout)
        .stderr(stderr);
    command.spawn().map_err(|error| format!("could not start {}: {error}", game.exe.display()))
}

fn finish(plan: &Plan, game: &GameCommand, mut child: Child, name: &str, result: &Path, output: &Path, began: Instant) -> TestRecord {
    let deadline = began + plan.startup + plan.timeout + game.grace;
    let ending = loop {
        match child.try_wait() {
            Ok(Some(status)) => break Ending::Exited(status.code()),
            Ok(None) if Instant::now() >= deadline => {
                let _ = child.kill();
                let _ = child.wait();
                break Ending::Killed;
            }
            Ok(None) => std::thread::sleep(Duration::from_millis(25)),
            Err(error) => break Ending::Lost(error.to_string()),
        }
    };
    let ms = began.elapsed().as_millis() as u64;
    if let Ok(text) = fs::read_to_string(result) {
        return match serde_json::from_str::<Value>(&text).map_err(|error| error.to_string()).and_then(|value| TestRecord::from_json(&value)) {
            Ok(mut record) => {
                // The worker names its test; the coordinator's name is the one reported.
                record.name = name.to_owned();
                record
            }
            Err(error) => TestRecord::fail(name, ms, format!("the worker's result is unreadable: {error}")),
        };
    }
    let tail = output_tail(output);
    let why = match ending {
        Ending::Killed => format!(
            "the worker did not finish within {} s and was killed",
            (plan.startup + plan.timeout + game.grace).as_secs()
        ),
        Ending::Exited(Some(0)) => "the worker exited without writing a result".to_owned(),
        Ending::Exited(Some(code)) => format!("the worker exited with code {code} before it ran the test"),
        Ending::Exited(None) => "the worker was ended by a signal before it ran the test".to_owned(),
        Ending::Lost(error) => format!("lost the worker: {error}"),
    };
    TestRecord::fail(name, ms, if tail.is_empty() { why } else { format!("{why}; its output ended: {tail}") })
}

enum Ending {
    Exited(Option<i32>),
    Killed,
    Lost(String),
}

fn output_tail(path: &Path) -> String {
    let Ok(bytes) = fs::read(path) else { return String::new() };
    let text = String::from_utf8_lossy(&bytes);
    let text: String = text.chars().map(|character| if character.is_control() && character != '\n' { ' ' } else { character }).collect();
    let text = text.trim();
    let skip = text.chars().count().saturating_sub(OUTPUT_TAIL);
    text.chars().skip(skip).collect::<String>().trim().replace('\n', " | ")
}

/// Removes `path` and everything under it; Windows may need a moment after the worker exits.
pub fn remove_tree(path: &Path) {
    for attempt in 0..10 {
        match fs::remove_dir_all(path) {
            Ok(()) => return,
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => return,
            Err(_) if attempt < 9 => std::thread::sleep(Duration::from_millis(100)),
            Err(error) => eprintln!("yoake test: could not delete {}: {error}", path.display()),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::install::tests::{project, scratch};
    use crate::report::exit_code;

    const FAKE: &str = "coordinator::tests::fake_game";

    /// The stand-in for the game: this very test binary, started with `--exact <FAKE> --nocapture --` so
    /// every game argument after it is a (harmless) test filter. As a child it reads its arguments like
    /// the game's worker would and acts by the test name: `pass`, `fail`, `crash`, `silent`, `hang`,
    /// `garbage`. Run normally (no `--yoake-test-run`) it does nothing.
    #[test]
    fn fake_game() {
        let args: Vec<String> = std::env::args().collect();
        let value = |flag: &str| args.iter().position(|argument| argument == flag).and_then(|index| args.get(index + 1)).cloned();
        let Some(test) = value("--yoake-test-run") else { return };
        let result = PathBuf::from(value("--yoake-test-result").expect("a result path"));
        // What a real worker would find: its folders, the package, the world and the flags that keep it hidden.
        let game_dir = PathBuf::from(value("--game-dir").expect("a game dir"));
        let mut problems = Vec::new();
        for flag in ["--offscreen", "--no-focus", "--no-pause"] {
            if !args.iter().any(|argument| argument == flag) {
                problems.push(format!("missing {flag}"));
            }
        }
        if fs::read_to_string(game_dir.join("yoake-mods.txt")).unwrap_or_default().trim() != "demo 1.2.3" {
            problems.push("yoake-mods.txt".into());
        }
        if !game_dir.join("yoake-mods/demo/1.2.3/yak.toml").is_file() {
            problems.push("the package".into());
        }
        if !game_dir.join("saves").join(WORLD).join("prepared").is_file() {
            problems.push("the world".into());
        }
        if value("--world").as_deref() != Some(WORLD) || value("--seed").as_deref() != Some("0") {
            problems.push("world or seed".into());
        }
        if !problems.is_empty() {
            let _ = crate::worker::write_result(&result, &TestRecord::fail(&test, 1, format!("bad worker setup: {problems:?}")));
            std::process::exit(1);
        }
        match test.as_str() {
            "pass" => {
                let _ = crate::worker::write_result(&result, &TestRecord::pass(&test, 7));
                std::process::exit(0);
            }
            "fail" => {
                let _ = crate::worker::write_result(&result, &TestRecord::fail(&test, 8, "expected stone, got dirt"));
                std::process::exit(1);
            }
            "crash" => {
                println!("starting");
                eprintln!("thread 'main' panicked: no GPU");
                std::process::exit(101);
            }
            "silent" => std::process::exit(0),
            "garbage" => {
                let _ = fs::write(&result, "{not json");
                std::process::exit(0);
            }
            "hang" => std::thread::sleep(Duration::from_secs(120)),
            other => panic!("unknown fake test {other}"),
        }
    }

    fn game(grace: Duration) -> GameCommand {
        GameCommand {
            exe: std::env::current_exe().unwrap(),
            lead_args: vec!["--exact".into(), FAKE.into(), "--nocapture".into(), "--".into()],
            grace,
        }
    }

    fn plan(target: &Path) -> Plan {
        Plan {
            target: target.to_owned(),
            module: None,
            with: Vec::new(),
            json: None,
            clients: 1,
            filter: None,
            timeout: Duration::from_secs(20),
            startup: Duration::from_secs(40),
            keep: false,
            game_args: vec!["--resources".into(), "res".into()],
        }
    }

    fn prepare(world: &Path) -> Result<(), String> {
        fs::write(world.join("prepared"), "flat seed 0").map_err(|error| error.to_string())
    }

    fn one(plan: &Plan, game: &GameCommand, name: &str) -> TestRecord {
        let dir = scratch("one");
        let record = run_one(plan, game, Path::new("tests.wasm"), &dir.join("t0"), name, &prepare);
        let _ = fs::remove_dir_all(dir);
        record
    }

    #[test]
    fn a_worker_that_reports_gives_its_record_and_gets_the_world_the_runner_promises() {
        let project_dir = scratch("coordinator");
        project(&project_dir, "demo", false);
        let plan = plan(&project_dir);
        let game = game(GRACE);
        let pass = one(&plan, &game, "pass");
        assert!(pass.ok, "{pass:?}");
        assert_eq!(pass.ms, 7, "the worker's own timing is reported");
        let fail = one(&plan, &game, "fail");
        assert_eq!((fail.ok, fail.error.as_deref()), (false, Some("expected stone, got dirt")));
        let _ = fs::remove_dir_all(project_dir);
    }

    #[test]
    fn a_worker_that_dies_without_a_result_fails_with_its_output() {
        let project_dir = scratch("dies");
        project(&project_dir, "demo", false);
        let plan = plan(&project_dir);
        let crash = one(&plan, &game(GRACE), "crash");
        let error = crash.error.unwrap();
        assert!(!crash.ok && error.contains("exited with code 101") && error.contains("no GPU"), "{error}");
        let silent = one(&plan, &game(GRACE), "silent");
        assert!(silent.error.unwrap().contains("without writing a result"));
        let garbage = one(&plan, &game(GRACE), "garbage");
        assert!(garbage.error.unwrap().contains("unreadable"));
        let _ = fs::remove_dir_all(project_dir);
    }

    #[test]
    fn a_worker_that_hangs_is_killed_at_the_deadline() {
        let project_dir = scratch("hang");
        project(&project_dir, "demo", false);
        let mut plan = plan(&project_dir);
        plan.startup = Duration::from_millis(700);
        plan.timeout = Duration::from_millis(300);
        let began = Instant::now();
        let record = one(&plan, &game(Duration::from_millis(500)), "hang");
        assert!(!record.ok && record.error.as_deref().unwrap().contains("was killed"), "{record:?}");
        assert!(began.elapsed() < Duration::from_secs(30), "{:?}", began.elapsed());
    }

    #[test]
    fn a_world_that_cannot_be_prepared_or_a_missing_game_is_the_tests_failure() {
        let project_dir = scratch("setup");
        project(&project_dir, "demo", false);
        let plan = plan(&project_dir);
        let dir = scratch("setup-run");
        let record = run_one(&plan, &game(GRACE), Path::new("t.wasm"), &dir.join("a"), "pass", &|_| Err("no flat preset".into()));
        assert!(record.error.unwrap().contains("could not prepare the test world: no flat preset"));
        let missing = GameCommand::new(dir.join("no-such-game.exe"));
        let record = run_one(&plan, &missing, Path::new("t.wasm"), &dir.join("b"), "pass", &prepare);
        assert!(record.error.unwrap().contains("could not start"));
        // The temporary folders are gone after a run, unless they were kept.
        assert!(!dir.join("a").exists() && !dir.join("b").exists());
        let mut kept = plan.clone();
        kept.keep = true;
        let record = run_one(&kept, &game(GRACE), Path::new("t.wasm"), &dir.join("c"), "pass", &prepare);
        assert!(record.ok, "{record:?}");
        assert!(dir.join("c/game/yoake-mods.txt").is_file());
        let _ = (fs::remove_dir_all(project_dir), fs::remove_dir_all(dir));
    }

    #[test]
    fn a_run_that_cannot_start_reports_why_and_exits_2() {
        let project_dir = scratch("cannot");
        project(&project_dir, "demo", false);
        let mut two = plan(&project_dir);
        two.clients = 2;
        let outcome = run(&two, &game(GRACE), &prepare);
        assert_eq!(outcome.code, 2);
        assert!(outcome.report["errors"][0].as_str().unwrap().contains("not implemented"), "{}", outcome.report);
        assert_eq!(outcome.report["ok"], json!(false));
        // A module that is not a component.
        let outcome = run(&plan(&project_dir), &game(GRACE), &prepare);
        assert!(outcome.report["errors"][0].as_str().unwrap().contains("could not list the tests"), "{}", outcome.report);
        // The module is not built.
        fs::remove_file(project_dir.join("bin/wasm/demo-tests.wasm")).unwrap();
        let outcome = run(&plan(&project_dir), &game(GRACE), &prepare);
        assert!(outcome.report["errors"][0].as_str().unwrap().contains("does not exist"), "{}", outcome.report);
        // A .yak has no tests of its own.
        let yak = project_dir.join("demo.yak");
        horizon_mods::yak::pack_directory(&project_dir, &yak, &Default::default()).unwrap();
        let outcome = run(&plan(&yak), &game(GRACE), &prepare);
        assert!(outcome.report["errors"][0].as_str().unwrap().contains("carries no tests"), "{}", outcome.report);
        // Not a project at all.
        let outcome = run(&plan(&project_dir.join("nothing")), &game(GRACE), &prepare);
        assert_eq!(outcome.code, 2);
        let _ = fs::remove_dir_all(project_dir);
    }

    #[test]
    fn a_whole_run_lists_filters_and_runs_every_test_of_the_module() {
        let Some(bytes) = yoake_wasm::test_guests::load(&yoake_wasm::test_guests::TEST_DEMO) else { return };
        let project_dir = scratch("whole");
        project(&project_dir, "demo", false);
        fs::write(project_dir.join("bin/wasm/demo-tests.wasm"), bytes).unwrap();
        // The module under test declares `passes`, `fails`, `driver_roundtrip`, `traps`, `spins`; the stand-in
        // game only knows `pass` and `fail`, so filter to the names the demo shares a prefix with.
        let mut plan = plan(&project_dir);
        plan.filter = Some("pass".into());
        let listed = list_tests(&project_dir.join("bin/wasm/demo-tests.wasm"), Duration::from_secs(20)).unwrap();
        assert_eq!(listed, ["passes", "fails", "driver_roundtrip", "traps", "spins"]);
        // `passes` is not a name the stand-in knows: it panics, which is a failed worker, not a failed run.
        let outcome = run(&plan, &game(GRACE), &prepare);
        assert_eq!(outcome.code, 1, "{}", outcome.report);
        assert_eq!(outcome.report["declared"], json!(5));
        assert_eq!(outcome.report["tests"].as_array().unwrap().len(), 1);
        assert_eq!(outcome.report["tests"][0]["name"], json!("passes"));
        assert_eq!(outcome.report["tests"][0]["ok"], json!(false));
        assert_eq!(exit_code(&[], &[]), 0);
        // A filter that matches nothing is not an error.
        plan.filter = Some("zzz".into());
        let outcome = run(&plan, &game(GRACE), &prepare);
        assert_eq!((outcome.code, outcome.report["ok"].clone()), (0, json!(true)));
        assert!(outcome.report["note"].as_str().unwrap().contains("filter"));
        let _ = fs::remove_dir_all(project_dir);
    }
}
