//! Yoake 2.0 full-access mods on the Mods page: one row per package in `<instance>/mods/` with an allow switch,
//! a confirmation before allowing, and the changes since the player last accepted the launch list.
//!
//! The launcher is the authority (see `horizon_launcher::full_access`): nothing here runs a mod, it only edits the
//! approved list. Reading the list hashes every package, so reads and changes run on a worker thread, one at a time.

use super::*;
use crate::plate::alpha;
use crate::shell::{c, Action};
use horizon_launcher::full_access::{self, FullAccessRow, RowState};
use horizon_mods::yak::control::{ApprovedEntry, ListDiff, Origin};
use std::path::Path;
use theme::night;

/// What the worker read for one instance.
#[derive(Clone, Default)]
pub(super) struct Snapshot {
    rows: Vec<FullAccessRow>,
    pending: Option<ListDiff>,
    control_problem: Option<String>,
    launch_problem: Option<String>,
    /// Mods fetched from hosts when joining (MX3JN2), by host.
    remote: Vec<full_access::RemoteHost>,
}

/// A finished job: `Some(result)` for a change (its error, if any), `None` for a plain read; then the list after it.
type Done = (Option<Option<String>>, Snapshot);

#[derive(Default)]
pub(super) struct FullAccess {
    snapshot: Snapshot,
    task: Option<(PathBuf, Receiver<Done>)>,
    /// A read was asked for while a job ran: start it when that job is done.
    reload: bool,
    /// The row the player asked to allow, waiting for the confirmation.
    confirm: Option<FullAccessRow>,
    /// The result of the last approve / revoke / remove / accept. A plain read keeps it.
    error: Option<String>,
    /// The dev folder's generation when the rows were last read (a change means the watcher installed something).
    dev_generation: u64,
}

fn read(home: &Path) -> Snapshot {
    Snapshot {
        rows: full_access::list(home),
        pending: full_access::pending_changes(home).filter(|diff| !diff.is_empty()),
        control_problem: full_access::control_problem(home),
        launch_problem: full_access::last_launch_problem(home),
        remote: full_access::remote_hosts(home),
    }
}

fn origin_label(origin: &str) -> &'static str {
    match origin {
        "local" => tr("From this PC"),
        "catalog" => tr("From the Yoake catalog"),
        "dev" => tr("From your dev folder"),
        _ => tr("Unknown origin"),
    }
}

fn entry_origin(origin: Origin) -> &'static str {
    match origin {
        Origin::Local => tr("From this PC"),
        Origin::Catalog => tr("From the Yoake catalog"),
        Origin::Dev => tr("From your dev folder"),
        _ => tr("Unknown origin"),
    }
}

fn short(sha256: &str) -> &str {
    &sha256[..sha256.len().min(12)]
}

fn entry_line(entry: &ApprovedEntry) -> String {
    format!(
        "{} {} · {} · {}",
        entry.id,
        entry.version,
        short(&entry.sha256),
        entry_origin(entry.origin)
    )
}

/// The player-facing text for a backend reason (the common ones are translated; the rest are shown as they are).
fn reason_text(reason: &str) -> String {
    if reason.starts_with("the launcher has no install record") {
        tr("This file was put into the mods folder by hand, so Yoake does not know where it came from. Remove it, then add it again with Add a .yak file….").to_owned()
    } else if reason.starts_with("not a valid .yak") {
        tr("This is not a valid .yak file.").to_owned()
    } else if reason.starts_with("a package from a ") {
        tr("A mod delivered by a server or a friend cannot be full-access.").to_owned()
    } else if reason.starts_with("the pending changes are not what you were shown") {
        tr("The changes moved while you were looking. Check them again.").to_owned()
    } else if reason.contains("changed while approving") {
        tr("The file changed while you were allowing it. Check it again.").to_owned()
    } else if reason.contains("is in the mods folder more than once") {
        tr("This mod id is in the mods folder more than once. Remove the extra file first.").to_owned()
    } else {
        reason.to_owned()
    }
}

