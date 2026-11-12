//! Bounded ZIP and signature primitives shared by the YAK reader and signing tool.

use crate::{PublicKey, Sha256, SigningKey, YOAKE_YAK_V1, parse_keys, sums_file, verify_bytes};
use base64::{Engine, engine::general_purpose::STANDARD};
use ring::rand::{SecureRandom, SystemRandom};
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;
use std::fs::{File, OpenOptions};
use std::io::{Read, Seek, SeekFrom, Write};
use std::path::{Component, Path, PathBuf};
use zip::{CompressionMethod, ZipArchive, ZipWriter, write::SimpleFileOptions};

pub const MANIFEST: &str = "yak.toml";
pub const SIGNATURE: &str = "SIGNATURE";
pub const MAX_FILE_SIZE: u64 = 256 * 1024 * 1024;
pub const MAX_TOTAL_SIZE: u64 = 1024 * 1024 * 1024;
pub const MAX_ENTRIES: u64 = 16_384;
pub const MAX_METADATA_SIZE: u64 = 32 * 1024 * 1024;
pub const MAX_ARCHIVE_SIZE: u64 = MAX_TOTAL_SIZE + MAX_METADATA_SIZE;
pub const MAX_COMPRESSION_RATIO: u64 = 200;
pub const MAX_MANIFEST_SIZE: u64 = 1024 * 1024;
pub const MAX_SIGNATURE_SIZE: u64 = 2 * 1024 * 1024;

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct Signature {
    pub signer: String,
    pub signature: String,
    pub files: BTreeMap<String, String>,
}

impl Signature {
    pub fn sign(files: BTreeMap<String, String>, key: &SigningKey) -> Self {
        let signature = key.sign(YOAKE_YAK_V1, canonical_hashes(&files).as_bytes());
        Self { signer: key.public_base64(), signature, files }
    }

    pub fn verify(&self, files: &BTreeMap<String, String>, keys: &[PublicKey]) -> Result<PublicKey, String> {
        self.check_hashes(files)?;
        let signer = self.signer_key()?;
        if !keys.contains(&signer) {
            return Err("SIGNATURE signer is not a caller-pinned Yoake key".into());
        }
        verify_bytes(YOAKE_YAK_V1, canonical_hashes(files).as_bytes(), &self.signature, &[signer])?;
        Ok(signer)
    }

    fn signer_key(&self) -> Result<PublicKey, String> {
        let keys = parse_keys(&self.signer)?;
        if keys.len() != 1 || self.signer.lines().count() != 1 {
            return Err("SIGNATURE signer must be one base64 Ed25519 public key".into());
        }
        Ok(keys[0])
    }

    fn check_hashes(&self, files: &BTreeMap<String, String>) -> Result<(), String> {
        self.signer_key()?;
        if self.signature.lines().count() != 1 || self.signature.trim().is_empty() {
            return Err("SIGNATURE must contain one signature".into());
        }
        if STANDARD.decode(self.signature.trim()).map_err(|_| "SIGNATURE is not base64")?.len() != 64 {
            return Err("SIGNATURE is not a 64-byte Ed25519 signature".into());
        }
        if &self.files != files {
            return Err("SIGNATURE file list or SHA-256 does not match the archive".into());
        }
        Ok(())
    }
}

pub fn canonical_hashes(files: &BTreeMap<String, String>) -> String {
    sums_file(&files.iter().map(|(name, hash)| (name.clone(), hash.clone())).collect::<Vec<_>>())
}

#[derive(Clone, Debug)]
pub struct Entry {
    pub name: String,
    pub size: u64,
    pub compressed_size: u64,
    pub sha256: String,
    index: usize,
}

/// Construction reads every regular file, checking CRC, sizes, paths and hashes.
pub struct Archive<Reader: Read + Seek> {
    zip: ZipArchive<Reader>,
    entries: Vec<Entry>,
    hashes: BTreeMap<String, String>,
    signature: Option<Signature>,
}

