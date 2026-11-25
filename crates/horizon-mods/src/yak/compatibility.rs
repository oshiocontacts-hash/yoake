use super::{Manifest, Target};
use semver::{Version, VersionReq};
use std::collections::{BTreeMap, BTreeSet};

#[derive(Clone, Debug)]
pub struct Environment<'a> {
    pub yoake: &'a Version,
    pub engine: &'a str,
    pub abi: u32,
    pub target: Target,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum CompatibilityIssue {
    Yoake { required: VersionReq, actual: Version },
    Engine { required: String, actual: String },
    Abi { required: u32, actual: u32 },
    Target { actual: Target, available: Vec<Target> },
}

pub fn check_data_compatibility(manifest: &Manifest, yoake: &Version) -> Vec<CompatibilityIssue> {
    if manifest.yoake.version.matches(yoake) {
        Vec::new()
    } else {
        vec![CompatibilityIssue::Yoake { required: manifest.yoake.version.clone(), actual: yoake.clone() }]
    }
}

pub fn check_compatibility(manifest: &Manifest, environment: &Environment<'_>) -> Vec<CompatibilityIssue> {
    let mut issues = check_data_compatibility(manifest, environment.yoake);
    if let Some(bin) = &manifest.bin {
        if bin.engine != environment.engine {
            issues.push(CompatibilityIssue::Engine { required: bin.engine.clone(), actual: environment.engine.to_owned() });
        }
        if bin.abi != environment.abi {
            issues.push(CompatibilityIssue::Abi { required: bin.abi, actual: environment.abi });
        }
        if !bin.targets.contains(&environment.target) {
            issues.push(CompatibilityIssue::Target { actual: environment.target, available: bin.targets.clone() });
        }
    }
    issues
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum ResolutionIssue {
    Duplicate { id: String },
    Missing { dependent: String, dependency: String, required: VersionReq },
    Version { dependent: String, dependency: String, required: VersionReq, actual: Version },
    Conflict { first: String, second: String, required: VersionReq, actual: Version },
    Cycle { ids: Vec<String> },
}

#[derive(Clone, Debug, Default)]
pub struct Resolution {
    pub order: Vec<String>,
    pub issues: Vec<ResolutionIssue>,
}

impl Resolution {
    pub fn is_compatible(&self) -> bool {
        self.issues.is_empty()
    }
}

/// Checks exactly the supplied set, without selecting, fetching or enabling mods.
pub fn resolve(manifests: &[Manifest]) -> Resolution {
    let mut result = Resolution::default();
    let mut installed = BTreeMap::new();
    let mut discovery_order = Vec::new();
    for manifest in manifests {
        if installed.insert(manifest.id.clone(), manifest).is_some() {
            result.issues.push(ResolutionIssue::Duplicate { id: manifest.id.clone() });
        } else {
            discovery_order.push(manifest.id.clone());
        }
    }
    let mut dependencies: BTreeMap<String, BTreeSet<String>> = installed.keys()
        .map(|id| (id.clone(), BTreeSet::new())).collect();
    for (id, manifest) in &installed {
        for (dependency, required) in &manifest.dependencies {
            match installed.get(dependency) {
                None => result.issues.push(ResolutionIssue::Missing { dependent: id.clone(), dependency: dependency.clone(), required: required.clone() }),
                Some(actual) => {
                    dependencies.get_mut(id).unwrap().insert(dependency.clone());
                    if !required.matches(&actual.version) {
                        result.issues.push(ResolutionIssue::Version { dependent: id.clone(), dependency: dependency.clone(), required: required.clone(), actual: actual.version.clone() });
                    }
                }
            }
        }
        for (dependency, required) in &manifest.optional {
            if let Some(actual) = installed.get(dependency) {
                dependencies.get_mut(id).unwrap().insert(dependency.clone());
                if !required.matches(&actual.version) {
                    result.issues.push(ResolutionIssue::Version { dependent: id.clone(), dependency: dependency.clone(), required: required.clone(), actual: actual.version.clone() });
                }
            }
        }
        for (other, required) in &manifest.conflicts {
            if let Some(actual) = installed.get(other).filter(|other| required.matches(&other.version)) {
                result.issues.push(ResolutionIssue::Conflict { first: id.clone(), second: other.clone(), required: required.clone(), actual: actual.version.clone() });
            }
        }
    }
    while !dependencies.is_empty() {
        let ready: Vec<_> = discovery_order.iter()
            .filter(|id| dependencies.get(*id).is_some_and(|required| required.is_empty()))
            .cloned().collect();
        if ready.is_empty() {
            result.issues.push(ResolutionIssue::Cycle { ids: dependencies.keys().cloned().collect() });
            break;
        }
        for id in &ready {
            dependencies.remove(id);
        }
        for required in dependencies.values_mut() {
            for id in &ready {
                required.remove(id);
            }
        }
        result.order.extend(ready);
    }
    if !result.issues.is_empty() {
        result.order.clear();
    }
    result
}
