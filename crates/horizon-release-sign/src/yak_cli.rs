use horizon_release_sign::{
    PublicKey, SigningKey, YOAKE_BUILTIN_V1, YOAKE_MODS_V1, YOAKE_YAK_V1, verify_bytes, yak,
};
use std::fs::File;
use std::io::{Read, Write};
use std::path::{Path, PathBuf};

const MAX_CATALOG_SIZE: u64 = 16 * 1024 * 1024;

pub fn sign(tag: &str, path: &Path, key: &SigningKey) -> Result<(), String> {
    match tag {
        YOAKE_YAK_V1 => yak::sign_file(path, key),
        YOAKE_MODS_V1 | YOAKE_BUILTIN_V1 => {
            let signature_path = catalog_signature_path(path)?;
            let bytes = read_bounded(path, MAX_CATALOG_SIZE)?;
            let signature = key.sign(tag, &bytes);
            yak::atomic_write(&signature_path, false, |file| {
                writeln!(file, "{signature}").map_err(|error| error.to_string())
            })
        }
        _ => Err("unsupported YAK signing domain".into()),
    }
}

pub fn verify(tag: &str, path: &Path, keys: &[PublicKey]) -> Result<(), String> {
    match tag {
        YOAKE_YAK_V1 => {
            yak::check_plain_path(path)?;
            let archive = yak::Archive::new(File::open(path).map_err(|error| error.to_string())?)?;
            archive.verify_signature(keys)?.ok_or("YAK has no SIGNATURE")?;
            println!("{}: {} files, embedded signature verified", path.display(), archive.entries().len());
            Ok(())
        }
        YOAKE_MODS_V1 | YOAKE_BUILTIN_V1 => {
            let signature_path = catalog_signature_path(path)?;
            let bytes = read_bounded(path, MAX_CATALOG_SIZE)?;
            let signature = read_bounded(&signature_path, 4096)?;
            let signature = std::str::from_utf8(&signature).map_err(|_| "catalog signature is not UTF-8")?;
            verify_bytes(tag, &bytes, signature, keys)?;
            println!("{}: detached catalog signature verified", path.display());
            Ok(())
        }
        _ => Err("unsupported YAK verification domain".into()),
    }
}

fn catalog_signature_path(path: &Path) -> Result<PathBuf, String> {
    if path.file_name().and_then(|name| name.to_str()) != Some("index.json") {
        return Err("a catalog tag expects an index.json file".into());
    }
    Ok(path.with_file_name("index.json.sig"))
}

fn read_bounded(path: &Path, limit: u64) -> Result<Vec<u8>, String> {
    yak::check_plain_path(path)?;
    let file = File::open(path).map_err(|error| format!("{}: {error}", path.display()))?;
    let metadata = file.metadata().map_err(|error| error.to_string())?;
    if !metadata.is_file() || metadata.len() > limit {
        return Err("signing input is not a bounded regular file".into());
    }
    let mut bytes = Vec::new();
    file.take(limit + 1).read_to_end(&mut bytes).map_err(|error| error.to_string())?;
    if bytes.len() as u64 > limit {
        return Err("signing input grew beyond its size limit".into());
    }
    Ok(bytes)
}