impl<Reader: Read + Seek> Archive<Reader> {
    pub fn new(mut reader: Reader) -> Result<Self, String> {
        preflight(&mut reader)?;
        let mut zip = ZipArchive::new(reader).map_err(|error| error.to_string())?;
        if zip.len() as u64 > MAX_ENTRIES {
            return Err("too many YAK ZIP entries".into());
        }
        let mut entries = Vec::new();
        let mut names = BTreeMap::new();
        let mut total = 0u64;
        for index in 0..zip.len() {
            let file = zip.by_index(index).map_err(|error| error.to_string())?;
            let name = file.name().map_err(|error| error.to_string())?.into_owned();
            if name.as_bytes() != file.name_raw() {
                return Err("YAK ZIP names must be UTF-8".into());
            }
            let directory = file.is_dir();
            check_path(&name, directory)?;
            let kind = file.unix_mode().unwrap_or(0) & 0o170000;
            if kind != 0 && kind != if directory { 0o040000 } else { 0o100000 } {
                return Err(format!("{name}: symlinks and special files are not allowed"));
            }
            let key = name.trim_end_matches('/').to_uppercase();
            if names.insert(key.clone(), directory).is_some() {
                return Err(format!("{name}: duplicate or case-colliding ZIP name"));
            }
            // A file may not also be a folder ("a" and "a/b"), in either order.
            let parent_is_file = key.match_indices('/').any(|(at, _)| names.get(&key[..at]) == Some(&false));
            let child_exists = !directory
                && names.range(format!("{key}/")..).next().is_some_and(|(other, _)| other.starts_with(&format!("{key}/")));
            if parent_is_file || child_exists {
                return Err(format!("{name}: a file and a folder share a path"));
            }
            if !matches!(file.compression(), CompressionMethod::Stored | CompressionMethod::Deflated) {
                return Err(format!("{name}: only stored or deflate compression is supported"));
            }
            if directory {
                if file.size() != 0 {
                    return Err(format!("{name}: directory has data"));
                }
                continue;
            }
            let limit = match name.as_str() {
                MANIFEST => MAX_MANIFEST_SIZE,
                SIGNATURE => MAX_SIGNATURE_SIZE,
                _ => MAX_FILE_SIZE,
            };
            if file.size() > limit {
                return Err(format!("{name}: file exceeds its size limit"));
            }
            if file.size() > file.compressed_size().saturating_mul(MAX_COMPRESSION_RATIO) {
                return Err(format!("{name}: compression ratio exceeds {MAX_COMPRESSION_RATIO}:1"));
            }
            total = total.checked_add(file.size()).ok_or("YAK size overflow")?;
            if total > MAX_TOTAL_SIZE {
                return Err("YAK uncompressed total exceeds 1 GiB".into());
            }
            let size = file.size();
            let compressed_size = file.compressed_size();
            entries.push(Entry { name, size, compressed_size, sha256: String::new(), index });
        }
        for name in names.keys() {
            let mut parent = name.as_str();
            while let Some((prefix, _)) = parent.rsplit_once('/') {
                if names.get(prefix) == Some(&false) {
                    return Err(format!("{name}: a parent path is a file"));
                }
                parent = prefix;
            }
        }
        for entry in &mut entries {
            let mut file = zip.by_index(entry.index).map_err(|error| error.to_string())?;
            entry.sha256 = copy_hashed(&mut file, &mut std::io::sink(), entry.size)?;
        }
        entries.sort_by(|left, right| left.name.cmp(&right.name));
        let hashes = entries.iter().filter(|entry| entry.name != SIGNATURE)
            .map(|entry| (entry.name.clone(), entry.sha256.clone())).collect();
        let mut archive = Self { zip, entries, hashes, signature: None };
        if !archive.hashes.contains_key(MANIFEST) {
            return Err("YAK is missing yak.toml".into());
        }
        if archive.entries.iter().any(|entry| entry.name == SIGNATURE) {
            let bytes = archive.read_small(SIGNATURE, MAX_SIGNATURE_SIZE)?;
            let text = std::str::from_utf8(&bytes).map_err(|_| "SIGNATURE is not UTF-8")?;
            let signature: Signature = toml::from_str(text).map_err(|error| format!("SIGNATURE: {error}"))?;
            signature.check_hashes(&archive.hashes)?;
            archive.signature = Some(signature);
        }
        Ok(archive)
    }

