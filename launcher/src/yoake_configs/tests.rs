use super::*;
use horizon_mods::yak::{CatalogEntry, CatalogVersion, Manifest, Side, Yoake};
use horizon_release_sign::SigningKey;
use serde_json::json;

fn builtin() -> BuiltinCatalog {
    BuiltinCatalog::parse(
        json!({
            "schema": "1.3",
            "kind": "yoake-builtin-catalog",
            "generated": "2026-11-14T00:00:00Z",
            "future_field": {"ignored": true},
            "mods": [
                {"id": "nestgate", "version": "0.1.0", "author": "Yoake",
                 "name": {"en": "Nestgate", "ja": "ネストゲート"},
                 "description": {"en": "Voice", "ja": "ボイス"},
                 "requires": [], "requires_engine": ["0.2.0-jinro.1"], "extra": 1},
                {"id": "yasagashi", "version": "0.1.0", "author": "Yoake",
                 "name": {"en": "Yasagashi"}, "description": {"en": "Werewolf"},
                 "requires": ["nestgate"], "requires_engine": ["0.2.0-jinro.1"]},
                {"id": "kagerou", "version": "0.1.1", "author": "Yoake",
                 "name": {"en": "Kagerou"}, "description": {"en": "Effects"}}
            ]
        })
        .to_string()
        .as_bytes(),
    )
    .unwrap()
}

fn yak_version(id: &str, version: &str) -> CatalogVersion {
    let version: Version = version.parse().unwrap();
    CatalogVersion {
        version: version.clone(),
        url: format!("https://portal.test/yoake/mods/{id}/{version}/{id}-{version}.yak"),
        size: 1,
        sha256: "a".repeat(64),
        manifest: Manifest {
            format: 1,
            id: id.into(),
            version,
            name: format!("Yak {id}"),
            name_ja: Some(format!("ヤク {id}")),
            authors: vec!["Someone".into()],
            description: "A yak".into(),
            license: None,
            homepage: None,
            yoake: Yoake {
                version: ">=1".parse().unwrap(),
            },
            side: Side {
                client: true,
                server: false,
                required_on_join: false,
            },
            dependencies: Default::default(),
            optional: Default::default(),
            conflicts: Default::default(),
            bin: None,
            shaders: Vec::new(),
            services: Vec::new(),
        },
        targets: Vec::new(),
        engine: None,
    }
}

fn yak_catalog(entries: &[(&str, &[&str])]) -> Catalog {
    Catalog {
        schema: 1,
        generated: "2026-11-14T00:00:00Z".into(),
        entries: entries
            .iter()
            .map(|(id, versions)| CatalogEntry {
                id: (*id).into(),
                versions: versions.iter().map(|v| yak_version(id, v)).collect(),
            })
            .collect(),
    }
}

fn bases() -> BTreeMap<String, Vec<String>> {
    BTreeMap::from([(
        "yasagashi-jinro".to_owned(),
        vec![
            "nestgate".to_owned(),
            "yasagashi".to_owned(),
            "kagerou".to_owned(),
        ],
    )])
}

fn configuration(mods: &[(&str, &str)]) -> Configuration {
    Configuration {
        name: "友だちと人狼".into(),
        base: "yasagashi-jinro".into(),
        mods: mods
            .iter()
            .map(|(id, version)| ModRef {
                id: (*id).into(),
                version: (*version).into(),
            })
            .collect(),
        settings: BTreeMap::new(),
    }
}

fn forged(json: &[u8]) -> String {
    let mut encoder = flate2::write::DeflateEncoder::new(Vec::new(), flate2::Compression::best());
    encoder.write_all(json).unwrap();
    let mut payload = encoder.finish().unwrap();
    let sum = checksum(&payload);
    payload.extend_from_slice(&sum);
    format!("YK1-{}", base32_encode(&payload))
}

#[test]
fn share_code_round_trip_with_the_documented_shape() {
    let original = configuration(&[("yasagashi", "latest"), ("kagerou", "0.1.1")]);
    let code = encode(&original).unwrap();
    assert!(code.starts_with("YK1-"));
    for group in code.trim_start_matches("YK1-").split('-') {
        assert!((1..=5).contains(&group.len()), "{group}");
        assert!(group.bytes().all(|b| ALPHABET.contains(&b)));
    }
    assert_eq!(decode(&code).unwrap(), original);
    // forgiving input: lower case, spaces and line breaks
    let sloppy = format!(" {}\n", code.to_lowercase());
    assert_eq!(decode(&sloppy).unwrap(), original);
    assert_eq!(encode(&decode(&code).unwrap()).unwrap(), code);
}

