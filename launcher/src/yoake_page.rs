//! The Yoake page: the group's signed modpacks (`yoake_feed`), sign-in to the portal
//! (`yoake_login`: members by a code approved in the browser, friends by an invite code), and
//! each installed Yoake instance with its update, rollback and progress (`yoake_install`,
//! `yoake_update`). Only in the top bar when the launcher was built with a feed or a portal.
use super::*;
use std::path::PathBuf;
use std::sync::mpsc;
use std::sync::{
    Arc,
    atomic::{AtomicBool, AtomicU8},
};
use yoake_feed::{FeedClient, FeedError, ParsedFeedIndex, VerifiedFeed};
use yoake_install::{InstallError, InstallEvent, InstallTask};

struct BaseTask {
    home: PathBuf,
    play: bool,
    cancel: Arc<AtomicBool>,
    events: Option<mpsc::Receiver<BaseEvent>>,
    preparing: Option<String>,
}

enum BaseEvent {
    Progress(f32),
    Finished(Result<String, String>),
}

struct LauncherTask {
    events: mpsc::Receiver<LauncherEvent>,
    state: Arc<AtomicU8>,
}

enum LauncherEvent {
    Progress(f32),
    Finished(Result<(), String>),
}

/// What the page keeps between frames.
pub(super) struct YoakePage {
    /// The verified feed, once fetched, or why it could not be.
    catalog: Option<Result<VerifiedFeed, FeedError>>,
    catalog_task: Option<mpsc::Receiver<Result<VerifiedFeed, FeedError>>>,
    launcher_update: Option<(String, String)>,
    launcher_update_task: Option<mpsc::Receiver<(bool, Result<Option<(String, String)>, String>)>>,
    launcher_update_checked: bool,
    launcher_job: Option<LauncherTask>,
    launcher_progress: f32,
    launcher_ready: bool,
    launcher_error: Option<String>,
    /// The install, update or rollback running now, and the instance it works on.
    job: Option<(PathBuf, InstallTask)>,
    base: Option<BaseTask>,
    /// How far the job is (0..1) and the file it is on.
    progress: Option<(f32, String)>,
    /// The last job's outcome, shown under the account.
    note: Option<(bool, String)>,
    pub(super) invite_code: line_edit::LineEdit,
    pub(super) invite_name: line_edit::LineEdit,
    pub(super) code_focus: FocusHandle,
    pub(super) name_focus: FocusHandle,
    code_focused: bool,
    name_focused: bool,
}

impl YoakePage {
    pub(super) fn new(cx: &mut App) -> Self {
        let mut page = Self {
            catalog: None,
            catalog_task: None,
            launcher_update: None,
            launcher_update_task: None,
            launcher_update_checked: false,
            launcher_job: None,
            launcher_progress: 0.,
            launcher_ready: false,
            launcher_error: None,
            job: None,
            base: None,
            progress: None,
            note: None,
            invite_code: line_edit::LineEdit::new(String::new(), 32, 16),
            invite_name: line_edit::LineEdit::new(String::new(), 32, 16),
            code_focus: cx.focus_handle(),
            name_focus: cx.focus_handle(),
            code_focused: false,
            name_focused: false,
        };
        if enabled() {
            page.check_launcher_update(false);
        }
        page
    }

    pub(super) fn check_launcher_update(&mut self, force: bool) {
        if self.launcher_job.is_some()
            || self.launcher_ready
            || self.launcher_update_task.is_some()
            || (!force && self.launcher_update_checked)
        {
            return;
        }
        let Some(portal) = option_env!("YOAKE_PORTAL_URL") else {
            if force {
                self.launcher_error = Some("This launcher has no portal configured.".into());
            }
            return;
        };
        if !signed_in() {
            self.launcher_update = None;
            self.launcher_update_checked = false;
            if force {
                self.launcher_error =
                    Some("Please sign in to the portal before checking for updates.".into());
            }
            return;
        }
        self.launcher_update_checked = true;
        if force {
            self.launcher_error = None;
            self.note = None;
        }
        let (tx, rx) = mpsc::channel();
        std::thread::spawn(move || {
            let auth = yoake_auth::TokenFileAuth::new(&backend::data_dir());
            let result = yoake_launcher_update::check_result(
                portal,
                &auth,
                &yoake_feed::HttpTransport,
                env!("CARGO_PKG_VERSION"),
            );
            let _ = tx.send((force, result));
        });
        self.launcher_update_task = Some(rx);
    }
}

/// True when the launcher was built with a Yoake feed or portal: the page is shown then.
pub(super) fn enabled() -> bool {
    horizon_launcher::brand::yoake()
}

fn signed_in() -> bool {
    matches!(
        yoake_auth::YoakeAuth::authorization(&yoake_auth::TokenFileAuth::new(&backend::data_dir())),
        Ok(Some(_))
    )
}

/// The saved account (`yoake-account.json`): member or friend, and when a friend's ends.
fn account() -> Option<yoake_auth::YoakeAccount> {
    let bytes = std::fs::read(backend::data_dir().join("yoake-account.json")).ok()?;
    serde_json::from_slice(&bytes).ok()
}

fn device_label() -> String {
    std::env::var("COMPUTERNAME")
        .or_else(|_| std::env::var("HOSTNAME"))
        .unwrap_or_else(|_| "Yoake".to_string())
}

