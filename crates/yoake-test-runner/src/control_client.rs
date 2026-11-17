//! The driver's reach into the second client: its control endpoint (`control.json`: a loopback TCP port and
//! a token, written by the client's game, `horizon-client/src/control.rs`). One short connection per call:
//! `auth <token>`, then one command line, then one JSON line back.
use std::io::{BufRead, BufReader, Write};
use std::net::{Ipv4Addr, SocketAddr, TcpStream};
use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};

/// Longest command line and longest reply taken.
const MAX_LINE: usize = 4096;
const MAX_REPLY: u64 = 256 * 1024;

/// The commands of the control protocol a test may send a client (the first word of the line). A client is
/// the test's own, but the commands that end it, change its connection or write files stay out of reach.
const ALLOWED: &[&str] = &[
    "getblock", "chat", "chat-log", "wait-for", "status", "events", "hello", "key", "release", "look", "turn", "hotbar",
    "mouse", "tp",
];

/// Checks a command line before it is sent: one line of plain text, an allowed command.
pub fn check_command(line: &str) -> Result<(), String> {
    if line.is_empty() || line.len() > MAX_LINE || line.chars().any(char::is_control) {
        return Err("a client command is one line of plain text".into());
    }
    let mut words = line.split_whitespace();
    let word = words.next().unwrap_or_default();
    if !ALLOWED.contains(&word) {
        return Err(format!("the client command {word:?} is not available to tests (allowed: {})", ALLOWED.join(", ")));
    }
    if word == "key" {
        let key = words.next().unwrap_or_default().to_ascii_lowercase();
        // No edit modifier (Ctrl+C/X/V/A would reach the clipboard shortcuts), no debug combo (F3+...) and no
        // function key (F1 to F12 are the debug keys, the screenshot and fullscreen).
        let function_key = key.strip_prefix('f').is_some_and(|rest| !rest.is_empty() && rest.bytes().all(|byte| byte.is_ascii_digit()));
        let blocked = ["ctrl", "control", "lctrl", "rctrl", "lcontrol", "rcontrol", "super", "meta", "cmd", "command", "win", "alt", "lalt", "ralt"];
        if function_key || blocked.contains(&key.as_str()) {
            return Err(format!("the key {key:?} is not available to tests: no edit modifier, debug or function key"));
        }
    }
    Ok(())
}

/// A client's control endpoint.
#[derive(Clone, Debug)]
pub struct Control {
    path: PathBuf,
}

impl Control {
    pub fn new(path: impl Into<PathBuf>) -> Self {
        Self { path: path.into() }
    }

    pub fn path(&self) -> &Path {
        &self.path
    }

    /// Whether the endpoint file exists yet (the client writes it once its game is up).
    pub fn exists(&self) -> bool {
        self.path.is_file()
    }

    fn endpoint(&self) -> Result<(u16, String), String> {
        let text = crate::guard::read_limited(&self.path, 4096).ok_or_else(|| format!("{} is not there yet", self.path.display()))?;
        let value: serde_json::Value = serde_json::from_str(&text).map_err(|error| format!("{}: {error}", self.path.display()))?;
        let port = value.get("port").and_then(serde_json::Value::as_u64).and_then(|port| u16::try_from(port).ok()).ok_or("the control file has no port")?;
        let token = value.get("token").and_then(serde_json::Value::as_str).ok_or("the control file has no token")?;
        Ok((port, token.to_owned()))
    }

    /// Sends `line` and returns the reply line (JSON text). The reply of a deferred command (`wait-for`)
    /// takes as long as the command waits; `timeout` bounds the whole exchange.
    pub fn call(&self, line: &str, timeout: Duration) -> Result<String, String> {
        check_command(line)?;
        let (port, token) = self.endpoint()?;
        let address = SocketAddr::from((Ipv4Addr::LOCALHOST, port));
        let stream = TcpStream::connect_timeout(&address, timeout.min(Duration::from_secs(5)).max(Duration::from_millis(1)))
            .map_err(|error| format!("the client does not answer: {error}"))?;
        let _ = stream.set_nodelay(true);
        stream.set_read_timeout(Some(timeout)).map_err(|error| error.to_string())?;
        stream.set_write_timeout(Some(Duration::from_secs(5))).map_err(|error| error.to_string())?;
        let mut writer = stream.try_clone().map_err(|error| error.to_string())?;
        let mut reader = BufReader::new(std::io::Read::take(stream, MAX_REPLY));
        let mut read_line = |reader: &mut BufReader<std::io::Take<TcpStream>>| -> Result<String, String> {
            let mut reply = String::new();
            match reader.read_line(&mut reply) {
                Ok(0) => Err("the client closed the connection".to_owned()),
                Ok(_) => Ok(reply.trim_end().to_owned()),
                Err(error) => Err(format!("the client did not answer in time: {error}")),
            }
        };
        writeln!(writer, "auth {token}").map_err(|error| error.to_string())?;
        let auth = read_line(&mut reader)?;
        if !auth.contains("\"ok\":true") {
            return Err(format!("the client refused the token: {auth}"));
        }
        writeln!(writer, "{line}").map_err(|error| error.to_string())?;
        read_line(&mut reader)
    }

