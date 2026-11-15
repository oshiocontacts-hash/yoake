//! File handling for the certificate tool: exclusive creation, cleanup, and
//! crash-safe replacement of the leaf pair.
use std::{
    fs::OpenOptions,
    io::{self, Write},
    path::{Path, PathBuf},
};

pub fn path_error(error: io::Error, path: &Path) -> io::Error {
    io::Error::new(error.kind(), format!("{}: {error}", path.display()))
}

pub fn ensure_absent(path: &Path) -> io::Result<()> {
    match std::fs::metadata(path) {
        Ok(_) => Err(io::Error::new(io::ErrorKind::AlreadyExists, format!("refusing to overwrite {}", path.display()))),
        Err(error) if error.kind() == io::ErrorKind::NotFound => Ok(()),
        Err(error) => Err(path_error(error, path)),
    }
}

pub fn remove_if_present(path: &Path) -> io::Result<()> {
    match std::fs::remove_file(path) {
        Ok(()) => Ok(()),
        Err(error) if error.kind() == io::ErrorKind::NotFound => Ok(()),
        Err(error) => Err(path_error(error, path)),
    }
}

fn write_new(path: &Path, bytes: &[u8], private: bool) -> io::Result<()> {
    let mut options = OpenOptions::new();
    options.write(true).create_new(true);
    #[cfg(not(unix))]
    let _ = private;
    #[cfg(unix)]
    if private {
        use std::os::unix::fs::OpenOptionsExt;
        options.mode(0o600);
    }
    let mut file = options.open(path).map_err(|error| path_error(error, path))?;
    file.write_all(bytes).and_then(|()| file.sync_all()).map_err(|error| path_error(error, path))
}

/// Create every file exclusively (`(path, bytes, private)`); if any step fails,
/// remove the files this call created, never a pre-existing one.
pub fn write_all_new(items: &[(PathBuf, &[u8], bool)]) -> io::Result<()> {
    for (index, (path, bytes, private)) in items.iter().enumerate() {
        if let Err(error) = write_new(path, bytes, *private) {
            let upto = if error.kind() == io::ErrorKind::AlreadyExists { index } else { index + 1 };
            for (created, _, _) in &items[..upto] { let _ = std::fs::remove_file(created); }
            return Err(error);
        }
    }
    Ok(())
}

/// Create the output directory; on Unix new directories are `0700`, not umask-dependent.
pub fn create_private_dir(dir: &Path) -> io::Result<()> {
    #[cfg(unix)]
    {
        use std::os::unix::fs::DirBuilderExt;
        std::fs::DirBuilder::new().recursive(true).mode(0o700).create(dir).map_err(|error| path_error(error, dir))
    }
    #[cfg(not(unix))]
    std::fs::create_dir_all(dir).map_err(|error| path_error(error, dir))
}

/// Unix: refuse a directory that group or others can write to.
pub fn check_dir_not_shared(dir: &Path) -> io::Result<()> {
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let mode = std::fs::metadata(dir).map_err(|error| path_error(error, dir))?.permissions().mode();
        if mode & 0o022 != 0 {
            return Err(io::Error::new(
                io::ErrorKind::PermissionDenied,
                format!("{} is writable by group or others (mode {:o}); run chmod 700 on it", dir.display(), mode & 0o777),
            ));
        }
    }
    #[cfg(not(unix))]
    let _ = dir;
    Ok(())
}

/// Unix: flush the directory entry changes to disk.
pub fn sync_dir(dir: &Path) -> io::Result<()> {
    #[cfg(unix)]
    return std::fs::File::open(dir).and_then(|file| file.sync_all()).map_err(|error| path_error(error, dir));
    #[cfg(not(unix))]
    {
        let _ = dir;
        Ok(())
    }
}

/// Keep the current file as `old` (a hard link, else a copy) without ever
/// removing `current`.
fn backup(current: &Path, old: &Path) -> io::Result<()> {
    remove_if_present(old)?;
    if std::fs::hard_link(current, old).is_err() {
        std::fs::copy(current, old).map_err(|error| path_error(error, old))?;
    }
    Ok(())
}

/// Replace each `(current, new, old)` with `new`, keeping the previous file as
/// `old`. `current` is only ever swapped by one atomic rename; if a later
/// rename fails, the earlier ones are put back from their backups.
pub fn replace_pairs(dir: &Path, pairs: &[(PathBuf, PathBuf, PathBuf)]) -> io::Result<()> {
    for (current, _, old) in pairs { backup(current, old)?; }
    sync_dir(dir)?;
    for (index, (current, new, _)) in pairs.iter().enumerate() {
        if let Err(error) = std::fs::rename(new, current) {
            for (current, _, old) in &pairs[..index] { let _ = std::fs::rename(old, current); }
            return Err(path_error(error, current));
        }
    }
    sync_dir(dir)
}
