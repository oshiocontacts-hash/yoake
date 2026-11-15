//! The Friends page in the glass look (`docs/design-system.md`, "Glass"): the page's name, then
//! one tabbed panel as Home's (`glass_home_tabs.rs`): Friends (your code and your friends),
//! Requests (add by code, incoming, outgoing), Groups (yours, one open at a time) and Blocked.
//! Its parts are the glass look's own (`glass_ui`): rows, quiet and solid buttons, fields, the
//! empty line and the danger line. Dark appearance only; Light says so.
use super::*;
use crate::friends_page::{
    day, friend_note, request_note, role_word, Confirm, Copied, FriendAction, FriendsStatus,
    FriendsTab, GroupPanel, InputKind, Outcome,
};
use crate::glass_home_lists::{danger_button, danger_line, picture};
use crate::glass_ui::{self as ui, white};
use crate::social::{self, Answer, Role};
use horizon_theme::glass::{ink, radius};

impl Launcher {
    pub(super) fn friends_page(&mut self, window: &mut Window, cx: &mut Context<Self>) -> Div {
        let t = self.theme;
        if !t.dark {
            return div()
                .flex()
                .flex_col()
                .gap(px(24.))
                .child(page_header(
                    "Friends",
                    "Friends and groups you share through the Yoake portal.",
                    &t,
                ))
                .child(quiet_note(
                    tr("Friends is shown in the dark look for now. Choose Dark in Settings."),
                    &t,
                ));
        }
        self.friends.note_focus(window, cx);
        if self.friends.status == FriendsStatus::Idle {
            self.friends_refresh();
        }
        let ready = self.friends.status == FriendsStatus::Ready;
        let counts = ready.then(|| {
            let l = &self.friends.list;
            (
                l.friends.len(),
                l.incoming.len() + l.outgoing.len(),
                self.friends.groups.len(),
                self.friends.blocks.len(),
            )
        });
        let tabs = [
            (FriendsTab::Friends, "user", tr("Friends"), counts.map(|c| c.0)),
            (
                FriendsTab::Requests,
                "user-add-01",
                tr("Requests"),
                counts.map(|c| c.1),
            ),
            (
                FriendsTab::Groups,
                "user-multiple-02",
                tr("Groups"),
                counts.map(|c| c.2),
            ),
            (
                FriendsTab::Blocked,
                "view-off-slash",
                tr("Blocked"),
                counts.map(|c| c.3),
            ),
        ];
        let open = self.friends.tab;
        let strip = div()
            .relative()
            .flex()
            .flex_none()
            .items_center()
            .gap(px(4.))
            .px(px(8.))
            .pt(px(8.))
            .pb(px(4.))
            .child(crate::glass::measure(self.friends.strip.clone()))
            .children(tabs.into_iter().map(|(tab, icon, name, count)| {
                ui::count_tab(
                    format!("friends-tab-{tab:?}"),
                    Some(icon),
                    name,
                    count.filter(|n| *n > 0),
                    tab == open,
                    &t,
                )
                .on_click(cx.listener(move |this, _: &ClickEvent, _, cx| {
                    this.friends_set_tab(tab);
                    cx.notify();
                }))
            }));
        let body = match self.friends.status {
            FriendsStatus::Idle | FriendsStatus::Loading => {
                ui::empty().child(tr("Loading…")).into_any_element()
            }
            FriendsStatus::NotAvailable => self.friends_notice(
                tr("Friends are not available yet."),
                tr("The portal has not turned them on. They will show up here once it does."),
                false,
                cx,
            ),
            FriendsStatus::Offline => self.friends_notice(
                tr("Cannot reach the portal."),
                tr("Check the connection and try again."),
                false,
                cx,
            ),
            FriendsStatus::SignedOut => self.friends_notice(
                tr("You are not signed in."),
                tr("Sign in on the Yoake page, then come back here."),
                true,
                cx,
            ),
            FriendsStatus::Ready => match open {
                FriendsTab::Friends => self.friends_tab(cx).into_any_element(),
                FriendsTab::Requests => self.requests_tab(cx).into_any_element(),
                FriendsTab::Groups => self.groups_tab(cx).into_any_element(),
                FriendsTab::Blocked => self.blocked_tab(cx).into_any_element(),
            },
        };
        let head = div()
            .flex()
            .items_end()
            .justify_between()
            .gap(px(16.))
            .child(
                div()
                    .flex()
                    .flex_col()
                    .min_w_0()
                    .child(
                        div()
                            .truncate()
                            .font_family(horizon_theme::glass::DISPLAY_FONT)
                            .font_weight(FontWeight::SEMIBOLD)
                            .text_size(px(56.))
                            .line_height(px(56.))
                            .text_color(white(ink::FULL))
                            .child(tr("Friends")),
                    )
                    .child(
                        ui::text(15., FontWeight::NORMAL, ink::NOTE)
                            .mt(px(8.))
                            .child(tr("Friends and groups you share through the Yoake portal.")),
                    ),
            )
            .child(
                ui::icon_button(
                    "friends-refresh",
                    "refresh",
                    44.,
                    19.,
                    radius::CONTROL,
                    self.friends.status == FriendsStatus::Loading,
                    &t,
                )
                .on_click(cx.listener(|this, _: &ClickEvent, _, cx| {
                    this.friends.note = None;
                    if this.friends.status != FriendsStatus::Ready {
                        this.friends.status = FriendsStatus::Idle;
                    }
                    this.friends_refresh();
                    cx.notify();
                })),
            );
        div()
            .flex()
            .flex_col()
            .pt(px(36.))
            .child(head)
            .child(
                div()
                    .relative()
                    .mt(px(28.))
                    .child(crate::glass::tabbed(self.friends.strip.clone()))
                    .child(div().relative().flex().child(strip))
                    .child(
                        div().relative().p(px(8.)).child(
                            div()
                                .flex()
                                .flex_col()
                                .gap(px(12.))
                                .min_h(px(340.))
                                .px(px(4.))
                                .pt(px(8.))
                                .pb(px(4.))
                                .child(body),
                        ),
                    ),
            )
    }

