//! Crash reports, laid out like vanilla's CrashReport.getFriendlyReport: a comment, the time,
//! a description, the error with its stack, then the details of the game and the machine.
//! Written to `crash-reports/crash-<yyyy-MM-dd_HH.mm.ss>-client.txt` in the game folder when
//! the main thread panics or `main` gets an error back. It leaves the machine only scrubbed, and
//! only if the player allows crash reports (`telemetry`).
use std::cell::{Cell, RefCell};
use std::fmt::Write as _;
use std::path::{Path, PathBuf};
use std::sync::Mutex;
use std::sync::atomic::{AtomicBool, Ordering};

use chrono::{DateTime, Local};

/// What the game is doing, set as it goes: graphics adapter, settings, packs, world.
static DETAILS: Mutex<Vec<(&'static str, String)>> = Mutex::new(Vec::new());
static REPORTED: AtomicBool = AtomicBool::new(false);

thread_local! {
    /// How many `contain` calls this thread is inside.
    static CONTAINED: Cell<u32> = const { Cell::new(0) };
    /// The message of the last panic `contain` caught on this thread, from the hook.
    static CAUGHT: RefCell<Option<String>> = const { RefCell::new(None) };
    /// The native mod hook this thread is inside, for the panic hook to name.
    static IN_NATIVE_MOD: Cell<Option<NativeModCall>> = const { Cell::new(None) };
}

/// A call into one hook of a native mod.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct NativeModCall {
    /// The mod's `[mod] id`, None in a build that didn't name its mods.
    pub id: Option<&'static str>,
    /// Its place in dispatch order, which names it then.
    pub index: usize,
    pub hook: &'static str,
}

/// The innermost native-mod call currently active on this thread.
pub fn native_mod_call() -> Option<NativeModCall> {
    IN_NATIVE_MOD.with(Cell::get)
}

/// Marks this thread as inside `call` until the guard drops, which also happens while a panic
/// unwinds. A hook called from a hook is named by the inner call.
pub fn enter_native_mod(call: NativeModCall) -> NativeModGuard {
    NativeModGuard {
        outer: IN_NATIVE_MOD.with(|c| c.replace(Some(call))),
    }
}

/// Puts back the call that `enter_native_mod` found.
#[must_use]
pub struct NativeModGuard {
    outer: Option<NativeModCall>,
}

impl Drop for NativeModGuard {
    fn drop(&mut self) {
        IN_NATIVE_MOD.with(|c| c.set(self.outer));
    }
}

/// `message`, with the native mod hook this thread is inside, if any.
fn attributed(message: String) -> String {
    match IN_NATIVE_MOD.with(Cell::get) {
        Some(NativeModCall {
            id: Some(id), hook, ..
        }) => format!("{message}\nin native mod {id}, hook {hook}"),
        Some(NativeModCall {
            id: None,
            index,
            hook,
        }) => format!("{message}\nin native mod #{index}, hook {hook}"),
        None => message,
    }
}

/// Runs `f`, and returns a panic in it as an error with the panic's message instead of letting
/// it end the game. For work that is not the game's own and whose failure the caller recovers
/// from, such as handing a shader pack's programs to the GPU driver's compiler: the panic goes
/// to the log with its stack, and no crash report is made for it.
pub fn contain<T>(f: impl FnOnce() -> T) -> Result<T, String> {
    CONTAINED.with(|c| c.set(c.get() + 1));
    let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(f));
    CONTAINED.with(|c| c.set(c.get() - 1));
    result.map_err(|payload| {
        CAUGHT
            .with(|c| c.borrow_mut().take())
            .unwrap_or_else(|| payload_text(&*payload))
    })
}

/// For the hook: a panic inside `contain` is logged with its stack and kept for `contain` to
/// return. False for any other panic.
fn contained_panic(info: &std::panic::PanicHookInfo) -> bool {
    if CONTAINED.with(Cell::get) == 0 {
        return false;
    }
    let message = panic_message(info);
    let stack = stack_lines(
        &std::backtrace::Backtrace::force_capture().to_string(),
        true,
    );
    log::error!(
        "Recovered from an error: {message}\n{}",
        stack
            .lines()
            .map(|l| format!("\tat {l}"))
            .collect::<Vec<_>>()
            .join("\n")
    );
    CAUGHT.with(|c| *c.borrow_mut() = Some(message));
    true
}

/// Sets one line of the report's details (replacing the last value under `key`).
pub fn set_detail(key: &'static str, value: impl Into<String>) {
    let mut details = DETAILS.lock().unwrap_or_else(|e| e.into_inner());
    let value = value.into();
    match details.iter_mut().find(|(k, _)| *k == key) {
        Some(entry) => entry.1 = value,
        None => details.push((key, value)),
    }
}

pub fn clear_detail(key: &'static str) {
    DETAILS
        .lock()
        .unwrap_or_else(|e| e.into_inner())
        .retain(|(k, _)| *k != key);
}

fn details() -> Vec<(&'static str, String)> {
    DETAILS.lock().unwrap_or_else(|e| e.into_inner()).clone()
}

/// Vanilla picks a line from a list of jokes; these only say what happened.
const COMMENTS: &[&str] = &[
    "Something went wrong.",
    "The game stopped here.",
    "This should not have happened.",
    "Details below.",
];

/// One crash, ready to be written.
pub struct Report {
    pub time: DateTime<Local>,
    pub description: String,
    /// The error's message, first line of the stack section.
    pub error: String,
    /// Where it happened and how it got there: one frame a line (`function (file:line:col)`).
    pub stack: String,
    pub thread: String,
    /// `client` or `server`, the end of the file name.
    pub side: &'static str,
    pub details: Vec<(String, String)>,
    pub log: Vec<String>,
}

/// `crash-2026-10-14_09.05.07-client.txt`
pub fn file_name(time: &DateTime<Local>, side: &str) -> String {
    format!("crash-{}-{side}.txt", time.format("%Y-%m-%d_%H.%M.%S"))
}

impl Report {
    /// The report's text.
    pub fn text(&self) -> String {
        let comment = COMMENTS[self.time.timestamp().rem_euclid(COMMENTS.len() as i64) as usize];
        let mut out = String::new();
        let _ = writeln!(out, "---- Horizon Crash Report ----");
        let _ = writeln!(out, "// {comment}\n");
        let _ = writeln!(out, "Time: {}", self.time.format("%Y-%m-%d %H:%M:%S"));
        let _ = writeln!(out, "Description: {}\n", self.description);
        let _ = writeln!(out, "{}", self.error);
        for line in self.stack.lines() {
            let _ = writeln!(out, "\tat {line}");
        }
        let _ = writeln!(
            out,
            "\n\nA detailed walkthrough of the error, its code path and all known details is as follows:"
        );
        let _ = writeln!(out, "{}\n", "-".repeat(87));
        let _ = writeln!(out, "-- Head --");
        let _ = writeln!(out, "Thread: {}\n", self.thread);
        let _ = writeln!(out, "-- System Details --");
        let _ = writeln!(out, "Details:");
        for (key, value) in &self.details {
            let _ = writeln!(out, "\t{key}: {value}");
        }
        let _ = writeln!(out, "\n-- Last {} log lines --", self.log.len());
        for line in &self.log {
            let _ = writeln!(out, "{line}");
        }
        out
    }

