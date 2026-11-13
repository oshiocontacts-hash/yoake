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
fn local_programs_require_a_signature_from_the_pinned_key() {
    let fixture = Fixture::new();
    let package = fixture.package(fixture.program("local-program"), false);
    assert!(matches!(
        fixture.local(&package.path),
        Err(Error::UnsignedProgram)
    ));
    let (wrong_key, _) = horizon_release_sign::SigningKey::generate().unwrap();
    horizon_release_sign::yak::sign_file(&package.path, &wrong_key).unwrap();
    assert!(fixture.local(&package.path).is_err());
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
    let result = fixture
        .plan(&catalog, "catalog-program")
        .and_then(|plan| fixture.apply(plan, true));
    assert!(matches!(result, Err(Error::UnsignedProgram)), "{result:?}");
    let signed = fixture.package(fixture.program("signed-catalog-program"), true);
    let catalog = fixture.publish(&[signed.version]);
    fixture
        .apply(fixture.plan(&catalog, "signed-catalog-program").unwrap(), true)
        .unwrap();
    assert_eq!(fixture.list(), "signed-catalog-program	1.0.0
");
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
