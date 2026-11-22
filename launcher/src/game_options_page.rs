//! Glass UI for game options (`horizon_launcher::game_options`):
//! - Instance page tab ("Game settings"): per-instance overrides, "Use usual settings" toggle,
//!   "Load this instance's current settings", "Copy to usual settings", and inherited preview.
//! - Launcher settings section ("Usual settings"): global defaults applied before launch.
use super::line_edit::{self, Key, LineEdit};
use super::*;
use crate::glass_ui::{self as ui, white};
use gpui::{
    canvas, deferred, Bounds, MouseButton, MouseDownEvent, MouseMoveEvent, MouseUpEvent, Pixels,
};
use horizon_launcher::game_options::{
    self, ApplyMode, ControlKind, FriendlyDef, GlobalProfile, OptionEntry,
};
use horizon_theme::glass::{fill, ink, radius};
use std::cell::RefCell;
use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::rc::Rc;

/// Which profile an editor widget targets.
#[derive(Clone, PartialEq, Eq, Hash, Debug)]
pub(super) enum Scope {
    Global,
    Instance(String),
}

impl Scope {
    fn tag(&self) -> String {
        match self {
            Scope::Global => "global".to_string(),
            Scope::Instance(id) => format!("inst-{id}"),
        }
    }
}

pub(super) struct GameOptionsUi {
    pub(super) global: GlobalProfile,
    add_menu: Option<Scope>,
    lang_menu: Option<Scope>,
    slider_areas: Rc<RefCell<HashMap<(Scope, String), Bounds<Pixels>>>>,
    slider_drag: Option<(Scope, String)>,
    search: LineEdit,
    search_focus: FocusHandle,
    search_focused: bool,
    raw_key: LineEdit,
    raw_key_focus: FocusHandle,
    raw_key_focused: bool,
    raw_val: LineEdit,
    raw_val_focus: FocusHandle,
    raw_val_focused: bool,
    raw_mode: ApplyMode,
    raw_editors: HashMap<(Scope, String), (LineEdit, FocusHandle, bool)>,
    scanned_home: Option<PathBuf>,
    scanned_options: Vec<(String, String)>,
    scanned_stale: bool,
    global_note: Option<(bool, String)>,
    instance_note: Option<(bool, String)>,
}

impl GameOptionsUi {
    pub(super) fn new(cx: &mut App) -> Self {
        Self {
            global: game_options::load_global(),
            add_menu: None,
            lang_menu: None,
            slider_areas: Rc::new(RefCell::new(HashMap::new())),
            slider_drag: None,
            search: LineEdit::new(String::new(), 128, 36),
            search_focus: cx.focus_handle().tab_index(0).tab_stop(true),
            search_focused: false,
            raw_key: LineEdit::new(String::new(), 128, 26),
            raw_key_focus: cx.focus_handle().tab_index(0).tab_stop(true),
            raw_key_focused: false,
            raw_val: LineEdit::new(String::new(), 512, 26),
            raw_val_focus: cx.focus_handle().tab_index(0).tab_stop(true),
            raw_val_focused: false,
            raw_mode: ApplyMode::Always,
            raw_editors: HashMap::new(),
            scanned_home: None,
            scanned_options: Vec::new(),
            scanned_stale: true,
            global_note: None,
            instance_note: None,
        }
    }

    pub(super) fn note_focus(&mut self, window: &Window) {
        self.search_focused = self.search_focus.is_focused(window);
        self.raw_key_focused = self.raw_key_focus.is_focused(window);
        self.raw_val_focused = self.raw_val_focus.is_focused(window);
        for (_, focus, focused) in self.raw_editors.values_mut() {
            *focused = focus.is_focused(window);
        }
    }

    pub(super) fn escape(&mut self) -> bool {
        if self.add_menu.is_some() || self.lang_menu.is_some() {
            self.add_menu = None;
            self.lang_menu = None;
            return true;
        }
        false
    }

    pub(super) fn stale_scan(&mut self) {
        self.scanned_stale = true;
    }

    fn ensure_scan(&mut self, home: Option<&Path>) {
        let Some(home) = home else {
            self.scanned_home = None;
            self.scanned_options.clear();
            self.scanned_stale = false;
            return;
        };
        if !self.scanned_stale && self.scanned_home.as_deref() == Some(home) {
            return;
        }
        self.scanned_home = Some(home.to_path_buf());
        self.scanned_options = game_options::read_options_txt(home)
            .ok()
            .flatten()
            .unwrap_or_default();
        self.scanned_stale = false;
    }

    fn set_note(&mut self, scope: &Scope, ok: bool, msg: String) {
        match scope {
            Scope::Global => self.global_note = Some((ok, msg)),
            Scope::Instance(_) => self.instance_note = Some((ok, msg)),
        }
    }

    fn note_for(&self, scope: &Scope) -> Option<&(bool, String)> {
        match scope {
            Scope::Global => self.global_note.as_ref(),
            Scope::Instance(_) => self.instance_note.as_ref(),
        }
    }
}

/// Range and step metadata for slider-backed friendly options.
struct SliderSpec {
    min: i32,
    max: i32,
    step: i32,
    value: i32,
    display: String,
}

fn slider_spec(kind: ControlKind, raw: &str) -> Option<SliderSpec> {
    match kind {
        ControlKind::UnitPercent => {
            let v = game_options::decode_unit_percent(raw).unwrap_or(100);
            Some(SliderSpec {
                min: 0,
                max: 100,
                step: 1,
                value: v,
                display: format!("{v}%"),
            })
        }
        ControlKind::Fov => {
            let v = game_options::decode_fov(raw).unwrap_or(70);
            Some(SliderSpec {
                min: 30,
                max: 110,
                step: 1,
                value: v,
                display: format!("{v}°"),
            })
        }
        ControlKind::RenderDistance => {
            let v = game_options::decode_render_distance(raw).unwrap_or(12);
            Some(SliderSpec {
                min: 2,
                max: 32,
                step: 1,
                value: v,
                display: trf("{n} chunks", &[("n", &v.to_string())]),
            })
        }
        ControlKind::SimulationDistance => {
            let v = game_options::decode_simulation_distance(raw).unwrap_or(12);
            Some(SliderSpec {
                min: 5,
                max: 32,
                step: 1,
                value: v,
                display: trf("{n} chunks", &[("n", &v.to_string())]),
            })
        }
        ControlKind::MaxFps => {
            let v = game_options::decode_max_fps(raw).unwrap_or(120);
            let display = if v >= 260 {
                tr("Unlimited").to_string()
            } else {
                format!("{v} FPS")
            };
            Some(SliderSpec {
                min: 10,
                max: 260,
                step: 10,
                value: v,
                display,
            })
        }
        ControlKind::MouseSensitivity => {
            let v = game_options::decode_mouse_sensitivity(raw).unwrap_or(100);
            Some(SliderSpec {
                min: 0,
                max: 200,
                step: 1,
                value: v,
                display: format!("{v}%"),
            })
        }
        _ => None,
    }
}