    /// Writes the report in `home/crash-reports` and returns its path.
    pub fn save(&self, home: &Path) -> std::io::Result<PathBuf> {
        let dir = crate::logging::crash_reports_dir(home);
        std::fs::create_dir_all(&dir)?;
        let path = dir.join(file_name(&self.time, self.side));
        std::fs::write(&path, self.text())?;
        Ok(path)
    }
}

/// The release this binary is (`v0.2.3`); a nightly's label (`Nightly 2026-10-28 · 0c02067`); for a
/// release from before 0.2, its tag and the client's crate version; for a build from source, that
/// version alone.
pub fn version() -> String {
    if let Some(label) = horizon_core::release::nightly_label() {
        return label;
    }
    let version = env!("CARGO_PKG_VERSION");
    match (horizon_core::release::RELEASE, release_tag()) {
        (Some(_), Some(tag)) => tag,
        (None, Some(tag)) => format!("{tag} (client {version})"),
        (_, None) => version.to_owned(),
    }
}

/// The release this binary came with: the version release.yml built in (docs/versioning.md), else
/// the folder the launcher installed it in, named after the tag (releases from before 0.2). A
/// nightly is its version as the launcher installs it (`nightly-2026-10-28-0c02067`).
pub fn release_tag() -> Option<String> {
    if let Some(nightly) = horizon_core::release::nightly_version() {
        return Some(nightly);
    }
    if let Some(version) = horizon_core::release::RELEASE {
        return Some(format!("v{version}"));
    }
    let exe = std::env::current_exe().ok()?;
    let folder = exe.parent()?.file_name()?.to_str()?;
    (folder.starts_with('v') && folder[1..].starts_with(|c: char| c.is_ascii_digit()))
        .then(|| folder.to_owned())
}

fn command_output(program: &str, args: &[&str]) -> Option<String> {
    let out = std::process::Command::new(program)
        .args(args)
        .output()
        .ok()?;
    let text = String::from_utf8_lossy(&out.stdout).trim().to_owned();
    (out.status.success() && !text.is_empty()).then_some(text)
}

fn read(path: &str) -> Option<String> {
    std::fs::read_to_string(path).ok()
}

/// `PRETTY_NAME` of /etc/os-release and the kernel, or the system's own version string.
fn operating_system() -> String {
    let arch = std::env::consts::ARCH;
    let version = if cfg!(target_os = "linux") {
        let name = read("/etc/os-release").and_then(|t| {
            t.lines()
                .find_map(|l| l.strip_prefix("PRETTY_NAME="))
                .map(|v| v.trim_matches('"').to_owned())
        });
        let kernel = read("/proc/sys/kernel/osrelease").map(|k| k.trim().to_owned());
        match (name, kernel) {
            (Some(n), Some(k)) => Some(format!("{n}, kernel {k}")),
            (n, k) => n.or(k),
        }
    } else if cfg!(target_os = "macos") {
        command_output("sw_vers", &["-productVersion"]).map(|v| format!("macOS {v}"))
    } else if cfg!(windows) {
        horizon_telemetry::system::windows_version()
    } else {
        None
    };
    format!(
        "{} ({arch}) version {}",
        std::env::consts::OS,
        version.unwrap_or_else(|| "unknown".into())
    )
}

/// Logical processors and the CPU's name (`12x AMD Ryzen 5 5600X ...`).
pub fn cpu() -> String {
    let threads = std::thread::available_parallelism().map_or(0, |n| n.get());
    let name = if cfg!(target_os = "linux") {
        read("/proc/cpuinfo").and_then(|t| horizon_telemetry::system::linux_cpu_name(&t))
    } else if cfg!(target_os = "macos") {
        command_output("sysctl", &["-n", "machdep.cpu.brand_string"])
    } else if cfg!(windows) {
        horizon_telemetry::system::windows_cpu()
    } else {
        std::env::var("PROCESSOR_IDENTIFIER").ok()
    };
    format!(
        "{threads}x {}",
        name.unwrap_or_else(|| std::env::consts::ARCH.into())
    )
}

/// Total memory and what this process uses, in MiB where the system says.
fn memory() -> String {
    const MIB: u64 = 1024 * 1024;
    let kib = |text: &str, key: &str| -> Option<u64> {
        text.lines()
            .find(|l| l.starts_with(key))?
            .split_whitespace()
            .nth(1)?
            .parse()
            .ok()
    };
    let (total, used) = if cfg!(target_os = "linux") {
        (
            read("/proc/meminfo")
                .and_then(|t| kib(&t, "MemTotal:"))
                .map(|k| k * 1024),
            read("/proc/self/status")
                .and_then(|t| kib(&t, "VmRSS:"))
                .map(|k| k * 1024),
        )
    } else if cfg!(target_os = "macos") {
        (
            command_output("sysctl", &["-n", "hw.memsize"]).and_then(|v| v.parse().ok()),
            None,
        )
    } else if cfg!(windows) {
        (
            command_output(
                "powershell",
                &[
                    "-NoProfile",
                    "-Command",
                    "(Get-CimInstance Win32_ComputerSystem).TotalPhysicalMemory",
                ],
            )
            .and_then(|v| v.parse().ok()),
            None,
        )
    } else {
        (None, None)
    };
    match (total, used) {
        (Some(t), Some(u)) => format!(
            "{} MiB used by the game, {} MiB installed",
            u / MIB,
            t / MIB
        ),
        (Some(t), None) => format!("{} MiB installed", t / MIB),
        _ => "unknown".into(),
    }
}

/// The report's details: Horizon's and the machine's, then what the game set as it ran. A dedicated
/// server renders nothing: it leaves out the render settings it read from options.txt.
pub fn system_details(side: &str) -> Vec<(String, String)> {
    let mut out = vec![
        ("Horizon Version".to_owned(), version()),
        (
            "Minecraft Version".to_owned(),
            horizon_core::VANILLA_VERSION.to_owned(),
        ),
        ("Operating System".to_owned(), operating_system()),
        ("CPU".to_owned(), cpu()),
        ("Memory".to_owned(), memory()),
    ];
    out.extend(
        details()
            .into_iter()
            .filter(|(k, _)| side != "server" || *k != "Render Settings")
            .map(|(k, v)| (k.to_owned(), v)),
    );
    out
}

/// The thread's name as vanilla's reports say it: the main thread is the client's "Render thread"
/// and the dedicated server's "Server thread".
fn thread_name(side: &str) -> String {
    let current = std::thread::current();
    match current.name() {
        Some("main") | None if side == "server" => "Server thread".into(),
        Some("main") | None => "Render thread".into(),
        Some(name) => name.into(),
    }
}

/// What starts the stderr line naming a saved report, the path follows: the Launch API's
/// promise to launchers (docs/api/launch.md), so it never changes within a line of that API.
pub const SAVED_LINE: &str = "Horizon: crash report saved to ";