    /// A state with nothing to list: what it is, what to do, and the way out.
    fn friends_notice(
        &mut self,
        title: &str,
        detail: &str,
        sign_in: bool,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let t = self.theme;
        let again = ui::quiet_icon("friends-retry", Some("refresh"), tr("Try again"), 36., &t)
            .on_click(cx.listener(|this, _: &ClickEvent, _, cx| {
                this.friends.status = FriendsStatus::Idle;
                this.friends_refresh();
                cx.notify();
            }));
        let yoake = sign_in.then(|| {
            ui::solid("friends-open-yoake", tr("Open Yoake"), 36., 14., &t).on_click(cx.listener(
                |this, _: &ClickEvent, _, cx| {
                    this.go(Page::Yoake);
                    cx.notify();
                },
            ))
        });
        div()
            .flex()
            .flex_col()
            .items_start()
            .gap(px(4.))
            .px(px(12.))
            .py(px(40.))
            .child(ui::text(16., FontWeight::SEMIBOLD, ink::FULL).child(title.to_string()))
            .child(ui::text(14., FontWeight::NORMAL, ink::NOTE).child(detail.to_string()))
            .child(
                div()
                    .mt(px(16.))
                    .flex()
                    .gap(px(8.))
                    .children(yoake)
                    .child(again),
            )
            .into_any_element()
    }

    /// The last result, over the list: red when it failed.
    fn friends_note_line(&self) -> Option<Div> {
        let (good, note) = self.friends.note.clone()?;
        Some(if good {
            div().px(px(12.)).pb(px(8.)).child(
                ui::text(13., FontWeight::NORMAL, ink::MUTED)
                    .flex()
                    .items_center()
                    .gap(px(6.))
                    .child(ui::icon(
                        "tick-02",
                        15.,
                        ui::color(self.look.accent_soft),
                    ))
                    .child(note),
            )
        } else {
            danger_line(note)
        })
    }

    /// A button that asks twice: `label`, then `ask` in red with a way back.
    fn friends_guarded(
        &self,
        id: String,
        confirm: Confirm,
        label: &'static str,
        ask: &'static str,
        cx: &mut Context<Self>,
        run: impl Fn(&mut Self) + 'static,
    ) -> AnyElement {
        let t = self.theme;
        let id = SharedString::from(id);
        if self.friends.confirm.as_ref() == Some(&confirm) {
            div()
                .flex()
                .items_center()
                .gap(px(2.))
                .child(
                    keyboard(danger_button(id.clone(), tr(ask).into()), &t).on_click(
                        cx.listener(move |this, _: &ClickEvent, _, cx| {
                            run(this);
                            cx.notify();
                        }),
                    ),
                )
                .child(
                    ui::icon_button(
                        SharedString::from(format!("{id}-back")),
                        "cancel-01",
                        36.,
                        17.,
                        radius::CONTROL,
                        false,
                        &t,
                    )
                    .on_click(cx.listener(|this, _: &ClickEvent, _, cx| {
                        this.friends.confirm = None;
                        cx.notify();
                    })),
                )
                .into_any_element()
        } else {
            ui::quiet(id, tr(label), 36., &t)
                .on_click(cx.listener(move |this, _: &ClickEvent, _, cx| {
                    this.friends.confirm = Some(confirm.clone());
                    cx.notify();
                }))
                .into_any_element()
        }
    }

