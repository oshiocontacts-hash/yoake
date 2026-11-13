use super::*;

mod fixtures;
mod lifecycle;
mod local;

use fixtures::Fixture;

#[test]
fn catalog_verifies_for_members_and_invited_players() {
    for token in ["Bearer member-token", "Bearer invited-player-token"] {
        let fixture = Fixture::with_token(Some(token));
        let catalog = fixture.publish(&[]);
        assert!(!catalog.cached);
        assert!(catalog.catalog.entries.is_empty());
        let requests = fixture.http.requests.lock().unwrap();
        assert_eq!(requests.len(), 2);
        assert!(requests.iter().all(|(url, auth)| url
            .starts_with("https://portal.yunipop.com/yoake/mods/")
            && auth.as_deref() == Some(token)));
    }
}

#[test]
fn configured_portal_base_path_matches_launcher_updates() {
    let fixture = Fixture::new();
    let catalog = fixture.publish(&[]);
    let mut client = fixture.client.clone();
    client.root = Client::with_components(
        "https://portal.yunipop.com/staging",
        client.keys.clone(),
        client.data.clone(),
        client.auth.clone(),
        client.http.clone(),
    )
    .unwrap()
    .root;
    assert_eq!(client.root.as_str(), "https://portal.yunipop.com/staging/");
    fixture.http.respond(
        "https://portal.yunipop.com/staging/yoake/mods/index.json",
        Ok(catalog.proof.json.into_bytes()),
    );
    fixture.http.respond(
        "https://portal.yunipop.com/staging/yoake/mods/index.json.sig",
        Ok(catalog.proof.signature.into_bytes()),
    );
    assert!(client.refresh().is_ok());
    assert!(fixture
        .http
        .requests
        .lock()
        .unwrap()
        .iter()
        .any(|(url, _)| url == "https://portal.yunipop.com/staging/yoake/mods/index.json.sig"));
}

#[test]
fn mods_need_an_exact_signed_build_identity_not_a_minecraft_release() {
    use crate::yoake_feed::YoakeBuildIdentity;
    assert!(matches!(
        InstanceEnvironment::from_identity(None),
        Err(Error::Incompatible(_))
    ));
    let identity = YoakeBuildIdentity {
        version: "1.0.0".into(),
        engine: "1.0.0+build.0123456789abcdef".into(),
        abi: 2,
    };
    let environment = InstanceEnvironment::from_identity(Some(&identity)).unwrap();
    assert_eq!(environment.yoake.to_string(), "1.0.0");
    for invalid in [
        YoakeBuildIdentity {
            version: "26.3".into(),
            ..identity.clone()
        },
        YoakeBuildIdentity {
            engine: "1.0.0+build.0123456789abcdeF".into(),
            ..identity.clone()
        },
        YoakeBuildIdentity {
            engine: "1.1.0+build.0123456789abcdef".into(),
            ..identity.clone()
        },
        YoakeBuildIdentity { abi: 0, ..identity },
    ] {
        assert!(matches!(
            InstanceEnvironment::from_identity(Some(&invalid)),
            Err(Error::Incompatible(_))
        ));
    }
}

#[test]
fn invalid_catalog_signature_never_falls_back_to_cache() {
    let fixture = Fixture::new();
    let catalog = fixture.publish(&[]);
    let wrong_context = fixture.key.sign(
        horizon_release_sign::YOAKE_YAK_V1,
        catalog.proof.json.as_bytes(),
    );
    fixture.http.respond(
        "https://portal.yunipop.com/yoake/mods/index.json.sig",
        Ok(wrong_context.into_bytes()),
    );
    assert!(matches!(
        fixture.client.refresh(),
        Err(Error::Feed(FeedError::BadSignature))
    ));
}

#[test]
fn signed_out_and_unauthorized_catalogs_require_login() {
    let signed_out = Fixture::with_token(None);
    assert!(matches!(
        signed_out.client.refresh(),
        Err(Error::Feed(FeedError::NeedsLogin))
    ));
    assert!(signed_out.http.requests.lock().unwrap().is_empty());
    let fixture = Fixture::new();
    fixture.publish(&[]);
    fixture.fail_index(FeedError::NeedsLogin);
    assert!(matches!(
        fixture.client.refresh(),
        Err(Error::Feed(FeedError::NeedsLogin))
    ));
    fixture.fail_index(FeedError::HttpStatus(401));
    assert!(matches!(
        fixture.client.refresh(),
        Err(Error::Feed(FeedError::NeedsLogin))
    ));
}