/// Writes the report and tells where it is; the log gets the error too.
fn deliver(home: &Path, report: Report) -> Option<PathBuf> {
    if REPORTED.swap(true, Ordering::SeqCst) {
        return None;
    }
    // The panic is on stderr already: the stack goes to the log only.
    crate::logging::write_only(
        log::Level::Error,
        &format!(
            "Horizon crashed: {}\n{}",
            report.error,
            report
                .stack
                .lines()
                .map(|l| format!("\tat {l}"))
                .collect::<Vec<_>>()
                .join("\n")
        ),
    );
    crate::logging::flush();
    crate::telemetry::keep_crash(home, &report);
    // A player of a shared game folder (`--player-dir`) finds the report beside its own log.
    match report.save(crate::logging::player_dir_override().unwrap_or(home)) {
        Ok(path) => {
            eprintln!("{SAVED_LINE}{}", path.display());
            Some(path)
        }
        Err(e) => {
            eprintln!("Horizon: could not save a crash report: {e}");
            eprintln!("{}", report.text());
            None
        }
    }
}

/// The report for a hung server tick, written by the watchdog thread
/// (`ServerWatchdog`): the stuck thread's stack is not available to Rust, so
/// the report carries none.
pub fn report_watchdog(home: &Path, side: &'static str, message: String) -> Option<PathBuf> {
    deliver(
        home,
        Report {
            time: Local::now(),
            description: "Watching Server".into(),
            error: format!("{message} (Watching Server)"),
            stack: String::new(),
            thread: thread_name(side),
            side,
            details: system_details(side),
            log: crate::logging::recent_lines(),
        },
    )
}

/// The report for a game that went over the player's RAM limit (`ram_limit`), written by the
/// watcher thread: it says nothing about where the memory went, so it carries no stack. The player
/// chose this limit, so it is no bug to send (`telemetry::keep_crash`), and leaving that out ends
/// the game seconds sooner: it keeps running, over its limit, until the report is written.
pub fn report_memory_limit(home: &Path, side: &'static str, message: String) {
    if REPORTED.swap(true, Ordering::SeqCst) {
        return;
    }
    let report = Report {
        time: Local::now(),
        description: "RAM limit reached".into(),
        error: message,
        stack: String::new(),
        thread: thread_name(side),
        side,
        details: system_details(side),
        log: crate::logging::recent_lines(),
    };
    match report.save(home) {
        Ok(path) => eprintln!("{SAVED_LINE}{}", path.display()),
        Err(e) => eprintln!("Horizon: could not save a crash report: {e}"),
    }
}

fn payload_text(payload: &(dyn std::any::Any + Send)) -> String {
    payload
        .downcast_ref::<&str>()
        .map(|s| s.to_string())
        .or_else(|| payload.downcast_ref::<String>().cloned())
        .unwrap_or_else(|| "Box<dyn Any>".into())
}

fn panic_message(info: &std::panic::PanicHookInfo) -> String {
    let text = payload_text(info.payload());
    attributed(match info.location() {
        Some(at) => format!("panicked at {at}:\n{text}"),
        None => format!("panicked:\n{text}"),
    })
}

/// A `std::backtrace` as one line a frame, `function (file:line:col)`. Captured in a panic hook
/// (`from_panic`), it starts at the frame that panicked: the hook's and the panic machinery's
/// frames go. Paths inside the workspace lose the build machine's prefix.
pub fn stack_lines(backtrace: &str, from_panic: bool) -> String {
    // The workspace as it was on the machine that built this binary (it need not exist here).
    // A report names workspace files the same way on every host, whatever
    // separator the build machine used.
    let manifest = env!("CARGO_MANIFEST_DIR").replace('\\', "/");
    let root = manifest
        .strip_suffix("crates/horizon-client-render")
        .unwrap_or(&manifest);
    let mut frames: Vec<(String, Option<String>)> = Vec::new();
    for line in backtrace.lines() {
        let line = line.trim();
        if let Some(at) = line.strip_prefix("at ") {
            if let Some(frame) = frames.last_mut() {
                frame.1 = Some(short_path(&at.replace('\\', "/"), root).to_owned());
            }
        } else if let Some((n, name)) = line.split_once(": ")
            && n.chars().all(|c| c.is_ascii_digit())
        {
            frames.push((name.to_owned(), None));
        }
    }
    let machinery = |name: &str| {
        [
            "std::panicking::",
            "core::panicking::",
            "std::sys::backtrace::",
            "__rustc::",
        ]
        .iter()
        .any(|p| name.starts_with(p))
            || name.contains("rust_begin_unwind")
    };
    // The hook's frames, then the panic machinery's, then the code that panicked.
    let top = frames.iter().take(30).position(|(name, _)| machinery(name));
    let first = match top.filter(|_| from_panic) {
        Some(start) => {
            start
                + frames[start..]
                    .iter()
                    .take_while(|(name, _)| machinery(name))
                    .count()
        }
        None => 0,
    };
    // Some platforms keep std's own capture frames (`<std::backtrace::Backtrace>::create`).
    let first = first
        + frames[first..]
            .iter()
            .take_while(|(name, _)| name.trim_start_matches('<').starts_with("std::backtrace"))
            .count();
    frames[first..]
        .iter()
        .map(|(name, at)| match at {
            Some(at) => format!("{name} ({at})\n"),
            None => format!("{name}\n"),
        })
        .collect()
}

/// A source path without the build machine's folders: relative to the workspace, or from the
/// crate's folder for dependencies (`winit-0.30.13/src/event_loop.rs:642:56`).
fn short_path<'a>(path: &'a str, root: &str) -> &'a str {
    if let Some(rest) = path.strip_prefix(root) {
        return rest;
    }
    if let Some(i) = path.find("/registry/src/") {
        let rest = &path[i + "/registry/src/".len()..];
        if let Some((_index, krate)) = rest.split_once('/') {
            return krate;
        }
    }
    path
}

/// A report for a panic, with the stack captured now (RUST_BACKTRACE or not).
pub fn panic_report(info: &std::panic::PanicHookInfo, side: &'static str) -> Report {
    Report {
        time: Local::now(),
        description: "Unexpected error".into(),
        error: panic_message(info),
        stack: stack_lines(
            &std::backtrace::Backtrace::force_capture().to_string(),
            true,
        ),
        thread: thread_name(side),
        side,
        details: system_details(side),
        log: crate::logging::recent_lines(),
    }
}

/// Whether `name` is one of the shared worker pool's threads (`horizon_threads::jobs`), vanilla's
/// `Util` worker threads.
fn is_worker_thread(name: Option<&str>) -> bool {
    name.is_some_and(|n| n.starts_with("horizon-worker-"))
}

/// `Util.onThreadException`: a panic on a worker thread is made a report titled "Exception on
/// worker thread", with the thread's name, as `BlockableEventLoop.relayDelayCrash` hands it to
/// the main thread.
pub fn worker_panic_report(info: &std::panic::PanicHookInfo, side: &'static str) -> Report {
    Report {
        description: "Exception on worker thread".into(),
        ..panic_report(info, side)
    }
}

