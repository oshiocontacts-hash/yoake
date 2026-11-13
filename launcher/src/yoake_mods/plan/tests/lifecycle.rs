use super::*;

#[test]
fn dependencies_are_reviewed_enabled_in_order_and_removed_safely() {
    let fixture = Fixture::new();
    let dependency = fixture.package(fixture.manifest("z-dependency", "1.0.0"), false);
    let mut manifest = fixture.manifest("a-parent", "1.0.0");
    manifest
        .dependencies
        .insert("z-dependency".into(), "^1".parse().unwrap());
    let parent = fixture.package(manifest, false);
    let catalog = fixture.publish(&[parent.version.clone(), dependency.version.clone()]);
    let plan = fixture.plan(&catalog, "a-parent").unwrap();
    assert_eq!(
        plan.changes
            .iter()
            .map(|manifest| manifest.id.as_str())
            .collect::<Vec<_>>(),
        vec!["z-dependency", "a-parent"]
    );
    fixture.apply(plan, true).unwrap();
    assert_eq!(fixture.list(), "z-dependency\t1.0.0\na-parent\t1.0.0\n");
    assert_eq!(fixture.client.installed(&fixture.home).unwrap().len(), 2);
    assert!(matches!(
        fixture.client.remove(&fixture.home, "z-dependency"),
        Err(Error::Resolution(_))
    ));
    assert!(matches!(
        fixture.client.set_enabled_for(
            &fixture.home,
            "z-dependency",
            false,
            fixture.environment.clone()
        ),
        Err(Error::Resolution(_))
    ));
    fixture
        .client
        .set_enabled_for(
            &fixture.home,
            "a-parent",
            false,
            fixture.environment.clone(),
        )
        .unwrap();
    fixture
        .client
        .set_enabled_for(
            &fixture.home,
            "z-dependency",
            false,
            fixture.environment.clone(),
        )
        .unwrap();
    assert!(fixture.list().is_empty());
    assert!(fixture
        .client
        .installed(&fixture.home)
        .unwrap()
        .iter()
        .all(|row| !row.enabled));
    fixture
        .client
        .set_enabled_for(&fixture.home, "a-parent", true, fixture.environment.clone())
        .unwrap();
    assert_eq!(fixture.list(), "z-dependency\t1.0.0\na-parent\t1.0.0\n");
    fixture.client.remove(&fixture.home, "a-parent").unwrap();
    assert_eq!(fixture.list(), "z-dependency\t1.0.0\n");
    fixture
        .client
        .remove(&fixture.home, "z-dependency")
        .unwrap();
    assert!(fixture.list().is_empty());
    assert!(fixture.client.installed(&fixture.home).unwrap().is_empty());
    for package in [&parent, &dependency] {
        assert!(!store::directory(&fixture.home, &package.version.manifest).exists());
        assert!(store::archive_path(&fixture.home, &package.version.sha256)
            .unwrap()
            .exists());
    }
}

#[test]
fn conflicts_are_refused_without_changing_the_installed_set() {
    let fixture = Fixture::new();
    let first = fixture.package(fixture.manifest("first", "1.0.0"), false);
    let mut conflict = fixture.manifest("conflict", "1.0.0");
    conflict
        .conflicts
        .insert("first".into(), "*".parse().unwrap());
    let conflict = fixture.package(conflict, false);
    let catalog = fixture.publish(&[first.version, conflict.version]);
    fixture
        .apply(fixture.plan(&catalog, "first").unwrap(), true)
        .unwrap();
    assert!(matches!(
        fixture.plan(&catalog, "conflict"),
        Err(Error::Resolution(_))
    ));
    assert_eq!(fixture.list(), "first\t1.0.0\n");
}

#[test]
fn update_selects_latest_compatible_and_retains_old_cache() {
    let fixture = Fixture::new();
    let first = fixture.package(fixture.manifest("example", "1.0.0"), false);
    let initial = fixture.publish(&[first.version.clone()]);
    fixture
        .apply(fixture.plan(&initial, "example").unwrap(), true)
        .unwrap();
    let next = fixture.package(fixture.manifest("example", "1.1.0"), false);
    let mut incompatible = fixture.manifest("example", "2.0.0");
    incompatible.yoake.version = ">=2".parse().unwrap();
    let incompatible = fixture.package(incompatible, false);
    let catalog = fixture.publish(&[
        first.version.clone(),
        next.version.clone(),
        incompatible.version,
    ]);
    assert!(fixture.client.update_available(
        &first.version.manifest,
        &catalog,
        &fixture.environment
    ));
    let plan = fixture.plan(&catalog, "example").unwrap();
    assert_eq!(plan.changes.len(), 1);
    assert_eq!(plan.changes[0].version.to_string(), "1.1.0");
    fixture.apply(plan, true).unwrap();
    assert_eq!(fixture.list(), "example\t1.1.0\n");
    assert!(!store::directory(&fixture.home, &first.version.manifest).exists());
    assert!(store::directory(&fixture.home, &next.version.manifest).exists());
    assert!(store::archive_path(&fixture.home, &first.version.sha256)
        .unwrap()
        .exists());
    assert!(!fixture.client.update_available(
        &next.version.manifest,
        &catalog,
        &fixture.environment
    ));
}