    /// One row of a list: a picture, a name over a note, and buttons.
    fn friends_row(
        &mut self,
        key: String,
        icon: &'static str,
        name: String,
        note: String,
        actions: Vec<AnyElement>,
        cx: &mut Context<Self>,
    ) -> Stateful<Div> {
        let t = self.theme;
        let key = SharedString::from(key);
        let hovered = self.list_hover.as_ref() == Some(&key);
        let hover = self.list_hover_listener(key.clone(), cx);
        ui::list_row(key, hovered, hover, &t)
            .child(picture(None, icon))
            .child(ui::list_text(name, note))
            .children(
                (!actions.is_empty()).then(|| {
                    div()
                        .relative()
                        .flex()
                        .flex_none()
                        .items_center()
                        .gap(px(6.))
                        .children(actions)
                }),
            )
    }

    /// A text field of the page, in the glass look's `FIELD`.
    fn friends_field(
        &mut self,
        kind: InputKind,
        placeholder: &'static str,
        width: f32,
        cx: &mut Context<Self>,
    ) -> Stateful<Div> {
        let t = self.theme;
        let handle = self.friends.input(kind).handle(cx);
        let focused = self.friends.input(kind).focused;
        self.friends
            .input(kind)
            .edit
            .show(((width - 32.) / 8.) as usize);
        let field = ui::field(focused, false)
            .id(SharedString::from(format!("friends-field-{kind:?}")))
            .track_focus(&handle)
            .w(px(width))
            .cursor_text()
            .child(line_edit::content(
                &self.friends.input(kind).edit,
                &t,
                focused,
                false,
                false,
                placeholder,
            ))
            .on_click(cx.listener(move |this, _: &ClickEvent, window, cx| {
                let handle = this.friends.input(kind).handle(cx);
                window.focus(&handle, cx);
                cx.notify();
            }))
            .on_key_down(cx.listener(move |this, ev: &KeyDownEvent, window, cx| {
                match line_edit::handle(&mut this.friends.input(kind).edit, ev, cx) {
                    line_edit::Key::Pass => return,
                    line_edit::Key::Enter => this.friends_enter(kind),
                    line_edit::Key::Escape => window.blur(cx),
                    _ => {}
                }
                cx.stop_propagation();
                cx.notify();
            }));
        line_edit::mouse(field, cx, move |this| &mut this.friends.input(kind).edit)
    }

    /// A field with the button that sends it, as Home's Add server line.
    fn friends_form(
        &mut self,
        kind: InputKind,
        placeholder: &'static str,
        width: f32,
        button_id: &'static str,
        label: &'static str,
        cx: &mut Context<Self>,
    ) -> Div {
        let t = self.theme;
        let can = self.friends_input_ok(kind);
        let field = self.friends_field(kind, placeholder, width, cx);
        let send = ui::solid(button_id, tr(label), 44., 14., &t);
        let send = if can && self.friends.busy == 0 {
            send.on_click(cx.listener(move |this, _: &ClickEvent, _, cx| {
                this.friends_enter(kind);
                cx.notify();
            }))
        } else {
            send.opacity(0.4).cursor_default()
        };
        div()
            .flex()
            .items_center()
            .gap(px(8.))
            .px(px(12.))
            .pb(px(8.))
            .child(field)
            .child(send)
    }

    /// "Copy" on a button, or "Copied" for a moment after.
    fn friends_copy_button(
        &self,
        id: &'static str,
        what: Copied,
        text: String,
        cx: &mut Context<Self>,
    ) -> Stateful<Div> {
        let t = self.theme;
        let copied = self.friends.copied.is_some_and(|(c, _)| c == what);
        ui::quiet_icon(
            id,
            Some(if copied { "tick-02" } else { "copy-01" }),
            if copied { tr("Copied") } else { tr("Copy") },
            36.,
            &t,
        )
        .on_click(cx.listener(move |this, _: &ClickEvent, _, cx| {
            this.friends_copy(what, text.clone(), cx);
            cx.notify();
        }))
    }

    // ----- Friends --------------------------------------------------------------------

