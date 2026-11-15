use std::collections::BTreeMap;
use super::*;
use crate::yoake_auth::AuthError;
use horizon_mods::yak::{Bin, CatalogEntry, Side, Yoake};
use horizon_release_sign::{SigningKey, YOAKE_MODS_V1};
use std::sync::{
    atomic::{AtomicUsize, Ordering},
    Mutex,
};

const ROOT: &str = "https://portal.yunipop.com/";
const INDEX: &str = "https://portal.yunipop.com/yoake/mods/index.json";
static NEXT_ROOT: AtomicUsize = AtomicUsize::new(0);

struct Auth(Option<String>);

impl YoakeAuth for Auth {
    fn authorization(&self) -> Result<Option<String>, AuthError> {
        Ok(self.0.clone())
    }
}

#[derive(Default)]
pub(super) struct Http {
    responses: Mutex<BTreeMap<String, Result<Vec<u8>, FeedError>>>,
    pub(super) requests: Mutex<Vec<(String, Option<String>)>>,
}

impl Http {
    pub(super) fn respond(&self, url: &str, response: Result<Vec<u8>, FeedError>) {
        self.responses
            .lock()
            .unwrap()
            .insert(url.to_owned(), response);
    }
}

impl FeedTransport for Http {
    fn get(
        &self,
        url: &str,
        authorization: Option<&str>,
        limit: usize,
    ) -> Result<Vec<u8>, FeedError> {
        self.requests
            .lock()
            .unwrap()
            .push((url.to_owned(), authorization.map(str::to_owned)));
        let bytes = self
            .responses
            .lock()
            .unwrap()
            .get(url)
            .cloned()
            .unwrap_or(Err(FeedError::HttpStatus(404)))?;
        if bytes.len() > limit {
            return Err(FeedError::TooLarge);
        }
        Ok(bytes)
    }
}

pub(super) struct Fixture {
    root: PathBuf,
    temp: PathBuf,
    next_file: AtomicUsize,
    pub(super) home: PathBuf,
    pub(super) client: Client,
    pub(super) http: Arc<Http>,
    pub(super) key: SigningKey,
    pub(super) environment: InstanceEnvironment,
}

pub(super) struct Package {
    pub(super) path: PathBuf,
    pub(super) version: CatalogVersion,
    pub(super) bytes: Vec<u8>,
}

impl Fixture {
    pub(super) fn new() -> Self {
        Self::with_token(Some("Bearer member-token"))
    }

    pub(super) fn with_token(token: Option<&str>) -> Self {
        let temp = std::env::temp_dir().canonicalize().unwrap();
        let nonce = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        let root = temp.join(format!(
            "yoake-yak-test-{}-{nonce}-{}",
            std::process::id(),
            NEXT_ROOT.fetch_add(1, Ordering::Relaxed)
        ));
        std::fs::create_dir(&root).unwrap();
        let root = root.canonicalize().unwrap();
        let home = root.join("instance");
        std::fs::create_dir(&home).unwrap();
        let (key, _) = SigningKey::generate().unwrap();
        let http = Arc::new(Http::default());
        let client = Client::with_components(
            ROOT,
            vec![key.public()],
            root.join("data"),
            Arc::new(Auth(token.map(str::to_owned))),
            http.clone(),
        )
        .unwrap();
        Self {
            root,
            temp,
            next_file: AtomicUsize::new(0),
            home,
            client,
            http,
            key,
            environment: InstanceEnvironment {
                yoake: "1.0.0".parse().unwrap(),
                engine: "1.0.0+build.0123456789abcdef".into(),
                abi: 2,
                target: Target::host().unwrap(),
            },
        }
    }

    pub(super) fn manifest(&self, id: &str, version: &str) -> Manifest {
        Manifest {
            format: 1,
            id: id.into(),
            version: version.parse().unwrap(),
            name: format!("Test {id}"),
            name_ja: Some(format!("テスト {id}")),
            authors: vec!["Fixture author".into()],
            description: "A launcher test fixture, never executable.".into(),
            license: Some("MIT".into()),
            homepage: None,
            yoake: Yoake {
                version: ">=1, <2".parse().unwrap(),
            },
            side: Side {
                client: true,
                server: true,
                required_on_join: false,
            },
            dependencies: BTreeMap::new(),
            optional: BTreeMap::new(),
            conflicts: BTreeMap::new(),
            bin: None,
            wasm: None,
            http: None,
            shaders: Vec::new(),
            services: Vec::new(),
            config: BTreeMap::new(),
            permissions: BTreeMap::new(),
        }
    }

    pub(super) fn program(&self, id: &str) -> Manifest {
        let mut manifest = self.manifest(id, "1.0.0");
        manifest.bin = Some(Bin {
            library: "fixture".into(),
            targets: vec![self.environment.target],
            engine: self.environment.engine.clone(),
            abi: self.environment.abi,
        });
        manifest
    }

