use super::*;
use crate::yoake_auth::AuthError;
use crate::yoake_configs::{BUILTIN_PATH, BuiltinCatalog};
use horizon_release_sign::{SigningKey, YOAKE_BUILTIN_V1, YOAKE_MODS_V1};
use serde_json::json;
use std::collections::BTreeMap;
use std::sync::Mutex;

#[derive(Default)]
struct FakeTransport {
    responses: Mutex<BTreeMap<String, Result<Vec<u8>, FeedError>>>,
    requests: Mutex<Vec<String>>,
}

impl FeedTransport for FakeTransport {
    fn get(&self, url: &str, _authorization: Option<&str>, _limit: usize) -> Result<Vec<u8>, FeedError> {
        self.requests.lock().unwrap().push(url.to_owned());
        self.responses
            .lock()
            .unwrap()
            .get(url)
            .cloned()
            .unwrap_or(Err(FeedError::Network))
    }
}

struct Auth;

impl YoakeAuth for Auth {
    fn authorization(&self) -> Result<Option<String>, AuthError> {
        Ok(Some("Bearer test".into()))
    }
}

struct Fixture {
    directory: PathBuf,
    key: SigningKey,
    transport: Arc<FakeTransport>,
    client: Client,
}

impl Fixture {
    fn new(name: &str) -> Self {
        let directory =
            std::env::temp_dir().join(format!("yoake-builtin-{name}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&directory);
        std::fs::create_dir_all(&directory).unwrap();
        let (key, _) = SigningKey::generate().unwrap();
        let transport = Arc::new(FakeTransport::default());
        let client = Client::with_components(
            "https://portal.test/",
            vec![key.public()],
            directory.clone(),
            Arc::new(Auth),
            transport.clone(),
        )
        .unwrap();
        Self {
            directory,
            key,
            transport,
            client,
        }
    }

    fn document(generated: &str, mod_version: &str) -> serde_json::Value {
        json!({
            "schema": "1.0",
            "kind": "yoake-builtin-catalog",
            "generated": generated,
            "mods": [{"id": "nestgate", "version": mod_version, "author": "Yoake",
                       "name": {"en": "Nestgate"}, "description": {"en": "Voice"}}]
        })
    }

    fn serve_signed(&self, document: &serde_json::Value, domain: &str) {
        let text = document.to_string();
        let signature = self.key.sign(domain, text.as_bytes());
        let envelope = json!({"json": text, "signature": signature});
        self.serve(Ok(envelope.to_string().into_bytes()));
    }

    fn serve(&self, response: Result<Vec<u8>, FeedError>) {
        self.transport
            .responses
            .lock()
            .unwrap()
            .insert(format!("https://portal.test/{BUILTIN_PATH}"), response);
    }

    fn cached_generated(&self) -> String {
        let proof: Proof = store::read_json(&self.directory.join("yoake-builtin-catalog.json")).unwrap();
        BuiltinCatalog::parse(proof.json.as_bytes()).unwrap().generated
    }
}

impl Drop for Fixture {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.directory);
    }
}

fn version_of(refresh: &BuiltinRefresh) -> Option<&str> {
    refresh.catalog.as_ref().map(|catalog| catalog.mods[0].version.as_str())
}

#[test]
fn a_published_catalog_is_one_request_and_is_cached() {
    let fixture = Fixture::new("fetch");
    fixture.serve_signed(&Fixture::document("2026-11-14T00:00:00Z", "0.1.0"), YOAKE_BUILTIN_V1);
    let refresh = fixture.client.refresh_builtin();
    assert!(refresh.problem.is_none());
    assert_eq!(version_of(&refresh), Some("0.1.0"));
    assert_eq!(fixture.transport.requests.lock().unwrap().len(), 1, "no separate signature file");
    assert_eq!(fixture.cached_generated(), "2026-11-14T00:00:00Z");
    // a server without a built-in catalog simply has none
    fixture.serve(Err(FeedError::HttpStatus(404)));
    let refresh = fixture.client.refresh_builtin();
    assert!(refresh.catalog.is_none() && refresh.problem.is_none());
}

#[test]
fn an_older_catalog_never_replaces_a_newer_one() {
    let fixture = Fixture::new("rollback");
    fixture.serve_signed(&Fixture::document("2026-11-14T12:00:00Z", "0.2.0"), YOAKE_BUILTIN_V1);
    assert_eq!(version_of(&fixture.client.refresh_builtin()), Some("0.2.0"));
    // an older, validly signed catalog is served again (rollback / replay)
    fixture.serve_signed(&Fixture::document("2026-11-13T00:00:00Z", "0.1.0"), YOAKE_BUILTIN_V1);
    let refresh = fixture.client.refresh_builtin();
    assert_eq!(version_of(&refresh), Some("0.2.0"), "the cached, newer copy stays");
    assert!(matches!(refresh.problem, Some(Error::Feed(FeedError::Replay))));
    assert_eq!(fixture.cached_generated(), "2026-11-14T12:00:00Z");
    // a newer one is taken
    fixture.serve_signed(&Fixture::document("2026-11-15T00:00:00Z", "0.3.0"), YOAKE_BUILTIN_V1);
    let refresh = fixture.client.refresh_builtin();
    assert_eq!((version_of(&refresh), refresh.problem.is_none()), (Some("0.3.0"), true));
    assert_eq!(fixture.cached_generated(), "2026-11-15T00:00:00Z");
}

