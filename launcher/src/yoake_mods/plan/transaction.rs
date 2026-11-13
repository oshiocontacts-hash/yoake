use super::*;
use horizon_release_sign::yak::atomic_write;
use std::io::{Seek, SeekFrom};

impl Client {
    pub fn set_enabled(&self, home: &Path, id: &str, enabled: bool) -> Result<(), Error> {
        self.set_enabled_for(home, id, enabled, InstanceEnvironment::read(home)?)
    }

    pub(super) fn set_enabled_for(
        &self,
        home: &Path,
        id: &str,
        enabled: bool,
        environment: InstanceEnvironment,
    ) -> Result<(), Error> {
        let _lock = store::lock(home)?;
        let (state, mut active, _) = self.verified_state(home)?;
        if !state.records.iter().any(|record| record.manifest.id == id) {
            return Err(Error::Invalid("Mod is not installed".into()));
        }
        active.retain(|other| other != id);
        let order = if enabled {
            let chosen = resolver::choose(
                &state,
                &active,
                None,
                &[(id.to_owned(), None)],
                false,
                None,
                &environment,
            )?;
            chosen
                .iter()
                .map(|candidate| candidate.manifest.id.clone())
                .collect()
        } else {
            enabled_order(&state, &active)?
        };
        store::commit(home, &state, &state, &order)
    }

    pub fn remove(&self, home: &Path, id: &str) -> Result<(), Error> {
        let _lock = store::lock(home)?;
        let (old, mut active, _) = self.verified_state(home)?;
        let manifest = old
            .records
            .iter()
            .find(|record| record.manifest.id == id)
            .ok_or_else(|| Error::Invalid("Mod is not installed".into()))?
            .manifest
            .clone();
        let mut next = old.clone();
        next.records.retain(|record| record.manifest.id != id);
        active.retain(|other| other != id);
        let order = enabled_order(&next, &active)?;
        let result = store::stage_remove(home, &old, &manifest)
            .and_then(|()| store::commit(home, &old, &next, &order));
        if result.is_err() {
            store::recover(home)?;
        }
        result?;
        store::finish_remove(home, &manifest)
    }

    pub fn apply(&self, plan: InstallPlan, confirmed: bool) -> Result<(), Error> {
        let environment = InstanceEnvironment::read(&plan.home)?;
        self.apply_for(plan, confirmed, environment)
    }

    fn check_archive(&self, path: &Path, candidate: &Candidate) -> Result<(), Error> {
        let archive = match &candidate.source {
            Source::Catalog(version, proof) => {
                self.verify(proof.clone(), false)?;
                Archive::from_catalog(File::open(path)?, version).map_err(Error::Invalid)?
            }
            _ => Archive::new(File::open(path)?).map_err(Error::Invalid)?,
        };
        let signed = archive
            .verify_signature(&self.keys)
            .map_err(Error::Invalid)?
            .is_some();
        if matches!(candidate.source, Source::Local { signed: true, .. }) && !signed {
            return Err(Error::UnsignedProgram);
        }
        // A program (bin/) always carries its own Yoake signature, from the catalog too: the game
        // checks the DLL against it before loading (docs/yak-format.md §5).
        if archive.manifest().bin.is_some() && !signed {
            return Err(Error::UnsignedProgram);
        }
        if !same_manifest(archive.manifest(), &candidate.manifest) {
            return Err(FeedError::HashMismatch.into());
        }
        Ok(())
    }