/// "12.3 MB": a download size in the units people read.
fn size_text(bytes: u64) -> String {
    const MB: f64 = 1024. * 1024.;
    if bytes < 1024 * 1024 {
        trf("{n} KB", &[("n", &bytes.div_ceil(1024).to_string())])
    } else {
        trf("{n} MB", &[("n", &format!("{:.1}", bytes as f64 / MB))])
    }
}

/// A login state in words.
fn login_text(event: &yoake_login::LoginEvent) -> String {
    use yoake_login::LoginEvent as E;
    match event {
        E::VerificationRequired { .. } | E::AuthorizationPending => {
            tr("Approve the sign-in on the portal page that opened in your browser.").to_string()
        }
        E::SlowDown { .. } => tr("Still waiting for the approval in your browser.").to_string(),
        E::Success(_) => tr("Signed in.").to_string(),
        E::WrongCode => tr("That invite code is wrong.").to_string(),
        E::ExpiredOrUsed => tr("That invite has expired or has been used up.").to_string(),
        E::TooManyTries => tr("Too many tries. Wait a while and try again.").to_string(),
        E::Expired => tr("The sign-in took too long. Start it again.").to_string(),
        E::AccessDenied => tr("The sign-in was refused on the portal.").to_string(),
        E::Cancelled => tr("Sign-in cancelled.").to_string(),
        E::Offline => tr("Cannot reach the portal. Check the connection.").to_string(),
        E::InvalidResponse => tr("The portal answered something unexpected.").to_string(),
        E::StorageError => tr("Could not save the sign-in on this computer.").to_string(),
    }
}

/// An install error in words.
pub(super) fn install_text(error: &InstallError) -> String {
    use InstallError as E;
    match error {
        E::NeedsLogin | E::Authentication => tr("Sign in first.").to_string(),
        E::Network | E::HttpStatus(_) => {
            tr("Cannot reach the pack server. Check the connection.").to_string()
        }
        E::LauncherTooOld => tr("This pack needs a newer launcher.").to_string(),
        E::UnsupportedTarget => tr("This pack is not made for this computer.").to_string(),
        E::NoRollback => tr("There is no earlier version to go back to.").to_string(),
        E::FeedRequired => {
            tr("The earlier version is not on this computer any more; go online to get it.")
                .to_string()
        }
        E::MissingBase => {
            tr("The pack's game files are missing; install the pack again.").to_string()
        }
        E::UnsupportedNativeMods => {
            tr("The pack's mods do not match its game build; ask the pack's maker.").to_string()
        }
        E::Conflict(path) | E::BaseConflict(path) => trf(
            "A file of yours is in the way: {path}. Move it and try again.",
            &[("path", path)],
        ),
        E::Cancelled => tr("Cancelled.").to_string(),
        E::Busy => tr("Another install is running on this instance.").to_string(),
        E::Io(e) => trf("Could not write the files: {error}", &[("error", e)]),
        other => trf(
            "The pack could not be installed ({error}).",
            &[("error", &format!("{other:?}"))],
        ),
    }
}

fn feed_text(error: &FeedError) -> String {
    match error {
        FeedError::NeedsLogin | FeedError::Authentication(_) | FeedError::HttpStatus(401 | 403) => {
            tr("Sign in to see the packs.").to_string()
        }
        FeedError::Network | FeedError::HttpStatus(_) => {
            tr("Cannot reach the pack server. Check the connection.").to_string()
        }
        _ => tr("The pack list could not be checked; it may have been tampered with.").to_string(),
    }
}

/// The newest version of a pack, by the launcher's version order.
fn newest(pack: &yoake_feed::PackV2) -> Option<&yoake_feed::PackVersionV2> {
    pack.versions
        .iter()
        .filter_map(|v| {
            yoake_install::launcher_version(&v.version)
                .ok()
                .map(|p| (v, p))
        })
        .max_by(|a, b| a.1.cmp(&b.1))
        .map(|(v, _)| v)
}

impl Launcher {
    fn begin_launcher_update(&mut self) {
        let Some((version, _)) = self.yoake.launcher_update.clone() else {
            return;
        };
        if self.yoake.launcher_job.is_some() || self.yoake.launcher_ready {
            return;
        }
        let (tx, rx) = mpsc::channel();
        let state = Arc::new(AtomicU8::new(0));
        let worker_state = Arc::clone(&state);
        std::thread::spawn(move || {
            let result = (|| {
                let portal = option_env!("YOAKE_PORTAL_URL")
                    .ok_or("This launcher has no portal configured.")?;
                let config = yoake_feed::FeedConfig::compiled()
                    .map_err(|_| "The launcher verification keys are invalid.")?
                    .ok_or("This launcher has no verification keys configured.")?;
                let data_dir = backend::data_dir();
                let auth = yoake_auth::TokenFileAuth::new(&data_dir);
                yoake_self_update::install(
                    portal,
                    &auth,
                    &yoake_feed::HttpTransport,
                    config.public_keys(),
                    env!("CARGO_PKG_VERSION"),
                    &version,
                    &data_dir,
                    &worker_state,
                    &mut |progress| {
                        let _ = tx.send(LauncherEvent::Progress(progress));
                    },
                )
            })();
            let _ = tx.send(LauncherEvent::Finished(result));
        });
        self.yoake.launcher_job = Some(LauncherTask { events: rx, state });
        self.yoake.launcher_progress = 0.;
        self.yoake.launcher_error = None;
    }

