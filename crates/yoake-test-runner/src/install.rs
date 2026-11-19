//! Installs the packages of a test run into a worker's game folder, in the layout the game reads at start
//! (`yoake-mods.txt` and `yoake-mods/<id>/<version>/`), so the mod under test is loaded by the game's own
//! loader and nothing else in the game knows it is being tested.
use std::fs::{self, File};
use std::path::{Path, PathBuf};

use horizon_mods::yak::{Archive, Manifest, Target, check_plain_path, trusted_keys};

/// A package placed in a game folder.
#[derive(Clone, Debug)]
pub struct Installed {
    pub id: String,
    pub version: String,
    pub manifest: Manifest,
}

/// What a project folder or a `.yak` is, without installing it.
pub fn describe(source: &Path) -> Result<Installed, String> {
    let manifest = if source.is_dir() {
        Manifest::read(&source.join("yak.toml"))?
    } else {
        archive(source)?.manifest().clone()
    };
    Ok(Installed { id: manifest.id.clone(), version: manifest.version.to_string(), manifest })
}

fn archive(path: &Path) -> Result<Archive<File>, String> {
    check_plain_path(path)?;
    let file = File::open(path).map_err(|error| format!("{}: {error}", path.display()))?;
    Archive::new(file).map_err(|error| format!("{}: {error}", path.display()))
}

/// Parts of a project folder that a package carries (the same list `yoake pack` uses), besides its
/// declared Wasm modules.
const PARTS: [&str; 6] = ["content", "assets", "shaders", "config", "LICENSE", "README.md"];

/// Installs the project folder or `.yak` at `source` under `game_dir`. A project folder is copied as it
/// stands (`yoake build` has put its modules where `yak.toml` says); a `.yak` is extracted, and a signed
/// one must be signed by a key this build trusts.
pub fn install(source: &Path, game_dir: &Path) -> Result<Installed, String> {
    let installed = describe(source)?;
    if installed.manifest.bin.is_some() {
        return Err(format!("{}: native (DLL) packages are not run by the test runner", installed.id));
    }
    let root = game_dir.join("yoake-mods").join(&installed.id).join(&installed.version);
    if root.exists() {
        return Err(format!("{} {} is installed twice", installed.id, installed.version));
    }
    if source.is_dir() {
        fs::create_dir_all(&root).map_err(|error| format!("{}: {error}", root.display()))?;
        copy(&source.join("yak.toml"), &root.join("yak.toml"))?;
        for part in PARTS {
            let from = source.join(part);
            if fs::symlink_metadata(&from).is_ok() {
                copy(&from, &root.join(part))?;
            }
        }
        if let Some(wasm) = &installed.manifest.wasm {
            for (side, path) in wasm.declared_files() {
                let from = source.join(path);
                if !from.is_file() {
                    return Err(format!("the {side} module {} is missing: run yoake build first", from.display()));
                }
                copy(&from, &root.join(path))?;
            }
        }
    } else {
        if let Some(parent) = root.parent() {
            fs::create_dir_all(parent).map_err(|error| format!("{}: {error}", parent.display()))?;
        }
        let target = Target::host()?;
        archive(source)?.extract_target(&root, target, &trusted_keys())?;
    }
    Ok(installed)
}

fn copy(from: &Path, to: &Path) -> Result<(), String> {
    check_plain_path(from)?;
    let metadata = fs::symlink_metadata(from).map_err(|error| format!("{}: {error}", from.display()))?;
    if metadata.is_dir() {
        fs::create_dir_all(to).map_err(|error| format!("{}: {error}", to.display()))?;
        for entry in fs::read_dir(from).map_err(|error| format!("{}: {error}", from.display()))? {
            let entry = entry.map_err(|error| format!("{}: {error}", from.display()))?;
            copy(&entry.path(), &to.join(entry.file_name()))?;
        }
    } else if metadata.is_file() {
        if let Some(parent) = to.parent() {
            fs::create_dir_all(parent).map_err(|error| format!("{}: {error}", parent.display()))?;
        }
        fs::copy(from, to).map_err(|error| format!("{}: {error}", from.display()))?;
    } else {
        return Err(format!("{} is not a regular file or folder", from.display()));
    }
    Ok(())
}

/// Writes `yoake-mods.txt` for `installed`, one `<id> <version>` line each.
pub fn write_list(game_dir: &Path, installed: &[Installed]) -> Result<(), String> {
    let text: String = installed.iter().map(|package| format!("{} {}\n", package.id, package.version)).collect();
    let path = game_dir.join("yoake-mods.txt");
    fs::write(&path, text).map_err(|error| format!("{}: {error}", path.display()))
}

/// Where a project's test module is by convention: `bin/wasm/<id>-tests.wasm`.
pub fn default_module(project: &Path, id: &str) -> PathBuf {
    project.join("bin").join("wasm").join(format!("{id}-tests.wasm"))
}

#[cfg(test)]
pub(crate) mod tests {
    use super::*;
    use std::sync::atomic::{AtomicU64, Ordering};

