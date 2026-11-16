use crate::yoake_auth::{TokenFileAuth, YoakeAuth};
use crate::yoake_feed::{FeedError, FeedTransport, HttpTransport};
use horizon_mods::yak::{
    self, Archive, Catalog, CatalogVersion, Manifest, PublicKey, Target, Version,
};
use serde::{Deserialize, Serialize};
use std::fs::File;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use url::Url;

mod plan;
mod store;
#[cfg(test)]
mod builtin_tests;
pub use plan::{InstallPlan, Installed};

#[derive(Debug)]
pub enum Error {
    Feed(FeedError),
    Invalid(String),
    Incompatible(String),
    Resolution(String),
    Storage(String),
    NotYoake,
    Busy,
    Stale,
    ConfirmationRequired,
    UnsignedProgram,
}

impl std::fmt::Display for Error {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Feed(error) => error.fmt(formatter),
            Self::Invalid(reason)
            | Self::Incompatible(reason)
            | Self::Resolution(reason)
            | Self::Storage(reason) => formatter.write_str(reason),
            Self::NotYoake => formatter.write_str("This is not an installed Yoake instance"),
            Self::Busy => formatter.write_str("Another mod operation is in progress"),
            Self::Stale => {
                formatter.write_str("The instance or file changed; review the plan again")
            }
            Self::ConfirmationRequired => formatter.write_str("Confirm the mod changes first"),
            Self::UnsignedProgram => {
                formatter.write_str("A program .yak needs a valid Yoake SIGNATURE")
            }
        }
    }
}

impl std::error::Error for Error {}
impl From<FeedError> for Error {
    fn from(error: FeedError) -> Self {
        Self::Feed(error)
    }
}
impl From<std::io::Error> for Error {
    fn from(error: std::io::Error) -> Self {
        Self::Storage(error.to_string())
    }
}

#[derive(Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct Proof {
    json: String,
    signature: String,
}

#[derive(Clone)]
pub struct VerifiedCatalog {
    pub catalog: Catalog,
    pub cached: bool,
    proof: Proof,
}

#[derive(Clone)]
pub struct Client {
    root: Url,
    keys: Vec<PublicKey>,
    data: PathBuf,
    auth: Arc<dyn YoakeAuth>,
    http: Arc<dyn FeedTransport>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct InstanceEnvironment {
    pub yoake: Version,
    pub engine: String,
    pub abi: u32,
    pub target: Target,
}

impl InstanceEnvironment {
    pub fn read(home: &Path) -> Result<Self, Error> {
        let pack = crate::yoake_install::read_manifest(home)
            .map_err(|error| Error::Invalid(error.to_string()))?
            .ok_or(Error::NotYoake)?;
        let proof = pack.rollback_proof.as_ref().ok_or_else(|| {
            Error::Incompatible("The instance has no signed Yoake build identity".into())
        })?;
        let feed = crate::yoake_feed::verify_saved_feed(proof)?;
        let crate::yoake_feed::ParsedFeedIndex::V2(index) = feed.index else {
            return Err(Error::NotYoake);
        };
        let base = index
            .bases
            .iter()
            .find(|base| base.version == pack.base_version)
            .ok_or(Error::NotYoake)?;
        let version = index
            .packs
            .iter()
            .find(|entry| entry.id == pack.pack_id)
            .and_then(|entry| {
                entry
                    .versions
                    .iter()
                    .find(|version| version.version == pack.pack_version)
            })
            .ok_or(Error::NotYoake)?;
        if index.sequence != pack.sequence
            || version.base_version != pack.base_version
            || crate::yoake_install::current_target().as_deref() != Ok(pack.target.as_str())
            || !base.targets.contains_key(&pack.target)
        {
            return Err(Error::NotYoake);
        }
        Self::from_identity(base.yoake.as_ref())
    }

    fn from_identity(
        identity: Option<&crate::yoake_feed::YoakeBuildIdentity>,
    ) -> Result<Self, Error> {
        let identity = identity
            .filter(|identity| identity.is_valid())
            .ok_or_else(|| {
                Error::Incompatible(
                    "The instance has no signed Yoake version, engine fingerprint and ABI identity"
                        .into(),
                )
            })?;
        Ok(Self {
            yoake: Version::parse(&identity.version)
                .map_err(|error| Error::Invalid(error.to_string()))?,
            engine: identity.engine.clone(),
            abi: identity.abi,
            target: Target::host().map_err(Error::Incompatible)?,
        })
    }