#[test]
fn problems_keep_the_last_good_catalog_and_say_why() {
    let fixture = Fixture::new("problems");
    // nothing cached yet: the problem alone
    fixture.serve(Err(FeedError::Network));
    let refresh = fixture.client.refresh_builtin();
    assert!(refresh.catalog.is_none());
    assert!(matches!(refresh.problem, Some(Error::Feed(FeedError::Network))));
    let mut future = Fixture::document("2026-11-14T00:00:00Z", "9.9.9");
    future["schema"] = json!("2.0");
    fixture.serve_signed(&future, YOAKE_BUILTIN_V1);
    let refresh = fixture.client.refresh_builtin();
    assert!(refresh.catalog.is_none());
    assert!(matches!(refresh.problem, Some(Error::Incompatible(_))));

    fixture.serve_signed(&Fixture::document("2026-11-14T00:00:00Z", "0.1.0"), YOAKE_BUILTIN_V1);
    assert_eq!(version_of(&fixture.client.refresh_builtin()), Some("0.1.0"));
    // offline: the cached copy, no complaint
    for failure in [FeedError::Network, FeedError::HttpStatus(503)] {
        fixture.serve(Err(failure));
        let refresh = fixture.client.refresh_builtin();
        assert_eq!((version_of(&refresh), refresh.problem.is_none()), (Some("0.1.0"), true));
    }
    // everything else keeps the cached copy too, with the reason
    let mut future = Fixture::document("2026-11-16T00:00:00Z", "9.9.9");
    future["schema"] = json!("2.0");
    fixture.serve_signed(&future, YOAKE_BUILTIN_V1);
    let refresh = fixture.client.refresh_builtin();
    assert_eq!(version_of(&refresh), Some("0.1.0"));
    assert!(matches!(refresh.problem, Some(Error::Incompatible(_))));
    let mut critical = Fixture::document("2026-11-16T00:00:00Z", "9.9.9");
    critical["critical_features"] = json!(["something-new"]);
    fixture.serve_signed(&critical, YOAKE_BUILTIN_V1);
    let refresh = fixture.client.refresh_builtin();
    assert_eq!(version_of(&refresh), Some("0.1.0"));
    assert!(matches!(refresh.problem, Some(Error::Incompatible(_))));
    fixture.serve(Err(FeedError::NeedsLogin));
    let refresh = fixture.client.refresh_builtin();
    assert_eq!(version_of(&refresh), Some("0.1.0"));
    assert!(matches!(refresh.problem, Some(Error::Feed(FeedError::NeedsLogin))));
    fixture.serve(Ok(b"not an envelope".to_vec()));
    assert_eq!(version_of(&fixture.client.refresh_builtin()), Some("0.1.0"));
    assert_eq!(fixture.cached_generated(), "2026-11-14T00:00:00Z");
}

#[test]
fn only_the_builtin_domain_is_accepted() {
    let fixture = Fixture::new("domain");
    for domain in [YOAKE_MODS_V1, "yoake-index-v1"] {
        fixture.serve_signed(&Fixture::document("2026-11-14T00:00:00Z", "0.1.0"), domain);
        let refresh = fixture.client.refresh_builtin();
        assert!(refresh.catalog.is_none(), "{domain}");
        assert!(refresh.problem.is_some(), "{domain}");
    }
    // a catalog whose text was changed after signing
    let text = Fixture::document("2026-11-14T00:00:00Z", "0.1.0").to_string();
    let signature = fixture.key.sign(YOAKE_BUILTIN_V1, text.as_bytes());
    let altered = text.replace("0.1.0", "6.6.6");
    fixture.serve(Ok(json!({"json": altered, "signature": signature}).to_string().into_bytes()));
    assert!(fixture.client.refresh_builtin().catalog.is_none());
}

#[test]
fn a_builtin_refresh_touches_only_its_own_path_and_cache() {
    let fixture = Fixture::new("independent");
    fixture.serve(Err(FeedError::NeedsLogin));
    let _ = fixture.client.refresh_builtin();
    let requests = fixture.transport.requests.lock().unwrap();
    assert!(requests.iter().all(|url| url.ends_with(BUILTIN_PATH)), "{requests:?}");
    assert!(!fixture.directory.join("yoake-mods-catalog.json").exists());
}