    fn cache_candidate(&self, home: &Path, candidate: &Candidate) -> Result<Record, Error> {
        let (hash, size, origin) = match &candidate.source {
            Source::Installed(record) => return Ok(record.clone()),
            Source::Catalog(version, proof) => {
                self.verify(proof.clone(), false)?;
                let digest = horizon_release_sign::sha256_hex(proof.json.as_bytes());
                store::write_json(
                    &store::cache(home).join(format!("proof-{digest}.json")),
                    proof,
                )?;
                (
                    version.sha256.clone(),
                    version.size,
                    Origin::Catalog(digest),
                )
            }
            Source::Local { path, hash, signed } => {
                if hash_file(path)? != *hash {
                    return Err(Error::Stale);
                }
                (
                    hash.clone(),
                    std::fs::metadata(path)?.len(),
                    if *signed {
                        Origin::Signed
                    } else {
                        Origin::UnsignedData
                    },
                )
            }
        };
        let destination = store::archive_path(home, &hash)?;
        store::plain(&destination)?;
        if destination.try_exists()? && hash_file(&destination)? != hash {
            return Err(FeedError::HashMismatch.into());
        }
        if !destination.try_exists()? {
            let mut source: Box<dyn Read + Send> = match &candidate.source {
                Source::Catalog(version, _) => {
                    let auth = self.authorization()?;
                    self.http.open(&version.url, Some(&auth), size)?
                }
                Source::Local { path, .. } => Box::new(File::open(path)?),
                Source::Installed(_) => unreachable!(),
            };
            let mut failure = None;
            let saved = atomic_write(&destination, false, |file| {
                let result = (|| -> Result<(), Error> {
                    let copied = std::io::copy(&mut source.by_ref().take(size + 1), file)?;
                    if copied != size {
                        return Err(FeedError::HashMismatch.into());
                    }
                    file.seek(SeekFrom::Start(0))?;
                    let mut digest = Sha256::new();
                    let mut buffer = [0_u8; 65536];
                    loop {
                        let count = file.read(&mut buffer)?;
                        if count == 0 {
                            break;
                        }
                        digest.update(&buffer[..count]);
                    }
                    if format!("{:x}", digest.finalize()) != hash {
                        return Err(FeedError::HashMismatch.into());
                    }
                    file.seek(SeekFrom::Start(0))?;
                    match &candidate.source {
                        Source::Catalog(version, _) => {
                            Archive::from_catalog(&mut *file, version).map_err(Error::Invalid)?;
                        }
                        _ => {
                            Archive::new(&mut *file).map_err(Error::Invalid)?;
                        }
                    }
                    Ok(())
                })();
                result.map_err(|error| {
                    let message = error.to_string();
                    failure = Some(error);
                    message
                })
            });
            if let Some(error) = failure {
                return Err(error);
            }
            saved.map_err(Error::Storage)?;
        }
        self.check_archive(&destination, candidate)?;
        Ok(Record {
            manifest: candidate.manifest.clone(),
            sha256: hash,
            origin,
        })
    }

    pub(super) fn apply_for(
        &self,
        plan: InstallPlan,
        confirmed: bool,
        environment: InstanceEnvironment,
    ) -> Result<(), Error> {
        if !confirmed {
            return Err(Error::ConfirmationRequired);
        }
        let _lock = store::lock(&plan.home)?;
        let (old, _, snapshot) = self.verified_state(&plan.home)?;
        if snapshot != plan.snapshot || environment != plan.environment {
            return Err(Error::Stale);
        }
        let mut records = Vec::new();
        let mut created = Vec::new();
        for candidate in &plan.chosen {
            environment.check(&candidate.manifest)?;
            let record = self.cache_candidate(&plan.home, candidate)?;
            let destination = store::directory(&plan.home, &record.manifest);
            store::plain(&destination)?;
            if destination.try_exists()? {
                if !old.records.iter().any(|existing| {
                    existing.manifest.id == record.manifest.id
                        && existing.manifest.version == record.manifest.version
                        && existing.sha256 == record.sha256
                }) {
                    return Err(Error::Invalid(
                        "A different archive already uses this mod version".into(),
                    ));
                }
            } else {
                created.push(record.manifest.clone());
            }
            records.push(record);
        }
        let mut next = old.clone();
        next.records.retain(|record| {
            !records
                .iter()
                .any(|other| record.manifest.id == other.manifest.id)
        });
        next.records.extend(records.iter().cloned());
        if next.records.len() > 128 {
            return Err(Error::Invalid("Too many installed mods".into()));
        }
        store::begin(&plan.home, &old, &created)?;
        let result = (|| {
            for manifest in &created {
                let record = records
                    .iter()
                    .find(|record| record.manifest.id == manifest.id)
                    .ok_or_else(|| Error::Invalid("Missing planned archive".into()))?;
                let path = store::archive_path(&plan.home, &record.sha256)?;
                let mut archive = Archive::new(File::open(path)?).map_err(Error::Invalid)?;
                archive
                    .extract_target(
                        &store::directory(&plan.home, manifest),
                        environment.target,
                        &self.keys,
                    )
                    .map_err(Error::Storage)?;
            }
            let order = plan
                .chosen
                .iter()
                .map(|candidate| candidate.manifest.id.clone())
                .collect::<Vec<_>>();
            store::commit(&plan.home, &old, &next, &order)
        })();
        if result.is_err() {
            store::recover(&plan.home)?;
        }
        result?;
        for record in &old.records {
            if records.iter().any(|other| {
                other.manifest.id == record.manifest.id
                    && other.manifest.version != record.manifest.version
            }) {
                store::remove_directory(&plan.home, &record.manifest)?;
            }
        }
        Ok(())
    }
}

fn enabled_order(state: &State, active: &[String]) -> Result<Vec<String>, Error> {
    let manifests = state
        .records
        .iter()
        .filter(|record| active.contains(&record.manifest.id))
        .map(|record| record.manifest.clone())
        .collect::<Vec<_>>();
    let resolution = yak::resolve(&manifests);
    if !resolution.is_compatible() {
        return Err(Error::Resolution(
            "Another enabled mod needs this mod; disable or remove it first".into(),
        ));
    }
    Ok(resolution.order)
}
