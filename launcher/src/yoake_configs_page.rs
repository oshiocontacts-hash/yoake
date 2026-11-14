//! Yoake mods and configurations on the Yoake page: every Yoake mod (built-in and `.yak`), a
//! builder for your own configuration ("構成"), `YK1-` share codes, and adding one by code.
//! The logic (catalog merge, codes, resolution) lives in `yoake_configs` and is tested there.
use super::*;
use horizon_launcher::yoake_configs::{
    self, CatalogMod, Configuration, Kind as ModKind, ModRef, Preview, Problem,
};
use horizon_launcher::yoake_mods::Client;
use std::collections::{BTreeMap, BTreeSet};
use std::path::PathBuf;
use std::sync::mpsc::{self, Receiver};
use std::time::{Duration, Instant};

/// One pack of the feed as a starting point: its newest build and the built-in mods that build has.
#[derive(Clone)]
pub(super) struct BaseInfo {
    pub pack_id: String,
    pub name: String,
    pub version: String,
    pub native_mods: Vec<String>,
}

type Fetched = (Vec<CatalogMod>, Option<String>);

#[derive(Clone, Copy, PartialEq, Eq)]
enum Field {
    Name,
    Code,
}

pub(super) struct State {
    catalog: Vec<CatalogMod>,
    task: Option<Receiver<Result<Fetched, String>>>,
    last_refresh: Option<Instant>,
    epoch: u64,
    /// Why the `.yak` half of the catalog is missing (the built-in half may still be there).
    notice: Option<String>,
    error: Option<String>,
    picked: BTreeSet<String>,
    selected: BTreeSet<String>,
    auto_added: BTreeSet<String>,
    base: Option<String>,
    saved: Vec<Configuration>,
    /// The share code on show, and the name of the configuration it belongs to.
    share: Option<(String, String)>,
    /// A pasted code that was read: the configuration and how it matches the catalog.
    preview: Option<(Configuration, Preview)>,
    note: Option<(bool, String)>,
    /// A new instance whose `.yak` mods go in once its base pack has finished installing.
    pending: Option<(PathBuf, Vec<(String, String)>)>,
    yak_task: Option<Receiver<Result<(), String>>>,
    name: line_edit::LineEdit,
    code: line_edit::LineEdit,
    name_focus: FocusHandle,
    code_focus: FocusHandle,
    name_focused: bool,
    code_focused: bool,
}

impl State {
    pub(super) fn new(cx: &mut App) -> Self {
        Self {
            catalog: Vec::new(),
            task: None,
            last_refresh: None,
            epoch: 0,
            notice: None,
            error: None,
            picked: BTreeSet::new(),
            selected: BTreeSet::new(),
            auto_added: BTreeSet::new(),
            base: None,
            saved: yoake_configs::load(&backend::data_dir()),
            share: None,
            preview: None,
            note: None,
            pending: None,
            yak_task: None,
            name: line_edit::LineEdit::new(String::new(), 48, 22),
            code: line_edit::LineEdit::new(String::new(), 2048, 22),
            name_focus: cx.focus_handle(),
            code_focus: cx.focus_handle(),
            name_focused: false,
            code_focused: false,
        }
    }
}

fn problem_text(problem: &Problem) -> String {
    match problem {
        Problem::UnknownMod(id) => trf("{id} is not in the Yoake mod catalog.", &[("id", id)]),
        Problem::UnknownVersion { id, version } => trf(
            "{id} has no version {version} in the catalog.",
            &[("id", id), ("version", version)],
        ),
        Problem::UnknownBase(base) => trf(
            "The base pack {base} is not available. Sign in and refresh first.",
            &[("base", base)],
        ),
        Problem::NotInBuild { id, base } => trf(
            "{id} is not part of the {base} build.",
            &[("id", id), ("base", base)],
        ),
        Problem::SettingsNotShareable(id) => {
            trf("{id} has no settings that can be shared.", &[("id", id)])
        }
    }
}