/// Reports panics: on the main thread a crash report (the panic then ends the game, exit code
/// 101); on a worker thread (`Util.onThreadException`) a crash report too, and the game ends
/// with it; on other threads the error goes to the log, as vanilla's uncaught exception handler
/// does. The default hook still prints the panic. A panic inside `contain` is only logged. A
/// panic inside a native mod's hook names the mod and the hook.
pub fn install(home: PathBuf, side: &'static str) {
    let default = std::panic::take_hook();
    std::panic::set_hook(Box::new(move |info| {
        if mod_guard::capture_panic(info) || contained_panic(info) {
            return;
        }
        default(info);
        let main = std::thread::current().name() == Some("main");
        if main {
            deliver(&home, panic_report(info, side));
        } else if is_worker_thread(std::thread::current().name()) {
            deliver(&home, worker_panic_report(info, side));
            // The crash ends the game, as the relayed report does on vanilla's main thread.
            std::process::exit(101);
        } else {
            log::error!(
                "Caught previously unhandled exception in thread {}: {}",
                thread_name(side),
                panic_message(info)
            );
        }
    }));
}

/// Vanilla's debug crash (`key.debug.crash` held for ten seconds): the error `main` gets back,
/// with the stack where it was raised.
#[derive(Debug)]
pub struct DebugCrash {
    stack: String,
}

impl DebugCrash {
    pub const MESSAGE: &str = "Manually triggered debug crash";
    #[allow(clippy::new_without_default)]
    pub fn new() -> Self {
        Self {
            stack: stack_lines(
                &std::backtrace::Backtrace::force_capture().to_string(),
                false,
            ),
        }
    }
}

impl std::fmt::Display for DebugCrash {
    fn fmt(&self, f: &mut std::fmt::Formatter) -> std::fmt::Result {
        f.write_str(Self::MESSAGE)
    }
}

impl std::error::Error for DebugCrash {}