    /// Fetches the feed in the background (Refresh, sign-in, opening the page).
    pub(super) fn yoake_refresh(&mut self) {
        self.yoake.check_launcher_update(false);
        let Ok(Some(client)) = FeedClient::configured() else {
            return;
        };
        let (tx, rx) = mpsc::channel();
        std::thread::spawn(move || {
            let _ = tx.send(client.fetch_feed());
        });
        self.yoake.catalog_task = Some(rx);
        self.check_yoake_updates();
    }

    /// The page's background work: the feed and the running job. True when something changed.
    pub(super) fn drain_yoake_page(&mut self) -> bool {
        let mut changed = false;
        let mut launcher_finished = None;
        if let Some(job) = &self.yoake.launcher_job {
            loop {
                match job.events.try_recv() {
                    Ok(LauncherEvent::Progress(progress)) => {
                        self.yoake.launcher_progress = progress;
                        changed = true;
                    }
                    Ok(LauncherEvent::Finished(result)) => {
                        launcher_finished = Some(result);
                        break;
                    }
                    Err(mpsc::TryRecvError::Disconnected) => {
                        launcher_finished =
                            Some(Err("The launcher update stopped unexpectedly.".into()));
                        break;
                    }
                    Err(mpsc::TryRecvError::Empty) => break,
                }
            }
        }
        if let Some(result) = launcher_finished {
            self.yoake.launcher_job = None;
            match result {
                Ok(()) => self.yoake.launcher_ready = true,
                Err(error) => self.yoake.launcher_error = Some(error),
            }
            changed = true;
        }
        if let Some(rx) = &self.yoake.launcher_update_task {
            match rx.try_recv() {
                Ok((force, result)) => {
                    if self.yoake.launcher_job.is_none()
                        && !self.yoake.launcher_ready
                        && signed_in()
                    {
                        match result {
                            Ok(update) => {
                                if force && update.is_none() {
                                    self.yoake.note = Some((
                                        true,
                                        trf("{id} is up to date.", &[("id", "Yoake")]),
                                    ));
                                }
                                self.yoake.launcher_update = update;
                                self.yoake.launcher_error = None;
                            }
                            Err(error) => {
                                self.yoake.launcher_update = None;
                                self.yoake.launcher_error = Some(error);
                            }
                        }
                    }
                    self.yoake.launcher_update_task = None;
                    changed = true;
                }
                Err(mpsc::TryRecvError::Disconnected) => {
                    self.yoake.launcher_update_task = None;
                    self.yoake.launcher_error =
                        Some("The launcher update check stopped unexpectedly.".into());
                    changed = true;
                }
                Err(mpsc::TryRecvError::Empty) => {}
            }
        }
        if let Some(rx) = &self.yoake.catalog_task {
            match rx.try_recv() {
                Ok(result) => {
                    self.yoake.catalog = Some(result);
                    self.yoake.catalog_task = None;
                    changed = true;
                }
                Err(mpsc::TryRecvError::Disconnected) => {
                    self.yoake.catalog_task = None;
                    changed = true;
                }
                Err(mpsc::TryRecvError::Empty) => {}
            }
        }
        let mut finished = None;
        if let Some((_, task)) = &self.yoake.job {
            while let Ok(event) = task.events.try_recv() {
                changed = true;
                match event {
                    InstallEvent::Download {
                        bytes_done,
                        total_bytes,
                        current_file,
                    } => {
                        let p = if total_bytes == 0 {
                            0.
                        } else {
                            bytes_done as f32 / total_bytes as f32
                        };
                        // Downloading is the first 90 %; applying the files the rest.
                        self.yoake.progress = Some((p * 0.9, current_file));
                    }
                    InstallEvent::Applying { current_file } => {
                        self.yoake.progress = Some((0.95, current_file));
                    }
                    InstallEvent::Finished(result) => finished = Some(result),
                }
            }
        }
        if let Some(result) = finished {
            let home = self.yoake.job.as_ref().map(|(home, _)| home.clone());
            self.yoake.job = None;
            self.yoake.progress = None;
            self.reload_instances();
            self.check_yoake_updates();
            match (result, home) {
                (Ok(_), Some(home)) => self.begin_yoake_base(home, false),
                (Err(error), _) => self.yoake.note = Some((false, install_text(&error))),
                _ => {}
            }
        }
        changed |= self.drain_yoake_base();
        changed
    }

    pub(super) fn yoake_busy(&self) -> bool {
        self.yoake.job.is_some() || self.yoake.base.is_some()
    }

    pub(super) fn yoake_base_pending_play(&self) -> bool {
        self.yoake.base.as_ref().is_some_and(|base| base.play)
    }

    pub(super) fn yoake_prepare_cancel(&self) -> Option<Arc<AtomicBool>> {
        self.yoake
            .base
            .as_ref()
            .filter(|base| base.preparing.is_some())
            .map(|base| Arc::clone(&base.cancel))
    }

    pub(super) fn begin_yoake_base(&mut self, home: PathBuf, play: bool) {
        if self.yoake_busy() {
            return;
        }
        self.yoake.base = Some(BaseTask {
            home,
            play,
            cancel: Arc::new(AtomicBool::new(false)),
            events: None,
            preparing: None,
        });
        self.yoake.progress = Some((0., tr("Waiting to prepare the game…").to_string()));
        self.yoake.note = None;
    }

    pub(super) fn cancel_yoake_base(&mut self) {
        if let Some(base) = &self.yoake.base {
            base.cancel.store(true, Ordering::Relaxed);
            if base.preparing.is_some() {
                horizon_launcher::mojang::CANCEL.store(true, Ordering::Relaxed);
            }
        }
    }

