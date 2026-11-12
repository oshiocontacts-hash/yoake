use super::support::*;
use horizon_mods::yak::{Archive, MAX_ARCHIVE_SIZE, MAX_ENTRIES, MAX_FILE_SIZE, Part, Target, pack_directory};
use std::collections::BTreeMap;
use std::fs::{self, File};
use std::io::{self, Cursor, Read, Seek, SeekFrom};

#[test]
fn data_round_trip_preserves_every_data_part() {
    let temp = Temp::new();
    let manifest = format!("{DATA}\n[[shaders]]\nid = 'kage'\nname = 'Kage'\n");
    temp.write("mod/yak.toml", manifest);
    for (path, bytes) in [("content/blocks.json", "{}"), ("assets/sample/icon.png", "fake-png"), ("shaders/kage/main.wgsl", "shader"), ("config/default.toml", "enabled = true"), ("LICENSE", "MIT"), ("README.md", "Sample")] {
        temp.write(&format!("mod/{path}"), bytes);
    }
    let output = temp.join("sample.yak");
    let summary = pack_directory(&temp.join("mod"), &output, &BTreeMap::new()).unwrap();
    assert!(!summary.signed);
    assert_eq!(summary.files, 7);
    let mut archive = Archive::new(File::open(&output).unwrap()).unwrap();
    assert_eq!(archive.verify_signature(&[]).unwrap(), None);
    assert_eq!(archive.files(&Part::Content).count(), 1);
    assert_eq!(archive.files(&Part::Assets).count(), 1);
    assert_eq!(archive.files(&Part::Shaders("kage".into())).count(), 1);
    let view = temp.join("view");
    archive.extract_target(&view, Target::MacosArm64, &[]).unwrap();
    assert_eq!(fs::read(view.join("content/blocks.json")).unwrap(), b"{}");
    assert_eq!(fs::read(view.join("shaders/kage/main.wgsl")).unwrap(), b"shader");
    assert!(view.join("config/default.toml").is_file());
    assert!(archive.extract_target(&view, Target::MacosArm64, &[]).is_err());
    assert!(pack_directory(&temp.join("mod"), &output, &BTreeMap::new()).is_err());
}

#[test]
fn program_round_trip_includes_only_the_selected_target_in_the_view() {
    let temp = Temp::new();
    temp.write("mod/yak.toml", program());
    temp.write("mod/content/example.json", b"{}");
    let windows = temp.write("artifacts/renamed.dll", b"fake-dll");
    temp.write("artifacts/linux/libsample.so", b"fake-so");
    temp.write("artifacts/linux/helper.dat", b"sidecar");
    let binaries = BTreeMap::from([(Target::WindowsX64, windows), (Target::LinuxX64, temp.join("artifacts/linux"))]);
    let output = temp.join("sample.yak");
    pack_directory(&temp.join("mod"), &output, &binaries).unwrap();
    let mut archive = Archive::new(File::open(output).unwrap()).unwrap();
    assert_eq!(archive.files(&Part::Bin(Target::LinuxX64)).count(), 2);
    assert!(archive.extract_target(&temp.join("unsupported"), Target::MacosX64, &[]).is_err());
    let view = temp.join("view");
    archive.extract_target(&view, Target::WindowsX64, &[]).unwrap();
    assert_eq!(fs::read(view.join("bin/windows-x64/sample.dll")).unwrap(), b"fake-dll");
    assert!(!view.join("bin/linux-x64").exists());
    assert!(view.join("content/example.json").exists());
}

