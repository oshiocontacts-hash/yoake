//! Yoake 2.0 full-access mods on the Mods page: one row per package in `<instance>/mods/` with an allow switch,
//! a confirmation before allowing, and the changes since the last accepted launch list.
//!
//! The launcher is the authority (see `horizon_launcher::full_access`): nothing here runs a mod, it only edits the
//! approved list. Reading the list hashes every package, so it happens on a worker thread.

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
}

#[derive(Default)]
pub(super) struct FullAccess {
    snapshot: Snapshot,
    /// A change (or a plain read) running on a worker: the change's error, then the list read after it.
    task: Option<(PathBuf, Receiver<(Option<String>, Snapshot)>)>,
    /// The row the player asked to allow, waiting for the confirmation.
    confirm: Option<FullAccessRow>,
    /// The result of the last approve / revoke / remove / accept, shown under the section title.
    error: Option<String>,
}

fn read(home: &Path) -> Snapshot {
    Snapshot {
        rows: full_access::list(home),
        pending: full_access::pending_changes(home).filter(|diff| !diff.is_empty()),
        control_problem: full_access::control_problem(home),
        launch_problem: full_access::last_launch_problem(home),
    }
}

fn origin_label(origin: &str) -> &'static str {
    match origin {
        "local" => tr("From this PC"),
        "catalog" => tr("From the Yoake catalog"),
        _ => tr("Unknown origin"),
    }
}

