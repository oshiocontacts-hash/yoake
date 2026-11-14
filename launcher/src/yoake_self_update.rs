use crate::yoake_auth::YoakeAuth;
use crate::yoake_feed::{FeedError, FeedTransport};
use horizon_release_sign::{PublicKey, Sha256, verify};
use semver::Version;
use serde::Deserialize;
use std::fs::{self, OpenOptions};
use std::io::{Read, Write};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU8, AtomicU64, Ordering};
use url::Url;

const MAX_METADATA: usize = 64 * 1024;
const MAX_SUMS: usize = 16 * 1024;
const MAX_SIGNATURE: usize = 8 * 1024;
const MAX_LAUNCHER: u64 = 8 * 1024 * 1024 * 1024;
static NEXT_TEMP: AtomicU64 = AtomicU64::new(0);

#[derive(Deserialize)]
struct Latest {
    version: String,
    tag: String,
    file: String,
    sha256: String,
    size: u64,
}

struct Staged(PathBuf);

impl Drop for Staged {
    fn drop(&mut self) {
        let _ = fs::remove_file(&self.0);
    }
}

fn cancelled(state: &AtomicU8) -> Result<(), String> {
    if state.load(Ordering::Acquire) == 1 {
        Err("The launcher update was cancelled.".into())
    } else {
        Ok(())
    }
}

fn already_installed(current: &str, offered: &str, installed: Option<&str>) -> bool {
    installed
        .filter(|version| *version != current)
        .and_then(|version| Some((Version::parse(version).ok()?, Version::parse(offered).ok()?)))
        .is_some_and(|(installed, offered)| installed.cmp_precedence(&offered).is_ge())
}

pub(super) fn transport_error(error: FeedError) -> String {
    match error {
        FeedError::NeedsLogin | FeedError::Authentication(_) => {
            "Please sign in to the portal again before updating the launcher.".into()
        }
        FeedError::TooLarge => "The launcher update is larger than expected.".into(),
        FeedError::HttpStatus(status) => {
            format!("The portal returned HTTP {status} while updating the launcher.")
        }
        _ => "Could not download the launcher update from the portal.".into(),
    }
}

pub(super) fn portal_root(portal: &str) -> Result<Url, String> {
    let mut root = Url::parse(portal).map_err(|_| "The portal address is invalid.")?;
    if root.scheme() != "https"
        || root.host_str().is_none()
        || !root.username().is_empty()
        || root.password().is_some()
        || root.query().is_some()
        || root.fragment().is_some()
    {
        return Err("The portal address is invalid.".into());
    }
    if !root.path().ends_with('/') {
        root.set_path(&format!("{}/", root.path()));
    }
    Ok(root)
}

pub(super) fn endpoint(root: &Url, path: &str) -> Result<Url, String> {
    let url = root
        .join(path)
        .map_err(|_| "The launcher update address is invalid.")?;
    if url.origin() != root.origin() {
        return Err("The launcher update must come from the configured portal.".into());
    }
    Ok(url)
}