    static NEXT: AtomicU64 = AtomicU64::new(0);

    pub(crate) fn scratch(label: &str) -> PathBuf {
        let path = std::env::temp_dir().join(format!("yoake-test-runner-{label}-{}-{}", std::process::id(), NEXT.fetch_add(1, Ordering::Relaxed)));
        let _ = fs::remove_dir_all(&path);
        fs::create_dir_all(&path).unwrap();
        path
    }

    pub(crate) fn project(root: &Path, id: &str, server: bool) {
        let side = if server { "client = false\nserver = true" } else { "client = true\nserver = false" };
        let wasm = if server { "server = \"bin/wasm/s.wasm\"" } else { "client = \"bin/wasm/c.wasm\"" };
        fs::write(
            root.join("yak.toml"),
            format!(
                "format = 1\nid = \"{id}\"\nversion = \"1.2.3\"\nname = \"Demo\"\nauthors = [\"T\"]\ndescription = \"d\"\n\n[yoake]\nversion = \"*\"\n\n[side]\n{side}\nrequired_on_join = false\n\n[wasm]\ninterface = \"1.0\"\n{wasm}\n"
            ),
        )
        .unwrap();
        fs::create_dir_all(root.join("bin/wasm")).unwrap();
        fs::write(root.join("bin/wasm").join(if server { "s.wasm" } else { "c.wasm" }), b"\0asm").unwrap();
        fs::write(root.join(format!("bin/wasm/{id}-tests.wasm")), b"tests").unwrap();
        fs::create_dir_all(root.join("content/data")).unwrap();
        fs::write(root.join("content/data/x.json"), b"{}").unwrap();
        fs::create_dir_all(root.join("src")).unwrap();
        fs::write(root.join("src/lib.rs"), b"// not packaged").unwrap();
    }

    #[test]
    fn a_project_is_copied_into_the_games_own_layout_without_its_tests_or_sources() {
        let (project_dir, game) = (scratch("project"), scratch("game"));
        project(&project_dir, "demo", false);
        let installed = install(&project_dir, &game).unwrap();
        assert_eq!((installed.id.as_str(), installed.version.as_str()), ("demo", "1.2.3"));
        let root = game.join("yoake-mods/demo/1.2.3");
        assert!(root.join("yak.toml").is_file());
        assert!(root.join("bin/wasm/c.wasm").is_file());
        assert!(root.join("content/data/x.json").is_file());
        assert!(!root.join("bin/wasm/demo-tests.wasm").exists(), "the tests are never installed in the game");
        assert!(!root.join("src").exists());
        write_list(&game, &[installed]).unwrap();
        assert_eq!(fs::read_to_string(game.join("yoake-mods.txt")).unwrap(), "demo 1.2.3\n");
        // The game's own checks accept what was written.
        assert!(horizon_mods::yak::check_installed_tree(&root).is_ok());
        assert!(Manifest::read(&root.join("yak.toml")).is_ok());
        let _ = (fs::remove_dir_all(project_dir), fs::remove_dir_all(game));
    }

    #[test]
    fn a_project_that_was_not_built_says_so() {
        let (project_dir, game) = (scratch("unbuilt"), scratch("unbuilt-game"));
        project(&project_dir, "demo", true);
        fs::remove_file(project_dir.join("bin/wasm/s.wasm")).unwrap();
        let error = install(&project_dir, &game).unwrap_err();
        assert!(error.contains("server module") && error.contains("yoake build"), "{error}");
        let _ = (fs::remove_dir_all(project_dir), fs::remove_dir_all(game));
    }

    #[test]
    fn a_package_cannot_be_installed_twice() {
        let (project_dir, game) = (scratch("twice"), scratch("twice-game"));
        project(&project_dir, "demo", false);
        install(&project_dir, &game).unwrap();
        assert!(install(&project_dir, &game).unwrap_err().contains("twice"));
        let _ = (fs::remove_dir_all(project_dir), fs::remove_dir_all(game));
    }

    #[test]
    fn a_packed_yak_is_extracted_and_a_missing_one_is_an_error() {
        let (project_dir, game) = (scratch("packed"), scratch("packed-game"));
        project(&project_dir, "demo", false);
        let yak = project_dir.join("demo.yak");
        horizon_mods::yak::pack_directory(&project_dir, &yak, &Default::default()).unwrap();
        let described = describe(&yak).unwrap();
        assert_eq!(described.id, "demo");
        let installed = install(&yak, &game).unwrap();
        assert_eq!(installed.version, "1.2.3");
        assert!(game.join("yoake-mods/demo/1.2.3/bin/wasm/c.wasm").is_file());
        assert!(describe(&project_dir.join("missing.yak")).is_err());
        let _ = (fs::remove_dir_all(project_dir), fs::remove_dir_all(game));
    }

    #[test]
    fn the_default_test_module_sits_next_to_the_mods_modules() {
        assert_eq!(default_module(Path::new("p"), "demo"), Path::new("p").join("bin").join("wasm").join("demo-tests.wasm"));
    }
}
