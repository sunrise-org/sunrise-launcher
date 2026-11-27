//! Settings › Account: who plays, and the Microsoft accounts signed in here.
use super::*;
use crate::widgets::button::{button, secondary, Look, Size, Style};

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

    pub(crate) fn move_account(&mut self, uuid: &str, delta: isize) {
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
        let listed = crate::account_chip::rows(&self.accounts, &self.auth_status);
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
                _ if needs_sign_in => tr("Signed out").to_string(),
                _ if plays => tr("Plays when you press Play").to_string(),
                _ => tr("Signed in").to_string(),
            };
            let mut actions = div();
            if needs_sign_in {
                let uuid = a.uuid.clone();
                // The warning mark sits in the button, where the banner's used to stand apart.
                let again = button(
                    format!("alt-account-again-{}", i),
                    tr("Sign in again"),
                    Look::new(Style::Secondary, Size::Medium).icon("alert-02"),
                );
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
                .tooltip(crate::glass_ui::tip_unless(
                    menu_open,
                    tr("Account options"),
                ))
                .child(crate::glass_ui::icon(
                    "more-vertical",
                    16.,
                    rgb(palette().muted).into(),
                ))
                .on_mouse_down(gpui::MouseButton::Left, |_, _, cx| cx.stop_propagation())
                .on_click(cx.listener(move |this, _: &ClickEvent, _, cx| {
                    let shut =
                        this.ui.settings.account_menu.as_deref() == Some(toggle_uuid.as_str());
                    this.close_menus();
                    if !shut {
                        this.ui.settings.account_menu = Some(toggle_uuid.clone());
                    }
                    cx.notify();
                }));
            let menu = menu_open.then(|| {
                // The same actions, icons and conditions as the account chip's submenu.
                let entries = listed
                    .get(i)
                    .map(|row| crate::account_chip::account_entries(row, i, total))
                    .unwrap_or_default();
                let mut list = crate::account_chip::entry_card()
                    .id(ElementId::named_usize("alt-account-menu-list", i))
                    .w(px(208.))
                    .occlude();
                for entry in entries {
                    if entry.rule_before {
                        list = list.child(crate::widgets::dropdown::rule());
                    }
                    let removing = matches!(entry.action, crate::account_chip::Action::Remove(_));
                    let action = entry.action.clone();
                    let item = crate::account_chip::entry_item(
                        &entry,
                        ElementId::named_usize(entry.id, i),
                    );
                    list = list.child(if busy && removing {
                        off(item)
                    } else {
                        item.on_click(cx.listener(move |this, _: &ClickEvent, _, cx| {
                            this.ui.settings.account_menu = None;
                            this.account_entry(action.clone(), cx);
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
                .child(crate::widgets::label::name_over(
                    a.name.clone(),
                    tokens::BODY,
                    Some((
                        SharedString::from(said),
                        if needs_sign_in && progress.is_none() {
                            palette().warning
                        } else {
                            palette().muted
                        },
                    )),
                ));
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
        // Short of room, the button is the logo and "Sign in", as the top bar's is.
        let narrow = self.settings_pane_width() < 520.;
        let sign_in = microsoft_button(
            "alt-add-account",
            if narrow {
                tr("Sign in")
            } else {
                tr("Sign in with Microsoft")
            },
            self.theme.dark,
        )
        .on_click(cx.listener(|this, _: &ClickEvent, _, cx| {
            this.sign_in_to_play();
            cx.notify();
        }));
        let title = tr(if none {
            "Not signed in"
        } else {
            "Another account"
        });
        // Narrow, the title and the button share one line and the description is left out.
        rows.push(if narrow {
            row_with(title, None, div().child(sign_in))
        } else {
            row(
                title,
                tr("Sign in with the Microsoft account that owns Minecraft: Java Edition."),
                div().child(sign_in),
            )
        });
        group(rows)
    }
}

/// Microsoft's sign-in button at the settings row's height: its four-square logo (the one the
/// top bar draws, `microsoft::logo`) before `label`, in Microsoft's dark or light colours.
fn microsoft_button(id: &'static str, label: impl Into<SharedString>, dark: bool) -> Stateful<Div> {
    let (fill, ink, border) = if dark {
        (0x2f2f2f, 0xffffff, 0x2f2f2f)
    } else {
        (0xffffff, 0x5e5e5e, 0x8c8c8c)
    };
    div()
        .id(id)
        .reach()
        .tab_index(0)
        .flex()
        .flex_none()
        .items_center()
        .gap(px(10.))
        .h(px(tokens::CONTROL))
        .px(px(12.))
        .rounded(tokens::r(tokens::R_CONTROL))
        .border_1()
        .border_color(rgb(border))
        .bg(rgb(fill))
        .text_size(px(tokens::BODY))
        .font_weight(FontWeight::SEMIBOLD)
        .text_color(rgb(ink))
        .whitespace_nowrap()
        .cursor_pointer()
        .hover(|s| s.opacity(0.9))
        .focus_visible(|s| s.shadow(crate::frame::focus_ring(palette().focus)))
        .child(crate::microsoft::logo(16.))
        .child(label.into())
}
