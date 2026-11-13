use super::*;
use horizon_launcher::yoake_mods::{
    Client, Error, InstallPlan, Installed, InstanceEnvironment, VerifiedCatalog,
};
use std::path::PathBuf;
use std::sync::mpsc::{self, Receiver};
use std::time::{Duration, Instant};

mod rows;

enum Outcome {
    Plan(InstallPlan),
    Done,
}

type Rows = Result<(Vec<Installed>, Result<InstanceEnvironment, Error>), Error>;

#[derive(Default)]
pub(super) struct State {
    home: Option<PathBuf>,
    rows: Vec<Installed>,
    environment: Option<InstanceEnvironment>,
    catalog: Option<VerifiedCatalog>,
    catalog_task: Option<Receiver<Result<VerifiedCatalog, Error>>>,
    rows_task: Option<(PathBuf, Receiver<Rows>)>,
    operation: Option<(PathBuf, Receiver<Result<Outcome, Error>>)>,
    preview: Option<InstallPlan>,
    error: Option<String>,
    login_required: bool,
    epoch: u64,
    last_refresh: Option<Instant>,
}

fn message(error: &Error) -> String {
    match error {
        Error::Feed(yoake_feed::FeedError::NeedsLogin | yoake_feed::FeedError::HttpStatus(401)) => {
            tr("Sign in to browse Yoake mods.").to_owned()
        }
        Error::UnsignedProgram => tr("A program .yak needs a valid Yoake signature.").to_owned(),
        Error::ConfirmationRequired => tr("Confirm the mod changes first.").to_owned(),
        Error::Stale => tr("The instance or file changed. Review the mod changes again.").to_owned(),
        Error::Busy => tr("Another mod operation is in progress.").to_owned(),
        Error::NotYoake => tr("Select an installed Yoake instance.").to_owned(),
        Error::Feed(yoake_feed::FeedError::BadSignature) => {
            tr("The Yoake mod catalog signature is invalid.").to_owned()
        }
        Error::Feed(yoake_feed::FeedError::HashMismatch) => {
            tr("The .yak download does not match the catalog.").to_owned()
        }
        _ => trf("Yoake mods: {reason}", &[("reason", &error.to_string())]),
    }
}

impl Launcher {
    pub(super) fn refresh_yak_catalog(&mut self) {
        if self.yak_mods.catalog_task.is_some() {
            return;
        }
        self.yak_mods.last_refresh = Some(Instant::now());
        let client = match Client::configured() {
            Ok(client) => client,
            Err(error) => {
                self.yak_mods.error = Some(message(&error));
                return;
            }
        };
        let (sender, receiver) = mpsc::channel();
        std::thread::spawn(move || {
            let _ = sender.send(client.refresh());
        });
        self.yak_mods.catalog_task = Some(receiver);
    }

    fn load_yak_rows(&mut self, home: PathBuf) {
        let (sender, receiver) = mpsc::channel();
        let selected = home.clone();
        std::thread::spawn(move || {
            let result = (|| {
                let rows = Client::configured()?.installed(&home)?;
                Ok((rows, InstanceEnvironment::read(&home)))
            })();
            let _ = sender.send(result);
        });
        self.yak_mods.rows_task = Some((selected, receiver));
    }