fn prepare(
    portal: &str,
    auth: &dyn YoakeAuth,
    transport: &dyn FeedTransport,
    keys: &[PublicKey],
    current: &str,
    offered: &str,
    data_dir: &Path,
    state: &AtomicU8,
    progress: &mut dyn FnMut(f32),
) -> Result<Staged, String> {
    cancelled(state)?;
    let root = portal_root(portal)?;
    let authorization = auth
        .authorization()
        .map_err(|_| "Cannot read the portal sign-in. Please sign in again.")?
        .ok_or("Please sign in to the portal before updating the launcher.")?;
    let get = |path: &str, limit| -> Result<Vec<u8>, String> {
        cancelled(state)?;
        let url = endpoint(&root, path)?;
        transport
            .get(url.as_str(), Some(&authorization), limit)
            .map_err(transport_error)
    };
    let bytes = get("yoake/launcher/latest.json", MAX_METADATA)?;
    let latest: Latest = serde_json::from_slice(&bytes)
        .map_err(|_| "The portal's launcher update information is invalid.")?;
    let version =
        Version::parse(&latest.version).map_err(|_| "The portal's launcher version is invalid.")?;
    let installed = Version::parse(current).map_err(|_| "This launcher's version is invalid.")?;
    if latest.version != offered
        || version.cmp_precedence(&installed) != std::cmp::Ordering::Greater
        || latest.tag != format!("yoake-launcher-{}", latest.version)
        || latest.file != format!("YoakeLauncher-{}.exe", latest.version)
        || latest.size == 0
        || latest.size > MAX_LAUNCHER
        || latest.sha256.len() != 64
        || !latest.sha256.bytes().all(|byte| byte.is_ascii_hexdigit())
    {
        return Err(
            "The portal's launcher update information is invalid or no longer current.".into(),
        );
    }
    let base = format!("yoake/launcher/{}/", latest.version);
    let sums = get(&format!("{base}SHA256SUMS"), MAX_SUMS)?;
    let signature = get(&format!("{base}SHA256SUMS.sig"), MAX_SIGNATURE)?;
    let signature =
        std::str::from_utf8(&signature).map_err(|_| "The launcher signature is invalid.")?;
    let entries = verify(&latest.tag, &sums, signature, keys)
        .map_err(|_| "The launcher signature could not be verified. Nothing was installed.")?;
    if entries.len() != 1
        || entries[0].0 != latest.file
        || !entries[0].1.eq_ignore_ascii_case(&latest.sha256)
    {
        return Err("The signed launcher checksum does not match the update information.".into());
    }
    cancelled(state)?;
    let directory = data_dir.join(".launcher-update");
    fs::create_dir_all(&directory).map_err(|_| "Cannot create the launcher update folder.")?;
    let metadata = fs::symlink_metadata(&directory)
        .map_err(|_| "Cannot inspect the launcher update folder.")?;
    if metadata.file_type().is_symlink() || !metadata.is_dir() {
        return Err("The launcher update folder is not a safe directory.".into());
    }
    let path = directory.join(format!(
        "download-{}-{}.tmp",
        std::process::id(),
        NEXT_TEMP.fetch_add(1, Ordering::Relaxed)
    ));
    let staged;
    let mut file = OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(&path)
        .map_err(|_| "Cannot create the launcher download file.")?;
    staged = Staged(path);
    let url = endpoint(&root, &format!("yoake/launcher/{}", latest.file))?;
    let mut reader = transport
        .open(url.as_str(), Some(&authorization), latest.size)
        .map_err(transport_error)?;
    let mut hash = Sha256::default();
    let mut received = 0u64;
    let mut buffer = [0u8; 64 * 1024];
    loop {
        cancelled(state)?;
        let read_limit =
            ((latest.size - received).saturating_add(1)).min(buffer.len() as u64) as usize;
        let count = reader
            .read(&mut buffer[..read_limit])
            .map_err(|_| "The launcher download was interrupted.")?;
        if count == 0 {
            break;
        }
        received += count as u64;
        if received > latest.size {
            return Err("The launcher update is larger than expected.".into());
        }
        hash.update(&buffer[..count]);
        file.write_all(&buffer[..count])
            .map_err(|_| "Cannot save the launcher download.")?;
        progress(received as f32 / latest.size as f32);
    }
    if received != latest.size || !hash.hex().eq_ignore_ascii_case(&entries[0].1) {
        return Err(
            "The launcher download does not match its signed checksum. Nothing was installed."
                .into(),
        );
    }
    file.sync_all()
        .map_err(|_| "Cannot save the launcher download.")?;
    drop(file);
    cancelled(state)?;
    Ok(staged)
}

