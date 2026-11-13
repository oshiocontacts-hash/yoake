use super::store::{Origin, Record, State};
use super::*;
use sha2::{Digest, Sha256};
use std::io::Read;

mod resolver;
#[cfg(test)]
mod tests;
mod transaction;

#[derive(Clone)]
pub struct Installed {
    pub manifest: Manifest,
    pub enabled: bool,
    pub unsigned: bool,
}

pub struct InstallPlan {
    pub changes: Vec<Manifest>,
    pub unsigned: bool,
    pub unsigned_program: bool,
    home: PathBuf,
    environment: InstanceEnvironment,
    snapshot: String,
    chosen: Vec<Candidate>,
}

#[derive(Clone)]
enum Source {
    Installed(Record),
    Catalog(CatalogVersion, Proof),
    Local {
        path: PathBuf,
        hash: String,
        signed: bool,
    },
}

#[derive(Clone)]
struct Candidate {
    manifest: Manifest,
    source: Source,
}

fn hash_file(path: &Path) -> Result<String, Error> {
    store::plain(path)?;
    let mut input = File::open(path)?;
    if !input.metadata()?.is_file() || input.metadata()?.len() > yak::MAX_ARCHIVE_SIZE {
        return Err(Error::Invalid("Not a bounded .yak file".into()));
    }
    let mut digest = Sha256::new();
    let mut total = 0_u64;
    let mut buffer = [0_u8; 65536];
    loop {
        let count = input.read(&mut buffer)?;
        if count == 0 {
            break;
        }
        total += count as u64;
        if total > yak::MAX_ARCHIVE_SIZE {
            return Err(FeedError::TooLarge.into());
        }
        digest.update(&buffer[..count]);
    }
    Ok(format!("{:x}", digest.finalize()))
}

fn same_manifest(left: &Manifest, right: &Manifest) -> bool {
    serde_json::to_value(left).ok() == serde_json::to_value(right).ok()
}

impl Client {
    fn verified_state(&self, home: &Path) -> Result<(State, Vec<String>, String), Error> {
        let loaded = store::load(home)?;
        for record in &loaded.0.records {
            let path = store::archive_path(home, &record.sha256)?;
            if hash_file(&path)? != record.sha256 {
                return Err(FeedError::HashMismatch.into());
            }
            let archive = match &record.origin {
                Origin::Catalog(digest) => {
                    let proof_path = store::cache(home).join(format!("proof-{digest}.json"));
                    store::archive_path(home, digest)?;
                    let proof: Proof = store::read_json(&proof_path)?;
                    if horizon_release_sign::sha256_hex(proof.json.as_bytes()) != *digest {
                        return Err(FeedError::BadSignature.into());
                    }
                    let verified = self.verify(proof, true)?;
                    let version = verified
                        .catalog
                        .entries
                        .iter()
                        .filter(|entry| entry.id == record.manifest.id)
                        .flat_map(|entry| &entry.versions)
                        .find(|version| {
                            version.version == record.manifest.version
                                && version.sha256 == record.sha256
                        })
                        .ok_or(FeedError::BadSignature)?;
                    Archive::from_catalog(File::open(&path)?, version).map_err(Error::Invalid)?
                }
                _ => Archive::new(File::open(&path)?).map_err(Error::Invalid)?,
            };
            let signed = archive
                .verify_signature(&self.keys)
                .map_err(Error::Invalid)?
                .is_some();
            if matches!(record.origin, Origin::Signed) && !signed {
                return Err(Error::UnsignedProgram);
            }
            if archive.manifest().bin.is_some()
                && !signed
                && !matches!(record.origin, Origin::UnsignedData)
            {
                return Err(Error::UnsignedProgram);
            }
            if !same_manifest(archive.manifest(), &record.manifest) {
                return Err(FeedError::HashMismatch.into());
            }
            let installed_root = store::directory(home, &record.manifest);
            yak::check_installed_tree(&installed_root).map_err(Error::Storage)?;
            for (part, entries) in archive.parts() {
                if matches!(part, yak::Part::Bin(target) if target != Target::host().map_err(Error::Incompatible)?)
                {
                    continue;
                }
                for entry in entries {
                    if let Some(expected) = archive.hashes().get(&entry.name) {
                        let actual = yak::hash_installed_file(&installed_root, &entry.name)
                            .map_err(Error::Storage)?;
                        if &actual != expected {
                            return Err(FeedError::HashMismatch.into());
                        }
                    }
                }
            }
            let installed_path = installed_root.join("yak.toml");
            store::plain(&installed_path)?;
            let installed = Manifest::read(&installed_path).map_err(Error::Invalid)?;
            if !same_manifest(&installed, archive.manifest()) {
                return Err(FeedError::HashMismatch.into());
            }
        }
        Ok(loaded)
    }

