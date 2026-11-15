//! Process hygiene of a test run: what a worker inherits, how a worker proves it was started by the
//! coordinator, random names, the output tail, and making sure no worker outlives its coordinator.
#![allow(unsafe_code)]

use std::collections::{HashSet, hash_map::RandomState};
use std::collections::VecDeque;
use std::ffi::{OsStr, OsString};
use std::hash::{BuildHasher, Hasher};
use std::io::Read;
use std::path::{Path, PathBuf};
use std::process::Child;
use std::sync::{Arc, Mutex};

use crate::plan::WorkerArgs;

/// The environment variable that carries a worker's token.
pub const TOKEN_ENV: &str = "YOAKE_TEST_TOKEN";
/// The name of the file in a worker's folder that holds the same token.
pub const TOKEN_FILE: &str = "token";
/// The folder name of the test world inside the worker's `saves/`.
pub const WORLD: &str = "yoake-test";
/// The name of a worker's result file.
pub const RESULT_FILE: &str = "result.json";

/// `bytes` random bytes as lowercase hex. Each `RandomState` is seeded by the operating system, so the
/// value cannot be guessed from the time or the process id.
pub fn random_hex(bytes: usize) -> String {
    let mut text = String::new();
    while text.len() < bytes * 2 {
        let word = RandomState::new().build_hasher().finish();
        text.push_str(&format!("{word:016x}"));
    }
    text.truncate(bytes * 2);
    text
}

// ---------------------------------------------------------------------------------------------
// The worker's environment
// ---------------------------------------------------------------------------------------------

/// Variable names a worker keeps from the coordinator's environment (compared without case): what the
/// system needs to find its files and a GPU driver. Everything else is dropped, so no `HORIZON_*`
/// switch (session file, command script, control socket, player name, fixtures), no `YOAKE_*` switch
/// (`YOAKE_DLL_MODS`, `YOAKE_DLL_MODS_UNSIGNED`, `YOAKE_WASM_DEV_DIR`) and nothing else reaches it.
const KEPT: &[&str] = &[
    "PATH", "SYSTEMROOT", "WINDIR", "COMSPEC", "PATHEXT", "TEMP", "TMP", "TMPDIR", "USERPROFILE", "APPDATA", "LOCALAPPDATA",
    "PROGRAMDATA", "PROGRAMFILES", "PROGRAMFILES(X86)", "PROGRAMW6432", "COMMONPROGRAMFILES", "SYSTEMDRIVE", "HOMEDRIVE",
    "HOMEPATH", "NUMBER_OF_PROCESSORS", "PROCESSOR_ARCHITECTURE", "OS", "HOME", "USER", "LOGNAME", "LANG", "LC_ALL",
    "XDG_RUNTIME_DIR", "DISPLAY", "WAYLAND_DISPLAY", "LD_LIBRARY_PATH", "DYLD_LIBRARY_PATH", "DRI_PRIME", "RUST_BACKTRACE",
    "__NV_PRIME_RENDER_OFFLOAD", "__GLX_VENDOR_LIBRARY_NAME", "ALLUSERSPROFILE",
];
/// Prefixes of the graphics stack's own settings.
const KEPT_PREFIXES: &[&str] = &["VK_", "WGPU_", "MESA_", "LIBGL_", "DXVK_", "__GL_", "NVIDIA_", "AMD_", "INTEL_"];

/// The part of `vars` a worker may inherit.
pub fn worker_env(vars: impl IntoIterator<Item = (OsString, OsString)>) -> Vec<(OsString, OsString)> {
    let kept: HashSet<&str> = KEPT.iter().copied().collect();
    vars.into_iter()
        .filter(|(name, _)| {
            let upper = name.to_string_lossy().to_ascii_uppercase();
            kept.contains(upper.as_str()) || KEPT_PREFIXES.iter().any(|prefix| upper.starts_with(prefix))
        })
        .collect()
}

// ---------------------------------------------------------------------------------------------
// Proving a worker is a worker
// ---------------------------------------------------------------------------------------------