    fn friends_tab(&mut self, cx: &mut Context<Self>) -> Div {
        let t = self.theme;
        let mut body = div().flex().flex_col();
        body = body.children(self.friends_note_line());
        let code = self
            .friends
            .me
            .as_ref()
            .map(|me| me.friend_code.clone())
            .unwrap_or_default();
        let new_code = self.friends_guarded(
            "friends-new-code".into(),
            Confirm::NewCode,
            "New code",
            "Click again to replace it",
            cx,
            |this| this.friends_new_code(),
        );
        body = body.child(
            div().px(px(12.)).pb(px(12.)).child(
                div()
                    .flex()
                    .items_center()
                    .justify_between()
                    .gap(px(16.))
                    .p(px(16.))
                    .rounded(px(radius::LARGE))
                    .bg(white(0.04))
                    .child(
                        div()
                            .flex()
                            .flex_col()
                            .min_w_0()
                            .child(
                                ui::text(13., FontWeight::NORMAL, ink::NOTE)
                                    .child(tr("Your friend code")),
                            )
                            .child(
                                ui::text(24., FontWeight::SEMIBOLD, ink::FULL)
                                    .line_height(px(32.))
                                    .truncate()
                                    .child(code.clone()),
                            )
                            .child(
                                ui::text(12.5, FontWeight::NORMAL, ink::QUIET).mt(px(2.)).child(tr(
                                    "Share it privately. Anyone who has it can send you a friend request.",
                                )),
                            ),
                    )
                    .child(
                        div()
                            .flex()
                            .flex_none()
                            .items_center()
                            .gap(px(6.))
                            .child(self.friends_copy_button(
                                "friends-copy-code",
                                Copied::FriendCode,
                                code,
                                cx,
                            ))
                            .child(new_code),
                    ),
            ),
        );
        let friends = self.friends.list.friends.clone();
        let note = match friends.len() {
            1 => tr("1 friend").to_string(),
            n => trf("{count} friends", &[("count", &n.to_string())]),
        };
        body = body.child(
            ui::list_head(note).child(
                ui::quiet_icon("friends-add", Some("user-add-01"), tr("Add friend"), 36., &t)
                    .on_click(cx.listener(|this, _: &ClickEvent, _, cx| {
                        this.friends_set_tab(FriendsTab::Requests);
                        this.friends.focus_soon = Some(InputKind::AddFriend);
                        cx.notify();
                    })),
            ),
        );
        if friends.is_empty() {
            return body.child(ui::empty().child(tr(
                "No friends yet. Share your friend code, or add a friend with theirs.",
            )));
        }
        for friend in friends {
            let p = friend.principal.clone();
            let remove = self.friends_guarded(
                format!("friend-{}-remove", p.id),
                Confirm::Remove(p.id.clone()),
                "Remove",
                "Click again to remove",
                cx,
                {
                    let id = p.id.clone();
                    move |this| this.friends_remove(id.clone())
                },
            );
            let block = self.friends_guarded(
                format!("friend-{}-block", p.id),
                Confirm::Block(p.id.clone()),
                "Block",
                "Click again to block",
                cx,
                {
                    let id = p.id.clone();
                    move |this| this.friends_block(id.clone())
                },
            );
            body = body.child(self.friends_row(
                format!("fr-friend-{}", p.id),
                "user",
                p.display_name.clone(),
                friend_note(&friend),
                vec![remove, block],
                cx,
            ));
        }
        body
    }

    // ----- Requests -------------------------------------------------------------------

    fn requests_tab(&mut self, cx: &mut Context<Self>) -> Div {
        let t = self.theme;
        let mut body = div().flex().flex_col();
        body = body.children(self.friends_note_line());
        body = body.child(self.friends_form(
            InputKind::AddFriend,
            "Friend code (YKF-XXXX-XXXX)",
            340.,
            "friends-send",
            "Send request",
            cx,
        ));
        let (incoming, outgoing) = (
            self.friends.list.incoming.clone(),
            self.friends.list.outgoing.clone(),
        );
        if incoming.is_empty() && outgoing.is_empty() {
            return body.child(ui::empty().child(tr("No requests waiting.")));
        }
        if !incoming.is_empty() {
            body = body.child(ui::list_head(trf(
                "Incoming · {count}",
                &[("count", &incoming.len().to_string())],
            )));
            for request in incoming {
                let (accept_id, decline_id) = (request.id.clone(), request.id.clone());
                let accept = ui::solid(
                    SharedString::from(format!("request-{}-accept", request.id)),
                    tr("Accept"),
                    36.,
                    14.,
                    &t,
                )
                .on_click(cx.listener(move |this, _: &ClickEvent, _, cx| {
                    this.friends_answer(accept_id.clone(), Answer::Accept);
                    cx.notify();
                }))
                .into_any_element();
                let decline = ui::quiet(
                    SharedString::from(format!("request-{}-decline", request.id)),
                    tr("Decline"),
                    36.,
                    &t,
                )
                .on_click(cx.listener(move |this, _: &ClickEvent, _, cx| {
                    this.friends_answer(decline_id.clone(), Answer::Decline);
                    cx.notify();
                }))
                .into_any_element();
                body = body.child(self.friends_row(
                    format!("fr-in-{}", request.id),
                    "user-add-01",
                    request.from.display_name.clone(),
                    request_note(&request, true),
                    vec![accept, decline],
                    cx,
                ));
            }
        }
        if !outgoing.is_empty() {
            body = body.child(
                div().pt(px(8.)).child(ui::list_head(trf(
                    "Sent · {count}",
                    &[("count", &outgoing.len().to_string())],
                ))),
            );
            for request in outgoing {
                let id = request.id.clone();
                let cancel = ui::quiet(
                    SharedString::from(format!("request-{}-cancel", request.id)),
                    tr("Cancel request"),
                    36.,
                    &t,
                )
                .on_click(cx.listener(move |this, _: &ClickEvent, _, cx| {
                    this.friends_answer(id.clone(), Answer::Cancel);
                    cx.notify();
                }))
                .into_any_element();
                body = body.child(self.friends_row(
                    format!("fr-out-{}", request.id),
                    "user",
                    request.to.display_name.clone(),
                    request_note(&request, false),
                    vec![cancel],
                    cx,
                ));
            }
        }
        body
    }

