//! The coordinator: lists the tests of a project, runs each in a worker process with its own temporary
//! game folder and world, and builds the report.
use std::fs;
use std::path::{Path, PathBuf};
use std::process::{Child, Command, Stdio};
use std::time::{Duration, Instant};

use serde_json::{Value, json};
use yoake_wasm::{Sandbox, TestHost, TestMod};

use crate::guard::{self, CLIENT_FOLDER, CONTROL_FILE, Job, PORT_FILE, RESULT_FILE, TOKEN_ENV, TOKEN_FILE, Tail, random_hex};
use crate::install::{self, Installed};
use crate::plan::Plan;
use crate::report::{self, TestRecord};

pub use crate::guard::WORLD;

/// Time a worker gets after its test's own limit to write its result and exit, before it is killed.
pub const GRACE: Duration = Duration::from_secs(10);
/// Time a worker that has written its result may take to exit before it is killed.
pub const LINGER: Duration = Duration::from_secs(5);
/// The tail of a silent worker's console output that goes into the failure.
const OUTPUT_TAIL: usize = 1500;
/// The largest result file the coordinator reads.
const MAX_RESULT_BYTES: u64 = 1024 * 1024;

/// The game process to start for a worker.
#[derive(Clone, Debug)]
pub struct GameCommand {
    pub exe: PathBuf,
    /// Arguments that come before everything else (tests use them to start a stand-in for the game).
    pub lead_args: Vec<String>,
    pub grace: Duration,
    pub linger: Duration,
    /// Asked while a worker runs: true when the user asked the coordinator to stop (Ctrl+C).
    pub interrupted: fn() -> bool,
}

impl GameCommand {
    pub fn new(exe: impl Into<PathBuf>) -> Self {
        Self { exe: exe.into(), lead_args: Vec::new(), grace: GRACE, linger: LINGER, interrupted: || false }
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
    let bytes = crate::worker::read_module(path)?;
    let mut module = TestMod::instantiate(&mut Sandbox::new(), "yoake-test-list", &bytes, Box::new(Listing), limit)
        .map_err(|error| format!("{}: {error}", path.display()))?;
    module.tests()
}

fn failed(errors: Vec<String>, extra: Value) -> Outcome {
    Outcome { report: report::report(&[], &errors, extra), code: 2 }
}

fn absolute(path: &Path) -> PathBuf {
    std::path::absolute(path).unwrap_or_else(|_| path.to_owned())
}

/// The run's temporary folder: created new under an unpredictable name, deleted when this is dropped (also
/// when the run ends early) unless it is to be kept.
pub struct TempRoot {
    path: PathBuf,
    keep: bool,
}

impl TempRoot {
    pub fn create(keep: bool) -> Result<Self, String> {
        for _ in 0..16 {
            let path = absolute(&std::env::temp_dir().join(format!("yoake-test-{}", random_hex(8))));
            match fs::create_dir(&path) {
                Ok(()) => {
                    guard::make_private(&path).map_err(|error| format!("{}: {error}", path.display()))?;
                    return Ok(Self { path, keep });
                }
                Err(error) if error.kind() == std::io::ErrorKind::AlreadyExists => continue,
                Err(error) => return Err(format!("{}: {error}", path.display())),
            }
        }
        Err("could not create a temporary folder".into())
    }