fn encode_slider_value(kind: ControlKind, value: i32) -> Result<String, String> {
    match kind {
        ControlKind::UnitPercent => game_options::encode_unit_percent(value.clamp(0, 100)),
        ControlKind::Fov => game_options::encode_fov(value.clamp(30, 110)),
        ControlKind::RenderDistance => game_options::encode_render_distance(value.clamp(2, 128)),
        ControlKind::SimulationDistance => {
            game_options::encode_simulation_distance(value.clamp(5, 32))
        }
        ControlKind::MaxFps => {
            let snapped = ((value.clamp(10, 260) + 5) / 10) * 10;
            game_options::encode_max_fps(snapped.clamp(10, 260))
        }
        ControlKind::MouseSensitivity => {
            game_options::encode_mouse_sensitivity(value.clamp(0, 200))
        }
        _ => Err("Not a slider option.".into()),
    }
}

fn format_friendly_summary(def: &FriendlyDef, raw: &str) -> String {
    match def.kind {
        ControlKind::UnitPercent => game_options::decode_unit_percent(raw)
            .map_or_else(|| raw.to_string(), |v| format!("{v}%")),
        ControlKind::Fov => game_options::decode_fov(raw)
            .map_or_else(|| raw.to_string(), |v| format!("{v}°")),
        ControlKind::RenderDistance => game_options::decode_render_distance(raw)
            .map_or_else(|| raw.to_string(), |v| trf("{n} chunks", &[("n", &v.to_string())])),
        ControlKind::SimulationDistance => game_options::decode_simulation_distance(raw)
            .map_or_else(|| raw.to_string(), |v| trf("{n} chunks", &[("n", &v.to_string())])),
        ControlKind::GuiScale => match game_options::decode_gui_scale(raw) {
            Some(0) => tr("Auto").to_string(),
            Some(n) => n.to_string(),
            None => raw.to_string(),
        },
        ControlKind::MaxFps => match game_options::decode_max_fps(raw) {
            Some(260) => tr("Unlimited").to_string(),
            Some(n) => format!("{n} FPS"),
            None => raw.to_string(),
        },
        ControlKind::MouseSensitivity => game_options::decode_mouse_sensitivity(raw)
            .map_or_else(|| raw.to_string(), |v| format!("{v}%")),
        ControlKind::Bool => match game_options::decode_bool(raw) {
            Some(true) => tr("On").to_string(),
            Some(false) => tr("Off").to_string(),
            None => raw.to_string(),
        },
        ControlKind::Language => language_label(raw),
    }
}

fn language_label(code: &str) -> String {
    game_options::LANGUAGES
        .iter()
        .find(|(c, _)| *c == code)
        .map_or_else(|| code.to_string(), |(_, name)| (*name).to_string())
}

impl Launcher {
    /// Home's "Game settings" tab for the selected instance.
    pub(super) fn glass_game_settings(&mut self, cx: &mut Context<Self>) -> Option<Div> {
        let instance = self.selected_instance()?.clone();
        let scope = Scope::Instance(instance.id.clone());
        Some(self.game_options_editor(scope, Some(&instance), cx))
    }

    /// Launcher Settings' "Usual settings" section.
    pub(super) fn usual_settings_section(&mut self, cx: &mut Context<Self>) -> Div {
        let t = self.theme;
        let selected = self.selected_instance().cloned();
        let body = self.game_options_editor(Scope::Global, selected.as_ref(), cx);
        if t.dark {
            crate::glass::plate(radius::PANEL)
                .p(px(24.))
                .child(body)
        } else {
            plate(&t).p(px(24.)).child(body)
        }
    }

    /// Count of configured override entries for the selected instance (shown on its tab).
    pub(super) fn game_settings_count(&self) -> Option<usize> {
        let count = self.selected_instance()?.game_options.entries.len();
        (count > 0).then_some(count)
    }

    fn entries_for(&self, scope: &Scope) -> Vec<OptionEntry> {
        match scope {
            Scope::Global => self.game_options.global.entries.clone(),
            Scope::Instance(id) => self
                .instances
                .list
                .iter()
                .find(|i| &i.id == id)
                .map(|i| i.game_options.entries.clone())
                .unwrap_or_default(),
        }
    }

    fn mutate_entries(
        &mut self,
        scope: &Scope,
        save_to_disk: bool,
        f: impl FnOnce(&mut Vec<OptionEntry>),
    ) {
        match scope {
            Scope::Global => {
                f(&mut self.game_options.global.entries);
                if save_to_disk {
                    if let Err(e) = game_options::save_global(&self.game_options.global) {
                        self.game_options.set_note(scope, false, e);
                    }
                }
            }
            Scope::Instance(id) => {
                if save_to_disk {
                    match instances::edit(id, |meta| f(&mut meta.game_options.entries)) {
                        Ok(_) => self.reload_instances(),
                        Err(e) => self.game_options.set_note(scope, false, e),
                    }
                } else if let Some(inst) = self.instances.list.iter_mut().find(|i| &i.id == id) {
                    f(&mut inst.game_options.entries);
                }
            }
        }
    }

    fn save_scope_now(&mut self, scope: &Scope) {
        match scope {
            Scope::Global => {
                if let Err(e) = game_options::save_global(&self.game_options.global) {
                    self.game_options.set_note(scope, false, e);
                }
            }
            Scope::Instance(id) => {
                let Some(entries) = self
                    .instances
                    .list
                    .iter()
                    .find(|i| &i.id == id)
                    .map(|i| i.game_options.entries.clone())
                else {
                    return;
                };
                match instances::edit(id, |meta| meta.game_options.entries = entries) {
                    Ok(_) => self.reload_instances(),
                    Err(e) => self.game_options.set_note(scope, false, e),
                }
            }
        }
    }

    fn set_use_usual(&mut self, id: &str, use_usual: bool) {
        match instances::edit(id, |meta| meta.game_options.use_usual = use_usual) {
            Ok(_) => self.reload_instances(),
            Err(e) => {
                self.game_options
                    .set_note(&Scope::Instance(id.to_string()), false, e);
            }
        }
    }

    fn load_from_instance_options(&mut self, scope: &Scope, home: &Path) {
        let existing = self.entries_for(scope);
        match game_options::load_profile_from_options_txt(home, &existing) {
            Ok(Some(loaded)) => {
                let count = loaded.len();
                self.mutate_entries(scope, true, |entries| {
                    *entries = loaded;
                });
                self.game_options.stale_scan();
                self.game_options.set_note(
                    scope,
                    true,
                    trf(
                        "Loaded {count} settings from options.txt.",
                        &[("count", &count.to_string())],
                    ),
                );
            }
            Ok(None) => {
                self.game_options.set_note(
                    scope,
                    false,
                    tr("No options.txt found in this instance yet.").to_string(),
                );
            }
            Err(e) => self.game_options.set_note(scope, false, e),
        }
    }

    fn copy_instance_to_usual(&mut self, scope: &Scope) {
        let entries = self.entries_for(scope);
        if entries.is_empty() {
            self.game_options.set_note(
                scope,
                false,
                tr("No instance settings to copy yet. Add settings or load from options.txt first.")
                    .to_string(),
            );
            return;
        }
        let count = entries.len();
        for entry in entries {
            game_options::upsert_entry(
                &mut self.game_options.global.entries,
                entry.key,
                entry.value,
                entry.mode,
            );
        }
        match game_options::save_global(&self.game_options.global) {
            Ok(()) => self.game_options.set_note(
                scope,
                true,
                trf(
                    "Copied {count} settings to usual settings.",
                    &[("count", &count.to_string())],
                ),
            ),
            Err(e) => self.game_options.set_note(scope, false, e),
        }
    }