impl Launcher {
    pub(super) fn load_full_access(&mut self, home: PathBuf) {
        if self.full_access_busy() {
            // Never replace a running job (its result would be lost, and a read racing a change shows stale rows).
            self.yak_mods.full_access.reload = true;
            return;
        }
        self.full_access_spawn(home, None::<fn(&Path) -> Result<(), String>>);
    }

    /// Reads the list again for the selected Yoake instance: the Mods page opened, a game started or ended.
    pub(crate) fn reload_full_access(&mut self) {
        if let Some(home) = self.yak_mods.home.clone() {
            self.load_full_access(home);
        }
    }

    fn full_access_spawn(
        &mut self,
        home: PathBuf,
        change: Option<impl FnOnce(&Path) -> Result<(), String> + Send + 'static>,
    ) {
        let (sender, receiver) = mpsc::channel();
        let selected = home.clone();
        std::thread::spawn(move || {
            let result = change.map(|change| change(&home).err());
            let _ = sender.send((result, read(&home)));
        });
        self.yak_mods.full_access.reload = false;
        self.yak_mods.full_access.task = Some((selected, receiver));
    }

    fn full_access_busy(&self) -> bool {
        self.yak_mods.full_access.task.is_some()
    }

    /// Takes a finished job. Returns whether the page needs a redraw.
    pub(super) fn drain_full_access(&mut self) -> bool {
        // The dev-folder watcher installed or removed something: read the rows again.
        if let Some(home) = self.yak_mods.home.clone() {
            let generation = self.dev_status(&home).map_or(0, |status| status.generation);
            if generation != self.yak_mods.full_access.dev_generation {
                self.yak_mods.full_access.dev_generation = generation;
                self.load_full_access(home);
            }
        }
        let Some((home, receiver)) = self.yak_mods.full_access.task.take() else {
            return false;
        };
        let current = self.yak_mods.home.as_ref() == Some(&home);
        match receiver.try_recv() {
            Ok((result, snapshot)) => {
                if current {
                    // A row waiting for confirmation must still be the same bytes, or the dialog goes away.
                    if let Some(confirm) = &self.yak_mods.full_access.confirm {
                        let same = snapshot
                            .rows
                            .iter()
                            .any(|row| row.file == confirm.file && row.sha256 == confirm.sha256);
                        if !same {
                            self.yak_mods.full_access.confirm = None;
                        }
                    }
                    if let Some(error) = result {
                        self.yak_mods.full_access.error = error.as_deref().map(reason_text);
                    }
                    self.yak_mods.full_access.snapshot = snapshot;
                }
            }
            Err(mpsc::TryRecvError::Empty) => {
                self.yak_mods.full_access.task = Some((home, receiver));
                return false;
            }
            Err(mpsc::TryRecvError::Disconnected) => {
                if current {
                    self.yak_mods.full_access.error =
                        Some(tr("The mod operation stopped unexpectedly.").to_owned());
                    self.yak_mods.full_access.reload = true;
                }
            }
        }
        if std::mem::take(&mut self.yak_mods.full_access.reload) {
            self.reload_full_access();
        }
        true
    }

    /// Starts a change for the selected instance. Does nothing while a job runs (the buttons are disabled then).
    fn full_access_act(&mut self, change: impl FnOnce(&Path) -> Result<(), String> + Send + 'static) {
        if self.full_access_busy() {
            return;
        }
        if let Some(home) = self.yak_mods.home.clone() {
            self.full_access_spawn(home, Some(change));
        }
    }

    fn dev_status(&self, home: &Path) -> Option<full_access::DevStatus> {
        full_access::status_for(home, full_access::active_dev_folder(&self.settings))
    }

    /// The ids of the full-access packages in the selected instance's `mods/` folder.
    pub(super) fn full_access_ids(&self) -> impl Iterator<Item = &str> {
        self.yak_mods.full_access.snapshot.rows.iter().map(|row| row.id.as_str())
    }