    pub fn entries(&self) -> &[Entry] {
        &self.entries
    }

    pub fn hashes(&self) -> &BTreeMap<String, String> {
        &self.hashes
    }

    pub fn signature(&self) -> Option<&Signature> {
        self.signature.as_ref()
    }

    /// None means unsigned, never implicitly trusted. No built-in keys are consulted.
    pub fn verify_signature(&self, keys: &[PublicKey]) -> Result<Option<PublicKey>, String> {
        self.signature.as_ref().map(|signature| signature.verify(&self.hashes, keys)).transpose()
    }

    pub fn read_small(&mut self, name: &str, limit: u64) -> Result<Vec<u8>, String> {
        let entry = self.entries.iter().find(|entry| entry.name == name).ok_or("YAK file not found")?;
        if entry.size > limit {
            return Err(format!("{name}: read limit exceeded"));
        }
        let mut bytes = Vec::new();
        self.copy_file(name, &mut bytes)?;
        Ok(bytes)
    }

    /// Rechecks the digest while copying, detecting changes since construction.
    pub fn copy_file(&mut self, name: &str, writer: &mut impl Write) -> Result<(), String> {
        let entry = self.entries.iter().find(|entry| entry.name == name).ok_or("YAK file not found")?;
        let mut file = self.zip.by_index(entry.index).map_err(|error| error.to_string())?;
        let hash = copy_hashed(&mut file, writer, entry.size)?;
        if hash != entry.sha256 {
            return Err(format!("{name}: changed while reading"));
        }
        Ok(())
    }

    pub fn write_signed(&mut self, writer: impl Write + Seek, key: &SigningKey) -> Result<(), String> {
        let signature = Signature::sign(self.hashes.clone(), key);
        let text = toml::to_string(&signature).map_err(|error| error.to_string())?;
        let names: Vec<_> = self.entries.iter().filter(|entry| entry.name != SIGNATURE)
            .map(|entry| entry.name.clone()).collect();
        let mut output = ZipWriter::new(writer);
        let options = SimpleFileOptions::default().compression_method(CompressionMethod::Stored).unix_permissions(0o644);
        for name in names {
            output.start_file(&name, options).map_err(|error| error.to_string())?;
            self.copy_file(&name, &mut output)?;
        }
        output.start_file(SIGNATURE, options).map_err(|error| error.to_string())?;
        output.write_all(text.as_bytes()).map_err(|error| error.to_string())?;
        output.finish().map_err(|error| error.to_string())?;
        Ok(())
    }
}

pub fn check_path(name: &str, directory: bool) -> Result<(), String> {
    let path = if directory { name.strip_suffix('/').unwrap_or(name) } else { name };
    if path.is_empty() || name.len() > 1024 || name.chars().any(|character| character.is_control())
        || name.contains(['\\', ':', '<', '>', '"', '|', '?', '*']) {
        return Err(format!("unsafe YAK path: {name:?}"));
    }
    for component in path.split('/') {
        let stem = component.split('.').next().unwrap_or_default().to_uppercase();
        let reserved = matches!(stem.as_str(), "CON" | "PRN" | "AUX" | "NUL" | "CONIN$" | "CONOUT$")
            || ["COM", "LPT"].iter().any(|prefix| stem.strip_prefix(*prefix).is_some_and(|suffix| {
                matches!(suffix, "1" | "2" | "3" | "4" | "5" | "6" | "7" | "8" | "9" | "¹" | "²" | "³")
            }));
        if component.is_empty() || matches!(component, "." | "..") || component.ends_with(['.', ' ']) || reserved {
            return Err(format!("unsafe YAK path: {name:?}"));
        }
    }
    Ok(())
}

fn copy_hashed(reader: &mut impl Read, writer: &mut impl Write, expected: u64) -> Result<String, String> {
    let mut total = 0u64;
    let mut hash = Sha256::default();
    let mut buffer = [0u8; 64 * 1024];
    loop {
        let length = reader.read(&mut buffer).map_err(|error| error.to_string())?;
        if length == 0 {
            break;
        }
        total += length as u64;
        if total > expected || total > MAX_FILE_SIZE {
            return Err("YAK file expands beyond its declared size".into());
        }
        hash.update(&buffer[..length]);
        writer.write_all(&buffer[..length]).map_err(|error| error.to_string())?;
    }
    if total != expected {
        return Err("YAK file length does not match ZIP metadata".into());
    }
    Ok(hash.hex())
}

