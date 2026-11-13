use super::*;
use std::collections::{BTreeMap, BTreeSet};

pub(super) fn choose(
    state: &State,
    enabled: &[String],
    catalog: Option<&VerifiedCatalog>,
    requested: &[(String, Option<Version>)],
    replace: bool,
    local: Option<Candidate>,
    environment: &InstanceEnvironment,
) -> Result<Vec<Candidate>, Error> {
    let mut roots = BTreeMap::new();
    if !replace {
        for id in enabled {
            roots.insert(id.clone(), None);
        }
    }
    let mut explicit = BTreeSet::new();
    for (id, version) in requested {
        if !explicit.insert(id.clone()) {
            return Err(Error::Invalid("Duplicate requested mod".into()));
        }
        roots.insert(id.clone(), version.clone());
    }
    let mut pools: BTreeMap<String, Vec<Candidate>> = BTreeMap::new();
    for record in &state.records {
        pools
            .entry(record.manifest.id.clone())
            .or_default()
            .push(Candidate {
                manifest: record.manifest.clone(),
                source: Source::Installed(record.clone()),
            });
    }
    if let Some(catalog) = catalog {
        for entry in &catalog.catalog.entries {
            let mut versions = entry.versions.clone();
            versions.sort_by(|left, right| right.version.cmp(&left.version));
            let pool = pools.entry(entry.id.clone()).or_default();
            let additions = versions
                .into_iter()
                .filter(|version| {
                    !pool
                        .iter()
                        .any(|candidate| candidate.manifest.version == version.version)
                })
                .map(|version| Candidate {
                    manifest: version.manifest.clone(),
                    source: Source::Catalog(version, catalog.proof.clone()),
                })
                .collect::<Vec<_>>();
            pool.extend(additions);
            if explicit.contains(&entry.id) {
                pool.sort_by(|left, right| right.manifest.version.cmp(&left.manifest.version));
            }
        }
    }
    if let Some(local) = local {
        pools.insert(local.manifest.id.clone(), vec![local]);
    }
    for (id, exact) in &roots {
        let candidates = pools
            .get(id)
            .ok_or_else(|| Error::Resolution(format!("Mod not found: {id}")))?;
        let matching = candidates
            .iter()
            .filter(|candidate| {
                exact
                    .as_ref()
                    .is_none_or(|version| version == &candidate.manifest.version)
            })
            .collect::<Vec<_>>();
        if matching.is_empty() {
            return Err(Error::Resolution(format!("Version not found: {id}")));
        }
        if matching
            .iter()
            .all(|candidate| environment.check(&candidate.manifest).is_err())
        {
            return environment
                .check(&matching[0].manifest)
                .map(|()| Vec::new());
        }
    }
    for pool in pools.values_mut() {
        pool.retain(|candidate| environment.check(&candidate.manifest).is_ok());
    }
    for (id, version) in requested {
        if version.is_none() {
            let latest = pools
                .get(id)
                .and_then(|pool| pool.first())
                .ok_or_else(|| Error::Resolution(format!("Version not found: {id}")))?;
            roots.insert(id.clone(), Some(latest.manifest.version.clone()));
        }
    }
    let mut attempts = 0;
    search(&roots, &pools, BTreeMap::new(), &mut attempts).ok_or_else(|| {
        Error::Resolution(
            "Dependencies, versions or conflicts cannot be resolved for this instance".into(),
        )
    })
}

fn search(
    roots: &BTreeMap<String, Option<Version>>,
    pools: &BTreeMap<String, Vec<Candidate>>,
    selected: BTreeMap<String, Candidate>,
    attempts: &mut usize,
) -> Option<Vec<Candidate>> {
    *attempts += 1;
    if *attempts > 10_000 || selected.len() > 128 {
        return None;
    }
    let mut required: BTreeSet<String> = roots.keys().cloned().collect();
    for candidate in selected.values() {
        let manifest = &candidate.manifest;
        if roots
            .get(&manifest.id)
            .and_then(Option::as_ref)
            .is_some_and(|version| version != &manifest.version)
        {
            return None;
        }
        required.extend(manifest.dependencies.keys().cloned());
        for (id, requirement) in manifest.dependencies.iter().chain(&manifest.optional) {
            if selected
                .get(id)
                .is_some_and(|other| !requirement.matches(&other.manifest.version))
            {
                return None;
            }
        }
        for (id, requirement) in &manifest.conflicts {
            if selected
                .get(id)
                .is_some_and(|other| requirement.matches(&other.manifest.version))
            {
                return None;
            }
        }
    }
    if let Some(id) = required.iter().find(|id| !selected.contains_key(*id)) {
        for candidate in pools.get(id)? {
            let mut next = selected.clone();
            next.insert(id.clone(), candidate.clone());
            if let Some(result) = search(roots, pools, next, attempts) {
                return Some(result);
            }
        }
        return None;
    }
    let manifests = selected
        .values()
        .map(|candidate| candidate.manifest.clone())
        .collect::<Vec<_>>();
    let resolution = yak::resolve(&manifests);
    if !resolution.is_compatible() {
        return None;
    }
    resolution
        .order
        .iter()
        .map(|id| selected.get(id).cloned())
        .collect()
}