    // ----- Blocked --------------------------------------------------------------------

    fn blocked_tab(&mut self, cx: &mut Context<Self>) -> Div {
        let t = self.theme;
        let mut body = div().flex().flex_col();
        body = body.children(self.friends_note_line());
        let blocks = self.friends.blocks.clone();
        if blocks.is_empty() {
            return body.child(ui::empty().child(tr("You have not blocked anyone.")));
        }
        body = body.child(ui::list_head(tr(
            "Blocked people cannot reach you with friend requests. In groups you do not manage, you do not see them.",
        )));
        for block in blocks {
            let id = block.blocked.id.clone();
            let unblock = ui::quiet(
                SharedString::from(format!("block-{}-undo", block.blocked.id)),
                tr("Unblock"),
                36.,
                &t,
            )
            .on_click(cx.listener(move |this, _: &ClickEvent, _, cx| {
                this.friends_unblock(id.clone());
                cx.notify();
            }))
            .into_any_element();
            body = body.child(self.friends_row(
                format!("fr-block-{}", block.blocked.id),
                "view-off-slash",
                block.blocked.display_name.clone(),
                trf("Blocked {date}", &[("date", day(&block.created_at))]),
                vec![unblock],
                cx,
            ));
        }
        body
    }

    // ----- Groups ---------------------------------------------------------------------

    fn groups_tab(&mut self, cx: &mut Context<Self>) -> Div {
        if self.friends.open_group.is_some() {
            return self.group_page(cx);
        }
        let t = self.theme;
        let mut body = div().flex().flex_col();
        body = body.children(self.friends_note_line());
        let groups = self.friends.groups.clone();
        let note = match groups.len() {
            1 => tr("1 group").to_string(),
            n => trf("{count} groups", &[("count", &n.to_string())]),
        };
        let panel = self.friends.panel;
        let toggle = |wanted: GroupPanel| {
            cx.listener(move |this: &mut Self, _: &ClickEvent, _, cx| {
                let open = this.friends.panel != wanted;
                this.friends.panel = if open { wanted } else { GroupPanel::None };
                this.friends.note = None;
                if open {
                    this.friends.focus_soon = Some(match wanted {
                        GroupPanel::Join => InputKind::JoinCode,
                        _ => InputKind::GroupName,
                    });
                }
                cx.notify();
            })
        };
        body = body.child(
            ui::list_head(note).child(
                div()
                    .flex()
                    .gap(px(8.))
                    .child(
                        ui::quiet_icon(
                            "group-join",
                            Some("link-01"),
                            tr("Join with code"),
                            36.,
                            &t,
                        )
                        .on_click(toggle(GroupPanel::Join)),
                    )
                    .child(
                        ui::quiet_icon(
                            "group-create",
                            Some("plus-sign"),
                            tr("Create group"),
                            36.,
                            &t,
                        )
                        .on_click(toggle(GroupPanel::Create)),
                    ),
            ),
        );
        match panel {
            GroupPanel::Create => {
                body = body.child(self.friends_form(
                    InputKind::GroupName,
                    "Group name",
                    340.,
                    "group-create-go",
                    "Create",
                    cx,
                ));
            }
            GroupPanel::Join => {
                body = body.child(self.friends_form(
                    InputKind::JoinCode,
                    "Invite code (YKG-XXXX-XXXX)",
                    340.,
                    "group-join-go",
                    "Join",
                    cx,
                ));
            }
            GroupPanel::None => {}
        }
        if groups.is_empty() && panel == GroupPanel::None {
            return body.child(ui::empty().child(tr(
                "You are in no groups yet. Create one, or join with an invite code.",
            )));
        }
        for group in groups {
            let key = SharedString::from(format!("fr-group-{}", group.id));
            let hovered = self.list_hover.as_ref() == Some(&key);
            let hover = self.list_hover_listener(key.clone(), cx);
            let id = group.id.clone();
            body = body.child(
                ui::list_row(key, hovered, hover, &t)
                    .cursor_pointer()
                    .child(picture(None, "user-multiple-02"))
                    .child(ui::list_text(
                        group.name.clone(),
                        trf(
                            "{role} · joined {date}",
                            &[
                                ("role", tr(role_word(group.role))),
                                ("date", day(&group.joined_at)),
                            ],
                        ),
                    ))
                    .child(
                        ui::icon("arrow-right-01", 16., white(ink::FAINT))
                            .relative()
                            .flex_none(),
                    )
                    .on_click(cx.listener(move |this, _: &ClickEvent, _, cx| {
                        this.friends.note = None;
                        this.friends_open_group(id.clone());
                        cx.notify();
                    })),
            );
        }
        body
    }