    pub fn check(&self, manifest: &Manifest) -> Result<(), Error> {
        if !manifest.side.client {
            return Err(Error::Incompatible(format!(
                "{} is server-only",
                manifest.id
            )));
        }
        if manifest.bin.is_some() && self.engine.is_empty() {
            return Err(Error::Incompatible("The instance has no exact Yoake engine fingerprint; program mods cannot be installed".into()));
        }
        let issues = yak::check_compatibility(
            manifest,
            &yak::Environment {
                yoake: &self.yoake,
                engine: &self.engine,
                abi: self.abi,
                target: self.target,
            },
        );
        if issues.is_empty() {
            Ok(())
        } else {
            Err(Error::Incompatible(format!("{}: {issues:?}", manifest.id)))
        }
    }
}

impl Client {
    pub fn configured() -> Result<Self, Error> {
        let data = crate::backend::data_dir();
        let keys = yak::trusted_keys();
        Self::with_components(
            option_env!("YOAKE_PORTAL_URL").unwrap_or("https://portal.yunipop.com/"),
            keys,
            data.clone(),
            Arc::new(TokenFileAuth::new(&data)),
            Arc::new(HttpTransport),
        )
    }

    pub fn with_components(
        root: &str,
        keys: Vec<PublicKey>,
        data: PathBuf,
        auth: Arc<dyn YoakeAuth>,
        http: Arc<dyn FeedTransport>,
    ) -> Result<Self, Error> {
        let mut root = Url::parse(root).map_err(|_| FeedError::Configuration)?;
        if root.scheme() != "https"
            || root.host_str().is_none()
            || !root.username().is_empty()
            || root.password().is_some()
            || root.query().is_some()
            || root.fragment().is_some()
        {
            return Err(FeedError::Configuration.into());
        }
        if !root.path().ends_with('/') {
            root.set_path(&format!("{}/", root.path()));
        }
        Ok(Self {
            root,
            keys,
            data,
            auth,
            http,
        })
    }

    fn authorization(&self) -> Result<String, Error> {
        self.auth
            .authorization()
            .map_err(FeedError::Authentication)?
            .ok_or_else(|| FeedError::NeedsLogin.into())
    }

    fn verify(&self, proof: Proof, cached: bool) -> Result<VerifiedCatalog, Error> {
        if self.keys.is_empty() {
            return Err(FeedError::Configuration.into());
        }
        let catalog = Catalog::parse_verified(proof.json.as_bytes(), &proof.signature, &self.keys)
            .map_err(|_| FeedError::BadSignature)?;
        for entry in &catalog.entries {
            for version in &entry.versions {
                let url = Url::parse(&version.url).map_err(|_| FeedError::WrongSource)?;
                if url.origin() != self.root.origin()
                    || !url.username().is_empty()
                    || url.password().is_some()
                    || url.query().is_some()
                    || url.fragment().is_some()
                {
                    return Err(FeedError::WrongSource.into());
                }
            }
        }
        Ok(VerifiedCatalog {
            catalog,
            cached,
            proof,
        })
    }

    pub fn refresh(&self) -> Result<VerifiedCatalog, Error> {
        let auth = self.authorization()?;
        let fetch = || -> Result<VerifiedCatalog, Error> {
            let url = self
                .root
                .join("yoake/mods/index.json")
                .map_err(|_| FeedError::Configuration)?;
            let json = self
                .http
                .get(url.as_str(), Some(&auth), yak::MAX_CATALOG_SIZE as usize)?;
            let signature = self.http.get(&format!("{url}.sig"), Some(&auth), 4096)?;
            if json.len() > yak::MAX_CATALOG_SIZE as usize || signature.len() > 4096 {
                return Err(FeedError::TooLarge.into());
            }
            let proof = Proof {
                json: String::from_utf8(json).map_err(|_| FeedError::Malformed)?,
                signature: String::from_utf8(signature).map_err(|_| FeedError::Malformed)?,
            };
            self.verify(proof, false)
        };
        let cache = self.data.join("yoake-mods-catalog.json");
        match fetch() {
            Ok(catalog) => {
                store::write_json(&cache, &catalog.proof)?;
                Ok(catalog)
            }
            Err(error @ Error::Feed(FeedError::Network | FeedError::HttpStatus(500..=599))) => {
                store::plain(&cache)?;
                if !cache.try_exists()? {
                    return Err(error);
                }
                let proof = store::read_json(&cache)?;
                self.verify(proof, true)
            }
            Err(error) => Err(error),
        }
    }
}

/// What the built-in catalog refresh found. A problem never hides a catalog that is still good: the
/// last verified copy is returned with it, and the `.yak` catalog is independent of all of this.
pub struct BuiltinRefresh {
    pub catalog: Option<crate::yoake_configs::BuiltinCatalog>,
    pub problem: Option<Error>,
}

/// The built-in catalog as it is published: the exact catalog text and its signature in one file,
/// so an upload (one rename) can never pair a catalog with another one's signature.
#[derive(Deserialize)]
struct BuiltinEnvelope {
    json: String,
    signature: String,
}

impl Client {
    fn verify_builtin(
        &self,
        proof: &Proof,
    ) -> Result<crate::yoake_configs::BuiltinCatalog, Error> {
        use crate::yoake_configs::{BuiltinCatalog, Error as ConfigError};
        BuiltinCatalog::parse_verified(proof.json.as_bytes(), &proof.signature, &self.keys)
            .map_err(|error| match error {
                ConfigError::NeedsUpdate => Error::Incompatible(error.to_string()),
                other => Error::Invalid(other.to_string()),
            })
    }