fn once<'a>(args: &'a [String], flag: &str) -> Result<Option<&'a str>, String> {
    let positions: Vec<usize> = args.iter().enumerate().filter(|(_, argument)| *argument == flag).map(|(index, _)| index).collect();
    match positions[..] {
        [] => Ok(None),
        [index] => match args.get(index + 1) {
            Some(value) if !value.starts_with('-') => Ok(Some(value)),
            _ => Err(format!("{flag} needs a value")),
        },
        _ => Err(format!("{flag} is given twice")),
    }
}

fn absolute(path: &str) -> Result<PathBuf, String> {
    std::path::absolute(path).map_err(|error| format!("{path}: {error}"))
}

/// Decides from the raw command line (without the program name) and the worker token of the environment
/// whether this process is a test worker. `Ok(None)`: it is not one (no `--yoake-test-run`). `Ok(Some)`:
/// the coordinator started it, with absolute paths. `Err`: the flags are there without the proof, and the
/// process must stop: the token matches the token file in the result's folder, the game is offscreen, the
/// world is `yoake-test` and the game folder is the `game` folder next to that result.
pub fn verify_worker(args: &[String], token: Option<&str>) -> Result<Option<WorkerArgs>, String> {
    if once(args, "--yoake-test-run")?.is_none() {
        return Ok(None);
    }
    let mut worker = WorkerArgs::default();
    for flag in WorkerArgs::FLAGS {
        if let Some(value) = once(args, flag)? {
            worker.set(flag, value.to_owned())?;
        }
    }
    let (Some(module), Some(result)) = (worker.module.take(), worker.result.take()) else {
        return Err("--yoake-test-run needs --yoake-test-module and --yoake-test-result".into());
    };
    let (module, result) = (absolute(&module.to_string_lossy())?, absolute(&result.to_string_lossy())?);
    let token = token.filter(|token| token.len() >= 32).ok_or("this is not a test worker: the coordinator's token is missing")?;
    let folder = result.parent().ok_or("the result path has no folder")?;
    if result.file_name() != Some(OsStr::new(RESULT_FILE)) {
        return Err(format!("the result must be {RESULT_FILE} in the worker's folder"));
    }
    let written = std::fs::read_to_string(folder.join(TOKEN_FILE)).map_err(|_| "this is not a test worker: no token file beside the result".to_owned())?;
    if written.trim() != token {
        return Err("this is not a test worker: the token does not match".into());
    }
    if !args.iter().any(|argument| argument == "--offscreen") {
        return Err("a test worker runs with --offscreen".into());
    }
    if once(args, "--world")? != Some(WORLD) {
        return Err(format!("a test worker's world is {WORLD}"));
    }
    let game = once(args, "--game-dir")?.ok_or("a test worker has a --game-dir")?;
    if absolute(game)? != folder.join("game") {
        return Err("a test worker's game folder is the game folder beside its result".into());
    }
    worker.module = Some(module);
    worker.result = Some(result);
    Ok(Some(worker))
}

// ---------------------------------------------------------------------------------------------
// The output tail
// ---------------------------------------------------------------------------------------------

/// The last bytes a worker wrote to its output and error streams. The streams are read by threads, so a
/// chatty worker cannot fill a disk.
pub struct Tail {
    bytes: Arc<Mutex<VecDeque<u8>>>,
}

/// Most bytes kept.
pub const TAIL_BYTES: usize = 8 * 1024;

impl Tail {
    /// Starts reading `child`'s piped output and error streams.
    pub fn capture(child: &mut Child) -> Self {
        let bytes = Arc::new(Mutex::new(VecDeque::new()));
        let streams: [Option<Box<dyn Read + Send>>; 2] = [
            child.stdout.take().map(|stream| Box::new(stream) as Box<dyn Read + Send>),
            child.stderr.take().map(|stream| Box::new(stream) as Box<dyn Read + Send>),
        ];
        for mut stream in streams.into_iter().flatten() {
            let bytes = Arc::clone(&bytes);
            let _ = std::thread::Builder::new().name("yoake-test-output".into()).spawn(move || {
                let mut chunk = [0u8; 4096];
                while let Ok(read @ 1..) = stream.read(&mut chunk) {
                    let mut kept = bytes.lock().unwrap_or_else(|error| error.into_inner());
                    kept.extend(&chunk[..read]);
                    let excess = kept.len().saturating_sub(TAIL_BYTES);
                    kept.drain(..excess);
                }
            });
        }
        Self { bytes }
    }