    fn commit_raw_add(&mut self, scope: &Scope) {
        let key = self.game_options.raw_key.text().trim().to_string();
        let value = self.game_options.raw_val.text().to_string();
        if let Err(e) = game_options::validate_entry(&key, &value) {
            self.game_options.set_note(scope, false, e);
            return;
        }
        let mode = self.game_options.raw_mode;
        self.mutate_entries(scope, true, |entries| {
            game_options::upsert_entry(entries, key.clone(), value.clone(), mode);
        });
        if let Some((edit, _, _)) = self
            .game_options
            .raw_editors
            .get_mut(&(scope.clone(), key.clone()))
        {
            edit.set(value);
        }
        self.game_options.raw_key.set(String::new());
        self.game_options.raw_val.set(String::new());
        self.game_options.set_note(
            scope,
            true,
            trf("Saved {key}.", &[("key", &key)]),
        );
    }

    fn pick_slider(&mut self, scope: &Scope, key: &str, kind: ControlKind, x: Pixels, save: bool) {
        let Some(bounds) = self
            .game_options
            .slider_areas
            .borrow()
            .get(&(scope.clone(), key.to_string()))
            .copied()
        else {
            return;
        };
        let width: f32 = bounds.size.width.into();
        if width <= 0. {
            return;
        }
        let frac = ((f32::from(x) - f32::from(bounds.origin.x)) / width).clamp(0., 1.);
        let raw_now = self
            .entries_for(scope)
            .into_iter()
            .find(|e| e.key == key)
            .map(|e| e.value)
            .unwrap_or_default();
        let Some(spec) = slider_spec(kind, &raw_now) else {
            return;
        };
        let steps = (spec.max - spec.min) / spec.step;
        let stepped = spec.min + (frac * steps as f32).round() as i32 * spec.step;
        let Ok(encoded) = encode_slider_value(kind, stepped) else {
            return;
        };
        let key_owned = key.to_string();
        self.mutate_entries(scope, save, |entries| {
            let mode = game_options::get_entry(entries, &key_owned)
                .map(|e| e.mode)
                .unwrap_or_default();
            game_options::upsert_entry(entries, key_owned, encoded, mode);
        });
    }

    fn game_options_editor(
        &mut self,
        scope: Scope,
        instance: Option<&instances::Instance>,
        cx: &mut Context<Self>,
    ) -> Div {
        let t = self.theme;
        let scan_home = instance.map(|i| i.home.as_path());
        self.game_options.ensure_scan(scan_home);

        let is_global = matches!(scope, Scope::Global);
        let title = if is_global {
            tr("Usual settings")
        } else {
            tr("Game settings")
        };
        let subtitle = if is_global {
            tr("Settings written to options.txt before Minecraft starts. Every instance uses these by default.")
        } else {
            tr("Set Minecraft options before this instance launches. Instance entries override your usual settings key by key.")
        };

        let mut head = div()
            .flex()
            .items_start()
            .justify_between()
            .gap(px(16.))
            .flex_wrap()
            .child(
                div()
                    .flex()
                    .flex_col()
                    .gap(px(4.))
                    .min_w_0()
                    .flex_1()
                    .child(
                        if is_global {
                            div()
                                .font_family(horizon_theme::glass::HEADING_FONT)
                                .font_weight(FontWeight::SEMIBOLD)
                                .text_size(px(20.))
                                .line_height(px(26.))
                                .text_color(white(ink::FULL))
                                .child(title)
                        } else {
                            ui::text(16., FontWeight::SEMIBOLD, ink::FULL).child(title)
                        },
                    )
                    .child(
                        ui::text(13., FontWeight::NORMAL, ink::NOTE)
                            .line_height(px(19.))
                            .child(subtitle),
                    ),
            );

        let mut actions = div().flex().flex_wrap().items_center().gap(px(8.));
        if let Some(inst) = instance.filter(|_| !is_global) {
            let home = inst.home.clone();
            let scope_load = scope.clone();
            actions = actions.child(
                ui::quiet_icon(
                    "game-opts-load-current",
                    Some("download-04"),
                    tr("Load this instance's current settings"),
                    36.,
                    &t,
                )
                .on_click(cx.listener(move |this, _: &ClickEvent, _, cx| {
                    this.load_from_instance_options(&scope_load, &home);
                    cx.notify();
                })),
            );
            let scope_copy = scope.clone();
            actions = actions.child(
                ui::quiet_icon(
                    "game-opts-copy-usual",
                    Some("copy-01"),
                    tr("Copy to usual settings"),
                    36.,
                    &t,
                )
                .on_click(cx.listener(move |this, _: &ClickEvent, _, cx| {
                    this.copy_instance_to_usual(&scope_copy);
                    cx.notify();
                })),
            );
        }
        actions = actions.child(self.add_option_button(&scope, instance, cx));
        head = head.child(actions);

        let mut body = div().flex().flex_col().gap(px(12.)).child(head);

        if let Some((ok, note)) = self.game_options.note_for(&scope).cloned() {
            body = body.child(
                ui::text(13., FontWeight::NORMAL, ink::FULL)
                    .px(px(12.))
                    .py(px(8.))
                    .rounded(px(radius::CONTROL))
                    .bg(if ok {
                        white(0.05)
                    } else {
                        gpui::rgba(horizon_theme::glass::DANGER_HOVER << 8 | 0x1f).into()
                    })
                    .text_color(if ok {
                        white(ink::STRONG)
                    } else {
                        ui::color(horizon_theme::glass::DANGER)
                    })
                    .child(note),
            );
        }

        let mut use_usual = true;
        if let Some(inst) = instance.filter(|_| !is_global) {
            use_usual = inst.game_options.use_usual;
            let inst_id = inst.id.clone();
            body = body.child(
                div()
                    .flex()
                    .items_center()
                    .justify_between()
                    .gap(px(16.))
                    .px(px(14.))
                    .py(px(10.))
                    .rounded(px(radius::ROW))
                    .bg(white(0.04))
                    .child(
                        div()
                            .flex()
                            .flex_col()
                            .gap(px(2.))
                            .min_w_0()
                            .flex_1()
                            .child(
                                ui::text(14., FontWeight::MEDIUM, ink::FULL)
                                    .child(tr("Use usual settings")),
                            )
                            .child(
                                ui::text(12.5, FontWeight::NORMAL, ink::QUIET).child(tr(
                                    "Apply your global usual settings first, then override with this instance's list.",
                                )),
                            ),
                    )
                    .child(
                        ui::toggle(
                            "game-opts-use-usual",
                            use_usual,
                            self.look.accent,
                            self.look.on_accent,
                            &t,
                        )
                        .on_click(cx.listener(move |this, _: &ClickEvent, _, cx| {
                            this.set_use_usual(&inst_id, !use_usual);
                            cx.notify();
                        })),
                    ),
            );
        }

        let entries = self.entries_for(&scope);
        if entries.is_empty() {
            let empty_msg = if is_global {
                tr("No usual settings yet. Click “Add setting” to set defaults like BGM 0% or FOV 90°.")
            } else if use_usual {
                tr("No overrides on this instance yet. Usual settings above apply unless you add an override here.")
            } else {
                tr("No settings on this instance yet. Click “Add setting” or load from options.txt.")
            };
            body = body.child(
                ui::text(13.5, FontWeight::NORMAL, ink::NOTE)
                    .px(px(14.))
                    .py(px(18.))
                    .rounded(px(radius::ROW))
                    .bg(white(0.03))
                    .child(empty_msg),
            );
        } else {
            let mut rows = div().flex().flex_col().gap(px(6.));
            for entry in &entries {
                rows = rows.child(self.option_row(&scope, entry, cx));
            }
            body = body.child(rows);
        }

        if !is_global && use_usual {
            let inherited: Vec<OptionEntry> = self
                .game_options
                .global
                .entries
                .iter()
                .filter(|g| game_options::get_entry(&entries, &g.key).is_none())
                .cloned()
                .collect();
            if !inherited.is_empty() {
                let mut list = div()
                    .flex()
                    .flex_col()
                    .gap(px(6.))
                    .mt(px(4.))
                    .child(
                        ui::text(12.5, FontWeight::MEDIUM, ink::DIM)
                            .px(px(4.))
                            .child(tr("Active from usual settings")),
                    );
                for g in inherited {
                    list = list.child(self.inherited_row(&scope, &g, cx));
                }
                body = body.child(list);
            }
        }

        body = body.child(self.other_options_panel(&scope, cx));
        body
    }

