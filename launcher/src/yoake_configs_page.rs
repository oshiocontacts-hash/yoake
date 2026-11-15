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

/// The merged catalog, why the `.yak` half is missing, and what is wrong with the built-in half.
type Fetched = (Vec<CatalogMod>, Option<String>, Option<String>);

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
    /// A problem with the built-in half (the last verified copy, if any, is still listed).
    builtin_notice: Option<String>,
    error: Option<String>,
    /// Local Yoake instances as (name, base pack id), read off the UI thread and refreshed on change.
    installed: Vec<(String, String)>,
    installed_task: Option<Receiver<Vec<(String, String)>>>,
    installed_at: Option<Instant>,
    installed_dirty: bool,
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
            builtin_notice: None,
            error: None,
            installed: Vec::new(),
            installed_task: None,
            installed_at: None,
            installed_dirty: true,
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
            code: line_edit::LineEdit::new(String::new(), yoake_configs::MAX_CODE_INPUT, 22),
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
        Problem::DependencyVersion {
            id,
            needs,
            requirement,
        } => trf(
            "{id} needs {needs} {requirement}, which the chosen version does not match.",
            &[("id", id), ("needs", needs), ("requirement", requirement)],
        ),
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
            // The two halves are independent: a problem with one never hides the other.
            let builtin = client.refresh_builtin();
            let (yak, notice) = match client.refresh() {
                Ok(yak) => (Some(yak.catalog), None),
                Err(error) => (None, Some(error.to_string())),
            };
            let builtin_notice = builtin.problem.as_ref().map(ToString::to_string);
            let catalog = yoake_configs::merge(builtin.catalog.as_ref(), yak.as_ref());
            let _ = sender.send(Ok((catalog, notice, builtin_notice)));
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
                Ok(Ok((catalog, notice, builtin_notice))) => {
                    self.yoake_configs.catalog = catalog;
                    self.yoake_configs.notice = notice;
                    self.yoake_configs.builtin_notice = builtin_notice;
                    self.yoake_configs.installed_dirty = true;
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
        // The local instances, read on a worker thread: on a change and every few seconds.
        if self.yoake_configs.installed_task.is_none()
            && (self.yoake_configs.installed_dirty
                || self
                    .yoake_configs
                    .installed_at
                    .is_none_or(|time| time.elapsed() >= Duration::from_secs(10)))
        {
            self.yoake_configs.installed_dirty = false;
            self.yoake_configs.installed_at = Some(Instant::now());
            let (sender, receiver) = mpsc::channel();
            std::thread::spawn(move || {
                let list = instances::list()
                    .into_iter()
                    .filter_map(|instance| {
                        let pack = yoake_install::read_manifest(&instance.home).ok().flatten()?;
                        Some((instance.name, pack.pack_id))
                    })
                    .collect();
                let _ = sender.send(list);
            });
            self.yoake_configs.installed_task = Some(receiver);
        }
        if let Some(receiver) = self.yoake_configs.installed_task.take() {
            match receiver.try_recv() {
                Ok(list) => {
                    if self.yoake_configs.installed != list {
                        self.yoake_configs.installed = list;
                        changed = true;
                    }
                }
                Err(mpsc::TryRecvError::Empty) => self.yoake_configs.installed_task = Some(receiver),
                Err(mpsc::TryRecvError::Disconnected) => {}
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
            self.yoake_configs.installed_dirty = true;
            self.yoake_configs.pending = Some((home, preview.yak_set()));
            self.yoake_configs.note = None;
        }
    }

    /// A text field of the section, in the glass look's `FIELD` (36 px, by the buttons).
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
        let element = glass_ui::field(focused, false)
            .id(id)
            .track_focus(focus)
            .w(px(width))
            .h(px(36.))
            .px(px(12.))
            .rounded(px(10.))
            .when(field == Field::Code, |d| {
                d.font_family(theme::MONO).text_size(px(13.))
            })
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
        use horizon_theme::glass::ink;
        let label = match kind {
            ModKind::Builtin => tr("Built-in"),
            ModKind::Yak => tr(".yak"),
            ModKind::Wasm => tr("WASM"),
        };
        div()
            .flex()
            .items_center()
            .px(px(8.))
            .h(px(22.))
            .rounded_full()
            .bg(glass_ui::white(horizon_theme::glass::fill::BUTTON))
            .child(glass_ui::text(12., FontWeight::NORMAL, ink::MUTED).child(label))
    }

    /// A card inside the section's panel (a share code, a code's preview): white at 6 %.
    fn config_card() -> Div {
        div()
            .flex()
            .flex_col()
            .gap(px(6.))
            .mx(px(12.))
            .mb(px(8.))
            .p(px(14.))
            .rounded(px(horizon_theme::glass::radius::ROW))
            .bg(glass_ui::white(horizon_theme::glass::fill::FIELD))
    }

    /// The whole section, below the packs.
    pub(super) fn yoake_configs_section(
        &mut self,
        window: &Window,
        cx: &mut Context<Self>,
    ) -> Div {
        use horizon_theme::glass::ink;
        let t = self.theme;
        self.yoake_configs.name_focused = self.yoake_configs.name_focus.is_focused(window);
        self.yoake_configs.code_focused = self.yoake_configs.code_focus.is_focused(window);
        let language = i18n::current().code();
        let bases = self.yoake_bases();
        let refresh = glass_ui::quiet("yoake-mods-refresh", tr("Refresh"), 36., &t).on_click(
            cx.listener(|this, _: &ClickEvent, _, cx| {
                this.yoake_configs.last_refresh = None;
                this.refresh_yoake_catalog();
                cx.notify();
            }),
        );
        let mut col = glass_body();
        if let Some(error) = &self.yoake_configs.error {
            col = col.child(glass_note(error.clone(), true));
        }
        if let Some(notice) = &self.yoake_configs.builtin_notice {
            col = col.child(glass_note(
                trf("Built-in mods: {reason}", &[("reason", notice)]),
                false,
            ));
        }
        if let Some(notice) = &self.yoake_configs.notice {
            col = col.child(glass_note(
                trf(".yak mods are not listed: {reason}", &[("reason", notice)]),
                false,
            ));
        }
        if self.yoake_configs.task.is_some() && self.yoake_configs.catalog.is_empty() {
            col = col.child(glass_ui::empty().py(px(16.)).child(tr("Checking the mods…")));
        } else if self.yoake_configs.catalog.is_empty() {
            col = col.child(
                glass_ui::empty()
                    .py(px(16.))
                    .child(tr("No Yoake mods are listed yet. Sign in and refresh.")),
            );
        }
        // Where each built-in mod is installed: the local Yoake instances whose build has it.
        let installed: Vec<(String, Vec<String>)> = self
            .yoake_configs
            .installed
            .iter()
            .map(|(name, pack_id)| {
                let native = bases
                    .iter()
                    .find(|base| &base.pack_id == pack_id)
                    .map(|base| base.native_mods.clone())
                    .unwrap_or_default();
                (name.clone(), native)
            })
            .collect();
        let yak_installed: BTreeSet<String> = self.yak_mods.installed_ids().into_iter().collect();
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
                        (!builds.is_empty()).then(|| {
                            trf("In the build of: {packs}", &[("packs", &builds.join(", "))])
                        })
                    } else {
                        Some(trf(
                            "Installed in: {instances}",
                            &[("instances", &here.join(", "))],
                        ))
                    }
                }
                _ => yak_installed
                    .contains(&entry.id)
                    .then(|| tr("Installed in the selected instance").to_string()),
            };
            if let Some(state_text) = state_text {
                detail = detail.child(
                    div()
                        .text_color(glass_ui::color(horizon_theme::glass::LIVE))
                        .child(state_text),
                );
            }
            let id = entry.id.clone();
            let toggle_id = SharedString::from(format!("yoake-mod-pick-{n}"));
            let toggle = if picked {
                glass_ui::solid(toggle_id, tr("Remove"), 36., 14., &t)
            } else {
                glass_ui::quiet(toggle_id, tr("Add to configuration"), 36., &t)
            };
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
            col = col.child(glass_index_row(
                entry.name(language).to_string(),
                Some(detail.into_any_element()),
                Some(right.into_any_element()),
            ));
        }

        // ---- the builder
        col = col.child(glass_subhead(tr("Make a configuration")));
        if self.yoake_configs.selected.is_empty() {
            col = col.child(glass_note(
                tr("Pick mods above. Requirements are added for you."),
                false,
            ));
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
            col = col.child(glass_note(names.join(" / "), false));
        }
        if !bases.is_empty() {
            let chosen = self
                .yoake_configs
                .base
                .clone()
                .unwrap_or_else(|| bases[0].pack_id.clone());
            let mut chips = div()
                .flex()
                .flex_wrap()
                .gap(px(8.))
                .items_center()
                .px(px(12.))
                .pb(px(8.))
                .child(
                    glass_ui::text(13., FontWeight::NORMAL, ink::NOTE).child(tr("Base pack")),
                );
            for (n, base) in bases.iter().enumerate() {
                let id = base.pack_id.clone();
                let chip_id = SharedString::from(format!("yoake-config-base-{n}"));
                let chip = if base.pack_id == chosen {
                    glass_ui::solid(chip_id, base.name.clone(), 36., 14., &t)
                } else {
                    glass_ui::quiet(chip_id, base.name.clone(), 36., &t)
                };
                chips = chips.child(chip.on_click(cx.listener(
                    move |this, _: &ClickEvent, _, cx| {
                        this.yoake_configs.base = Some(id.clone());
                        this.yoake_configs.share = None;
                        cx.notify();
                    },
                )));
            }
            col = col.child(chips);
        }
        let actions = div()
            .flex()
            .flex_wrap()
            .items_center()
            .gap(px(8.))
            .px(px(12.))
            .pb(px(8.))
            .child(self.config_field(Field::Name, cx))
            .child(
                glass_ui::solid("yoake-config-save", tr("Save configuration"), 36., 14., &t)
                    .on_click(cx.listener(|this, _: &ClickEvent, _, cx| {
                        this.save_draft();
                        cx.notify();
                    })),
            )
            .child(
                glass_ui::quiet("yoake-config-share", tr("Share code"), 36., &t).on_click(
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
                Self::config_card()
                    .child(
                        glass_ui::text(13., FontWeight::NORMAL, ink::NOTE)
                            .child(trf("Share code for {name}", &[("name", &name)])),
                    )
                    .child(
                        glass_ui::text(13., FontWeight::NORMAL, ink::FULL)
                            .font_family(theme::MONO)
                            .child(code),
                    )
                    .child(
                        div().flex().child(
                            glass_ui::quiet("yoake-config-copy", tr("Copy"), 36., &t).on_click(
                                cx.listener(move |_, _: &ClickEvent, _, cx| {
                                    copy_to_clipboard(
                                        cx,
                                        gpui::ClipboardItem::new_string(copy.clone()),
                                    );
                                }),
                            ),
                        ),
                    ),
            );
        }

        // ---- saved configurations
        let saved = self.yoake_configs.saved.clone();
        let bases_map = self.bases_by_pack();
        for (n, configuration) in saved.iter().enumerate() {
            let preview =
                yoake_configs::resolve(configuration, &self.yoake_configs.catalog, &bases_map);
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
                detail = detail.child(
                    div()
                        .text_color(glass_ui::color(horizon_theme::glass::DANGER))
                        .child(problem_text(problem)),
                );
            }
            let busy = self.yoake_busy() || self.yoake_configs.pending.is_some();
            let (share_cfg, remove_name) = (configuration.clone(), configuration.name.clone());
            let install_preview = preview.clone();
            let install = glass_ui::solid(
                SharedString::from(format!("yoake-config-install-{n}")),
                tr("Install as a new instance"),
                36.,
                14.,
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
                    glass_ui::quiet(
                        SharedString::from(format!("yoake-config-share-{n}")),
                        tr("Share code"),
                        36.,
                        &t,
                    )
                    .on_click(cx.listener(move |this, _: &ClickEvent, _, cx| {
                        this.share_configuration(&share_cfg);
                        cx.notify();
                    })),
                )
                .child(
                    glass_ui::quiet(
                        SharedString::from(format!("yoake-config-delete-{n}")),
                        tr("Delete"),
                        36.,
                        &t,
                    )
                    .on_click(cx.listener(move |this, _: &ClickEvent, _, cx| {
                        let _ = yoake_configs::remove(&backend::data_dir(), &remove_name);
                        this.yoake_configs.saved = yoake_configs::load(&backend::data_dir());
                        cx.notify();
                    })),
                );
            col = col.child(glass_index_row(
                configuration.name.clone(),
                Some(detail.into_any_element()),
                Some(right.into_any_element()),
            ));
        }

        // ---- add by code
        col = col.child(glass_subhead(tr("Add by code")));
        col = col.child(
            div()
                .flex()
                .flex_wrap()
                .items_center()
                .gap(px(8.))
                .px(px(12.))
                .pb(px(8.))
                .child(self.config_field(Field::Code, cx))
                .child(
                    glass_ui::quiet("yoake-config-read", tr("Preview"), 36., &t).on_click(
                        cx.listener(|this, _: &ClickEvent, _, cx| {
                            this.read_code();
                            cx.notify();
                        }),
                    ),
                ),
        );
        if let Some((configuration, preview)) = self.yoake_configs.preview.clone() {
            let mut card = Self::config_card().child(
                glass_ui::text(14., FontWeight::MEDIUM, ink::FULL).child(preview.name.clone()),
            );
            for entry in &preview.mods {
                let name = self
                    .yoake_configs
                    .catalog
                    .iter()
                    .find(|m| m.id == entry.id)
                    .map(|m| m.name(language).to_owned())
                    .unwrap_or_else(|| entry.id.clone());
                let line = if entry.auto_added {
                    trf(
                        "{name} {version} (required)",
                        &[("name", &name), ("version", &entry.version)],
                    )
                } else {
                    format!("{name} {}", entry.version)
                };
                card = card.child(glass_ui::text(13., FontWeight::NORMAL, ink::NOTE).child(line));
            }
            for problem in &preview.problems {
                card = card.child(
                    glass_ui::text(13., FontWeight::NORMAL, ink::FULL)
                        .text_color(glass_ui::color(horizon_theme::glass::DANGER))
                        .child(problem_text(problem)),
                );
            }
            let busy = self.yoake_busy() || self.yoake_configs.pending.is_some();
            let install_preview = preview.clone();
            let install = glass_ui::solid(
                "yoake-config-preview-install",
                tr("Install as a new instance"),
                36.,
                14.,
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
            let keep = configuration.clone();
            card = card.child(
                div()
                    .flex()
                    .gap(px(8.))
                    .child(install)
                    .child(
                        glass_ui::quiet(
                            "yoake-config-preview-save",
                            tr("Save configuration"),
                            36.,
                            &t,
                        )
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
            col = col.child(glass_note(tr("Setting up the new instance…"), false));
        }
        if let Some((ok, note)) = &self.yoake_configs.note {
            col = col.child(glass_note(note.clone(), !*ok));
        }
        glass_section(
            "Yoake mods",
            "package-01",
            Some(refresh.into_any_element()),
            col,
        )
    }
}
