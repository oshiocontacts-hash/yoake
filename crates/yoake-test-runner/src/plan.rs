//! The command line of the test runner.
use std::path::PathBuf;
use std::time::Duration;

/// The flag that turns a game process into the coordinator.
pub const FLAG: &str = "--yoake-test";
/// Default time one test may run, waiting for its world included.
pub const DEFAULT_TIMEOUT_MS: u64 = 30_000;
/// Default time a worker may take from its start to the first moment the test can run (game start, world
/// generation, the owner joining).
pub const DEFAULT_STARTUP_SECONDS: u64 = 240;
/// Longest value of `--yoake-test-timeout-ms` (one hour).
const MAX_TIMEOUT_MS: u64 = 3_600_000;

/// What the coordinator was asked to do.
#[derive(Clone, Debug, PartialEq)]
pub struct Plan {
    /// A project folder (with `yak.toml`) or a `.yak` file.
    pub target: PathBuf,
    /// The test module (a `test-mod` component). Default for a project folder: `bin/wasm/<id>-tests.wasm`;
    /// a `.yak` carries no tests, so it needs this.
    pub module: Option<PathBuf>,
    /// More packages to install next to the one under test (mods it calls with `mod-call`).
    pub with: Vec<PathBuf>,
    /// Where to write the JSON report (it is printed on stdout too).
    pub json: Option<PathBuf>,
    /// Clients that take part: 1 (a plain run) or 2 (not implemented, see the design notes).
    pub clients: u32,
    /// Only the tests whose name contains this.
    pub filter: Option<String>,
    pub timeout: Duration,
    pub startup: Duration,
    /// Keep the temporary folders (game folder, world, logs) instead of deleting them.
    pub keep: bool,
    /// Every other argument, handed to each worker as it is (`--resources PATH`, `--headless`, ...).
    pub game_args: Vec<String>,
}

/// Arguments the coordinator decides itself: a worker's world, folders and role are not the caller's to choose.
const OWNED: &[&str] = &[
    "--world", "--seed", "--game-dir", "--gameDir", "--home", "--player-dir", "--server", "--quickPlaySingleplayer",
    "--quickPlayMultiplayer", "--dedicated", "--benchmark", "--session-file", "--session-stdin", "--control",
    "--control-socket", "--dev-mod", "--help", "-h", "--launch-api",
];

fn number(flag: &str, value: &str, maximum: u64) -> Result<u64, String> {
    let parsed: u64 = value.parse().map_err(|_| format!("{flag} takes a whole number, not {value:?}"))?;
    if parsed == 0 || parsed > maximum {
        return Err(format!("{flag} takes 1 to {maximum}"));
    }
    Ok(parsed)
}

/// Reads the coordinator's plan from the process arguments (without the program name). `Ok(None)` when
/// `--yoake-test` is not among them: the process is a normal game or a worker.
pub fn parse(args: &[String]) -> Result<Option<Plan>, String> {
    if !args.iter().any(|argument| argument == FLAG) {
        return Ok(None);
    }
    let mut plan = Plan {
        target: PathBuf::new(),
        module: None,
        with: Vec::new(),
        json: None,
        clients: 1,
        filter: None,
        timeout: Duration::from_millis(DEFAULT_TIMEOUT_MS),
        startup: Duration::from_secs(DEFAULT_STARTUP_SECONDS),
        keep: false,
        game_args: Vec::new(),
    };
    let mut target = None;
    let mut iterator = args.iter();
    while let Some(argument) = iterator.next() {
        let mut value = |flag: &str| iterator.next().cloned().ok_or_else(|| format!("missing value after {flag}"));
        match argument.as_str() {
            FLAG => target = Some(PathBuf::from(value(FLAG)?)),
            "--yoake-test-json" => plan.json = Some(PathBuf::from(value(argument)?)),
            "--yoake-test-module" => plan.module = Some(PathBuf::from(value(argument)?)),
            "--yoake-test-with" => plan.with.push(PathBuf::from(value(argument)?)),
            "--yoake-test-filter" => plan.filter = Some(value(argument)?),
            "--yoake-test-timeout-ms" => {
                plan.timeout = Duration::from_millis(number(argument, &value(argument)?, MAX_TIMEOUT_MS)?);
            }
            "--yoake-test-startup-seconds" => {
                plan.startup = Duration::from_secs(number(argument, &value(argument)?, MAX_TIMEOUT_MS / 1000)?);
            }
            "--yoake-test-clients" => plan.clients = clients(&value(argument)?)?,
            "--yoake-test-keep" => plan.keep = true,
            other if other.starts_with("--yoake-test-clients=") => {
                plan.clients = clients(&other["--yoake-test-clients=".len()..])?;
            }
            other if other.starts_with("--yoake-test") => return Err(format!("unknown test option {other}")),
            other => {
                if OWNED.contains(&other) {
                    return Err(format!("{other} cannot be used with {FLAG}: each test gets its own world and game folder"));
                }
                plan.game_args.push(other.to_owned());
            }
        }
    }
    plan.target = target
        .filter(|path| !path.as_os_str().is_empty())
        .ok_or_else(|| format!("{FLAG} needs a project folder or a .yak"))?;
    Ok(Some(plan))
}

