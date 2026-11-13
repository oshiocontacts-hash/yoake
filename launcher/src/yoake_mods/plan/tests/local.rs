use super::*;

#[test]
fn unsigned_data_requires_confirmation_and_keeps_its_trust_mark() {
    let fixture = Fixture::new();
    let package = fixture.package(fixture.manifest("local-data", "1.0.0"), false);
    let plan = fixture.local(&package.path).unwrap();
    assert!(plan.unsigned);
    assert!(matches!(
        fixture.apply(plan, false),
        Err(Error::ConfirmationRequired)
    ));
    assert!(!fixture.home.join("yoake-mods.txt").exists());
    fixture
        .apply(fixture.local(&package.path).unwrap(), true)
        .unwrap();
    let installed = fixture.client.installed(&fixture.home).unwrap();
    assert_eq!(installed.len(), 1);
    assert!(installed[0].unsigned);
    assert_eq!(fixture.list(), "local-data\t1.0.0\n");
    assert!(fixture.http.requests.lock().unwrap().is_empty());
}

#[test]
fn unsigned_local_program_requires_developer_mode_at_plan_and_apply() {
    let fixture = Fixture::new();
    let package = fixture.package(fixture.program("local-program"), false);
    assert!(matches!(
        fixture.local_with_dev(&package.path, false),
        Err(Error::UnsignedProgram)
    ));

    let plan = fixture.local_with_dev(&package.path, true).unwrap();
    assert!(plan.unsigned);
    assert!(plan.unsigned_program);
    assert!(matches!(
        fixture.apply_with_dev(plan, true, false),
        Err(Error::UnsignedProgram)
    ));
    assert!(!fixture.home.join("yoake-mods.txt").exists());

    let plan = fixture.local_with_dev(&package.path, true).unwrap();
    fixture.apply_with_dev(plan, true, true).unwrap();
    let installed = fixture.client.installed(&fixture.home).unwrap();
    assert_eq!(installed.len(), 1);
    assert!(installed[0].unsigned);
    assert_eq!(fixture.list(), "local-program\t1.0.0\n");

    let signed = fixture.package(fixture.program("signed-program"), true);
    let plan = fixture.local_with_dev(&signed.path, false).unwrap();
    fixture.apply_with_dev(plan, true, false).unwrap();
    assert!(fixture
        .client
        .installed(&fixture.home)
        .unwrap()
        .iter()
        .any(|mod_| mod_.manifest.id == "local-program" && mod_.unsigned));
    fixture.client.remove(&fixture.home, "local-program").unwrap();
    assert_eq!(fixture.list(), "signed-program\t1.0.0\n");
}

#[test]
fn developer_mode_does_not_accept_unsigned_signed_local_candidates() {
    let fixture = Fixture::new();
    let package = fixture.package(fixture.program("signed-candidate"), false);
    for enabled in [false, true] {
        let mut plan = fixture.local_with_dev(&package.path, true).unwrap();
        let Source::Local { signed, .. } = &mut plan.chosen[0].source else {
            panic!("expected a local candidate");
        };
        *signed = true;
        assert!(matches!(
            fixture.apply_with_dev(plan, true, enabled),
            Err(Error::UnsignedProgram)
        ));
    }
}

#[test]
fn local_programs_require_a_signature_from_the_pinned_key() {
    let fixture = Fixture::new();
    let package = fixture.package(fixture.program("local-program"), false);
    assert!(matches!(
        fixture.local(&package.path),
        Err(Error::UnsignedProgram)
    ));
    let (wrong_key, _) = horizon_release_sign::SigningKey::generate().unwrap();
    horizon_release_sign::yak::sign_file(&package.path, &wrong_key).unwrap();
    for enabled in [false, true] {
        assert!(matches!(
            fixture.local_with_dev(&package.path, enabled),
            Err(Error::Invalid(_))
        ));
    }
    let signed = fixture.package(fixture.program("signed-program"), true);
    let plan = fixture.local(&signed.path).unwrap();
    assert!(!plan.unsigned);
    fixture.apply(plan, true).unwrap();
    assert!(!fixture.client.installed(&fixture.home).unwrap()[0].unsigned);
    assert_eq!(fixture.list(), "signed-program\t1.0.0\n");
}

