//! Handing a room to the game: a private file, and its path as the game's only argument.
//!
//! What the portal gave (the host token, the ticket, the grant) never goes on a command line, in
//! an environment variable, a log or an error message. It is written once to a file that only
//! this user can read (`yoake_auth::create_private_file`: 0600 on Unix, a protected ACL for the
//! current user's SID alone on Windows), in a folder of its own under the launcher's data, with a
//! random name. The game reads it and deletes it. The launcher deletes what is left: when the
//! game fails to start, and at its own start (and before each new file) for files older than
//! [`MAX_AGE`].
use super::rooms::{HostedRoom, JoinGrant};
use serde_json::{json, Value};
use std::io::Write;
use std::path::{Path, PathBuf};
use std::time::{Duration, SystemTime};

/// The game's argument that takes the file's path. Keep it the same as the game's: it is set
/// with the game's side (GALLERIA's Codex).
pub const HANDOFF_ARG: &str = "--yoake-social-handoff";

/// The host's game also gets this flag (with no value): it then reads the launcher's control
/// lines on its stdin (`pair`, `reprovision`, `end`; see `host.rs`). Keep it the same as the
/// game's. The host's session goes by `--session-file`, never `--session-stdin`, since stdin is
/// the control channel.
pub const CONTROL_STDIN_ARG: &str = "--yoake-social-control-stdin";

/// A hand-off file is at most this big; the game refuses a bigger one.
pub const MAX_BYTES: usize = 64 * 1024;

/// The folder, under the launcher's data folder.
const FOLDER: &str = "social-handoff";
const PREFIX: &str = "handoff-";
const SUFFIX: &str = ".json";
/// A hand-off file older than this was never read.
pub const MAX_AGE: Duration = Duration::from_secs(10 * 60);

/// Why a file could not be made. No detail on purpose: nothing of the room is in it.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum HandoffError {
    /// The folder or the file could not be made, or it was not private.
    Unwritable,
}

pub fn folder(data_dir: &Path) -> PathBuf {
    data_dir.join(FOLDER)
}

/// The host's file: exactly the keys the game takes (it refuses others).
pub fn host_json(room: &HostedRoom) -> Value {
    json!({
        "role": "host",
        "session_id": room.session_id,
        "host_principal": room.host_principal,
        "host_token": room.host_token.expose(),
        "provision": {
            "ticket": room.provision.ticket.expose(),
            "signature": room.provision.signature.expose(),
        },
        "relay": room.relay,
    })
}

/// The guest's file: exactly the keys the game takes.
pub fn guest_json(session_id: &str, join: &JoinGrant) -> Value {
    json!({
        "role": "guest",
        "session_id": session_id,
        "grant": join.grant.expose(),
        "signature": join.signature.expose(),
        "relay": join.relay,
    })
}

pub fn write_host(folder: &Path, room: &HostedRoom) -> Result<PathBuf, HandoffError> {
    write(folder, &host_json(room))
}

pub fn write_guest(folder: &Path, session_id: &str, join: &JoinGrant) -> Result<PathBuf, HandoffError> {
    write(folder, &guest_json(session_id, join))
}

fn write(folder: &Path, body: &Value) -> Result<PathBuf, HandoffError> {
    std::fs::create_dir_all(folder).map_err(|_| HandoffError::Unwritable)?;
    sweep(folder, MAX_AGE);
    let mut random = [0_u8; 16];
    getrandom::fill(&mut random).map_err(|_| HandoffError::Unwritable)?;
    let name: String = random.iter().map(|byte| format!("{byte:02x}")).collect();
    let path = folder.join(format!("{PREFIX}{name}{SUFFIX}"));
    let bytes = serde_json::to_vec(body).map_err(|_| HandoffError::Unwritable)?;
    if bytes.len() > MAX_BYTES {
        return Err(HandoffError::Unwritable);
    }
    let result = (|| {
        let mut file =
            crate::yoake_auth::create_private_file(&path).map_err(|_| HandoffError::Unwritable)?;
        crate::yoake_auth::check_private_file(&file, &path).map_err(|_| HandoffError::Unwritable)?;
        file.write_all(&bytes).map_err(|_| HandoffError::Unwritable)?;
        file.sync_all().map_err(|_| HandoffError::Unwritable)
    })();
    match result {
        Ok(()) => Ok(path),
        Err(error) => {
            let _ = std::fs::remove_file(&path);
            Err(error)
        }
    }
}