    pub(super) fn yoake_base_progress(&self) -> Option<(f32, String)> {
        let base = self.yoake.base.as_ref()?;
        let instance = self.selected_instance()?;
        (instance.home == base.home)
            .then(|| self.yoake.progress.clone())
            .flatten()
    }

    fn complete_yoake_base(&mut self, result: Result<String, String>) {
        let Some(base) = self.yoake.base.take() else {
            return;
        };
        self.yoake.progress = None;
        if base.cancel.load(Ordering::Relaxed) {
            if base.play {
                self.quick_play = None;
                self.joining = None;
            }
            self.yoake.note = Some((false, tr("Cancelled.").to_string()));
            return;
        }
        match result {
            Ok(tag) if base.play => {
                if self
                    .selected_instance()
                    .is_some_and(|instance| instance.home == base.home)
                    && self.client.is_none()
                    && self.starting_client.is_none()
                {
                    self.start_client(tag);
                } else {
                    self.quick_play = None;
                    self.joining = None;
                    self.yoake.note = Some((
                        false,
                        tr("Select the pack and press Play again.").to_string(),
                    ));
                }
            }
            Ok(_) => self.yoake.note = Some((true, tr("The pack is ready to play.").to_string())),
            Err(error) => {
                if base.play {
                    self.quick_play = None;
                    self.joining = None;
                }
                self.client_error = Some(error.clone());
                self.yoake.note = Some((false, error));
            }
        }
    }

    pub(super) fn finish_yoake_base_prepare(&mut self) {
        let Some(tag) = self
            .yoake
            .base
            .as_ref()
            .and_then(|base| base.preparing.clone())
        else {
            return;
        };
        let result = if self.prepare_error.is_none() && backend::is_prepared(&tag) {
            Ok(tag)
        } else {
            Err(
                tr("Could not prepare the game. Check your Minecraft installation and try again.")
                    .to_string(),
            )
        };
        self.complete_yoake_base(result);
    }

    fn drain_yoake_base(&mut self) -> bool {
        let Some(base) = self.yoake.base.as_mut() else {
            return false;
        };
        if base.events.is_none() && base.preparing.is_none() {
            if base.cancel.load(Ordering::Relaxed) {
                self.complete_yoake_base(Err(tr("Cancelled.").to_string()));
                return true;
            }
            if self.installing.is_some() || self.preparing || !self.removing.is_empty() {
                return false;
            }
            let home = base.home.clone();
            let cancel = Arc::clone(&base.cancel);
            let (tx, rx) = mpsc::channel();
            base.events = Some(rx);
            self.yoake.progress = Some((0., tr("Checking the game version…").to_string()));
            std::thread::spawn(move || {
                let result = (|| {
                    let program = yoake_install::prepare_launch(&home, &backend::data_dir())
                        .map_err(|error| install_text(&error))?
                        .ok_or_else(|| {
                            tr("The pack's game files are missing; install the pack again.")
                                .to_string()
                        })?;
                    let tag = program.resource_tag;
                    if cancel.load(Ordering::Relaxed) {
                        return Err(tr("Cancelled.").to_string());
                    }
                    if !backend::installed().contains(&tag) {
                        let release = backend::yoake_release(&tag)
                            .map_err(|_| {
                                tr("Cannot check game versions. Check the connection and try again.")
                                    .to_string()
                            })?
                            .ok_or_else(|| {
                                trf(
                                    "The required game version {version} is not available.",
                                    &[("version", &tag)],
                                )
                            })?;
                        let progress = |done| {
                            let _ = tx.send(BaseEvent::Progress(done));
                        };
                        backend::install_for_yoake(&release, progress, &cancel).map_err(|_| {
                            trf(
                                "Could not install game version {version}. Check the connection and try again.",
                                &[("version", &tag)],
                            )
                        })?;
                    }
                    Ok(tag)
                })();
                let _ = tx.send(BaseEvent::Finished(result));
            });
            return true;
        }
        if base.preparing.is_some() {
            if let Some((done, total)) = horizon_launcher::mojang::progress() {
                let fraction = done as f32 / total.max(1) as f32;
                self.yoake.progress =
                    Some((fraction.min(1.), tr("Preparing game files…").to_string()));
                return true;
            }
            return false;
        }
        let mut finished = None;
        let mut changed = false;
        if let Some(events) = &base.events {
            loop {
                match events.try_recv() {
                    Ok(BaseEvent::Progress(done)) => {
                        self.yoake.progress =
                            Some((done, tr("Installing the game version…").to_string()));
                        changed = true;
                    }
                    Ok(BaseEvent::Finished(result)) => {
                        finished = Some(result);
                        break;
                    }
                    Err(mpsc::TryRecvError::Disconnected) => {
                        finished = Some(Err(
                            tr("Could not install the game version. Try again.").to_string()
                        ));
                        break;
                    }
                    Err(mpsc::TryRecvError::Empty) => break,
                }
            }
        }
        if let Some(result) = finished {
            self.installed = backend::installed();
            match result {
                Ok(tag) if !backend::is_prepared(&tag) && !base.cancel.load(Ordering::Relaxed) => {
                    base.events = None;
                    base.preparing = Some(tag.clone());
                    self.yoake.progress = Some((0., tr("Preparing game files…").to_string()));
                    self.prepare_tag(tag, backend::MinecraftFrom::Install);
                }
                result => self.complete_yoake_base(result),
            }
            return true;
        }
        changed
    }