fn code_error(error: &yoake_configs::Error) -> String {
    use yoake_configs::Error as E;
    match error {
        E::NotACode => tr("This is not a Yoake configuration code.").to_string(),
        E::ServerCodeUnsupported => tr("Short codes (YK2-) are not supported yet.").to_string(),
        E::BadEncoding => tr("The code has characters that are not allowed.").to_string(),
        E::Checksum => tr("The code is damaged: copy it again.").to_string(),
        E::TooLarge => tr("The code is too large.").to_string(),
        E::NeedsUpdate => tr("This code needs a newer Yoake launcher.").to_string(),
        E::Invalid(reason) => trf("The code is not valid: {reason}", &[("reason", reason)]),
        E::Malformed | E::BadSignature => {
            tr("The code does not describe a configuration.").to_string()
        }
    }
}

impl Launcher {
    fn bases_by_pack(&self) -> BTreeMap<String, Vec<String>> {
        self.yoake_bases()
            .into_iter()
            .map(|base| (base.pack_id, base.native_mods))
            .collect()
    }

    fn refresh_yoake_catalog(&mut self) {
        if self.yoake_configs.task.is_some() {
            return;
        }
        self.yoake_configs.last_refresh = Some(Instant::now());
        let client = match Client::configured() {
            Ok(client) => client,
            Err(error) => {
                self.yoake_configs.error = Some(error.to_string());
                return;
            }
        };
        let (sender, receiver) = mpsc::channel();
        std::thread::spawn(move || {
            let builtin = client.refresh_builtin();
            let yak = client.refresh();
            let result = match (builtin, yak) {
                (Err(error), _) => Err(error.to_string()),
                (Ok(builtin), Ok(yak)) => {
                    Ok((yoake_configs::merge(builtin.as_ref(), Some(&yak.catalog)), None))
                }
                // The built-in list does not need the .yak catalog: show it, say what is missing.
                (Ok(builtin), Err(error)) => Ok((
                    yoake_configs::merge(builtin.as_ref(), None),
                    Some(error.to_string()),
                )),
            };
            let _ = sender.send(result);
        });
        self.yoake_configs.task = Some(receiver);
    }

    /// Called every frame the launcher runs: catalog refresh results and the pending `.yak` set.
    pub(super) fn drain_yoake_configs(&mut self) -> bool {
        let mut changed = false;
        let epoch = yoake_auth::credential_epoch();
        if self.yoake_configs.epoch != epoch {
            self.yoake_configs.epoch = epoch;
            self.yoake_configs.last_refresh = None;
            changed = true;
        }
        if yoake_page::enabled()
            && self.yoake_configs.task.is_none()
            && self
                .yoake_configs
                .last_refresh
                .is_none_or(|time| time.elapsed() >= Duration::from_secs(300))
        {
            self.refresh_yoake_catalog();
        }
        if let Some(receiver) = self.yoake_configs.task.take() {
            match receiver.try_recv() {
                Ok(Ok((catalog, notice))) => {
                    self.yoake_configs.catalog = catalog;
                    self.yoake_configs.notice = notice;
                    self.yoake_configs.error = None;
                    changed = true;
                }
                Ok(Err(error)) => {
                    self.yoake_configs.error = Some(error);
                    changed = true;
                }
                Err(mpsc::TryRecvError::Empty) => self.yoake_configs.task = Some(receiver),
                Err(mpsc::TryRecvError::Disconnected) => {
                    self.yoake_configs.error =
                        Some(tr("The mod operation stopped unexpectedly.").to_string());
                    changed = true;
                }
            }
        }
        if let Some((home, set)) = self.yoake_configs.pending.clone() {
            if !self.yoake_busy() && self.yoake_configs.yak_task.is_none() {
                self.yoake_configs.pending = None;
                if set.is_empty() {
                    self.yoake_configs.note =
                        Some((true, tr("The new instance is ready.").to_string()));
                } else if yoake_install::read_manifest(&home).ok().flatten().is_some() {
                    let (sender, receiver) = mpsc::channel();
                    std::thread::spawn(move || {
                        let _ = sender.send(
                            horizon_launcher::yoake_mods::install_set(&home, &set)
                                .map_err(|error| error.to_string()),
                        );
                    });
                    self.yoake_configs.yak_task = Some(receiver);
                } else {
                    self.yoake_configs.note = Some((
                        false,
                        tr("The base pack did not install, so the mods were not added.").to_string(),
                    ));
                }
                changed = true;
            }
        }
        if let Some(receiver) = self.yoake_configs.yak_task.take() {
            match receiver.try_recv() {
                Ok(result) => {
                    self.yoake_configs.note = Some(match result {
                        Ok(()) => (true, tr("The new instance is ready.").to_string()),
                        Err(error) => (
                            false,
                            trf("Yoake mods: {reason}", &[("reason", &error)]),
                        ),
                    });
                    changed = true;
                }
                Err(mpsc::TryRecvError::Empty) => self.yoake_configs.yak_task = Some(receiver),
                Err(mpsc::TryRecvError::Disconnected) => {
                    self.yoake_configs.note =
                        Some((false, tr("The mod operation stopped unexpectedly.").to_string()));
                    changed = true;
                }
            }
        }
        changed
    }