/// Deletes the hand-off files older than `max_age` (all of them with zero).
pub fn sweep(folder: &Path, max_age: Duration) {
    let Ok(entries) = std::fs::read_dir(folder) else {
        return;
    };
    for entry in entries.flatten() {
        let name = entry.file_name();
        let name = name.to_string_lossy();
        if !(name.starts_with(PREFIX) && name.ends_with(SUFFIX)) {
            continue;
        }
        let old = entry
            .metadata()
            .and_then(|m| m.modified())
            .is_ok_and(|at| SystemTime::now().duration_since(at).is_ok_and(|age| age >= max_age));
        if old {
            let _ = std::fs::remove_file(entry.path());
        }
    }
}

/// Deletes one file the game will not read (it did not start).
pub fn discard(path: &Path) {
    let _ = std::fs::remove_file(path);
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::BTreeSet;

    fn room() -> HostedRoom {
        serde_json::from_value(json!({
            "session_id": "s1", "host_principal": "p1", "relay_only": true,
            "host_token": "TOKEN-SECRET",
            "provision": { "ticket": { "n": 1 }, "signature": "SIG" },
            "relay": "relay.example:7000", "expires_at": 1_800_000_000
        }))
        .unwrap()
    }

    fn grant() -> JoinGrant {
        serde_json::from_value(json!({
            "grant": { "g": 1 }, "signature": "SIG2", "relay": "relay.example:7000"
        }))
        .unwrap()
    }

    fn keys(value: &Value) -> BTreeSet<&str> {
        value.as_object().unwrap().keys().map(String::as_str).collect()
    }

    fn scratch(name: &str) -> PathBuf {
        let mut random = [0_u8; 8];
        getrandom::fill(&mut random).unwrap();
        let dir = std::env::temp_dir().join(format!(
            "yoake-handoff-{name}-{}",
            random.iter().map(|b| format!("{b:02x}")).collect::<String>()
        ));
        std::fs::create_dir_all(&dir).unwrap();
        dir
    }

    #[test]
    fn the_files_have_exactly_the_keys_the_game_takes() {
        let host = host_json(&room());
        assert_eq!(
            keys(&host),
            BTreeSet::from(["role", "session_id", "host_principal", "host_token", "provision", "relay"])
        );
        assert_eq!(keys(&host["provision"]), BTreeSet::from(["ticket", "signature"]));
        assert_eq!(host["role"], "host");
        assert_eq!(host["host_token"], "TOKEN-SECRET");
        assert_eq!(host["relay"], "relay.example:7000");
        let guest = guest_json("s9", &grant());
        assert_eq!(
            keys(&guest),
            BTreeSet::from(["role", "session_id", "grant", "signature", "relay"])
        );
        assert_eq!((guest["role"].as_str(), guest["session_id"].as_str()), (Some("guest"), Some("s9")));
        assert_eq!(guest["grant"], json!({ "g": 1 }));
    }

    #[test]
    fn a_written_file_holds_the_json_under_a_random_name_and_is_private() {
        let dir = scratch("write");
        let folder = folder(&dir);
        let a = write_host(&folder, &room()).unwrap();
        let b = write_guest(&folder, "s9", &grant()).unwrap();
        assert_ne!(a, b);
        assert_eq!(a.parent(), Some(folder.as_path()));
        let name = a.file_name().unwrap().to_string_lossy().into_owned();
        assert!(name.starts_with("handoff-") && name.ends_with(".json") && name.len() > 30, "{name}");
        let read: Value = serde_json::from_slice(&std::fs::read(&a).unwrap()).unwrap();
        assert_eq!(read, host_json(&room()));
        let read: Value = serde_json::from_slice(&std::fs::read(&b).unwrap()).unwrap();
        assert_eq!(read, guest_json("s9", &grant()));

        // Only this user can read it (the same check the saved sign-in passes).
        let file = std::fs::File::open(&a).unwrap();
        assert!(crate::yoake_auth::check_private_file(&file, &a).is_ok());
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            assert_eq!(std::fs::metadata(&a).unwrap().permissions().mode() & 0o777, 0o600);
        }
        let _ = std::fs::remove_dir_all(dir);
    }

    /// The Windows ACL, read back as the system reports it: the DACL does not inherit, and it
    /// has one entry, for the current user.
    #[cfg(windows)]
    #[test]
    fn on_windows_the_file_is_for_the_current_user_alone() {
        let dir = scratch("acl");
        let path = write_host(&folder(&dir), &room()).unwrap();
        let out = std::process::Command::new("icacls").arg(&path).output().unwrap();
        let text = String::from_utf8_lossy(&out.stdout).into_owned();
        let user = std::env::var("USERNAME").unwrap();
        let entries: Vec<&str> = text
            .lines()
            .map(str::trim)
            .filter(|l| l.contains(":(") || l.contains("(F)"))
            .collect();
        // icacls prints the path and the first entry on the first line: count the grants.
        let grants = text.matches("(F)").count() + text.matches("(M)").count() + text.matches("(R)").count();
        assert_eq!(grants, 1, "{text}");
        assert!(entries.iter().any(|l| l.contains(&user)), "{text}");
        assert!(!text.contains("(I)"), "inherited entries: {text}");
        for other in ["Everyone", "BUILTIN\\Users", "Authenticated Users", "SYSTEM", "Administrators"] {
            assert!(!text.contains(other), "{other} in {text}");
        }
        let _ = std::fs::remove_dir_all(dir);
    }

    #[test]
    fn old_files_are_swept_and_other_files_are_left() {
        let dir = scratch("sweep");
        let folder = folder(&dir);
        let path = write_host(&folder, &room()).unwrap();
        let other = folder.join("notes.txt");
        std::fs::write(&other, "x").unwrap();
        // Fresh files stay; with a zero age everything of ours goes, and only ours.
        sweep(&folder, MAX_AGE);
        assert!(path.exists());
        sweep(&folder, Duration::ZERO);
        assert!(!path.exists());
        assert!(other.exists());
        // A file the game will not read is discarded by path.
        let again = write_guest(&folder, "s9", &grant()).unwrap();
        discard(&again);
        assert!(!again.exists());
        // An old file is swept by the next write.
        let old = write_host(&folder, &room()).unwrap();
        let file = std::fs::OpenOptions::new().write(true).open(&old).unwrap();
        file.set_modified(SystemTime::now() - MAX_AGE - Duration::from_secs(5)).unwrap();
        drop(file);
        let fresh = write_host(&folder, &room()).unwrap();
        assert!(!old.exists() && fresh.exists());
        let _ = std::fs::remove_dir_all(dir);
    }

    #[test]
    fn a_file_over_64_kib_is_refused_and_nothing_is_left() {
        let dir = scratch("big");
        let folder = folder(&dir);
        let mut big = room();
        big.provision.ticket =
            super::super::rooms::Redacted::new(json!({ "x": "a".repeat(MAX_BYTES) }));
        assert_eq!(write_host(&folder, &big), Err(HandoffError::Unwritable));
        assert_eq!(std::fs::read_dir(&folder).unwrap().count(), 0);
        let _ = std::fs::remove_dir_all(dir);
    }

    #[test]
    fn no_secret_is_in_a_debug_print_or_an_error() {
        let shown = format!("{:?} {:?} {:?}", room(), grant(), HandoffError::Unwritable);
        for secret in ["TOKEN-SECRET", "SIG2", "\"SIG\""] {
            assert!(!shown.contains(secret), "{shown}");
        }
        // A folder that cannot be made says nothing of the room.
        let dir = scratch("blocked");
        let file = dir.join("a-file");
        std::fs::write(&file, "x").unwrap();
        let error = write_host(&file.join("inside"), &room()).unwrap_err();
        assert!(!format!("{error:?} {error:?}").contains("TOKEN"));
        let _ = std::fs::remove_dir_all(dir);
    }
}
