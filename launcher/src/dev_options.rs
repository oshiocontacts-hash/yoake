//! Developer options: a Settings section nothing in the window points to. Seven clicks on the
//! sidebar's "Launcher x.y.z" reveal it for this run; it stays shown while one of its options is
//! on. Offline names remain unavailable in owner-required builds; testing unsigned local DLL mods
//! requires a separate confirmation and is available in every build. WASM development has its own
//! confirmation and never enables native code; WASM modules still receive their declared permissions.
use super::*;

/// Clicks on the launcher's version that reveal the section.
const CLICKS: u8 = 7;
const WASM_DEVELOPER_NOTICE: &str = "For testing your own unsigned .yak mods. WASM runs in a sandbox, but the mod's declared permissions, including service.commands for server modules, still apply. Do not use this for mods from others.";

#[derive(Clone, Copy)]
enum WasmToggleAction {
    Click,
    Confirm,
    Cancel,
}

fn update_wasm_toggle(enabled: &mut bool, confirmation: &mut bool, action: WasmToggleAction) -> bool {
    match action {
        WasmToggleAction::Click if *enabled => {
            *enabled = false;
            *confirmation = false;
            true
        }
        WasmToggleAction::Click => {
            *confirmation = true;
            false
        }
        WasmToggleAction::Confirm => {
            *enabled = true;
            *confirmation = false;
            true
        }
        WasmToggleAction::Cancel => {
            *confirmation = false;
            false
        }
    }
}

impl Launcher {
    /// A click on the sidebar's launcher version; the last one opens Settings on the section.
    pub(super) fn version_clicked(&mut self) {
        if self.dev_options {
            return;
        }
        self.dev_clicks += 1;
        if self.dev_clicks >= CLICKS {
            self.dev_options = true;
            self.go(Page::Settings);
        }
    }

    /// The section, when revealed or when one of its options is on.
    pub(super) fn dev_options_section(&self, cx: &mut Context<Self>) -> Option<Div> {
        if !self.dev_options
            && !self.settings.dev_unsigned_dll
            && !self.settings.dev_unsigned_wasm
            && (account::owner_required() || !self.settings.dev_offline)
        {
            return None;
        }
        let t = self.theme;
        let mut section = glass_body();
        if !account::owner_required() {
            let toggle = switch("dev-offline", self.settings.dev_offline, &t).on_click(
                cx.listener(|this, _: &ClickEvent, _, cx| {
                    if !account::owner_required() {
                        this.settings.dev_offline = !this.settings.dev_offline;
                        this.save_settings();
                    }
                    cx.notify();
                }),
            );
            section = section.child(index_row(
                0,
                tr("Offline names without an account"),
                Some(
                    div()
                        .child(tr(
                            "Offline names play without a Microsoft account that owns the game. \
                             Minecraft still comes from your own install.",
                        ))
                        .into_any_element(),
                ),
                Some(toggle.into_any_element()),
                &t,
            ));
        }
        let toggle = switch("dev-unsigned-dll", self.settings.dev_unsigned_dll, &t).on_click(
            cx.listener(|this, _: &ClickEvent, _, cx| {
                if this.settings.dev_unsigned_dll {
                    this.settings.dev_unsigned_dll = false;
                    this.save_settings();
                } else {
                    this.dev_unsigned_dll_confirmation = true;
                }
                cx.notify();
            }),
        );
        section = section.child(index_row(
            0,
            tr("Test your own DLL mods (developer mode)"),
            Some(
                div()
                    .child(tr("Lets you install and run unsigned .yak files that contain DLLs from your own PC. Do not use it for mods you got from someone else."))
                    .into_any_element(),
            ),
            Some(toggle.into_any_element()),
            &t,
        ));
        let toggle = switch("dev-unsigned-wasm", self.settings.dev_unsigned_wasm, &t).on_click(
            cx.listener(|this, _: &ClickEvent, _, cx| {
                if update_wasm_toggle(
                    &mut this.settings.dev_unsigned_wasm,
                    &mut this.dev_unsigned_wasm_confirmation,
                    WasmToggleAction::Click,
                ) {
                    this.save_settings();
                }
                cx.notify();
            }),
        );
        section = section.child(index_row(
            0,
            tr("Test your own WASM mods"),
            Some(div().child(tr(WASM_DEVELOPER_NOTICE)).into_any_element()),
            Some(toggle.into_any_element()),
            &t,
        ));
        if self.settings.dev_unsigned_wasm {
            section = section.child(self.dev_folder_row(cx));
        }
        if self.dev_unsigned_dll_confirmation {
            section = section.child(
                div()
                    .relative()
                    .flex()
                    .flex_col()
                    .gap(px(8.))
                    .p(px(12.))
                    .child(glass_ui::text(13., FontWeight::NORMAL, 1.).relative().text_color(glass_ui::color(horizon_theme::glass::WARN)).child(tr(
                        "Lets you install and run unsigned .yak files that contain DLLs from your own PC. Do not use it for mods you got from someone else.",
                    )))
                    .child(
                        div()
                            .flex()
                            .gap(px(8.))
                            .child(
                                button("confirm-unsigned-dll", "Turn on", Kind::Primary, &t)
                                    .on_click(cx.listener(|this, _: &ClickEvent, _, cx| {
                                        this.settings.dev_unsigned_dll = true;
                                        this.dev_unsigned_dll_confirmation = false;
                                        this.save_settings();
                                        cx.notify();
                                    })),
                            )
                            .child(
                                button("cancel-unsigned-dll", "Cancel activation", Kind::Secondary, &t)
                                    .on_click(cx.listener(|this, _: &ClickEvent, _, cx| {
                                        this.dev_unsigned_dll_confirmation = false;
                                        cx.notify();
                                    })),
                            ),
                    ),
            );
        }
        if self.dev_unsigned_wasm_confirmation {
            section = section.child(
                div()
                    .relative()
                    .flex()
                    .flex_col()
                    .gap(px(8.))
                    .p(px(12.))
                    .child(
                        glass_ui::text(13., FontWeight::NORMAL, 1.)
                            .relative()
                            .text_color(glass_ui::color(horizon_theme::glass::WARN))
                            .child(tr(WASM_DEVELOPER_NOTICE)),
                    )
                    .child(
                        div()
                            .flex()
                            .gap(px(8.))
                            .child(
                                button("confirm-unsigned-wasm", "Turn on", Kind::Primary, &t)
                                    .on_click(cx.listener(|this, _: &ClickEvent, _, cx| {
                                        if update_wasm_toggle(
                                            &mut this.settings.dev_unsigned_wasm,
                                            &mut this.dev_unsigned_wasm_confirmation,
                                            WasmToggleAction::Confirm,
                                        ) {
                                            this.save_settings();
                                        }
                                        cx.notify();
                                    })),
                            )
                            .child(
                                button("cancel-unsigned-wasm", "Cancel activation", Kind::Secondary, &t)
                                    .on_click(cx.listener(|this, _: &ClickEvent, _, cx| {
                                        update_wasm_toggle(
                                            &mut this.settings.dev_unsigned_wasm,
                                            &mut this.dev_unsigned_wasm_confirmation,
                                            WasmToggleAction::Cancel,
                                        );
                                        cx.notify();
                                    })),
                            ),
                    ),
            );
        }
        Some(glass_section(
            "Developer options",
            "computer-terminal-01",
            None,
            section,
        ))
    }
}