#[test]
fn unsafe_names_and_windows_aliases_are_rejected() {
    for name in ["../evil", "/evil", "C:/evil", "content/../evil", "content/./evil", "content//evil", "content/evil.", "content/evil ", "content/CON", "content/con.txt", "content/COM1", "content/LPT9", "content/COM¹", "content/a:b", "content/a?b", "content/a|b", "content/a\\b", "content/evil\n"] {
        let placeholder = "x".repeat(name.len());
        let mut bytes = stored(&[("yak.toml", DATA.as_bytes()), (&placeholder, b"bad")]);
        rename(&mut bytes, 1, name.as_bytes());
        reject(bytes);
    }
    let mut bytes = stored(&[("yak.toml", DATA.as_bytes()), ("content/x", b"bad")]);
    rename(&mut bytes, 1, b"content/\xff");
    reject(bytes);
}

#[test]
fn duplicates_case_collisions_and_file_parent_collisions_are_rejected() {
    for second in ["content/a", "CONTENT/A"] {
        let mut bytes = stored(&[("yak.toml", DATA.as_bytes()), ("content/a", b"a"), ("content/b", b"b")]);
        rename(&mut bytes, 2, second.as_bytes());
        reject(bytes);
    }
    reject(stored(&[("yak.toml", DATA.as_bytes()), ("content/a", b"a"), ("content/a/b", b"b")]));
    reject(stored(&[("yak.toml", DATA.as_bytes()), ("content/A", b"a"), ("CONTENT/a/b", b"b")]));
}

#[test]
fn links_encryption_unsupported_compression_and_invalid_headers_are_rejected() {
    for mode in [0o120777u32, 0o020666, 0o060666] {
        let mut bytes = data_zip();
        let central = central_offsets(&bytes)[1];
        bytes[central + 5] = 3;
        set32(&mut bytes, central + 38, mode << 16);
        reject(bytes);
    }
    for flags in [1, 0x40, 0x2000] {
        let mut bytes = data_zip();
        let central = central_offsets(&bytes)[1];
        let local = local_offset(&bytes, central);
        set16(&mut bytes, central + 8, flags);
        set16(&mut bytes, local + 6, flags);
        reject(bytes);
    }
    let mut bytes = data_zip();
    let central = central_offsets(&bytes)[1];
    let local = local_offset(&bytes, central);
    set16(&mut bytes, central + 10, 99);
    set16(&mut bytes, local + 8, 99);
    reject(bytes);
    let mut bytes = data_zip();
    let central = central_offsets(&bytes)[1];
    let local = local_offset(&bytes, central);
    bytes[local + 30] ^= 1;
    reject(bytes);
    let mut bytes = data_zip();
    bytes.truncate(bytes.len() - 1);
    reject(bytes);
}

#[test]
fn corrupt_crc_or_missing_or_undeclared_parts_are_rejected() {
    let mut bytes = data_zip();
    let central = central_offsets(&bytes)[1];
    let local = local_offset(&bytes, central);
    let length = u16::from_le_bytes(bytes[local + 26..local + 28].try_into().unwrap()) as usize;
    let extra = u16::from_le_bytes(bytes[local + 28..local + 30].try_into().unwrap()) as usize;
    bytes[local + 30 + length + extra] ^= 1;
    reject(bytes);
    reject(stored(&[("content/a", b"a")]));
    reject(stored(&[("yak.toml", DATA.as_bytes()), ("unknown/a", b"a")]));
    reject(stored(&[("yak.toml", DATA.as_bytes()), ("bin/windows-x64/sample.dll", b"fake")]));
    reject(stored(&[("yak.toml", program().as_bytes()), ("bin/windows-x64/sample.dll", b"fake")]));
    let shaders = format!("{DATA}\n[[shaders]]\nid = 'kage'\nname = 'Kage'\n");
    reject(stored(&[("yak.toml", shaders.as_bytes())]));
    reject(stored(&[("yak.toml", DATA.as_bytes()), ("shaders/unknown/a", b"shader")]));
}