    /// How long a call of `line` may take: 30 s, and for `wait-for ... --timeout N` (the client answers when its
    /// condition holds or after N seconds) N seconds and a margin.
    pub fn timeout_for(line: &str) -> Duration {
        let mut words = line.split_whitespace();
        let waited = words.by_ref().skip_while(|word| *word != "--timeout").nth(1).and_then(|seconds| seconds.parse::<u64>().ok());
        match (line.starts_with("wait-for"), waited) {
            (true, Some(seconds)) => Duration::from_secs(seconds.min(3600) + 10),
            // `wait-for` has its own default of 30 s.
            (true, None) => Duration::from_secs(40),
            _ => Duration::from_secs(30),
        }
    }

    /// Repeats `line` every 50 ms until its reply contains `expect` (plain text), up to `total`. Returns that
    /// reply; on timeout the error carries the last reply.
    pub fn wait(&self, line: &str, expect: &str, total: Duration) -> Result<String, String> {
        let deadline = Instant::now() + total;
        let mut last = String::new();
        loop {
            // Each call gets what is left of the total (at most 10 s), so the wait ends when it said it would.
            let left = deadline.saturating_duration_since(Instant::now()).clamp(Duration::from_millis(50), Duration::from_secs(10));
            match self.call(line, left) {
                Ok(reply) if reply.contains(expect) => return Ok(reply),
                Ok(reply) => last = reply,
                Err(error) => last = error,
            }
            if Instant::now() >= deadline {
                return Err(format!("the client did not show {expect:?} within {} ms; its last answer to {line:?}: {last}", total.as_millis()));
            }
            std::thread::sleep(Duration::from_millis(50));
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::net::TcpListener;
    use std::sync::atomic::{AtomicU32, Ordering};
    use std::sync::Arc;

    /// A control endpoint that answers like the game's: it wants `auth TOKEN` first, then answers each line
    /// (`getblock` with a block that appears after `appear` calls).
    fn fake_client(appear: u32) -> (Control, Arc<AtomicU32>, std::thread::JoinHandle<()>, Arc<std::sync::atomic::AtomicBool>) {
        let listener = TcpListener::bind((Ipv4Addr::LOCALHOST, 0)).unwrap();
        let port = listener.local_addr().unwrap().port();
        let folder = std::env::temp_dir().join(format!("yoake-control-{}", crate::guard::random_hex(6)));
        std::fs::create_dir_all(&folder).unwrap();
        let path = folder.join("control.json");
        std::fs::write(&path, format!("{{\"transport\":\"tcp\",\"port\":{port},\"token\":\"sekrit\"}}")).unwrap();
        let calls = Arc::new(AtomicU32::new(0));
        let stop = Arc::new(std::sync::atomic::AtomicBool::new(false));
        let (counted, stopped) = (Arc::clone(&calls), Arc::clone(&stop));
        listener.set_nonblocking(true).unwrap();
        let handle = std::thread::spawn(move || {
            while !stopped.load(Ordering::Relaxed) {
                let Ok((stream, _)) = listener.accept() else {
                    std::thread::sleep(Duration::from_millis(5));
                    continue;
                };
                stream.set_nonblocking(false).unwrap();
                let mut reader = BufReader::new(stream.try_clone().unwrap());
                let mut writer = stream;
                let mut auth = String::new();
                reader.read_line(&mut auth).unwrap();
                if auth.trim() != "auth sekrit" {
                    writeln!(writer, "{{\"error\":\"bad token\"}}").unwrap();
                    continue;
                }
                writeln!(writer, "{{\"ok\":true}}").unwrap();
                let mut line = String::new();
                reader.read_line(&mut line).unwrap();
                let seen = counted.fetch_add(1, Ordering::Relaxed) + 1;
                let block = if seen > appear { "minecraft:stone" } else { "minecraft:air" };
                if line.trim().starts_with("getblock") {
                    writeln!(writer, "{{\"block\":\"{block}\"}}").unwrap();
                } else {
                    writeln!(writer, "{{\"ok\":true,\"echo\":\"{}\"}}", line.trim()).unwrap();
                }
            }
        });
        (Control::new(path), calls, handle, stop)
    }

    #[test]
    fn commands_are_checked_before_they_are_sent() {
        for good in ["getblock 1 -60 2", "chat hello", "chat-log 20", "wait-for chat hi --timeout 5", "status"] {
            assert!(check_command(good).is_ok(), "{good}");
        }
        for bad in ["", "quit", "disconnect", "connect example.com", "save-all", "screenshot x.png", "chat a\nquit", "give diamond 64"] {
            assert!(check_command(bad).is_err(), "{bad:?}");
        }
        assert!(check_command(&format!("chat {}", "x".repeat(5000))).is_err());
        // Plain keys pass; the edit modifiers, the debug combo and the function keys do not.
        for good in ["key w down", "key space tap", "key shift down", "key e tap"] {
            assert!(check_command(good).is_ok(), "{good}");
        }
        for bad in ["key ctrl down", "key Control tap", "key super down", "key cmd down", "key alt down", "key f3 down", "key F3 tap", "key f11 tap", "key f1 tap"] {
            assert!(check_command(bad).unwrap_err().contains("not available to tests"), "{bad}");
        }
    }

    #[test]
    fn a_call_waits_as_long_as_the_command_does() {
        assert_eq!(Control::timeout_for("status"), Duration::from_secs(30));
        assert_eq!(Control::timeout_for("chat-log 5"), Duration::from_secs(30));
        assert_eq!(Control::timeout_for("wait-for chat hi --timeout 60"), Duration::from_secs(70));
        assert_eq!(Control::timeout_for("wait-for chat hi"), Duration::from_secs(40));
        assert_eq!(Control::timeout_for("wait-for loaded --timeout 5 --since 3"), Duration::from_secs(15));
    }

    #[test]
    fn a_wait_ends_when_it_said_it_would_even_if_the_client_hangs() {
        // A listener that accepts and never answers.
        let listener = TcpListener::bind((Ipv4Addr::LOCALHOST, 0)).unwrap();
        let folder = std::env::temp_dir().join(format!("yoake-control-{}", crate::guard::random_hex(6)));
        std::fs::create_dir_all(&folder).unwrap();
        let path = folder.join("control.json");
        std::fs::write(&path, format!("{{\"port\":{},\"token\":\"t\"}}", listener.local_addr().unwrap().port())).unwrap();
        let began = Instant::now();
        let error = Control::new(&path).wait("status", "x", Duration::from_millis(600)).unwrap_err();
        assert!(error.contains("did not show"), "{error}");
        assert!(began.elapsed() < Duration::from_secs(3), "{:?}", began.elapsed());
        drop(listener);
        let _ = std::fs::remove_dir_all(folder);
    }

    #[test]
    fn a_call_authenticates_sends_one_line_and_returns_the_reply() {
        let (control, calls, handle, stop) = fake_client(0);
        assert!(control.exists());
        assert_eq!(control.call("getblock 1 2 3", Duration::from_secs(5)).unwrap(), "{\"block\":\"minecraft:stone\"}");
        assert!(control.call("chat hi there", Duration::from_secs(5)).unwrap().contains("chat hi there"));
        assert!(control.call("quit", Duration::from_secs(5)).is_err());
        assert_eq!(calls.load(Ordering::Relaxed), 2, "a refused command is not sent");
        stop.store(true, Ordering::Relaxed);
        handle.join().unwrap();
        let _ = std::fs::remove_dir_all(control.path().parent().unwrap());
    }

    #[test]
    fn wait_repeats_until_the_reply_shows_what_was_expected() {
        let (control, calls, handle, stop) = fake_client(3);
        let reply = control.wait("getblock 1 2 3", "\"block\":\"minecraft:stone", Duration::from_secs(10)).unwrap();
        assert!(reply.contains("minecraft:stone"));
        assert_eq!(calls.load(Ordering::Relaxed), 4, "three air answers, then stone");
        // What never shows up is a timeout that names the last answer.
        let error = control.wait("getblock 1 2 3", "minecraft:diamond_block", Duration::from_millis(300)).unwrap_err();
        assert!(error.contains("did not show") && error.contains("minecraft:stone"), "{error}");
        stop.store(true, Ordering::Relaxed);
        handle.join().unwrap();
        let _ = std::fs::remove_dir_all(control.path().parent().unwrap());
    }

    #[test]
    fn a_missing_endpoint_or_a_dead_client_is_a_clear_error() {
        let missing = Control::new(std::env::temp_dir().join("yoake-no-such-control.json"));
        assert!(!missing.exists());
        assert!(missing.call("status", Duration::from_secs(1)).unwrap_err().contains("not there yet"));
        let (control, _calls, handle, stop) = fake_client(0);
        stop.store(true, Ordering::Relaxed);
        handle.join().unwrap();
        // The listener is gone: the connection is refused.
        assert!(control.call("status", Duration::from_secs(1)).unwrap_err().contains("does not answer"));
    }
}