#[test]
fn a_catalog_program_still_needs_its_own_signature() {
    // The game checks a DLL against the package's SIGNATURE, so the catalog alone is not enough.
    let fixture = Fixture::new();
    let package = fixture.package(fixture.program("catalog-program"), false);
    let catalog = fixture.publish(&[package.version]);
    for enabled in [false, true] {
        let result = fixture
            .plan(&catalog, "catalog-program")
            .and_then(|plan| fixture.apply_with_dev(plan, true, enabled));
        assert!(matches!(result, Err(Error::UnsignedProgram)), "{result:?}");
    }
    let signed = fixture.package(fixture.program("signed-catalog-program"), true);
    let catalog = fixture.publish(&[signed.version]);
    fixture
        .apply(fixture.plan(&catalog, "signed-catalog-program").unwrap(), true)
        .unwrap();
    assert_eq!(fixture.list(), "signed-catalog-program	1.0.0
");
}

#[test]
fn developer_mode_does_not_accept_a_bad_catalog_signature() {
    let fixture = Fixture::new();
    let mut package = fixture.package(fixture.program("bad-signature"), false);
    let (wrong_key, _) = horizon_release_sign::SigningKey::generate().unwrap();
    horizon_release_sign::yak::sign_file(&package.path, &wrong_key).unwrap();
    let bytes = std::fs::read(&package.path).unwrap();
    package.version.sha256 = horizon_release_sign::sha256_hex(&bytes);
    package.version.size = bytes.len() as u64;
    fixture.http.respond(&package.version.url, Ok(bytes));
    let catalog = fixture.publish(&[package.version]);
    for enabled in [false, true] {
        let plan = fixture.plan(&catalog, "bad-signature").unwrap();
        assert!(matches!(
            fixture.apply_with_dev(plan, true, enabled),
            Err(Error::Invalid(_))
        ));
    }
}

#[test]
fn signed_origin_still_requires_a_valid_signature() {
    let fixture = Fixture::new();
    let package = fixture.package(fixture.program("signed-origin"), false);
    let plan = fixture.local_with_dev(&package.path, true).unwrap();
    fixture.apply_with_dev(plan, true, true).unwrap();

    let (mut state, _, _) = store::load(&fixture.home).unwrap();
    state.records[0].origin = Origin::Signed;
    let state_path = store::cache(&fixture.home).join("installed.json");
    store::write_json(&state_path, &state).unwrap();
    let check_package = fixture.package(fixture.program("check-signed"), true);
    for enabled in [false, true] {
        assert!(matches!(
            fixture.local_with_dev(&check_package.path, enabled),
            Err(Error::UnsignedProgram)
        ));
    }

    let (wrong_key, _) = horizon_release_sign::SigningKey::generate().unwrap();
    horizon_release_sign::yak::sign_file(&package.path, &wrong_key).unwrap();
    let bytes = std::fs::read(&package.path).unwrap();
    state.records[0].sha256 = horizon_release_sign::sha256_hex(&bytes);
    let path = store::archive_path(&fixture.home, &state.records[0].sha256).unwrap();
    std::fs::copy(&package.path, path).unwrap();
    store::write_json(&state_path, &state).unwrap();
    for enabled in [false, true] {
        assert!(matches!(
            fixture.local_with_dev(&check_package.path, enabled),
            Err(Error::Invalid(_))
        ));
    }
}

#[test]
fn local_file_changes_after_review_are_refused() {
    let fixture = Fixture::new();
    let package = fixture.package(fixture.manifest("local-data", "1.0.0"), false);
    let plan = fixture.local(&package.path).unwrap();
    let mut bytes = package.bytes;
    bytes[0] ^= 1;
    std::fs::write(&package.path, bytes).unwrap();
    assert!(matches!(fixture.apply(plan, true), Err(Error::Stale)));
    assert!(!fixture.home.join("yoake-mods.txt").exists());
}
