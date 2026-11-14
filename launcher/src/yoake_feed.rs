use crate::yoake_auth::{AuthError, TokenFileAuth, YoakeAuth};
use horizon_release_sign::{parse_keys, sha256_hex, verify, PublicKey};
use serde::{Deserialize, Serialize};
use std::collections::{BTreeMap, BTreeSet};
use std::fs::{self, OpenOptions};
use std::io::{self, Read, Write};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::Arc;
use std::time::Duration;
use url::Url;

const DOMAIN: &str = "yoake-index-v1";
const MAX_POINTER: usize = 16 * 1024;
const MAX_INDEX: usize = 1024 * 1024;
const MAX_SUMS: usize = 16 * 1024;
const MAX_SIGNATURE: usize = 8 * 1024;
/// The high-water file keeps the exact shape and size cap that every released launcher reads.
const MAX_STATE: usize = MAX_POINTER;
const MAX_SNAPSHOT: usize = 16 * MAX_INDEX + 8 * (MAX_SUMS + MAX_SIGNATURE);
const MAX_ICON: u64 = 8 * 1024 * 1024;
const MAX_FILE: u64 = 8 * 1024 * 1024 * 1024;
static NEXT_TEMP: AtomicU64 = AtomicU64::new(0);

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum FeedError {
    Configuration,
    NeedsLogin,
    Authentication(AuthError),
    HttpStatus(u16),
    Network,
    TooLarge,
    Malformed,
    NeedsUpdate,
    BadSignature,
    HashMismatch,
    WrongSource,
    Replay,
    UnsafePath,
    Storage,
}

impl std::fmt::Display for FeedError {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(formatter, "Yoake feed: {self:?}")
    }
}

impl std::error::Error for FeedError {}

#[derive(Clone)]
pub struct FeedConfig {
    root: Url,
    keys: Vec<PublicKey>,
    source_id: Option<String>,
}

impl FeedConfig {
    pub fn new(
        root: &str,
        keys: Vec<PublicKey>,
        source_id: Option<String>,
    ) -> Result<Self, FeedError> {
        let mut root = Url::parse(root).map_err(|_| FeedError::Configuration)?;
        if root.scheme() != "https"
            || root.host_str().is_none()
            || !root.username().is_empty()
            || root.password().is_some()
            || root.query().is_some()
            || root.fragment().is_some()
            || keys.is_empty()
            || source_id.as_ref().is_some_and(|id| !safe_component(id))
        {
            return Err(FeedError::Configuration);
        }
        if !root.path().ends_with('/') {
            let path = format!("{}/", root.path());
            root.set_path(&path);
        }
        Ok(Self {
            root,
            keys,
            source_id,
        })
    }

    pub fn compiled() -> Result<Option<Self>, FeedError> {
        match (
            option_env!("YOAKE_COMPILED_FEED_URL"),
            option_env!("YOAKE_COMPILED_PUBLIC_KEYS"),
        ) {
            (None, None) => Ok(None),
            (Some(root), Some(keys)) => {
                let keys =
                    parse_keys(&keys.replace(';', "\n")).map_err(|_| FeedError::Configuration)?;
                let source_id = option_env!("YOAKE_COMPILED_SOURCE_ID").map(str::to_owned);
                Ok(Some(Self::new(root, keys, source_id)?))
            }
            _ => Err(FeedError::Configuration),
        }
    }

    pub fn public_keys(&self) -> &[PublicKey] {
        &self.keys
    }
}

#[derive(Debug, Clone, Deserialize)]
pub struct FeedIndex {
    #[serde(deserialize_with = "deserialize_schema_major")]
    pub schema: u32,
    pub source_id: String,
    pub sequence: u64,
    pub packs: Vec<Pack>,
}