    /// What was written last, as one line of text without control characters.
    pub fn text(&self, limit: usize) -> String {
        let kept = self.bytes.lock().unwrap_or_else(|error| error.into_inner());
        let bytes: Vec<u8> = kept.iter().copied().collect();
        let text = String::from_utf8_lossy(&bytes);
        let text: String = text.chars().map(|character| if character.is_control() && character != '\n' { ' ' } else { character }).collect();
        let text = text.trim();
        let skip = text.chars().count().saturating_sub(limit);
        text.chars().skip(skip).collect::<String>().trim().replace('\n', " | ")
    }
}

// ---------------------------------------------------------------------------------------------
// No worker outlives its coordinator
// ---------------------------------------------------------------------------------------------

/// On Windows a job object with kill-on-close: the workers put in it die with the coordinator, however it
/// ends (Ctrl+C, a crash, a kill). Elsewhere it does nothing (workers watch their parent instead,
/// [`parent_gone`]).
pub struct Job {
    #[cfg(windows)]
    handle: windows_sys::Win32::Foundation::HANDLE,
}

#[cfg(windows)]
impl Job {
    pub fn new() -> Option<Self> {
        use windows_sys::Win32::System::JobObjects::{
            CreateJobObjectW, JOB_OBJECT_LIMIT_KILL_ON_JOB_CLOSE, JOBOBJECT_EXTENDED_LIMIT_INFORMATION, JobObjectExtendedLimitInformation,
            SetInformationJobObject,
        };
        // SAFETY: plain Win32 calls with a zeroed information record of the type the class asks for; the
        // handle is closed in `Drop`.
        unsafe {
            let handle = CreateJobObjectW(std::ptr::null(), std::ptr::null());
            if handle.is_null() {
                return None;
            }
            let mut info: JOBOBJECT_EXTENDED_LIMIT_INFORMATION = std::mem::zeroed();
            info.BasicLimitInformation.LimitFlags = JOB_OBJECT_LIMIT_KILL_ON_JOB_CLOSE;
            let set = SetInformationJobObject(
                handle,
                JobObjectExtendedLimitInformation,
                (&raw const info).cast(),
                std::mem::size_of::<JOBOBJECT_EXTENDED_LIMIT_INFORMATION>() as u32,
            );
            if set == 0 {
                windows_sys::Win32::Foundation::CloseHandle(handle);
                return None;
            }
            Some(Self { handle })
        }
    }

    /// Puts `child` in the job; false when the system refused (the child is then only killed by the coordinator).
    pub fn add(&self, child: &Child) -> bool {
        use std::os::windows::io::AsRawHandle;
        // SAFETY: both handles are valid for the call.
        unsafe { windows_sys::Win32::System::JobObjects::AssignProcessToJobObject(self.handle, child.as_raw_handle().cast()) != 0 }
    }
}

#[cfg(windows)]
impl Drop for Job {
    fn drop(&mut self) {
        // SAFETY: the handle came from `CreateJobObjectW` and is closed once; closing kills what is left in the job.
        unsafe {
            windows_sys::Win32::Foundation::CloseHandle(self.handle);
        }
    }
}

#[cfg(not(windows))]
impl Job {
    pub fn new() -> Option<Self> {
        Some(Self {})
    }

    pub fn add(&self, _child: &Child) -> bool {
        false
    }
}

static INTERRUPTED: std::sync::atomic::AtomicBool = std::sync::atomic::AtomicBool::new(false);

/// Windows: makes Ctrl+C and Ctrl+Break set a flag ([`interrupted`]) instead of ending the coordinator at
/// once, so the run can kill its worker and delete its temporary folder. (Elsewhere the game's own signal
/// handler counts the request, `horizon_threads::stop_requests`.)
pub fn install_interrupt_handler() {
    #[cfg(windows)]
    {
        unsafe extern "system" fn handler(_kind: u32) -> i32 {
            INTERRUPTED.store(true, std::sync::atomic::Ordering::SeqCst);
            1
        }
        // SAFETY: registers a handler that only stores to an atomic.
        unsafe {
            windows_sys::Win32::System::Console::SetConsoleCtrlHandler(Some(handler), 1);
        }
    }
}

