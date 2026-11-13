use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{mpsc, Arc};
use std::thread;

use crate::yoake_feed::{FeedClient, FeedError, ParsedFeedIndex, VerifiedFeed};
use crate::yoake_install::{self, InstalledPack};

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum UpdateState {
    UpToDate,
    Available {
        from: String,
        to: String,
        download_bytes: u64,
    },
    NeedsLogin,
    Offline,
    Error,
}

#[derive(Debug)]
pub struct InstanceUpdate {
    pub instance_dir: PathBuf,
    pub state: UpdateState,
}

pub trait UpdateFeedSource: Send + Sync {
    fn fetch(&self) -> Result<VerifiedFeed, FeedError>;
}

impl UpdateFeedSource for FeedClient {
    fn fetch(&self) -> Result<VerifiedFeed, FeedError> {
        let (_, auth, _) = self.install_connection();
        if auth
            .authorization()
            .map_err(FeedError::Authentication)?
            .is_none()
        {
            return Err(FeedError::NeedsLogin);
        }
        self.fetch_feed()
    }
}

pub struct UpdateTask {
    pub events: mpsc::Receiver<InstanceUpdate>,
    cancelled: Arc<AtomicBool>,
}

impl UpdateTask {
    pub fn cancel(&self) {
        self.cancelled.store(true, Ordering::Relaxed);
    }
}

pub fn start_update_check(client: FeedClient, instances: Vec<PathBuf>) -> UpdateTask {
    let (_, _, data_dir) = client.install_connection();
    start_update_check_with_source(
        Arc::new(client),
        instances,
        data_dir,
        env!("CARGO_PKG_VERSION").to_owned(),
    )
}

pub fn start_update_check_with_source(
    source: Arc<dyn UpdateFeedSource>,
    instances: Vec<PathBuf>,
    data_dir: PathBuf,
    launcher_version: String,
) -> UpdateTask {
    let (sender, events) = mpsc::channel();
    let cancelled = Arc::new(AtomicBool::new(false));
    let worker_cancelled = Arc::clone(&cancelled);
    thread::spawn(move || {
        let mut installed = Vec::new();
        for instance_dir in instances {
            if worker_cancelled.load(Ordering::Relaxed) {
                return;
            }
            match yoake_install::read_manifest(&instance_dir) {
                Ok(Some(manifest)) => installed.push((instance_dir, manifest)),
                Ok(None) => {}
                Err(_) => {
                    let _ = sender.send(InstanceUpdate {
                        instance_dir,
                        state: UpdateState::Error,
                    });
                }
            }
        }
        if installed.is_empty() || worker_cancelled.load(Ordering::Relaxed) {
            return;
        }
        let feed = source.fetch();
        for (instance_dir, manifest) in installed {
            if worker_cancelled.load(Ordering::Relaxed) {
                return;
            }
            let state = match &feed {
                Ok(feed) => compare(feed, &manifest, &instance_dir, &data_dir, &launcher_version),
                Err(FeedError::NeedsLogin | FeedError::HttpStatus(401 | 403)) => {
                    UpdateState::NeedsLogin
                }
                Err(FeedError::Network) => UpdateState::Offline,
                Err(_) => UpdateState::Error,
            };
            let _ = sender.send(InstanceUpdate {
                instance_dir,
                state,
            });
        }
    });
    UpdateTask { events, cancelled }
}

fn compare(
    feed: &VerifiedFeed,
    installed: &InstalledPack,
    instance_dir: &Path,
    data_dir: &Path,
    launcher_version: &str,
) -> UpdateState {
    let ParsedFeedIndex::V2(index) = &feed.index else {
        return UpdateState::Error;
    };
    if index.sequence < installed.sequence {
        return UpdateState::Error;
    }
    let Some(pack) = index.packs.iter().find(|pack| pack.id == installed.pack_id) else {
        return UpdateState::Error;
    };
    let Ok(current) = yoake_install::launcher_version(&installed.pack_version) else {
        return UpdateState::Error;
    };
    let mut newest = None;
    for version in &pack.versions {
        let Ok(parsed) = yoake_install::launcher_version(&version.version) else {
            return UpdateState::Error;
        };
        if newest
            .as_ref()
            .is_none_or(|(_, previous)| parsed > *previous)
        {
            newest = Some((version, parsed));
        }
    }
    let Some((newest, newest_version)) = newest else {
        return UpdateState::Error;
    };
    if newest_version <= current {
        return UpdateState::UpToDate;
    }
    if yoake_install::current_target().as_deref() != Ok(installed.target.as_str()) {
        return UpdateState::Error;
    }
    let Ok(plan) = yoake_install::plan_install_at(
        feed,
        &installed.pack_id,
        &newest.version,
        instance_dir,
        data_dir,
        &installed.target,
        launcher_version,
    ) else {
        return UpdateState::Error;
    };
    match yoake_install::download_bytes(&plan) {
        Ok(download_bytes) => UpdateState::Available {
            from: installed.pack_version.clone(),
            to: newest.version.clone(),
            download_bytes,
        },
        Err(_) => UpdateState::Error,
    }
}
