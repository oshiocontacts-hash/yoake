use super::{Archive, Manifest, Summary, Target};
use horizon_release_sign::yak::{MAX_ENTRIES, MAX_FILE_SIZE, MAX_TOTAL_SIZE, atomic_write, check_path, check_plain_path, is_link};
use std::collections::BTreeMap;
use std::fs::File;
use std::io::{Read, Seek, SeekFrom, Write};
use std::path::{Path, PathBuf};
use zip::{CompressionMethod, ZipWriter, write::SimpleFileOptions};

/// Builds an unsigned archive with stable file order and stored compression.
/// Binary overrides are files (renamed to the declared library name) or whole target directories.
/// The caller must exclusively control source paths and the output parent during packing.
pub fn pack_directory(root: &Path, output: &Path, binaries: &BTreeMap<Target, PathBuf>) -> Result<Summary, String> {
    check_plain_path(root)?;
    let manifest_path = root.join("yak.toml");
    let manifest = Manifest::read(&manifest_path)?;
    let mut sources = BTreeMap::new();
    collect(&manifest_path, "yak.toml", &mut sources)?;
    for name in ["content", "assets", "shaders", "config", "LICENSE", "README.md"] {
        let path = root.join(name);
        if std::fs::symlink_metadata(&path).is_ok() {
            collect(&path, name, &mut sources)?;
        } else if path.try_exists().map_err(|error| error.to_string())? {
            return Err(format!("cannot inspect {}", path.display()));
        }
    }
    if let Some(bin) = &manifest.bin {
        for target in binaries.keys() {
            if !bin.targets.contains(target) {
                return Err(format!("--bin target {target} is not declared in yak.toml"));
            }
        }
        for target in &bin.targets {
            let base = format!("bin/{target}");
            let path = binaries.get(target).cloned().unwrap_or_else(|| root.join(&base));
            check_plain_path(&path)?;
            if path.is_file() {
                collect(&path, &format!("{base}/{}", target.library_filename(&bin.library)), &mut sources)?;
            } else {
                collect(&path, &base, &mut sources)?;
            }
        }
    } else if !binaries.is_empty() {
        return Err("--bin requires [bin] in yak.toml".into());
    }
    let total: u64 = sources.values().map(|(_, size)| size).sum();
    if total > MAX_TOTAL_SIZE {
        return Err("YAK uncompressed total exceeds 1 GiB".into());
    }
    let mut summary = None;
    atomic_write(output, false, |file| {
        let mut zip = ZipWriter::new(&mut *file);
        let options = SimpleFileOptions::default().compression_method(CompressionMethod::Stored).unix_permissions(0o644);
        let mut buffer = [0u8; 64 * 1024];
        for (name, (path, expected)) in &sources {
            check_plain_path(path)?;
            let mut source = File::open(path).map_err(|error| error.to_string())?;
            if !source.metadata().map_err(|error| error.to_string())?.is_file() {
                return Err(format!("{} is not a regular file", path.display()));
            }
            zip.start_file(name, options).map_err(|error| error.to_string())?;
            let mut size = 0u64;
            loop {
                let length = source.read(&mut buffer).map_err(|error| error.to_string())?;
                if length == 0 {
                    break;
                }
                size += length as u64;
                if size > *expected {
                    return Err(format!("{} grew while packing", path.display()));
                }
                zip.write_all(&buffer[..length]).map_err(|error| error.to_string())?;
            }
            if size != *expected {
                return Err(format!("{} changed size while packing", path.display()));
            }
        }
        zip.finish().map_err(|error| error.to_string())?;
        file.seek(SeekFrom::Start(0)).map_err(|error| error.to_string())?;
        let archive = Archive::new(&mut *file)?;
        let actual = serde_json::to_value(archive.manifest()).map_err(|error| error.to_string())?;
        let expected = serde_json::to_value(&manifest).map_err(|error| error.to_string())?;
        if actual != expected {
            return Err("yak.toml changed while packing".into());
        }
        summary = Some(archive.summary());
        Ok(())
    })?;
    summary.ok_or_else(|| "packing produced no summary".into())
}

fn collect(path: &Path, name: &str, sources: &mut BTreeMap<String, (PathBuf, u64)>) -> Result<(), String> {
    check_plain_path(path)?;
    let metadata = std::fs::symlink_metadata(path).map_err(|error| format!("{}: {error}", path.display()))?;
    if is_link(&metadata) {
        return Err(format!("{}: symlinks and reparse points are not allowed", path.display()));
    }
    check_path(name, metadata.is_dir())?;
    if metadata.is_dir() {
        for child in std::fs::read_dir(path).map_err(|error| error.to_string())? {
            let child = child.map_err(|error| error.to_string())?;
            let child_name = child.file_name().into_string().map_err(|_| "YAK source names must be UTF-8")?;
            collect(&child.path(), &format!("{name}/{child_name}"), sources)?;
        }
    } else if metadata.is_file() {
        if metadata.len() > MAX_FILE_SIZE || sources.len() as u64 >= MAX_ENTRIES {
            return Err(format!("{name}: YAK size or entry limit exceeded"));
        }
        if sources.insert(name.to_owned(), (path.to_path_buf(), metadata.len())).is_some() {
            return Err(format!("duplicate YAK source: {name}"));
        }
    } else {
        return Err(format!("{}: not a regular file or directory", path.display()));
    }
    Ok(())
}