    pub(super) fn full_access_section(&self, cx: &mut Context<Self>) -> Option<Div> {
        let state = &self.yak_mods.full_access;
        let snapshot = &state.snapshot;
        let dev = self.yak_mods.home.as_deref().and_then(|home| self.dev_status(home));
        let sync = self.yak_mods.home.as_deref().and_then(full_access::last_join_sync_for);
        if snapshot.rows.is_empty()
            && snapshot.pending.is_none()
            && snapshot.control_problem.is_none()
            && snapshot.launch_problem.is_none()
            && dev.is_none()
            && snapshot.remote.is_empty()
            && sync.is_none()
            && self.join_again_host().is_none()
        {
            return None;
        }
        let theme = self.theme;
        let mut section = div().flex().flex_col().gap(px(10.)).child(
            div()
                .flex()
                .flex_col()
                .gap(px(4.))
                .child(
                    shell::text(night::UI, c(theme.night.ink))
                        .font_weight(FontWeight::SEMIBOLD)
                        .child(tr("Full-access mods")),
                )
                .child(
                    shell::text(night::CAPTION, c(theme.night.muted)).child(tr(
                        "These mods get the game's own power, like a Java mod. Allow only mods you made or got from someone you trust.",
                    )),
                ),
        );
        if let Some(dev) = &dev {
            let line = if dev.watching {
                trf("Dev folder: watching {path}", &[("path", &dev.folder.display().to_string())])
            } else {
                trf(
                    "Dev folder: {path} (watched while the game runs)",
                    &[("path", &dev.folder.display().to_string())],
                )
            };
            section = section.child(shell::text(night::CAPTION, c(theme.night.muted)).child(line));
            if let Some(event) = &dev.last_event {
                section = section.child(
                    shell::text(night::CAPTION, c(theme.night.muted))
                        .child(trf("Last change: {event}", &[("event", event)])),
                );
            }
            if let Some(error) = &dev.last_error {
                section = section.child(
                    shell::text(night::CAPTION, c(theme.night.warn))
                        .child(trf("Dev folder problem: {reason}", &[("reason", error)])),
                );
            }
        }
        if let Some(problem) = &snapshot.control_problem {
            section = section.child(shell::text(night::UI, theme.danger).child(trf(
                "The full-access settings failed their check, so no full-access mod will run: {reason}",
                &[("reason", problem)],
            )));
        }
        if let Some(problem) = &snapshot.launch_problem {
            if snapshot.control_problem.as_ref() != Some(problem) {
                section = section.child(shell::text(night::CAPTION, c(theme.night.warn)).child(
                    trf("Last launch: {reason}", &[("reason", &reason_text(problem))]),
                ));
            }
        }
        if let Some(error) = &state.error {
            section = section.child(shell::text(night::UI, theme.danger).child(error.clone()));
        }
        if self.full_access_busy() {
            section = section.child(
                shell::text(night::CAPTION, c(theme.night.muted))
                    .child(tr("Working on Yoake mods…")),
            );
        }
        if let Some(confirm) = &state.confirm {
            section = section.child(self.full_access_confirm(confirm, cx));
        }
        if snapshot.pending.is_some() || snapshot.control_problem.is_some() {
            section = section.child(self.full_access_pending(
                snapshot.pending.clone().unwrap_or_default(),
                snapshot.control_problem.is_some(),
                cx,
            ));
        }
        for row in &snapshot.rows {
            section = section.child(self.full_access_row(row, cx));
        }
        if let Some(sync) = &sync {
            section = section.child(match &sync.error {
                None => shell::text(night::CAPTION, c(theme.night.muted)).child(trf(
                    "Joining {host}: {count} mod(s) fetched and installed for that host.",
                    &[("host", &sync.host_display), ("count", &sync.installed.to_string())],
                )),
                Some(error) => shell::text(night::CAPTION, c(theme.night.warn)).child(trf(
                    "Joining {host}: the host's mods could not be installed: {reason}",
                    &[("host", &sync.host_display), ("reason", error)],
                )),
            });
        }
        if self.join_again_host().is_some() {
            section = section.child(
                div()
                    .flex()
                    .items_center()
                    .gap(px(8.))
                    .child(shell::text(night::CAPTION, c(theme.night.muted)).child(tr(
                        "The host's mods are installed. Join again when the host's room shows up.",
                    )))
                    .child(
                        shell::action("fa-join-again", Action::Main, &theme)
                            .child(shell::label(tr("Join again")))
                            .on_click(cx.listener(|this, _: &ClickEvent, _, cx| {
                                this.join_again();
                                cx.notify();
                            })),
                    ),
            );
        }
        for host in &snapshot.remote {
            section = section.child(self.remote_host_row(host, cx));
        }
        Some(
            plate::cut(
                night::PIXEL_CORNER,
                2,
                Some(alpha(theme.night.warn, 0.06)),
                Some((alpha(theme.night.warn, 0.5), 1.)),
            )
            .flex()
            .flex_col()
            .p(px(14.))
            .child(section),
        )
    }