fn clients(value: &str) -> Result<u32, String> {
    match value.parse::<u32>() {
        Ok(count @ 1..=2) => Ok(count),
        _ => Err(format!("--yoake-test-clients is 1 or 2, not {value:?}")),
    }
}

/// What a worker process is told (the flags the coordinator adds). Parsed by the game's own option parser
/// one flag at a time ([`WorkerArgs::set`]).
#[derive(Clone, Debug, PartialEq)]
pub struct WorkerArgs {
    pub test: Option<String>,
    pub module: Option<PathBuf>,
    pub result: Option<PathBuf>,
    pub timeout: Duration,
}

impl Default for WorkerArgs {
    fn default() -> Self {
        Self { test: None, module: None, result: None, timeout: Duration::from_millis(DEFAULT_TIMEOUT_MS) }
    }
}

impl WorkerArgs {
    pub const FLAGS: [&'static str; 4] =
        ["--yoake-test-run", "--yoake-test-module", "--yoake-test-result", "--yoake-test-timeout-ms"];

    /// Whether `flag` is one of the worker's.
    pub fn is_flag(flag: &str) -> bool {
        Self::FLAGS.contains(&flag)
    }

    /// Takes `flag`'s value. `Ok(false)` when `flag` is not a worker flag.
    pub fn set(&mut self, flag: &str, value: String) -> Result<bool, String> {
        match flag {
            "--yoake-test-run" => self.test = Some(value),
            "--yoake-test-module" => self.module = Some(value.into()),
            "--yoake-test-result" => self.result = Some(value.into()),
            "--yoake-test-timeout-ms" => self.timeout = Duration::from_millis(number(flag, &value, MAX_TIMEOUT_MS)?),
            _ => return Ok(false),
        }
        Ok(true)
    }

    /// Whether this process is a worker.
    pub fn is_active(&self) -> bool {
        self.test.is_some()
    }

    /// A worker needs its module and its result path.
    pub fn validate(&self) -> Result<(), String> {
        if !self.is_active() {
            return Ok(());
        }
        if self.module.is_none() || self.result.is_none() {
            return Err("--yoake-test-run needs --yoake-test-module and --yoake-test-result".into());
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn args(list: &[&str]) -> Vec<String> {
        list.iter().map(|argument| (*argument).to_owned()).collect()
    }

    #[test]
    fn without_the_flag_the_process_is_not_a_coordinator() {
        assert_eq!(parse(&args(&["--offscreen", "--world", "x"])), Ok(None));
        // A worker's own flags do not make it a coordinator.
        assert_eq!(parse(&args(&["--yoake-test-run", "a", "--yoake-test-module", "m.wasm"])), Ok(None));
        assert_eq!(parse(&[]), Ok(None));
    }

    #[test]
    fn a_plan_has_defaults_and_keeps_the_games_own_arguments_in_order() {
        let plan = parse(&args(&["--resources", "res", "--yoake-test", "proj", "--headless"])).unwrap().unwrap();
        assert_eq!(plan.target, PathBuf::from("proj"));
        assert_eq!(plan.game_args, ["--resources", "res", "--headless"]);
        assert_eq!(plan.clients, 1);
        assert_eq!(plan.timeout, Duration::from_millis(DEFAULT_TIMEOUT_MS));
        assert_eq!(plan.startup, Duration::from_secs(DEFAULT_STARTUP_SECONDS));
        assert!(plan.json.is_none() && plan.module.is_none() && plan.filter.is_none() && !plan.keep);
    }

    #[test]
    fn every_option_is_read() {
        let plan = parse(&args(&[
            "--yoake-test", "a.yak", "--yoake-test-json", "out.json", "--yoake-test-module", "t.wasm",
            "--yoake-test-with", "peer", "--yoake-test-with", "peer2.yak", "--yoake-test-filter", "place",
            "--yoake-test-timeout-ms", "1500", "--yoake-test-startup-seconds", "10", "--yoake-test-clients=2",
            "--yoake-test-keep",
        ]))
        .unwrap()
        .unwrap();
        assert_eq!(plan.json, Some("out.json".into()));
        assert_eq!(plan.module, Some("t.wasm".into()));
        assert_eq!(plan.with, [PathBuf::from("peer"), PathBuf::from("peer2.yak")]);
        assert_eq!(plan.filter.as_deref(), Some("place"));
        assert_eq!(
            (plan.timeout, plan.startup, plan.clients, plan.keep),
            (Duration::from_millis(1500), Duration::from_secs(10), 2, true)
        );
        assert_eq!(parse(&args(&["--yoake-test", "p", "--yoake-test-clients", "1"])).unwrap().unwrap().clients, 1);
    }

    #[test]
    fn bad_command_lines_are_refused_with_the_reason() {
        for (bad, why) in [
            (&["--yoake-test"][..], "missing value"),
            (&["--yoake-test", "p", "--yoake-test-clients=3"][..], "1 or 2"),
            (&["--yoake-test", "p", "--yoake-test-clients=0"][..], "1 or 2"),
            (&["--yoake-test", "p", "--yoake-test-timeout-ms", "0"][..], "takes 1 to"),
            (&["--yoake-test", "p", "--yoake-test-timeout-ms", "soon"][..], "whole number"),
            (&["--yoake-test", "p", "--yoake-test-bogus"][..], "unknown test option"),
            (&["--yoake-test", "p", "--world", "mine"][..], "own world"),
            (&["--yoake-test", "p", "--dedicated"][..], "own world"),
            (&["--yoake-test", ""][..], "needs a project"),
        ] {
            let error = parse(&args(bad)).unwrap_err();
            assert!(error.contains(why), "{bad:?}: {error}");
        }
    }

    #[test]
    fn worker_flags_are_taken_one_at_a_time() {
        let mut worker = WorkerArgs::default();
        assert!(!worker.is_active());
        assert_eq!(worker.set("--offscreen", "x".into()), Ok(false));
        assert_eq!(worker.set("--yoake-test-run", "places".into()), Ok(true));
        assert!(worker.is_active());
        assert!(worker.validate().is_err(), "a worker needs a module and a result path");
        assert_eq!(worker.set("--yoake-test-module", "t.wasm".into()), Ok(true));
        assert_eq!(worker.set("--yoake-test-result", "r.json".into()), Ok(true));
        assert_eq!(worker.set("--yoake-test-timeout-ms", "2000".into()), Ok(true));
        assert_eq!(worker.validate(), Ok(()));
        assert_eq!(worker.timeout, Duration::from_millis(2000));
        assert!(worker.set("--yoake-test-timeout-ms", "0".into()).is_err());
        assert!(WorkerArgs::is_flag("--yoake-test-result") && !WorkerArgs::is_flag("--yoake-test"));
        // A process that is not a worker needs nothing.
        assert_eq!(WorkerArgs::default().validate(), Ok(()));
    }
}
