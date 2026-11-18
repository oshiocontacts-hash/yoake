//! The pinned toolchain (`rust-toolchain.toml` at the workspace root). The rustdoc JSON the generator reads changes
//! with the compiler, so the generator and the regeneration gate refuse any other `rustc`.

use std::path::Path;
use std::process::Command;

/// The `channel` of `<root>/rust-toolchain.toml` (an exact version such as `1.100.0`).
pub fn pinned_channel(root: &Path) -> Result<String, String> {
    let path = root.join("rust-toolchain.toml");
    let text = std::fs::read_to_string(&path).map_err(|e| format!("cannot read {}: {e}", path.display()))?;
    parse_channel(&text).ok_or_else(|| format!("{} has no `channel = \"...\"` line", path.display()))
}

/// The value of the `channel = "..."` line of a `rust-toolchain.toml`.
pub fn parse_channel(text: &str) -> Option<String> {
    text.lines().find_map(|line| {
        let rest = line.trim().strip_prefix("channel")?.trim_start().strip_prefix('=')?.trim();
        Some(rest.trim_matches('"').to_string())
    })
}

/// `Ok` when `rustc_version` (the first line of `rustc -V`) is the pinned `channel`.
pub fn matches(channel: &str, rustc_version: &str) -> Result<(), String> {
    if !channel.chars().next().is_some_and(|c| c.is_ascii_digit()) {
        return Err(format!("rust-toolchain.toml pins `{channel}`: the generator needs an exact version"));
    }
    let line = rustc_version.trim();
    if line == format!("rustc {channel}") || line.starts_with(&format!("rustc {channel} ")) {
        Ok(())
    } else {
        Err(format!("the workspace pins rustc {channel} (rust-toolchain.toml), but `rustc -V` is `{line}`"))
    }
}

/// Runs `rustc -V` in `root` (so rustup picks the toolchain the way cargo will) and checks it against the pin.
pub fn check_rustc(root: &Path) -> Result<(), String> {
    let channel = pinned_channel(root)?;
    let output = Command::new("rustc").arg("-V").current_dir(root).output().map_err(|e| format!("cannot run rustc -V: {e}"))?;
    matches(&channel, &String::from_utf8_lossy(&output.stdout))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_channel_line_is_read_and_compared_exactly() {
        let text = "[toolchain]\nchannel = \"1.100.0\"\ncomponents = [\"rustfmt\"]\n";
        assert_eq!(parse_channel(text).as_deref(), Some("1.100.0"));
        assert!(matches("1.100.0", "rustc 1.100.0 (0ef1bc386 2026-11-09)\n").is_ok());
        assert!(matches("1.100.0", "rustc 1.100.0").is_ok());
        assert!(matches("1.100.0", "rustc 1.100.01 (x)").is_err());
        assert!(matches("1.100.0", "rustc 1.99.1 (x)").is_err());
        assert!(matches("stable", "rustc 1.100.0 (x)").is_err(), "a moving channel cannot be checked");
    }

    #[test]
    fn the_workspace_pins_an_exact_version() {
        let root = Path::new(env!("CARGO_MANIFEST_DIR")).join("../..");
        let channel = pinned_channel(&root).unwrap();
        assert!(channel.split('.').count() == 3 && channel.split('.').all(|p| p.parse::<u32>().is_ok()), "{channel}");
    }
}