    pub fn path(&self) -> &Path {
        &self.path
    }
}

impl Drop for TempRoot {
    fn drop(&mut self) {
        if !self.keep {
            remove_tree(&self.path);
        }
    }
}

/// Runs the plan. `prepare_world` makes the world folder it is given (`<game>/saves/yoake-test`) a fresh
/// superflat world with the fixed seed 0; the game supplies it (it writes the world's generator settings).
pub fn run(plan: &Plan, game: &GameCommand, prepare_world: &dyn Fn(&Path) -> Result<(), String>) -> Outcome {
    if !(1..=2).contains(&plan.clients) {
        return failed(vec![format!("--yoake-test-clients is 1 or 2, not {}", plan.clients)], json!({"clients": plan.clients}));
    }
    // The game changes its working directory: every path the workers get is absolute.
    let mut plan = plan.clone();
    plan.target = absolute(&plan.target);
    plan.with = plan.with.iter().map(|path| absolute(path)).collect();
    plan.module = plan.module.as_deref().map(absolute);
    let plan = &plan;
    let project = match install::describe(&plan.target) {
        Ok(project) => project,
        Err(error) => return failed(vec![error], json!({})),
    };
    let extra = json!({"id": project.id, "version": project.version, "clients": plan.clients});
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

    let root = match TempRoot::create(plan.keep) {
        Ok(root) => root,
        Err(error) => return failed(vec![format!("no temporary folder: {error}")], extra),
    };
    let job = Job::new();
    let mut records = Vec::new();
    let mut errors = Vec::new();
    for (index, name) in names.iter().enumerate() {
        if (game.interrupted)() {
            errors.push("interrupted".to_owned());
            break;
        }
        let record = run_one(plan, game, &module, &root.path().join(format!("t{index}")), name, prepare_world, job.as_ref());
        eprintln!("yoake test: {} {} ({} ms)", if record.ok { "ok  " } else { "FAIL" }, record.name, record.ms);
        records.push(record);
    }
    if plan.keep {
        extra["kept"] = json!(root.path().display().to_string());
    }
    let code = report::exit_code(&records, &errors);
    Outcome { report: report::report(&records, &errors, extra), code }
}

/// A child process that is killed when it is dropped.
struct Worker(Child);

impl Drop for Worker {
    fn drop(&mut self) {
        let _ = self.0.kill();
        let _ = self.0.wait();
    }
}

/// Runs one test in a worker and says how it went. Never panics: every failure is the test's record.
pub fn run_one(
    plan: &Plan,
    game: &GameCommand,
    module: &Path,
    dir: &Path,
    name: &str,
    prepare_world: &dyn Fn(&Path) -> Result<(), String>,
    job: Option<&Job>,
) -> TestRecord {
    let began = Instant::now();
    let fail = |error: String| TestRecord::fail(name, began.elapsed().as_millis() as u64, error);
    let dir = absolute(dir);
    let token = random_hex(16);
    let game_dir = dir.join("game");
    let home = dir.join("home");
    let result = dir.join(RESULT_FILE);
    let prepared = (|| -> Result<(), String> {
        for folder in [&game_dir, &home] {
            fs::create_dir_all(folder).map_err(|error| format!("{}: {error}", folder.display()))?;
        }
        guard::write_private(&dir.join(TOKEN_FILE), &token).map_err(|error| format!("{}: {error}", dir.display()))?;
        // Every game of the test has the packages installed: the worker's, and the second client's.
        let mut folders = vec![game_dir.clone()];
        if plan.clients == 2 {
            let second = dir.join(CLIENT_FOLDER);
            for folder in [second.join("game"), second.join("home")] {
                fs::create_dir_all(&folder).map_err(|error| format!("{}: {error}", folder.display()))?;
            }
            guard::write_private(&second.join(TOKEN_FILE), &token).map_err(|error| format!("{}: {error}", second.display()))?;
            folders.push(second.join("game"));
        }
        for folder in folders {
            let mut installed: Vec<Installed> = Vec::new();
            for source in std::iter::once(&plan.target).chain(plan.with.iter()) {
                installed.push(install::install(source, &folder)?);
            }
            install::write_list(&folder, &installed)?;
        }
        let world = game_dir.join("saves").join(WORLD);
        fs::create_dir_all(&world).map_err(|error| format!("{}: {error}", world.display()))?;
        prepare_world(&world)
    })();
    if let Err(error) = prepared {
        remove_unless_kept(plan, &dir);
        return fail(format!("could not prepare the test world: {error}"));
    }
    let record = match spawn(plan, game, &absolute(module), name, &game_dir, &home, &result, &token) {
        Ok(mut child) => {
            if let Some(job) = job {
                job.add(&child);
            }
            let tail = Tail::capture(&mut child);
            let mut second = (plan.clients == 2).then(|| Second::new(&dir, &token));
            let mut record = finish(plan, game, Worker(child), name, &result, &tail, began, second.as_mut(), job);
            if !record.ok {
                // What the game's own log said about mods in each process: a mod that did not load is the usual reason.
                let mut said = guard::log_lines_with(&home.join("logs").join("latest.log"), "wasm", 6);
                said.extend(guard::log_lines_with(&dir.join(CLIENT_FOLDER).join("home").join("logs").join("latest.log"), "wasm", 3));
                if !said.is_empty() {
                    record.error = Some(report::clip(&format!("{}; the game log's mod lines: {}", record.error.unwrap_or_default(), said.join(" | "))));
                }
            }
            record
        }
        Err(error) => fail(error),
    };
    remove_unless_kept(plan, &dir);
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
    token: &str,
) -> Result<Child, String> {
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
        .args(["--yoake-test-startup-seconds", &plan.startup.as_secs().max(1).to_string()]);
    if plan.clients == 2 {
        command
            .args(["--yoake-test-clients", "2"])
            .arg("--yoake-test-client-control")
            .arg(result.parent().unwrap_or(Path::new("")).join(CLIENT_FOLDER).join(CONTROL_FILE));
    }
    command.args(&plan.game_args);
    // A worker inherits only what the system and the graphics driver need: no `HORIZON_*` or `YOAKE_*` switch
    // of the coordinator's (session file, command script, DLL mods, developer mode, ...) reaches it.
    command.env_clear();
    for (variable, value) in guard::worker_env(std::env::vars_os()) {
        command.env(variable, value);
    }
    command
        .env(TOKEN_ENV, token)
        .env("HORIZON_NO_AUDIO", "1")
        .env("HORIZON_NO_FOCUS", "1")
        .env("HORIZON_HOME", home)
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());
    command.spawn().map_err(|error| format!("could not start {}: {error}", game.exe.display()))
}