/// Whether Ctrl+C or Ctrl+Break arrived since [`install_interrupt_handler`].
pub fn interrupted() -> bool {
    INTERRUPTED.load(std::sync::atomic::Ordering::SeqCst)
}

/// A worker's check that its coordinator is still there. Unix: the parent process id changes when the
/// parent dies. Windows: the job object above kills the worker, so this is always false.
pub fn parent_gone(original: Option<u32>) -> bool {
    #[cfg(unix)]
    {
        original.is_some_and(|original| std::os::unix::process::parent_id() != original)
    }
    #[cfg(not(unix))]
    {
        let _ = original;
        false
    }
}

/// The process id of this process's parent, as a worker reads it at start (Unix only).
pub fn parent_id() -> Option<u32> {
    #[cfg(unix)]
    {
        Some(std::os::unix::process::parent_id())
    }
    #[cfg(not(unix))]
    {
        None
    }
}

/// Reads at most `limit` bytes of the file at `path` as text (`None` when it cannot be read).
pub fn read_limited(path: &Path, limit: u64) -> Option<String> {
    let file = std::fs::File::open(path).ok()?;
    let mut bytes = Vec::new();
    file.take(limit + 1).read_to_end(&mut bytes).ok()?;
    if bytes.len() as u64 > limit {
        return None;
    }
    Some(String::from_utf8_lossy(&bytes).into_owned())
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;

    fn args(list: &[&str]) -> Vec<String> {
        list.iter().map(|argument| (*argument).to_owned()).collect()
    }

    fn pair(name: &str, value: &str) -> (OsString, OsString) {
        (name.into(), value.into())
    }

    #[test]
    fn random_names_differ_and_have_the_asked_length() {
        let (one, two) = (random_hex(16), random_hex(16));
        assert_eq!((one.len(), two.len()), (32, 32));
        assert_ne!(one, two);
        assert!(one.bytes().all(|byte| byte.is_ascii_hexdigit()));
        assert_eq!(random_hex(3).len(), 6);
    }

    #[test]
    fn a_worker_keeps_the_systems_variables_and_nothing_of_the_games_or_the_mods() {
        let kept = worker_env([
            pair("PATH", "/bin"),
            pair("SystemRoot", "C:\\Windows"),
            pair("TEMP", "t"),
            pair("VK_ICD_FILENAMES", "icd.json"),
            pair("WGPU_BACKEND", "vulkan"),
            pair("HORIZON_SESSION_FILE", "session.json"),
            pair("HORIZON_COMMAND_SCRIPT", "s"),
            pair("HORIZON_COMMAND_OUTPUT", "o"),
            pair("HORIZON_CONTROL", "c"),
            pair("HORIZON_PLAYER_NAME", "n"),
            pair("HORIZON_SPLIT", "2"),
            pair("YOAKE_DLL_MODS", "evil.dll"),
            pair("YOAKE_DLL_MODS_UNSIGNED", "1"),
            pair("YOAKE_WASM_DEV_DIR", "d"),
            pair("RANDOM_SECRET", "x"),
        ]);
        let names: Vec<String> = kept.iter().map(|(name, _)| name.to_string_lossy().into_owned()).collect();
        assert_eq!(names, ["PATH", "SystemRoot", "TEMP", "VK_ICD_FILENAMES", "WGPU_BACKEND"]);
    }

    /// A folder like a coordinator makes it: `<root>/token`, and `game` beside it.
    fn worker_folder(token: &str) -> PathBuf {
        let folder = std::env::temp_dir().join(format!("yoake-guard-{}", random_hex(6)));
        fs::create_dir_all(folder.join("game")).unwrap();
        fs::write(folder.join(TOKEN_FILE), token).unwrap();
        folder
    }

    fn worker_args(folder: &Path) -> Vec<String> {
        let path = |name: &str| folder.join(name).display().to_string();
        args(&[
            "--offscreen", "--no-focus", "--game-dir", &path("game"), "--world", "yoake-test", "--yoake-test-run", "places",
            "--yoake-test-module", "tests.wasm", "--yoake-test-result", &path("result.json"),
        ])
    }

    #[test]
    fn a_worker_must_prove_it_was_started_by_the_coordinator() {
        let token = random_hex(16);
        let folder = worker_folder(&token);
        let good = worker_args(&folder);
        // Not a worker at all.
        assert_eq!(verify_worker(&args(&["--offscreen"]), None), Ok(None));
        let worker = verify_worker(&good, Some(&token)).unwrap().unwrap();
        assert_eq!(worker.test.as_deref(), Some("places"));
        assert!(worker.module.unwrap().is_absolute(), "a relative module path is made absolute before the game changes directory");
        assert_eq!(worker.result, Some(folder.join("result.json")));

        let with = |edit: &dyn Fn(&mut Vec<String>)| {
            let mut changed = good.clone();
            edit(&mut changed);
            verify_worker(&changed, Some(&token)).unwrap_err()
        };
        assert!(verify_worker(&good, None).unwrap_err().contains("token is missing"));
        assert!(verify_worker(&good, Some("short")).unwrap_err().contains("token is missing"));
        assert!(verify_worker(&good, Some(&random_hex(16))).unwrap_err().contains("does not match"));
        assert!(with(&|a| a.retain(|x| x != "--offscreen")).contains("--offscreen"));
        assert!(with(&|a| { let i = a.iter().position(|x| x == "yoake-test").unwrap(); a[i] = "mine".into(); }).contains("world"));
        assert!(with(&|a| { let i = a.iter().position(|x| x.ends_with("result.json")).unwrap(); a[i] = folder.join("other.json").display().to_string(); }).contains("result.json"));
        assert!(with(&|a| { let i = a.iter().position(|x| x == "--game-dir").unwrap(); a[i + 1] = std::env::temp_dir().display().to_string(); }).contains("game folder"));
        assert!(with(&|a| a.extend(args(&["--world", "other"]))).contains("twice"));
        // A result in a folder without the token file proves nothing.
        let bare = std::env::temp_dir().join(format!("yoake-guard-{}", random_hex(6)));
        fs::create_dir_all(bare.join("game")).unwrap();
        assert!(verify_worker(&worker_args(&bare), Some(&token)).unwrap_err().contains("no token file"));
        let _ = (fs::remove_dir_all(folder), fs::remove_dir_all(bare));
    }

    #[test]
    fn the_tail_keeps_only_the_end_of_a_chatty_output() {
        let mut child = std::process::Command::new(std::env::current_exe().unwrap())
            .args(["--exact", "guard::tests::chatty", "--nocapture"])
            .env("YOAKE_GUARD_CHATTY", "1")
            .stdout(std::process::Stdio::piped())
            .stderr(std::process::Stdio::piped())
            .spawn()
            .unwrap();
        let tail = Tail::capture(&mut child);
        child.wait().unwrap();
        for _ in 0..100 {
            if tail.text(100_000).contains("the end") {
                break;
            }
            std::thread::sleep(std::time::Duration::from_millis(50));
        }
        let text = tail.text(100_000);
        assert!(text.contains("the end"), "{}", &text[text.len().saturating_sub(120)..]);
        assert!(text.len() <= TAIL_BYTES * 3, "{}", text.len());
    }

    /// The chatty child of the test above: it writes 1 MiB, then a last line.
    #[test]
    fn chatty() {
        if std::env::var_os("YOAKE_GUARD_CHATTY").is_none() {
            return;
        }
        for _ in 0..1024 {
            println!("{}", "x".repeat(1023));
        }
        println!("the end");
    }

    #[test]
    fn a_limited_read_refuses_what_is_too_big() {
        let path = std::env::temp_dir().join(format!("yoake-guard-read-{}", random_hex(6)));
        fs::write(&path, vec![b'a'; 100]).unwrap();
        assert_eq!(read_limited(&path, 100).map(|text| text.len()), Some(100));
        assert_eq!(read_limited(&path, 99), None);
        assert_eq!(read_limited(&path.with_extension("none"), 99), None);
        let _ = fs::remove_file(path);
    }
}