    fn add_option_button(
        &self,
        scope: &Scope,
        instance: Option<&instances::Instance>,
        cx: &mut Context<Self>,
    ) -> Div {
        let t = self.theme;
        let open = self.game_options.add_menu.as_ref() == Some(scope);
        let scope_tag = scope.tag();
        let scope_btn = scope.clone();
        let btn = ui::quiet_icon(
            format!("game-opts-add-{scope_tag}"),
            Some("plus-sign"),
            tr("Add setting"),
            36.,
            &t,
        )
        .on_click(cx.listener(move |this, event: &ClickEvent, _, cx| {
            if !this.closed_by_this_press(event) {
                this.game_options.lang_menu = None;
                this.game_options.add_menu = if this.game_options.add_menu.as_ref() == Some(&scope_btn)
                {
                    None
                } else {
                    Some(scope_btn.clone())
                };
            }
            cx.notify();
        }));
        let mut wrap = div().relative().child(btn);
        if open {
            let entries = self.entries_for(scope);
            let scanned_map: HashMap<&str, &str> = self
                .game_options
                .scanned_options
                .iter()
                .map(|(k, v)| (k.as_str(), v.as_str()))
                .collect();
            let has_instance = instance.is_some();
            let mut pop = ui::popover(320.)
                .id(SharedString::from(format!("game-opts-add-pop-{scope_tag}")))
                .occlude()
                .on_mouse_down_out(cx.listener(|this, event: &MouseDownEvent, _, cx| {
                    this.game_options.add_menu = None;
                    this.menu_closed_at = Some(event.position);
                    cx.notify();
                }));
            let mut list = div()
                .id(SharedString::from(format!("game-opts-add-scroll-{scope_tag}")))
                .flex()
                .flex_col()
                .gap(px(2.))
                .max_h(px(360.))
                .overflow_y_scroll();
            for cat in game_options::Category::ALL {
                list = list.child(
                    ui::text(11.5, FontWeight::MEDIUM, ink::DIM)
                        .px(px(10.))
                        .pt(px(8.))
                        .pb(px(2.))
                        .child(tr(cat.label())),
                );
                for def in game_options::FRIENDLY
                    .iter()
                    .filter(|d| d.category == cat)
                {
                    let added = game_options::get_entry(&entries, def.key).is_some();
                    let initial_val = if has_instance {
                        scanned_map
                            .get(def.key)
                            .copied()
                            .filter(|raw| game_options::validate_entry(def.key, raw).is_ok())
                            .unwrap_or(def.default_raw)
                    } else {
                        def.default_raw
                    }
                    .to_string();
                    let scope_pick = scope.clone();
                    let key = def.key.to_string();
                    list = list.child(
                        keyboard(
                            div()
                                .id(SharedString::from(format!(
                                    "game-opts-pick-{scope_tag}-{}",
                                    def.key
                                )))
                                .flex()
                                .items_center()
                                .justify_between()
                                .gap(px(8.))
                                .h(px(34.))
                                .px(px(10.))
                                .rounded(px(radius::CONTROL))
                                .cursor_pointer()
                                .hover(|s| s.bg(white(0.08)))
                                .child(
                                    ui::text(13.5, FontWeight::NORMAL, ink::FULL)
                                        .truncate()
                                        .child(tr(def.label)),
                                )
                                .children(added.then(|| {
                                    ui::icon("tick-02", 15., ui::color(self.look.accent_soft))
                                }))
                                .on_click(cx.listener(move |this, _: &ClickEvent, _, cx| {
                                    if !added {
                                        this.mutate_entries(&scope_pick, true, |entries| {
                                            game_options::upsert_entry(
                                                entries,
                                                key.clone(),
                                                initial_val.clone(),
                                                ApplyMode::Always,
                                            );
                                        });
                                    }
                                    this.game_options.add_menu = None;
                                    cx.notify();
                                })),
                            &t,
                        ),
                    );
                }
            }
            pop = pop.child(list);
            wrap = wrap.child(
                deferred(div().absolute().top(px(42.)).right_0().child(pop)).with_priority(2),
            );
        }
        wrap
    }

    fn option_row(
        &mut self,
        scope: &Scope,
        entry: &OptionEntry,
        cx: &mut Context<Self>,
    ) -> Div {
        let t = self.theme;
        let scope_tag = scope.tag();
        let def = game_options::find_friendly(&entry.key);
        let title = def.map_or_else(|| entry.key.clone(), |d| tr(d.label).to_string());
        let subtitle = def.map(|d| d.key).unwrap_or("options.txt");

        let control = match def {
            Some(d) => self.friendly_control(scope, d, &entry.value, cx),
            None => self.raw_value_control(scope, &entry.key, &entry.value, cx),
        };

        let mode_chip = self.mode_chip(scope, &entry.key, entry.mode, cx);
        let scope_rm = scope.clone();
        let key_rm = entry.key.clone();
        let remove = ui::icon_button(
            SharedString::from(format!("game-opt-rm-{scope_tag}-{}", entry.key)),
            "delete-02",
            32.,
            16.,
            radius::CONTROL,
            false,
            &t,
        )
        .on_click(cx.listener(move |this, _: &ClickEvent, _, cx| {
            this.mutate_entries(&scope_rm, true, |entries| {
                game_options::remove_entry(entries, &key_rm);
            });
            cx.notify();
        }));

        div()
            .flex()
            .items_center()
            .justify_between()
            .gap(px(12.))
            .px(px(14.))
            .py(px(10.))
            .rounded(px(radius::ROW))
            .bg(white(0.04))
            .child(
                div()
                    .flex()
                    .flex_col()
                    .min_w(px(150.))
                    .flex_1()
                    .child(
                        ui::text(14., FontWeight::MEDIUM, ink::FULL)
                            .truncate()
                            .child(title),
                    )
                    .child(
                        div()
                            .font_family(theme::MONO)
                            .text_size(px(11.5))
                            .text_color(white(ink::DIM))
                            .truncate()
                            .child(subtitle),
                    ),
            )
            .child(
                div()
                    .flex()
                    .flex_none()
                    .items_center()
                    .gap(px(10.))
                    .child(control)
                    .child(mode_chip)
                    .child(remove),
            )
    }