#[test]
fn archive_size_ratio_and_metadata_limits_are_enforced() {
    let mut bytes = data_zip();
    let central = central_offsets(&bytes)[1];
    set32(&mut bytes, central + 24, MAX_FILE_SIZE as u32 + 1);
    set32(&mut bytes, central + 20, MAX_FILE_SIZE as u32 + 1);
    reject(bytes);
    let mut bytes = data_zip();
    let central = central_offsets(&bytes)[0];
    set32(&mut bytes, central + 24, 1024 * 1024 + 1);
    set32(&mut bytes, central + 20, 1024 * 1024 + 1);
    reject(bytes);
    let mut bytes = stored(&[("yak.toml", DATA.as_bytes()), ("SIGNATURE", b"x")]);
    let central = central_offsets(&bytes)[1];
    set32(&mut bytes, central + 24, 2 * 1024 * 1024 + 1);
    set32(&mut bytes, central + 20, 2 * 1024 * 1024 + 1);
    reject(bytes);
    reject(zip_files(&[("yak.toml", DATA.as_bytes()), ("content/bomb", &[0; 65536])], zip::CompressionMethod::Deflated));
    let mut bytes = stored(&[("yak.toml", DATA.as_bytes()), ("content/a", b"a"), ("content/b", b"b"), ("content/c", b"c"), ("content/d", b"d")]);
    for central in central_offsets(&bytes).into_iter().skip(1) {
        set32(&mut bytes, central + 24, MAX_FILE_SIZE as u32);
        set32(&mut bytes, central + 20, 2 * 1024 * 1024);
    }
    reject(bytes);
    let mut bytes = data_zip();
    let end = bytes.len() - 22;
    set16(&mut bytes, end + 8, MAX_ENTRIES as u16 + 1);
    set16(&mut bytes, end + 10, MAX_ENTRIES as u16 + 1);
    reject(bytes);
    let mut bytes = data_zip();
    let end = bytes.len() - 22;
    set32(&mut bytes, end + 12, 32 * 1024 * 1024 + 1);
    reject(bytes);
    let mut bytes = data_zip();
    let end = bytes.len() - 22;
    set16(&mut bytes, end + 4, 1);
    reject(bytes);
    struct Oversized;
    impl Read for Oversized {
        fn read(&mut self, _: &mut [u8]) -> io::Result<usize> {
            panic!("oversized archive must be refused before reading");
        }
    }
    impl Seek for Oversized {
        fn seek(&mut self, _: SeekFrom) -> io::Result<u64> {
            Ok(MAX_ARCHIVE_SIZE + 1)
        }
    }
    assert!(Archive::new(Oversized).is_err());
}

#[test]
fn valid_zip64_archive_is_accepted() {
    let mut bytes = data_zip();
    let end = bytes.len() - 22;
    let mut ordinary_end = bytes.split_off(end);
    let count = u16::from_le_bytes(ordinary_end[10..12].try_into().unwrap()) as u64;
    let size = u32::from_le_bytes(ordinary_end[12..16].try_into().unwrap()) as u64;
    let start = u32::from_le_bytes(ordinary_end[16..20].try_into().unwrap()) as u64;
    let mut extended = vec![0u8; 56];
    extended[..4].copy_from_slice(b"PK\x06\x06");
    extended[4..12].copy_from_slice(&44u64.to_le_bytes());
    set16(&mut extended, 12, 45);
    set16(&mut extended, 14, 45);
    extended[24..32].copy_from_slice(&count.to_le_bytes());
    extended[32..40].copy_from_slice(&count.to_le_bytes());
    extended[40..48].copy_from_slice(&size.to_le_bytes());
    extended[48..56].copy_from_slice(&start.to_le_bytes());
    let mut locator = vec![0u8; 20];
    locator[..4].copy_from_slice(b"PK\x06\x07");
    locator[8..16].copy_from_slice(&(end as u64).to_le_bytes());
    set32(&mut locator, 16, 1);
    set16(&mut ordinary_end, 8, u16::MAX);
    set16(&mut ordinary_end, 10, u16::MAX);
    set32(&mut ordinary_end, 12, u32::MAX);
    set32(&mut ordinary_end, 16, u32::MAX);
    bytes.extend(extended);
    bytes.extend(locator);
    bytes.extend(ordinary_end);
    assert_eq!(Archive::new(Cursor::new(bytes)).unwrap().manifest().id, "sample-mod");
}

