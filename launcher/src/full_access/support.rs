//! S1: which of the Yoake 2.0 arguments a game accepts. A 1.0 game (or any build before the options were added) refuses
//! an unknown option and does not start, so the launcher asks the game itself: `--launch-api` prints the supported
//! options as JSON (docs/api/launch.md) and runs nothing else. The answer is cached per game executable
//! (path, size, modified time), so the process is started once per build. A game that does not answer (an old one
//! exits with 2, or it cannot be started) gets none of the new arguments.
use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::Mutex;
use std::time::SystemTime;

pub const APPROVED: &str = "--full-access-approved";
pub const DEV: &str = "--full-access-dev";
pub const SYNC: &str = "--join-mod-sync";
pub const REMOTE: &str = "--remote-mods";

/// Which new arguments the game takes.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct GameSupport {
    pub approved: bool,
    pub dev: bool,
    pub join_sync: bool,
    pub remote: bool,
}

impl GameSupport {
    /// A game that takes all four (what the tests of the arguments themselves assume).
    pub const ALL: GameSupport = GameSupport { approved: true, dev: true, join_sync: true, remote: true };
    /// An old game: none of them.
    pub const NONE: GameSupport = GameSupport { approved: false, dev: false, join_sync: false, remote: false };

    /// From `--launch-api`'s JSON reply: an option counts when `options[].name` lists it.
    pub fn from_reply(json: &str) -> GameSupport {
        let Ok(value) = serde_json::from_str::<serde_json::Value>(json) else { return Self::NONE };
        let names: Vec<&str> = value.get("options").and_then(|o| o.as_array()).map(|list| list.iter().filter_map(|o| o.get("name").and_then(|n| n.as_str())).collect()).unwrap_or_default();
        GameSupport { approved: names.contains(&APPROVED), dev: names.contains(&DEV), join_sync: names.contains(&SYNC), remote: names.contains(&REMOTE) }
    }

    /// Asks the game executable `program` (`--launch-api`, cached per build). Never opens a window.
    pub fn detect(program: &Path) -> GameSupport {
        let key = std::fs::metadata(program).ok().map(|m| (program.to_path_buf(), m.len(), m.modified().ok()));
        if let Some(key) = &key {
            if let Some(found) = CACHE.lock().unwrap_or_else(|e| e.into_inner()).as_ref().and_then(|c| c.get(key).copied()) {
                return found;
            }
        }
        let found = ask(program);
        if let (Some(key), Some(found)) = (key, found) {
            CACHE.lock().unwrap_or_else(|e| e.into_inner()).get_or_insert_with(HashMap::new).insert(key, found);
        }
        found.unwrap_or(Self::NONE)
    }
}

type CacheKey = (PathBuf, u64, Option<SystemTime>);
static CACHE: Mutex<Option<HashMap<CacheKey, GameSupport>>> = Mutex::new(None);

/// `None` when the game could not be asked (it is asked again next time); `Some(NONE)` when it answered "no".
fn ask(program: &Path) -> Option<GameSupport> {
    use std::io::Read;
    let mut child = std::process::Command::new(program)
        .arg("--launch-api")
        .stdin(std::process::Stdio::null())
        .stdout(std::process::Stdio::piped())
        .stderr(std::process::Stdio::null())
        .spawn()
        .ok()?;
    let mut out = child.stdout.take()?;
    let reader = std::thread::spawn(move || {
        let mut text = String::new();
        let _ = out.by_ref().take(1 << 20).read_to_string(&mut text);
        text
    });
    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(10);
    let status = loop {
        match child.try_wait() {
            Ok(Some(status)) => break status,
            Ok(None) if std::time::Instant::now() < deadline => std::thread::sleep(std::time::Duration::from_millis(20)),
            _ => {
                let _ = child.kill();
                let _ = child.wait();
                return None;
            }
        }
    };
    let text = reader.join().unwrap_or_default();
    Some(if status.success() { GameSupport::from_reply(&text) } else { GameSupport::NONE })
}