#[derive(Debug, Clone, Deserialize)]
pub struct Pack {
    pub id: String,
    pub name: String,
    pub icon: Object,
    pub versions: Vec<PackVersion>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Object {
    pub sha256: String,
    pub size: u64,
}

#[derive(Debug, Clone, Deserialize)]
pub struct PackVersion {
    pub version: String,
    pub minimum_launcher: String,
    pub changelog: String,
    pub base: Base,
    pub enabled_content_mods: Vec<String>,
    pub targets: BTreeMap<String, Target>,
}

#[derive(Debug, Clone, Deserialize)]
pub struct Base {
    pub horizon_release: String,
    pub research_lock_sha256: String,
    pub prepared_format: String,
}

#[derive(Debug, Clone, Deserialize)]
pub struct Target {
    pub files: Vec<PackFile>,
}

#[derive(Debug, Clone, Deserialize)]
pub struct PackFile {
    pub scope: Scope,
    pub policy: Option<Policy>,
    pub destination: String,
    pub sha256: String,
    pub size: u64,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum Scope {
    Version,
    Instance,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum Policy {
    Managed,
    DefaultOnCreate,
}

#[derive(Debug, Clone)]
pub enum ParsedFeedIndex {
    V1(FeedIndex),
    V2(FeedIndexV2),
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct FeedIndexV2 {
    #[serde(deserialize_with = "deserialize_schema_major")]
    pub schema: u32,
    pub source_id: String,
    pub sequence: u64,
    pub bases: Vec<BaseVersionV2>,
    pub packs: Vec<PackV2>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct BaseVersionV2 {
    pub version: String,
    pub horizon_release: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub yoake: Option<YoakeBuildIdentity>,
    pub research_lock_sha256: String,
    pub prepared_format: String,
    /// Native mods compiled into this base exe. They always run, so a pack
    /// version on this base must enable exactly this set.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub native_mods: Vec<String>,
    #[serde(deserialize_with = "deserialize_targets")]
    pub targets: BTreeMap<String, BaseTargetV2>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct YoakeBuildIdentity {
    pub version: String,
    pub engine: String,
    pub abi: u32,
}

impl YoakeBuildIdentity {
    pub fn is_valid(&self) -> bool {
        semver::Version::parse(&self.version).is_ok()
            && self.abi != 0
            && self
                .engine
                .strip_prefix(&format!("{}+build.", self.version))
                .is_some_and(|fingerprint| {
                    fingerprint.len() == 16
                        && fingerprint
                            .bytes()
                            .all(|byte| byte.is_ascii_digit() || (b'a'..=b'f').contains(&byte))
                })
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct BaseTargetV2 {
    pub files: Vec<BaseFileV2>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct BaseFileV2 {
    pub scope: Scope,
    pub destination: String,
    pub sha256: String,
    pub size: u64,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct PackV2 {
    pub id: String,
    pub name: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub category: Option<String>,
    pub icon: Object,
    pub versions: Vec<PackVersionV2>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct PackVersionV2 {
    pub version: String,
    pub minimum_launcher: String,
    pub description: String,
    pub changelog: String,
    pub base_version: String,
    pub enabled_native_mods: Vec<String>,
    pub enabled_content_mods: Vec<String>,
    #[serde(deserialize_with = "deserialize_targets")]
    pub targets: BTreeMap<String, PackTargetV2>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct PackTargetV2 {
    pub files: Vec<PackFileV2>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct PackFileV2 {
    pub scope: Scope,
    pub policy: Policy,
    pub destination: String,
    pub sha256: String,
    pub size: u64,
}

fn deserialize_targets<'de, D, T>(deserializer: D) -> Result<BTreeMap<String, T>, D::Error>
where
    D: serde::Deserializer<'de>,
    T: Deserialize<'de>,
{
    struct TargetVisitor<T>(std::marker::PhantomData<T>);

    impl<'de, T: Deserialize<'de>> serde::de::Visitor<'de> for TargetVisitor<T> {
        type Value = BTreeMap<String, T>;

        fn expecting(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
            formatter.write_str("a map of unique targets")
        }

        fn visit_map<A: serde::de::MapAccess<'de>>(
            self,
            mut entries: A,
        ) -> Result<Self::Value, A::Error> {
            let mut targets = BTreeMap::new();
            while let Some((name, target)) = entries.next_entry::<String, T>()? {
                if targets.insert(name, target).is_some() {
                    return Err(serde::de::Error::custom("duplicate target"));
                }
            }
            Ok(targets)
        }
    }

    deserializer.deserialize_map(TargetVisitor(std::marker::PhantomData))
}

#[derive(Debug, Clone)]
pub struct VerifiedFeed {
    pub index: ParsedFeedIndex,
    pub index_sha256: String,
    pub proof: Option<FeedProof>,
    pub presentation: Option<Presentation>,
    pub needs_update: bool,
}

#[derive(Debug, Clone, Deserialize)]
pub struct Presentation {
    #[serde(default)]
    pub news: Vec<NewsItem>,
    #[serde(default)]
    pub links: Vec<PresentationLink>,
    #[serde(default)]
    pub categories: Vec<PackCategory>,
    #[serde(default, rename = "feature_flags")]
    _feature_flags: BTreeMap<String, bool>,
}

#[derive(Debug, Clone, Deserialize)]
pub struct NewsItem {
    pub id: String,
    pub date: String,
    pub title: BTreeMap<String, String>,
    pub body: BTreeMap<String, String>,
    #[serde(default)]
    pub link: Option<String>,
}

#[derive(Debug, Clone, Deserialize)]
pub struct PresentationLink {
    pub label: BTreeMap<String, String>,
    pub url: String,
}

#[derive(Debug, Clone, Deserialize)]
pub struct PackCategory {
    pub id: String,
    pub label: BTreeMap<String, String>,
    pub order: i64,
    /// Pack ids in this category. The index is never extended with a category field, so launchers
    /// that only know the old index keep parsing it.
    #[serde(default)]
    pub packs: Vec<String>,
}

impl PackCategory {
    fn contains(&self, pack: &PackV2) -> bool {
        pack.category.as_deref() == Some(self.id.as_str()) || self.packs.contains(&pack.id)
    }
}

pub fn localized<'a>(translations: &'a BTreeMap<String, String>, language: &str) -> &'a str {
    translations
        .get(language)
        .filter(|translation| !translation.is_empty())
        .or_else(|| translations.get("en"))
        .map(String::as_str)
        .unwrap_or_default()
}

pub type PackGroup<'a> = (Option<&'a PackCategory>, Vec<(usize, &'a PackV2)>);

pub fn pack_groups<'a>(
    packs: &'a [PackV2],
    presentation: Option<&'a Presentation>,
) -> Vec<PackGroup<'a>> {
    let mut groups = Vec::new();
    if let Some(presentation) = presentation {
        let mut categories: Vec<_> = presentation.categories.iter().collect();
        categories.sort_by(|left, right| {
            left.order.cmp(&right.order).then_with(|| left.id.cmp(&right.id))
        });
        for category in categories {
            let members: Vec<_> = packs
                .iter()
                .enumerate()
                .filter(|(_, pack)| category.contains(pack))
                .collect();
            if !members.is_empty() {
                groups.push((Some(category), members));
            }
        }
    }
    let remaining: Vec<_> = packs
        .iter()
        .enumerate()
        .filter(|(_, pack)| {
            !groups
                .iter()
                .any(|(category, _)| category.is_some_and(|category| category.contains(pack)))
        })
        .collect();
    if !remaining.is_empty() {
        groups.push((None, remaining));
    }
    groups
}

/// The validated URL in its serialised form. That is what gets opened, never the raw string.
fn https_url(value: &str) -> Option<String> {
    Url::parse(value)
        .ok()
        .filter(|url| {
            url.scheme() == "https"
                && url.host_str().is_some()
                && url.username().is_empty()
                && url.password().is_none()
        })
        .map(String::from)
}

fn parse_presentation(bytes: &[u8]) -> Result<Presentation, FeedError> {
    let schema: Schema = serde_json::from_slice(bytes).map_err(|_| FeedError::Malformed)?;
    schema.check(1)?;
    let mut presentation: Presentation =
        serde_json::from_slice(bytes).map_err(|_| FeedError::Malformed)?;
    let mut news_ids = BTreeSet::new();
    for item in &mut presentation.news {
        insert_unique(&mut news_ids, &item.id)?;
        if let Some(link) = &mut item.link {
            *link = https_url(link).ok_or(FeedError::Malformed)?;
        }
        if item.date.is_empty()
            || item.title.get("en").is_none_or(String::is_empty)
            || item.body.get("en").is_none_or(String::is_empty)
        {
            return Err(FeedError::Malformed);
        }
    }
    for link in &mut presentation.links {
        if link.label.get("en").is_none_or(String::is_empty) {
            return Err(FeedError::Malformed);
        }
        link.url = https_url(&link.url).ok_or(FeedError::Malformed)?;
    }
    let mut category_ids = BTreeSet::new();
    for category in &presentation.categories {
        insert_unique(&mut category_ids, &category.id)?;
        if category.label.get("en").is_none_or(String::is_empty) {
            return Err(FeedError::Malformed);
        }
    }
    Ok(presentation)
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct FeedProof {
    pub index_json: String,
    pub sums: String,
    pub signature: String,
}

/// `presentation.json` with its own signed checksum file. It lives next to the index but is not
/// listed in the index's `SHA256SUMS`, so launchers that only know the index keep working.
/// `presentation.sums` lists the verified index (`index.json`) and `presentation.json`, so a
/// presentation only ever belongs to the one signed index it was published with.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct PresentationProof {
    pub json: String,
    pub sums: String,
    pub signature: String,
}

fn verify_presentation(
    config: &FeedConfig,
    proof: &PresentationProof,
    index_sha256: &str,
) -> Result<Presentation, FeedError> {
    if proof.json.len() > MAX_INDEX
        || proof.sums.len() > MAX_SUMS
        || proof.signature.len() > MAX_SIGNATURE
    {
        return Err(FeedError::TooLarge);
    }
    let entries = verify(DOMAIN, proof.sums.as_bytes(), &proof.signature, &config.keys)
        .map_err(|_| FeedError::BadSignature)?;
    let [(first_name, first), (second_name, second)] = entries.as_slice() else {
        return Err(FeedError::Malformed);
    };
    let (index, presentation) = match (first_name.as_str(), second_name.as_str()) {
        ("index.json", "presentation.json") => (first, second),
        ("presentation.json", "index.json") => (second, first),
        _ => return Err(FeedError::Malformed),
    };
    if index != index_sha256 {
        return Err(FeedError::WrongSource);
    }
    if *presentation != sha256_hex(proof.json.as_bytes()) {
        return Err(FeedError::HashMismatch);
    }
    parse_presentation(proof.json.as_bytes())
}

pub fn verify_saved_feed(proof: &FeedProof) -> Result<VerifiedFeed, FeedError> {
    let config = FeedConfig::compiled()?.ok_or(FeedError::Configuration)?;
    if proof.index_json.len() > MAX_INDEX
        || proof.sums.len() > MAX_SUMS
        || proof.signature.len() > MAX_SIGNATURE
    {
        return Err(FeedError::TooLarge);
    }
    let entries = verify(
        DOMAIN,
        proof.sums.as_bytes(),
        &proof.signature,
        &config.keys,
    )
    .map_err(|_| FeedError::BadSignature)?;
    let index_digest = signed_digests(&entries)?;
    let index_sha256 = sha256_hex(proof.index_json.as_bytes());
    if index_digest != index_sha256 {
        return Err(FeedError::HashMismatch);
    }
    let schema: Schema = serde_json::from_str(&proof.index_json).map_err(|_| FeedError::Malformed)?;
    if schema.check(2)? != 2 {
        return Err(FeedError::Malformed);
    }
    let index: FeedIndexV2 =
        serde_json::from_str(&proof.index_json).map_err(|_| FeedError::Malformed)?;
    validate_index_v2(&index)?;
    if config
        .source_id
        .as_ref()
        .is_some_and(|source_id| source_id != &index.source_id)
    {
        return Err(FeedError::WrongSource);
    }
    Ok(VerifiedFeed {
        index: ParsedFeedIndex::V2(index),
        index_sha256,
        proof: Some(proof.clone()),
        presentation: None,
        needs_update: false,
    })
}

#[derive(Deserialize)]
#[serde(untagged)]
enum SchemaVersion {
    Legacy(u32),
    Version(String),
}

impl SchemaVersion {
    fn major(&self) -> Result<u32, FeedError> {
        match self {
            Self::Legacy(major) if *major > 0 => Ok(*major),
            Self::Version(version) => {
                let (major, minor) = version.split_once('.').ok_or(FeedError::Malformed)?;
                if major.is_empty()
                    || minor.is_empty()
                    || !major.bytes().all(|byte| byte.is_ascii_digit())
                    || !minor.bytes().all(|byte| byte.is_ascii_digit())
                {
                    return Err(FeedError::Malformed);
                }
                let major = major.parse::<u32>().map_err(|_| FeedError::Malformed)?;
                if major == 0 {
                    return Err(FeedError::Malformed);
                }
                Ok(major)
            }
            _ => Err(FeedError::Malformed),
        }
    }
}

fn deserialize_schema_major<'de, D>(deserializer: D) -> Result<u32, D::Error>
where
    D: serde::Deserializer<'de>,
{
    SchemaVersion::deserialize(deserializer)?
        .major()
        .map_err(serde::de::Error::custom)
}

#[derive(Deserialize)]
struct Schema {
    schema: SchemaVersion,
    #[serde(default)]
    critical_features: Vec<String>,
}

impl Schema {
    fn check(&self, maximum: u32) -> Result<u32, FeedError> {
        let major = self.schema.major()?;
        if major > maximum || !self.critical_features.is_empty() {
            return Err(FeedError::NeedsUpdate);
        }
        Ok(major)
    }
}

fn signed_digests(entries: &[(String, String)]) -> Result<&str, FeedError> {
    match entries {
        [(name, digest)] if name == "index.json" => Ok(digest.as_str()),
        _ => Err(FeedError::Malformed),
    }
}

fn index_identity(index: &ParsedFeedIndex) -> (&str, u64) {
    match index {
        ParsedFeedIndex::V1(index) => (&index.source_id, index.sequence),
        ParsedFeedIndex::V2(index) => (&index.source_id, index.sequence),
    }
}

fn parse_verified_feed(config: &FeedConfig, proof: FeedProof) -> Result<VerifiedFeed, FeedError> {
    if proof.index_json.len() > MAX_INDEX
        || proof.sums.len() > MAX_SUMS
        || proof.signature.len() > MAX_SIGNATURE
    {
        return Err(FeedError::TooLarge);
    }
    let entries = verify(DOMAIN, proof.sums.as_bytes(), &proof.signature, &config.keys)
        .map_err(|_| FeedError::BadSignature)?;
    let index_digest = signed_digests(&entries)?;
    let index_sha256 = sha256_hex(proof.index_json.as_bytes());
    if index_sha256 != index_digest {
        return Err(FeedError::HashMismatch);
    }
    let schema: Schema = serde_json::from_str(&proof.index_json).map_err(|_| FeedError::Malformed)?;
    let index = match schema.check(2)? {
        1 => {
            let index: FeedIndex =
                serde_json::from_str(&proof.index_json).map_err(|_| FeedError::Malformed)?;
            validate_index(&index)?;
            ParsedFeedIndex::V1(index)
        }
        2 => {
            let index: FeedIndexV2 =
                serde_json::from_str(&proof.index_json).map_err(|_| FeedError::Malformed)?;
            validate_index_v2(&index)?;
            ParsedFeedIndex::V2(index)
        }
        _ => return Err(FeedError::Malformed),
    };
    if config
        .source_id
        .as_deref()
        .is_some_and(|source_id| source_id != index_identity(&index).0)
    {
        return Err(FeedError::WrongSource);
    }
    Ok(VerifiedFeed {
        index,
        index_sha256,
        proof: Some(proof),
        presentation: None,
        needs_update: false,
    })
}

#[derive(Deserialize)]
struct Pointer {
    source_id: String,
    sequence: u64,
}

/// Exactly the three fields (and the 16 KiB cap) of the released launchers' high-water file, so a
/// downgrade or a second build (Store next to portable) keeps reading it.
#[derive(Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
struct HighWater {
    source_id: String,
    sequence: u64,
    index_sha256: String,
}

/// The last compatible signed feed, in a file of its own (`<feed id>.snapshot.json`) that
/// released launchers never open. Unknown fields are ignored.
#[derive(Deserialize, Serialize)]
struct Snapshot {
    source_id: String,
    sequence: u64,
    index_sha256: String,
    proof: FeedProof,
    #[serde(default)]
    presentation: Option<PresentationProof>,
}

#[derive(Deserialize)]
struct IndexIdentity {
    source_id: String,
    sequence: u64,
}

pub trait FeedTransport: Send + Sync {
    fn get(
        &self,
        url: &str,
        authorization: Option<&str>,
        limit: usize,
    ) -> Result<Vec<u8>, FeedError>;

    fn open(
        &self,
        url: &str,
        authorization: Option<&str>,
        limit: u64,
    ) -> Result<Box<dyn Read + Send>, FeedError> {
        let limit = usize::try_from(limit).map_err(|_| FeedError::TooLarge)?;
        Ok(Box::new(io::Cursor::new(self.get(
            url,
            authorization,
            limit,
        )?)))
    }
}

pub struct HttpTransport;

impl FeedTransport for HttpTransport {
    fn get(
        &self,
        url: &str,
        authorization: Option<&str>,
        limit: usize,
    ) -> Result<Vec<u8>, FeedError> {
        let mut bytes = Vec::new();
        self.open(url, authorization, limit as u64)?
            .take(limit as u64 + 1)
            .read_to_end(&mut bytes)
            .map_err(|_| FeedError::Network)?;
        if bytes.len() > limit {
            return Err(FeedError::TooLarge);
        }
        Ok(bytes)
    }

    fn open(
        &self,
        url: &str,
        authorization: Option<&str>,
        _limit: u64,
    ) -> Result<Box<dyn Read + Send>, FeedError> {
        let builder = ureq::AgentBuilder::new()
            .redirects(0)
            .timeout_connect(Duration::from_secs(15))
            .timeout_read(Duration::from_secs(30));
        let agent = crate::proxy::agent(builder, url).build();
        let mut request = agent.get(url);
        if let Some(authorization) = authorization {
            request = request.set("Authorization", authorization);
        }
        let response = match request.call() {
            Ok(response) => response,
            Err(ureq::Error::Status(status, _)) => return Err(status_error(status)),
            Err(ureq::Error::Transport(_)) => return Err(FeedError::Network),
        };
        if !(200..300).contains(&response.status()) {
            return Err(status_error(response.status()));
        }
        Ok(response.into_reader())
    }
}

fn status_error(status: u16) -> FeedError {
    if status == 401 || status == 403 {
        FeedError::NeedsLogin
    } else {
        FeedError::HttpStatus(status)
    }
}

pub struct FeedClient {
    config: FeedConfig,
    auth: Arc<dyn YoakeAuth>,
    transport: Arc<dyn FeedTransport>,
    data_dir: PathBuf,
}

impl FeedClient {
    pub fn install_connection(&self) -> (Url, Arc<dyn YoakeAuth>, PathBuf) {
        (
            self.config.root.clone(),
            Arc::clone(&self.auth),
            self.data_dir.clone(),
        )
    }

    pub fn configured() -> Result<Option<Self>, FeedError> {
        let Some(config) = FeedConfig::compiled()? else {
            return Ok(None);
        };
        let data_dir = crate::backend::data_dir();
        Ok(Some(Self::new(config, data_dir)))
    }

    pub fn new(config: FeedConfig, data_dir: PathBuf) -> Self {
        Self::with_components(
            config,
            Arc::new(TokenFileAuth::new(&data_dir)),
            Arc::new(HttpTransport),
            data_dir,
        )
    }

    pub fn with_components(
        config: FeedConfig,
        auth: Arc<dyn YoakeAuth>,
        transport: Arc<dyn FeedTransport>,
        data_dir: PathBuf,
    ) -> Self {
        Self {
            config,
            auth,
            transport,
            data_dir,
        }
    }

    fn resource(&self, relative: &str, limit: usize) -> Result<Vec<u8>, FeedError> {
        let url = self
            .config
            .root
            .join(relative)
            .map_err(|_| FeedError::Configuration)?;
        let authorization = self
            .auth
            .authorization()
            .map_err(FeedError::Authentication)?;
        let bytes = self
            .transport
            .get(url.as_str(), authorization.as_deref(), limit)?;
        if bytes.len() > limit {
            return Err(FeedError::TooLarge);
        }
        Ok(bytes)
    }

    pub fn fetch_feed(&self) -> Result<VerifiedFeed, FeedError> {
        let pointer: Pointer = serde_json::from_slice(&self.resource("current.json", MAX_POINTER)?)
            .map_err(|_| FeedError::Malformed)?;
        if !safe_component(&pointer.source_id) {
            return Err(FeedError::WrongSource);
        }
        if self
            .config
            .source_id
            .as_ref()
            .is_some_and(|id| id != &pointer.source_id)
        {
            return Err(FeedError::WrongSource);
        }
        let prefix = format!("indexes/{:06}/", pointer.sequence);
        let sums = self.resource(&format!("{prefix}SHA256SUMS"), MAX_SUMS)?;
        let signature = self.resource(&format!("{prefix}SHA256SUMS.sig"), MAX_SIGNATURE)?;
        let signature = std::str::from_utf8(&signature).map_err(|_| FeedError::BadSignature)?;
        let entries = verify(DOMAIN, &sums, signature, &self.config.keys)
            .map_err(|_| FeedError::BadSignature)?;
        let index_digest = signed_digests(&entries)?;
        let bytes = self.resource(&format!("{prefix}index.json"), MAX_INDEX)?;
        let index_sha256 = sha256_hex(&bytes);
        if index_sha256 != index_digest {
            return Err(FeedError::HashMismatch);
        }
        let identity: IndexIdentity =
            serde_json::from_slice(&bytes).map_err(|_| FeedError::Malformed)?;
        if identity.sequence != pointer.sequence || identity.source_id != pointer.source_id {
            return Err(FeedError::WrongSource);
        }
        let schema: Schema = serde_json::from_slice(&bytes).map_err(|_| FeedError::Malformed)?;
        match schema.check(2) {
            Ok(_) => {}
            Err(FeedError::NeedsUpdate) => return self.cached(&pointer, &index_sha256),
            Err(error) => return Err(error),
        }
        let proof = FeedProof {
            index_json: String::from_utf8(bytes).map_err(|_| FeedError::Malformed)?,
            sums: String::from_utf8(sums).map_err(|_| FeedError::Malformed)?,
            signature: signature.to_owned(),
        };
        let mut feed = match parse_verified_feed(&self.config, proof) {
            Ok(feed) => feed,
            Err(FeedError::NeedsUpdate) => return self.cached(&pointer, &index_sha256),
            Err(error) => return Err(error),
        };
        // The presentation is optional: whatever goes wrong with it (missing, unreachable,
        // unsigned, for another index, from a newer schema) must not take the pack feed down.
        let presentation = match self.fetch_presentation(&prefix, &index_sha256) {
            Ok(found) => Some(found),
            Err(error) => {
                feed.needs_update |= error == FeedError::NeedsUpdate;
                self.last_good_presentation(&index_sha256)
            }
        };
        let presentation_proof = presentation.as_ref().map(|(proof, _)| proof.clone());
        feed.presentation = presentation.map(|(_, document)| document);
        self.remember(&feed, presentation_proof)?;
        Ok(feed)
    }

    /// The optional signed presentation of one index folder, bound to `index_sha256`.
    fn fetch_presentation(
        &self,
        prefix: &str,
        index_sha256: &str,
    ) -> Result<(PresentationProof, Presentation), FeedError> {
        let sums = self.resource(&format!("{prefix}presentation.sums"), MAX_SUMS)?;
        let signature = self.resource(&format!("{prefix}presentation.sums.sig"), MAX_SIGNATURE)?;
        let json = self.resource(&format!("{prefix}presentation.json"), MAX_INDEX)?;
        let proof = PresentationProof {
            json: String::from_utf8(json).map_err(|_| FeedError::Malformed)?,
            sums: String::from_utf8(sums).map_err(|_| FeedError::Malformed)?,
            signature: String::from_utf8(signature).map_err(|_| FeedError::BadSignature)?,
        };
        let document = verify_presentation(&self.config, &proof, index_sha256)?;
        Ok((proof, document))
    }

    /// The presentation saved with the same signed index, verified again.
    fn last_good_presentation(&self, index_sha256: &str) -> Option<(PresentationProof, Presentation)> {
        let (state_path, _, _lock) = self.lock_state().ok()?;
        let snapshot = Self::read_snapshot(&state_path)?;
        if snapshot.index_sha256 != index_sha256 {
            return None;
        }
        let proof = snapshot.presentation?;
        let document = verify_presentation(&self.config, &proof, index_sha256).ok()?;
        Some((proof, document))
    }

    // CHECK: L3 needs a streaming verifier for objects larger than the icon limit.
    pub fn fetch_object(&self, object: &Object) -> Result<Vec<u8>, FeedError> {
        if !valid_digest(&object.sha256) || object.size > MAX_ICON {
            return Err(FeedError::Malformed);
        }
        let bytes = self.resource(
            &format!("objects/sha256/{}", object.sha256),
            object.size as usize,
        )?;
        if bytes.len() as u64 != object.size || sha256_hex(&bytes) != object.sha256 {
            return Err(FeedError::HashMismatch);
        }
        Ok(bytes)
    }

    fn lock_state(&self) -> Result<(PathBuf, String, fs::File), FeedError> {
        let directory = self.data_dir.join("yoake-feeds");
        fs::create_dir_all(&directory).map_err(|_| FeedError::Storage)?;
        let feed_id = sha256_hex(self.config.root.as_str().as_bytes());
        let state_path = directory.join(format!("{feed_id}.json"));
        let lock = OpenOptions::new()
            .read(true)
            .write(true)
            .create(true)
            .truncate(false)
            .open(directory.join(format!("{feed_id}.lock")))
            .map_err(|_| FeedError::Storage)?;
        lock.lock().map_err(|_| FeedError::Storage)?;
        Ok((state_path, feed_id, lock))
    }

    fn snapshot_path(state_path: &Path) -> PathBuf {
        state_path.with_extension("snapshot.json")
    }

    fn read_bounded(path: &Path, limit: usize) -> Result<Option<Vec<u8>>, FeedError> {
        match fs::File::open(path) {
            Ok(file) => {
                let mut contents = Vec::new();
                file.take(limit as u64 + 1)
                    .read_to_end(&mut contents)
                    .map_err(|_| FeedError::Storage)?;
                if contents.len() > limit {
                    return Err(FeedError::Storage);
                }
                Ok(Some(contents))
            }
            Err(error) if error.kind() == io::ErrorKind::NotFound => Ok(None),
            Err(_) => Err(FeedError::Storage),
        }
    }

    fn read_state(path: &Path) -> Result<Option<HighWater>, FeedError> {
        let Some(contents) = Self::read_bounded(path, MAX_STATE)? else {
            return Ok(None);
        };
        let previous: HighWater =
            serde_json::from_slice(&contents).map_err(|_| FeedError::Storage)?;
        if !valid_digest(&previous.index_sha256) {
            return Err(FeedError::Storage);
        }
        Ok(Some(previous))
    }

    /// The saved snapshot, if there is a usable one. It is only a convenience, so every problem
    /// reads as "none".
    fn read_snapshot(state_path: &Path) -> Option<Snapshot> {
        let contents = Self::read_bounded(&Self::snapshot_path(state_path), MAX_SNAPSHOT)
            .ok()??;
        let snapshot: Snapshot = serde_json::from_slice(&contents).ok()?;
        valid_digest(&snapshot.index_sha256).then_some(snapshot)
    }

    fn check_high_water(
        previous: &HighWater,
        source_id: &str,
        sequence: u64,
        digest: &str,
    ) -> Result<(), FeedError> {
        if previous.source_id != source_id {
            return Err(FeedError::WrongSource);
        }
        if sequence < previous.sequence
            || (sequence == previous.sequence && digest != previous.index_sha256)
        {
            return Err(FeedError::Replay);
        }
        Ok(())
    }

    /// The last compatible feed, for an index this launcher cannot read.
    fn cached(&self, pointer: &Pointer, digest: &str) -> Result<VerifiedFeed, FeedError> {
        let (path, _, _lock) = self.lock_state()?;
        let previous = Self::read_state(&path)?.ok_or(FeedError::NeedsUpdate)?;
        Self::check_high_water(&previous, &pointer.source_id, pointer.sequence, digest)?;
        let snapshot = Self::read_snapshot(&path).ok_or(FeedError::NeedsUpdate)?;
        // A released launcher may have moved the high-water mark past the snapshot; it can
        // never be ahead of it.
        if snapshot.source_id != previous.source_id
            || snapshot.sequence > previous.sequence
            || (snapshot.sequence == previous.sequence
                && snapshot.index_sha256 != previous.index_sha256)
        {
            return Err(FeedError::Storage);
        }
        let mut feed = parse_verified_feed(&self.config, snapshot.proof)?;
        let (source_id, sequence) = index_identity(&feed.index);
        if source_id != snapshot.source_id
            || sequence != snapshot.sequence
            || feed.index_sha256 != snapshot.index_sha256
        {
            return Err(FeedError::Storage);
        }
        feed.presentation = snapshot
            .presentation
            .and_then(|proof| verify_presentation(&self.config, &proof, &feed.index_sha256).ok());
        feed.needs_update = true;
        Ok(feed)
    }

    fn remember(
        &self,
        feed: &VerifiedFeed,
        presentation: Option<PresentationProof>,
    ) -> Result<(), FeedError> {
        let (state_path, feed_id, _lock) = self.lock_state()?;
        let (source_id, sequence) = index_identity(&feed.index);
        let previous = Self::read_state(&state_path)?;
        if let Some(previous) = &previous {
            Self::check_high_water(previous, source_id, sequence, &feed.index_sha256)?;
        }
        if previous.as_ref().is_none_or(|previous| sequence > previous.sequence) {
            let state = HighWater {
                source_id: source_id.to_owned(),
                sequence,
                index_sha256: feed.index_sha256.clone(),
            };
            let contents = serde_json::to_vec(&state).map_err(|_| FeedError::Storage)?;
            Self::write_atomic(&state_path, &feed_id, &contents)?;
        }
        let snapshot = Snapshot {
            source_id: source_id.to_owned(),
            sequence,
            index_sha256: feed.index_sha256.clone(),
            proof: feed.proof.clone().ok_or(FeedError::Storage)?,
            presentation,
        };
        let contents = serde_json::to_vec(&snapshot).map_err(|_| FeedError::Storage)?;
        if contents.len() > MAX_SNAPSHOT {
            return Err(FeedError::Storage);
        }
        let snapshot_path = Self::snapshot_path(&state_path);
        if Self::read_bounded(&snapshot_path, MAX_SNAPSHOT).ok().flatten().as_deref()
            != Some(contents.as_slice())
        {
            Self::write_atomic(&snapshot_path, &feed_id, &contents)?;
        }
        Ok(())
    }

    fn write_atomic(path: &Path, feed_id: &str, contents: &[u8]) -> Result<(), FeedError> {
        let temp_path = path.with_file_name(format!(
            "{feed_id}.{}.{}.tmp",
            std::process::id(),
            NEXT_TEMP.fetch_add(1, Ordering::Relaxed)
        ));
        let result = (|| {
            let mut file = OpenOptions::new()
                .write(true)
                .create_new(true)
                .open(&temp_path)?;
            file.write_all(contents)?;
            file.sync_all()?;
            drop(file);
            fs::rename(&temp_path, path)
        })();
        if result.is_err() {
            let _ = fs::remove_file(&temp_path);
            return Err(FeedError::Storage);
        }
        Ok(())
    }
}

fn valid_digest(value: &str) -> bool {
    value.len() == 64
        && value
            .bytes()
            .all(|byte| byte.is_ascii_digit() || (b'a'..=b'f').contains(&byte))
}

fn safe_component(value: &str) -> bool {
    if value.is_empty()
        || !value.is_ascii()
        || value.len() > 255
        || value == "."
        || value == ".."
        || value.ends_with([' ', '.'])
        || value
            .chars()
            .any(|ch| ch.is_control() || ch == '\u{22}' || "<>:\\|?*/".contains(ch))
    {
        return false;
    }
    let stem = value
        .split('.')
        .next()
        .unwrap_or_default()
        .to_ascii_uppercase();
    !matches!(
        stem.as_str(),
        "CON" | "PRN" | "AUX" | "NUL" | "CONIN$" | "CONOUT$"
    ) && !(stem.len() == 4
        && (stem.starts_with("COM") || stem.starts_with("LPT"))
        && matches!(stem.as_bytes()[3], b'1'..=b'9'))
}

pub fn validate_destination(destination: &str) -> Result<(), FeedError> {
    if destination.len() > 4096 || !destination.split('/').all(safe_component) {
        return Err(FeedError::UnsafePath);
    }
    Ok(())
}

fn insert_unique(set: &mut BTreeSet<String>, value: &str) -> Result<(), FeedError> {
    if !safe_component(value) || !set.insert(value.to_lowercase()) {
        return Err(FeedError::UnsafePath);
    }
    Ok(())
}

fn validate_index(index: &FeedIndex) -> Result<(), FeedError> {
    if index.schema != 1 || !safe_component(&index.source_id) {
        return Err(FeedError::Malformed);
    }
    let mut pack_ids = BTreeSet::new();
    for pack in &index.packs {
        insert_unique(&mut pack_ids, &pack.id)?;
        if pack.name.is_empty() || !valid_digest(&pack.icon.sha256) || pack.icon.size > MAX_ICON {
            return Err(FeedError::Malformed);
        }
        let mut versions = BTreeSet::new();
        for version in &pack.versions {
            insert_unique(&mut versions, &version.version)?;
            if !safe_component(&version.minimum_launcher)
                || !safe_component(&version.base.horizon_release)
                || !safe_component(&version.base.prepared_format)
                || !valid_digest(&version.base.research_lock_sha256)
            {
                return Err(FeedError::Malformed);
            }
            let mut content = BTreeSet::new();
            for id in &version.enabled_content_mods {
                insert_unique(&mut content, id)?;
            }
            let mut targets = BTreeSet::new();
            for (name, target) in &version.targets {
                insert_unique(&mut targets, name)?;
                let mut destinations = BTreeSet::new();
                for file in &target.files {
                    validate_destination(&file.destination)?;
                    if !valid_digest(&file.sha256) || file.size > MAX_FILE {
                        return Err(FeedError::Malformed);
                    }
                    if matches!(
                        (file.scope, file.policy),
                        (Scope::Version, Some(_)) | (Scope::Instance, None)
                    ) {
                        return Err(FeedError::Malformed);
                    }
                    let scope = match file.scope {
                        Scope::Version => "version",
                        Scope::Instance => "instance",
                    };
                    let destination = format!("{scope}/{}", file.destination.to_lowercase());
                    if destinations.contains(&destination) {
                        return Err(FeedError::UnsafePath);
                    }
                    let mut parent = destination.as_str();
                    while let Some((prefix, _)) = parent.rsplit_once('/') {
                        if destinations.contains(prefix) {
                            return Err(FeedError::UnsafePath);
                        }
                        parent = prefix;
                    }
                    let descendant = format!("{destination}/");
                    if destinations
                        .range(descendant.clone()..)
                        .next()
                        .is_some_and(|entry: &String| entry.starts_with(&descendant))
                    {
                        return Err(FeedError::UnsafePath);
                    }
                    destinations.insert(destination);
                }
            }
        }
    }
    Ok(())
}

/// True when a pack's enabled native mods are exactly the ones compiled into
/// its base (compared case-insensitively, order ignored).
pub(crate) fn same_native_mods(enabled: &[String], base: &[String]) -> bool {
    let lower = |mods: &[String]| -> BTreeSet<String> {
        mods.iter().map(|id| id.to_ascii_lowercase()).collect()
    };
    enabled.len() == base.len() && lower(enabled) == lower(base)
}

pub(crate) fn validate_index_v2(index: &FeedIndexV2) -> Result<(), FeedError> {
    if index.schema != 2 || index.source_id != "yoake-official" || index.sequence == 0 {
        return Err(FeedError::Malformed);
    }
    let mut bases = BTreeMap::new();
    for base in &index.bases {
        if !safe_component(&base.version)
            || bases
                .insert(base.version.to_ascii_lowercase(), base)
                .is_some()
            || !safe_component(&base.horizon_release)
            || base
                .yoake
                .as_ref()
                .is_some_and(|identity| !identity.is_valid())
            || !safe_component(&base.prepared_format)
            || !valid_digest(&base.research_lock_sha256)
            || base.targets.is_empty()
        {
            return Err(FeedError::Malformed);
        }
        let mut native_ids = BTreeSet::new();
        for id in &base.native_mods {
            insert_unique(&mut native_ids, id)?;
        }
        let mut targets = BTreeSet::new();
        for (name, target) in &base.targets {
            insert_unique(&mut targets, name)?;
            let [file] = target.files.as_slice() else {
                return Err(FeedError::Malformed);
            };
            if !matches!(file.scope, Scope::Version)
                || file.destination != "horizon-client.exe"
                || !valid_digest(&file.sha256)
                || file.size > MAX_FILE
            {
                return Err(FeedError::Malformed);
            }
        }
    }

    let mut pack_ids = BTreeSet::new();
    for pack in &index.packs {
        insert_unique(&mut pack_ids, &pack.id)?;
        if !valid_text(&pack.name) || !valid_digest(&pack.icon.sha256) || pack.icon.size > MAX_ICON
        {
            return Err(FeedError::Malformed);
        }
        let mut versions = BTreeSet::new();
        for version in &pack.versions {
            insert_unique(&mut versions, &version.version)?;
            if !safe_component(&version.minimum_launcher)
                || !safe_component(&version.base_version)
                || !valid_text(&version.description)
                || !valid_text(&version.changelog)
                || version.targets.is_empty()
            {
                return Err(FeedError::Malformed);
            }
            let base = bases
                .get(&version.base_version.to_ascii_lowercase())
                .ok_or(FeedError::Malformed)?;
            for mods in [&version.enabled_native_mods, &version.enabled_content_mods] {
                let mut ids = BTreeSet::new();
                for id in mods {
                    insert_unique(&mut ids, id)?;
                }
            }
            if !same_native_mods(&version.enabled_native_mods, &base.native_mods) {
                return Err(FeedError::Malformed);
            }
            let mut targets = BTreeSet::new();
            for (name, target) in &version.targets {
                insert_unique(&mut targets, name)?;
                if !base.targets.contains_key(name) {
                    return Err(FeedError::Malformed);
                }
                let mut destinations = BTreeSet::new();
                for file in &target.files {
                    if !matches!(file.scope, Scope::Instance)
                        || !valid_digest(&file.sha256)
                        || file.size > MAX_FILE
                    {
                        return Err(FeedError::Malformed);
                    }
                    validate_pack_destination(&file.destination, file.policy)?;
                    insert_destination(&mut destinations, &file.destination)?;
                }
            }
        }
    }
    Ok(())
}

fn valid_text(value: &str) -> bool {
    !value.is_empty() && value.len() <= 4096
}

pub(crate) fn validate_pack_destination(
    destination: &str,
    policy: Policy,
) -> Result<(), FeedError> {
    validate_destination(destination)?;
    let lower = destination.to_ascii_lowercase();
    let parts: Vec<&str> = lower.split('/').collect();
    const FORBIDDEN: &[&str] = &[
        ".local",
        ".minecraft",
        "horizon-resources.json",
        "horizon-prepare.json",
        "minecraft.exe",
        "minecraftlauncher.exe",
        "minecraftinstaller.exe",
        ".horizon-prepare.lock",
        "horizon-data.bin",
        "asset-index.json",
        "horizon-sprites.cache",
        "prepared.json",
        "facts.json",
        "block-tags.json",
        "attributes.json",
        "landing-definitions.json",
        "damage-types.json",
        "damage-tags.json",
        "damage-catalog.json",
        "player-float-ids.json",
        "render-definitions.json",
        "gameplay-facts.json",
        "entity-types.json",
        "block-behaviours.json",
        "model-states.json",
        "entity-models.json",
        "mob-data.json",
        "command-registries.json",
        "creative-inventory.json",
        "survival-rules.json",
        "survival-extra-definitions.json",
        "placement-definitions.json",
    ];
    if parts
        .iter()
        .any(|part| FORBIDDEN.contains(part) || part.ends_with(".jar") || part.ends_with(".class"))
        || parts.windows(2).any(|pair| {
            matches!(
                pair,
                ["assets", "indexes" | "objects" | "minecraft"]
                    | ["data", "minecraft"]
                    | ["com", "mojang"]
            )
        })
    {
        return Err(FeedError::UnsafePath);
    }
    let extension = parts
        .last()
        .and_then(|name| name.rsplit_once('.'))
        .filter(|(stem, _)| !stem.is_empty())
        .map(|(_, extension)| extension)
        .unwrap_or_default();
    let allowed = match parts.as_slice() {
        ["horizon-mods", _] => extension == "hmod",
        ["resourcepacks" | "datapacks", _] => extension == "zip",
        ["config", _, ..] => matches!(
            extension,
            "toml" | "json" | "properties" | "txt" | "yaml" | "yml"
        ),
        ["saves", _, _, ..] if matches!(policy, Policy::DefaultOnCreate) => {
            matches!(extension, "dat" | "mca" | "json" | "txt")
                || (parts.len() == 4 && parts[2] == "datapacks" && extension == "zip")
        }
        _ => false,
    };
    if !allowed {
        return Err(FeedError::UnsafePath);
    }
    Ok(())
}

fn insert_destination(
    destinations: &mut BTreeSet<String>,
    destination: &str,
) -> Result<(), FeedError> {
    let destination = destination.to_ascii_lowercase();
    if destinations.contains(&destination) {
        return Err(FeedError::UnsafePath);
    }
    let mut parent = destination.as_str();
    while let Some((prefix, _)) = parent.rsplit_once('/') {
        if destinations.contains(prefix) {
            return Err(FeedError::UnsafePath);
        }
        parent = prefix;
    }
    let descendant = format!("{destination}/");
    if destinations
        .range(descendant.clone()..)
        .next()
        .is_some_and(|entry| entry.starts_with(&descendant))
    {
        return Err(FeedError::UnsafePath);
    }
    destinations.insert(destination);
    Ok(())
}

pub fn validate_existing_path(root: &Path, destination: &str) -> Result<(), FeedError> {
    validate_destination(destination)?;
    let mut path = root.to_path_buf();
    for part in std::iter::once("").chain(destination.split('/')) {
        if !part.is_empty() {
            path.push(part);
        }
        match fs::symlink_metadata(&path) {
            Ok(metadata) => {
                if metadata.file_type().is_symlink() {
                    return Err(FeedError::UnsafePath);
                }
                #[cfg(windows)]
                {
                    use std::os::windows::fs::MetadataExt;
                    if metadata.file_attributes() & 0x400 != 0 {
                        return Err(FeedError::UnsafePath);
                    }
                }
            }
            Err(error) if error.kind() == io::ErrorKind::NotFound => return Ok(()),
            Err(_) => return Err(FeedError::Storage),
        }
    }
    // CHECK: L3 must use handle-relative, no-follow creation; this preflight is not race-free.
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use horizon_release_sign::{sums_file, SigningKey};
    use serde_json::{json, Value};
    use std::sync::Mutex;
    use std::time::{SystemTime, UNIX_EPOCH};

    #[derive(Default)]
    struct FakeTransport {
        responses: Mutex<BTreeMap<String, Result<Vec<u8>, FeedError>>>,
        requests: Mutex<Vec<(String, Option<String>)>>,
    }

    impl FakeTransport {
        fn respond(&self, path: &str, response: Result<Vec<u8>, FeedError>) {
            self.responses
                .lock()
                .unwrap()
                .insert(format!("https://feed.test/{path}"), response);
        }
    }

    impl FeedTransport for FakeTransport {
        fn get(
            &self,
            url: &str,
            authorization: Option<&str>,
            _limit: usize,
        ) -> Result<Vec<u8>, FeedError> {
            self.requests
                .lock()
                .unwrap()
                .push((url.to_owned(), authorization.map(str::to_owned)));
            self.responses
                .lock()
                .unwrap()
                .get(url)
                .cloned()
                .unwrap_or(Err(FeedError::Network))
        }
    }

    struct FixedAuth(Option<String>);

    impl YoakeAuth for FixedAuth {
        fn authorization(&self) -> Result<Option<String>, AuthError> {
            Ok(self.0.clone())
        }
    }

    struct Fixture {
        root: PathBuf,
        client: FeedClient,
        transport: Arc<FakeTransport>,
        key: SigningKey,
    }

    impl Fixture {
        fn new(authorization: Option<&str>) -> Self {
            let (key, _) = SigningKey::generate().unwrap();
            let root = std::env::temp_dir().join(format!(
                "yoake-feed-test-{}-{}-{}",
                std::process::id(),
                SystemTime::now()
                    .duration_since(UNIX_EPOCH)
                    .unwrap()
                    .as_nanos(),
                NEXT_TEMP.fetch_add(1, Ordering::Relaxed)
            ));
            let config = FeedConfig::new(
                "https://feed.test/",
                vec![key.public()],
                Some("yoake-official".to_owned()),
            )
            .unwrap();
            let transport = Arc::new(FakeTransport::default());
            let client = FeedClient::with_components(
                config,
                Arc::new(FixedAuth(authorization.map(str::to_owned))),
                transport.clone(),
                root.clone(),
            );
            Self {
                root,
                client,
                transport,
                key,
            }
        }

        fn publish(&self, sequence: u64, index: &Value, domain: &str) {
            let index = serde_json::to_vec(index).unwrap();
            let sums = sums_file(&[("index.json".to_owned(), sha256_hex(&index))]);
            let signature = self.key.sign(domain, sums.as_bytes());
            let prefix = format!("indexes/{sequence:06}");
            self.transport.respond(
                "current.json",
                Ok(serde_json::to_vec(&json!({
                    "source_id": "yoake-official",
                    "sequence": sequence,
                }))
                .unwrap()),
            );
            self.transport
                .respond(&format!("{prefix}/SHA256SUMS"), Ok(sums.into_bytes()));
            self.transport.respond(
                &format!("{prefix}/SHA256SUMS.sig"),
                Ok(signature.into_bytes()),
            );
            self.transport
                .respond(&format!("{prefix}/index.json"), Ok(index));
            self.transport.respond(
                &format!("{prefix}/presentation.sums"),
                Err(FeedError::HttpStatus(404)),
            );
        }

        fn publish_with_presentation(&self, sequence: u64, index: &Value, presentation: &Value) {
            self.publish(sequence, index, DOMAIN);
            self.sign_presentation(sequence, &index_sha(index), presentation);
        }

        /// Signs `presentation.sums` over `<index sha>  index.json` and the presentation itself.
        fn sign_presentation(&self, sequence: u64, index_sha: &str, presentation: &Value) {
            let presentation = serde_json::to_vec(presentation).unwrap();
            let sums = sums_file(&[
                ("index.json".to_owned(), index_sha.to_owned()),
                ("presentation.json".to_owned(), sha256_hex(&presentation)),
            ]);
            let signature = self.key.sign(DOMAIN, sums.as_bytes());
            let prefix = format!("indexes/{sequence:06}");
            self.transport
                .respond(&format!("{prefix}/presentation.sums"), Ok(sums.into_bytes()));
            self.transport.respond(
                &format!("{prefix}/presentation.sums.sig"),
                Ok(signature.into_bytes()),
            );
            self.transport
                .respond(&format!("{prefix}/presentation.json"), Ok(presentation));
        }

        fn restart(&self) -> FeedClient {
            FeedClient::with_components(
                self.client.config.clone(),
                Arc::new(FixedAuth(None)),
                self.transport.clone(),
                self.root.clone(),
            )
        }

        fn response(&self, path: &str) -> Result<Vec<u8>, FeedError> {
            self.transport.responses.lock().unwrap()[&format!("https://feed.test/{path}")].clone()
        }
    }

    fn index_sha(index: &Value) -> String {
        sha256_hex(&serde_json::to_vec(index).unwrap())
    }

    impl Drop for Fixture {
        fn drop(&mut self) {
            let _ = fs::remove_dir_all(&self.root);
        }
    }

    fn index(sequence: u64) -> Value {
        json!({
            "schema": 1,
            "source_id": "yoake-official",
            "sequence": sequence,
            "packs": [{
                "id": "adventure",
                "name": "Yoake Adventure",
                "icon": { "sha256": "a".repeat(64), "size": 1234 },
                "versions": [{
                    "version": "1.2.0",
                    "minimum_launcher": "1.0.0",
                    "changelog": "Release notes",
                    "base": {
                        "horizon_release": "26.3",
                        "research_lock_sha256": "b".repeat(64),
                        "prepared_format": "format-1"
                    },
                    "enabled_content_mods": ["example"],
                    "targets": {
                        "windows-x86_64": { "files": [{
                            "scope": "instance",
                            "policy": "managed",
                            "destination": "horizon-mods/example.hmod",
                            "sha256": "c".repeat(64),
                            "size": 23456
                        }] }
                    }
                }]
            }]
        })
    }

    fn index_v2(sequence: u64) -> Value {
        let mut index: Value =
            serde_json::from_slice(include_bytes!("../tests/fixtures/yoake-schema2/index.json"))
                .unwrap();
        index["sequence"] = json!(sequence);
        index
    }

    fn presentation() -> Value {
        json!({
            "schema": "1.0",
            "news": [{
                "id": "launch",
                "date": "2026-11-14",
                "title": {"en": "Welcome", "ja": "Japanese title"},
                "body": {"en": "Hello", "ja": "Japanese body"},
                "link": "https://yoake.example/news"
            }],
            "links": [{
                "label": {"en": "Help", "ja": "Japanese help"},
                "url": "https://yoake.example/help"
            }],
            "categories": [{
                "id": "featured",
                "label": {"en": "Featured", "ja": "Japanese featured"},
                "order": 1
            }],
            "feature_flags": {"future_feature": true}
        })
    }

    fn pack_file_v2(index: &mut Value) -> &mut Value {
        &mut index["packs"][0]["versions"][0]["targets"]["windows-x86_64"]["files"][0]
    }

    #[test]
    fn accepts_newer_minor_versions_unknown_fields_and_localized_presentation() {
        let fixture = Fixture::new(None);
        let mut new_index = index_v2(42);
        new_index["schema"] = json!("2.99");
        new_index["future_field"] = json!({"value": true});
        new_index["packs"][0]["future_field"] = json!(true);
        new_index["packs"][0]["category"] = json!("featured");
        pack_file_v2(&mut new_index)["future_field"] = json!("ignored");
        let mut presentation = presentation();
        presentation["schema"] = json!("1.17");
        presentation["news"][0]["future_field"] = json!("ignored");
        presentation["links"][0]["label"]["de"] = json!("");
        fixture.publish_with_presentation(42, &new_index, &presentation);

        let feed = fixture.client.fetch_feed().unwrap();
        assert!(!feed.needs_update);
        let presentation = feed.presentation.as_ref().unwrap();
        assert_eq!(localized(&presentation.news[0].title, "ja"), "Japanese title");
        assert_eq!(localized(&presentation.news[0].title, "de"), "Welcome");
        assert_eq!(localized(&presentation.news[0].body, "ja"), "Japanese body");
        assert_eq!(localized(&presentation.news[0].body, "fr"), "Hello");
        assert_eq!(localized(&presentation.links[0].label, "ja"), "Japanese help");
        assert_eq!(localized(&presentation.links[0].label, "de"), "Help");
        assert_eq!(localized(&presentation.categories[0].label, "de"), "Featured");
        let ParsedFeedIndex::V2(parsed_index) = &feed.index else {
            panic!("expected a schema 2 feed");
        };
        let groups = pack_groups(&parsed_index.packs, Some(presentation));
        assert_eq!(groups.len(), 1);
        assert_eq!(groups[0].0.unwrap().id, "featured");
        assert_eq!(groups[0].1[0].0, 0);

        let legacy = Fixture::new(None);
        let mut old_index = index(42);
        old_index["schema"] = json!("1.9");
        old_index["packs"][0]["unknown"] = json!(true);
        legacy.publish(42, &old_index, DOMAIN);
        assert!(legacy.client.fetch_feed().is_ok());
    }

    #[test]
    fn missing_presentation_keeps_existing_feed_and_pack_order() {
        let fixture = Fixture::new(None);
        fixture.publish(42, &index_v2(42), DOMAIN);
        let feed = fixture.client.fetch_feed().unwrap();
        assert!(feed.presentation.is_none());
        assert!(!feed.needs_update);
        assert!(!fixture
            .transport
            .requests
            .lock()
            .unwrap()
            .iter()
            .any(|(url, _)| url.ends_with("/presentation.json")));
        let ParsedFeedIndex::V2(index) = &feed.index else {
            panic!("expected a schema 2 feed");
        };
        let groups = pack_groups(&index.packs, None);
        assert_eq!(groups.len(), 1);
        assert!(groups[0].0.is_none());
        assert_eq!(groups[0].1[0].0, 0);
    }

    #[test]
    fn incompatible_index_restores_the_last_signed_snapshot_after_restart() {
        for incompatible in ["index major", "index feature"] {
            let fixture = Fixture::new(None);
            fixture.publish_with_presentation(42, &index_v2(42), &presentation());
            fixture.client.fetch_feed().unwrap();
            let mut index = index_v2(43);
            match incompatible {
                "index major" => index["schema"] = json!("3.0"),
                "index feature" => index["critical_features"] = json!(["future-index-feature"]),
                _ => unreachable!(),
            }
            fixture.publish_with_presentation(43, &index, &presentation());
            let feed = fixture.restart().fetch_feed().unwrap();
            assert!(feed.needs_update, "{incompatible}");
            assert_eq!(index_identity(&feed.index).1, 42, "{incompatible}");
            assert_eq!(localized(&feed.presentation.unwrap().news[0].title, "en"), "Welcome");
        }
    }

    #[test]
    fn incompatible_presentation_keeps_the_new_index_and_asks_for_an_update() {
        for incompatible in ["presentation major", "presentation feature"] {
            let fixture = Fixture::new(None);
            fixture.publish_with_presentation(42, &index_v2(42), &presentation());
            fixture.client.fetch_feed().unwrap();
            let mut new_presentation = presentation();
            match incompatible {
                "presentation major" => new_presentation["schema"] = json!("2.0"),
                _ => {
                    new_presentation["critical_features"] = json!(["future-presentation-feature"])
                }
            }
            fixture.publish_with_presentation(43, &index_v2(43), &new_presentation);
            let feed = fixture.restart().fetch_feed().unwrap();
            assert!(feed.needs_update, "{incompatible}");
            assert_eq!(index_identity(&feed.index).1, 43, "{incompatible}");
            assert!(feed.presentation.is_none(), "{incompatible}");
            // the pack index moved on: the high-water mark is 43
            fixture.publish(42, &index_v2(42), DOMAIN);
            assert_eq!(fixture.client.fetch_feed().unwrap_err(), FeedError::Replay);
        }
    }

    #[test]
    fn incompatible_index_restores_snapshot_when_presentation_is_unavailable() {
        let fixture = Fixture::new(None);
        fixture.publish_with_presentation(42, &index_v2(42), &presentation());
        fixture.client.fetch_feed().unwrap();
        let mut index = index_v2(43);
        index["schema"] = json!("3.0");
        fixture.publish_with_presentation(43, &index, &presentation());
        fixture.transport.respond(
            "indexes/000043/presentation.json",
            Err(FeedError::Network),
        );

        let feed = fixture.client.fetch_feed().unwrap();
        assert!(feed.needs_update);
        assert_eq!(index_identity(&feed.index).1, 42);
    }

    #[test]
    fn incompatible_document_without_snapshot_requires_an_update() {
        let fixture = Fixture::new(None);
        let mut index = index_v2(42);
        index["schema"] = json!("3.0");
        fixture.publish(42, &index, DOMAIN);
        assert!(matches!(fixture.client.fetch_feed(), Err(FeedError::NeedsUpdate)));
    }

    #[test]
    fn presentation_requires_https_links_and_english_fallbacks() {
        for invalid in ["http link", "credentials", "http news", "missing english", "non_boolean_flag"] {
            let mut document = presentation();
            match invalid {
                "http link" => document["links"][0]["url"] = json!("http://yoake.example/help"),
                "credentials" => document["links"][0]["url"] = json!("https://user@yoake.example/help"),
                "http news" => document["news"][0]["link"] = json!("http://yoake.example/news"),
                "missing english" => document["news"][0]["title"] = json!({"ja": "title"}),
                "non_boolean_flag" => document["feature_flags"]["future_feature"] = json!("yes"),
                _ => unreachable!(),
            }
            assert!(parse_presentation(&serde_json::to_vec(&document).unwrap()).is_err(), "{invalid}");
        }
    }

    #[test]
    fn presentation_problems_never_fail_the_pack_feed() {
        let cases = [
            "network",
            "server error",
            "forbidden",
            "tampered",
            "bad signature",
            "invalid document",
            "other index",
            "not utf-8",
        ];
        for name in cases {
            let fixture = Fixture::new(None);
            fixture.publish_with_presentation(42, &index_v2(42), &presentation());
            let t = &fixture.transport;
            match name {
                "network" => t.respond("indexes/000042/presentation.json", Err(FeedError::Network)),
                "server error" => t.respond(
                    "indexes/000042/presentation.sums",
                    Err(FeedError::HttpStatus(503)),
                ),
                "forbidden" => {
                    t.respond("indexes/000042/presentation.sums.sig", Err(status_error(403)))
                }
                "tampered" => t.respond("indexes/000042/presentation.json", Ok(b"{}".to_vec())),
                "bad signature" => {
                    t.respond("indexes/000042/presentation.sums.sig", Ok(b"nope".to_vec()))
                }
                "invalid document" => {
                    let mut document = presentation();
                    document["links"][0]["url"] = json!("http://yoake.example/help");
                    fixture.sign_presentation(42, &index_sha(&index_v2(42)), &document);
                }
                "other index" => fixture.sign_presentation(42, &"0".repeat(64), &presentation()),
                _ => t.respond("indexes/000042/presentation.json", Ok(vec![0xff, 0xfe])),
            }
            let feed = fixture
                .client
                .fetch_feed()
                .unwrap_or_else(|error| panic!("{name}: {error:?}"));
            assert!(feed.presentation.is_none(), "{name}");
            assert!(!feed.needs_update, "{name}");
        }
    }

    #[test]
    fn a_transient_presentation_failure_keeps_the_last_good_one_and_never_replays() {
        let fixture = Fixture::new(None);
        fixture.publish_with_presentation(42, &index_v2(42), &presentation());
        fixture.client.fetch_feed().unwrap();
        for failure in [
            Err(FeedError::HttpStatus(404)),
            Err(FeedError::Network),
            Err(FeedError::HttpStatus(500)),
            Err(status_error(403)),
        ] {
            fixture
                .transport
                .respond("indexes/000042/presentation.sums", failure);
            let feed = fixture.restart().fetch_feed().unwrap();
            assert_eq!(localized(&feed.presentation.unwrap().news[0].title, "en"), "Welcome");
        }
        // a different signed presentation for the same index is only ever a presentation
        let mut changed = presentation();
        changed["news"][0]["title"]["en"] = json!("Changed");
        fixture.sign_presentation(42, &index_sha(&index_v2(42)), &changed);
        let feed = fixture.client.fetch_feed().unwrap();
        assert_eq!(localized(&feed.presentation.unwrap().news[0].title, "en"), "Changed");
        // the pack index itself is still replay-protected
        let mut equivocation = index_v2(42);
        equivocation["packs"][0]["name"] = json!("Another pack name");
        fixture.publish(42, &equivocation, DOMAIN);
        assert_eq!(fixture.client.fetch_feed().unwrap_err(), FeedError::Replay);
    }

    #[test]
    fn a_presentation_is_bound_to_the_signed_index() {
        let fixture = Fixture::new(None);
        fixture.publish_with_presentation(41, &index_v2(41), &presentation());
        let old_sums = fixture.response("indexes/000041/presentation.sums");
        let old_signature = fixture.response("indexes/000041/presentation.sums.sig");
        // the sequence 41 presentation files copied into the sequence 42 folder
        fixture.publish_with_presentation(42, &index_v2(42), &presentation());
        fixture.transport.respond("indexes/000042/presentation.sums", old_sums);
        fixture.transport.respond("indexes/000042/presentation.sums.sig", old_signature);
        assert!(fixture.client.fetch_feed().unwrap().presentation.is_none());

        // a signed presentation.sums without the index line
        let fixture = Fixture::new(None);
        fixture.publish_with_presentation(42, &index_v2(42), &presentation());
        let document = serde_json::to_vec(&presentation()).unwrap();
        let only = sums_file(&[("presentation.json".to_owned(), sha256_hex(&document))]);
        fixture
            .transport
            .respond("indexes/000042/presentation.sums", Ok(only.clone().into_bytes()));
        fixture.transport.respond(
            "indexes/000042/presentation.sums.sig",
            Ok(fixture.key.sign(DOMAIN, only.as_bytes()).into_bytes()),
        );
        assert!(fixture.client.fetch_feed().unwrap().presentation.is_none());
        // the index SHA256SUMS is not a presentation.sums either
        fixture
            .transport
            .respond("indexes/000042/presentation.sums", fixture.response("indexes/000042/SHA256SUMS"));
        fixture.transport.respond(
            "indexes/000042/presentation.sums.sig",
            fixture.response("indexes/000042/SHA256SUMS.sig"),
        );
        assert!(fixture.client.fetch_feed().unwrap().presentation.is_none());
        // and a presentation.sums can never stand in for the index SHA256SUMS
        let sums = sums_file(&[
            ("index.json".to_owned(), index_sha(&index_v2(42))),
            ("presentation.json".to_owned(), sha256_hex(&document)),
        ]);
        let signature = fixture.key.sign(DOMAIN, sums.as_bytes());
        let entries = verify(DOMAIN, sums.as_bytes(), &signature, &fixture.client.config.keys).unwrap();
        assert_eq!(signed_digests(&entries), Err(FeedError::Malformed));
    }

    #[test]
    fn presentation_urls_are_opened_in_their_validated_form() {
        let mut document = presentation();
        document["links"][0]["url"] = json!("https://Yoake.Example/he lp?x=1");
        document["news"][0]["link"] = json!("https://yoake.example");
        let parsed = parse_presentation(&serde_json::to_vec(&document).unwrap()).unwrap();
        assert_eq!(parsed.links[0].url, "https://yoake.example/he%20lp?x=1");
        assert_eq!(parsed.news[0].link.as_deref(), Some("https://yoake.example/"));
    }

    /// The high-water file as the released launchers define it (copied from the parent commit).
    mod released {
        #[derive(serde::Deserialize, serde::Serialize)]
        #[serde(deny_unknown_fields)]
        pub struct HighWater {
            pub source_id: String,
            pub sequence: u64,
            pub index_sha256: String,
        }
        pub const MAX_POINTER: usize = 16 * 1024;
    }

    #[test]
    fn the_state_file_stays_readable_by_released_launchers_and_the_new_one_reads_theirs() {
        let fixture = Fixture::new(None);
        fixture.publish_with_presentation(42, &index_v2(42), &presentation());
        let feed = fixture.client.fetch_feed().unwrap();
        let directory = fixture.root.join("yoake-feeds");
        let feed_id = sha256_hex(fixture.client.config.root.as_str().as_bytes());
        let state = fs::read(directory.join(format!("{feed_id}.json"))).unwrap();
        assert!(state.len() < released::MAX_POINTER);
        let old: released::HighWater = serde_json::from_slice(&state).unwrap();
        assert_eq!((old.source_id.as_str(), old.sequence), ("yoake-official", 42));
        assert_eq!(old.index_sha256, feed.index_sha256);
        assert_eq!(state, serde_json::to_vec(&old).unwrap(), "byte-identical to the released shape");
        let mut names: Vec<String> = fs::read_dir(&directory)
            .unwrap()
            .map(|entry| entry.unwrap().file_name().to_string_lossy().into_owned())
            .collect();
        names.sort();
        let mut expected = vec![
            format!("{feed_id}.json"),
            format!("{feed_id}.lock"),
            format!("{feed_id}.snapshot.json"),
        ];
        expected.sort();
        assert_eq!(names, expected);

        // a released launcher (side by side or after a downgrade) takes sequence 43 and writes its
        // own three-field file; the new launcher keeps working and keeps its snapshot
        let mut incompatible = index_v2(44);
        incompatible["schema"] = json!("3.0");
        let moved = released::HighWater {
            source_id: "yoake-official".into(),
            sequence: 44,
            index_sha256: index_sha(&incompatible),
        };
        fs::write(directory.join(format!("{feed_id}.json")), serde_json::to_vec(&moved).unwrap())
            .unwrap();
        fixture.publish(44, &incompatible, DOMAIN);
        let feed = fixture.restart().fetch_feed().unwrap();
        assert!(feed.needs_update);
        assert_eq!(index_identity(&feed.index).1, 42, "snapshot older than the high-water mark");
        fixture.publish_with_presentation(45, &index_v2(45), &presentation());
        let feed = fixture.restart().fetch_feed().unwrap();
        assert!(!feed.needs_update);
        assert_eq!(index_identity(&feed.index).1, 45);
        let old: released::HighWater =
            serde_json::from_slice(&fs::read(directory.join(format!("{feed_id}.json"))).unwrap())
                .unwrap();
        assert_eq!(old.sequence, 45);
    }

    #[test]
    fn an_unreadable_snapshot_is_not_fatal() {
        let fixture = Fixture::new(None);
        fixture.publish(42, &index_v2(42), DOMAIN);
        fixture.client.fetch_feed().unwrap();
        let feed_id = sha256_hex(fixture.client.config.root.as_str().as_bytes());
        let snapshot = fixture.root.join(format!("yoake-feeds/{feed_id}.snapshot.json"));
        fs::write(&snapshot, b"garbage").unwrap();
        fixture.client.fetch_feed().unwrap();
        let mut future = index_v2(43);
        future["schema"] = json!("3.0");
        fixture.publish(43, &future, DOMAIN);
        fs::write(&snapshot, b"garbage").unwrap();
        assert_eq!(fixture.client.fetch_feed().unwrap_err(), FeedError::NeedsUpdate);
    }

    #[test]
    fn pack_categories_order_groups_and_leave_unknown_categories_last() {
        let index: FeedIndexV2 = serde_json::from_value(index_v2(42)).unwrap();
        let mut late = index.packs[0].clone();
        late.category = Some("late".to_owned());
        let mut early = late.clone();
        early.category = Some("early".to_owned());
        let mut other = late.clone();
        other.category = Some("unknown".to_owned());
        let packs = vec![late, early, other];
        let mut document = presentation();
        document["categories"] = json!([
            {"id": "late", "label": {"en": "Late"}, "order": 2},
            {"id": "early", "label": {"en": "Early"}, "order": 1}
        ]);
        let presentation = parse_presentation(&serde_json::to_vec(&document).unwrap()).unwrap();
        let groups = pack_groups(&packs, Some(&presentation));
        assert_eq!(groups.len(), 3);
        assert_eq!(groups[0].0.unwrap().id, "early");
        assert_eq!(groups[0].1[0].0, 1);
        assert_eq!(groups[1].0.unwrap().id, "late");
        assert_eq!(groups[1].1[0].0, 0);
        assert!(groups[2].0.is_none());
        assert_eq!(groups[2].1[0].0, 2);
        let groups = pack_groups(&packs, None);
        assert_eq!(groups.len(), 1);
        assert!(groups[0].0.is_none());
        assert_eq!(groups[0].1.iter().map(|(position, _)| *position).collect::<Vec<_>>(), [0, 1, 2]);
    }

    #[test]
    fn schema_two_build_identity_is_optional_but_validated_when_present() {
        let legacy: FeedIndexV2 = serde_json::from_value(index_v2(42)).unwrap();
        assert!(legacy.bases[0].yoake.is_none());
        assert!(validate_index_v2(&legacy).is_ok());
        let mut index = index_v2(42);
        index["bases"][0]["yoake"] = json!({
            "version": "1.0.0", "engine": "1.0.0+build.0123456789abcdef", "abi": 2
        });
        let valid: FeedIndexV2 = serde_json::from_value(index.clone()).unwrap();
        assert!(validate_index_v2(&valid).is_ok());
        index["bases"][0]["yoake"]["engine"] = json!("26.3");
        let invalid: FeedIndexV2 = serde_json::from_value(index).unwrap();
        assert!(validate_index_v2(&invalid).is_err());
    }

    #[test]
    fn accepts_python_signed_schema_two_fixture() {
        let mut fixture = Fixture::new(None);
        fixture.client.config.keys = parse_keys(include_str!(
            "../tests/fixtures/yoake-schema2/test-only-public-key.b64"
        ))
        .unwrap();
        fixture.transport.respond(
            "current.json",
            Ok(br#"{"source_id":"yoake-official","sequence":42}"#.to_vec()),
        );
        let files: [(&str, &[u8]); 3] = [
            (
                "index.json",
                include_bytes!("../tests/fixtures/yoake-schema2/index.json"),
            ),
            (
                "SHA256SUMS",
                include_bytes!("../tests/fixtures/yoake-schema2/SHA256SUMS"),
            ),
            (
                "SHA256SUMS.sig",
                include_bytes!("../tests/fixtures/yoake-schema2/SHA256SUMS.sig"),
            ),
        ];
        for (name, bytes) in files {
            fixture
                .transport
                .respond(&format!("indexes/000042/{name}"), Ok(bytes.to_vec()));
        }

        fixture.transport.respond(
            "indexes/000042/presentation.sums",
            Err(FeedError::HttpStatus(404)),
        );
        let feed = fixture.client.fetch_feed().unwrap();
        let ParsedFeedIndex::V2(index) = feed.index else {
            panic!("expected schema 2");
        };
        assert_eq!(index.sequence, 42);
        assert_eq!(index.bases[0].version, "1");
        assert_eq!(index.packs[0].id, "example");
        assert_eq!(index.packs[0].versions[0].base_version, "1");
        assert_eq!(
            index.packs[0].versions[0].targets["windows-x86_64"].files[0].destination,
            "config/test.toml"
        );
    }

    #[test]
    fn accepts_a_python_signed_presentation_bound_to_its_index() {
        let mut fixture = Fixture::new(None);
        fixture.client.config.keys = parse_keys(include_str!(
            "../tests/fixtures/yoake-presentation/test-only-public-key.b64"
        ))
        .unwrap();
        fixture.transport.respond(
            "current.json",
            Ok(br#"{"source_id":"yoake-official","sequence":42}"#.to_vec()),
        );
        let files: [(&str, &[u8]); 6] = [
            ("index.json", include_bytes!("../tests/fixtures/yoake-presentation/index.json")),
            ("SHA256SUMS", include_bytes!("../tests/fixtures/yoake-presentation/SHA256SUMS")),
            ("SHA256SUMS.sig", include_bytes!("../tests/fixtures/yoake-presentation/SHA256SUMS.sig")),
            ("presentation.json", include_bytes!("../tests/fixtures/yoake-presentation/presentation.json")),
            ("presentation.sums", include_bytes!("../tests/fixtures/yoake-presentation/presentation.sums")),
            (
                "presentation.sums.sig",
                include_bytes!("../tests/fixtures/yoake-presentation/presentation.sums.sig"),
            ),
        ];
        for (name, bytes) in files {
            fixture
                .transport
                .respond(&format!("indexes/000042/{name}"), Ok(bytes.to_vec()));
        }
        let feed = fixture.client.fetch_feed().unwrap();
        let presentation = feed.presentation.as_ref().expect("the Python presentation verifies");
        assert_eq!(localized(&presentation.news[0].title, "ja"), "ようこそ");
        assert_eq!(presentation.news[0].link.as_deref(), Some("https://example.com/news"));
        assert_eq!(presentation.categories[0].packs, ["example"]);
        assert!(!feed.needs_update);
    }

    #[test]
    fn rejects_invalid_schema_two_indexes() {
        let fixture = Fixture::new(None);
        let mut cases = Vec::new();

        let mut invalid = index_v2(42);
        invalid["schema"] = json!(3);
        cases.push(("unsupported schema", invalid));

        let mut invalid = index_v2(42);
        invalid["packs"][0]["id"] = json!(format!("bad{}name", '\u{22}'));
        cases.push(("invalid Windows identifier", invalid));

        let mut invalid = index_v2(42);
        let duplicate = invalid["bases"][0].clone();
        invalid["bases"].as_array_mut().unwrap().push(duplicate);
        cases.push(("duplicate base version", invalid));

        let mut invalid = index_v2(42);
        let mut duplicate = invalid["packs"][0].clone();
        duplicate["id"] = json!("EXAMPLE");
        invalid["packs"].as_array_mut().unwrap().push(duplicate);
        cases.push(("duplicate pack id", invalid));

        let mut invalid = index_v2(42);
        let duplicate = invalid["packs"][0]["versions"][0].clone();
        invalid["packs"][0]["versions"]
            .as_array_mut()
            .unwrap()
            .push(duplicate);
        cases.push(("duplicate pack version", invalid));

        let mut invalid = index_v2(42);
        invalid["packs"][0]["versions"][0]["base_version"] = json!("missing");
        cases.push(("missing base", invalid));

        let mut invalid = index_v2(42);
        let target = invalid["packs"][0]["versions"][0]["targets"]["windows-x86_64"].clone();
        invalid["packs"][0]["versions"][0]["targets"] = json!({"linux-x86_64": target});
        cases.push(("incompatible target", invalid));

        let mut invalid = index_v2(42);
        invalid["bases"][0]["targets"] = json!({});
        cases.push(("empty base targets", invalid));

        let mut invalid = index_v2(42);
        invalid["packs"][0]["versions"][0]["targets"] = json!({});
        cases.push(("empty pack targets", invalid));

        let mut invalid = index_v2(42);
        invalid["bases"][0]["targets"]["windows-x86_64"]["files"][0]["scope"] = json!("instance");
        cases.push(("base scope", invalid));

        let mut invalid = index_v2(42);
        invalid["bases"][0]["targets"]["windows-x86_64"]["files"][0]["destination"] =
            json!("other.exe");
        cases.push(("base destination", invalid));

        let mut invalid = index_v2(42);
        pack_file_v2(&mut invalid)["scope"] = json!("version");
        cases.push(("pack scope", invalid));

        let mut invalid = index_v2(42);
        pack_file_v2(&mut invalid)["policy"] = json!("replace");
        cases.push(("pack policy", invalid));

        let mut invalid = index_v2(42);
        pack_file_v2(&mut invalid)
            .as_object_mut()
            .unwrap()
            .remove("policy");
        cases.push(("missing pack policy", invalid));

        let mut invalid = index_v2(42);
        invalid["bases"][0]["research_lock_sha256"] = json!("a".repeat(63));
        cases.push(("invalid base hash", invalid));

        let mut invalid = index_v2(42);
        invalid["packs"][0]["icon"]["size"] = json!(MAX_ICON + 1);
        cases.push(("oversize icon", invalid));

        let mut invalid = index_v2(42);
        pack_file_v2(&mut invalid)["size"] = json!(MAX_FILE + 1);
        cases.push(("oversize file", invalid));

        let mut invalid = index_v2(42);
        invalid["packs"][0]["versions"][0]["enabled_native_mods"] = json!(["native", "NATIVE"]);
        cases.push(("duplicate native mod", invalid));

        let mut invalid = index_v2(42);
        invalid["packs"][0]["versions"][0]["enabled_native_mods"] = json!(["other"]);
        cases.push(("native mod not in base", invalid));

        let mut invalid = index_v2(42);
        invalid["bases"][0]["native_mods"] = json!(["native", "NATIVE"]);
        cases.push(("duplicate base native mod", invalid));

        let mut invalid = index_v2(42);
        invalid["packs"][0]["versions"][0]["description"] = json!("");
        cases.push(("empty description", invalid));

        let mut invalid = index_v2(42);
        pack_file_v2(&mut invalid)["destination"] = json!("config/../bad.toml");
        cases.push(("unsafe destination", invalid));

        let mut invalid = index_v2(42);
        let mut duplicate = pack_file_v2(&mut invalid).clone();
        duplicate["destination"] = json!("CONFIG/TEST.TOML");
        invalid["packs"][0]["versions"][0]["targets"]["windows-x86_64"]["files"]
            .as_array_mut()
            .unwrap()
            .push(duplicate);
        cases.push(("case-insensitive collision", invalid));

        let mut invalid = index_v2(42);
        let mut child = pack_file_v2(&mut invalid).clone();
        child["destination"] = json!("config/test.toml/child.toml");
        invalid["packs"][0]["versions"][0]["targets"]["windows-x86_64"]["files"]
            .as_array_mut()
            .unwrap()
            .push(child);
        cases.push(("file-directory collision", invalid));

        for (description, document) in cases {
            fixture.publish(42, &document, DOMAIN);
            assert!(fixture.client.fetch_feed().is_err(), "{description}");
        }
    }

    #[test]
    fn schema_two_targets_reject_duplicate_json_keys() {
        let document = index_v2(42);
        let serialized = serde_json::to_string(&document).unwrap();
        let target =
            serde_json::to_string(&document["bases"][0]["targets"]["windows-x86_64"]).unwrap();
        let entry = format!("{:?}:{target}", "windows-x86_64");
        let duplicate = serialized.replacen(&entry, &format!("{entry},{entry}"), 1);
        assert_ne!(serialized, duplicate);
        assert!(serde_json::from_str::<FeedIndexV2>(&duplicate).is_err());
    }

    #[test]
    fn schema_two_pack_paths_enforce_publisher_allowlist() {
        for (path, policy) in [
            ("horizon-mods/example.hmod", Policy::Managed),
            ("resourcepacks/example.zip", Policy::Managed),
            ("datapacks/example.zip", Policy::Managed),
            ("config/nested/example.toml", Policy::DefaultOnCreate),
            ("saves/world/level.dat", Policy::DefaultOnCreate),
            ("saves/world/datapacks/example.zip", Policy::DefaultOnCreate),
        ] {
            assert!(validate_pack_destination(path, policy).is_ok(), "{path}");
        }
        for (path, policy) in [
            ("config/../example.toml", Policy::Managed),
            ("config/facts.json", Policy::Managed),
            ("config/example.jar", Policy::Managed),
            ("config/com/mojang/example.json", Policy::Managed),
            ("horizon-mods/nested/example.hmod", Policy::Managed),
            ("saves/world/level.dat", Policy::Managed),
            ("saves/world/file.zip", Policy::DefaultOnCreate),
        ] {
            assert!(validate_pack_destination(path, policy).is_err(), "{path}");
        }
        let invalid = format!("config/bad{}name.toml", '\u{22}');
        assert!(validate_pack_destination(&invalid, Policy::Managed).is_err());
    }

    #[test]
    fn checks_signatures_and_hashes_before_schema_dispatch() {
        let fixture = Fixture::new(None);
        let mut unsupported = index_v2(42);
        unsupported["schema"] = json!(3);
        fixture.publish(42, &unsupported, "wrong-domain");
        assert_eq!(
            fixture.client.fetch_feed().unwrap_err(),
            FeedError::BadSignature
        );
        fixture.publish(42, &unsupported, DOMAIN);
        fixture
            .transport
            .respond("indexes/000042/index.json", Ok(b"tampered".to_vec()));
        assert_eq!(
            fixture.client.fetch_feed().unwrap_err(),
            FeedError::HashMismatch
        );
        fixture.publish(42, &unsupported, DOMAIN);
        assert_eq!(
            fixture.client.fetch_feed().unwrap_err(),
            FeedError::NeedsUpdate
        );
    }

    #[test]
    fn replay_protection_spans_both_feed_schemas() {
        let fixture = Fixture::new(None);
        fixture.publish(41, &index(41), DOMAIN);
        fixture.client.fetch_feed().unwrap();
        fixture.publish(42, &index_v2(42), DOMAIN);
        fixture.client.fetch_feed().unwrap();
        fixture.publish(41, &index(41), DOMAIN);
        assert_eq!(fixture.client.fetch_feed().unwrap_err(), FeedError::Replay);
        let mut equivocation = index_v2(42);
        equivocation["packs"][0]["name"] = json!("Another pack name");
        fixture.publish(42, &equivocation, DOMAIN);
        assert_eq!(fixture.client.fetch_feed().unwrap_err(), FeedError::Replay);
    }

    #[test]
    fn valid_feed_and_object_are_verified_with_authorization() {
        let fixture = Fixture::new(Some("Bearer test-token"));
        let icon = b"verified icon";
        let mut document = index(42);
        document["packs"][0]["icon"] = json!({
            "sha256": sha256_hex(icon),
            "size": icon.len(),
        });
        fixture.publish(42, &document, DOMAIN);
        let feed = fixture.client.fetch_feed().unwrap();
        let ParsedFeedIndex::V1(index) = &feed.index else {
            panic!("expected schema 1");
        };
        assert_eq!(index.sequence, 42);
        let object = &index.packs[0].icon;
        fixture.transport.respond(
            &format!("objects/sha256/{}", object.sha256),
            Ok(icon.to_vec()),
        );
        assert_eq!(fixture.client.fetch_object(object).unwrap(), icon.to_vec());
        let requests = fixture.transport.requests.lock().unwrap();
        // pointer, sums, signature, index, the (absent) presentation, icon
        assert_eq!(requests.len(), 6);
        assert!(requests
            .iter()
            .all(|(_, header)| header.as_deref() == Some("Bearer test-token")));
        drop(requests);
        fixture.transport.respond(
            &format!("objects/sha256/{}", object.sha256),
            Ok(b"tampered icon".to_vec()),
        );
        assert_eq!(
            fixture.client.fetch_object(object).unwrap_err(),
            FeedError::HashMismatch
        );
    }

    #[test]
    fn rejects_bad_signature_and_wrong_domain() {
        let fixture = Fixture::new(None);
        fixture.publish(42, &index(42), DOMAIN);
        fixture.transport.respond(
            "indexes/000042/SHA256SUMS.sig",
            Ok(b"not a signature".to_vec()),
        );
        assert_eq!(
            fixture.client.fetch_feed().unwrap_err(),
            FeedError::BadSignature
        );
        fixture.publish(42, &index(42), "wrong-domain");
        assert_eq!(
            fixture.client.fetch_feed().unwrap_err(),
            FeedError::BadSignature
        );
    }

    #[test]
    fn rejects_hash_mismatch_and_oversize_index() {
        let fixture = Fixture::new(None);
        fixture.publish(42, &index(42), DOMAIN);
        fixture
            .transport
            .respond("indexes/000042/index.json", Ok(b"tampered".to_vec()));
        assert_eq!(
            fixture.client.fetch_feed().unwrap_err(),
            FeedError::HashMismatch
        );
        fixture
            .transport
            .respond("indexes/000042/index.json", Ok(vec![b' '; MAX_INDEX + 1]));
        assert_eq!(
            fixture.client.fetch_feed().unwrap_err(),
            FeedError::TooLarge
        );
    }

    #[test]
    fn rejects_lower_sequence_and_same_sequence_equivocation() {
        let fixture = Fixture::new(None);
        fixture.publish(42, &index(42), DOMAIN);
        fixture.client.fetch_feed().unwrap();
        let restarted_client = FeedClient::with_components(
            fixture.client.config.clone(),
            Arc::new(FixedAuth(None)),
            fixture.transport.clone(),
            fixture.root.clone(),
        );
        restarted_client.fetch_feed().unwrap();
        fixture.publish(41, &index(41), DOMAIN);
        assert_eq!(
            restarted_client.fetch_feed().unwrap_err(),
            FeedError::Replay
        );
        let mut changed = index(42);
        changed["packs"][0]["name"] = json!("Different signed index");
        fixture.publish(42, &changed, DOMAIN);
        assert_eq!(fixture.client.fetch_feed().unwrap_err(), FeedError::Replay);
    }

    #[test]
    fn rejects_a_different_signed_source_after_first_acceptance() {
        let mut fixture = Fixture::new(None);
        fixture.client.config.source_id = None;
        fixture.publish(42, &index(42), DOMAIN);
        fixture.client.fetch_feed().unwrap();
        let mut changed = index(43);
        changed["source_id"] = json!("another-source");
        fixture.publish(43, &changed, DOMAIN);
        fixture.transport.respond(
            "current.json",
            Ok(serde_json::to_vec(&json!({
                "source_id": "another-source",
                "sequence": 43,
            }))
            .unwrap()),
        );
        assert_eq!(
            fixture.client.fetch_feed().unwrap_err(),
            FeedError::WrongSource
        );
    }

    #[test]
    fn rejects_unsafe_destinations_and_collisions() {
        let fixture = Fixture::new(None);
        for destination in [
            "../world",
            "/absolute",
            "C:/drive",
            "dir\\file",
            "dir//file",
            "con.txt",
            "dir/../file",
            "dir/name.",
            "dir/name ",
        ] {
            let mut document = index(42);
            document["packs"][0]["versions"][0]["targets"]["windows-x86_64"]["files"][0]
                ["destination"] = json!(destination);
            fixture.publish(42, &document, DOMAIN);
            assert_eq!(
                fixture.client.fetch_feed().unwrap_err(),
                FeedError::UnsafePath,
                "{destination}"
            );
        }
        let mut document = index(42);
        let files = document["packs"][0]["versions"][0]["targets"]["windows-x86_64"]["files"]
            .as_array_mut()
            .unwrap();
        let mut duplicate = files[0].clone();
        duplicate["destination"] = json!("HORIZON-MODS/EXAMPLE.HMOD");
        files.push(duplicate);
        fixture.publish(42, &document, DOMAIN);
        assert_eq!(
            fixture.client.fetch_feed().unwrap_err(),
            FeedError::UnsafePath
        );
    }

    #[cfg(unix)]
    #[test]
    fn rejects_existing_symlinks_in_destination() {
        let fixture = Fixture::new(None);
        fs::create_dir_all(&fixture.root).unwrap();
        std::os::unix::fs::symlink(&fixture.root, fixture.root.join("linked")).unwrap();
        assert_eq!(
            validate_existing_path(&fixture.root, "linked/file"),
            Err(FeedError::UnsafePath)
        );
    }

    #[test]
    fn unauthorized_response_needs_login() {
        for status in [401, 403] {
            let fixture = Fixture::new(None);
            fixture
                .transport
                .respond("current.json", Err(status_error(status)));
            assert_eq!(
                fixture.client.fetch_feed().unwrap_err(),
                FeedError::NeedsLogin
            );
        }
    }

    #[test]
    fn requests_without_a_token_have_no_authorization_header() {
        let fixture = Fixture::new(None);
        fixture.publish(42, &index(42), DOMAIN);
        fixture.client.fetch_feed().unwrap();
        assert!(fixture
            .transport
            .requests
            .lock()
            .unwrap()
            .iter()
            .all(|(_, header)| header.is_none()));
    }

    #[test]
    fn unconfigured_build_has_no_feed() {
        if option_env!("YOAKE_COMPILED_FEED_URL").is_none() {
            assert!(FeedConfig::compiled().unwrap().is_none());
        }
    }
}
