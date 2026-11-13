use super::*;
use crate::plate::alpha;
use crate::shell::{c, Action};
use gpui::ExternalPaths;
use horizon_mods::yak::Manifest;
use theme::night;

fn name(manifest: &Manifest) -> &str {
    if i18n::current() == i18n::Language::Japanese {
        manifest.name_ja.as_deref().unwrap_or(&manifest.name)
    } else {
        &manifest.name
    }
}

impl Launcher {
    pub(crate) fn yak_mods_tab(&mut self, cx: &mut Context<Self>) -> (Div, Div) {
        let theme = self.theme;
        let night_theme = theme.night;
        let busy = self.yak_busy();
        let actions = div()
            .flex()
            .items_center()
            .gap(px(8.))
            .child(
                shell::action("yak-add", Action::Raised, &theme)
                    .child(shell::label(tr("Add a .yak file…")))
                    .when(!busy, |button| {
                        button.on_click(cx.listener(|this, _: &ClickEvent, _, cx| {
                            this.choose_local_yak(cx);
                            cx.notify();
                        }))
                    })
                    .when(busy, |button| button.opacity(theme::DISABLED)),
            )
            .child(
                shell::action("yak-refresh", Action::Quiet, &theme)
                    .child(shell::label(tr("Refresh")))
                    .on_click(cx.listener(|this, _: &ClickEvent, _, cx| {
                        this.yak_mods.error = None;
                        this.refresh_yak_catalog();
                        cx.notify();
                    })),
            );
        let mut content = div().flex().flex_col().gap(px(14.)).child(
            shell::text(night::UI, c(night_theme.ink))
                .font_weight(FontWeight::SEMIBOLD)
                .child(tr("Yoake mods")),
        );
        if let Some(error) = &self.yak_mods.error {
            content = content.child(shell::text(night::UI, theme.danger).child(error.clone()));
        }
        if busy {
            content = content.child(
                shell::text(night::CAPTION, c(night_theme.muted))
                    .child(tr("Working on Yoake mods…")),
            );
        }
        if self.yak_mods.preview.is_some() {
            content = content.child(self.yak_preview(cx));
        }
        if self.yak_mods.rows.is_empty() && !busy {
            content = content.child(
                shell::text(night::UI, c(night_theme.muted)).child(tr("No .yak mods installed.")),
            );
        }
        for row in self.yak_mods.rows.clone() {
            content = content.child(self.yak_installed_row(row, cx));
        }
        content = content.child(
            shell::text(night::UI, c(night_theme.ink))
                .font_weight(FontWeight::SEMIBOLD)
                .child(tr("Yoake catalog")),
        );
        if self.yak_mods.login_required {
            content = content
                .child(
                    shell::text(night::UI, c(night_theme.muted))
                        .child(tr("Sign in to browse Yoake mods.")),
                )
                .child(
                    shell::action("yak-sign-in", Action::Main, &theme)
                        .child(shell::label(tr("Sign in")))
                        .on_click(cx.listener(|this, _: &ClickEvent, _, cx| {
                            this.go(Page::Yoake);
                            cx.notify();
                        })),
                );
        } else if let Some(catalog) = self.yak_mods.catalog.clone() {
            if catalog.cached {
                content = content.child(
                    shell::text(night::CAPTION, c(night_theme.warn))
                        .child(tr("Showing the verified cached catalog.")),
                );
            }
            if catalog.catalog.entries.is_empty() {
                content = content.child(
                    shell::text(night::UI, c(night_theme.muted))
                        .child(tr("The Yoake catalog is empty.")),
                );
            }
            for entry in catalog.catalog.entries {
                let compatible = self.yak_mods.environment.as_ref().and_then(|environment| {
                    entry
                        .versions
                        .iter()
                        .filter(|version| environment.check(&version.manifest).is_ok())
                        .max_by(|left, right| left.version.cmp(&right.version))
                });
                let selected = compatible.or_else(|| {
                    entry
                        .versions
                        .iter()
                        .max_by(|left, right| left.version.cmp(&right.version))
                });
                if let Some(version) = selected {
                    content = content.child(self.yak_catalog_row(
                        version.manifest.clone(),
                        compatible.is_some(),
                        cx,
                    ));
                }
            }
        } else if self.yak_mods.catalog_task.is_some() {
            content = content.child(
                shell::text(night::UI, c(night_theme.muted))
                    .child(tr("Loading the Yoake catalog…")),
            );
        }
        let home = self.yak_mods.home.clone();
        let drop_zone = div()
            .id("yak-drop")
            .flex()
            .flex_col()
            .p(px(18.))
            .border_2()
            .border_color(gpui::transparent_black())
            .drag_over::<ExternalPaths>(move |style, _, _, _| style.border_color(theme.accent))
            .on_drop(cx.listener(move |this, paths: &ExternalPaths, _, cx| {
                if let Some(home) = home.clone() {
                    this.add_local_yak(home, paths.paths().to_vec());
                }
                cx.stop_propagation();
                cx.notify();
            }))
            .child(content)
            .child(
                shell::text(night::CAPTION, c(night_theme.muted))
                    .mt(px(18.))
                    .child(tr("Drop a .yak file here, or use Add a .yak file….")),
            );
        let card = plate::cut(
            night::PIXEL_CORNER,
            2,
            Some(alpha(night_theme.card, 0.94)),
            Some((alpha(night_theme.stroke, night_theme.stroke_alpha), 1.)),
        )
        .flex()
        .flex_col()
        .child(drop_zone);
        (actions, card)
    }