    pub fn installed(&self, home: &Path) -> Result<Vec<Installed>, Error> {
        let _lock = store::lock(home)?;
        let (state, enabled, _) = self.verified_state(home)?;
        Ok(state
            .records
            .into_iter()
            .map(|record| Installed {
                enabled: enabled.contains(&record.manifest.id),
                unsigned: matches!(record.origin, Origin::UnsignedData),
                manifest: record.manifest,
            })
            .collect())
    }

    pub fn update_available(
        &self,
        installed: &Manifest,
        catalog: &VerifiedCatalog,
        environment: &InstanceEnvironment,
    ) -> bool {
        catalog
            .catalog
            .entries
            .iter()
            .filter(|entry| entry.id == installed.id)
            .flat_map(|entry| &entry.versions)
            .any(|version| {
                version.version > installed.version && environment.check(&version.manifest).is_ok()
            })
    }

    pub fn plan_install(
        &self,
        home: &Path,
        catalog: &VerifiedCatalog,
        id: &str,
    ) -> Result<InstallPlan, Error> {
        self.plan_for(
            home,
            Some(catalog),
            &[(id.to_owned(), None)],
            false,
            None,
            InstanceEnvironment::read(home)?,
        )
    }

    pub fn plan_set(
        &self,
        home: &Path,
        catalog: &VerifiedCatalog,
        requested: &[(String, String)],
    ) -> Result<InstallPlan, Error> {
        let requested = requested
            .iter()
            .map(|(id, version)| {
                Ok((
                    id.clone(),
                    Some(
                        version
                            .parse()
                            .map_err(|error: semver::Error| Error::Invalid(error.to_string()))?,
                    ),
                ))
            })
            .collect::<Result<Vec<_>, Error>>()?;
        self.plan_for(
            home,
            Some(catalog),
            &requested,
            true,
            None,
            InstanceEnvironment::read(home)?,
        )
    }

    pub fn plan_local(
        &self,
        home: &Path,
        path: &Path,
        catalog: Option<&VerifiedCatalog>,
        dev_unsigned_dll: bool,
    ) -> Result<InstallPlan, Error> {
        self.local_for(
            home,
            path,
            catalog,
            InstanceEnvironment::read(home)?,
            dev_unsigned_dll,
        )
    }

    fn local_for(
        &self,
        home: &Path,
        path: &Path,
        catalog: Option<&VerifiedCatalog>,
        environment: InstanceEnvironment,
        dev_unsigned_dll: bool,
    ) -> Result<InstallPlan, Error> {
        if !path
            .extension()
            .and_then(|extension| extension.to_str())
            .is_some_and(|extension| extension.eq_ignore_ascii_case("yak"))
        {
            return Err(Error::Invalid("Choose a .yak file".into()));
        }
        let hash = hash_file(path)?;
        let archive = Archive::new(File::open(path)?).map_err(Error::Invalid)?;
        let signed = archive
            .verify_signature(&self.keys)
            .map_err(Error::Invalid)?
            .is_some();
        if archive.manifest().bin.is_some() && !signed && !dev_unsigned_dll {
            return Err(Error::UnsignedProgram);
        }
        let manifest = archive.manifest().clone();
        let requested = [(manifest.id.clone(), Some(manifest.version.clone()))];
        self.plan_for(
            home,
            catalog,
            &requested,
            false,
            Some(Candidate {
                manifest,
                source: Source::Local {
                    path: path.to_path_buf(),
                    hash,
                    signed,
                },
            }),
            environment,
        )
    }

    fn plan_for(
        &self,
        home: &Path,
        catalog: Option<&VerifiedCatalog>,
        requested: &[(String, Option<Version>)],
        replace: bool,
        local: Option<Candidate>,
        environment: InstanceEnvironment,
    ) -> Result<InstallPlan, Error> {
        let _lock = store::lock(home)?;
        let catalog = catalog
            .map(|catalog| self.verify(catalog.proof.clone(), catalog.cached))
            .transpose()?;
        let (state, enabled, snapshot) = self.verified_state(home)?;
        let chosen = resolver::choose(
            &state,
            &enabled,
            catalog.as_ref(),
            requested,
            replace,
            local,
            &environment,
        )?;
        let changes = chosen
            .iter()
            .filter(|candidate| !matches!(candidate.source, Source::Installed(_)))
            .map(|candidate| candidate.manifest.clone())
            .collect();
        let unsigned = chosen
            .iter()
            .any(|candidate| matches!(candidate.source, Source::Local { signed: false, .. }));
        let unsigned_program = chosen.iter().any(|candidate| {
            matches!(candidate.source, Source::Local { signed: false, .. })
                && candidate.manifest.bin.is_some()
        });
        Ok(InstallPlan {
            changes,
            unsigned,
            unsigned_program,
            home: home.to_path_buf(),
            environment,
            snapshot,
            chosen,
        })
    }
}