enum Ending {
    Exited(Option<i32>),
    Killed,
    Interrupted,
    /// The second client could not start or ended before the worker reported.
    ClientDied(String),
    /// The worker wrote its result and did not exit: killed after the linger.
    Lingered,
    Lost(String),
}

/// The second client of a two-client test: started once the worker has published its world and written the port.
struct Second {
    folder: PathBuf,
    token: String,
    started: Option<(Worker, Tail)>,
    failure: Option<String>,
}

impl Second {
    fn new(dir: &Path, token: &str) -> Self {
        Self { folder: dir.to_owned(), token: token.to_owned(), started: None, failure: None }
    }

    /// Starts the client when the port file is there and it has not started yet.
    fn poll(&mut self, plan: &Plan, game: &GameCommand, job: Option<&Job>) {
        if self.started.is_some() || self.failure.is_some() {
            return;
        }
        let Some(port) = guard::read_limited(&self.folder.join(PORT_FILE), 16).and_then(|text| text.trim().parse::<u16>().ok()) else { return };
        match spawn_client(plan, game, &self.folder, port, &self.token) {
            Ok(mut child) => {
                if let Some(job) = job {
                    job.add(&child);
                }
                let tail = Tail::capture(&mut child);
                self.started = Some((Worker(child), tail));
            }
            Err(error) => self.failure = Some(error),
        }
    }

    /// Why the client cannot join any more: it could not be started, or it ended.
    fn dead(&mut self) -> Option<String> {
        if let Some(error) = &self.failure {
            return Some(format!("it could not start: {error}"));
        }
        let (worker, _) = self.started.as_mut()?;
        match worker.0.try_wait() {
            Ok(Some(status)) => Some(format!("it exited with {:?}", status.code())),
            _ => None,
        }
    }

    /// What to add to a failure: the client's trouble, and the end of its output.
    fn note(&mut self) -> String {
        let mut note = String::new();
        if let Some(error) = &self.failure {
            note += &format!("; the second client could not start: {error}");
        }
        if let Some((worker, tail)) = &mut self.started {
            let state = match worker.0.try_wait() {
                Ok(Some(status)) => format!("exited with {:?}", status.code()),
                _ => "was still running".to_owned(),
            };
            if state.starts_with("exited") {
                // The reader thread may still be taking the last of its output.
                std::thread::sleep(Duration::from_millis(150));
            }
            let output = tail.text(OUTPUT_TAIL / 2);
            note += &format!("; the second client {state}{}", if output.is_empty() { String::new() } else { format!(", its output ended: {output}") });
        }
        note
    }
}