    fn yak_services(&self, manifest: &Manifest, scope: &str, cx: &mut Context<Self>) -> Div {
        let theme = self.theme;
        let mut services = div().flex().flex_col().gap(px(4.));
        for service in &manifest.services {
            let mut line = div().flex().items_center().gap(px(8.)).child(
                shell::text(night::CAPTION, c(theme.night.muted))
                    .child(trf("This mod requires {name}.", &[("name", &service.name)])),
            );
            if let Some(url) = &service.help_url {
                let url = url.clone();
                line = line.child(
                    shell::action(
                        SharedString::from(format!(
                            "yak-help-{scope}-{}-{}",
                            manifest.id, service.id
                        )),
                        Action::Quiet,
                        &theme,
                    )
                    .child(shell::label(tr("Help")))
                    .on_click(cx.listener(move |_, _: &ClickEvent, _, cx| {
                        cx.open_url(&url);
                    })),
                );
            }
            services = services.child(line);
        }
        services
    }

    fn yak_installed_row(&self, row: Installed, cx: &mut Context<Self>) -> Div {
        let theme = self.theme;
        let busy = self.yak_busy();
        let id = row.manifest.id.clone();
        let state_color = if row.unsigned {
            c(theme.night.warn)
        } else if row.enabled {
            theme.success
        } else {
            theme.muted
        };
        let mut details = div()
            .flex_1()
            .flex()
            .flex_col()
            .gap(px(4.))
            .child(shell::text(night::UI, c(theme.night.ink)).child(name(&row.manifest).to_owned()))
            .child(
                shell::text(night::CAPTION, c(theme.night.muted))
                    .child(format!("{} · Yoake", row.manifest.version)),
            );
        if row.unsigned {
            details = details
                .child(shell::text(night::CAPTION, c(theme.night.warn)).child(tr("Unsigned .yak")));
        }
        details = details.child(self.yak_services(&row.manifest, "installed", cx));
        let mut controls = div().flex().items_center().gap(px(8.));
        let update = self
            .yak_mods
            .catalog
            .as_ref()
            .zip(self.yak_mods.environment.as_ref())
            .and_then(|(catalog, environment)| {
                catalog
                    .catalog
                    .entries
                    .iter()
                    .find(|entry| entry.id == id)
                    .and_then(|entry| {
                        entry
                            .versions
                            .iter()
                            .filter(|version| {
                                version.version > row.manifest.version
                                    && environment.check(&version.manifest).is_ok()
                            })
                            .max_by(|left, right| left.version.cmp(&right.version))
                    })
            });
        if let Some(update) = update {
            let update_id = id.clone();
            controls = controls.child(
                div()
                    .id(SharedString::from(format!("yak-update-{id}")))
                    .flex()
                    .items_center()
                    .h(px(24.))
                    .px(px(10.))
                    .rounded(px(night::RADIUS_CONTROL))
                    .bg(alpha(theme.night.warn, 0.16))
                    .cursor_pointer()
                    .child(
                        shell::text(night::CAPTION, c(theme.night.warn))
                            .font_weight(FontWeight::SEMIBOLD)
                            .child(trf(
                                "Update {version}",
                                &[("version", &update.version.to_string())],
                            )),
                    )
                    .when(!busy, |button| {
                        button.on_click(cx.listener(move |this, _: &ClickEvent, _, cx| {
                            this.plan_catalog_yak(update_id.clone());
                            cx.notify();
                        }))
                    }),
            );
        }
        let home = self.yak_mods.home.clone();
        let toggle_id = id.clone();
        let can_toggle = !busy && self.yak_mods.environment.is_some();
        controls = controls.child(
            switch(format!("yak-switch-{id}"), row.enabled, &theme)
                .when(can_toggle, |button| {
                    button.on_click(cx.listener(move |this, _: &ClickEvent, _, cx| {
                        if let Some(home) = home.clone() {
                            let selected = home.clone();
                            let id = toggle_id.clone();
                            this.yak_operation(home, move |client| {
                                client
                                    .set_enabled(&selected, &id, !row.enabled)
                                    .map(|()| Outcome::Done)
                            });
                        }
                        cx.notify();
                    }))
                })
                .when(!can_toggle, |button| button.opacity(theme::DISABLED)),
        );
        let home = self.yak_mods.home.clone();
        controls = controls.child(
            shell::action(
                SharedString::from(format!("yak-remove-{id}")),
                Action::Quiet,
                &theme,
            )
            .child(shell::label(tr("Remove")))
            .when(!busy, |button| {
                button.on_click(cx.listener(move |this, _: &ClickEvent, _, cx| {
                    if let Some(home) = home.clone() {
                        let selected = home.clone();
                        let id = id.clone();
                        this.yak_operation(home, move |client| {
                            client.remove(&selected, &id).map(|()| Outcome::Done)
                        });
                    }
                    cx.notify();
                }))
            }),
        );
        div()
            .flex()
            .items_center()
            .gap(px(14.))
            .px(px(18.))
            .py(px(12.))
            .child(div().size(px(8.)).bg(state_color))
            .child(details)
            .child(controls)
    }