    /// Installs `version` of pack `id` into a new instance named after the pack.
    fn yoake_install_new(&mut self, id: String, name: String, version: String) {
        let (Ok(Some(client)), Some(Ok(feed))) = (FeedClient::configured(), &self.yoake.catalog)
        else {
            return;
        };
        let instance = match instances::create(&name, None) {
            Ok(instance) => instance,
            Err(e) => {
                self.yoake.note = Some((false, e));
                return;
            }
        };
        let home = instance.home.clone();
        let task = yoake_install::start_install(&client, feed.clone(), id, version, home.clone());
        self.yoake.job = Some((home, task));
        self.yoake.progress = Some((0., String::new()));
        self.yoake.note = None;
        self.reload_instances();
    }

    /// Updates the instance in `home` to `version` of its pack.
    fn yoake_update(&mut self, home: PathBuf, id: String, version: String) {
        let (Ok(Some(client)), Some(Ok(feed))) = (FeedClient::configured(), &self.yoake.catalog)
        else {
            self.yoake_refresh();
            return;
        };
        let task = yoake_install::start_install(&client, feed.clone(), id, version, home.clone());
        self.yoake.job = Some((home, task));
        self.yoake.progress = Some((0., String::new()));
        self.yoake.note = None;
    }

    fn yoake_rollback(&mut self, home: PathBuf) {
        let task = yoake_install::rollback(home.clone());
        self.yoake.job = Some((home, task));
        self.yoake.progress = Some((0., String::new()));
        self.yoake.note = None;
    }

    pub(super) fn yoake_page(&mut self, window: &Window, cx: &mut Context<Self>) -> Div {
        let t = self.theme;
        self.yoake.code_focused = self.yoake.code_focus.is_focused(window);
        self.yoake.name_focused = self.yoake.name_focus.is_focused(window);
        if self.yoake.catalog.is_none() && self.yoake.catalog_task.is_none() && signed_in() {
            self.yoake_refresh();
        }
        let mut page = div().flex().flex_col().gap(px(24.)).child(page_header(
            "Yoake",
            "The group's modpacks. Sign in once; packs install and update from here.",
            &t,
        ));
        if let Some((version, url)) = self.yoake.launcher_update.clone().filter(|_| {
            signed_in()
                || self.yoake.launcher_job.is_some()
                || self.yoake.launcher_ready
                || self.yoake.launcher_error.is_some()
        }) {
            let mut banner = plate(&t).flex().flex_col().gap(px(12.)).p(px(16.)).child(
                text(theme::UI).text_color(t.ink).child(trf(
                    "A new launcher ({version}) is available.",
                    &[("version", &version)],
                )),
            );
            if self.yoake.launcher_job.is_some() {
                banner = banner
                    .child(
                        text(theme::UI)
                            .text_color(t.muted)
                            .child(tr("Downloading launcher update...")),
                    )
                    .child(crate::glass_ui::bar(
                        self.yoake.launcher_progress,
                        self.look.accent_soft,
                    ))
                    .child(
                        button("yoake-launcher-cancel", "Cancel", Kind::Secondary, &t).on_click(
                            cx.listener(|this, _: &ClickEvent, _, cx| {
                                if let Some(job) = &this.yoake.launcher_job {
                                    let _ = job.state.compare_exchange(
                                        0,
                                        1,
                                        Ordering::AcqRel,
                                        Ordering::Acquire,
                                    );
                                }
                                cx.notify();
                            }),
                        ),
                    );
            } else if self.yoake.launcher_ready {
                banner = banner
                    .child(
                        text(theme::UI)
                            .text_color(t.muted)
                            .child(tr("The game keeps running while the launcher restarts.")),
                    )
                    .child(
                        button(
                            "yoake-launcher-restart",
                            "Restart to update",
                            Kind::Primary,
                            &t,
                        )
                        .on_click(cx.listener(
                            |this, _: &ClickEvent, _, cx| {
                                this.yoake.launcher_error = Some(backend::restart());
                                cx.notify();
                            },
                        )),
                    );
            } else {
                banner = banner.child(
                    button(
                        "yoake-launcher-update",
                        "Update launcher",
                        Kind::Primary,
                        &t,
                    )
                    .on_click(cx.listener(|this, _: &ClickEvent, _, cx| {
                        this.begin_launcher_update();
                        cx.notify();
                    })),
                );
            }
            if let Some(error) = &self.yoake.launcher_error {
                banner = banner.child(text(theme::UI).text_color(t.danger).child(error.clone()));
            }
            page = page.child(banner.child(
                button("yoake-launcher-portal", "View on portal", Kind::Ghost, &t).on_click(
                    cx.listener(move |_, _: &ClickEvent, _, cx| {
                        cx.open_url(&url);
                    }),
                ),
            ));
        } else if let Some(error) = &self.yoake.launcher_error {
            let mut banner = plate(&t)
                .flex()
                .flex_col()
                .gap(px(12.))
                .p(px(16.))
                .child(text(theme::UI).text_color(t.danger).child(error.clone()));
            if let Some(url) =
                option_env!("YOAKE_PORTAL_URL").and_then(yoake_launcher_update::portal_page)
            {
                banner = banner.child(
                    button("yoake-launcher-portal", "View on portal", Kind::Ghost, &t).on_click(
                        cx.listener(move |_, _: &ClickEvent, _, cx| {
                            cx.open_url(&url);
                        }),
                    ),
                );
            }
            page = page.child(banner);
        }
        page = page.child(self.yoake_account(cx));
        if let Some((done, file)) = self.yoake.progress.clone() {
            let label = if file.is_empty() {
                tr("Starting…").to_string()
            } else {
                file
            };
            page = page.child(
                plate(&t)
                    .flex()
                    .flex_col()
                    .gap(px(8.))
                    .p(px(16.))
                    .child(
                        div()
                            .flex()
                            .justify_between()
                            .gap(px(12.))
                            .child(text(theme::UI).text_color(t.ink).truncate().child(label))
                            .child(
                                button("yoake-job-cancel", "Cancel", Kind::Secondary, &t).on_click(
                                    cx.listener(|this, _: &ClickEvent, _, cx| {
                                        if let Some((_, task)) = &this.yoake.job {
                                            task.cancel();
                                        }
                                        this.cancel_yoake_base();
                                        cx.notify();
                                    }),
                                ),
                            ),
                    )
                    .child(crate::glass_ui::bar(done, self.look.accent_soft)),
            );
        }
        if let Some((good, note)) = &self.yoake.note {
            page = page.child(
                text(theme::UI)
                    .text_color(if *good { t.ink } else { t.danger })
                    .child(note.clone()),
            );
        }
        page.child(self.yoake_packs(cx))
    }