#[test]
fn damaged_codes_fail_the_checksum_or_the_alphabet() {
    let code = encode(&configuration(&[("nestgate", "latest")])).unwrap();
    let body = code.trim_start_matches("YK1-").replace('-', "");
    let mut flipped: Vec<char> = body.chars().collect();
    let index = flipped.len() / 2;
    flipped[index] = if flipped[index] == '7' { '8' } else { '7' };
    let damaged = format!("YK1-{}", flipped.into_iter().collect::<String>());
    assert!(matches!(
        decode(&damaged),
        Err(Error::Checksum | Error::Malformed | Error::BadEncoding)
    ));
    assert_eq!(decode("YK1-U!U!U"), Err(Error::BadEncoding));
    assert_eq!(decode("HELLO-WORLD"), Err(Error::NotACode));
    assert_eq!(decode("YK2-ABCD1234"), Err(Error::ServerCodeUnsupported));
    let short = &code[..code.len() - 7];
    assert!(decode(short).is_err());
}

#[test]
fn checksum_failure_is_reported_for_a_changed_payload() {
    let code = encode(&configuration(&[("nestgate", "latest")])).unwrap();
    let body = code.trim_start_matches("YK1-").replace('-', "");
    let mut bytes = base32_decode(&body).unwrap();
    bytes[0] ^= 1;
    let tampered = format!("YK1-{}", base32_encode(&bytes));
    assert_eq!(decode(&tampered), Err(Error::Checksum));
}

#[test]
fn oversized_input_is_refused_before_and_after_inflating() {
    let huge = format!("YK1-{}", "0".repeat(2 * MAX_DECODED + 1));
    assert_eq!(decode(&huge), Err(Error::TooLarge));
    // a deflate bomb with a correct checksum: more than 4 KiB once inflated
    assert_eq!(decode(&forged(&vec![b' '; 5000])), Err(Error::TooLarge));
    // a configuration that cannot be valid is refused when encoding
    let mut big = configuration(&[]);
    big.name = "x".repeat(49);
    assert!(encode(&big).is_err());
    let many: Vec<(String, String)> = (0..=MAX_MODS)
        .map(|n| (format!("mod-{n:03}"), "latest".to_owned()))
        .collect();
    let mut config = configuration(&[]);
    config.mods = many
        .into_iter()
        .map(|(id, version)| ModRef { id, version })
        .collect();
    assert!(encode(&config).is_err());
}