#[test]
fn exact_install_set_disables_unrequested_mods_and_preserves_dependencies() {
    let fixture = Fixture::new();
    let other = fixture.package(fixture.manifest("other", "1.0.0"), false);
    let dependency = fixture.package(fixture.manifest("dependency", "1.0.0"), false);
    let mut parent = fixture.manifest("parent", "1.0.0");
    parent
        .dependencies
        .insert("dependency".into(), "=1.0.0".parse().unwrap());
    let parent = fixture.package(parent, false);
    let newer = fixture.package(fixture.manifest("parent", "1.1.0"), false);
    let catalog = fixture.publish(&[
        other.version,
        dependency.version,
        parent.version,
        newer.version,
    ]);
    fixture
        .apply(fixture.plan(&catalog, "other").unwrap(), true)
        .unwrap();
    let plan = fixture
        .client
        .plan_for(
            &fixture.home,
            Some(&catalog),
            &[("parent".into(), Some("1.0.0".parse().unwrap()))],
            true,
            None,
            fixture.environment.clone(),
        )
        .unwrap();
    fixture.apply(plan, true).unwrap();
    assert_eq!(fixture.list(), "dependency\t1.0.0\nparent\t1.0.0\n");
    let rows = fixture.client.installed(&fixture.home).unwrap();
    assert_eq!(rows.len(), 3);
    assert!(rows
        .iter()
        .any(|row| row.manifest.id == "other" && !row.enabled));
}

#[test]
fn a_conflicting_update_is_refused_instead_of_silently_keeping_the_old_version() {
    let fixture = Fixture::new();
    let first = fixture.package(fixture.manifest("example", "1.0.0"), false);
    let other = fixture.package(fixture.manifest("other", "1.0.0"), false);
    let initial = fixture.publish(&[first.version.clone(), other.version.clone()]);
    fixture
        .apply(fixture.plan(&initial, "example").unwrap(), true)
        .unwrap();
    fixture
        .apply(fixture.plan(&initial, "other").unwrap(), true)
        .unwrap();
    let mut newer = fixture.manifest("example", "1.1.0");
    newer.conflicts.insert("other".into(), "*".parse().unwrap());
    let newer = fixture.package(newer, false);
    let catalog = fixture.publish(&[first.version, other.version, newer.version]);
    assert!(matches!(
        fixture.plan(&catalog, "example"),
        Err(Error::Resolution(_))
    ));
    assert_eq!(fixture.list(), "example\t1.0.0\nother\t1.0.0\n");
}

#[test]
fn list_accepts_comments_and_whitespace_like_the_game() {
    let fixture = Fixture::new();
    let package = fixture.package(fixture.manifest("example", "1.0.0"), false);
    let catalog = fixture.publish(&[package.version]);
    fixture
        .apply(fixture.plan(&catalog, "example").unwrap(), true)
        .unwrap();
    std::fs::write(
        fixture.home.join("yoake-mods.txt"),
        "\n  # Installed Yoake mods\n  example  1.0.0  \n\n",
    )
    .unwrap();
    let installed = fixture.client.installed(&fixture.home).unwrap();
    assert_eq!(installed.len(), 1);
    assert!(installed[0].enabled);
    fixture
        .client
        .set_enabled_for(&fixture.home, "example", false, fixture.environment.clone())
        .unwrap();
    assert!(fixture.list().is_empty());
}

#[test]
fn interrupted_removal_restores_the_folder_and_enabled_list() {
    let fixture = Fixture::new();
    let package = fixture.package(fixture.manifest("example", "1.0.0"), false);
    let catalog = fixture.publish(&[package.version.clone()]);
    fixture
        .apply(fixture.plan(&catalog, "example").unwrap(), true)
        .unwrap();
    let _lock = store::lock(&fixture.home).unwrap();
    let (old, _, _) = store::load(&fixture.home).unwrap();
    let destination = store::directory(&fixture.home, &package.version.manifest);
    store::stage_remove(&fixture.home, &old, &package.version.manifest).unwrap();
    assert!(!destination.exists());
    store::recover(&fixture.home).unwrap();
    assert!(destination.exists());
    assert_eq!(fixture.list(), "example\t1.0.0\n");
    assert_eq!(store::load(&fixture.home).unwrap().0.records.len(), 1);
}

#[test]
fn stale_preview_cannot_overwrite_newer_switch_changes() {
    let fixture = Fixture::new();
    let first = fixture.package(fixture.manifest("example", "1.0.0"), false);
    let catalog = fixture.publish(&[first.version]);
    fixture
        .apply(fixture.plan(&catalog, "example").unwrap(), true)
        .unwrap();
    let stale = fixture.plan(&catalog, "example").unwrap();
    fixture
        .client
        .set_enabled_for(&fixture.home, "example", false, fixture.environment.clone())
        .unwrap();
    assert!(matches!(fixture.apply(stale, true), Err(Error::Stale)));
    assert!(fixture.list().is_empty());
}

#[test]
fn interrupted_extraction_restores_the_previous_list() {
    let fixture = Fixture::new();
    let package = fixture.package(fixture.manifest("interrupted", "1.0.0"), false);
    let _lock = store::lock(&fixture.home).unwrap();
    let (old, _, _) = store::load(&fixture.home).unwrap();
    store::begin(&fixture.home, &old, &[package.version.manifest.clone()]).unwrap();
    let destination = store::directory(&fixture.home, &package.version.manifest);
    let mut archive = Archive::new(File::open(&package.path).unwrap()).unwrap();
    archive
        .extract_target(
            &destination,
            fixture.environment.target,
            &[fixture.key.public()],
        )
        .unwrap();
    assert!(destination.exists());
    store::recover(&fixture.home).unwrap();
    assert!(!destination.exists());
    assert!(!fixture.home.join("yoake-mods.txt").exists());
    assert!(store::load(&fixture.home).unwrap().0.records.is_empty());
}