fn entry_origin(origin: Origin) -> &'static str {
    match origin {
        Origin::Local => tr("From this PC"),
        Origin::Catalog => tr("From the Yoake catalog"),
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

impl Launcher {
    pub(super) fn load_full_access(&mut self, home: PathBuf) {
        self.full_access_change(home, |_| Ok(()));
    }

    /// Runs one change to the approved list on a worker (approving and reading hash every package), then reads the
    /// list again. A change already running is dropped from view; its successor's read covers both.
    fn full_access_change(
        &mut self,
        home: PathBuf,
        change: impl FnOnce(&Path) -> Result<(), String> + Send + 'static,
    ) {
        let (sender, receiver) = mpsc::channel();
        let selected = home.clone();
        std::thread::spawn(move || {
            let error = change(&home).err();
            let _ = sender.send((error, read(&home)));
        });
        self.yak_mods.full_access.task = Some((selected, receiver));
    }

    fn full_access_busy(&self) -> bool {
        self.yak_mods.full_access.task.is_some()
    }

    /// Takes a finished change or read. Returns whether the page needs a redraw.
    pub(super) fn drain_full_access(&mut self) -> bool {
        let Some((home, receiver)) = self.yak_mods.full_access.task.take() else {
            return false;
        };
        match receiver.try_recv() {
            Ok((error, snapshot)) => {
                if self.yak_mods.home.as_ref() == Some(&home) {
                    // A row waiting for confirmation must still be the same bytes, or the dialog goes away.
                    if let Some(confirm) = &self.yak_mods.full_access.confirm {
                        let same = snapshot
                            .rows
                            .iter()
                            .any(|row| row.id == confirm.id && row.sha256 == confirm.sha256);
                        if !same {
                            self.yak_mods.full_access.confirm = None;
                        }
                    }
                    self.yak_mods.full_access.error = error;
                    self.yak_mods.full_access.snapshot = snapshot;
                }
                true
            }
            Err(mpsc::TryRecvError::Empty) => {
                self.yak_mods.full_access.task = Some((home, receiver));
                false
            }
            Err(mpsc::TryRecvError::Disconnected) => {
                self.yak_mods.full_access.error =
                    Some(tr("The mod operation stopped unexpectedly.").to_owned());
                true
            }
        }
    }

    /// Starts a change for the selected instance.
    fn full_access_act(&mut self, change: impl FnOnce(&Path) -> Result<(), String> + Send + 'static) {
        if self.full_access_busy() {
            return;
        }
        if let Some(home) = self.yak_mods.home.clone() {
            self.full_access_change(home, change);
        }
    }

    pub(super) fn full_access_section(&self, cx: &mut Context<Self>) -> Option<Div> {
        let state = &self.yak_mods.full_access;
        let snapshot = &state.snapshot;
        if snapshot.rows.is_empty()
            && snapshot.pending.is_none()
            && snapshot.control_problem.is_none()
            && snapshot.launch_problem.is_none()
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
                        "These mods can change anything in the game, like a Java mod. Allow only mods you made or got from someone you trust.",
                    )),
                ),
        );
        if let Some(problem) = &snapshot.control_problem {
            section = section.child(shell::text(night::UI, theme.danger).child(trf(
                "The full-access settings failed their check, so no full-access mod will run: {reason}",
                &[("reason", problem)],
            )));
        }
        if let Some(problem) = &snapshot.launch_problem {
            section = section.child(shell::text(night::CAPTION, c(theme.night.warn)).child(trf(
                "Last launch: {reason}",
                &[("reason", problem)],
            )));
        }
        if let Some(error) = &state.error {
            section = section.child(shell::text(night::UI, theme.danger).child(error.clone()));
        }
        if let Some(confirm) = &state.confirm {
            section = section.child(self.full_access_confirm(confirm, cx));
        }
        if let Some(diff) = &snapshot.pending {
            section = section.child(self.full_access_pending(diff, cx));
        }
        for row in &snapshot.rows {
            section = section.child(self.full_access_row(row, cx));
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
        let (state_text, state_color) = match &row.state {
            RowState::Approved if row.passed_at_launch => (tr("Allowed").to_owned(), theme.success),
            RowState::Approved => (
                tr("Allowed · runs after you accept the changes below").to_owned(),
                c(theme.night.warn),
            ),
            RowState::NotApproved => (tr("Not allowed · will not run").to_owned(), theme.muted),
            RowState::ContentChanged => (
                tr("The file changed since you allowed it · will not run until allowed again").to_owned(),
                c(theme.night.warn),
            ),
            RowState::DuplicateId => (
                tr("Two files have this mod id · neither will run").to_owned(),
                theme.danger,
            ),
            RowState::Refused(reason) => (
                trf("Cannot run as full-access: {reason}", &[("reason", reason)]),
                theme.danger,
            ),
        };
        let mut meta = format!("{} · {} · {}", row.version, row.sha256_short, origin_label(&row.origin));
        if let Some(signer) = &row.signer {
            meta.push_str(" · ");
            meta.push_str(&trf("signed by {signer}", &[("signer", signer)]));
        }
        let details = div()
            .flex_1()
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
        let mut controls = div().flex().items_center().gap(px(8.)).child(
            switch(format!("fa-switch-{}", row.id), on, &theme)
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
        let id = row.id.clone();
        controls = controls.child(
            shell::action(
                SharedString::from(format!("fa-remove-{}", row.id)),
                Action::Quiet,
                &theme,
            )
            .child(shell::label(tr("Remove")))
            .when(busy, |button| button.opacity(theme::DISABLED))
            .when(!busy, |button| button.on_click(cx.listener(move |this, _: &ClickEvent, _, cx| {
                let id = id.clone();
                this.yak_mods.full_access.confirm = None;
                this.full_access_act(move |home| full_access::remove(home, &id));
                cx.notify();
            }))),
        );
        div()
            .flex()
            .items_center()
            .gap(px(14.))
            .py(px(6.))
            .child(div().size(px(8.)).bg(state_color))
            .child(details)
            .child(controls)
    }

    fn full_access_confirm(&self, row: &FullAccessRow, cx: &mut Context<Self>) -> Div {
        let theme = self.theme;
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
                "This mod gets the same power as the game itself: it can read and change your worlds, settings and files the game can reach, and talk to the network. Yoake cannot check what it does.",
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
                            .on_click(cx.listener(move |this, _: &ClickEvent, _, cx| {
                                this.yak_mods.full_access.confirm = None;
                                let (id, sha256) = (id.clone(), sha256.clone());
                                this.full_access_act(move |home| {
                                    full_access::approve(home, &id, &sha256)
                                });
                                cx.notify();
                            })),
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

    fn full_access_pending(&self, diff: &ListDiff, cx: &mut Context<Self>) -> Div {
        let theme = self.theme;
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
                    .child(tr("Full-access changes since the last launch")),
            )
            .child(shell::text(night::CAPTION, c(theme.night.muted)).child(tr(
                "Until you accept them, the game keeps running only the full-access mods you accepted before.",
            )));
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
        let shown = diff.clone();
        panel.child(
            div().flex().gap(px(8.)).child(
                shell::action("fa-accept", Action::Main, &theme)
                    .child(shell::label(tr("Accept these changes")))
                    .on_click(cx.listener(move |this, _: &ClickEvent, _, cx| {
                        // Accepts exactly what was shown; if the list moved meanwhile, this fails and the
                        // re-read shows the new changes.
                        let shown = shown.clone();
                        this.full_access_act(move |home| full_access::accept_changes_shown(home, &shown));
                        cx.notify();
                    })),
            ),
        )
    }
}