    /// Who is signed in, or the two ways to sign in.
    fn yoake_account(&mut self, cx: &mut Context<Self>) -> Div {
        let t = self.theme;
        let mut col = div()
            .flex()
            .flex_col()
            .child(section_title("Account", "user", &t).mb(px(12.)));
        if option_env!("YOAKE_PORTAL_URL").is_none() {
            return col.child(index_row(
                0,
                tr("No portal in this build").to_string(),
                None,
                None,
                &t,
            ));
        }
        // Signing in now: the code to check and a way to reopen the page.
        if self.yoake_login_task.is_some() {
            let detail = self
                .yoake_login_event
                .as_ref()
                .map(login_text)
                .unwrap_or_else(|| tr("Contacting the portal…").to_string());
            let code = self.yoake_user_code.clone();
            let mut right = div().flex().gap(px(8.));
            if code.is_some() {
                right = right.child(
                    button("yoake-reopen", "Open the page again", Kind::Secondary, &t).on_click(
                        cx.listener(|this, _: &ClickEvent, _, cx| {
                            if let Some(yoake_login::LoginEvent::VerificationRequired {
                                url, ..
                            }) = &this.yoake_login_event
                            {
                                cx.open_url(url);
                            } else if let Some(code) = &this.yoake_user_code {
                                if let Some(base) = option_env!("YOAKE_PORTAL_URL") {
                                    let url = format!(
                                        "{}/launcher?code={code}",
                                        base.trim_end_matches('/')
                                    );
                                    cx.open_url(&url);
                                }
                            }
                        }),
                    ),
                );
            }
            right = right.child(
                button("yoake-cancel", "Cancel", Kind::Secondary, &t).on_click(cx.listener(
                    |this, _: &ClickEvent, _, cx| {
                        if let Some(task) = &this.yoake_login_task {
                            task.cancel();
                        }
                        cx.notify();
                    },
                )),
            );
            let name = match code {
                Some(code) => trf("Code {code}", &[("code", &code)]),
                None => tr("Signing in…").to_string(),
            };
            return col.child(index_row(
                0,
                name,
                Some(div().child(detail).into_any_element()),
                Some(right.into_any_element()),
                &t,
            ));
        }
        if signed_in() {
            let (name, detail) = match account() {
                Some(a) if a.kind == "friend" => {
                    let until = a
                        .expires_at
                        .as_str()
                        .map(|s| s.get(..10).unwrap_or(s).to_string());
                    (
                        tr("Signed in as a friend").to_string(),
                        match until {
                            Some(d) => trf(
                                "Valid until {date}; it renews while you play.",
                                &[("date", &d)],
                            ),
                            None => String::new(),
                        },
                    )
                }
                _ => (
                    tr("Signed in as a member").to_string(),
                    tr("Your Discord membership keeps this sign-in valid.").to_string(),
                ),
            };
            let logout = button("yoake-logout", "Sign out", Kind::Secondary, &t).on_click(
                cx.listener(|this, _: &ClickEvent, _, cx| {
                    this.yoake_login_event = match yoake_auth::logout(&backend::data_dir()) {
                        Ok(()) => None,
                        Err(_) => Some(yoake_login::LoginEvent::StorageError),
                    };
                    this.yoake.catalog = None;
                    if let Some(job) = &this.yoake.launcher_job {
                        let _ =
                            job.state
                                .compare_exchange(0, 1, Ordering::AcqRel, Ordering::Acquire);
                    } else if !this.yoake.launcher_ready {
                        this.yoake.launcher_update = None;
                    }
                    this.yoake.launcher_update_task = None;
                    this.yoake.launcher_update_checked = false;
                    this.check_yoake_updates();
                    cx.notify();
                }),
            );
            return col.child(index_row(
                0,
                name,
                (!detail.is_empty()).then(|| div().child(detail).into_any_element()),
                Some(logout.into_any_element()),
                &t,
            ));
        }
        // Signed out: members in the browser, friends with a code.
        if let Some(event) = &self.yoake_login_event {
            if !matches!(event, yoake_login::LoginEvent::Success(_)) {
                col = col.child(
                    text(theme::UI)
                        .mb(px(8.))
                        .text_color(t.danger)
                        .child(login_text(event)),
                );
            }
        }
        let member = button("yoake-login", "Sign in with Discord", Kind::Primary, &t).on_click(
            cx.listener(|this, _: &ClickEvent, _, cx| {
                match yoake_login::PortalClient::configured() {
                    Ok(Some(client)) => {
                        this.yoake_login_task = Some(client.start_device_login(&device_label()));
                        this.yoake_login_event = None;
                        this.yoake_user_code = None;
                    }
                    _ => this.yoake_login_event = Some(yoake_login::LoginEvent::InvalidResponse),
                }
                cx.notify();
            }),
        );
        col = col.child(index_row(
            0,
            tr("Members").to_string(),
            Some(
                div()
                    .child(tr(
                        "Opens the portal in your browser; approve this computer there.",
                    ))
                    .into_any_element(),
            ),
            Some(member.into_any_element()),
            &t,
        ));
        let code = self.yoake_field(true, cx);
        let name = self.yoake_field(false, cx);
        let ready = !self.yoake.invite_code.text().trim().is_empty()
            && !self.yoake.invite_name.text().trim().is_empty();
        let join = button("yoake-redeem", "Join with the invite", Kind::Secondary, &t);
        let join = if ready {
            join.on_click(cx.listener(|this, _: &ClickEvent, _, cx| {
                this.yoake_redeem();
                cx.notify();
            }))
        } else {
            disabled(join)
        };
        col.child(index_row(
            0,
            tr("Friends").to_string(),
            Some(
                div()
                    .flex()
                    .flex_col()
                    .gap(px(8.))
                    .mt(px(4.))
                    .child(tr(
                        "Enter the invite code a member gave you, and your name.",
                    ))
                    .child(div().flex().gap(px(8.)).child(code).child(name))
                    .into_any_element(),
            ),
            Some(join.into_any_element()),
            &t,
        ))
    }