    /// Adds or removes a hand-picked mod; what it requires follows (and stays while something
    /// else still needs it).
    fn toggle_config_mod(&mut self, id: &str) {
        let state = &mut self.yoake_configs;
        if !state.picked.remove(id) {
            state.picked.insert(id.to_owned());
        }
        let (all, added) = yoake_configs::with_requirements(&state.picked, &state.catalog);
        state.selected = all;
        state.auto_added = added;
        state.share = None;
    }

    fn draft(&self) -> Result<Configuration, String> {
        let state = &self.yoake_configs;
        let base = state
            .base
            .clone()
            .or_else(|| self.yoake_bases().first().map(|b| b.pack_id.clone()))
            .ok_or_else(|| tr("Sign in and pick a base pack first.").to_string())?;
        let configuration = Configuration {
            name: state.name.text().trim().to_owned(),
            base,
            mods: state
                .selected
                .iter()
                .map(|id| ModRef {
                    id: id.clone(),
                    version: yoake_configs::LATEST.to_owned(),
                })
                .collect(),
            settings: BTreeMap::new(),
        };
        configuration
            .validate()
            .map_err(|error| code_error(&error))
            .and_then(|()| {
                if configuration.mods.is_empty() {
                    Err(tr("Pick at least one mod.").to_string())
                } else {
                    Ok(configuration)
                }
            })
    }

    fn save_draft(&mut self) {
        match self.draft().and_then(|configuration| {
            yoake_configs::save(&backend::data_dir(), configuration).map_err(|e| e)
        }) {
            Ok(()) => {
                self.yoake_configs.saved = yoake_configs::load(&backend::data_dir());
                self.yoake_configs.note = Some((true, tr("Configuration saved.").to_string()));
            }
            Err(error) => self.yoake_configs.note = Some((false, error)),
        }
    }

    fn share_configuration(&mut self, configuration: &Configuration) {
        match yoake_configs::encode(configuration) {
            Ok(code) => {
                self.yoake_configs.share = Some((configuration.name.clone(), code));
                self.yoake_configs.note = None;
            }
            Err(error) => self.yoake_configs.note = Some((false, code_error(&error))),
        }
    }

    fn read_code(&mut self) {
        let text = self.yoake_configs.code.text().to_owned();
        match yoake_configs::decode(&text) {
            Ok(configuration) => {
                let preview = yoake_configs::resolve(
                    &configuration,
                    &self.yoake_configs.catalog,
                    &self.bases_by_pack(),
                );
                self.yoake_configs.preview = Some((configuration, preview));
                self.yoake_configs.note = None;
            }
            Err(error) => {
                self.yoake_configs.preview = None;
                self.yoake_configs.note = Some((false, code_error(&error)));
            }
        }
    }

    /// A new instance from a configuration that matches the catalog: the base pack first, then
    /// the `.yak` mods once it is in place. Built-in mods come with the base build.
    fn install_configuration(&mut self, preview: &Preview) {
        if !preview.installable() || self.yoake_busy() {
            return;
        }
        let Some(base) = self
            .yoake_bases()
            .into_iter()
            .find(|base| base.pack_id == preview.base)
        else {
            return;
        };
        if let Some(home) = self.yoake_install_new(base.pack_id, preview.name.clone(), base.version)
        {
            self.yoake_configs.pending = Some((home, preview.yak_set()));
            self.yoake_configs.note = None;
        }
    }