pub(super) fn install(
    portal: &str,
    auth: &dyn YoakeAuth,
    transport: &dyn FeedTransport,
    keys: &[PublicKey],
    current: &str,
    offered: &str,
    data_dir: &Path,
    state: &AtomicU8,
    progress: &mut dyn FnMut(f32),
) -> Result<(), String> {
    if horizon_launcher::brand::store_distribution() {
        return Err(horizon_launcher::i18n::tr("The Microsoft Store updates Yoake.").to_owned());
    }
    if !cfg!(windows) {
        return Err("The portal currently provides a Windows launcher update only.".into());
    }
    let staged = prepare(
        portal, auth, transport, keys, current, offered, data_dir, state, progress,
    )?;
    let lock = OpenOptions::new()
        .read(true)
        .write(true)
        .create(true)
        .truncate(false)
        .open(data_dir.join(".launcher-update").join("replace.lock"))
        .map_err(|_| "Cannot coordinate launcher updates.")?;
    loop {
        cancelled(state)?;
        match lock.try_lock() {
            Ok(()) => break,
            Err(std::fs::TryLockError::WouldBlock) => {
                std::thread::sleep(std::time::Duration::from_millis(100));
            }
            Err(std::fs::TryLockError::Error(_)) => {
                return Err("Cannot coordinate launcher updates.".into());
            }
        }
    }
    cancelled(state)?;
    let target = horizon_launcher::backend::update_target();
    let installed = horizon_launcher::backend::installed_launcher_version(&target);
    cancelled(state)?;
    state
        .compare_exchange(0, 2, Ordering::AcqRel, Ordering::Acquire)
        .map_err(|_| "The launcher update was cancelled.")?;
    if already_installed(current, offered, installed.as_deref()) {
        progress(1.0);
        return Ok(());
    }
    horizon_launcher::backend::replace_binary(&staged.0, &target)
        .map_err(|error| format!("Cannot replace the launcher: {error}"))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::yoake_auth::AuthError;
    use horizon_release_sign::{SigningKey, sha256_hex, sums_file};
    use std::collections::HashMap;
    use std::io::Cursor;
    use std::sync::Mutex;

    const PORTAL: &str = "https://portal.example/base/";
    const VERSION: &str = "1.2.4";
    const FILE: &str = "YoakeLauncher-1.2.4.exe";
    const TAG: &str = "yoake-launcher-1.2.4";

    struct Auth;

    impl YoakeAuth for Auth {
        fn authorization(&self) -> Result<Option<String>, AuthError> {
            Ok(Some("Bearer saved-token".into()))
        }
    }

    struct FakeTransport {
        objects: HashMap<String, Vec<u8>>,
        unauthorized: bool,
        requests: Mutex<Vec<(String, Option<String>)>>,
    }

    impl FeedTransport for FakeTransport {
        fn get(
            &self,
            url: &str,
            authorization: Option<&str>,
            limit: usize,
        ) -> Result<Vec<u8>, FeedError> {
            self.requests
                .lock()
                .unwrap()
                .push((url.to_string(), authorization.map(str::to_string)));
            if self.unauthorized {
                return Err(FeedError::NeedsLogin);
            }
            let bytes = self.objects.get(url).ok_or(FeedError::Network)?;
            if bytes.len() > limit {
                return Err(FeedError::TooLarge);
            }
            Ok(bytes.clone())
        }

        fn open(
            &self,
            url: &str,
            authorization: Option<&str>,
            _limit: u64,
        ) -> Result<Box<dyn Read + Send>, FeedError> {
            Ok(Box::new(Cursor::new(self.get(
                url,
                authorization,
                usize::MAX,
            )?)))
        }
    }

    struct Fixture {
        key: SigningKey,
        transport: FakeTransport,
        directory: PathBuf,
    }

    impl Fixture {
        fn new(payload: &[u8]) -> Self {
            let (key, _) = SigningKey::generate().unwrap();
            let hash = sha256_hex(payload);
            let sums = sums_file(&[(FILE.into(), hash.clone())]);
            let latest = serde_json::json!({
                "version": VERSION,
                "tag": TAG,
                "file": FILE,
                "sha256": hash,
                "size": payload.len(),
            });
            let objects = HashMap::from([
                (
                    format!("{PORTAL}yoake/launcher/latest.json"),
                    latest.to_string().into_bytes(),
                ),
                (
                    format!("{PORTAL}yoake/launcher/{VERSION}/SHA256SUMS"),
                    sums.as_bytes().to_vec(),
                ),
                (
                    format!("{PORTAL}yoake/launcher/{VERSION}/SHA256SUMS.sig"),
                    key.sign(TAG, sums.as_bytes()).into_bytes(),
                ),
                (format!("{PORTAL}yoake/launcher/{FILE}"), payload.to_vec()),
            ]);
            let directory = std::env::temp_dir().join(format!(
                "yoake-launcher-update-test-{}-{}",
                std::process::id(),
                NEXT_TEMP.fetch_add(1, Ordering::Relaxed)
            ));
            Self {
                key,
                transport: FakeTransport {
                    objects,
                    unauthorized: false,
                    requests: Mutex::new(Vec::new()),
                },
                directory,
            }
        }

        fn prepare(
            &self,
            current: &str,
            keys: &[PublicKey],
            state: &AtomicU8,
            progress: &mut dyn FnMut(f32),
        ) -> Result<Staged, String> {
            prepare(
                PORTAL,
                &Auth,
                &self.transport,
                keys,
                current,
                VERSION,
                &self.directory,
                state,
                progress,
            )
        }
    }

    impl Drop for Fixture {
        fn drop(&mut self) {
            let _ = fs::remove_dir_all(&self.directory);
        }
    }

    #[test]
    fn accepts_signed_release_and_authenticates_every_request() {
        let fixture = Fixture::new(b"signed launcher");
        let mut progress_values = Vec::new();
        let staged = fixture
            .prepare(
                "1.2.3",
                &[fixture.key.public()],
                &AtomicU8::new(0),
                &mut |value| progress_values.push(value),
            )
            .ok()
            .unwrap();
        assert_eq!(fs::read(&staged.0).unwrap(), b"signed launcher");
        assert_eq!(progress_values.last(), Some(&1.0));
        let requests = fixture.transport.requests.lock().unwrap();
        assert_eq!(requests.len(), 4);
        assert!(
            requests.iter().all(|(url, auth)| url.starts_with(PORTAL)
                && auth.as_deref() == Some("Bearer saved-token"))
        );
        drop(requests);
        let path = staged.0.clone();
        drop(staged);
        assert!(!path.exists());
    }

    #[test]
    fn rejects_wrong_key_and_wrong_signed_tag() {
        let mut fixture = Fixture::new(b"launcher");
        let (other, _) = SigningKey::generate().unwrap();
        let error = fixture
            .prepare("1.2.3", &[other.public()], &AtomicU8::new(0), &mut |_| {})
            .err()
            .unwrap();
        assert!(error.contains("signature"));
        let sums = fixture.transport.objects
            [&format!("{PORTAL}yoake/launcher/{VERSION}/SHA256SUMS")]
            .clone();
        fixture.transport.objects.insert(
            format!("{PORTAL}yoake/launcher/{VERSION}/SHA256SUMS.sig"),
            fixture.key.sign("yoake-launcher-1.2.3", &sums).into_bytes(),
        );
        let error = fixture
            .prepare(
                "1.2.3",
                &[fixture.key.public()],
                &AtomicU8::new(0),
                &mut |_| {},
            )
            .err()
            .unwrap();
        assert!(error.contains("signature"));
    }

    #[test]
    fn does_not_replace_a_newer_installed_launcher() {
        assert!(already_installed("1.2.3", "1.2.4", Some("1.2.5")));
        assert!(already_installed("1.2.3", "1.2.4", Some("1.2.4")));
        assert!(!already_installed("1.2.3", "1.2.4", Some("1.2.3")));
        assert!(!already_installed("1.2.3", "1.2.4", None));
    }

    #[test]
    fn rejects_older_or_equal_version_before_downloading() {
        let fixture = Fixture::new(b"launcher");
        for current in ["1.2.4", "1.2.5"] {
            let error = fixture
                .prepare(
                    current,
                    &[fixture.key.public()],
                    &AtomicU8::new(0),
                    &mut |_| {},
                )
                .err()
                .unwrap();
            assert!(error.contains("no longer current"));
        }
        assert_eq!(fixture.transport.requests.lock().unwrap().len(), 2);
    }

    #[test]
    fn rejects_hash_mismatch_and_oversize() {
        let mut fixture = Fixture::new(b"launcher");
        let url = format!("{PORTAL}yoake/launcher/{FILE}");
        fixture
            .transport
            .objects
            .insert(url.clone(), b"tampered".to_vec());
        let error = fixture
            .prepare(
                "1.2.3",
                &[fixture.key.public()],
                &AtomicU8::new(0),
                &mut |_| {},
            )
            .err()
            .unwrap();
        assert!(error.contains("checksum"));
        fixture.transport.objects.insert(url, b"too large".to_vec());
        let error = fixture
            .prepare(
                "1.2.3",
                &[fixture.key.public()],
                &AtomicU8::new(0),
                &mut |_| {},
            )
            .err()
            .unwrap();
        assert!(error.contains("larger"));
        assert_eq!(
            fs::read_dir(fixture.directory.join(".launcher-update"))
                .unwrap()
                .count(),
            0
        );
    }

    #[test]
    fn reports_unauthorized_without_downloading() {
        let mut fixture = Fixture::new(b"launcher");
        fixture.transport.unauthorized = true;
        let error = fixture
            .prepare(
                "1.2.3",
                &[fixture.key.public()],
                &AtomicU8::new(0),
                &mut |_| {},
            )
            .err()
            .unwrap();
        assert!(error.contains("sign in"));
        assert!(!fixture.directory.exists());
    }

    #[test]
    fn cancellation_removes_partial_download() {
        let fixture = Fixture::new(&vec![42; 128 * 1024]);
        let state = AtomicU8::new(0);
        let error = fixture
            .prepare("1.2.3", &[fixture.key.public()], &state, &mut |progress| {
                if progress > 0.0 {
                    state.store(1, Ordering::Release);
                }
            })
            .err()
            .unwrap();
        assert!(error.contains("cancelled"));
        assert_eq!(
            fs::read_dir(fixture.directory.join(".launcher-update"))
                .unwrap()
                .count(),
            0
        );
    }
}