    fn inherited_row(
        &self,
        scope: &Scope,
        entry: &OptionEntry,
        cx: &mut Context<Self>,
    ) -> Div {
        let t = self.theme;
        let scope_tag = scope.tag();
        let def = game_options::find_friendly(&entry.key);
        let title = def.map_or_else(|| entry.key.clone(), |d| tr(d.label).to_string());
        let summary = def.map_or_else(
            || entry.value.clone(),
            |d| format_friendly_summary(d, &entry.value),
        );
        let mode_text = match entry.mode {
            ApplyMode::Always => tr("Every launch"),
            ApplyMode::Once => tr("First time only"),
        };
        let scope_ov = scope.clone();
        let cloned = entry.clone();
        div()
            .flex()
            .items_center()
            .justify_between()
            .gap(px(12.))
            .px(px(14.))
            .py(px(8.))
            .rounded(px(radius::ROW))
            .bg(white(0.025))
            .child(
                div()
                    .flex()
                    .flex_col()
                    .min_w_0()
                    .flex_1()
                    .child(
                        ui::text(13.5, FontWeight::MEDIUM, ink::BODY)
                            .truncate()
                            .child(title),
                    )
                    .child(
                        ui::text(12., FontWeight::NORMAL, ink::DIM)
                            .truncate()
                            .child(format!("{summary} · {mode_text}")),
                    ),
            )
            .child(
                ui::quiet(
                    SharedString::from(format!("game-opt-override-{scope_tag}-{}", entry.key)),
                    tr("Override"),
                    30.,
                    &t,
                )
                .on_click(cx.listener(move |this, _: &ClickEvent, _, cx| {
                    this.mutate_entries(&scope_ov, true, |entries| {
                        game_options::upsert_entry(
                            entries,
                            cloned.key.clone(),
                            cloned.value.clone(),
                            cloned.mode,
                        );
                    });
                    cx.notify();
                })),
            )
    }

    fn mode_chip(
        &self,
        scope: &Scope,
        key: &str,
        mode: ApplyMode,
        cx: &mut Context<Self>,
    ) -> Stateful<Div> {
        let t = self.theme;
        let scope_tag = scope.tag();
        let scope_click = scope.clone();
        let key_click = key.to_string();
        let accent = ui::color(self.look.accent);
        let soft = ui::color(self.look.accent_soft);
        let (label, bg, fg) = match mode {
            ApplyMode::Always => (tr("Every launch"), accent.opacity(0.22), soft),
            ApplyMode::Once => (
                tr("First time only"),
                white(fill::BUTTON),
                white(ink::BODY),
            ),
        };
        keyboard(
            div()
                .id(SharedString::from(format!("game-opt-mode-{scope_tag}-{key}")))
                .flex()
                .flex_none()
                .items_center()
                .h(px(30.))
                .px(px(10.))
                .rounded(px(radius::CONTROL))
                .bg(bg)
                .hover(|s| s.bg(white(fill::BUTTON_HOVER)))
                .cursor_pointer()
                .font_family(horizon_theme::glass::FONT)
                .text_size(px(12.5))
                .font_weight(FontWeight::MEDIUM)
                .text_color(fg)
                .child(label)
                .on_click(cx.listener(move |this, _: &ClickEvent, _, cx| {
                    let next = match mode {
                        ApplyMode::Always => ApplyMode::Once,
                        ApplyMode::Once => ApplyMode::Always,
                    };
                    this.mutate_entries(&scope_click, true, |entries| {
                        if let Some(e) = entries.iter_mut().find(|e| e.key == key_click) {
                            e.mode = next;
                        }
                    });
                    cx.notify();
                })),
            &t,
        )
    }