    /// The last verified copy, if there is one.
    fn cached_builtin(
        &self,
        cache: &Path,
    ) -> Option<(crate::yoake_configs::BuiltinCatalog, Proof)> {
        store::plain(cache).ok()?;
        if !cache.try_exists().ok()? {
            return None;
        }
        let proof: Proof = store::read_json(cache).ok()?;
        let catalog = self.verify_builtin(&proof).ok()?;
        Some((catalog, proof))
    }

    fn fetch_builtin(
        &self,
    ) -> Result<Option<(crate::yoake_configs::BuiltinCatalog, Proof)>, Error> {
        use crate::yoake_configs::{BUILTIN_PATH, MAX_BUILTIN};
        let auth = self.authorization()?;
        let url = self
            .root
            .join(BUILTIN_PATH)
            .map_err(|_| FeedError::Configuration)?;
        let limit = 2 * MAX_BUILTIN + 8192;
        let bytes = match self.http.get(url.as_str(), Some(&auth), limit) {
            Ok(bytes) => bytes,
            Err(FeedError::HttpStatus(404)) => return Ok(None),
            Err(error) => return Err(error.into()),
        };
        if bytes.len() > limit {
            return Err(FeedError::TooLarge.into());
        }
        let envelope: BuiltinEnvelope =
            serde_json::from_slice(&bytes).map_err(|_| FeedError::Malformed)?;
        let proof = Proof {
            json: envelope.json,
            signature: envelope.signature,
        };
        Ok(Some((self.verify_builtin(&proof)?, proof)))
    }

    /// The signed built-in mods catalog (`yoake/mods/builtin/signed.json`). A server without one
    /// (HTTP 404) simply has no built-in entries. The catalog never goes backwards: one older
    /// than the cached copy is refused. Every failure keeps the last verified copy.
    pub fn refresh_builtin(&self) -> BuiltinRefresh {
        if self.keys.is_empty() {
            return BuiltinRefresh {
                catalog: None,
                problem: Some(FeedError::Configuration.into()),
            };
        }
        let cache = self.data.join("yoake-builtin-catalog.json");
        let cached = self.cached_builtin(&cache);
        let last_good = |problem: Option<Error>| BuiltinRefresh {
            catalog: cached.as_ref().map(|(catalog, _)| catalog.clone()),
            problem,
        };
        match self.fetch_builtin() {
            Ok(Some((catalog, proof))) => {
                if cached
                    .as_ref()
                    .is_some_and(|(old, _)| catalog.generated < old.generated)
                {
                    return last_good(Some(FeedError::Replay.into()));
                }
                if let Err(error) = store::write_json(&cache, &proof) {
                    return BuiltinRefresh {
                        catalog: Some(catalog),
                        problem: Some(error),
                    };
                }
                BuiltinRefresh {
                    catalog: Some(catalog),
                    problem: None,
                }
            }
            Ok(None) => BuiltinRefresh {
                catalog: None,
                problem: None,
            },
            // offline: the last copy is as good as it was
            Err(Error::Feed(FeedError::Network | FeedError::HttpStatus(500..=599)))
                if cached.is_some() =>
            {
                last_good(None)
            }
            Err(error) => last_good(Some(error)),
        }
    }
}

pub fn installed_yak_mods(instance: &Path) -> Result<Vec<(String, String)>, Error> {
    Ok(Client::configured()?
        .installed(instance)?
        .into_iter()
        .map(|installed| {
            (
                installed.manifest.id,
                installed.manifest.version.to_string(),
            )
        })
        .collect())
}

pub fn install_set(instance: &Path, requested: &[(String, String)]) -> Result<(), Error> {
    let client = Client::configured()?;
    let catalog = client.refresh()?;
    let plan = client.plan_set(instance, &catalog, requested)?;
    client.apply(plan, true, false)
}