#[test]
fn codes_carry_only_ids_and_versions_never_urls_or_paths() {
    for (id, version) in [
        ("https://evil/x", "latest"),
        ("a/b", "latest"),
        ("..", "latest"),
        ("nestgate", "http://x"),
    ] {
        let mut config = configuration(&[]);
        config.mods = vec![ModRef {
            id: id.into(),
            version: version.into(),
        }];
        assert!(encode(&config).is_err(), "{id} {version}");
    }
    let mut config = configuration(&[]);
    config.base = "C:\\Users\\x".into();
    assert!(encode(&config).is_err());
    // the same checks apply to a code that someone assembled by hand
    let by_hand = forged(br#"{"v":1,"n":"x","b":"base","m":[["https://evil.test/a","latest"]]}"#);
    assert!(matches!(decode(&by_hand), Err(Error::Invalid(_))));
    let unknown_field = forged(br#"{"v":1,"n":"x","b":"base","m":[],"url":"https://evil.test"}"#);
    assert_eq!(decode(&unknown_field), Err(Error::Malformed));
}

#[test]
fn unknown_ids_and_versions_are_clear_problems() {
    let catalog = merge(Some(&builtin()), None);
    let preview = resolve(
        &configuration(&[("nestgate", "latest"), ("ghost", "latest"), ("kagerou", "9.9.9")]),
        &catalog,
        &bases(),
    );
    assert!(!preview.installable());
    assert!(preview.problems.contains(&Problem::UnknownMod("ghost".into())));
    assert!(preview.problems.contains(&Problem::UnknownVersion {
        id: "kagerou".into(),
        version: "9.9.9".into()
    }));
    let unknown_base = resolve(
        &configuration(&[("nestgate", "latest")]),
        &catalog,
        &BTreeMap::new(),
    );
    assert_eq!(
        unknown_base.problems,
        vec![Problem::UnknownBase("yasagashi-jinro".into())]
    );
}

#[test]
fn requirements_are_added_automatically() {
    let catalog = merge(Some(&builtin()), None);
    let preview = resolve(&configuration(&[("yasagashi", "latest")]), &catalog, &bases());
    assert!(preview.installable(), "{:?}", preview.problems);
    let ids: Vec<_> = preview
        .mods
        .iter()
        .map(|m| (m.id.as_str(), m.auto_added))
        .collect();
    assert_eq!(ids, vec![("yasagashi", false), ("nestgate", true)]);
    let selected = BTreeSet::from(["yasagashi".to_owned()]);
    let (all, added) = with_requirements(&selected, &catalog);
    assert_eq!(
        all,
        BTreeSet::from(["yasagashi".to_owned(), "nestgate".to_owned()])
    );
    assert_eq!(added, BTreeSet::from(["nestgate".to_owned()]));
}

#[test]
fn builtin_mods_must_be_in_the_chosen_build() {
    let catalog = merge(Some(&builtin()), None);
    let narrow = BTreeMap::from([("yasagashi-jinro".to_owned(), vec!["nestgate".to_owned()])]);
    let preview = resolve(&configuration(&[("kagerou", "latest")]), &catalog, &narrow);
    assert_eq!(
        preview.problems,
        vec![Problem::NotInBuild {
            id: "kagerou".into(),
            base: "yasagashi-jinro".into()
        }]
    );
}

#[test]
fn a_yak_beats_the_builtin_entry_with_the_same_id() {
    let catalog = merge(
        Some(&builtin()),
        Some(&yak_catalog(&[
            ("kagerou", &["0.2.0", "0.3.0"]),
            ("extra-mod", &["1.0.0"]),
        ])),
    );
    let kagerou = catalog.iter().find(|m| m.id == "kagerou").unwrap();
    assert_eq!(kagerou.kind, Kind::Yak);
    assert_eq!(kagerou.version, "0.3.0");
    assert_eq!(kagerou.versions, vec!["0.3.0", "0.2.0"]);
    assert_eq!(kagerou.name("ja"), "ヤク kagerou");
    assert_eq!(
        catalog.iter().find(|m| m.id == "nestgate").unwrap().kind,
        Kind::Builtin
    );
    // a .yak is not checked against the build's native list; it goes to the install set
    let preview = resolve(
        &configuration(&[("kagerou", "0.2.0"), ("extra-mod", "latest")]),
        &catalog,
        &BTreeMap::from([("yasagashi-jinro".to_owned(), vec![])]),
    );
    assert!(preview.installable(), "{:?}", preview.problems);
    assert_eq!(
        preview.yak_set(),
        vec![
            ("kagerou".to_owned(), "0.2.0".to_owned()),
            ("extra-mod".to_owned(), "1.0.0".to_owned())
        ]
    );
}

#[test]
fn builtin_catalog_parsing_rules() {
    let catalog = builtin();
    assert_eq!(catalog.mods.len(), 3);
    assert_eq!(merge(Some(&catalog), None)[0].name("ja"), "Kagerou");
    let ok = |value: serde_json::Value| BuiltinCatalog::parse(value.to_string().as_bytes());
    let base = json!({"schema": "1.0", "kind": "yoake-builtin-catalog", "mods": []});
    assert!(ok(base.clone()).is_ok());
    let mut major = base.clone();
    major["schema"] = json!("2.0");
    assert_eq!(ok(major).unwrap_err(), Error::NeedsUpdate);
    let mut other = base.clone();
    other["kind"] = json!("something-else");
    assert_eq!(ok(other).unwrap_err(), Error::Malformed);
    let mut dup = base;
    dup["mods"] = json!([
        {"id": "nestgate", "version": "0.1.0", "author": "a", "name": {"en": "n"}, "description": {"en": "d"}},
        {"id": "nestgate", "version": "0.1.0", "author": "a", "name": {"en": "n"}, "description": {"en": "d"}}
    ]);
    assert_eq!(ok(dup).unwrap_err(), Error::Malformed);
}

#[test]
fn builtin_catalog_needs_the_mods_signature() {
    let (key, _) = SigningKey::generate().unwrap();
    let (other, _) = SigningKey::generate().unwrap();
    let bytes = json!({"schema": "1.0", "kind": "yoake-builtin-catalog", "mods": []}).to_string();
    let signature = key.sign(YOAKE_MODS_V1, bytes.as_bytes());
    assert!(BuiltinCatalog::parse_verified(bytes.as_bytes(), &signature, &[key.public()]).is_ok());
    assert_eq!(
        BuiltinCatalog::parse_verified(bytes.as_bytes(), &signature, &[other.public()])
            .unwrap_err(),
        Error::BadSignature
    );
    assert_eq!(
        BuiltinCatalog::parse_verified(b"{}", &signature, &[key.public()]).unwrap_err(),
        Error::BadSignature
    );
    let wrong_domain = key.sign("yoake-index-v1", bytes.as_bytes());
    assert!(
        BuiltinCatalog::parse_verified(bytes.as_bytes(), &wrong_domain, &[key.public()]).is_err()
    );
}

#[test]
fn saved_configurations_survive_a_restart_and_replace_by_name() {
    let directory = std::env::temp_dir().join(format!("yoake-configs-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&directory);
    save(&directory, configuration(&[("nestgate", "latest")])).unwrap();
    save(&directory, configuration(&[("kagerou", "latest")])).unwrap();
    let list = load(&directory);
    assert_eq!(list.len(), 1);
    assert_eq!(list[0].mods[0].id, "kagerou");
    remove(&directory, "友だちと人狼").unwrap();
    assert!(load(&directory).is_empty());
    let _ = std::fs::remove_dir_all(&directory);
}