    fn full_access_row(&self, row: &FullAccessRow, cx: &mut Context<Self>) -> Div {
        let theme = self.theme;
        let control_problem = self.yak_mods.full_access.snapshot.control_problem.as_ref();
        let (state_text, state_color) = match &row.state {
            RowState::Approved if row.passed_at_launch => (tr("Allowed").to_owned(), theme.success),
            RowState::Approved => (
                tr("Allowed · runs after you accept the changes above").to_owned(),
                c(theme.night.warn),
            ),
            RowState::NotApproved => (tr("Not allowed · will not run").to_owned(), theme.muted),
            RowState::ContentChanged => (
                tr("The file changed since you allowed it · will not run until allowed again").to_owned(),
                c(theme.night.warn),
            ),
            RowState::DuplicateId => (
                tr("Two files have this mod id · neither will run. Remove the one you do not want.").to_owned(),
                theme.danger,
            ),
            RowState::Refused(reason) if control_problem == Some(reason) => (
                tr("Waits for the settings check above").to_owned(),
                theme.danger,
            ),
            RowState::Refused(reason) => (
                trf("Cannot run as full-access: {reason}", &[("reason", &reason_text(reason))]),
                theme.danger,
            ),
        };
        let mut meta = format!(
            "{} · {} · {} · {}",
            row.version,
            row.file,
            row.sha256_short,
            origin_label(&row.origin)
        );
        if let Some(signer) = &row.signer {
            meta.push_str(" · ");
            meta.push_str(&trf("signed by {signer}", &[("signer", signer)]));
        }
        let details = div()
            .flex_1()
            .min_w_0()
            .flex()
            .flex_col()
            .gap(px(4.))
            .child(
                div()
                    .flex()
                    .items_center()
                    .gap(px(8.))
                    .child(shell::text(night::UI, c(theme.night.ink)).child(row.name.clone()))
                    .child(
                        div()
                            .px(px(6.))
                            .py(px(1.))
                            .bg(alpha(theme.night.warn, 0.18))
                            .child(
                                shell::text(night::CAPTION, c(theme.night.warn))
                                    .font_weight(FontWeight::SEMIBOLD)
                                    .child(tr("Full access")),
                            ),
                    ),
            )
            .child(shell::text(night::CAPTION, c(theme.night.muted)).child(meta))
            .child(shell::text(night::CAPTION, state_color).child(state_text));
        let busy = self.full_access_busy();
        let can_switch = !busy
            && !matches!(row.state, RowState::DuplicateId | RowState::Refused(_))
            && self.yak_mods.full_access.confirm.is_none();
        let on = row.approved;
        let switch_row = row.clone();
        let mut controls = div().flex().flex_none().items_center().gap(px(8.)).child(
            switch(format!("fa-switch-{}", row.file), on, &theme)
                .when(can_switch, |button| {
                    button.on_click(cx.listener(move |this, _: &ClickEvent, _, cx| {
                        if on {
                            let id = switch_row.id.clone();
                            this.full_access_act(move |home| full_access::revoke(home, &id));
                        } else {
                            // Allowing hands the mod the game's own power: ask first.
                            this.yak_mods.full_access.error = None;
                            this.yak_mods.full_access.confirm = Some(switch_row.clone());
                        }
                        cx.notify();
                    }))
                })
                .when(!can_switch, |button| button.opacity(theme::DISABLED)),
        );
        let file = row.file.clone();
        controls = controls.child(
            shell::action(
                SharedString::from(format!("fa-remove-{}", row.file)),
                Action::Quiet,
                &theme,
            )
            .child(shell::label(tr("Remove")))
            .when(busy, |button| button.opacity(theme::DISABLED))
            .when(!busy, |button| {
                button.on_click(cx.listener(move |this, _: &ClickEvent, _, cx| {
                    // Only this file: with a duplicate id, the other copy stays.
                    let file = file.clone();
                    this.yak_mods.full_access.confirm = None;
                    this.full_access_act(move |home| full_access::remove_file(home, &file));
                    cx.notify();
                }))
            }),
        );
        div()
            .flex()
            .items_center()
            .gap(px(14.))
            .py(px(6.))
            .child(div().flex_none().size(px(8.)).bg(state_color))
            .child(details)
            .child(controls)
    }