    pub(super) fn package(&self, manifest: Manifest, signed: bool) -> Package {
        let serial = self.next_file.fetch_add(1, Ordering::Relaxed);
        let source = self.root.join(format!("source-{serial}"));
        std::fs::create_dir(&source).unwrap();
        std::fs::write(source.join("yak.toml"), toml::to_string(&manifest).unwrap()).unwrap();
        std::fs::write(source.join("README.md"), b"Headless launcher fixture").unwrap();
        let mut binaries = BTreeMap::new();
        if let Some(bin) = &manifest.bin {
            let stub = source.join("fixture.bin");
            std::fs::write(&stub, b"Fixture bytes; never load this library").unwrap();
            for target in &bin.targets {
                binaries.insert(*target, stub.clone());
            }
        }
        let path = self.root.join(format!("local-{serial}.yak"));
        yak::pack_directory(&source, &path, &binaries).unwrap();
        if signed {
            horizon_release_sign::yak::sign_file(&path, &self.key).unwrap();
        }
        let bytes = std::fs::read(&path).unwrap();
        let url = format!(
            "{ROOT}yoake/mods/{}/{}/{}-{}.yak",
            manifest.id, manifest.version, manifest.id, manifest.version
        );
        let version = CatalogVersion {
            version: manifest.version.clone(),
            url: url.clone(),
            size: bytes.len() as u64,
            sha256: horizon_release_sign::sha256_hex(&bytes),
            targets: manifest
                .bin
                .as_ref()
                .map(|bin| bin.targets.clone())
                .unwrap_or_default(),
            engine: manifest.bin.as_ref().map(|bin| bin.engine.clone()),
            manifest,
        };
        self.http.respond(&url, Ok(bytes.clone()));
        Package {
            path,
            version,
            bytes,
        }
    }

    pub(super) fn publish(&self, versions: &[CatalogVersion]) -> VerifiedCatalog {
        let mut entries = BTreeMap::<String, Vec<CatalogVersion>>::new();
        for version in versions {
            entries
                .entry(version.manifest.id.clone())
                .or_default()
                .push(version.clone());
        }
        let catalog = Catalog {
            schema: 1,
            generated: "2026-11-13T00:00:00Z".into(),
            entries: entries
                .into_iter()
                .map(|(id, versions)| CatalogEntry { id, versions })
                .collect(),
        };
        let json = serde_json::to_vec(&catalog).unwrap();
        self.http.respond(INDEX, Ok(json.clone()));
        self.http.respond(
            &format!("{INDEX}.sig"),
            Ok(self.key.sign(YOAKE_MODS_V1, &json).into_bytes()),
        );
        self.client.refresh().unwrap()
    }

    pub(super) fn fail_index(&self, error: FeedError) {
        self.http.respond(INDEX, Err(error));
    }

    pub(super) fn plan(&self, catalog: &VerifiedCatalog, id: &str) -> Result<InstallPlan, Error> {
        self.client.plan_for(
            &self.home,
            Some(catalog),
            &[(id.into(), None)],
            false,
            None,
            self.environment.clone(),
        )
    }

    pub(super) fn apply(&self, plan: InstallPlan, confirmed: bool) -> Result<(), Error> {
        self.apply_with_dev(plan, confirmed, false)
    }

    pub(super) fn apply_with_dev(
        &self,
        plan: InstallPlan,
        confirmed: bool,
        dev_unsigned_dll: bool,
    ) -> Result<(), Error> {
        self.client
            .apply_for(plan, confirmed, self.environment.clone(), dev_unsigned_dll)
    }

    pub(super) fn local(&self, path: &Path) -> Result<InstallPlan, Error> {
        self.local_with_dev(path, false)
    }

    pub(super) fn local_with_dev(
        &self,
        path: &Path,
        dev_unsigned_dll: bool,
    ) -> Result<InstallPlan, Error> {
        self.client.local_for(
            &self.home,
            path,
            None,
            self.environment.clone(),
            dev_unsigned_dll,
        )
    }

    pub(super) fn list(&self) -> String {
        std::fs::read_to_string(self.home.join("yoake-mods.txt")).unwrap()
    }
}

impl Drop for Fixture {
    fn drop(&mut self) {
        if self.root.canonicalize().ok().as_ref() == Some(&self.root)
            && self.root.parent() == Some(self.temp.as_path())
            && self
                .root
                .file_name()
                .and_then(|name| name.to_str())
                .is_some_and(|name| name.starts_with("yoake-yak-test-"))
            && yak::check_installed_tree(&self.root).is_ok()
        {
            let _ = std::fs::remove_dir_all(&self.root);
        }
    }
}