/// The part of the forwarded game arguments the second client gets: where the prepared data is and how it draws.
/// Nothing else of the caller's reaches a game that joins a server.
pub fn client_game_args(args: &[String]) -> Vec<String> {
    let mut kept = Vec::new();
    let mut iterator = args.iter();
    while let Some(argument) = iterator.next() {
        match argument.as_str() {
            "--headless" => kept.push(argument.clone()),
            "--resources" | "--width" | "--height" | "--render-distance" => {
                if let Some(value) = iterator.next() {
                    kept.push(argument.clone());
                    kept.push(value.clone());
                }
            }
            _ => {}
        }
    }
    kept
}

/// Starts the second client: a plain offscreen game that joins 127.0.0.1:port in its own folders.
fn spawn_client(plan: &Plan, game: &GameCommand, dir: &Path, port: u16, token: &str) -> Result<Child, String> {
    let folder = dir.join(CLIENT_FOLDER);
    let mut command = Command::new(&game.exe);
    command
        .args(&game.lead_args)
        .args(["--offscreen", "--no-focus", "--no-pause"])
        .arg("--game-dir")
        .arg(folder.join("game"))
        .arg("--home")
        .arg(folder.join("home"))
        .args(["--server", &format!("127.0.0.1:{port}"), "--name", "TestB", "--render-distance", "4"])
        .arg("--control-socket")
        .arg(folder.join(CONTROL_FILE))
        .args(["--yoake-test-client", "1"])
        .args(["--yoake-test-lifetime-seconds", &(plan.startup + plan.timeout).as_secs().max(1).to_string()])
        .args(client_game_args(&plan.game_args));
    command.env_clear();
    for (variable, value) in guard::worker_env(std::env::vars_os()) {
        command.env(variable, value);
    }
    command
        .env(TOKEN_ENV, token)
        .env("HORIZON_NO_AUDIO", "1")
        .env("HORIZON_NO_FOCUS", "1")
        .env("HORIZON_HOME", folder.join("home"))
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());
    command.spawn().map_err(|error| format!("could not start {}: {error}", game.exe.display()))
}