    /// Mods one host gave this instance when joining: they run only when joining that host.
    fn remote_host_row(&self, host: &full_access::RemoteHost, cx: &mut Context<Self>) -> Div {
        let theme = self.theme;
        let busy = self.full_access_busy();
        let mut details = div()
            .flex_1()
            .min_w_0()
            .flex()
            .flex_col()
            .gap(px(4.))
            .child(
                shell::text(night::UI, c(theme.night.ink))
                    .child(trf("From {host}", &[("host", &host.display)])),
            )
            .child(shell::text(night::CAPTION, c(theme.night.muted)).child(tr(
                "Runs only when you join this host.",
            )));
        for m in &host.mods {
            let mut line = format!("{} {} · {}", m.id, m.version, m.sha256_short);
            if m.full_access {
                line.push_str(" · ");
                line.push_str(tr("Full access"));
            }
            details = details.child(shell::text(night::CAPTION, c(theme.night.muted)).child(line));
        }
        let key = host.key.clone();
        div()
            .flex()
            .items_center()
            .gap(px(14.))
            .py(px(6.))
            .child(div().flex_none().size(px(8.)).bg(theme.accent))
            .child(details)
            .child(
                shell::action(
                    SharedString::from(format!("fa-remote-remove-{}", host.key)),
                    Action::Quiet,
                    &theme,
                )
                .child(shell::label(tr("Remove all from this host")))
                .when(busy, |button| button.opacity(theme::DISABLED))
                .when(!busy, |button| {
                    button.on_click(cx.listener(move |this, _: &ClickEvent, _, cx| {
                        let key = key.clone();
                        this.full_access_act(move |home| full_access::remove_remote_host(home, &key));
                        cx.notify();
                    }))
                }),
            )
    }

    fn full_access_confirm(&self, row: &FullAccessRow, cx: &mut Context<Self>) -> Div {
        let theme = self.theme;
        let busy = self.full_access_busy();
        let id = row.id.clone();
        let sha256 = row.sha256.clone();
        div()
            .flex()
            .flex_col()
            .gap(px(8.))
            .p(px(12.))
            .border_1()
            .border_color(c(theme.night.warn))
            .child(
                shell::text(night::UI, c(theme.night.ink))
                    .font_weight(FontWeight::SEMIBOLD)
                    .child(trf("Allow {name}?", &[("name", &row.name)])),
            )
            .child(shell::text(night::UI, c(theme.night.warn)).child(tr(
                "This mod gets the same power as the game itself: anything the game can do, including changing your worlds and settings, reading and writing files on this PC, and using the network. Yoake cannot check what it does.",
            )))
            .child(shell::text(night::CAPTION, c(theme.night.muted)).child(trf(
                "Only this exact file is allowed ({version} · {hash} · {origin}). If it changes, you will be asked again.",
                &[
                    ("version", &row.version),
                    ("hash", &row.sha256_short),
                    ("origin", origin_label(&row.origin)),
                ],
            )))
            .child(
                div()
                    .flex()
                    .gap(px(8.))
                    .child(
                        shell::action("fa-confirm", Action::Main, &theme)
                            .child(shell::label(tr("Allow")))
                            .when(busy, |button| button.opacity(theme::DISABLED))
                            .when(!busy, |button| {
                                button.on_click(cx.listener(move |this, _: &ClickEvent, _, cx| {
                                    this.yak_mods.full_access.confirm = None;
                                    let (id, sha256) = (id.clone(), sha256.clone());
                                    // The full hash of the bytes shown: approve refuses if the file is not those
                                    // bytes any more.
                                    this.full_access_act(move |home| {
                                        full_access::approve(home, &id, &sha256)
                                    });
                                    cx.notify();
                                }))
                            }),
                    )
                    .child(
                        shell::action("fa-cancel", Action::Quiet, &theme)
                            .child(shell::label(tr("Cancel")))
                            .on_click(cx.listener(|this, _: &ClickEvent, _, cx| {
                                this.yak_mods.full_access.confirm = None;
                                cx.notify();
                            })),
                    ),
            )
    }

