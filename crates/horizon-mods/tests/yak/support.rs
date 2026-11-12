use horizon_mods::yak::{Archive, Manifest};
use std::collections::BTreeMap;
use std::fs;
use std::io::{Cursor, Write};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::{SystemTime, UNIX_EPOCH};
use zip::write::SimpleFileOptions;
use zip::{CompressionMethod, ZipWriter};

pub const DATA: &str = r#"format = 1
id = "sample-mod"
version = "0.3.1"
name = "Sample"
name_ja = "サンプル"
authors = ["Test author"]
description = "A data-only test package."
license = "MIT"
homepage = "https://example.test/sample"
[yoake]
version = ">=0.3, <0.4"
[side]
client = true
server = true
required_on_join = false
"#;

pub fn program() -> String {
    [DATA, r#"
[bin]
library = "sample"
targets = ["windows-x64", "linux-x64"]
engine = "0.3.1+build.test"
abi = 2
"#].concat()
}

pub fn named(id: &str) -> Manifest {
    Manifest::parse(&DATA.replace("sample-mod", id)).unwrap()
}

pub fn zip_files(files: &[(&str, &[u8])], method: CompressionMethod) -> Vec<u8> {
    let mut writer = ZipWriter::new(Cursor::new(Vec::new()));
    for (name, bytes) in files {
        writer.start_file(*name, SimpleFileOptions::default().compression_method(method).unix_permissions(0o644)).unwrap();
        writer.write_all(bytes).unwrap();
    }
    writer.finish().unwrap().into_inner()
}

pub fn stored(files: &[(&str, &[u8])]) -> Vec<u8> {
    zip_files(files, CompressionMethod::Stored)
}

pub fn data_zip() -> Vec<u8> {
    stored(&[("yak.toml", DATA.as_bytes()), ("content/example.json", b"{}")])
}

pub fn unpack(bytes: &[u8]) -> BTreeMap<String, Vec<u8>> {
    let mut archive = horizon_release_sign::yak::Archive::new(Cursor::new(bytes)).unwrap();
    let names: Vec<_> = archive.entries().iter().map(|entry| entry.name.clone()).collect();
    names.into_iter().map(|name| {
        let bytes = archive.read_small(&name, 1024 * 1024).unwrap();
        (name, bytes)
    }).collect()
}

pub fn repack(files: &BTreeMap<String, Vec<u8>>) -> Vec<u8> {
    stored(&files.iter().map(|(name, bytes)| (name.as_str(), bytes.as_slice())).collect::<Vec<_>>())
}

pub fn reject(bytes: Vec<u8>) {
    assert!(Archive::new(Cursor::new(bytes)).is_err());
}

pub fn central_offsets(bytes: &[u8]) -> Vec<usize> {
    let end = bytes.windows(4).rposition(|bytes| bytes == b"PK\x05\x06").unwrap();
    let count = u16::from_le_bytes(bytes[end + 10..end + 12].try_into().unwrap());
    let mut offset = u32::from_le_bytes(bytes[end + 16..end + 20].try_into().unwrap()) as usize;
    let mut offsets = Vec::new();
    for _ in 0..count {
        assert_eq!(&bytes[offset..offset + 4], b"PK\x01\x02");
        offsets.push(offset);
        let extra: usize = (28..34).step_by(2).map(|field| u16::from_le_bytes(bytes[offset + field..offset + field + 2].try_into().unwrap()) as usize).sum();
        offset += 46 + extra;
    }
    offsets
}

pub fn local_offset(bytes: &[u8], central: usize) -> usize {
    u32::from_le_bytes(bytes[central + 42..central + 46].try_into().unwrap()) as usize
}

pub fn set16(bytes: &mut [u8], offset: usize, value: u16) {
    bytes[offset..offset + 2].copy_from_slice(&value.to_le_bytes());
}

pub fn set32(bytes: &mut [u8], offset: usize, value: u32) {
    bytes[offset..offset + 4].copy_from_slice(&value.to_le_bytes());
}

pub fn rename(bytes: &mut [u8], index: usize, name: &[u8]) {
    let central = central_offsets(bytes)[index];
    let local = local_offset(bytes, central);
    let length = u16::from_le_bytes(bytes[central + 28..central + 30].try_into().unwrap()) as usize;
    assert_eq!(length, name.len());
    bytes[central + 46..central + 46 + length].copy_from_slice(name);
    bytes[local + 30..local + 30 + length].copy_from_slice(name);
}

pub struct Temp(pub PathBuf);

impl Temp {
    pub fn new() -> Self {
        static NEXT: AtomicU64 = AtomicU64::new(0);
        let stamp = SystemTime::now().duration_since(UNIX_EPOCH).unwrap().as_nanos();
        let path = std::env::temp_dir().join(format!("horizon-yak-test-{}-{stamp}-{}", std::process::id(), NEXT.fetch_add(1, Ordering::Relaxed)));
        fs::create_dir(&path).unwrap();
        Self(path)
    }

    pub fn write(&self, name: &str, bytes: impl AsRef<[u8]>) -> PathBuf {
        let path = self.0.join(name);
        fs::create_dir_all(path.parent().unwrap()).unwrap();
        fs::write(&path, bytes).unwrap();
        path
    }

    pub fn join(&self, name: impl AsRef<Path>) -> PathBuf {
        self.0.join(name)
    }
}

impl Drop for Temp {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.0);
    }
}