fn preflight(reader: &mut (impl Read + Seek)) -> Result<(), String> {
    let length = reader.seek(SeekFrom::End(0)).map_err(|error| error.to_string())?;
    if !(22..=MAX_ARCHIVE_SIZE).contains(&length) {
        return Err("YAK ZIP size is invalid or exceeds its bound".into());
    }
    let tail_size = length.min(65_557) as usize;
    reader.seek(SeekFrom::End(-(tail_size as i64))).map_err(|error| error.to_string())?;
    let mut tail = vec![0u8; tail_size];
    reader.read_exact(&mut tail).map_err(|error| error.to_string())?;
    let offset = (0..=tail_size - 22).rev().find(|&offset| {
        tail[offset..offset + 4] == *b"PK\x05\x06"
            && offset + 22 + word(&tail, offset + 20) as usize == tail_size
    }).ok_or("YAK ZIP has no end record")?;
    let record = &tail[offset..];
    let end_position = length - tail_size as u64 + offset as u64;
    if word(record, 4) != 0 || word(record, 6) != 0 || word(record, 8) != word(record, 10) {
        return Err("multi-disk YAK ZIP is unsupported".into());
    }
    let mut count = word(record, 10) as u64;
    let mut size = dword(record, 12) as u64;
    let mut start = dword(record, 16) as u64;
    let mut directory_end = end_position;
    if count == u16::MAX as u64 || size == u32::MAX as u64 || start == u32::MAX as u64 {
        if end_position < 20 {
            return Err("missing ZIP64 locator".into());
        }
        let mut locator = [0u8; 20];
        reader.seek(SeekFrom::Start(end_position - 20)).map_err(|error| error.to_string())?;
        reader.read_exact(&mut locator).map_err(|error| error.to_string())?;
        if locator[..4] != *b"PK\x06\x07" || dword(&locator, 4) != 0 || dword(&locator, 16) != 1 {
            return Err("invalid ZIP64 locator".into());
        }
        directory_end = qword(&locator, 8);
        if directory_end.checked_add(56).is_none_or(|end| end > end_position - 20) {
            return Err("invalid ZIP64 end offset".into());
        }
        let mut extended = [0u8; 56];
        reader.seek(SeekFrom::Start(directory_end)).map_err(|error| error.to_string())?;
        reader.read_exact(&mut extended).map_err(|error| error.to_string())?;
        if extended[..4] != *b"PK\x06\x06" || qword(&extended, 4) < 44
            || qword(&extended, 4) > MAX_METADATA_SIZE
            || directory_end.checked_add(12).and_then(|start| start.checked_add(qword(&extended, 4))) != Some(end_position - 20)
            || dword(&extended, 16) != 0 || dword(&extended, 20) != 0
            || qword(&extended, 24) != qword(&extended, 32) {
            return Err("invalid or multi-disk ZIP64 end record".into());
        }
        count = qword(&extended, 32);
        size = qword(&extended, 40);
        start = qword(&extended, 48);
    }
    if count == 0 || count > MAX_ENTRIES || size > MAX_METADATA_SIZE
        || size < count * 46 || start.checked_add(size) != Some(directory_end) {
        return Err("YAK central directory exceeds bounds or has invalid offsets".into());
    }
    let mut position = start;
    // The raw names: a ZIP reader keeps one of two equal names, so duplicates and case
    // collisions are caught here, before it indexes them.
    let mut raw_names = std::collections::BTreeSet::new();
    for _ in 0..count {
        if position.checked_add(46).is_none_or(|end| end > directory_end) {
            return Err("truncated YAK central directory".into());
        }
        reader.seek(SeekFrom::Start(position)).map_err(|error| error.to_string())?;
        let mut header = [0u8; 46];
        reader.read_exact(&mut header).map_err(|error| error.to_string())?;
        if header[..4] != *b"PK\x01\x02" || word(&header, 8) & 0x2041 != 0 {
            return Err("invalid or encrypted YAK ZIP entry".into());
        }
        let name_size = word(&header, 28) as usize;
        let extra_size = word(&header, 30) as usize;
        let next = position.checked_add(46 + name_size as u64 + extra_size as u64 + word(&header, 32) as u64)
            .filter(|end| *end <= directory_end).ok_or("YAK central entry exceeds bounds")?;
        if name_size == 0 || name_size > 1024 {
            return Err("YAK ZIP entry name exceeds bounds".into());
        }
        let mut name = vec![0u8; name_size];
        let mut extra = vec![0u8; extra_size];
        reader.read_exact(&mut name).map_err(|error| error.to_string())?;
        reader.read_exact(&mut extra).map_err(|error| error.to_string())?;
        let local_offset = local_offset(&header, &extra)?;
        if local_offset.checked_add(30).is_none_or(|end| end > start) {
            return Err("YAK local header overlaps its central directory".into());
        }
        reader.seek(SeekFrom::Start(local_offset)).map_err(|error| error.to_string())?;
        let mut local = [0u8; 30];
        reader.read_exact(&mut local).map_err(|error| error.to_string())?;
        if local[..4] != *b"PK\x03\x04" || word(&local, 6) != word(&header, 8)
            || word(&local, 8) != word(&header, 10) || word(&local, 26) as usize != name_size
            || local_offset.checked_add(30 + name_size as u64 + word(&local, 28) as u64)
                .is_none_or(|end| end > start) {
            return Err("inconsistent YAK local header".into());
        }
        let mut local_name = vec![0u8; name_size];
        reader.read_exact(&mut local_name).map_err(|error| error.to_string())?;
        if local_name != name {
            return Err("YAK local and central names differ".into());
        }
        let key = String::from_utf8_lossy(&name).trim_end_matches('/').to_uppercase();
        if !raw_names.insert(key) {
            return Err("duplicate or case-colliding YAK ZIP name".into());
        }
        position = next;
    }
    if position != directory_end {
        return Err("unexpected data in YAK central directory".into());
    }
    reader.seek(SeekFrom::Start(0)).map_err(|error| error.to_string())?;
    Ok(())
}

