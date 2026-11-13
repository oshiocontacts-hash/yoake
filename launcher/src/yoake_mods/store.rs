use super::*;
use horizon_release_sign::yak::{atomic_write, check_plain_path};
use std::fs::{self, OpenOptions};
use std::io::{Read, Write};

#[derive(Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub(super) enum Origin {
    Catalog(String),
    Signed,
    UnsignedData,
}

#[derive(Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub(super) struct Record {
    pub manifest: Manifest,
    pub sha256: String,
    pub origin: Origin,
}

#[derive(Clone, Default, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub(super) struct State {
    pub records: Vec<Record>,
}

#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct Journal {
    state: State,
    list: Option<Vec<u8>>,
    created: Vec<Manifest>,
    #[serde(default)]
    removed: Vec<Manifest>,
}

pub(super) fn plain(path: &Path) -> Result<(), Error> {
    check_plain_path(path).map_err(Error::Storage)
}

pub(super) fn cache(home: &Path) -> PathBuf {
    home.join("yoake-mods/.cache")
}
pub(super) fn directory(home: &Path, manifest: &Manifest) -> PathBuf {
    home.join("yoake-mods")
        .join(&manifest.id)
        .join(manifest.version.to_string())
}
pub(super) fn archive_path(home: &Path, hash: &str) -> Result<PathBuf, Error> {
    if hash.len() != 64 || !hash.bytes().all(|byte| byte.is_ascii_hexdigit()) {
        return Err(Error::Invalid("Invalid archive digest".into()));
    }
    Ok(cache(home).join(format!("{hash}.yak")))
}

pub(super) fn lock(home: &Path) -> Result<File, Error> {
    let path = cache(home);
    plain(&path)?;
    fs::create_dir_all(&path)?;
    let path = path.join("install.lock");
    plain(&path)?;
    let lock = OpenOptions::new()
        .read(true)
        .write(true)
        .create(true)
        .truncate(false)
        .open(path)?;
    lock.try_lock().map_err(|_| Error::Busy)?;
    Ok(lock)
}

pub(super) fn read_bytes(path: &Path) -> Result<Vec<u8>, Error> {
    plain(path)?;
    let mut bytes = Vec::new();
    File::open(path)?
        .take(64 * 1024 * 1024 + 1)
        .read_to_end(&mut bytes)?;
    if bytes.len() > 64 * 1024 * 1024 {
        return Err(Error::Invalid("Mod metadata is too large".into()));
    }
    Ok(bytes)
}

pub(super) fn read_json<Value: serde::de::DeserializeOwned>(path: &Path) -> Result<Value, Error> {
    serde_json::from_slice(&read_bytes(path)?).map_err(|error| Error::Invalid(error.to_string()))
}

pub(super) fn write_bytes(path: &Path, bytes: &[u8]) -> Result<(), Error> {
    plain(path)?;
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent)?;
    }
    atomic_write(path, true, |file| {
        file.write_all(bytes).map_err(|error| error.to_string())
    })
    .map_err(Error::Storage)
}

pub(super) fn write_json<Value: Serialize>(path: &Path, value: &Value) -> Result<(), Error> {
    let bytes = serde_json::to_vec(value).map_err(|error| Error::Invalid(error.to_string()))?;
    write_bytes(path, &bytes)
}

fn optional_bytes(path: &Path) -> Result<Option<Vec<u8>>, Error> {
    plain(path)?;
    if !path.try_exists()? {
        return Ok(None);
    }
    read_bytes(path).map(Some)
}