    fn yoake_redeem(&mut self) {
        let code = self.yoake.invite_code.text().trim().to_uppercase();
        let name = self.yoake.invite_name.text().trim().to_string();
        if code.is_empty() || name.is_empty() {
            return;
        }
        match yoake_login::PortalClient::configured() {
            Ok(Some(client)) => {
                self.yoake_login_task =
                    Some(client.start_invite_redeem(code, name, &device_label()));
                self.yoake_login_event = None;
                self.yoake_user_code = None;
            }
            _ => self.yoake_login_event = Some(yoake_login::LoginEvent::InvalidResponse),
        }
    }

    /// The invite code (`code`) or the friend's name field.
    fn yoake_field(&self, code: bool, cx: &mut Context<Self>) -> Stateful<Div> {
        let t = self.theme;
        let (focused, focus, edit, hint, id) = if code {
            (
                self.yoake.code_focused,
                &self.yoake.code_focus,
                &self.yoake.invite_code,
                tr("YK-XXXX-XXXX"),
                "yoake-code",
            )
        } else {
            (
                self.yoake.name_focused,
                &self.yoake.name_focus,
                &self.yoake.invite_name,
                tr("Your name"),
                "yoake-name",
            )
        };
        let field = div()
            .id(id)
            .track_focus(focus)
            .flex()
            .items_center()
            .w(px(180.))
            .h(px(30.))
            .px(px(12.))
            .overflow_hidden()
            .border_2()
            .border_color(if focused { t.accent } else { t.outline })
            .bg(t.well)
            .font_family(if code { theme::MONO } else { theme::FONT })
            .text_size(px(12.))
            .cursor_text()
            .child(line_edit::content(edit, &t, focused, false, false, hint))
            .on_click(cx.listener(move |this, _: &ClickEvent, window, cx| {
                let focus = if code {
                    &this.yoake.code_focus
                } else {
                    &this.yoake.name_focus
                };
                window.focus(focus, cx);
                cx.notify();
            }))
            .on_key_down(cx.listener(move |this, ev: &KeyDownEvent, window, cx| {
                let edit = if code {
                    &mut this.yoake.invite_code
                } else {
                    &mut this.yoake.invite_name
                };
                match line_edit::handle(edit, ev, cx) {
                    line_edit::Key::Pass => return,
                    line_edit::Key::Enter => this.yoake_redeem(),
                    line_edit::Key::Escape => window.blur(cx),
                    _ => {}
                }
                cx.stop_propagation();
                cx.notify();
            }));
        if code {
            line_edit::mouse(field, cx, |this| &mut this.yoake.invite_code)
        } else {
            line_edit::mouse(field, cx, |this| &mut this.yoake.invite_name)
        }
    }

