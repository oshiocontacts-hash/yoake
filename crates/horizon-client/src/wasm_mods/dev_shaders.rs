use super::{Watch, emit};
use crate::gfx::mod_gpu_shader::{MAX_SHADER_BYTES, ValidationError, validate};
use std::{
    collections::BTreeMap,
    io::Read,
    path::{Component, Path, PathBuf},
    time::SystemTime,
};

const MAX_FILES: usize = 64;

struct Entry {
    source: String,
    seen: Option<(SystemTime, u64)>,
    applied: Option<(SystemTime, u64)>,
}

pub(super) struct DevShaders {
    root: PathBuf,
    entries: BTreeMap<PathBuf, Entry>,
}

fn invalid(message: impl Into<String>) -> ValidationError {
    ValidationError {
        message: message.into(),
        line: 0,
        column: 0,
    }
}

fn relative_path(path: &str) -> Result<PathBuf, ValidationError> {
    let path = Path::new(path);
    if path.as_os_str().is_empty()
        || path.as_os_str().len() > 512
        || path.as_os_str().to_string_lossy().contains(':')
        || path.extension().is_none_or(|extension| extension != "wgsl")
        || path
            .components()
            .any(|component| !matches!(component, Component::Normal(_)))
    {
        return Err(invalid(
            "shader path must be a relative .wgsl path without traversal",
        ));
    }
    Ok(path.to_owned())
}

impl DevShaders {
    pub(super) fn new(component: &Path, id: &str) -> Option<Self> {
        let parent = component.parent()?.canonicalize().ok()?;
        Some(Self {
            root: parent.join(id),
            entries: BTreeMap::new(),
        })
    }

    fn read(&self, path: &Path) -> Result<Option<String>, ValidationError> {
        let root = match self.root.canonicalize() {
            Ok(root) => root,
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(None),
            Err(error) => return Err(invalid(error.to_string())),
        };
        if root != self.root {
            return Err(invalid(
                "shader directory must not redirect outside the development root",
            ));
        }
        let file = match self.root.join(path).canonicalize() {
            Ok(file) => file,
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(None),
            Err(error) => return Err(invalid(error.to_string())),
        };
        if !file.starts_with(&root) {
            return Err(invalid("shader file escapes its development directory"));
        }
        let mut bytes = Vec::new();
        std::fs::File::open(file)
            .map_err(|error| invalid(error.to_string()))?
            .take(MAX_SHADER_BYTES as u64 + 1)
            .read_to_end(&mut bytes)
            .map_err(|error| invalid(error.to_string()))?;
        if bytes.len() > MAX_SHADER_BYTES {
            return Err(invalid("WGSL exceeds 256 KiB"));
        }
        let source = String::from_utf8(bytes).map_err(|error| invalid(error.to_string()))?;
        validate(&source)?;
        Ok(Some(source))
    }

    pub(super) fn source(&mut self, path: &str, fallback: &str) -> Result<String, ValidationError> {
        let path = relative_path(path)?;
        if let Some(entry) = self.entries.get(&path) {
            return Ok(entry.source.clone());
        }
        if self.entries.len() >= MAX_FILES {
            return Err(invalid(
                "at most 64 development shader files may be watched",
            ));
        }
        let source = match self.read(&path)? {
            Some(source) => source,
            None => {
                validate(fallback)?;
                fallback.to_owned()
            }
        };
        let stamp = Watch::stat(&self.root.join(&path));
        self.entries.insert(
            path,
            Entry {
                source: source.clone(),
                seen: stamp,
                applied: stamp,
            },
        );
        Ok(source)
    }

    pub(super) fn poll(&mut self) -> bool {
        let mut changed = false;
        let paths: Vec<_> = self.entries.keys().cloned().collect();
        for path in paths {
            let stamp = Watch::stat(&self.root.join(&path));
            let entry = self.entries.get_mut(&path).unwrap();
            if stamp != entry.seen {
                entry.seen = stamp;
                continue;
            }
            if stamp == entry.applied {
                continue;
            }
            entry.applied = stamp;
            match self.read(&path) {
                Ok(Some(source)) => {
                    let entry = self.entries.get_mut(&path).unwrap();
                    if source != entry.source {
                        entry.source = source;
                        changed = true;
                    }
                }
                Ok(None) => {}
                Err(error) => emit(format!(
                    "wasm: {}:{}:{}: {}; keeping the previous shader",
                    self.root.join(path).display(),
                    error.line,
                    error.column,
                    error.message
                )),
            }
        }
        changed
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const SHADER: &str = "@compute @workgroup_size(1) fn main() {}";

    #[test]
    fn paths_are_confined_and_only_valid_stable_edits_reload() {
        let scratch = std::env::temp_dir().join(format!(
            "yoake-dev-shaders-{}-{}",
            std::process::id(),
            SystemTime::now()
                .duration_since(SystemTime::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        std::fs::create_dir(&scratch).unwrap();
        let component = scratch.join("demo.wasm");
        let mut shaders = DevShaders::new(&component, "demo").unwrap();
        for path in [
            "",
            "../other.wgsl",
            "C:/other.wgsl",
            "/other.wgsl",
            "a.txt",
            "a.wgsl:other.wgsl",
        ] {
            assert!(shaders.source(path, SHADER).is_err(), "{path}");
        }
        assert_eq!(shaders.source("effect.wgsl", SHADER).unwrap(), SHADER);
        std::fs::create_dir(&shaders.root).unwrap();
        let path = shaders.root.join("effect.wgsl");
        let edited = "@compute @workgroup_size(2) fn main() {}";
        std::fs::write(&path, edited).unwrap();
        assert!(!shaders.poll());
        assert!(shaders.poll());
        assert_eq!(shaders.source("effect.wgsl", SHADER).unwrap(), edited);
        std::fs::write(&path, "not valid WGSL").unwrap();
        assert!(!shaders.poll());
        assert!(!shaders.poll());
        assert_eq!(shaders.source("effect.wgsl", SHADER).unwrap(), edited);
        std::fs::remove_file(path).unwrap();
        assert!(!shaders.poll());
        assert!(!shaders.poll());
        assert_eq!(shaders.source("effect.wgsl", SHADER).unwrap(), edited);
        for index in 1..MAX_FILES {
            shaders
                .source(&format!("shader{index}.wgsl"), SHADER)
                .unwrap();
        }
        assert!(shaders.source("extra.wgsl", SHADER).is_err());
        let reload = std::sync::Arc::new(super::super::Reload {
            shaders: Some(std::sync::Arc::new(std::sync::Mutex::new(shaders))),
            ..super::super::Reload::default()
        });
        std::fs::write(&component, b"component").unwrap();
        let mut watch = Watch::new(component.clone(), reload.clone());
        std::fs::write(scratch.join("demo/effect.wgsl"), SHADER).unwrap();
        watch.poll();
        watch.seen = None;
        watch.poll();
        assert!(!reload.ready.load(std::sync::atomic::Ordering::Acquire));
        watch.poll();
        assert!(reload.ready.load(std::sync::atomic::Ordering::Acquire));
        assert_eq!(
            reload.bytes.lock().unwrap().as_deref(),
            Some(b"component".as_slice())
        );
        std::fs::remove_file(scratch.join("demo/effect.wgsl")).unwrap();
        std::fs::remove_dir(scratch.join("demo")).unwrap();
        std::fs::remove_file(component).unwrap();
        std::fs::remove_dir(scratch).unwrap();
    }
}