    fn full_access_pending(&self, diff: ListDiff, control_problem: bool, cx: &mut Context<Self>) -> Div {
        let theme = self.theme;
        let busy = self.full_access_busy();
        let mut panel = div()
            .flex()
            .flex_col()
            .gap(px(6.))
            .p(px(12.))
            .border_1()
            .border_color(c(theme.night.warn))
            .child(
                shell::text(night::UI, c(theme.night.ink))
                    .font_weight(FontWeight::SEMIBOLD)
                    .child(tr("Full-access changes since you last accepted")),
            )
            .child(shell::text(night::CAPTION, c(theme.night.muted)).child(tr(
                "Until you accept them, the game keeps running only the full-access mods you accepted before.",
            )));
        if !diff.added.is_empty() || !diff.changed.is_empty() {
            panel = panel.child(shell::text(night::CAPTION, c(theme.night.warn)).child(tr(
                "These were not allowed for this instance on this page (for example, allowed for another instance, or the settings were changed outside Yoake). Each added mod gets the same power as the game itself. Accept only if you recognize every one.",
            )));
        }
        if control_problem {
            panel = panel.child(shell::text(night::CAPTION, c(theme.night.warn)).child(tr(
                "Accepting also trusts the full-access settings files as they are now. Do it only if you changed them yourself or after checking the list below.",
            )));
        }
        if diff.is_empty() {
            panel = panel.child(
                shell::text(night::CAPTION, c(theme.night.muted))
                    .child(tr("No mod changes. Only the settings files changed.")),
            );
        }
        for entry in &diff.added {
            panel = panel.child(
                shell::text(night::CAPTION, c(theme.night.ink))
                    .child(trf("Added: {mod}", &[("mod", &entry_line(entry))])),
            );
        }
        for (before, after) in &diff.changed {
            panel = panel.child(shell::text(night::CAPTION, c(theme.night.warn)).child(trf(
                "Changed: {before} → {after}",
                &[("before", &entry_line(before)), ("after", &entry_line(after))],
            )));
        }
        for entry in &diff.removed {
            panel = panel.child(
                shell::text(night::CAPTION, c(theme.night.muted))
                    .child(trf("Removed: {mod}", &[("mod", &entry_line(entry))])),
            );
        }
        panel.child(
            div().flex().gap(px(8.)).child(
                shell::action("fa-accept", Action::Main, &theme)
                    .child(shell::label(tr("Accept these changes")))
                    .when(busy, |button| button.opacity(theme::DISABLED))
                    .when(!busy, |button| {
                        button.on_click(cx.listener(move |this, _: &ClickEvent, _, cx| {
                            // Accepts exactly what was shown; if the list moved meanwhile, this fails and the
                            // read after it shows the new changes.
                            let shown = diff.clone();
                            this.full_access_act(move |home| {
                                full_access::accept_changes_shown(home, &shown)
                            });
                            cx.notify();
                        }))
                    }),
            ),
        )
    }
}