#[test]
fn offline_catalog_reverifies_the_cached_signature() {
    let fixture = Fixture::new();
    fixture.publish(&[]);
    fixture.fail_index(FeedError::Network);
    assert!(fixture.client.refresh().unwrap().cached);
    let path = fixture.client.data.join("yoake-mods-catalog.json");
    let mut proof: Proof = store::read_json(&path).unwrap();
    proof.json.push(' ');
    store::write_json(&path, &proof).unwrap();
    assert!(fixture.client.refresh().is_err());
}

#[test]
fn signed_catalog_cannot_send_credentials_to_another_origin() {
    let fixture = Fixture::new();
    let package = fixture.package(fixture.manifest("example", "1.0.0"), false);
    let mut catalog = fixture.publish(&[package.version]);
    catalog.catalog.entries[0].versions[0].url =
        "https://other.example/yoake/mods/example/1.0.0/example-1.0.0.yak".into();
    let json = serde_json::to_string(&catalog.catalog).unwrap();
    let proof = Proof {
        signature: fixture
            .key
            .sign(horizon_release_sign::YOAKE_MODS_V1, json.as_bytes()),
        json,
    };
    assert!(fixture.client.verify(proof, false).is_err());
    assert!(fixture
        .http
        .requests
        .lock()
        .unwrap()
        .iter()
        .all(|(url, _)| !url.contains("other.example")));
}

#[test]
fn incompatible_yoake_engine_abi_os_and_server_only_are_refused() {
    let fixture = Fixture::new();
    let mut too_new = fixture.manifest("too-new", "1.0.0");
    too_new.yoake.version = ">=2".parse().unwrap();
    let package = fixture.package(too_new, false);
    let catalog = fixture.publish(&[package.version]);
    assert!(matches!(
        fixture.plan(&catalog, "too-new"),
        Err(Error::Incompatible(_))
    ));
    let mut program = fixture.program("program");
    program.bin.as_mut().unwrap().engine = "1.0.0+build.other".into();
    assert!(matches!(
        fixture.environment.check(&program),
        Err(Error::Incompatible(_))
    ));
    let package = fixture.package(program, false);
    let catalog = fixture.publish(&[package.version]);
    assert!(matches!(
        fixture.plan(&catalog, "program"),
        Err(Error::Incompatible(_))
    ));
    let mut program = fixture.program("program");
    program.bin.as_mut().unwrap().abi += 1;
    assert!(fixture.environment.check(&program).is_err());
    let mut program = fixture.program("program");
    program.bin.as_mut().unwrap().targets.clear();
    assert!(fixture.environment.check(&program).is_err());
    let mut data = fixture.manifest("server-only", "1.0.0");
    data.side.client = false;
    assert!(fixture.environment.check(&data).is_err());
    let mut unknown_build = fixture.environment.clone();
    unknown_build.engine.clear();
    assert!(unknown_build.check(&fixture.program("program")).is_err());
}

#[test]
fn download_size_and_hash_are_checked_before_extraction() {
    for corruption in ["short", "long", "hash"] {
        let fixture = Fixture::new();
        let package = fixture.package(fixture.manifest("example", "1.0.0"), false);
        let catalog = fixture.publish(&[package.version.clone()]);
        let mut bytes = package.bytes.clone();
        match corruption {
            "short" => {
                bytes.pop();
            }
            "long" => bytes.push(0),
            _ => bytes[0] ^= 1,
        }
        fixture.http.respond(&package.version.url, Ok(bytes));
        let plan = fixture.plan(&catalog, "example").unwrap();
        assert!(fixture.apply(plan, true).is_err());
        assert!(!store::directory(&fixture.home, &package.version.manifest).exists());
        assert!(!fixture.home.join("yoake-mods.txt").exists());
        assert!(!store::archive_path(&fixture.home, &package.version.sha256)
            .unwrap()
            .exists());
    }
}