fn local_offset(header: &[u8], extra: &[u8]) -> Result<u64, String> {
    let offset = dword(header, 42);
    let disk = word(header, 34);
    if offset != u32::MAX && disk == 0 {
        return Ok(offset as u64);
    }
    if disk != 0 && disk != u16::MAX {
        return Err("multi-disk YAK ZIP is unsupported".into());
    }
    let mut position = 0;
    while position + 4 <= extra.len() {
        let kind = word(extra, position);
        let size = word(extra, position + 2) as usize;
        position += 4;
        let data = extra.get(position..position + size).ok_or("truncated YAK ZIP extra field")?;
        position += size;
        if kind != 1 {
            continue;
        }
        let mut index = usize::from(dword(header, 24) == u32::MAX) * 8
            + usize::from(dword(header, 20) == u32::MAX) * 8;
        let offset = if offset == u32::MAX {
            let bytes = data.get(index..index + 8).ok_or("missing ZIP64 local offset")?;
            index += 8;
            qword(bytes, 0)
        } else { offset as u64 };
        if disk == u16::MAX && data.get(index..index + 4).map(|bytes| dword(bytes, 0)) != Some(0) {
            return Err("multi-disk YAK ZIP64 is unsupported".into());
        }
        return Ok(offset);
    }
    Err("missing YAK ZIP64 offset field".into())
}

fn word(bytes: &[u8], offset: usize) -> u16 {
    u16::from_le_bytes(bytes[offset..offset + 2].try_into().unwrap())
}

fn dword(bytes: &[u8], offset: usize) -> u32 {
    u32::from_le_bytes(bytes[offset..offset + 4].try_into().unwrap())
}

fn qword(bytes: &[u8], offset: usize) -> u64 {
    u64::from_le_bytes(bytes[offset..offset + 8].try_into().unwrap())
}