/// A report for an error `main` got back: the game ends with it.
pub fn report_error(
    home: &Path,
    side: &'static str,
    error: &(dyn std::error::Error + 'static),
) -> Option<PathBuf> {
    if let Some(debug) = error.downcast_ref::<DebugCrash>() {
        return deliver(
            home,
            Report {
                time: Local::now(),
                description: DebugCrash::MESSAGE.into(),
                error: DebugCrash::MESSAGE.into(),
                stack: debug.stack.clone(),
                thread: thread_name(side),
                side,
                details: system_details(side),
                log: crate::logging::recent_lines(),
            },
        );
    }
    let mut message = error.to_string();
    let mut source = error.source();
    while let Some(e) = source {
        let _ = write!(message, "\nCaused by: {e}");
        source = e.source();
    }
    deliver(
        home,
        Report {
            time: Local::now(),
            description: "Fatal error".into(),
            error: message,
            stack: String::new(),
            thread: thread_name(side),
            side,
            details: system_details(side),
            log: crate::logging::recent_lines(),
        },
    )
}

/// Called every frame: panics once `HORIZON_DEBUG_CRASH` asks (development builds only).
pub fn debug_crash_tick() {
    static AT: std::sync::OnceLock<Option<(std::time::Instant, std::time::Duration)>> =
        std::sync::OnceLock::new();
    if let Some((first, after)) =
        AT.get_or_init(|| debug_crash_after().map(|d| (std::time::Instant::now(), d)))
        && first.elapsed() >= *after
    {
        panic!("Manually triggered debug crash");
    }
}

/// Development builds crash on purpose when `HORIZON_DEBUG_CRASH` is set (vanilla has F3+C held
/// for ten seconds): the number is seconds after the first frame (default 5).
pub fn debug_crash_after() -> Option<std::time::Duration> {
    if !cfg!(debug_assertions) {
        return None;
    }
    let value = std::env::var("HORIZON_DEBUG_CRASH").ok()?;
    let seconds = value.parse::<f64>().unwrap_or(5.0).max(0.0);
    Some(std::time::Duration::from_secs_f64(seconds))
}

pub mod mod_guard {
    use std::cell::{Cell, RefCell};
    use std::panic::{AssertUnwindSafe, PanicHookInfo};
    use std::sync::atomic::{AtomicBool, AtomicU64, AtomicUsize, Ordering};
    use std::sync::{Mutex, OnceLock};
    use std::time::Instant;

    const OPEN: usize = 1 << (usize::BITS - 1);
    const USERS: usize = OPEN - 1;
    const LANES: usize = 3;
    const VALUE_MASK: u64 = u32::MAX as u64;

    #[derive(Clone, Copy, Debug, PartialEq, Eq)]
    pub enum PanicMode {
        Contain,
        Crash,
    }

    pub fn panic_mode(value: Option<&str>) -> PanicMode {
        if value == Some("crash") {
            PanicMode::Crash
        } else {
            PanicMode::Contain
        }
    }

    #[derive(Clone, Copy, Debug, PartialEq, Eq)]
    pub enum ModThread {
        Client,
        Server,
        Background,
    }

    impl ModThread {
        pub fn as_str(self) -> &'static str {
            match self {
                Self::Client => "client",
                Self::Server => "server",
                Self::Background => "background",
            }
        }

        fn index(self) -> usize {
            match self {
                Self::Client => 0,
                Self::Server => 1,
                Self::Background => 2,
            }
        }
    }

    thread_local! {
        static THREAD: Cell<ModThread> = const { Cell::new(ModThread::Client) };
        static CAPTURING: Cell<bool> = const { Cell::new(false) };
        static PANIC: RefCell<Option<(String, String)>> = const { RefCell::new(None) };
    }

    pub struct ThreadScope(ModThread);

    pub fn enter_thread(thread: ModThread) -> ThreadScope {
        ThreadScope(THREAD.with(|current| current.replace(thread)))
    }

    impl Drop for ThreadScope {
        fn drop(&mut self) {
            THREAD.with(|current| current.set(self.0));
        }
    }

    struct CaptureScope {
        active: bool,
        previous: Option<(String, String)>,
    }

    impl CaptureScope {
        fn new() -> Self {
            Self {
                active: CAPTURING.with(|active| active.replace(true)),
                previous: PANIC.with(|panic| panic.take()),
            }
        }
    }

    impl Drop for CaptureScope {
        fn drop(&mut self) {
            CAPTURING.with(|active| active.set(self.active));
            PANIC.with(|panic| *panic.borrow_mut() = self.previous.take());
        }
    }

    pub(super) fn capture_panic(info: &PanicHookInfo<'_>) -> bool {
        if !CAPTURING.with(Cell::get) {
            return false;
        }
        let mut message = super::payload_text(info.payload());
        if let Some(location) = info.location() {
            use std::fmt::Write;
            let _ = write!(message, " at {}:{}", location.file(), location.line());
        }
        let stack = std::backtrace::Backtrace::force_capture().to_string();
        PANIC.with(|panic| *panic.borrow_mut() = Some((message, stack)));
        true
    }

    #[derive(Clone, Debug)]
    #[non_exhaustive]
    pub struct ModError {
        pub hook: &'static str,
        pub message: String,
    }

    #[derive(Clone, Debug)]
    #[non_exhaustive]
    pub struct ModStatus {
        pub id: String,
        pub index: usize,
        pub enabled: bool,
        pub first_error: Option<ModError>,
    }

    #[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
    #[non_exhaustive]
    pub struct CostWindow {
        pub calls: u64,
        pub total_ns: u64,
        pub max_ns: u64,
    }

    #[derive(Clone, Debug)]
    #[non_exhaustive]
    pub struct ModCost {
        pub id: String,
        pub index: usize,
        pub hook: &'static str,
        pub thread: ModThread,
        pub last_second: CostWindow,
        pub since_enabled: CostWindow,
    }

    struct ModState {
        id: Option<&'static str>,
        label: String,
        enabled: AtomicBool,
        first_error: OnceLock<ModError>,
        local_notice: AtomicBool,
        server_notice: AtomicBool,
    }

    pub struct Registry {
        mods: Box<[ModState]>,
        hooks: &'static [&'static str],
        mode: PanicMode,
        profiling: AtomicBool,
        gate: AtomicUsize,
        profile: OnceLock<ProfileSession>,
        control: Mutex<()>,
    }

    static REGISTRY: OnceLock<Registry> = OnceLock::new();

    impl Registry {
        pub fn new(
            ids: &[&'static str],
            count: usize,
            hooks: &'static [&'static str],
            mode: PanicMode,
        ) -> Self {
            Self {
                mods: (0..count)
                    .map(|index| ModState {
                        id: ids.get(index).copied(),
                        label: ids
                            .get(index)
                            .map_or_else(|| format!("#{index}"), |id| (*id).into()),
                        enabled: AtomicBool::new(true),
                        first_error: OnceLock::new(),
                        local_notice: AtomicBool::new(false),
                        server_notice: AtomicBool::new(false),
                    })
                    .collect(),
                hooks,
                mode,
                profiling: AtomicBool::new(false),
                gate: AtomicUsize::new(0),
                profile: OnceLock::new(),
                control: Mutex::new(()),
            }
        }

        pub fn dispatch(&self, count: usize, hook: &'static str) -> Option<Dispatch<'_>> {
            if count == 0 {
                return None;
            }
            let lease = if self.profiling.load(Ordering::Relaxed) {
                self.hooks
                    .iter()
                    .position(|candidate| *candidate == hook)
                    .and_then(|hook_index| self.lease(hook_index))
            } else {
                None
            };
            Some(Dispatch {
                registry: Some(self),
                hook,
                lease,
            })
        }

        fn lease(&self, hook_index: usize) -> Option<ProfileLease<'_>> {
            let mut gate = self.gate.load(Ordering::Acquire);
            loop {
                if gate & OPEN == 0 || gate & USERS == USERS {
                    return None;
                }
                match self.gate.compare_exchange_weak(
                    gate,
                    gate + 1,
                    Ordering::Acquire,
                    Ordering::Relaxed,
                ) {
                    Ok(_) => {
                        return Some(ProfileLease {
                            registry: self,
                            session: self.profile.get().expect("open profiler"),
                            hook_index,
                            thread: THREAD.with(Cell::get),
                        });
                    }
                    Err(current) => gate = current,
                }
            }
        }

        pub fn set_profile_enabled(&self, enabled: bool) -> Result<(), &'static str> {
            let _control = self
                .control
                .lock()
                .unwrap_or_else(|error| error.into_inner());
            if enabled == self.profiling.load(Ordering::Relaxed) {
                return Ok(());
            }
            if !enabled {
                self.profiling.store(false, Ordering::Relaxed);
                self.gate.fetch_and(USERS, Ordering::AcqRel);
                if let Some(session) = self.profile.get() {
                    session.stopped_ns.store(session.now(), Ordering::Relaxed);
                }
                return Ok(());
            }
            if self.gate.load(Ordering::Acquire) != 0 {
                return Err("Previous profiled calls are finishing; retry mod-profile on");
            }
            let session = self
                .profile
                .get_or_init(|| ProfileSession::new(self.mods.len() * self.hooks.len() * LANES));
            session.reset();
            self.gate.store(OPEN, Ordering::Release);
            self.profiling.store(true, Ordering::Relaxed);
            Ok(())
        }

        pub fn profile_snapshot(&self) -> Vec<ModCost> {
            let _control = self
                .control
                .lock()
                .unwrap_or_else(|error| error.into_inner());
            let Some(session) = self.profile.get() else {
                return Vec::new();
            };
            let now = if self.profiling.load(Ordering::Relaxed) {
                session.now()
            } else {
                session.stopped_ns.load(Ordering::Relaxed)
            };
            let second = session.second(now);
            let mut costs = Vec::new();
            for (index, state) in self.mods.iter().enumerate() {
                for (hook_index, hook) in self.hooks.iter().enumerate() {
                    for thread in [ModThread::Client, ModThread::Server, ModThread::Background] {
                        let cell = &session.cells
                            [(index * self.hooks.len() + hook_index) * LANES + thread.index()];
                        let since_enabled = cell.total();
                        if since_enabled.calls != 0 {
                            costs.push(ModCost {
                                id: state.label.clone(),
                                index,
                                hook,
                                thread,
                                last_second: second
                                    .checked_sub(1)
                                    .map_or_else(CostWindow::default, |second| cell.window(second)),
                                since_enabled,
                            });
                        }
                    }
                }
            }
            costs.sort_by(|left, right| {
                right
                    .since_enabled
                    .total_ns
                    .cmp(&left.since_enabled.total_ns)
                    .then_with(|| left.index.cmp(&right.index))
                    .then_with(|| left.hook.cmp(right.hook))
                    .then_with(|| left.thread.index().cmp(&right.thread.index()))
            });
            costs
        }

        pub fn statuses(&self) -> Vec<ModStatus> {
            self.mods
                .iter()
                .enumerate()
                .map(|(index, state)| ModStatus {
                    id: state.label.clone(),
                    index,
                    enabled: state.enabled.load(Ordering::Relaxed),
                    first_error: state.first_error.get().cloned(),
                })
                .collect()
        }

        pub fn take_notices(&self, server: bool) -> Vec<ModNotice> {
            self.mods
                .iter()
                .enumerate()
                .filter_map(|(index, state)| {
                    let pending = if server {
                        &state.server_notice
                    } else {
                        &state.local_notice
                    };
                    if !pending.swap(false, Ordering::AcqRel) {
                        return None;
                    }
                    state.first_error.get().map(|error| ModNotice {
                        id: state.label.clone(),
                        index,
                        hook: error.hook,
                    })
                })
                .collect()
        }

        pub fn maintenance(&self) {
            if !self.profiling.load(Ordering::Relaxed) {
                return;
            }
            let Some(session) = self.profile.get() else {
                return;
            };
            let second = session.second(session.now());
            let previous = session.summary_second.load(Ordering::Relaxed);
            if second.saturating_sub(previous) < 10
                || session
                    .summary_second
                    .compare_exchange(previous, second, Ordering::Relaxed, Ordering::Relaxed)
                    .is_err()
            {
                return;
            }
            let mut costs = self.profile_snapshot();
            costs.sort_by_key(|cost| std::cmp::Reverse(cost.last_second.total_ns));
            let top = costs
                .iter()
                .take(5)
                .map(|cost| {
                    format!(
                        "{}/{} [{}] {:.3} ms/s",
                        cost.id,
                        cost.hook,
                        cost.thread.as_str(),
                        cost.last_second.total_ns as f64 / 1_000_000.0
                    )
                })
                .collect::<Vec<_>>()
                .join(", ");
            log::info!("Native mod profile: {top}");
        }
    }

    #[derive(Clone, Debug)]
    #[non_exhaustive]
    pub struct ModNotice {
        pub id: String,
        pub index: usize,
        pub hook: &'static str,
    }

    #[derive(Debug, PartialEq, Eq)]
    pub enum CallOutcome<T> {
        Skipped,
        Completed(T),
        Panicked,
    }

    pub struct Dispatch<'a> {
        registry: Option<&'a Registry>,
        hook: &'static str,
        lease: Option<ProfileLease<'a>>,
    }

    impl Dispatch<'_> {
        pub fn call<T>(&self, index: usize, call: impl FnOnce() -> T) -> CallOutcome<T> {
            let Some(registry) = self.registry else {
                return CallOutcome::Completed(call());
            };
            let Some(state) = registry.mods.get(index) else {
                return CallOutcome::Skipped;
            };
            if !state.enabled.load(Ordering::Relaxed) {
                return CallOutcome::Skipped;
            }
            let _mark = super::enter_native_mod(super::NativeModCall {
                id: state.id,
                index,
                hook: self.hook,
            });
            let started = self.lease.as_ref().map(|lease| lease.session.now());
            let result = match registry.mode {
                PanicMode::Crash => Ok(call()),
                PanicMode::Contain => {
                    let _capture = CaptureScope::new();
                    std::panic::catch_unwind(AssertUnwindSafe(call)).map_err(|payload| {
                        PANIC.with(|panic| panic.take()).unwrap_or_else(|| {
                            (
                                super::payload_text(&*payload),
                                std::backtrace::Backtrace::force_capture().to_string(),
                            )
                        })
                    })
                }
            };
            if let (Some(lease), Some(started)) = (&self.lease, started) {
                let ended = lease.session.now();
                let offset = (index * registry.hooks.len() + lease.hook_index) * LANES
                    + lease.thread.index();
                lease.session.cells[offset]
                    .record(lease.session.second(ended), ended.saturating_sub(started));
            }
            match result {
                Ok(value) => CallOutcome::Completed(value),
                Err((message, stack)) => {
                    if state.enabled.swap(false, Ordering::Relaxed) {
                        let _ = state.first_error.set(ModError {
                            hook: self.hook,
                            message: message.clone(),
                        });
                        state.local_notice.store(true, Ordering::Release);
                        state.server_notice.store(true, Ordering::Release);
                        log::error!(
                            "Native mod {} stopped: hook={}, message={message:?}, stack={stack:?}",
                            state.label,
                            self.hook
                        );
                    }
                    CallOutcome::Panicked
                }
            }
        }
    }

    struct ProfileLease<'a> {
        registry: &'a Registry,
        session: &'a ProfileSession,
        hook_index: usize,
        thread: ModThread,
    }

    impl Drop for ProfileLease<'_> {
        fn drop(&mut self) {
            self.registry.gate.fetch_sub(1, Ordering::Release);
        }
    }

    #[derive(Default)]
    struct SecondSlot {
        calls: AtomicU64,
        total_us: AtomicU64,
        max_us: AtomicU64,
    }

    fn update_stamped(value: &AtomicU64, second: u64, amount: u64, maximum: bool) {
        let stamp = second.saturating_add(1).min(VALUE_MASK);
        let mut previous = value.load(Ordering::Relaxed);
        loop {
            let old_stamp = previous >> 32;
            if old_stamp > stamp {
                return;
            }
            let old_value = if old_stamp == stamp {
                previous & VALUE_MASK
            } else {
                0
            };
            let next = if maximum {
                old_value.max(amount)
            } else {
                old_value.saturating_add(amount)
            }
            .min(VALUE_MASK);
            match value.compare_exchange_weak(
                previous,
                (stamp << 32) | next,
                Ordering::Relaxed,
                Ordering::Relaxed,
            ) {
                Ok(_) => return,
                Err(current) => previous = current,
            }
        }
    }

    fn read_stamped(value: &AtomicU64, second: u64) -> u64 {
        let value = value.load(Ordering::Relaxed);
        if value >> 32 == second.saturating_add(1).min(VALUE_MASK) {
            value & VALUE_MASK
        } else {
            0
        }
    }

    #[derive(Default)]
    struct CostCell {
        calls: AtomicU64,
        total_ns: AtomicU64,
        max_ns: AtomicU64,
        seconds: [SecondSlot; 3],
    }

    impl CostCell {
        fn record(&self, second: u64, elapsed: u64) {
            self.calls.fetch_add(1, Ordering::Relaxed);
            self.total_ns.fetch_add(elapsed, Ordering::Relaxed);
            self.max_ns.fetch_max(elapsed, Ordering::Relaxed);
            let slot = &self.seconds[second as usize % self.seconds.len()];
            update_stamped(&slot.calls, second, 1, false);
            update_stamped(&slot.total_us, second, elapsed / 1_000, false);
            update_stamped(&slot.max_us, second, elapsed / 1_000, true);
        }

        fn total(&self) -> CostWindow {
            CostWindow {
                calls: self.calls.load(Ordering::Relaxed),
                total_ns: self.total_ns.load(Ordering::Relaxed),
                max_ns: self.max_ns.load(Ordering::Relaxed),
            }
        }

        fn window(&self, second: u64) -> CostWindow {
            let slot = &self.seconds[second as usize % self.seconds.len()];
            CostWindow {
                calls: read_stamped(&slot.calls, second),
                total_ns: read_stamped(&slot.total_us, second) * 1_000,
                max_ns: read_stamped(&slot.max_us, second) * 1_000,
            }
        }

        fn reset(&self) {
            self.calls.store(0, Ordering::Relaxed);
            self.total_ns.store(0, Ordering::Relaxed);
            self.max_ns.store(0, Ordering::Relaxed);
            for slot in &self.seconds {
                slot.calls.store(0, Ordering::Relaxed);
                slot.total_us.store(0, Ordering::Relaxed);
                slot.max_us.store(0, Ordering::Relaxed);
            }
        }
    }

    struct ProfileSession {
        origin: Instant,
        started_ns: AtomicU64,
        stopped_ns: AtomicU64,
        summary_second: AtomicU64,
        cells: Box<[CostCell]>,
        #[cfg(test)]
        clock_reads: AtomicU64,
    }

    impl ProfileSession {
        fn new(count: usize) -> Self {
            Self {
                origin: Instant::now(),
                started_ns: AtomicU64::new(0),
                stopped_ns: AtomicU64::new(0),
                summary_second: AtomicU64::new(0),
                cells: (0..count).map(|_| CostCell::default()).collect(),
                #[cfg(test)]
                clock_reads: AtomicU64::new(0),
            }
        }

        fn now(&self) -> u64 {
            #[cfg(test)]
            self.clock_reads.fetch_add(1, Ordering::Relaxed);
            self.origin.elapsed().as_nanos().min(u64::MAX as u128) as u64
        }

        fn second(&self, now: u64) -> u64 {
            now.saturating_sub(self.started_ns.load(Ordering::Relaxed)) / 1_000_000_000
        }

        fn reset(&self) {
            for cell in &self.cells {
                cell.reset();
            }
            let now = self.now();
            self.started_ns.store(now, Ordering::Relaxed);
            self.stopped_ns.store(now, Ordering::Relaxed);
            self.summary_second.store(0, Ordering::Relaxed);
        }
    }

    pub fn initialize(
        ids: &[&'static str],
        count: usize,
        hooks: &'static [&'static str],
    ) -> Result<(), &'static str> {
        let mode = panic_mode(std::env::var("HORIZON_MOD_PANIC").ok().as_deref());
        let registry = Registry::new(ids, count, hooks, mode);
        if std::env::var("HORIZON_MOD_PROFILE").as_deref() == Ok("1") {
            registry.set_profile_enabled(true)?;
        }
        REGISTRY
            .set(registry)
            .map_err(|_| "Mod guard already initialized")
    }

    pub fn dispatch(count: usize, hook: &'static str) -> Option<Dispatch<'static>> {
        if count == 0 {
            return None;
        }
        match REGISTRY.get() {
            Some(registry) => registry.dispatch(count, hook),
            None => Some(Dispatch {
                registry: None,
                hook,
                lease: None,
            }),
        }
    }

    pub fn mod_statuses() -> Vec<ModStatus> {
        REGISTRY.get().map_or_else(Vec::new, Registry::statuses)
    }

    pub fn take_notices(server: bool) -> Vec<ModNotice> {
        REGISTRY
            .get()
            .map_or_else(Vec::new, |registry| registry.take_notices(server))
    }

    pub fn profile_enabled() -> bool {
        REGISTRY
            .get()
            .is_some_and(|registry| registry.profiling.load(Ordering::Relaxed))
    }

    pub fn set_profile_enabled(enabled: bool) -> Result<(), &'static str> {
        REGISTRY
            .get()
            .ok_or("Mod guard is not initialized")?
            .set_profile_enabled(enabled)
    }

    pub fn profile_snapshot() -> Vec<ModCost> {
        REGISTRY
            .get()
            .map_or_else(Vec::new, Registry::profile_snapshot)
    }

    pub fn maintenance() {
        if let Some(registry) = REGISTRY.get() {
            registry.maintenance();
        }
    }

    #[cfg(test)]
    mod tests {
        use super::*;

        #[test]
        fn containment_restores_marks_and_quarantines_all_hooks() {
            let registry = Registry::new(
                &["broken", "good"],
                2,
                &["tick", "render"],
                PanicMode::Contain,
            );
            let outer = super::super::enter_native_mod(super::super::NativeModCall {
                id: Some("outer"),
                index: 9,
                hook: "outer",
            });
            let dispatch = registry.dispatch(2, "tick").unwrap();
            assert_eq!(
                dispatch.call(0, || panic!("broken tick")),
                CallOutcome::<()>::Panicked,
            );
            assert_eq!(super::super::native_mod_call().unwrap().id, Some("outer"));
            assert_eq!(
                dispatch.call(0, || panic!("must not run")),
                CallOutcome::<()>::Skipped,
            );
            assert_eq!(
                registry.dispatch(2, "render").unwrap().call(0, || ()),
                CallOutcome::Skipped,
            );
            assert_eq!(dispatch.call(1, || 7), CallOutcome::Completed(7));
            assert_eq!(registry.take_notices(false).len(), 1);
            assert!(registry.take_notices(false).is_empty());
            assert_eq!(registry.take_notices(true).len(), 1);
            assert_eq!(
                registry.statuses()[0].first_error.as_ref().unwrap().hook,
                "tick",
            );
            drop(outer);
        }

        #[test]
        fn developer_mode_keeps_unwinding() {
            assert_eq!(panic_mode(Some("crash")), PanicMode::Crash);
            assert_eq!(panic_mode(None), PanicMode::Contain);
            let registry = Registry::new(&[], 1, &["tick"], panic_mode(Some("crash")));
            assert!(
                std::panic::catch_unwind(AssertUnwindSafe(|| {
                    registry
                        .dispatch(1, "tick")
                        .unwrap()
                        .call(0, || panic!("unwind"))
                }))
                .is_err(),
            );
            assert!(registry.statuses()[0].enabled);
        }

        #[test]
        fn profiling_has_no_clock_or_counter_work_when_off_or_empty() {
            let registry = Registry::new(&[], 1, &["tick"], PanicMode::Contain);
            registry.dispatch(1, "tick").unwrap().call(0, || ());
            assert!(registry.profile.get().is_none());
            registry.set_profile_enabled(true).unwrap();
            let session = registry.profile.get().unwrap();
            let before = session.clock_reads.load(Ordering::Relaxed);
            assert!(registry.dispatch(0, "tick").is_none());
            assert_eq!(before, session.clock_reads.load(Ordering::Relaxed));
            registry.dispatch(1, "tick").unwrap().call(0, || {
                std::thread::sleep(std::time::Duration::from_millis(1));
            });
            {
                let _server = enter_thread(ModThread::Server);
                registry.dispatch(1, "tick").unwrap().call(0, || ());
            }
            assert_eq!(registry.profile_snapshot().len(), 2);
            let costs = registry.profile_snapshot();
            assert_eq!(costs[0].since_enabled.calls, 1);
            assert!(costs[0].since_enabled.total_ns > 0);
            registry.set_profile_enabled(false).unwrap();
            let before = session.clock_reads.load(Ordering::Relaxed);
            registry.dispatch(1, "tick").unwrap().call(0, || ());
            assert_eq!(before, session.clock_reads.load(Ordering::Relaxed));
            assert_eq!(registry.profile_snapshot()[0].since_enabled.calls, 1);
        }

        #[test]
        fn completed_seconds_roll_over_without_sleeping() {
            let cell = CostCell::default();
            cell.record(0, 12_000);
            cell.record(0, 5_000);
            cell.record(1, 20_000);
            assert_eq!(
                cell.window(0),
                CostWindow {
                    calls: 2,
                    total_ns: 17_000,
                    max_ns: 12_000,
                },
            );
            assert_eq!(cell.window(1).calls, 1);
            cell.record(3, 1_000);
            assert_eq!(cell.window(0), CostWindow::default());
            assert_eq!(cell.total().calls, 4);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_release_reports_its_built_in_version() {
        // release.yml builds with HORIZON_VERSION (docs/versioning.md): `v0.2.3`, wherever it is installed.
        if let Some(label) = horizon_core::release::nightly_label() {
            assert_eq!(super::version(), label);
            assert_eq!(release_tag(), horizon_core::release::nightly_version());
            return;
        }
        match horizon_core::release::RELEASE {
            Some(version) => {
                assert_eq!(release_tag(), Some(format!("v{version}")));
                assert_eq!(super::version(), format!("v{version}"));
            }
            None => assert!(release_tag().is_none() || super::version().contains("(client ")),
        }
    }

    #[test]
    fn a_panic_in_a_native_mods_hook_names_it() {
        let outer = enter_native_mod(NativeModCall {
            id: None,
            index: 1,
            hook: "render",
        });
        let inner = enter_native_mod(NativeModCall {
            id: Some("b"),
            index: 0,
            hook: "tick",
        });
        assert!(attributed("boom".into()).ends_with("\nin native mod b, hook tick"));
        drop(inner);
        assert!(attributed("boom".into()).ends_with("\nin native mod #1, hook render"));
        drop(outer);
        assert_eq!(attributed("boom".into()), "boom");
    }

    fn report() -> Report {
        Report {
            time: chrono::TimeZone::with_ymd_and_hms(&Local, 2026, 10, 14, 9, 5, 7).unwrap(),
            description: "Unexpected error".into(),
            error: "panicked at src/game.rs:10:5:\nboom".into(),
            stack: "horizon_client::game::frame (src/game.rs:10:5)\n".into(),
            thread: "Render thread".into(),
            side: "client",
            details: vec![
                ("Horizon Version".into(), "0.1.0".into()),
                ("Minecraft Version".into(), "26.3".into()),
                ("Graphics".into(), "Test GPU (Vulkan), driver 1.0".into()),
            ],
            log: vec!["[09:05:06] [Render thread/INFO]: load assets: 1.20s".into()],
        }
    }

    #[test]
    fn file_names_are_vanillas() {
        assert_eq!(
            file_name(&report().time, "client"),
            "crash-2026-10-14_09.05.07-client.txt"
        );
    }

    #[test]
    fn the_report_reads_like_vanillas() {
        let text = report().text();
        let lines: Vec<&str> = text.lines().collect();
        assert_eq!(lines[0], "---- Horizon Crash Report ----");
        assert!(lines[1].starts_with("// "));
        assert_eq!(lines[3], "Time: 2026-10-14 09:05:07");
        assert_eq!(lines[4], "Description: Unexpected error");
        assert_eq!(lines[6], "panicked at src/game.rs:10:5:");
        assert_eq!(lines[7], "boom");
        assert_eq!(
            lines[8],
            "\tat horizon_client::game::frame (src/game.rs:10:5)"
        );
        assert!(text.contains("-- Head --\nThread: Render thread\n"));
        assert!(text.contains(
            "-- System Details --\nDetails:\n\tHorizon Version: 0.1.0\n\tMinecraft Version: 26.3\n\tGraphics: Test GPU (Vulkan), driver 1.0\n"
        ));
        assert!(text.ends_with(
            "-- Last 1 log lines --\n[09:05:06] [Render thread/INFO]: load assets: 1.20s\n"
        ));
    }

    #[test]
    fn a_dedicated_servers_report_names_its_server_thread_and_no_render_settings() {
        let unnamed = |side: &'static str| {
            std::thread::spawn(move || thread_name(side))
                .join()
                .unwrap()
        };
        assert_eq!(unnamed("server"), "Server thread");
        assert_eq!(unnamed("client"), "Render thread");
        set_detail("Render Settings", "render distance 12");
        let has = |side| {
            system_details(side)
                .iter()
                .any(|(k, _)| k == "Render Settings")
        };
        assert!(has("client"));
        assert!(!has("server"));
        clear_detail("Render Settings");
    }

    #[test]
    fn stacks_start_at_the_panicking_frame() {
        let bt = format!(
            "   0: horizon_client::crash::panic_report\n             at {root}/src/crash.rs:296:16\n   1: std::panicking::panic_with_hook\n             at /rustc/abc/library/std/src/panicking.rs:823:13\n   2: core::panicking::panic_fmt\n             at /rustc/abc/library/core/src/panicking.rs:80:14\n   3: horizon_client::crash::debug_crash_tick\n             at {root}/src/crash.rs:359:9\n   4: winit::dispatch\n             at /home/someone/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f/winit-0.30.13/src/event_loop.rs:642:56\n   5: main\n",
            root = env!("CARGO_MANIFEST_DIR")
        );
        assert_eq!(
            stack_lines(&bt, true),
            "horizon_client::crash::debug_crash_tick (crates/horizon-client-render/src/crash.rs:359:9)\nwinit::dispatch (winit-0.30.13/src/event_loop.rs:642:56)\nmain\n"
        );
    }

    #[test]
    fn contained_panics_come_back_as_errors() {
        assert_eq!(contain(|| 7), Ok(7));
        let error = contain(|| -> u8 { panic!("a pack's program") }).unwrap_err();
        assert!(error.contains("a pack's program"), "{error}");
        // Nested, and the thread is no longer inside one after.
        let outer = contain(|| contain(|| -> u8 { panic!("inner") }));
        assert!(outer.unwrap().unwrap_err().contains("inner"));
        assert_eq!(CONTAINED.with(Cell::get), 0);
    }

    #[test]
    fn the_debug_crash_is_reported_as_vanillas() {
        let error: Box<dyn std::error::Error> = DebugCrash::new().into();
        assert_eq!(error.to_string(), "Manually triggered debug crash");
        let debug = error.downcast_ref::<DebugCrash>().unwrap();
        // The stack starts where the crash was raised (DebugCrash::new, in this file).
        let first = debug.stack.lines().next().unwrap();
        assert!(first.contains("crash.rs"), "{}", debug.stack);
    }

    #[test]
    fn worker_threads_are_the_streaming_pool() {
        assert!(is_worker_thread(Some("horizon-worker-3")));
        assert!(!is_worker_thread(Some("main")));
        assert!(!is_worker_thread(None));
    }

    #[test]
    fn a_panic_becomes_a_report() {
        // A real panic through a hook, as the game's would be.
        let caught = std::sync::Arc::new(Mutex::new(Vec::new()));
        let sink = caught.clone();
        let previous = std::panic::take_hook();
        std::panic::set_hook(Box::new(move |info| {
            if panic_message(info).contains("the test crash") {
                sink.lock().unwrap().push(panic_report(info, "client"));
            }
        }));
        let _ = std::panic::catch_unwind(|| panic!("the test crash"));
        // The same panic inside a native mod's hook, which the hook sees before the unwind.
        let _ = std::panic::catch_unwind(|| {
            let _guard = enter_native_mod(NativeModCall {
                id: Some("a"),
                index: 0,
                hook: "tick",
            });
            panic!("the test crash")
        });
        std::panic::set_hook(previous);
        // A caught panic leaves the thread as it found it.
        assert_eq!(attributed("boom".into()), "boom");
        let mut reports = std::mem::take(&mut *caught.lock().unwrap());
        assert_eq!(reports.len(), 2, "the hook ran for both panics");
        let in_mod = reports.pop().unwrap();
        assert!(
            in_mod.error.ends_with("\nin native mod a, hook tick"),
            "{}",
            in_mod.error
        );
        let report = reports.pop().unwrap();
        assert!(!report.error.contains("native mod"));
        assert!(report.error.contains("the test crash"));
        assert!(report.error.contains("crash.rs"));
        // The first frame is the test's own code, not the panic machinery.
        let first = report.stack.lines().next().unwrap();
        assert!(first.contains("crash.rs"), "{}", report.stack);
        let text = report.text();
        assert!(text.contains("Description: Unexpected error"));
        assert!(text.contains(&format!(
            "\tMinecraft Version: {}\n",
            horizon_core::VANILLA_VERSION
        )));
        assert!(text.contains("\tOperating System: "));
        assert!(text.contains("\tCPU: "));
        let home = std::env::temp_dir().join(format!("horizon-crash-save-{}", std::process::id()));
        let path = report.save(&home).unwrap();
        assert_eq!(std::fs::read_to_string(&path).unwrap(), text);
        assert!(
            path.file_name()
                .unwrap()
                .to_str()
                .unwrap()
                .ends_with("-client.txt")
        );
        let _ = std::fs::remove_dir_all(&home);
    }
}