    /// One group: its members, invites and bans, with what the player may do.
    fn group_page(&mut self, cx: &mut Context<Self>) -> Div {
        let t = self.theme;
        let mut body = div().flex().flex_col();
        let back = div()
            .id("group-back")
            .flex()
            .items_center()
            .gap(px(4.))
            .px(px(12.))
            .pb(px(8.))
            .cursor_pointer()
            .text_color(white(ink::NOTE))
            .hover(|s| s.text_color(white(ink::FULL)))
            .child(
                ui::icon("arrow-right-01", 15., white(ink::NOTE)).with_transformation(
                    gpui::Transformation::rotate(gpui::radians(std::f32::consts::PI)),
                ),
            )
            .child(
                div()
                    .font_family(horizon_theme::glass::FONT)
                    .text_size(px(13.))
                    .child(tr("Groups")),
            )
            .on_click(cx.listener(|this, _: &ClickEvent, _, cx| {
                this.friends_close_group();
                cx.notify();
            }));
        body = body.child(keyboard(back, &t));
        body = body.children(self.friends_note_line());
        let Some(detail) = self.friends.detail.clone() else {
            let name = self
                .friends
                .groups
                .iter()
                .find(|g| Some(&g.id) == self.friends.open_group.as_ref())
                .map(|g| g.name.clone())
                .unwrap_or_default();
            return body
                .child(
                    ui::text(22., FontWeight::SEMIBOLD, ink::FULL)
                        .px(px(12.))
                        .child(name),
                )
                .child(ui::empty().child(tr("Loading…")));
        };
        let me = self.friends.my_id().to_string();
        let role = detail.role;
        let manages = role.manages() || detail.can_recover_ownership;
        let owner = role == Role::Owner;

        // The head: name and role, then what ends it.
        let mut ends = div().flex().items_center().gap(px(8.));
        if owner {
            ends = ends.child(self.friends_guarded(
                "group-delete".into(),
                Confirm::DeleteGroup,
                "Delete group",
                "Click again to delete the group",
                cx,
                |this| {
                    this.friends_group_act(FriendAction::DeleteGroup, |api, group| {
                        api.delete_group(group).map(|()| Outcome::Done)
                    })
                },
            ));
        } else {
            ends = ends.child(self.friends_guarded(
                "group-leave".into(),
                Confirm::Leave,
                "Leave group",
                "Click again to leave",
                cx,
                |this| this.friends_leave(),
            ));
        }
        body = body.child(
            div()
                .flex()
                .items_center()
                .justify_between()
                .gap(px(16.))
                .px(px(12.))
                .pb(px(12.))
                .child(
                    div()
                        .flex()
                        .flex_col()
                        .min_w_0()
                        .child(
                            ui::text(22., FontWeight::SEMIBOLD, ink::FULL)
                                .line_height(px(30.))
                                .truncate()
                                .child(detail.name.clone()),
                        )
                        .child(
                            ui::text(13., FontWeight::NORMAL, ink::NOTE).child(trf(
                                "{role} · {count} of {max} members",
                                &[
                                    ("role", tr(role_word(role))),
                                    ("count", &detail.members.len().to_string()),
                                    ("max", &social::MAX_GROUP_MEMBERS.to_string()),
                                ],
                            )),
                        ),
                )
                .child(ends),
        );
        if detail.can_recover_ownership {
            body = body.child(
                ui::text(13., FontWeight::NORMAL, ink::NOTE)
                    .px(px(12.))
                    .pb(px(12.))
                    .child(tr(
                        "The owner is gone and you are next in line: creating or revoking an invite makes you the owner.",
                    )),
            );
        }

        // Invites: make one, see the new code once, see and revoke the others.
        if manages {
            let uses = social::INVITE_USES[self.friends.invite_uses % social::INVITE_USES.len()];
            let days = social::INVITE_DAYS[self.friends.invite_days % social::INVITE_DAYS.len()];
            let make = div()
                .flex()
                .items_center()
                .gap(px(8.))
                .px(px(12.))
                .pb(px(8.))
                .child(
                    ui::quiet(
                        "invite-uses",
                        trf("{count} uses", &[("count", &uses.to_string())]),
                        36.,
                        &t,
                    )
                    .on_click(cx.listener(|this, _: &ClickEvent, _, cx| {
                        this.friends.invite_uses += 1;
                        cx.notify();
                    })),
                )
                .child(
                    ui::quiet(
                        "invite-days",
                        trf("{count} days", &[("count", &days.to_string())]),
                        36.,
                        &t,
                    )
                    .on_click(cx.listener(|this, _: &ClickEvent, _, cx| {
                        this.friends.invite_days += 1;
                        cx.notify();
                    })),
                )
                .child(
                    ui::solid("invite-create", tr("Create invite"), 36., 14., &t).on_click(
                        cx.listener(|this, _: &ClickEvent, _, cx| {
                            this.friends_create_invite();
                            cx.notify();
                        }),
                    ),
                );
            body = body.child(ui::list_head(tr("Invites")));
            body = body.child(make);
            if let Some((group, code)) = self.friends.new_invite.clone() {
                if group == detail.id {
                    body = body.child(
                        div().px(px(12.)).pb(px(12.)).child(
                            div()
                                .flex()
                                .items_center()
                                .justify_between()
                                .gap(px(16.))
                                .p(px(16.))
                                .rounded(px(radius::LARGE))
                                .bg(white(0.04))
                                .child(
                                    div()
                                        .flex()
                                        .flex_col()
                                        .min_w_0()
                                        .child(
                                            ui::text(13., FontWeight::NORMAL, ink::NOTE)
                                                .child(tr("New invite code")),
                                        )
                                        .child(
                                            ui::text(24., FontWeight::SEMIBOLD, ink::FULL)
                                                .line_height(px(32.))
                                                .child(code.clone()),
                                        )
                                        .child(
                                            ui::text(12.5, FontWeight::NORMAL, ink::QUIET)
                                                .mt(px(2.))
                                                .child(tr(
                                                    "Shown only once. Send it privately, not in a public chat.",
                                                )),
                                        ),
                                )
                                .child(self.friends_copy_button(
                                    "group-copy-invite",
                                    Copied::InviteCode,
                                    code,
                                    cx,
                                )),
                        ),
                    );
                }
            }
            let today = social::day_from_now(0);
            let active: Vec<_> = detail
                .invites
                .iter()
                .filter(|i| {
                    i.revoked_at.is_none() && i.uses < i.max_uses && day(&i.expires_at) >= today.as_str()
                })
                .cloned()
                .collect();
            if active.is_empty() {
                body = body.child(
                    ui::text(13., FontWeight::NORMAL, ink::NOTE)
                        .px(px(12.))
                        .pb(px(12.))
                        .child(tr("No active invites.")),
                );
            } else {
                body = body.child(ui::list_head(trf(
                    "{count} of {max} active invites",
                    &[
                        ("count", &active.len().to_string()),
                        ("max", &social::MAX_ACTIVE_INVITES.to_string()),
                    ],
                )));
            }
            for invite in active {
                let id = invite.id.clone();
                let revoke = ui::quiet(
                    SharedString::from(format!("invite-{}-revoke", invite.id)),
                    tr("Revoke"),
                    36.,
                    &t,
                )
                .on_click(cx.listener(move |this, _: &ClickEvent, _, cx| {
                    let id = id.clone();
                    this.friends_group_act(FriendAction::RevokeInvite, move |api, group| {
                        api.revoke_invite(group, &id).map(|()| Outcome::Done)
                    });
                    cx.notify();
                }))
                .into_any_element();
                body = body.child(self.friends_row(
                    format!("fr-invite-{}", invite.id),
                    "link-01",
                    tr("Invite").to_string(),
                    trf(
                        "{used} of {max} used · expires {date}",
                        &[
                            ("used", &invite.uses.to_string()),
                            ("max", &invite.max_uses.to_string()),
                            ("date", day(&invite.expires_at)),
                        ],
                    ),
                    vec![revoke],
                    cx,
                ));
            }
        }

        // Members.
        body = body.child(
            div().pt(px(8.)).child(ui::list_head(trf(
                "Members · {count}",
                &[("count", &detail.members.len().to_string())],
            ))),
        );
        for member in detail.members.clone() {
            let p = member.principal.clone();
            let you = member.principal_id == me;
            let mut actions = Vec::new();
            let known = self.friends.list.friends.iter().any(|f| f.principal.id == member.principal_id)
                || self.friends.list.outgoing.iter().any(|r| r.to_id == member.principal_id)
                || self.friends.list.incoming.iter().any(|r| r.from_id == member.principal_id)
                || self.friends.blocks.iter().any(|b| b.blocked_id == member.principal_id);
            if !you && !known {
                let id = member.principal_id.clone();
                actions.push(
                    ui::quiet(
                        SharedString::from(format!("member-{}-friend", member.principal_id)),
                        tr("Add friend"),
                        36.,
                        &t,
                    )
                    .on_click(cx.listener(move |this, _: &ClickEvent, _, cx| {
                        this.friends_send_to_member(id.clone());
                        cx.notify();
                    }))
                    .into_any_element(),
                );
            }
            if !you && member.role != Role::Owner {
                if owner {
                    let (id, to) = (
                        member.principal_id.clone(),
                        if member.role == Role::Admin {
                            Role::Member
                        } else {
                            Role::Admin
                        },
                    );
                    let label = if member.role == Role::Admin {
                        tr("Make member")
                    } else {
                        tr("Make admin")
                    };
                    actions.push(
                        ui::quiet(
                            SharedString::from(format!("member-{}-role", member.principal_id)),
                            label,
                            36.,
                            &t,
                        )
                        .on_click(cx.listener(move |this, _: &ClickEvent, _, cx| {
                            let id = id.clone();
                            this.friends_group_act(FriendAction::SetRole, move |api, group| {
                                api.set_role(group, &id, to).map(|()| Outcome::Done)
                            });
                            cx.notify();
                        }))
                        .into_any_element(),
                    );
                    if !p.is_guest() {
                        let id = member.principal_id.clone();
                        actions.push(self.friends_guarded(
                            format!("member-{}-owner", member.principal_id),
                            Confirm::Transfer(member.principal_id.clone()),
                            "Make owner",
                            "Click again to hand over the group",
                            cx,
                            move |this| {
                                let id = id.clone();
                                this.friends_group_act(FriendAction::Transfer, move |api, group| {
                                    api.transfer_ownership(group, &id).map(|()| Outcome::Done)
                                })
                            },
                        ));
                    }
                }
                if owner || (role == Role::Admin && member.role == Role::Member) {
                    let id = member.principal_id.clone();
                    actions.push(self.friends_guarded(
                        format!("member-{}-kick", member.principal_id),
                        Confirm::Kick(member.principal_id.clone()),
                        "Remove",
                        "Click again to remove and ban",
                        cx,
                        move |this| {
                            let id = id.clone();
                            this.friends_group_act(FriendAction::Kick, move |api, group| {
                                api.remove_member(group, &id).map(|()| Outcome::Done)
                            })
                        },
                    ));
                }
            }
            let name = if you {
                trf("{name} (you)", &[("name", &p.display_name)])
            } else {
                p.display_name.clone()
            };
            body = body.child(self.friends_row(
                format!("fr-member-{}", member.principal_id),
                "user",
                name,
                trf(
                    "{role} · joined {date}",
                    &[
                        ("role", tr(role_word(member.role))),
                        ("date", day(&member.joined_at)),
                    ],
                ),
                actions,
                cx,
            ));
        }

        // Bans, for those who can lift them.
        if manages && !detail.bans.is_empty() {
            body = body.child(
                div().pt(px(8.)).child(ui::list_head(trf(
                    "Banned · {count}",
                    &[("count", &detail.bans.len().to_string())],
                ))),
            );
            for ban in detail.bans.clone() {
                let id = ban.principal_id.clone();
                let lift = ui::quiet(
                    SharedString::from(format!("ban-{}-lift", ban.principal_id)),
                    tr("Lift ban"),
                    36.,
                    &t,
                )
                .on_click(cx.listener(move |this, _: &ClickEvent, _, cx| {
                    let id = id.clone();
                    this.friends_group_act(FriendAction::LiftBan, move |api, group| {
                        api.lift_ban(group, &id).map(|()| Outcome::Done)
                    });
                    cx.notify();
                }))
                .into_any_element();
                body = body.child(self.friends_row(
                    format!("fr-ban-{}", ban.principal_id),
                    "view-off-slash",
                    ban.principal.display_name.clone(),
                    trf("Banned {date}", &[("date", day(&ban.created_at))]),
                    vec![lift],
                    cx,
                ));
            }
        }
        body
    }
}