    fn yak_catalog_row(&self, manifest: Manifest, compatible: bool, cx: &mut Context<Self>) -> Div {
        let theme = self.theme;
        let installed = self
            .yak_mods
            .rows
            .iter()
            .any(|row| row.manifest.id == manifest.id);
        let enabled = compatible && !installed && !self.yak_busy();
        let mut details = div()
            .flex_1()
            .flex()
            .flex_col()
            .gap(px(4.))
            .child(shell::text(night::UI, c(theme.night.ink)).child(name(&manifest).to_owned()))
            .child(
                shell::text(night::CAPTION, c(theme.night.muted))
                    .child(format!("{} · Yoake", manifest.version)),
            )
            .child(self.yak_services(&manifest, "catalog", cx));
        if !compatible {
            details = details.child(
                shell::text(night::CAPTION, c(theme.night.warn))
                    .child(tr("No compatible version for this instance and OS.")),
            );
        }
        let id = manifest.id;
        div()
            .flex()
            .items_center()
            .gap(px(14.))
            .px(px(18.))
            .py(px(12.))
            .child(div().size(px(8.)).bg(if compatible {
                theme.accent
            } else {
                c(theme.night.warn)
            }))
            .child(details)
            .child(
                shell::action(
                    SharedString::from(format!("yak-install-{id}")),
                    Action::Raised,
                    &theme,
                )
                .child(shell::label(if installed {
                    tr("Installed")
                } else {
                    tr("Install")
                }))
                .when(enabled, |button| {
                    button.on_click(cx.listener(move |this, _: &ClickEvent, _, cx| {
                        this.plan_catalog_yak(id.clone());
                        cx.notify();
                    }))
                })
                .when(!enabled, |button| button.opacity(theme::DISABLED)),
            )
    }

    fn yak_preview(&self, cx: &mut Context<Self>) -> Div {
        let theme = self.theme;
        let plan = self.yak_mods.preview.as_ref().unwrap();
        let mut preview = div().flex().flex_col().gap(px(8.)).p(px(14.)).child(
            shell::text(night::UI, c(theme.night.ink))
                .font_weight(FontWeight::SEMIBOLD)
                .child(tr("Review mod changes")),
        );
        if plan.changes.is_empty() {
            preview = preview.child(
                shell::text(night::UI, c(theme.night.muted)).child(tr("No mod downloads needed.")),
            );
        }
        for manifest in &plan.changes {
            preview = preview
                .child(shell::text(night::UI, c(theme.night.ink)).child(format!(
                    "{} · {}",
                    name(manifest),
                    manifest.version
                )))
                .child(self.yak_services(manifest, "preview", cx));
        }
        if plan.unsigned {
            let warning = if plan.unsigned_program {
                tr("Lets you install and run unsigned .yak files that contain DLLs from your own PC. Do not use it for mods you got from someone else.")
            } else {
                tr("Unsigned data-only mod. Install only if you trust its source.")
            };
            preview = preview.child(shell::text(night::UI, c(theme.night.warn)).child(warning));
        }
        preview.child(
            div()
                .flex()
                .gap(px(8.))
                .child(
                    shell::action("yak-confirm", Action::Main, &theme)
                        .child(shell::label(tr("Confirm")))
                        .on_click(cx.listener(|this, _: &ClickEvent, _, cx| {
                            if let Some(plan) = this.yak_mods.preview.take() {
                                if let Some(home) = this.yak_mods.home.clone() {
                                    let dev_unsigned_dll = this.settings.dev_unsigned_dll;
                                    this.yak_operation(home, move |client| {
                                        client
                                            .apply(plan, true, dev_unsigned_dll)
                                            .map(|()| Outcome::Done)
                                    });
                                }
                            }
                            cx.notify();
                        })),
                )
                .child(
                    shell::action("yak-cancel", Action::Quiet, &theme)
                        .child(shell::label(tr("Cancel")))
                        .on_click(cx.listener(|this, _: &ClickEvent, _, cx| {
                            this.yak_mods.preview = None;
                            cx.notify();
                        })),
                ),
        )
    }
}