    /// The feed's packs: each with its installed instances (update, roll back) or Install.
    fn yoake_packs(&mut self, cx: &mut Context<Self>) -> Div {
        let t = self.theme;
        let refresh = button("yoake-refresh", "Refresh", Kind::Ghost, &t);
        let refresh = if self.yoake.catalog_task.is_some() {
            disabled(refresh)
        } else {
            refresh.on_click(cx.listener(|this, _: &ClickEvent, _, cx| {
                this.yoake.check_launcher_update(true);
                this.yoake_refresh();
                cx.notify();
            }))
        };
        let mut col = div().flex().flex_col().child(
            div()
                .flex()
                .items_center()
                .justify_between()
                .mb(px(12.))
                .child(section_title("Packs", "package", &t))
                .child(refresh),
        );
        if option_env!("YOAKE_COMPILED_FEED_URL").is_none() {
            return col.child(index_row(
                0,
                tr("No pack server in this build").to_string(),
                None,
                None,
                &t,
            ));
        }
        let index = match &self.yoake.catalog {
            _ if self.yoake.catalog_task.is_some() => {
                return col.child(index_row(
                    0,
                    tr("Checking the packs…").to_string(),
                    None,
                    None,
                    &t,
                ));
            }
            None => {
                return col.child(index_row(
                    0,
                    tr("Sign in to see the packs.").to_string(),
                    None,
                    None,
                    &t,
                ));
            }
            Some(Err(e)) => {
                return col.child(index_row(0, feed_text(e), None, None, &t));
            }
            Some(Ok(feed)) => match &feed.index {
                ParsedFeedIndex::V2(index) => index.clone(),
                ParsedFeedIndex::V1(_) => {
                    return col.child(index_row(
                        0,
                        tr("This pack server is too old for this launcher.").to_string(),
                        None,
                        None,
                        &t,
                    ));
                }
            },
        };
        if index.packs.is_empty() {
            return col.child(index_row(
                0,
                tr("No packs yet.").to_string(),
                None,
                None,
                &t,
            ));
        }
        // Installed Yoake instances, by pack id.
        let installed: Vec<(instances::Instance, yoake_install::InstalledPack)> = instances::list()
            .into_iter()
            .filter_map(|i| {
                let pack = yoake_install::read_manifest(&i.home).ok().flatten()?;
                Some((i, pack))
            })
            .collect();
        let busy = self.yoake_busy();
        for (n, pack) in index.packs.iter().enumerate() {
            let Some(latest) = newest(pack) else {
                continue;
            };
            let mut detail = div()
                .flex()
                .flex_col()
                .gap(px(2.))
                .child(trf("Newest {version}", &[("version", &latest.version)]));
            if !latest.description.is_empty() {
                detail = detail.child(latest.description.clone());
            }
            let install = button(
                SharedString::from(format!("yoake-install-{n}")),
                "Install",
                Kind::Primary,
                &t,
            );
            let install = if busy {
                disabled(install)
            } else {
                let (id, name, version) =
                    (pack.id.clone(), pack.name.clone(), latest.version.clone());
                install.on_click(cx.listener(move |this, _: &ClickEvent, _, cx| {
                    this.yoake_install_new(id.clone(), name.clone(), version.clone());
                    cx.notify();
                }))
            };
            let mine: Vec<_> = installed
                .iter()
                .filter(|(_, p)| p.pack_id == pack.id)
                .collect();
            col = col.child(index_row(
                n,
                pack.name.clone(),
                Some(detail.into_any_element()),
                mine.is_empty().then(|| install.into_any_element()),
                &t,
            ));
            for (m, (instance, installed)) in mine.into_iter().enumerate() {
                col =
                    col.child(self.yoake_instance_row(n * 100 + m, pack, instance, installed, cx));
            }
        }
        col
    }

    /// One installed instance of a pack: its version, its update state, Update and Go back.
    fn yoake_instance_row(
        &self,
        n: usize,
        pack: &yoake_feed::PackV2,
        instance: &instances::Instance,
        installed: &yoake_install::InstalledPack,
        cx: &mut Context<Self>,
    ) -> Div {
        let t = self.theme;
        let busy = self.yoake_busy();
        let state = self.yoake_update_states.get(&instance.home);
        let detail = match state {
            _ if self.yoake_update_task.is_some() => tr("Checking for an update…").to_string(),
            Some(yoake_update::UpdateState::UpToDate) => tr("Up to date.").to_string(),
            Some(yoake_update::UpdateState::Available {
                to, download_bytes, ..
            }) => trf(
                "{version} is out ({size} to download).",
                &[("version", to), ("size", &size_text(*download_bytes))],
            ),
            Some(yoake_update::UpdateState::NeedsLogin) => tr("Sign in to check.").to_string(),
            Some(yoake_update::UpdateState::Offline) => {
                tr("Offline; playing the installed version.").to_string()
            }
            Some(yoake_update::UpdateState::Error) | None => String::new(),
        };
        let mut right = div().flex().gap(px(8.));
        if let Some(yoake_update::UpdateState::Available { to, .. }) = state {
            let update = button(
                SharedString::from(format!("yoake-update-{n}")),
                "Update",
                Kind::Primary,
                &t,
            );
            right = right.child(if busy {
                disabled(update)
            } else {
                let (home, id, to) = (instance.home.clone(), pack.id.clone(), to.clone());
                update.on_click(cx.listener(move |this, _: &ClickEvent, _, cx| {
                    this.yoake_update(home.clone(), id.clone(), to.clone());
                    cx.notify();
                }))
            });
        }
        if instance.home.join("yoake-pack.previous.json").is_file() {
            let back = button(
                SharedString::from(format!("yoake-back-{n}")),
                "Go back a version",
                Kind::Ghost,
                &t,
            );
            right = right.child(if busy {
                disabled(back)
            } else {
                let home = instance.home.clone();
                back.on_click(cx.listener(move |this, _: &ClickEvent, _, cx| {
                    this.yoake_rollback(home.clone());
                    cx.notify();
                }))
            });
        }
        let name = trf(
            "{instance} · {version}",
            &[
                ("instance", &instance.name),
                ("version", &installed.pack_version),
            ],
        );
        index_row(
            n,
            name,
            (!detail.is_empty()).then(|| div().child(detail).into_any_element()),
            Some(right.into_any_element()),
            &t,
        )
        .ml(px(24.))
    }
}
