use super::support::{DATA, named, program};
use horizon_mods::yak::{CompatibilityIssue, Environment, Manifest, ResolutionIssue, Target, Version, VersionReq, check_compatibility, check_data_compatibility, resolve};

#[test]
fn data_packages_only_require_the_yoake_version() {
    let manifest = Manifest::parse(DATA).unwrap();
    let yoake = Version::parse("0.3.9").unwrap();
    let environment = Environment { yoake: &yoake, engine: "different-build", abi: 999, target: Target::MacosArm64 };
    assert!(check_compatibility(&manifest, &environment).is_empty());
    assert!(matches!(check_data_compatibility(&manifest, &Version::parse("0.4.0").unwrap()).as_slice(), [CompatibilityIssue::Yoake { .. }]));
}

#[test]
fn program_packages_require_the_exact_build_abi_and_target() {
    let manifest = Manifest::parse(&program()).unwrap();
    let yoake = Version::parse("0.3.1").unwrap();
    let environment = Environment { yoake: &yoake, engine: "0.3.1+build.test", abi: 2, target: Target::WindowsX64 };
    assert!(check_compatibility(&manifest, &environment).is_empty());
    let wrong = Environment { engine: "0.3.1+build.other", abi: 3, target: Target::MacosX64, ..environment };
    let issues = check_compatibility(&manifest, &wrong);
    assert!(matches!(issues.as_slice(), [CompatibilityIssue::Engine { .. }, CompatibilityIssue::Abi { .. }, CompatibilityIssue::Target { .. }]));
}

#[test]
fn resolver_orders_dependencies_and_ignores_absent_optional_mods() {
    let base = named("base-mod");
    let mut addon = named("addon-mod");
    addon.dependencies.insert(base.id.clone(), VersionReq::parse("^0.3").unwrap());
    addon.optional.insert("absent-mod".into(), VersionReq::STAR);
    let resolution = resolve(&[addon, base, named("extra-mod")]);
    assert!(resolution.is_compatible());
    assert_eq!(resolution.order, ["base-mod", "extra-mod", "addon-mod"]);
    assert!(resolve(&[]).is_compatible());
}

#[test]
fn resolver_reports_missing_wrong_versions_conflicts_and_duplicates() {
    let base = named("base-mod");
    let mut addon = named("addon-mod");
    addon.dependencies.insert("missing-mod".into(), VersionReq::STAR);
    addon.optional.insert(base.id.clone(), VersionReq::parse("^1").unwrap());
    addon.conflicts.insert(base.id.clone(), VersionReq::STAR);
    let resolution = resolve(&[base.clone(), addon, base]);
    assert!(!resolution.is_compatible());
    assert!(resolution.order.is_empty());
    assert!(resolution.issues.iter().any(|issue| matches!(issue, ResolutionIssue::Missing { .. })));
    assert!(resolution.issues.iter().any(|issue| matches!(issue, ResolutionIssue::Version { .. })));
    assert!(resolution.issues.iter().any(|issue| matches!(issue, ResolutionIssue::Conflict { .. })));
    assert!(resolution.issues.iter().any(|issue| matches!(issue, ResolutionIssue::Duplicate { .. })));
}

#[test]
fn resolver_detects_required_and_present_optional_cycles() {
    let mut first = named("first-mod");
    let mut second = named("second-mod");
    first.dependencies.insert(second.id.clone(), VersionReq::STAR);
    second.optional.insert(first.id.clone(), VersionReq::STAR);
    let resolution = resolve(&[second, first]);
    assert!(resolution.order.is_empty());
    assert!(matches!(resolution.issues.as_slice(), [ResolutionIssue::Cycle { ids }] if ids == &["first-mod", "second-mod"]));
}