    pub(super) fn drain_yak_mods(&mut self) -> bool {
        let mut changed = false;
        let home = self
            .selected_instance()
            .filter(|instance| {
                instance.java.is_none() && yoake_install::has_manifest_or_pending(&instance.home)
            })
            .map(|instance| instance.home.clone());
        if self.yak_mods.home != home {
            self.yak_mods.home = home.clone();
            self.yak_mods.rows.clear();
            self.yak_mods.rows_task = None;
            self.yak_mods.environment = None;
            self.yak_mods.preview = None;
            self.yak_mods.error = None;
            if let Some(home) = home {
                self.load_yak_rows(home);
            }
            changed = true;
        }
        let epoch = yoake_auth::credential_epoch();
        if self.yak_mods.epoch != epoch {
            self.yak_mods.epoch = epoch;
            self.yak_mods.catalog = None;
            self.yak_mods.catalog_task = None;
            self.yak_mods.preview = None;
            self.yak_mods.last_refresh = None;
            changed = true;
        }
        if self.yak_mods.home.is_some()
            && self.yak_mods.catalog_task.is_none()
            && self
                .yak_mods
                .last_refresh
                .is_none_or(|time| time.elapsed() >= Duration::from_secs(300))
        {
            self.refresh_yak_catalog();
        }
        if let Some(receiver) = self.yak_mods.catalog_task.take() {
            match receiver.try_recv() {
                Ok(Ok(catalog)) => {
                    self.yak_mods.catalog = Some(catalog);
                    self.yak_mods.login_required = false;
                    changed = true;
                }
                Ok(Err(error)) => {
                    self.yak_mods.login_required = matches!(
                        error,
                        Error::Feed(
                            yoake_feed::FeedError::NeedsLogin
                                | yoake_feed::FeedError::HttpStatus(401)
                        )
                    );
                    self.yak_mods.catalog = None;
                    self.yak_mods.error = Some(message(&error));
                    changed = true;
                }
                Err(mpsc::TryRecvError::Empty) => self.yak_mods.catalog_task = Some(receiver),
                Err(mpsc::TryRecvError::Disconnected) => {
                    self.yak_mods.error = Some(tr("The mod operation stopped unexpectedly.").to_owned());
                    changed = true;
                }
            }
        }
        if let Some((home, receiver)) = self.yak_mods.rows_task.take() {
            match receiver.try_recv() {
                Ok(result) => {
                    if self.yak_mods.home.as_ref() == Some(&home) {
                        match result {
                            Ok((rows, environment)) => {
                                self.yak_mods.rows = rows;
                                match environment {
                                    Ok(environment) => {
                                        self.yak_mods.environment = Some(environment)
                                    }
                                    Err(error) => {
                                        self.yak_mods.environment = None;
                                        self.yak_mods.error = Some(message(&error));
                                    }
                                }
                            }
                            Err(error) => self.yak_mods.error = Some(message(&error)),
                        }
                        changed = true;
                    }
                }
                Err(mpsc::TryRecvError::Empty) => self.yak_mods.rows_task = Some((home, receiver)),
                Err(mpsc::TryRecvError::Disconnected) => {
                    self.yak_mods.error = Some(tr("The mod operation stopped unexpectedly.").to_owned());
                    changed = true;
                }
            }
        }
        if let Some((home, receiver)) = self.yak_mods.operation.take() {
            match receiver.try_recv() {
                Ok(result) => {
                    if self.yak_mods.home.as_ref() == Some(&home) {
                        match result {
                            Ok(Outcome::Plan(plan)) => self.yak_mods.preview = Some(plan),
                            Ok(Outcome::Done) => self.load_yak_rows(home),
                            Err(error) => self.yak_mods.error = Some(message(&error)),
                        }
                    }
                    changed = true;
                }
                Err(mpsc::TryRecvError::Empty) => self.yak_mods.operation = Some((home, receiver)),
                Err(mpsc::TryRecvError::Disconnected) => {
                    self.yak_mods.error = Some(tr("The mod operation stopped unexpectedly.").to_owned());
                    changed = true;
                }
            }
        }
        changed
    }

    fn yak_busy(&self) -> bool {
        self.yak_mods.operation.is_some() || self.yak_mods.rows_task.is_some()
    }

    fn yak_operation(
        &mut self,
        home: PathBuf,
        work: impl FnOnce(Client) -> Result<Outcome, Error> + Send + 'static,
    ) {
        if self.yak_busy() {
            return;
        }
        self.yak_mods.error = None;
        self.yak_mods.preview = None;
        let (sender, receiver) = mpsc::channel();
        std::thread::spawn(move || {
            let _ = sender.send(Client::configured().and_then(work));
        });
        self.yak_mods.operation = Some((home, receiver));
    }

    fn plan_catalog_yak(&mut self, id: String) {
        let Some(home) = self.yak_mods.home.clone() else {
            return;
        };
        let Some(catalog) = self.yak_mods.catalog.clone() else {
            return;
        };
        let selected = home.clone();
        self.yak_operation(home, move |client| {
            client
                .plan_install(&selected, &catalog, &id)
                .map(Outcome::Plan)
        });
    }

    fn add_local_yak(&mut self, home: PathBuf, paths: Vec<PathBuf>) {
        if self.yak_mods.home.as_ref() != Some(&home) || self.yak_busy() {
            return;
        }
        if paths.len() != 1 {
            self.yak_mods.error = Some(tr("Choose one .yak file at a time.").to_owned());
            return;
        }
        let path = paths.into_iter().next().unwrap();
        let catalog = self.yak_mods.catalog.clone();
        let selected = home.clone();
        let dev_unsigned_dll = self.settings.dev_unsigned_dll;
        self.yak_operation(home, move |client| {
            client
                .plan_local(&selected, &path, catalog.as_ref(), dev_unsigned_dll)
                .map(Outcome::Plan)
        });
    }

    fn choose_local_yak(&mut self, cx: &mut Context<Self>) {
        let Some(home) = self.yak_mods.home.clone() else {
            return;
        };
        let paths = cx.prompt_for_paths(PathPromptOptions {
            files: true,
            directories: false,
            multiple: false,
            prompt: Some(tr("Add a .yak file…").into()),
        });
        cx.spawn(async move |this, cx| {
            if let Ok(Ok(Some(paths))) = paths.await {
                let _ = this.update(cx, |this, cx| {
                    this.add_local_yak(home, paths);
                    cx.notify();
                });
            }
        })
        .detach();
    }
}