    fn friendly_control(
        &mut self,
        scope: &Scope,
        def: &'static FriendlyDef,
        raw: &str,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let t = self.theme;
        let scope_tag = scope.tag();
        if let Some(spec) = slider_spec(def.kind, raw) {
            return self.slider_control(scope, def.key, def.kind, spec, cx);
        }
        match def.kind {
            ControlKind::GuiScale => {
                let scale = game_options::decode_gui_scale(raw).unwrap_or(0);
                let label = if scale == 0 {
                    tr("Auto").to_string()
                } else {
                    scale.to_string()
                };
                let scope_minus = scope.clone();
                let scope_plus = scope.clone();
                div()
                    .flex()
                    .items_center()
                    .gap(px(6.))
                    .child(
                        ui::icon_button(
                            SharedString::from(format!("game-opt-dec-{scope_tag}-{}", def.key)),
                            "minus-sign",
                            28.,
                            14.,
                            radius::CONTROL,
                            false,
                            &t,
                        )
                        .on_click(cx.listener(move |this, _: &ClickEvent, _, cx| {
                            let next = scale.saturating_sub(1);
                            if let Ok(encoded) = game_options::encode_gui_scale(next) {
                                this.mutate_entries(&scope_minus, true, |entries| {
                                    let mode = game_options::get_entry(entries, def.key)
                                        .map(|e| e.mode)
                                        .unwrap_or_default();
                                    game_options::upsert_entry(
                                        entries,
                                        def.key.to_string(),
                                        encoded,
                                        mode,
                                    );
                                });
                            }
                            cx.notify();
                        })),
                    )
                    .child(
                        div()
                            .w(px(56.))
                            .h(px(28.))
                            .flex()
                            .items_center()
                            .justify_center()
                            .rounded(px(radius::CONTROL))
                            .bg(white(0.06))
                            .font_family(horizon_theme::glass::FONT)
                            .text_size(px(13.))
                            .font_weight(FontWeight::MEDIUM)
                            .text_color(white(ink::FULL))
                            .child(label),
                    )
                    .child(
                        ui::icon_button(
                            SharedString::from(format!("game-opt-inc-{scope_tag}-{}", def.key)),
                            "plus-sign",
                            28.,
                            14.,
                            radius::CONTROL,
                            false,
                            &t,
                        )
                        .on_click(cx.listener(move |this, _: &ClickEvent, _, cx| {
                            let next = (scale + 1).min(10);
                            if let Ok(encoded) = game_options::encode_gui_scale(next) {
                                this.mutate_entries(&scope_plus, true, |entries| {
                                    let mode = game_options::get_entry(entries, def.key)
                                        .map(|e| e.mode)
                                        .unwrap_or_default();
                                    game_options::upsert_entry(
                                        entries,
                                        def.key.to_string(),
                                        encoded,
                                        mode,
                                    );
                                });
                            }
                            cx.notify();
                        })),
                    )
                    .into_any_element()
            }
            ControlKind::Bool => {
                let on = game_options::decode_bool(raw).unwrap_or(true);
                let scope_tgl = scope.clone();
                ui::toggle(
                    SharedString::from(format!("game-opt-tgl-{scope_tag}-{}", def.key)),
                    on,
                    self.look.accent,
                    self.look.on_accent,
                    &t,
                )
                .on_click(cx.listener(move |this, _: &ClickEvent, _, cx| {
                    let encoded = game_options::encode_bool(!on);
                    this.mutate_entries(&scope_tgl, true, |entries| {
                        let mode = game_options::get_entry(entries, def.key)
                            .map(|e| e.mode)
                            .unwrap_or_default();
                        game_options::upsert_entry(entries, def.key.to_string(), encoded, mode);
                    });
                    cx.notify();
                }))
                .into_any_element()
            }
            ControlKind::Language => {
                let open = self.game_options.lang_menu.as_ref() == Some(scope);
                let scope_btn = scope.clone();
                let btn = keyboard(
                    div()
                        .id(SharedString::from(format!("game-opt-lang-{scope_tag}")))
                        .flex()
                        .items_center()
                        .justify_between()
                        .gap(px(8.))
                        .w(px(210.))
                        .h(px(32.))
                        .px(px(12.))
                        .rounded(px(radius::CONTROL))
                        .bg(white(fill::BUTTON))
                        .hover(|s| s.bg(white(fill::BUTTON_HOVER)))
                        .cursor_pointer()
                        .child(
                            ui::text(13., FontWeight::MEDIUM, ink::FULL)
                                .truncate()
                                .child(language_label(raw)),
                        )
                        .child(ui::icon("arrow-down-01", 15., white(ink::NOTE)))
                        .on_click(cx.listener(move |this, event: &ClickEvent, _, cx| {
                            if !this.closed_by_this_press(event) {
                                this.game_options.add_menu = None;
                                this.game_options.lang_menu =
                                    if this.game_options.lang_menu.as_ref() == Some(&scope_btn) {
                                        None
                                    } else {
                                        Some(scope_btn.clone())
                                    };
                            }
                            cx.notify();
                        })),
                    &t,
                );
                let mut wrap = div().relative().child(btn);
                if open {
                    let mut menu = ui::popover(250.)
                        .id(SharedString::from(format!("game-opt-lang-pop-{scope_tag}")))
                        .occlude()
                        .on_mouse_down_out(cx.listener(|this, event: &MouseDownEvent, _, cx| {
                            this.game_options.lang_menu = None;
                            this.menu_closed_at = Some(event.position);
                            cx.notify();
                        }));
                    let mut list = div()
                        .id(SharedString::from(format!("game-opt-lang-list-{scope_tag}")))
                        .flex()
                        .flex_col()
                        .gap(px(2.))
                        .max_h(px(260.))
                        .overflow_y_scroll();
                    for &(code, label) in game_options::LANGUAGES {
                        let chosen = raw == code;
                        let scope_pick = scope.clone();
                        list = list.child(
                            keyboard(
                                div()
                                    .id(SharedString::from(format!(
                                        "game-opt-lang-{scope_tag}-{code}"
                                    )))
                                    .flex()
                                    .items_center()
                                    .justify_between()
                                    .gap(px(8.))
                                    .h(px(32.))
                                    .px(px(10.))
                                    .rounded(px(radius::CONTROL))
                                    .cursor_pointer()
                                    .hover(|s| s.bg(white(0.08)))
                                    .child(ui::text(13., FontWeight::NORMAL, ink::FULL).child(label))
                                    .children(chosen.then(|| {
                                        ui::icon("tick-02", 15., ui::color(self.look.accent_soft))
                                    }))
                                    .on_click(cx.listener(move |this, _: &ClickEvent, _, cx| {
                                        this.mutate_entries(&scope_pick, true, |entries| {
                                            let mode = game_options::get_entry(entries, "lang")
                                                .map(|e| e.mode)
                                                .unwrap_or_default();
                                            game_options::upsert_entry(
                                                entries,
                                                "lang".to_string(),
                                                code.to_string(),
                                                mode,
                                            );
                                        });
                                        this.game_options.lang_menu = None;
                                        cx.notify();
                                    })),
                                &t,
                            ),
                        );
                    }
                    menu = menu.child(list);
                    wrap = wrap.child(
                        deferred(div().absolute().top(px(36.)).right_0().child(menu))
                            .with_priority(2),
                    );
                }
                wrap.into_any_element()
            }
            _ => div().into_any_element(),
        }
    }

    fn slider_control(
        &self,
        scope: &Scope,
        key: &'static str,
        kind: ControlKind,
        spec: SliderSpec,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let t = self.theme;
        let scope_tag = scope.tag();
        let areas = self.game_options.slider_areas.clone();
        let key_store = (scope.clone(), key.to_string());
        let frac = if spec.max > spec.min {
            ((spec.value - spec.min) as f32 / (spec.max - spec.min) as f32).clamp(0., 1.)
        } else {
            0.
        };
        let scope_down = scope.clone();
        let scope_move = scope.clone();
        let scope_up = scope.clone();
        let scope_up_out = scope.clone();
        let scope_dec = scope.clone();
        let scope_inc = scope.clone();
        let value_now = spec.value;
        let step = spec.step;

        let track = div()
            .id(SharedString::from(format!("game-opt-slider-{scope_tag}-{key}")))
            .relative()
            .flex()
            .items_center()
            .w(px(150.))
            .h(px(26.))
            .cursor_pointer()
            .child(
                canvas(
                    move |bounds, _, _| {
                        areas.borrow_mut().insert(key_store.clone(), bounds);
                    },
                    |_, _, _, _| {},
                )
                .absolute()
                .size_full(),
            )
            .child(
                div()
                    .relative()
                    .w_full()
                    .h(px(6.))
                    .rounded_full()
                    .bg(white(fill::ACTIVE))
                    .child(
                        div()
                            .h_full()
                            .w(gpui::relative(frac))
                            .rounded_full()
                            .bg(ui::color(self.look.accent_soft)),
                    )
                    .child(
                        div()
                            .absolute()
                            .top(px(-3.))
                            .left(gpui::relative(frac))
                            .ml(px(-6.))
                            .size(px(12.))
                            .rounded_full()
                            .bg(gpui::white()),
                    ),
            )
            .on_mouse_down(
                MouseButton::Left,
                cx.listener(move |this, ev: &MouseDownEvent, _, cx| {
                    this.game_options.slider_drag = Some((scope_down.clone(), key.to_string()));
                    this.pick_slider(&scope_down, key, kind, ev.position.x, false);
                    cx.notify();
                }),
            )
            .on_mouse_move(cx.listener(move |this, ev: &MouseMoveEvent, _, cx| {
                if ev.pressed_button == Some(MouseButton::Left)
                    && this.game_options.slider_drag.as_ref()
                        == Some(&(scope_move.clone(), key.to_string()))
                {
                    this.pick_slider(&scope_move, key, kind, ev.position.x, false);
                    cx.notify();
                }
            }))
            .on_mouse_up(
                MouseButton::Left,
                cx.listener(move |this, ev: &MouseUpEvent, _, cx| {
                    if this.game_options.slider_drag.take().is_some() {
                        this.pick_slider(&scope_up, key, kind, ev.position.x, true);
                        cx.notify();
                    }
                }),
            )
            .on_mouse_up_out(
                MouseButton::Left,
                cx.listener(move |this, _: &MouseUpEvent, _, cx| {
                    if this.game_options.slider_drag.take().is_some() {
                        this.save_scope_now(&scope_up_out);
                        cx.notify();
                    }
                }),
            );

        let max_allowed = if matches!(kind, ControlKind::RenderDistance) {
            128
        } else {
            spec.max
        };
        let dec_val = (value_now - step).clamp(spec.min, max_allowed);
        let inc_val = (value_now + step).clamp(spec.min, max_allowed);

        div()
            .flex()
            .items_center()
            .gap(px(6.))
            .child(
                ui::icon_button(
                    SharedString::from(format!("game-opt-sdec-{scope_tag}-{key}")),
                    "minus-sign",
                    24.,
                    12.,
                    radius::CONTROL,
                    false,
                    &t,
                )
                .on_click(cx.listener(move |this, _: &ClickEvent, _, cx| {
                    if let Ok(encoded) = encode_slider_value(kind, dec_val) {
                        this.mutate_entries(&scope_dec, true, |entries| {
                            let mode = game_options::get_entry(entries, key)
                                .map(|e| e.mode)
                                .unwrap_or_default();
                            game_options::upsert_entry(entries, key.to_string(), encoded.clone(), mode);
                        });
                    }
                    cx.notify();
                })),
            )
            .child(track)
            .child(
                ui::icon_button(
                    SharedString::from(format!("game-opt-sinc-{scope_tag}-{key}")),
                    "plus-sign",
                    24.,
                    12.,
                    radius::CONTROL,
                    false,
                    &t,
                )
                .on_click(cx.listener(move |this, _: &ClickEvent, _, cx| {
                    if let Ok(encoded) = encode_slider_value(kind, inc_val) {
                        this.mutate_entries(&scope_inc, true, |entries| {
                            let mode = game_options::get_entry(entries, key)
                                .map(|e| e.mode)
                                .unwrap_or_default();
                            game_options::upsert_entry(entries, key.to_string(), encoded.clone(), mode);
                        });
                    }
                    cx.notify();
                })),
            )
            .child(
                ui::text(13., FontWeight::MEDIUM, ink::STRONG)
                    .w(px(72.))
                    .text_right()
                    .child(spec.display),
            )
            .into_any_element()
    }