impl Launcher {
    /// "Full-access dev folder": full-access .yak files built there are installed, allowed and reloaded in the running
    /// game automatically (only while "Test your own WASM mods" is on).
    fn dev_folder_row(&self, cx: &mut Context<Self>) -> Div {
        let t = self.theme;
        let folder = self.settings.full_access_dev_folder.clone();
        let mut description = div().flex().flex_col().gap(px(4.)).child(tr(
            "Full-access mods you build into this folder are installed, allowed and reloaded in the running game automatically. Choose a folder only you write to.",
        ));
        description = description.child(match &folder {
            Some(path) => trf("Folder: {path}", &[("path", &path.display().to_string())]),
            None => tr("No folder chosen.").to_owned(),
        });
        if let Some(error) = &self.dev_folder_error {
            description = description.child(
                div().text_color(glass_ui::color(horizon_theme::glass::WARN)).child(error.clone()),
            );
        }
        let mut controls = div().flex().gap(px(8.)).child(
            button("dev-folder-choose", "Choose folder…", Kind::Secondary, &t).on_click(cx.listener(
                |this, _: &ClickEvent, _, cx| {
                    this.choose_dev_folder(cx);
                    cx.notify();
                },
            )),
        );
        if folder.is_some() {
            controls = controls.child(
                button("dev-folder-clear", "Clear", Kind::Secondary, &t).on_click(cx.listener(
                    |this, _: &ClickEvent, _, cx| {
                        this.apply_dev_folder(None);
                        cx.notify();
                    },
                )),
            );
        }
        index_row(
            0,
            tr("Full-access dev folder"),
            Some(description.into_any_element()),
            Some(controls.into_any_element()),
            &t,
        )
    }

    fn choose_dev_folder(&mut self, cx: &mut Context<Self>) {
        let paths = cx.prompt_for_paths(PathPromptOptions {
            files: false,
            directories: true,
            multiple: false,
            prompt: Some(tr("Choose folder…").into()),
        });
        cx.spawn(async move |this, cx| {
            if let Ok(Ok(Some(paths))) = paths.await {
                if let Some(path) = paths.into_iter().next() {
                    let _ = this.update(cx, |this, cx| {
                        this.apply_dev_folder(Some(path));
                        cx.notify();
                    });
                }
            }
        })
        .detach();
    }

    /// Stores the folder through the backend (it validates and writes settings.json), then takes the stored value into
    /// this window's settings so a later save keeps it.
    fn apply_dev_folder(&mut self, path: Option<std::path::PathBuf>) {
        match horizon_launcher::full_access::set_dev_folder(path) {
            Ok(()) => {
                self.dev_folder_error = None;
                self.settings.full_access_dev_folder =
                    backend::Settings::load().full_access_dev_folder;
            }
            Err(reason) => {
                self.dev_folder_error =
                    Some(trf("This folder cannot be used: {reason}", &[("reason", &reason)]));
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn wasm_unsigned_toggle_requires_confirmation_and_can_be_cancelled_or_disabled() {
        let (mut enabled, mut confirmation) = (false, false);
        assert!(!update_wasm_toggle(&mut enabled, &mut confirmation, WasmToggleAction::Click));
        assert!(!enabled);
        assert!(confirmation);

        assert!(!update_wasm_toggle(&mut enabled, &mut confirmation, WasmToggleAction::Cancel));
        assert!(!enabled);
        assert!(!confirmation);

        assert!(!update_wasm_toggle(&mut enabled, &mut confirmation, WasmToggleAction::Click));
        assert!(confirmation);
        assert!(update_wasm_toggle(&mut enabled, &mut confirmation, WasmToggleAction::Confirm));
        assert!(enabled);
        assert!(!confirmation);

        assert!(update_wasm_toggle(&mut enabled, &mut confirmation, WasmToggleAction::Click));
        assert!(!enabled);
        assert!(!confirmation);
    }
}