#[test]
fn failed_pack_or_extraction_leaves_no_partial_destination() {
    let temp = Temp::new();
    temp.write("mod/yak.toml", DATA);
    temp.write("mod/shaders/undeclared/main.wgsl", "shader");
    let output = temp.join("bad.yak");
    assert!(pack_directory(&temp.join("mod"), &output, &BTreeMap::new()).is_err());
    assert!(!output.exists());
    let destination = temp.join("view");
    let result = horizon_release_sign::yak::atomic_directory(&destination, |directory| {
        fs::write(directory.join("partial"), b"partial").unwrap();
        Err("injected extraction failure".into())
    });
    assert!(result.is_err());
    assert!(!destination.exists());
    assert!(fs::read_dir(&temp.0).unwrap().all(|entry| !entry.unwrap().file_name().to_string_lossy().starts_with(".yak-")));
}

#[test]
fn changes_between_validation_and_extraction_are_detected_and_cleaned_up() {
    use std::cell::RefCell;
    use std::rc::Rc;
    struct MutableReader {
        bytes: Rc<RefCell<Vec<u8>>>,
        position: u64,
    }
    impl Read for MutableReader {
        fn read(&mut self, buffer: &mut [u8]) -> io::Result<usize> {
            let bytes = self.bytes.borrow();
            let mut cursor = Cursor::new(bytes.as_slice());
            cursor.set_position(self.position);
            let count = cursor.read(buffer)?;
            self.position = cursor.position();
            Ok(count)
        }
    }
    impl Seek for MutableReader {
        fn seek(&mut self, position: SeekFrom) -> io::Result<u64> {
            let bytes = self.bytes.borrow();
            let mut cursor = Cursor::new(bytes.as_slice());
            cursor.set_position(self.position);
            self.position = cursor.seek(position)?;
            Ok(self.position)
        }
    }
    let bytes = data_zip();
    let central = central_offsets(&bytes)[1];
    let local = local_offset(&bytes, central);
    let length = u16::from_le_bytes(bytes[local + 26..local + 28].try_into().unwrap()) as usize;
    let extra = u16::from_le_bytes(bytes[local + 28..local + 30].try_into().unwrap()) as usize;
    let shared = Rc::new(RefCell::new(bytes));
    let mut archive = Archive::new(MutableReader { bytes: shared.clone(), position: 0 }).unwrap();
    shared.borrow_mut()[local + 30 + length + extra] ^= 1;
    let temp = Temp::new();
    let destination = temp.join("view");
    assert!(archive.extract_target(&destination, Target::WindowsX64, &[]).is_err());
    assert!(!destination.exists());
    assert_eq!(fs::read_dir(&temp.0).unwrap().count(), 0);
}

#[cfg(unix)]
#[test]
fn filesystem_links_are_not_followed() {
    use std::os::unix::fs::symlink;
    let temp = Temp::new();
    temp.write("mod/yak.toml", DATA);
    let external = temp.write("external", "outside");
    fs::create_dir(temp.join("mod/content")).unwrap();
    symlink(&external, temp.join("mod/content/link")).unwrap();
    assert!(pack_directory(&temp.join("mod"), &temp.join("bad.yak"), &BTreeMap::new()).is_err());
    let real = temp.join("real");
    fs::create_dir(&real).unwrap();
    symlink(&real, temp.join("linked")).unwrap();
    let mut archive = Archive::new(Cursor::new(data_zip())).unwrap();
    assert!(archive.extract_target(&temp.join("linked/view"), Target::WindowsX64, &[]).is_err());
    assert!(!real.join("view").exists());
}