    fn raw_value_control(
        &mut self,
        scope: &Scope,
        key: &str,
        value: &str,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let scope_tag = scope.tag();
        let editor_key = (scope.clone(), key.to_string());
        if !self.game_options.raw_editors.contains_key(&editor_key) {
            self.game_options.raw_editors.insert(
                editor_key.clone(),
                (
                    LineEdit::new(value.to_string(), 512, 22),
                    cx.focus_handle().tab_index(0).tab_stop(true),
                    false,
                ),
            );
        }
        let (edit, focus, focused) = self
            .game_options
            .raw_editors
            .get_mut(&editor_key)
            .expect("inserted above");
        if !*focused && edit.text() != value {
            edit.set(value.to_string());
        }
        let is_focused = *focused;
        let focus = focus.clone();
        let key_owned = key.to_string();
        let scope_click = scope.clone();
        let scope_key = scope.clone();
        let ek_click = editor_key.clone();
        let ek_key = editor_key.clone();
        let ek_mouse = editor_key.clone();
        let t = self.theme;
        let invalid = game_options::validate_raw_value(edit.text()).is_err();

        let field = ui::field(is_focused, invalid)
            .id(SharedString::from(format!("game-opt-raw-{scope_tag}-{key}")))
            .track_focus(&focus)
            .w(px(210.))
            .h(px(32.))
            .px(px(10.))
            .rounded(px(radius::CONTROL))
            .font_family(theme::MONO)
            .text_size(px(12.5))
            .cursor_text()
            .child(line_edit::content(
                edit,
                &t,
                is_focused,
                false,
                false,
                "",
            ))
            .on_click(cx.listener(move |this, _: &ClickEvent, window, cx| {
                let _ = &scope_click;
                if let Some((_, h, _)) = this.game_options.raw_editors.get(&ek_click) {
                    window.focus(h, cx);
                }
                cx.notify();
            }))
            .on_key_down(cx.listener(move |this, ev: &KeyDownEvent, window, cx| {
                let Some((edit, _, _)) = this.game_options.raw_editors.get_mut(&ek_key) else {
                    return;
                };
                match line_edit::handle(edit, ev, cx) {
                    Key::Pass => return,
                    Key::Escape => window.blur(cx),
                    Key::Edited | Key::Enter | Key::Paste | Key::Cut(_) => {
                        let new_val = edit.text().to_string();
                        if game_options::validate_raw_value(&new_val).is_ok() {
                            this.mutate_entries(&scope_key, true, |entries| {
                                let mode = game_options::get_entry(entries, &key_owned)
                                    .map(|e| e.mode)
                                    .unwrap_or_default();
                                game_options::upsert_entry(
                                    entries,
                                    key_owned.clone(),
                                    new_val.clone(),
                                    mode,
                                );
                            });
                        }
                    }
                    _ => {}
                }
                cx.stop_propagation();
                cx.notify();
            }));

        line_edit::mouse(field, cx, move |this| {
            &mut this
                .game_options
                .raw_editors
                .get_mut(&ek_mouse)
                .expect("raw editor exists")
                .0
        })
        .into_any_element()
    }