/// Refuses existing symlinks and Windows reparse points, including ancestor directories.
pub fn check_plain_path(path: &Path) -> Result<(), String> {
    let absolute = if path.is_absolute() { path.to_path_buf() } else {
        std::env::current_dir().map_err(|error| error.to_string())?.join(path)
    };
    let mut checked = PathBuf::new();
    for component in absolute.components() {
        if component == Component::ParentDir {
            return Err("filesystem paths must not contain ..".into());
        }
        checked.push(component.as_os_str());
        match std::fs::symlink_metadata(&checked) {
            Ok(metadata) if is_link(&metadata) => return Err(format!("{} is a link or reparse point", checked.display())),
            Ok(_) => (),
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => (),
            Err(error) => return Err(error.to_string()),
        }
    }
    Ok(())
}

pub fn is_link(metadata: &std::fs::Metadata) -> bool {
    #[cfg(windows)]
    {
        use std::os::windows::fs::MetadataExt;
        metadata.file_attributes() & 0x400 != 0
    }
    #[cfg(not(windows))]
    {
        metadata.file_type().is_symlink()
    }
}

fn temporary_path(parent: &Path) -> Result<PathBuf, String> {
    let mut random = [0u8; 16];
    SystemRandom::new().fill(&mut random).map_err(|_| "temporary-name randomness failed")?;
    let suffix: String = random.iter().map(|byte| format!("{byte:02x}")).collect();
    Ok(parent.join(format!(".yak-{suffix}.tmp")))
}

struct Cleanup(PathBuf, bool);

impl Drop for Cleanup {
    fn drop(&mut self) {
        if self.1 {
            let _ = std::fs::remove_dir_all(&self.0);
        } else {
            let _ = std::fs::remove_file(&self.0);
        }
    }
}

/// Publishes a sibling temporary file; replacement is reserved for in-place signing.
pub fn atomic_write(path: &Path, replace: bool, write: impl FnOnce(&mut File) -> Result<(), String>) -> Result<(), String> {
    check_plain_path(path)?;
    let parent = path.parent().filter(|parent| !parent.as_os_str().is_empty()).unwrap_or(Path::new("."));
    let temporary_path = temporary_path(parent)?;
    let mut file = OpenOptions::new().write(true).read(true).create_new(true).open(&temporary_path)
        .map_err(|error| error.to_string())?;
    let temporary = Cleanup(temporary_path, false);
    let result = write(&mut file).and_then(|()| file.sync_all().map_err(|error| error.to_string()));
    drop(file);
    result?;
    check_plain_path(path)?;
    if replace {
        std::fs::rename(&temporary.0, path).map_err(|error| error.to_string())?;
    } else {
        std::fs::hard_link(&temporary.0, path).map_err(|error| error.to_string())?;
    }
    Ok(())
}

/// The caller must own the destination parent exclusively during extraction.
pub fn atomic_directory(path: &Path, write: impl FnOnce(&Path) -> Result<(), String>) -> Result<(), String> {
    check_plain_path(path)?;
    if path.try_exists().map_err(|error| error.to_string())? {
        return Err("YAK extraction destination already exists".into());
    }
    let parent = path.parent().filter(|parent| !parent.as_os_str().is_empty()).unwrap_or(Path::new("."));
    let temporary_path = temporary_path(parent)?;
    std::fs::create_dir(&temporary_path).map_err(|error| error.to_string())?;
    let temporary = Cleanup(temporary_path, true);
    write(&temporary.0)?;
    check_plain_path(path)?;
    if path.try_exists().map_err(|error| error.to_string())? {
        return Err("YAK extraction destination appeared while extracting".into());
    }
    std::fs::rename(&temporary.0, path).map_err(|error| error.to_string())
}

pub fn sign_file(path: &Path, key: &SigningKey) -> Result<(), String> {
    check_plain_path(path)?;
    let file = File::open(path).map_err(|error| error.to_string())?;
    let mut archive = Archive::new(file)?;
    atomic_write(path, true, |file| {
        archive.write_signed(&mut *file, key)?;
        file.seek(SeekFrom::Start(0)).map_err(|error| error.to_string())?;
        Archive::new(&mut *file)?.verify_signature(&[key.public()])?;
        drop(archive);
        Ok(())
    })
}