pub(super) fn load(home: &Path) -> Result<(State, Vec<String>, String), Error> {
    recover(home)?;
    let path = cache(home).join("installed.json");
    let state: State = match optional_bytes(&path)? {
        Some(bytes) => {
            serde_json::from_slice(&bytes).map_err(|error| Error::Invalid(error.to_string()))?
        }
        None => State::default(),
    };
    if state.records.len() > 128 {
        return Err(Error::Invalid("Too many installed mods".into()));
    }
    let mut ids = std::collections::BTreeSet::new();
    for record in &state.records {
        record.manifest.validate().map_err(Error::Invalid)?;
        archive_path(home, &record.sha256)?;
        if !ids.insert(record.manifest.id.clone()) {
            return Err(Error::Invalid("Duplicate installed mod".into()));
        }
    }
    let list = optional_bytes(&home.join("yoake-mods.txt"))?.unwrap_or_default();
    if list.len() > 1024 * 1024 {
        return Err(Error::Invalid("Mod list is too large".into()));
    }
    let text =
        std::str::from_utf8(&list).map_err(|_| Error::Invalid("Mod list is not UTF-8".into()))?;
    let mut enabled = Vec::new();
    for line in text
        .lines()
        .map(str::trim)
        .filter(|line| !line.is_empty() && !line.starts_with('#'))
    {
        let mut fields = line.split_whitespace();
        let id = fields
            .next()
            .ok_or_else(|| Error::Invalid("Invalid mod list line".into()))?;
        let version = fields
            .next()
            .ok_or_else(|| Error::Invalid("Invalid mod list line".into()))?;
        if fields.next().is_some() {
            return Err(Error::Invalid("Invalid mod list line".into()));
        }
        if enabled.iter().any(|existing| existing == id)
            || !state.records.iter().any(|record| {
                record.manifest.id == id && record.manifest.version.to_string() == version
            })
        {
            return Err(Error::Invalid(
                "Mod list does not match installed mods".into(),
            ));
        }
        enabled.push(id.to_owned());
    }
    let mut snapshot =
        serde_json::to_vec(&state).map_err(|error| Error::Invalid(error.to_string()))?;
    snapshot.extend_from_slice(&list);
    Ok((state, enabled, horizon_release_sign::sha256_hex(&snapshot)))
}

pub(super) fn recover(home: &Path) -> Result<(), Error> {
    let path = cache(home).join("pending.json");
    plain(&path)?;
    if !path.try_exists()? {
        return Ok(());
    }
    let journal: Journal = read_json(&path)?;
    for manifest in &journal.created {
        if journal.state.records.iter().any(|record| {
            record.manifest.id == manifest.id && record.manifest.version == manifest.version
        }) {
            return Err(Error::Invalid("Invalid mod transaction journal".into()));
        }
        remove_directory(home, manifest)?;
    }
    for manifest in &journal.removed {
        if !journal.state.records.iter().any(|record| {
            record.manifest.id == manifest.id && record.manifest.version == manifest.version
        }) {
            return Err(Error::Invalid("Invalid mod removal journal".into()));
        }
        let staged = removal_path(home, manifest)?;
        if staged.try_exists()? {
            let destination = directory(home, manifest);
            plain(&destination)?;
            let root = fs::canonicalize(home.join("yoake-mods"))?;
            let source = fs::canonicalize(&staged)?;
            let parent = destination
                .parent()
                .ok_or_else(|| Error::Invalid("Unsafe mod recovery".into()))?;
            let parent = fs::canonicalize(parent)?;
            if !source.starts_with(&root) || !parent.starts_with(&root) {
                return Err(Error::Invalid("Unsafe mod recovery".into()));
            }
            yak::check_installed_tree(&source).map_err(Error::Storage)?;
            fs::rename(source, parent.join(manifest.version.to_string()))?;
        }
    }
    write_json(&cache(home).join("installed.json"), &journal.state)?;
    let list_path = home.join("yoake-mods.txt");
    match journal.list {
        Some(bytes) => write_bytes(&list_path, &bytes)?,
        None => {
            plain(&list_path)?;
            if list_path.try_exists()? {
                fs::remove_file(list_path)?;
            }
        }
    }
    fs::remove_file(path)?;
    Ok(())
}