    fn config_field(&self, field: Field, cx: &mut Context<Self>) -> Stateful<Div> {
        let t = self.theme;
        let state = &self.yoake_configs;
        let (focused, focus, edit, hint, id, width) = match field {
            Field::Name => (
                state.name_focused,
                &state.name_focus,
                &state.name,
                tr("Configuration name"),
                "yoake-config-name",
                220.,
            ),
            Field::Code => (
                state.code_focused,
                &state.code_focus,
                &state.code,
                tr("YK1-XXXXX-XXXXX-…"),
                "yoake-config-code",
                320.,
            ),
        };
        let element = div()
            .id(id)
            .track_focus(focus)
            .flex()
            .items_center()
            .w(px(width))
            .h(px(30.))
            .px(px(12.))
            .overflow_hidden()
            .border_2()
            .border_color(if focused { t.accent } else { t.outline })
            .bg(t.well)
            .font_family(if field == Field::Code {
                theme::MONO
            } else {
                theme::FONT
            })
            .text_size(px(12.))
            .cursor_text()
            .child(line_edit::content(edit, &t, focused, false, false, hint))
            .on_click(cx.listener(move |this, _: &ClickEvent, window, cx| {
                let focus = match field {
                    Field::Name => &this.yoake_configs.name_focus,
                    Field::Code => &this.yoake_configs.code_focus,
                };
                window.focus(focus, cx);
                cx.notify();
            }))
            .on_key_down(cx.listener(move |this, ev: &KeyDownEvent, window, cx| {
                let edit = match field {
                    Field::Name => &mut this.yoake_configs.name,
                    Field::Code => &mut this.yoake_configs.code,
                };
                match line_edit::handle(edit, ev, cx) {
                    line_edit::Key::Pass => return,
                    line_edit::Key::Enter => {
                        if field == Field::Code {
                            this.read_code();
                        } else {
                            this.save_draft();
                        }
                    }
                    line_edit::Key::Escape => window.blur(cx),
                    _ => {}
                }
                cx.stop_propagation();
                cx.notify();
            }));
        match field {
            Field::Name => line_edit::mouse(element, cx, |this| &mut this.yoake_configs.name),
            Field::Code => line_edit::mouse(element, cx, |this| &mut this.yoake_configs.code),
        }
    }

    fn mod_badge(&self, kind: ModKind) -> Div {
        let t = self.theme;
        let label = match kind {
            ModKind::Builtin => tr("Built-in"),
            ModKind::Yak => tr(".yak"),
            ModKind::Wasm => tr("WASM"),
        };
        div()
            .px(px(6.))
            .border_1()
            .border_color(t.outline)
            .child(text(theme::CAPTION).text_color(t.accent).child(label))
    }

