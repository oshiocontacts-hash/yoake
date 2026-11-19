//! The directories a mod may see.
//!
//! * `/data`: `<game dir>/mod-data/<mod id>/`, the mod's own folder (created when needed), read-write.
//! * `/game`: the instance game directory, read-write.
//! * `/host/<letter>` (lowercase drive letter; `/host/root` on Unix): every local drive root, read-write,
//!   only when the manifest says `[wasm] host_filesystem = true`.

use std::path::PathBuf;

use wasmtime_wasi::{FsPerms, WasiCtxBuilder};

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct FileAccess {
    pub game_dir: PathBuf,
    pub mod_id: String,
    pub host_filesystem: bool,
}

/// A mod id that may name a folder: ASCII letters, digits, `-`, `_` and `.`, not starting with a dot.
pub fn valid_mod_id(id: &str) -> bool {
    !id.is_empty()
        && id.len() <= 64
        && !id.starts_with('.')
        && id.bytes().all(|b| b.is_ascii_alphanumeric() || matches!(b, b'-' | b'_' | b'.'))
}

impl FileAccess {
    pub fn new(game_dir: impl Into<PathBuf>, mod_id: impl Into<String>, host_filesystem: bool) -> Self {
        Self { game_dir: game_dir.into(), mod_id: mod_id.into(), host_filesystem }
    }

    /// `<game dir>/mod-data/<id>/`, created.
    pub fn data_dir(&self) -> Result<PathBuf, String> {
        if !valid_mod_id(&self.mod_id) {
            return Err(format!("mod id {:?} cannot name a data folder", self.mod_id));
        }
        let dir = self.game_dir.join("mod-data").join(&self.mod_id);
        std::fs::create_dir_all(&dir).map_err(|error| format!("cannot create {}: {error}", dir.display()))?;
        Ok(dir)
    }

    /// Adds the preopens to a WASI context under construction (p1 and p2 share the builder).
    pub fn apply(&self, builder: &mut WasiCtxBuilder) -> Result<(), String> {
        let data = self.data_dir()?;
        builder.preopened_dir(&data, "/data", FsPerms::ReadWrite).map_err(|e| format!("/data: {e:#}"))?;
        if self.game_dir.is_dir() {
            builder.preopened_dir(&self.game_dir, "/game", FsPerms::ReadWrite).map_err(|e| format!("/game: {e:#}"))?;
        }
        if self.host_filesystem {
            for (name, root) in drive_roots() {
                if let Err(error) = builder.preopened_dir(&root, format!("/host/{name}"), FsPerms::ReadWrite) {
                    eprintln!("yoake: /host/{name} not available: {error:#}");
                }
            }
        }
        Ok(())
    }
}

/// The local drive roots as `(name, path)`.
pub fn drive_roots() -> Vec<(String, PathBuf)> {
    #[cfg(windows)]
    {
        use std::os::windows::ffi::OsStrExt;
        unsafe extern "system" {
            fn GetDriveTypeW(root: *const u16) -> u32;
        }
        const DRIVE_REMOVABLE: u32 = 2;
        const DRIVE_FIXED: u32 = 3;
        let mut roots = Vec::new();
        for letter in b'A'..=b'Z' {
            let path = format!("{}:\\", letter as char);
            let wide: Vec<u16> = std::ffi::OsStr::new(&path).encode_wide().chain(Some(0)).collect();
            // SAFETY: `wide` is a NUL-terminated UTF-16 string that outlives the call.
            let kind = unsafe { GetDriveTypeW(wide.as_ptr()) };
            if (kind == DRIVE_FIXED || kind == DRIVE_REMOVABLE) && std::path::Path::new(&path).is_dir() {
                roots.push(((letter as char).to_ascii_lowercase().to_string(), PathBuf::from(path)));
            }
        }
        roots
    }
    #[cfg(not(windows))]
    {
        vec![("root".to_owned(), PathBuf::from("/"))]
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn mod_ids_that_name_folders() {
        for ok in ["a", "my-mod_2", "a.b"] {
            assert!(valid_mod_id(ok), "{ok}");
        }
        for bad in ["", ".hidden", "a/b", "a\\b", "..", "a b", "ü"] {
            assert!(!valid_mod_id(bad), "{bad}");
        }
    }

    #[test]
    fn data_dir_is_created_under_mod_data() {
        let root = std::env::temp_dir().join(format!("yoake-mod-io-files-{}", std::process::id()));
        let access = FileAccess::new(&root, "demo", false);
        let dir = access.data_dir().unwrap();
        assert!(dir.is_dir());
        assert!(dir.ends_with("mod-data/demo") || dir.ends_with("mod-data\\demo"));
        assert!(FileAccess::new(&root, "../x", false).data_dir().is_err());
        let _ = std::fs::remove_dir_all(root);
    }
}
