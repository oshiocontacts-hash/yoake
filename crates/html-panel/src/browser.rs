//! 使えるブラウザ（Chrome / Edge / Chromium）を探して、DevTools の口を開かせる。

use std::fs;
use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};

use serde_json::{Value, json};

use crate::ws::{self, Message};

/// 使えるブラウザを探す。`HTML_PANEL_BROWSER` で指定もできる。見つからなければ None。
pub fn find() -> Option<PathBuf> {
    if let Ok(p) = std::env::var("HTML_PANEL_BROWSER") {
        let p = PathBuf::from(p);
        if p.is_file() {
            return Some(p);
        }
    }
    let mut candidates = Vec::new();
    #[cfg(windows)]
    for var in ["ProgramFiles", "ProgramFiles(x86)", "LOCALAPPDATA"] {
        if let Ok(base) = std::env::var(var) {
            let base = PathBuf::from(base);
            candidates.push(base.join("Microsoft/Edge/Application/msedge.exe"));
        }
    }
    // Edge を先に：Windows 10/11 に必ず入っている（Chrome は入っていない PC がある）
    #[cfg(windows)]
    for var in ["ProgramFiles", "ProgramFiles(x86)", "LOCALAPPDATA"] {
        if let Ok(base) = std::env::var(var) {
            let base = PathBuf::from(base);
            candidates.push(base.join("Google/Chrome/Application/chrome.exe"));
            candidates.push(base.join("Chromium/Application/chrome.exe"));
        }
    }
    #[cfg(target_os = "macos")]
    {
        candidates.push(PathBuf::from("/Applications/Google Chrome.app/Contents/MacOS/Google Chrome"));
        candidates.push(PathBuf::from("/Applications/Microsoft Edge.app/Contents/MacOS/Microsoft Edge"));
        candidates.push(PathBuf::from("/Applications/Chromium.app/Contents/MacOS/Chromium"));
    }
    #[cfg(all(unix, not(target_os = "macos")))]
    for p in ["/usr/bin/google-chrome", "/usr/bin/google-chrome-stable", "/usr/bin/chromium", "/usr/bin/chromium-browser", "/usr/bin/microsoft-edge"] {
        candidates.push(PathBuf::from(p));
    }
    candidates.into_iter().find(|p| p.is_file())
}

/// `profile/DevToolsActivePort` が書かれるまで待つ。(port, ブラウザ全体の口のパス)。
/// `alive` が false を返したら（ブラウザが落ちた・閉じられた）すぐやめる。
pub fn wait_port(profile: &Path, timeout: Duration, mut alive: impl FnMut() -> bool) -> Result<(u16, String), String> {
    let port_file = profile.join("DevToolsActivePort");
    let started = Instant::now();
    loop {
        if let Ok(text) = fs::read_to_string(&port_file) {
            let mut lines = text.lines();
            if let (Some(port), Some(path)) = (lines.next().and_then(|l| l.trim().parse::<u16>().ok()), lines.next()) {
                return Ok((port, path.trim().to_owned()));
            }
        }
        if !alive() {
            return Err(format!("the browser quit at once; is another copy using {}?", profile.display()));
        }
        if started.elapsed() > timeout {
            return Err(format!("the browser did not open its DevTools port in {} s", timeout.as_secs()));
        }
        std::thread::sleep(Duration::from_millis(30));
    }
}

/// ブラウザ全体の口に命令を 1 つ送って、答えを待つ。
pub fn call(port: u16, browser_path: &str, method: &str, params: Value) -> Result<Value, String> {
    let (mut reader, mut writer) = ws::connect(port, browser_path).map_err(|e| e.to_string())?;
    writer.text(&json!({ "id": 1, "method": method, "params": params }).to_string()).map_err(|e| e.to_string())?;
    let started = Instant::now();
    loop {
        if started.elapsed() > Duration::from_secs(15) {
            writer.close();
            return Err(format!("{method}: no answer"));
        }
        match reader.read().map_err(|e| e.to_string())? {
            Message::Text(t) => {
                let v: Value = serde_json::from_str(&t).map_err(|e| e.to_string())?;
                if v.get("id").and_then(Value::as_u64) == Some(1) {
                    writer.close();
                    if let Some(e) = v.get("error") {
                        return Err(format!("{method}: {e}"));
                    }
                    return Ok(v.get("result").cloned().unwrap_or(Value::Null));
                }
            }
            Message::Ping(p) => {
                let _ = writer.pong(&p);
            }
            Message::Close => return Err(format!("{method}: closed")),
        }
    }
}