    /// The whole section, below the packs.
    pub(super) fn yoake_configs_section(
        &mut self,
        window: &Window,
        cx: &mut Context<Self>,
    ) -> Div {
        let t = self.theme;
        self.yoake_configs.name_focused = self.yoake_configs.name_focus.is_focused(window);
        self.yoake_configs.code_focused = self.yoake_configs.code_focus.is_focused(window);
        let language = i18n::current().code();
        let bases = self.yoake_bases();
        let mut col = div().flex().flex_col().gap(px(12.)).child(
            div()
                .flex()
                .items_center()
                .justify_between()
                .mt(px(12.))
                .child(section_title("Yoake mods", "package", &t))
                .child(
                    button("yoake-mods-refresh", "Refresh", Kind::Ghost, &t).on_click(
                        cx.listener(|this, _: &ClickEvent, _, cx| {
                            this.yoake_configs.last_refresh = None;
                            this.refresh_yoake_catalog();
                            cx.notify();
                        }),
                    ),
                ),
        );
        if let Some(error) = &self.yoake_configs.error {
            col = col.child(text(theme::UI).text_color(t.danger).child(error.clone()));
        }
        if let Some(notice) = &self.yoake_configs.notice {
            col = col.child(
                text(theme::CAPTION)
                    .text_color(t.muted)
                    .child(trf(".yak mods are not listed: {reason}", &[("reason", notice)])),
            );
        }
        if self.yoake_configs.task.is_some() && self.yoake_configs.catalog.is_empty() {
            col = col.child(text(theme::UI).text_color(t.muted).child(tr("Checking the mods…")));
        } else if self.yoake_configs.catalog.is_empty() {
            col = col.child(
                text(theme::UI)
                    .text_color(t.muted)
                    .child(tr("No Yoake mods are listed yet. Sign in and refresh.")),
            );
        }
        // Where each built-in mod is installed: the local Yoake instances whose build has it.
        let installed: Vec<(String, Vec<String>)> = instances::list()
            .into_iter()
            .filter_map(|instance| {
                let pack = yoake_install::read_manifest(&instance.home).ok().flatten()?;
                let native = bases
                    .iter()
                    .find(|base| base.pack_id == pack.pack_id)
                    .map(|base| base.native_mods.clone())
                    .unwrap_or_default();
                Some((instance.name, native))
            })
            .collect();
        let yak_installed: BTreeSet<String> = self
            .yak_mods
            .installed_ids()
            .into_iter()
            .collect();
        let catalog = self.yoake_configs.catalog.clone();
        for (n, entry) in catalog.iter().enumerate() {
            let picked = self.yoake_configs.selected.contains(&entry.id);
            let required = self.yoake_configs.auto_added.contains(&entry.id);
            let mut detail = div().flex().flex_col().gap(px(2.));
            detail = detail.child(entry.description(language).to_string());
            detail = detail.child(trf(
                "{version} · by {author}",
                &[("version", &entry.version), ("author", &entry.author)],
            ));
            if !entry.requires.is_empty() {
                detail = detail.child(trf(
                    "Needs: {mods}",
                    &[("mods", &entry.requires.join(", "))],
                ));
            }
            let state_text = match entry.kind {
                ModKind::Builtin => {
                    let here: Vec<&str> = installed
                        .iter()
                        .filter(|(_, native)| native.contains(&entry.id))
                        .map(|(name, _)| name.as_str())
                        .collect();
                    if here.is_empty() {
                        let builds: Vec<&str> = bases
                            .iter()
                            .filter(|base| base.native_mods.contains(&entry.id))
                            .map(|base| base.name.as_str())
                            .collect();
                        (!builds.is_empty())
                            .then(|| trf("In the build of: {packs}", &[("packs", &builds.join(", "))]))
                    } else {
                        Some(trf("Installed in: {instances}", &[("instances", &here.join(", "))]))
                    }
                }
                _ => yak_installed
                    .contains(&entry.id)
                    .then(|| tr("Installed in the selected instance").to_string()),
            };
            if let Some(state_text) = state_text {
                detail = detail.child(text(theme::CAPTION).text_color(t.success).child(state_text));
            }
            let id = entry.id.clone();
            let toggle = button(
                SharedString::from(format!("yoake-mod-pick-{n}")),
                if picked { "Remove" } else { "Add to configuration" },
                if picked { Kind::Secondary } else { Kind::Ghost },
                &t,
            );
            let toggle = if required {
                disabled(toggle)
            } else {
                toggle.on_click(cx.listener(move |this, _: &ClickEvent, _, cx| {
                    this.toggle_config_mod(&id);
                    cx.notify();
                }))
            };
            let right = div()
                .flex()
                .items_center()
                .gap(px(8.))
                .child(self.mod_badge(entry.kind))
                .child(toggle);
            col = col.child(index_row(
                n,
                entry.name(language).to_string(),
                Some(detail.into_any_element()),
                Some(right.into_any_element()),
                &t,
            ));
        }

        // ---- the builder
        col = col.child(
            text(theme::UI)
                .text_color(t.ink)
                .mt(px(12.))
                .child(tr("Make a configuration")),
        );
        if self.yoake_configs.selected.is_empty() {
            col = col.child(
                text(theme::CAPTION)
                    .text_color(t.muted)
                    .child(tr("Pick mods above. Requirements are added for you.")),
            );
        } else {
            let names: Vec<String> = catalog
                .iter()
                .filter(|m| self.yoake_configs.selected.contains(&m.id))
                .map(|m| {
                    if self.yoake_configs.auto_added.contains(&m.id) {
                        trf("{name} (required)", &[("name", m.name(language))])
                    } else {
                        m.name(language).to_owned()
                    }
                })
                .collect();
            col = col.child(text(theme::UI).text_color(t.muted).child(names.join(" / ")));
        }
        if !bases.is_empty() {
            let chosen = self
                .yoake_configs
                .base
                .clone()
                .unwrap_or_else(|| bases[0].pack_id.clone());
            let mut chips = div().flex().flex_wrap().gap(px(8.)).items_center().child(
                text(theme::CAPTION).text_color(t.muted).child(tr("Base pack")),
            );
            for (n, base) in bases.iter().enumerate() {
                let id = base.pack_id.clone();
                chips = chips.child(
                    button_label(
                        SharedString::from(format!("yoake-config-base-{n}")),
                        base.name.clone(),
                        if base.pack_id == chosen { Kind::Secondary } else { Kind::Ghost },
                        &t,
                    )
                    .on_click(cx.listener(move |this, _: &ClickEvent, _, cx| {
                        this.yoake_configs.base = Some(id.clone());
                        this.yoake_configs.share = None;
                        cx.notify();
                    })),
                );
            }
            col = col.child(chips);
        }
        let actions = div()
            .flex()
            .flex_wrap()
            .items_center()
            .gap(px(8.))
            .child(self.config_field(Field::Name, cx))
            .child(
                button("yoake-config-save", "Save configuration", Kind::Primary, &t).on_click(
                    cx.listener(|this, _: &ClickEvent, _, cx| {
                        this.save_draft();
                        cx.notify();
                    }),
                ),
            )
            .child(
                button("yoake-config-share", "Share code", Kind::Secondary, &t).on_click(
                    cx.listener(|this, _: &ClickEvent, _, cx| {
                        match this.draft() {
                            Ok(configuration) => this.share_configuration(&configuration),
                            Err(error) => this.yoake_configs.note = Some((false, error)),
                        }
                        cx.notify();
                    }),
                ),
            );
        col = col.child(actions);
        if let Some((name, code)) = self.yoake_configs.share.clone() {
            let copy = code.clone();
            col = col.child(
                plate(&t)
                    .flex()
                    .flex_col()
                    .gap(px(8.))
                    .p(px(12.))
                    .child(
                        text(theme::CAPTION)
                            .text_color(t.muted)
                            .child(trf("Share code for {name}", &[("name", &name)])),
                    )
                    .child(
                        div()
                            .font_family(theme::MONO)
                            .text_size(px(12.))
                            .text_color(t.ink)
                            .child(code),
                    )
                    .child(
                        button("yoake-config-copy", "Copy", Kind::Ghost, &t).on_click(
                            cx.listener(move |_, _: &ClickEvent, _, cx| {
                                copy_to_clipboard(cx, gpui::ClipboardItem::new_string(copy.clone()));
                            }),
                        ),
                    ),
            );
        }

        // ---- saved configurations
        let saved = self.yoake_configs.saved.clone();
        let bases_map = self.bases_by_pack();
        for (n, configuration) in saved.iter().enumerate() {
            let preview = yoake_configs::resolve(configuration, &self.yoake_configs.catalog, &bases_map);
            let names: Vec<String> = preview
                .mods
                .iter()
                .map(|entry| {
                    self.yoake_configs
                        .catalog
                        .iter()
                        .find(|m| m.id == entry.id)
                        .map(|m| m.name(language).to_owned())
                        .unwrap_or_else(|| entry.id.clone())
                })
                .collect();
            let mut detail = div().flex().flex_col().gap(px(2.)).child(names.join(" / "));
            for problem in &preview.problems {
                detail = detail.child(text(theme::CAPTION).text_color(t.danger).child(problem_text(problem)));
            }
            let busy = self.yoake_busy() || self.yoake_configs.pending.is_some();
            let (share_cfg, remove_name) = (configuration.clone(), configuration.name.clone());
            let install_preview = preview.clone();
            let install = button(
                SharedString::from(format!("yoake-config-install-{n}")),
                "Install as a new instance",
                Kind::Primary,
                &t,
            );
            let install = if busy || !preview.installable() {
                disabled(install)
            } else {
                install.on_click(cx.listener(move |this, _: &ClickEvent, _, cx| {
                    this.install_configuration(&install_preview);
                    cx.notify();
                }))
            };
            let right = div()
                .flex()
                .items_center()
                .gap(px(8.))
                .child(install)
                .child(
                    button(
                        SharedString::from(format!("yoake-config-share-{n}")),
                        "Share code",
                        Kind::Secondary,
                        &t,
                    )
                    .on_click(cx.listener(move |this, _: &ClickEvent, _, cx| {
                        this.share_configuration(&share_cfg);
                        cx.notify();
                    })),
                )
                .child(
                    button(
                        SharedString::from(format!("yoake-config-delete-{n}")),
                        "Delete",
                        Kind::Ghost,
                        &t,
                    )
                    .on_click(cx.listener(move |this, _: &ClickEvent, _, cx| {
                        let _ = yoake_configs::remove(&backend::data_dir(), &remove_name);
                        this.yoake_configs.saved = yoake_configs::load(&backend::data_dir());
                        cx.notify();
                    })),
                );
            col = col.child(index_row(
                n,
                configuration.name.clone(),
                Some(detail.into_any_element()),
                Some(right.into_any_element()),
                &t,
            ));
        }

        // ---- add by code
        col = col.child(
            text(theme::UI)
                .text_color(t.ink)
                .mt(px(12.))
                .child(tr("Add by code")),
        );
        col = col.child(
            div()
                .flex()
                .flex_wrap()
                .items_center()
                .gap(px(8.))
                .child(self.config_field(Field::Code, cx))
                .child(
                    button("yoake-config-read", "Preview", Kind::Secondary, &t).on_click(
                        cx.listener(|this, _: &ClickEvent, _, cx| {
                            this.read_code();
                            cx.notify();
                        }),
                    ),
                ),
        );
        if let Some((configuration, preview)) = self.yoake_configs.preview.clone() {
            let mut card = plate(&t)
                .flex()
                .flex_col()
                .gap(px(6.))
                .p(px(12.))
                .child(text(theme::UI).text_color(t.ink).child(preview.name.clone()));
            for entry in &preview.mods {
                let name = self
                    .yoake_configs
                    .catalog
                    .iter()
                    .find(|m| m.id == entry.id)
                    .map(|m| m.name(language).to_owned())
                    .unwrap_or_else(|| entry.id.clone());
                let line = if entry.auto_added {
                    trf("{name} {version} (required)", &[("name", &name), ("version", &entry.version)])
                } else {
                    format!("{name} {}", entry.version)
                };
                card = card.child(text(theme::CAPTION).text_color(t.muted).child(line));
            }
            for problem in &preview.problems {
                card = card.child(text(theme::CAPTION).text_color(t.danger).child(problem_text(problem)));
            }
            let busy = self.yoake_busy() || self.yoake_configs.pending.is_some();
            let install_preview = preview.clone();
            let install = button("yoake-config-preview-install", "Install as a new instance", Kind::Primary, &t);
            let install = if busy || !preview.installable() {
                disabled(install)
            } else {
                install.on_click(cx.listener(move |this, _: &ClickEvent, _, cx| {
                    this.install_configuration(&install_preview);
                    cx.notify();
                }))
            };
            let keep = configuration.clone();
            card = card.child(
                div()
                    .flex()
                    .gap(px(8.))
                    .child(install)
                    .child(
                        button("yoake-config-preview-save", "Save configuration", Kind::Secondary, &t)
                            .on_click(cx.listener(move |this, _: &ClickEvent, _, cx| {
                                match yoake_configs::save(&backend::data_dir(), keep.clone()) {
                                    Ok(()) => {
                                        this.yoake_configs.saved =
                                            yoake_configs::load(&backend::data_dir());
                                        this.yoake_configs.note =
                                            Some((true, tr("Configuration saved.").to_string()));
                                    }
                                    Err(error) => this.yoake_configs.note = Some((false, error)),
                                }
                                cx.notify();
                            })),
                    ),
            );
            col = col.child(card);
        }
        if self.yoake_configs.yak_task.is_some() || self.yoake_configs.pending.is_some() {
            col = col.child(
                text(theme::UI)
                    .text_color(t.muted)
                    .child(tr("Setting up the new instance…")),
            );
        }
        if let Some((ok, note)) = &self.yoake_configs.note {
            col = col.child(
                text(theme::UI)
                    .text_color(if *ok { t.success } else { t.danger })
                    .child(note.clone()),
            );
        }
        col
    }
}