    fn other_options_panel(&mut self, scope: &Scope, cx: &mut Context<Self>) -> Div {
        let t = self.theme;
        let scope_tag = scope.tag();
        let query = self.game_options.search.text().trim().to_lowercase();
        let matches: Vec<(String, String)> = self
            .game_options
            .scanned_options
            .iter()
            .filter(|(k, _)| query.is_empty() || k.to_lowercase().contains(&query))
            .take(40)
            .cloned()
            .collect();

        let search_focused = self.game_options.search_focused;
        let search_focus = self.game_options.search_focus.clone();
        let search_field = ui::field(search_focused, false)
            .id(SharedString::from(format!("game-opts-search-{scope_tag}")))
            .track_focus(&search_focus)
            .w_full()
            .h(px(36.))
            .pl(px(36.))
            .pr(px(12.))
            .rounded(px(radius::CONTROL))
            .cursor_text()
            .child(line_edit::content(
                &self.game_options.search,
                &t,
                search_focused,
                false,
                false,
                tr("Search option keys in options.txt (e.g. renderClouds, key_key.jump)…"),
            ))
            .on_click(cx.listener(|this, _: &ClickEvent, window, cx| {
                window.focus(&this.game_options.search_focus, cx);
                cx.notify();
            }))
            .on_key_down(cx.listener(|this, ev: &KeyDownEvent, window, cx| {
                match line_edit::handle(&mut this.game_options.search, ev, cx) {
                    Key::Pass => return,
                    Key::Escape => window.blur(cx),
                    _ => {}
                }
                cx.stop_propagation();
                cx.notify();
            }));
        let search_field = line_edit::mouse(search_field, cx, |this| &mut this.game_options.search);
        let search_wrap = div()
            .relative()
            .child(search_field)
            .child(
                div()
                    .absolute()
                    .top_0()
                    .left(px(12.))
                    .h_full()
                    .flex()
                    .items_center()
                    .child(ui::icon("search-01", 15., white(ink::DIM))),
            );

        let key_focused = self.game_options.raw_key_focused;
        let key_focus = self.game_options.raw_key_focus.clone();
        let key_field = ui::field(key_focused, false)
            .id(SharedString::from(format!("game-opts-raw-key-{scope_tag}")))
            .track_focus(&key_focus)
            .w(px(220.))
            .h(px(36.))
            .px(px(12.))
            .rounded(px(radius::CONTROL))
            .font_family(theme::MONO)
            .text_size(px(13.))
            .cursor_text()
            .child(line_edit::content(
                &self.game_options.raw_key,
                &t,
                key_focused,
                false,
                false,
                tr("Option key"),
            ))
            .on_click(cx.listener(|this, _: &ClickEvent, window, cx| {
                window.focus(&this.game_options.raw_key_focus, cx);
                cx.notify();
            }))
            .on_key_down(cx.listener({
                let scope = scope.clone();
                move |this, ev: &KeyDownEvent, window, cx| {
                    match line_edit::handle(&mut this.game_options.raw_key, ev, cx) {
                        Key::Pass => return,
                        Key::Escape => window.blur(cx),
                        Key::Enter => this.commit_raw_add(&scope),
                        _ => {}
                    }
                    cx.stop_propagation();
                    cx.notify();
                }
            }));
        let key_field = line_edit::mouse(key_field, cx, |this| &mut this.game_options.raw_key);

        let val_focused = self.game_options.raw_val_focused;
        let val_focus = self.game_options.raw_val_focus.clone();
        let val_field = ui::field(val_focused, false)
            .id(SharedString::from(format!("game-opts-raw-val-{scope_tag}")))
            .track_focus(&val_focus)
            .flex_1()
            .min_w(px(140.))
            .h(px(36.))
            .px(px(12.))
            .rounded(px(radius::CONTROL))
            .font_family(theme::MONO)
            .text_size(px(13.))
            .cursor_text()
            .child(line_edit::content(
                &self.game_options.raw_val,
                &t,
                val_focused,
                false,
                false,
                tr("Raw value"),
            ))
            .on_click(cx.listener(|this, _: &ClickEvent, window, cx| {
                window.focus(&this.game_options.raw_val_focus, cx);
                cx.notify();
            }))
            .on_key_down(cx.listener({
                let scope = scope.clone();
                move |this, ev: &KeyDownEvent, window, cx| {
                    match line_edit::handle(&mut this.game_options.raw_val, ev, cx) {
                        Key::Pass => return,
                        Key::Escape => window.blur(cx),
                        Key::Enter => this.commit_raw_add(&scope),
                        _ => {}
                    }
                    cx.stop_propagation();
                    cx.notify();
                }
            }));
        let val_field = line_edit::mouse(val_field, cx, |this| &mut this.game_options.raw_val);

        let raw_mode = self.game_options.raw_mode;
        let accent = ui::color(self.look.accent);
        let soft = ui::color(self.look.accent_soft);
        let (mode_label, mode_bg, mode_fg) = match raw_mode {
            ApplyMode::Always => (tr("Every launch"), accent.opacity(0.22), soft),
            ApplyMode::Once => (
                tr("First time only"),
                white(fill::BUTTON),
                white(ink::BODY),
            ),
        };
        let raw_mode_btn = keyboard(
            div()
                .id(SharedString::from(format!("game-opts-raw-mode-{scope_tag}")))
                .flex()
                .flex_none()
                .items_center()
                .h(px(36.))
                .px(px(10.))
                .rounded(px(radius::CONTROL))
                .bg(mode_bg)
                .hover(|s| s.bg(white(fill::BUTTON_HOVER)))
                .cursor_pointer()
                .font_family(horizon_theme::glass::FONT)
                .text_size(px(12.5))
                .font_weight(FontWeight::MEDIUM)
                .text_color(mode_fg)
                .child(mode_label)
                .on_click(cx.listener(|this, _: &ClickEvent, _, cx| {
                    this.game_options.raw_mode = match this.game_options.raw_mode {
                        ApplyMode::Always => ApplyMode::Once,
                        ApplyMode::Once => ApplyMode::Always,
                    };
                    cx.notify();
                })),
            &t,
        );

        let scope_add = scope.clone();
        let add_btn = ui::quiet(
            SharedString::from(format!("game-opts-raw-add-{scope_tag}")),
            tr("Add"),
            36.,
            &t,
        )
        .on_click(cx.listener(move |this, _: &ClickEvent, _, cx| {
            this.commit_raw_add(&scope_add);
            cx.notify();
        }));

        let keys_box = if self.game_options.scanned_options.is_empty() {
            div()
                .child(
                    ui::text(12.5, FontWeight::NORMAL, ink::DIM).child(tr(
                        "No keys found in options.txt yet. Launch the game once or type any key below.",
                    )),
                )
                .into_any_element()
        } else if matches.is_empty() {
            div()
                .child(
                    ui::text(12.5, FontWeight::NORMAL, ink::DIM)
                        .child(tr("No option key matches that search.")),
                )
                .into_any_element()
        } else {
            div()
                .id(SharedString::from(format!("game-opts-keys-{scope_tag}")))
                .flex()
                .flex_col()
                .max_h(px(170.))
                .overflow_y_scroll()
                .p(px(4.))
                .rounded(px(radius::ROW))
                .bg(white(0.03))
                .children(matches.into_iter().map(|(k, v)| {
                    let k_pick = k.clone();
                    let v_pick = v.clone();
                    keyboard(
                        div()
                            .id(SharedString::from(format!("game-opts-k-{scope_tag}-{k}")))
                            .flex()
                            .items_center()
                            .justify_between()
                            .gap(px(12.))
                            .px(px(10.))
                            .py(px(6.))
                            .rounded(px(radius::CONTROL))
                            .cursor_pointer()
                            .hover(|s| s.bg(white(fill::HOVER)))
                            .child(
                                div()
                                    .font_family(theme::MONO)
                                    .text_size(px(12.5))
                                    .text_color(white(ink::FULL))
                                    .truncate()
                                    .child(k),
                            )
                            .child(
                                div()
                                    .font_family(theme::MONO)
                                    .text_size(px(12.))
                                    .text_color(white(ink::DIM))
                                    .truncate()
                                    .max_w(px(200.))
                                    .child(v),
                            )
                            .on_click(cx.listener(move |this, _: &ClickEvent, window, cx| {
                                this.game_options.raw_key.set(k_pick.clone());
                                this.game_options.raw_val.set(v_pick.clone());
                                window.focus(&this.game_options.raw_val_focus, cx);
                                cx.notify();
                            })),
                        &t,
                    )
                }))
                .into_any_element()
        };

        div()
            .flex()
            .flex_col()
            .gap(px(10.))
            .mt(px(4.))
            .p(px(14.))
            .rounded(px(radius::POPOVER))
            .bg(white(0.03))
            .child(
                div()
                    .flex()
                    .flex_col()
                    .gap(px(2.))
                    .child(
                        ui::text(14., FontWeight::SEMIBOLD, ink::FULL)
                            .child(tr("Other options")),
                    )
                    .child(
                        ui::text(12.5, FontWeight::NORMAL, ink::QUIET).child(tr(
                            "Pick any key from options.txt or enter a key and raw value directly.",
                        )),
                    ),
            )
            .child(search_wrap)
            .child(keys_box)
            .child(
                div()
                    .flex()
                    .flex_wrap()
                    .items_center()
                    .gap(px(8.))
                    .child(key_field)
                    .child(val_field)
                    .child(raw_mode_btn)
                    .child(add_btn),
            )
    }
}
