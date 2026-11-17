//! Settings' Language row (`horizon_launcher::i18n`): a dropdown of Japanese and English, each
//! named in itself.
use super::*;
use horizon_launcher::i18n::Language;

impl Launcher {
    pub(super) fn language_row(&self, cx: &mut Context<Self>) -> Div {
        use horizon_theme::glass::{ink, radius};
        let t = self.theme;
        let mut wrap = div().relative().child(
            glass_ui::quiet_icon(
                "language-picker",
                None,
                self.settings.language().name(),
                36.,
                &t,
            )
            .relative()
            .child(glass_ui::icon(
                "arrow-down-01",
                16.,
                glass_ui::white(ink::NOTE),
            ))
            .on_click(cx.listener(|this, _: &ClickEvent, _, cx| {
                this.language_menu = !this.language_menu;
                cx.notify();
            })),
        );
        if self.language_menu {
            let mut menu = glass_ui::popover(220.)
                .id("language-menu")
                .absolute()
                .top(px(40.))
                .right_0()
                .occlude();
            for choice in Language::ALL {
                let chosen = choice == self.settings.language();
                let id = choice.code();
                let row_id = SharedString::from(format!("language-pick-{id}"));
                menu = menu.child(
                    div()
                        .id(row_id)
                        .relative()
                        .flex()
                        .items_center()
                        .justify_between()
                        .gap(px(8.))
                        .h(px(40.))
                        .px(px(12.))
                        .rounded(px(radius::CONTROL))
                        .cursor_pointer()
                        .hover(|s| s.bg(glass_ui::white(0.07)))
                        .child(
                            glass_ui::text(14., FontWeight::NORMAL, ink::FULL)
                                .relative()
                                .child(choice.name()),
                        )
                        .when(chosen, |d| {
                            d.child(
                                glass_ui::icon(
                                    "tick-02",
                                    16.,
                                    glass_ui::accent_soft(self.look.accent_soft),
                                )
                                .relative(),
                            )
                        })
                        .on_click(cx.listener(move |this, _: &ClickEvent, _, cx| {
                            this.settings.language = Some(choice);
                            this.save_settings();
                            i18n::set(this.settings.language());
                            this.language_menu = false;
                            cx.notify();
                        })),
                );
            }
            wrap = wrap.child(gpui::deferred(menu).with_priority(1));
        }
        index_row(0, tr("Language"), None, Some(wrap.into_any_element()), &t)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_dropdown_offers_japanese_then_english_each_named_in_itself() {
        assert_eq!(Language::ALL.map(Language::name), ["日本語", "English"]);
    }
}
