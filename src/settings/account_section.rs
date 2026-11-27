//! Settings › Account: who plays, and the Microsoft accounts signed in here.
use super::*;
use crate::widgets::button::secondary;
use crate::widgets::feedback::{banner, Tone};

impl Launcher {
    pub(crate) fn remove_account(&mut self, uuid: &str) {
        if self.account_locked() {
            self.say_account_locked();
            return;
        }
        let Some(generation) = self.begin_auth_job(uuid) else {
            return;
        };
        let (tx, uuid) = (self.tx.clone(), uuid.to_owned());
        std::thread::spawn(move || {
            let mut accounts = Accounts::load();
            let _ = tx.send(Msg::AccountProgress(
                uuid.clone(),
                generation,
                account::session::Phase::SigningOut,
            ));
            let result = accounts
                .remove(account::store(), &uuid)
                .and_then(|()| accounts.save())
                .map(|()| accounts.has_owner(account::store()));
            let _ = tx.send(Msg::AccountRemoved(uuid, generation, result));
        });
    }

    fn move_account(&mut self, uuid: &str, delta: isize) {
        match Accounts::reorder(uuid, delta) {
            Ok(accounts) => self.accounts = accounts,
            Err(error) => self.ui.settings.note = Some(error),
        }
    }

    pub(super) fn settings_account(&mut self, cx: &mut Context<Self>) -> Div {
        let theme = self.theme;
        let selected = self.accounts.selected.clone();
        let total = self.accounts.accounts.len();
        let open_menu = self.ui.settings.account_menu.clone();
        let mut rows = Vec::new();
        for (i, a) in self.accounts.accounts.clone().into_iter().enumerate() {
            let plays = selected.as_ref() == Some(&a.uuid);
            let status = self.auth_status.get(&a.uuid);
            let needs_sign_in =
                status.is_some_and(|s| s.state == account::session::State::SignInRequired);
            let progress = self.auth_progress.get(&a.uuid).map(|phase| phase.label());
            let busy = self.auth_busy.contains_key(&a.uuid);
            let said = match (progress, status) {
                (Some(doing), _) => tr(doing).to_string(),
                (None, Some(s)) if needs_sign_in => tr(&s.message).to_string(),
                _ if plays => tr("Plays when you press Play").to_string(),
                _ => tr("Signed in").to_string(),
            };
            let mut actions = div();
            if needs_sign_in {
                let uuid = a.uuid.clone();
                let again = secondary(format!("alt-account-again-{}", i), tr("Sign in again"));
                actions = actions.child(if busy {
                    off(again)
                } else {
                    again.on_click(cx.listener(move |this, _: &ClickEvent, _, cx| {
                        this.re_sign_in(&uuid);
                        cx.notify();
                    }))
                });
            } else {
                if !plays {
                    let uuid = a.uuid.clone();
                    actions = actions.child(
                        secondary(format!("alt-account-play-{}", i), tr("Play as")).on_click(
                            cx.listener(move |this, _: &ClickEvent, _, cx| {
                                this.play_as(&uuid);
                                cx.notify();
                            }),
                        ),
                    );
                }
                let uuid = a.uuid.clone();
                let out = secondary(format!("alt-account-out-{}", i), tr("Sign out"));
                actions = actions.child(if busy {
                    off(out)
                } else {
                    out.on_click(cx.listener(move |this, _: &ClickEvent, _, cx| {
                        this.remove_account(&uuid);
                        cx.notify();
                    }))
                });
            }
            let menu_open = open_menu.as_deref() == Some(a.uuid.as_str());
            let toggle_uuid = a.uuid.clone();
            let dots = div()
                .id(ElementId::named_usize("alt-account-menu", i))
                .reach()
                .size(px(tokens::CONTROL))
                .rounded(tokens::r(tokens::R_CONTROL))
                .when(menu_open, |d| d.bg(rgb(palette().hover)))
                .hover(|s| s.bg(rgb(palette().hover)))
                .cursor_pointer()
                .flex()
                .items_center()
                .justify_center()
                .tooltip(crate::glass_ui::tip(tr("Account options")))
                .child(crate::glass_ui::icon(
                    "more-vertical",
                    16.,
                    rgb(palette().muted).into(),
                ))
                .on_mouse_down(gpui::MouseButton::Left, |_, _, cx| cx.stop_propagation())
                .on_click(cx.listener(move |this, _: &ClickEvent, _, cx| {
                    if this.ui.settings.account_menu.as_deref() == Some(toggle_uuid.as_str()) {
                        this.ui.settings.account_menu = None;
                    } else {
                        this.ui.settings.account_menu = Some(toggle_uuid.clone());
                    }
                    cx.notify();
                }));
            let menu = menu_open.then(|| {
                let item = |id: (&'static str, usize), label: &'static str, danger: bool| {
                    div()
                        .id(id)
                        .reach()
                        .flex()
                        .items_center()
                        .px(px(10.))
                        .py(px(7.))
                        .rounded(tokens::r(tokens::R_SMALL))
                        .text_size(px(tokens::LABEL))
                        .text_color(rgb(if danger {
                            palette().danger
                        } else {
                            palette().text
                        }))
                        .cursor_pointer()
                        .hover(|s| s.bg(rgb(palette().hover)))
                        .child(label)
                };
                let mut list = div()
                    .id(ElementId::named_usize("alt-account-menu-list", i))
                    .w(px(180.))
                    .p(px(6.))
                    .rounded(tokens::r(tokens::R_CONTROL))
                    .border_1()
                    .border_color(palette().input_edge())
                    .bg(rgb(palette().raised))
                    .shadow_lg()
                    .occlude()
                    .on_mouse_down(gpui::MouseButton::Left, |_, _, cx| cx.stop_propagation())
                    .flex()
                    .flex_col();
                if !plays {
                    let uuid = a.uuid.clone();
                    list = list.child(
                        item(("alt-account-default", i), tr("Set default"), false).on_click(
                            cx.listener(move |this, _: &ClickEvent, _, cx| {
                                this.ui.settings.account_menu = None;
                                this.play_as(&uuid);
                                cx.notify();
                            }),
                        ),
                    );
                }
                if i > 0 {
                    let uuid = a.uuid.clone();
                    list = list.child(item(("alt-account-up", i), tr("Move up"), false).on_click(
                        cx.listener(move |this, _: &ClickEvent, _, cx| {
                            this.ui.settings.account_menu = None;
                            this.move_account(&uuid, -1);
                            cx.notify();
                        }),
                    ));
                }
                if i + 1 < total {
                    let uuid = a.uuid.clone();
                    list = list.child(
                        item(("alt-account-down", i), tr("Move down"), false).on_click(
                            cx.listener(move |this, _: &ClickEvent, _, cx| {
                                this.ui.settings.account_menu = None;
                                this.move_account(&uuid, 1);
                                cx.notify();
                            }),
                        ),
                    );
                }
                {
                    let uuid = a.uuid.clone();
                    list = list.child(
                        item(("alt-account-skins", i), tr("Manage skins"), false).on_click(
                            cx.listener(move |this, _: &ClickEvent, _, cx| {
                                this.ui.settings.account_menu = None;
                                if this.accounts.selected.as_deref() != Some(uuid.as_str()) {
                                    if this.account_locked() {
                                        this.say_account_locked();
                                        cx.notify();
                                        return;
                                    }
                                    this.play_as(&uuid);
                                }
                                this.navigate(crate::frame::Page::Skins);
                                cx.notify();
                            }),
                        ),
                    );
                }
                {
                    let uuid = a.uuid.clone();
                    let remove = item(("alt-account-remove", i), tr("Remove"), true);
                    list = list.child(if busy {
                        off(remove)
                    } else {
                        remove.on_click(cx.listener(move |this, _: &ClickEvent, _, cx| {
                            this.ui.settings.account_menu = None;
                            this.remove_account(&uuid);
                            cx.notify();
                        }))
                    });
                }
                gpui::deferred(
                    div()
                        .absolute()
                        .top(px(tokens::CONTROL + 6.))
                        .right_0()
                        .child(list),
                )
                .with_priority(1)
            });
            actions = actions.child(div().relative().child(dots).children(menu));
            // As in `row`: when the head and name would get narrower than a short line, the
            // buttons go under them instead of squeezing the name to a letter a line.
            let who = div()
                .flex_1()
                .min_w(px(160. + 36. + 12.))
                .flex()
                .items_center()
                .gap(px(12.))
                .child(
                    div()
                        .flex_none()
                        .rounded(crate::tokens::r(crate::tokens::R_CONTROL))
                        .overflow_hidden()
                        .child(
                            self.profile_heads
                                .draw(Some(&a.uuid), 36., self.scale, &theme),
                        ),
                )
                .child(
                    div()
                        .flex_1()
                        .min_w_0()
                        .flex()
                        .flex_col()
                        .gap(px(2.))
                        .child(
                            div()
                                .font_weight(FontWeight::SEMIBOLD)
                                .child(a.name.clone()),
                        )
                        .child(if needs_sign_in {
                            banner(Tone::Danger, said, None)
                        } else {
                            hint(said)
                        }),
                );
            rows.push(
                div()
                    .flex()
                    .flex_wrap()
                    .items_center()
                    .gap_x(px(16.))
                    .gap_y(px(10.))
                    .child(who)
                    .child(actions.flex_none().flex().items_center().gap(px(6.))),
            );
        }
        let none = rows.is_empty();
        rows.push(row(
            tr(if none {
                "Not signed in"
            } else {
                "Another account"
            }),
            tr("Sign in with the Microsoft account that owns Minecraft: Java Edition."),
            div().child(
                secondary("alt-add-account", tr("Sign in with Microsoft")).on_click(cx.listener(
                    |this, _: &ClickEvent, _, cx| {
                        this.sign_in_to_play();
                        cx.notify();
                    },
                )),
            ),
        ));
        group(rows)
    }
}
