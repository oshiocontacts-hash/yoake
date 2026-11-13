//! Developer options: a Settings section nothing in the window points to. Seven clicks on the
//! sidebar's "Launcher x.y.z" reveal it for this run; it stays shown while one of its options is
//! on. Offline names remain unavailable in owner-required builds; testing unsigned local DLL mods
//! requires a separate confirmation and is available in every build.
use super::*;

/// Clicks on the launcher's version that reveal the section.
const CLICKS: u8 = 7;

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
            && (account::owner_required() || !self.settings.dev_offline)
        {
            return None;
        }
        let t = self.theme;
        let mut section = div().flex().flex_col().child(
            section_title("Developer options", "terminal", &t)
                .mt(px(20.))
                .mb(px(12.)),
        );
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
        if self.dev_unsigned_dll_confirmation {
            section = section.child(
                div()
                    .flex()
                    .flex_col()
                    .gap(px(8.))
                    .p(px(12.))
                    .child(shell::text(theme::night::UI, shell::c(t.night.warn)).child(tr(
                        "Lets you install and run unsigned .yak files that contain DLLs from your own PC. Do not use it for mods you got from someone else.",
                    )))
                    .child(
                        div()
                            .flex()
                            .gap(px(8.))
                            .child(
                                shell::action("confirm-unsigned-dll", shell::Action::Raised, &t)
                                    .child(shell::label(tr("Turn on")))
                                    .on_click(cx.listener(|this, _: &ClickEvent, _, cx| {
                                        this.settings.dev_unsigned_dll = true;
                                        this.dev_unsigned_dll_confirmation = false;
                                        this.save_settings();
                                        cx.notify();
                                    })),
                            )
                            .child(
                                shell::action("cancel-unsigned-dll", shell::Action::Quiet, &t)
                                    .child(shell::label(tr("Cancel activation")))
                                    .on_click(cx.listener(|this, _: &ClickEvent, _, cx| {
                                        this.dev_unsigned_dll_confirmation = false;
                                        cx.notify();
                                    })),
                            ),
                    ),
            );
        }
        Some(section)
    }
}