pub(super) fn commit(
    home: &Path,
    old: &State,
    state: &State,
    order: &[String],
) -> Result<(), Error> {
    let pending = cache(home).join("pending.json");
    plain(&pending)?;
    if !pending.try_exists()? {
        begin(home, old, &[])?;
    }
    let result = (|| {
        write_json(&cache(home).join("installed.json"), state)?;
        let mut list = String::new();
        for id in order {
            let record = state
                .records
                .iter()
                .find(|record| &record.manifest.id == id)
                .ok_or_else(|| Error::Invalid("Missing resolved mod".into()))?;
            list.push_str(&format!("{}\t{}\n", id, record.manifest.version));
        }
        write_bytes(&home.join("yoake-mods.txt"), list.as_bytes())?;
        fs::remove_file(&pending)?;
        Ok(())
    })();
    if result.is_err() {
        recover(home)?;
    }
    result
}

pub(super) fn begin(home: &Path, old: &State, created: &[Manifest]) -> Result<(), Error> {
    let journal = Journal {
        state: old.clone(),
        list: optional_bytes(&home.join("yoake-mods.txt"))?,
        created: created.to_vec(),
        removed: Vec::new(),
    };
    write_json(&cache(home).join("pending.json"), &journal)
}

fn removal_path(home: &Path, manifest: &Manifest) -> Result<PathBuf, Error> {
    manifest.validate().map_err(Error::Invalid)?;
    let path = cache(home).join(format!("removed-{}-{}", manifest.id, manifest.version));
    plain(&path)?;
    Ok(path)
}

pub(super) fn stage_remove(home: &Path, old: &State, manifest: &Manifest) -> Result<(), Error> {
    let staged = removal_path(home, manifest)?;
    if staged.try_exists()? {
        finish_remove(home, manifest)?;
    }
    let source = directory(home, manifest);
    plain(&source)?;
    let root = fs::canonicalize(home.join("yoake-mods"))?;
    let resolved_source = fs::canonicalize(&source)?;
    let resolved_parent = fs::canonicalize(cache(home))?;
    if !resolved_source.starts_with(&root) || !resolved_parent.starts_with(&root) {
        return Err(Error::Invalid("Unsafe mod removal".into()));
    }
    yak::check_installed_tree(&resolved_source).map_err(Error::Storage)?;
    let journal = Journal {
        state: old.clone(),
        list: optional_bytes(&home.join("yoake-mods.txt"))?,
        created: Vec::new(),
        removed: vec![manifest.clone()],
    };
    write_json(&cache(home).join("pending.json"), &journal)?;
    fs::rename(resolved_source, staged)?;
    Ok(())
}

pub(super) fn finish_remove(home: &Path, manifest: &Manifest) -> Result<(), Error> {
    let staged = removal_path(home, manifest)?;
    if !staged.try_exists()? {
        return Ok(());
    }
    let root = fs::canonicalize(cache(home))?;
    let resolved = fs::canonicalize(staged)?;
    if !resolved.starts_with(&root) || resolved.parent() != Some(root.as_path()) {
        return Err(Error::Invalid("Unsafe mod removal".into()));
    }
    yak::check_installed_tree(&resolved).map_err(Error::Storage)?;
    fs::remove_dir_all(resolved)?;
    Ok(())
}

pub(super) fn remove_directory(home: &Path, manifest: &Manifest) -> Result<(), Error> {
    manifest.validate().map_err(Error::Invalid)?;
    let path = directory(home, manifest);
    plain(&path)?;
    if !path.try_exists()? {
        return Ok(());
    }
    let root = fs::canonicalize(home.join("yoake-mods"))?;
    let target = fs::canonicalize(&path)?;
    if !target.starts_with(&root)
        || target
            .strip_prefix(&root)
            .map_or(true, |tail| tail.components().count() != 2)
    {
        return Err(Error::Invalid("Unsafe mod directory".into()));
    }
    yak::check_installed_tree(&target).map_err(Error::Storage)?;
    fs::remove_dir_all(target)?;
    Ok(())
}