#[allow(clippy::too_many_arguments)]
fn finish(
    plan: &Plan,
    game: &GameCommand,
    mut worker: Worker,
    name: &str,
    result: &Path,
    tail: &Tail,
    began: Instant,
    mut second: Option<&mut Second>,
    job: Option<&Job>,
) -> TestRecord {
    let deadline = began + plan.startup + plan.timeout + game.grace;
    let mut result_seen: Option<Instant> = None;
    let ending = loop {
        if let Some(second) = second.as_deref_mut() {
            second.poll(plan, game, job);
            // A client that is gone cannot join: fail now, not when the worker's own wait runs out.
            if !result.exists() && let Some(why) = second.dead() {
                break Ending::ClientDied(why);
            }
        }
        if (game.interrupted)() {
            break Ending::Interrupted;
        }
        match worker.0.try_wait() {
            Ok(Some(status)) => break Ending::Exited(status.code()),
            Ok(None) => {}
            Err(error) => break Ending::Lost(error.to_string()),
        }
        let now = Instant::now();
        if result_seen.is_none() && result.exists() {
            result_seen = Some(now);
        }
        if result_seen.is_some_and(|seen| now.duration_since(seen) >= game.linger) {
            break Ending::Lingered;
        }
        if now >= deadline {
            break Ending::Killed;
        }
        std::thread::sleep(Duration::from_millis(25));
    };
    // Every ending but a plain exit ends the worker here (dropping it would too).
    if !matches!(ending, Ending::Exited(_)) {
        let _ = worker.0.kill();
    }
    let _ = worker.0.wait();
    let ms = began.elapsed().as_millis() as u64;
    let note = second.as_deref_mut().map(Second::note).unwrap_or_default();
    // The second client goes with the worker (dropping it kills it).
    drop(second.as_deref_mut().and_then(|second| second.started.take()));
    if result.exists() {
        return match guard::read_limited(result, MAX_RESULT_BYTES) {
            None => TestRecord::fail(name, ms, "the worker's result is unreadable or larger than 1 MiB"),
            Some(text) => match serde_json::from_str::<Value>(&text).map_err(|error| error.to_string()).and_then(|value| TestRecord::from_json(&value)) {
                Ok(mut record) => {
                    // The worker names its test; the coordinator's name is the one reported.
                    record.name = name.to_owned();
                    if !record.ok && !note.is_empty() {
                        record.error = Some(report::clip(&format!("{}{note}", record.error.unwrap_or_default())));
                    }
                    record
                }
                Err(error) => TestRecord::fail(name, ms, format!("the worker's result is unreadable: {error}")),
            },
        };
    }
    let output = tail.text(OUTPUT_TAIL);
    let why = match ending {
        Ending::Killed => format!("the worker did not finish within {} s and was killed", (plan.startup + plan.timeout + game.grace).as_secs()),
        Ending::Interrupted => "interrupted".to_owned(),
        Ending::ClientDied(why) => format!("the second client did not join: {why}"),
        Ending::Lingered => "the worker's result vanished".to_owned(),
        Ending::Exited(Some(0)) => "the worker exited without writing a result".to_owned(),
        Ending::Exited(Some(code)) => format!("the worker exited with code {code} before it ran the test"),
        Ending::Exited(None) => "the worker was ended by a signal before it ran the test".to_owned(),
        Ending::Lost(error) => format!("lost the worker: {error}"),
    };
    TestRecord::fail(name, ms, if output.is_empty() { format!("{why}{note}") } else { format!("{why}; its output ended: {output}{note}") })
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
    use std::sync::atomic::{AtomicBool, Ordering};

    const FAKE: &str = "coordinator::tests::fake_game";

    /// The stand-in for the game: this very test binary, started with `--exact <FAKE> --nocapture --` so
    /// every game argument after it is a (harmless) test filter. As a child it checks, as the game's own
    /// start does, that it is a verified worker (`guard::verify_worker`) with an environment stripped of the
    /// coordinator's switches, and then acts by the test name: `pass`, `fail`, `crash`, `silent`, `hang`,
    /// `garbage`, `stall`, `huge`. Run normally (no `--yoake-test-run`) it does nothing.
    #[test]
    fn fake_game() {
        let args: Vec<String> = std::env::args().collect();
        let value = |flag: &str| args.iter().position(|argument| argument == flag).and_then(|index| args.get(index + 1)).cloned();
        // The second client of a two-client test: it proves itself, leaves a marker in its folder and waits to be killed.
        if value("--yoake-test-client").is_some() {
            if std::env::var_os("YOAKE_NEVER_SET").is_none() && value("--name").as_deref() == Some("TestB") && value("--server").is_some_and(|server| server.ends_with(":4343")) {
                println!("client says goodbye");
                std::process::exit(7);
            }
            let proof = guard::verify_client(&args[1..], std::env::var(TOKEN_ENV).ok().as_deref());
            let folder = PathBuf::from(value("--game-dir").expect("a game dir")).parent().unwrap().to_owned();
            let server = value("--server").unwrap_or_default();
            let note = if matches!(proof, Ok(Some(_))) && server.starts_with("127.0.0.1:4242") { "joined".to_owned() } else { format!("bad client: {proof:?} {server}") };
            fs::write(folder.join("started"), note).unwrap();
            std::thread::sleep(Duration::from_secs(120));
            return;
        }
        let Some(test) = value("--yoake-test-run") else { return };
        let result = PathBuf::from(value("--yoake-test-result").expect("a result path"));
        let game_dir = PathBuf::from(value("--game-dir").expect("a game dir"));
        let mut problems = Vec::new();
        match guard::verify_worker(&args[1..], std::env::var(TOKEN_ENV).ok().as_deref()) {
            Ok(Some(worker)) => {
                if !worker.module.as_deref().is_some_and(Path::is_absolute) {
                    problems.push("module not absolute".to_owned());
                }
            }
            other => problems.push(format!("not a verified worker: {other:?}")),
        }
        if !value("--yoake-test-module").is_some_and(|module| Path::new(&module).is_absolute()) {
            problems.push("the module path was not made absolute".into());
        }
        for forbidden in ["YOAKE_DLL_MODS", "YOAKE_DLL_MODS_UNSIGNED", "YOAKE_WASM_DEV_DIR", "HORIZON_SESSION_FILE", "HORIZON_COMMAND_SCRIPT"] {
            if std::env::var_os(forbidden).is_some() {
                problems.push(format!("{forbidden} leaked"));
            }
        }
        if std::env::var_os("HORIZON_HOME").is_none() {
            problems.push("HORIZON_HOME".into());
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
            "huge" => {
                let _ = fs::write(&result, vec![b' '; 2 * 1024 * 1024]);
                std::process::exit(0);
            }
            "stall" => {
                let _ = crate::worker::write_result(&result, &TestRecord::pass(&test, 3));
                std::thread::sleep(Duration::from_secs(120));
            }
            "hang" => std::thread::sleep(Duration::from_secs(120)),
            "two-dies" => {
                // Publishes a port the stand-in client exits on, then waits for a client that will never join.
                let folder = result.parent().unwrap().to_owned();
                fs::write(folder.join(guard::PORT_FILE), "4343").unwrap();
                std::thread::sleep(Duration::from_secs(60));
            }
            "two" => {
                // The worker of a two-client test: publishes (a port file), waits for its client, then reports.
                let folder = result.parent().unwrap().to_owned();
                fs::write(folder.join(guard::PORT_FILE), "4242").unwrap();
                let marker = folder.join(CLIENT_FOLDER).join("started");
                let deadline = Instant::now() + Duration::from_secs(60);
                while !marker.exists() && Instant::now() < deadline {
                    std::thread::sleep(Duration::from_millis(50));
                }
                let said = fs::read_to_string(&marker).unwrap_or_else(|_| "the client never started".into());
                let record = if said == "joined" { TestRecord::pass(&test, 5) } else { TestRecord::fail(&test, 5, said) };
                let _ = crate::worker::write_result(&result, &record);
                std::process::exit(if record.ok { 0 } else { 1 });
            }
            other => panic!("unknown fake test {other}"),
        }
    }

    fn game(grace: Duration) -> GameCommand {
        GameCommand {
            exe: std::env::current_exe().unwrap(),
            lead_args: vec!["--exact".into(), FAKE.into(), "--nocapture".into(), "--".into()],
            grace,
            linger: Duration::from_millis(400),
            interrupted: || false,
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

    fn one_with(plan: &Plan, game: &GameCommand, name: &str) -> TestRecord {
        let dir = scratch("one");
        // A relative module path, as a user types it: the worker must still find it.
        let record = run_one(plan, game, Path::new("tests.wasm"), &dir.join("t0"), name, &prepare, None);
        let _ = fs::remove_dir_all(dir);
        record
    }

    #[test]
    fn a_worker_that_reports_gives_its_record_and_gets_the_world_the_runner_promises() {
        let project_dir = scratch("coordinator");
        project(&project_dir, "demo", false);
        let plan = plan(&project_dir);
        let game = game(GRACE);
        let pass = one_with(&plan, &game, "pass");
        assert!(pass.ok, "{pass:?}");
        assert_eq!(pass.ms, 7, "the worker's own timing is reported");
        let fail = one_with(&plan, &game, "fail");
        assert_eq!((fail.ok, fail.error.as_deref()), (false, Some("expected stone, got dirt")));
        let _ = fs::remove_dir_all(project_dir);
    }

    #[test]
    fn a_worker_that_dies_without_a_result_fails_with_its_output() {
        let project_dir = scratch("dies");
        project(&project_dir, "demo", false);
        let plan = plan(&project_dir);
        let crash = one_with(&plan, &game(GRACE), "crash");
        let error = crash.error.unwrap();
        assert!(!crash.ok && error.contains("exited with code 101") && error.contains("no GPU"), "{error}");
        let silent = one_with(&plan, &game(GRACE), "silent");
        assert!(silent.error.unwrap().contains("without writing a result"));
        let garbage = one_with(&plan, &game(GRACE), "garbage");
        assert!(garbage.error.unwrap().contains("unreadable"));
        let huge = one_with(&plan, &game(GRACE), "huge");
        assert!(huge.error.unwrap().contains("larger than 1 MiB"));
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
        let record = one_with(&plan, &game(Duration::from_millis(500)), "hang");
        assert!(!record.ok && record.error.as_deref().unwrap().contains("was killed"), "{record:?}");
        assert!(began.elapsed() < Duration::from_secs(30), "{:?}", began.elapsed());
        let _ = fs::remove_dir_all(project_dir);
    }

    #[test]
    fn a_worker_that_reported_but_does_not_exit_is_killed_after_the_linger() {
        let project_dir = scratch("stall");
        project(&project_dir, "demo", false);
        let plan = plan(&project_dir);
        let began = Instant::now();
        // The grace is a minute: only the linger (400 ms here) can end this quickly.
        let record = one_with(&plan, &game(Duration::from_secs(60)), "stall");
        assert!(record.ok, "{record:?}");
        assert!(began.elapsed() < Duration::from_secs(30), "{:?}", began.elapsed());
        let _ = fs::remove_dir_all(project_dir);
    }

    static INTERRUPT: AtomicBool = AtomicBool::new(false);

    #[test]
    fn an_interrupt_ends_the_worker_and_the_run() {
        let project_dir = scratch("interrupt");
        project(&project_dir, "demo", false);
        let plan = plan(&project_dir);
        let mut command = game(GRACE);
        command.interrupted = || INTERRUPT.load(Ordering::Relaxed);
        let flag = std::thread::spawn(|| {
            std::thread::sleep(Duration::from_millis(1500));
            INTERRUPT.store(true, Ordering::Relaxed);
        });
        let began = Instant::now();
        let record = one_with(&plan, &command, "hang");
        assert_eq!(record.error.as_deref().map(|error| error.starts_with("interrupted")), Some(true), "{record:?}");
        assert!(began.elapsed() < Duration::from_secs(30));
        flag.join().unwrap();
        INTERRUPT.store(false, Ordering::Relaxed);
        let _ = fs::remove_dir_all(project_dir);
    }

    #[test]
    fn a_world_that_cannot_be_prepared_or_a_missing_game_is_the_tests_failure() {
        let project_dir = scratch("setup");
        project(&project_dir, "demo", false);
        let plan = plan(&project_dir);
        let dir = scratch("setup-run");
        let record = run_one(&plan, &game(GRACE), Path::new("t.wasm"), &dir.join("a"), "pass", &|_| Err("no flat preset".into()), None);
        assert!(record.error.unwrap().contains("could not prepare the test world: no flat preset"));
        let missing = GameCommand::new(dir.join("no-such-game.exe"));
        let record = run_one(&plan, &missing, Path::new("t.wasm"), &dir.join("b"), "pass", &prepare, None);
        assert!(record.error.unwrap().contains("could not start"));
        // The temporary folders are gone after a run, unless they were kept.
        assert!(!dir.join("a").exists() && !dir.join("b").exists());
        let mut kept = plan.clone();
        kept.keep = true;
        let record = run_one(&kept, &game(GRACE), Path::new("t.wasm"), &dir.join("c"), "pass", &prepare, None);
        assert!(record.ok, "{record:?}");
        assert!(dir.join("c/game/yoake-mods.txt").is_file());
        assert!(dir.join("c").join(TOKEN_FILE).is_file());
        let _ = (fs::remove_dir_all(project_dir), fs::remove_dir_all(dir));
    }

    #[test]
    fn the_second_client_gets_only_the_resources_and_drawing_arguments() {
        let args: Vec<String> = ["--resources", "res", "--headless", "--render-distance", "6", "--no-pause", "--name", "Mallory", "--width", "800"]
            .iter()
            .map(|argument| (*argument).to_owned())
            .collect();
        assert_eq!(client_game_args(&args), ["--resources", "res", "--headless", "--render-distance", "6", "--width", "800"]);
        assert!(client_game_args(&["--resources".to_owned()]).is_empty(), "a flag without its value is dropped");
    }

    #[test]
    fn a_second_client_that_dies_fails_the_run_at_once_with_its_output() {
        let project_dir = scratch("clientdies");
        project(&project_dir, "demo", false);
        let mut plan = plan(&project_dir);
        plan.clients = 2;
        let began = Instant::now();
        // The worker waits 60 s for a client that exits at once (the stand-in client of "two-dies").
        let record = one_with(&plan, &game(GRACE), "two-dies");
        let error = record.error.unwrap_or_default();
        assert!(!record.ok && error.contains("the second client did not join: it exited with Some(7)"), "{error}");
        assert!(error.contains("client says goodbye"), "its output tail is in the failure: {error}");
        assert!(began.elapsed() < Duration::from_secs(30), "{:?}", began.elapsed());
        let _ = fs::remove_dir_all(project_dir);
    }

    #[test]
    fn a_two_client_run_starts_the_second_client_when_the_port_appears_and_kills_it_with_the_worker() {
        let project_dir = scratch("two");
        project(&project_dir, "demo", false);
        let mut plan = plan(&project_dir);
        plan.clients = 2;
        let began = Instant::now();
        let record = one_with(&plan, &game(GRACE), "two");
        assert!(record.ok, "{record:?}");
        assert!(began.elapsed() < Duration::from_secs(40), "{:?}", began.elapsed());
        let _ = fs::remove_dir_all(project_dir);
    }

    #[test]
    fn the_temporary_root_is_new_unpredictable_and_removed_with_its_guard() {
        let (one, two) = (TempRoot::create(false).unwrap(), TempRoot::create(false).unwrap());
        assert_ne!(one.path(), two.path());
        assert!(one.path().is_dir() && one.path().is_absolute());
        let name = one.path().file_name().unwrap().to_string_lossy().into_owned();
        assert!(name.starts_with("yoake-test-") && name.len() == "yoake-test-".len() + 16, "{name}");
        let path = one.path().to_owned();
        drop(one);
        assert!(!path.exists());
        let kept = TempRoot::create(true).unwrap();
        let path = kept.path().to_owned();
        drop(kept);
        assert!(path.is_dir());
        remove_tree(&path);
        drop(two);
    }

    #[test]
    fn a_run_that_cannot_start_reports_why_and_exits_2() {
        let project_dir = scratch("cannot");
        project(&project_dir, "demo", false);
        let mut three = plan(&project_dir);
        three.clients = 3;
        let outcome = run(&three, &game(GRACE), &prepare);
        assert_eq!(outcome.code, 2);
        assert!(outcome.report["errors"][0].as_str().unwrap().contains("1 or 2"), "{}", outcome.report);
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
        // game only knows `pass` and `fail`, so `passes` (an unknown name for it) fails its worker.
        let mut plan = plan(&project_dir);
        plan.filter = Some("pass".into());
        let listed = list_tests(&project_dir.join("bin/wasm/demo-tests.wasm"), Duration::from_secs(20)).unwrap();
        assert_eq!(listed, ["passes", "fails", "driver_roundtrip", "two_clients", "traps", "spins"]);
        let outcome = run(&plan, &game(GRACE), &prepare);
        assert_eq!(outcome.code, 1, "{}", outcome.report);
        assert_eq!(outcome.report["declared"], json!(6));
        assert_eq!(outcome.report["tests"].as_array().unwrap().len(), 1);
        assert_eq!(outcome.report["tests"][0]["name"], json!("passes"));
        assert_eq!(outcome.report["tests"][0]["ok"], json!(false));
        // A filter that matches nothing is not an error.
        plan.filter = Some("zzz".into());
        let outcome = run(&plan, &game(GRACE), &prepare);
        assert_eq!((outcome.code, outcome.report["ok"].clone()), (0, json!(true)));
        assert!(outcome.report["note"].as_str().unwrap().contains("filter"));
        let _ = fs::remove_dir_all(project_dir);
    }
}
