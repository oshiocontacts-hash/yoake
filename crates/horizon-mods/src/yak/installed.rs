use super::{MAX_ENTRIES, MAX_FILE_SIZE, MAX_TOTAL_SIZE, PublicKey, Signature};
use horizon_release_sign::{Sha256, yak as container};
use std::{collections::BTreeMap, fs::File, io::Read, path::Path};

pub use container::check_plain_path;

/// Yoake's signing keys, the same ones the launcher pins for its updates (`yoake_feed.rs`): set at
/// build time in `YOAKE_COMPILED_PUBLIC_KEYS` (`;`-separated), never in the source. Empty in a build
/// without them, so signed DLL mods are refused (unless the developer flag allows unsigned ones).
pub fn trusted_keys() -> Vec<PublicKey> {
    option_env!("YOAKE_COMPILED_PUBLIC_KEYS")
        .map(|keys| super::parse_keys(&keys.replace(';', "
")).unwrap_or_default())
        .unwrap_or_default()
}

pub fn check_installed_tree(root: &Path) -> Result<(), String> {
    fn visit(path: &Path, count: &mut u64, size: &mut u64, depth: usize) -> Result<(), String> {
        check_plain_path(path)?;
        if depth > 64 {
            return Err("installed YAK nesting limit exceeded".into());
        }
        let metadata = std::fs::symlink_metadata(path).map_err(|error| error.to_string())?;
        *count += 1;
        if *count > MAX_ENTRIES {
            return Err("installed YAK entry limit exceeded".into());
        }
        if metadata.is_dir() {
            for entry in std::fs::read_dir(path).map_err(|error| error.to_string())? {
                visit(&entry.map_err(|error| error.to_string())?.path(), count, size, depth + 1)?;
            }
        } else if metadata.is_file() {
            *size = size.checked_add(metadata.len()).ok_or("installed YAK size overflow")?;
            if metadata.len() > MAX_FILE_SIZE || *size > MAX_TOTAL_SIZE {
                return Err("installed YAK size limit exceeded".into());
            }
        } else {
            return Err("installed YAK contains a special file".into());
        }
        Ok(())
    }
    visit(root, &mut 0, &mut 0, 0)
}

pub fn hash_installed_file(root: &Path, name: &str) -> Result<String, String> {
    container::check_path(name, false)?;
    let path = root.join(name);
    check_plain_path(&path)?;
    let mut file = File::open(path).map_err(|error| error.to_string())?;
    let metadata = file.metadata().map_err(|error| error.to_string())?;
    if !metadata.is_file() || metadata.len() > MAX_FILE_SIZE {
        return Err("invalid installed YAK file size or type".into());
    }
    let mut hash = Sha256::default();
    let mut buffer = [0; 16384];
    let mut size = 0u64;
    loop {
        let length = file.read(&mut buffer).map_err(|error| error.to_string())?;
        if length == 0 {
            break;
        }
        size += length as u64;
        if size > MAX_FILE_SIZE {
            return Err("installed YAK file grew beyond its limit".into());
        }
        hash.update(&buffer[..length]);
    }
    Ok(hash.hex())
}

pub fn verify_installed(root: &Path, names: &[&str], keys: &[PublicKey]) -> Result<(), String> {
    let path = root.join("SIGNATURE");
    check_plain_path(&path)?;
    let file = File::open(path).map_err(|error| format!("SIGNATURE: {error}"))?;
    if !file.metadata().map_err(|error| error.to_string())?.is_file() {
        return Err("SIGNATURE is not a regular file".into());
    }
    let mut bytes = Vec::new();
    file.take(container::MAX_SIGNATURE_SIZE + 1).read_to_end(&mut bytes).map_err(|error| error.to_string())?;
    if bytes.len() as u64 > container::MAX_SIGNATURE_SIZE {
        return Err("SIGNATURE exceeds its size limit".into());
    }
    let signature: Signature = serde_json::from_slice(&bytes).map_err(|error| error.to_string())?;
    if signature.files.len() as u64 > MAX_ENTRIES {
        return Err("SIGNATURE exceeds its entry limit".into());
    }
    for (name, hash) in &signature.files {
        container::check_path(name, false)?;
        if hash.len() != 64 || !hash.bytes().all(|byte| byte.is_ascii_hexdigit() && !byte.is_ascii_uppercase()) {
            return Err("invalid SIGNATURE checksum".into());
        }
    }
    signature.verify(&signature.files, keys)?;
    let mut checked = BTreeMap::new();
    for name in std::iter::once("yak.toml").chain(names.iter().copied()) {
        checked.insert(name, hash_installed_file(root, name)?);
    }
    for (name, actual) in checked {
        if signature.files.get(name) != Some(&actual) {
            return Err(format!("SIGNATURE checksum mismatch: {name}"));
        }
    }
    Ok(())
}
