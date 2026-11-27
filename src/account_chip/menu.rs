//! The account menu as drawn: the card under the chip, one row per account, and the submenu that
//! opens beside a row with that account's actions.
//!
//! The card is the shared menu look (`widgets::dropdown`, the profile menu): raised fill, line
//! border, `R_CONTROL` corners, 4 px padding, 32 px rows filled on hover. Each account row starts
//! with a chevron pointing left, the way its submenu opens; the pointer resting on the row, or a
//! press on the chevron, opens the submenu to the
//! left of the card (the card hangs from the window's right edge). It fades and slides in on
//! `QUICK`, and closes a moment after the pointer has left both the row and the submenu, so a path
//! across the gap between them doesn't lose it.
use super::*;
use crate::widgets::dropdown;
use gpui::{MouseButton, MouseDownEvent, MouseMoveEvent};
use std::time::Duration;

/// An account row: its face, and the name with its status right under it.
const ACCOUNT_ROW: f32 = 40.;
/// The submenu's width, and how far it sits from the card.
const SUB_WIDTH: f32 = 208.;
const SUB_GAP: f32 = 2.;
/// How long the pointer may be away from the row and the submenu before the submenu closes.
const LEAVE_DELAY: Duration = Duration::from_millis(220);
/// How far the submenu slides while it fades in.
const SLIDE: f32 = 4.;

/// One 32 px row: an icon and a label, filled on hover, a Tab stop.
fn item(id: &'static str, icon: &'static str, label: &str, ink: u32) -> Stateful<Div> {
    div()
        .id(id)
        .reach()
        .flex_none()
        .flex()
        .items_center()
        .gap(px(8.))
        .h(px(MENU_ROW))
        .px(px(8.))
        .cursor_pointer()
        .text_size(px(tokens::LABEL))
        .text_color(rgb(ink))
        .hover(|s| s.bg(rgb(palette().hover)))
        .child(crate::glass_ui::icon(icon, 16., rgb(ink).into()))
        .child(label.to_string())
}

/// The shared menu card: raised fill, line border, 4 px padding.
fn card() -> Div {
    div()
        .flex()
        .flex_col()
        .gap(px(1.))
        .p(px(4.))
        .rounded(crate::tokens::r(crate::tokens::R_CONTROL))
        .border_1()
        .border_color(palette().input_edge())
        .bg(rgb(palette().raised))
        .text_color(rgb(palette().text))
        .shadow_lg()
        .on_mouse_down(MouseButton::Left, |_: &MouseDownEvent, _, cx| {
            cx.stop_propagation()
        })
}

impl Launcher {
    /// The pointer is on `uuid`'s row or on its submenu: that submenu is open, and no close timer
    /// still running may shut it.
    fn sub_enter(&mut self, uuid: &str, cx: &mut Context<Self>) {
        self.account_menu.leaves += 1;
        if self.account_menu.sub.as_deref() != Some(uuid) {
            self.account_menu.sub = Some(uuid.to_owned());
            cx.notify();
        }
    }

    /// The pointer is on a row that has no submenu: the open one goes at once.
    fn sub_close_now(&mut self, cx: &mut Context<Self>) {
        self.account_menu.leaves += 1;
        if self.account_menu.sub.take().is_some() {
            cx.notify();
        }
    }

    /// The pointer left the row or the submenu: it closes after a short while, unless the pointer
    /// has come back to either by then.
    fn sub_leave(&mut self, cx: &mut Context<Self>) {
        self.account_menu.leaves += 1;
        let mine = self.account_menu.leaves;
        cx.spawn(async move |this, cx| {
            cx.background_executor().timer(LEAVE_DELAY).await;
            let _ = this.update(cx, |this, cx| {
                if this.account_menu.leaves == mine && this.account_menu.sub.take().is_some() {
                    cx.notify();
                }
            });
        })
        .detach();
    }

    /// One account's row: the chevron opens its submenu; press the face and name to play as it.
    fn account_row(&self, n: usize, row: Row, cx: &mut Context<Self>) -> Stateful<Div> {
        let p = palette();
        let open = self.account_menu.sub.as_deref() == Some(row.uuid.as_str());
        let seen = self.account_menu.rows.clone();
        let (key, play, enter, toggle) = (
            row.uuid.clone(),
            row.uuid.clone(),
            row.uuid.clone(),
            row.uuid.clone(),
        );
        let main = div()
            .id(ElementId::named_usize("account-menu-play-as", n))
            .reach()
            .flex_1()
            .min_w_0()
            .h_full()
            .flex()
            .items_center()
            .gap(px(10.))
            .px(px(6.))
            .cursor_pointer()
            .child(self.face(&row.uuid, FACE))
            .child(
                div()
                    .flex_1()
                    .min_w_0()
                    .flex()
                    .flex_col()
                    .gap(px(0.))
                    .child(
                        div()
                            .truncate()
                            .line_height(px(16.))
                            .text_size(px(tokens::LABEL))
                            .font_weight(FontWeight::MEDIUM)
                            .child(row.name),
                    )
                    .when(row.signed_out, |d| {
                        d.child(
                            div()
                                .truncate()
                                .line_height(px(14.))
                                .text_size(px(tokens::SMALL))
                                .text_color(rgb(p.warning))
                                .child(tr("Signed out")),
                        )
                    }),
            )
            .children(
                row.plays
                    .then(|| crate::glass_ui::icon("tick-02", 16., rgb(p.accent_ink).into())),
            )
            .on_click(cx.listener(move |this, _, _, cx| {
                this.account_action(Action::PlayAs(play.clone()));
                cx.notify();
            }));
        let chevron = div()
            .id(ElementId::named_usize("account-menu-more", n))
            .reach()
            .flex_none()
            .w(px(22.))
            .h(px(28.))
            .flex()
            .items_center()
            .justify_center()
            .cursor_pointer()
            .child(crate::glass_ui::icon(
                "arrow-left-01",
                14.,
                rgb(if open { p.text } else { p.muted }).into(),
            ))
            .on_click(cx.listener(move |this, _, _, cx| {
                this.account_menu.leaves += 1;
                let shut = this.account_menu.sub.as_deref() == Some(toggle.as_str());
                this.account_menu.sub = (!shut).then(|| toggle.clone());
                cx.notify();
            }));
        div()
            .id(ElementId::named_usize("account-menu-row", n))
            .flex_none()
            .flex()
            .items_center()
            .h(px(ACCOUNT_ROW))
            .pl(px(2.))
            .pr(px(4.))
            .rounded(crate::tokens::r(crate::tokens::R_SMALL))
            .when(open, |d| d.bg(rgb(p.hover)))
            .hover(|s| s.bg(rgb(p.hover)))
            .child(
                gpui::canvas(
                    move |bounds, _, _| {
                        seen.borrow_mut().insert(key.clone(), bounds);
                    },
                    |_, _, _, _| {},
                )
                .absolute()
                .inset_0(),
            )
            .child(chevron)
            .child(main)
            .on_hover(cx.listener(move |this, hovered: &bool, _, cx| {
                if *hovered {
                    this.sub_enter(&enter, cx);
                } else {
                    this.sub_leave(cx);
                }
            }))
    }

    /// The actions for `row`'s account, in a card beside the menu. None until a row is open.
    fn account_submenu(&self, right: f32, cx: &mut Context<Self>) -> Option<AnyElement> {
        let p = palette();
        let uuid = self.account_menu.sub.clone()?;
        let row = rows(&self.accounts, &self.auth_status)
            .into_iter()
            .find(|r| r.uuid == uuid)?;
        let at = self.account_menu.rows.borrow().get(&uuid).copied()?;
        let top = f32::from(at.top()) - 4.;
        let base = right + MENU_WIDTH + SUB_GAP;
        let go = |action: Action| {
            cx.listener(move |this, _: &ClickEvent, _, cx| {
                this.account_action(action.clone());
                cx.notify();
            })
        };
        let mut list = card()
            .id("account-submenu")
            .absolute()
            .w(px(SUB_WIDTH))
            .child(
                div()
                    .flex_none()
                    .px(px(8.))
                    .pt(px(4.))
                    .pb(px(2.))
                    .truncate()
                    .text_size(px(tokens::SMALL))
                    .text_color(rgb(p.muted))
                    .child(row.name.clone()),
            );
        if row.signed_out {
            list = list.child(
                item(
                    "account-sub-sign-in",
                    "refresh",
                    tr("Sign in again"),
                    p.text,
                )
                .on_click(go(Action::SignInAgain(uuid.clone()))),
            );
        }
        if !row.plays && !row.signed_out {
            list = list.child(
                item(
                    "account-sub-play",
                    "play",
                    tr("Play as this account"),
                    p.text,
                )
                .on_click(go(Action::PlayAs(uuid.clone()))),
            );
        }
        let copied = uuid.clone();
        list = list
            .child(
                item("account-sub-copy", "copy-01", tr("Copy UUID"), p.text).on_click(cx.listener(
                    move |this, _: &ClickEvent, _, cx| {
                        cx.write_to_clipboard(gpui::ClipboardItem::new_string(copied.clone()));
                        this.account_action(Action::CopyUuid(copied.clone()));
                        cx.notify();
                    },
                )),
            )
            .child(
                item("account-sub-skin", "t-shirt", tr("Manage skin…"), p.text)
                    .on_click(go(Action::ManageSkin(uuid.clone()))),
            )
            .child(dropdown::rule())
            .child(
                item(
                    "account-sub-remove",
                    "delete-02",
                    tr("Remove account"),
                    p.danger,
                )
                .on_click(go(Action::Remove(uuid.clone()))),
            );
        let list = list
            .on_hover(cx.listener({
                let uuid = uuid.clone();
                move |this, hovered: &bool, _, cx| {
                    this.account_menu.in_sub = *hovered;
                    if *hovered {
                        this.menu_enter();
                        this.sub_enter(&uuid, cx);
                    } else {
                        this.menu_leave(cx);
                        this.sub_leave(cx);
                    }
                }
            }))
            .with_animation(
                SharedString::from(format!("account-sub-in-{uuid}")),
                crate::motion::eased(sunrise_launcher_theme::QUICK),
                move |d, t| d.opacity(t).top(px(top)).right(px(base - SLIDE * (1. - t))),
            )
            .into_any_element();
        Some(list)
    }

    /// The menu, as the layer the frame draws over the page; none while it is closed or the
    /// sign-in it started is in front.
    pub(crate) fn account_menu(&mut self, cx: &mut Context<Self>) -> Option<Stateful<Div>> {
        if !shows(self.ui.sheet, self.sign_in_screen.open) {
            self.account_menu.sub = None;
            self.menu_closed();
            return None;
        }
        self.account_menu.was_open = true;
        if self.account_menu.sub.is_none() {
            self.account_menu.in_sub = false;
        }
        let (top, right) = place(self.account_menu.chip.get(), self.window_width);
        let mut list = div()
            .id("account-menu-list")
            .flex()
            .flex_col()
            .min_h_0()
            .overflow_y_scroll()
            .track_scroll(&crate::frame::scrolls("account-menu"));
        for (n, row) in rows(&self.accounts, &self.auth_status)
            .into_iter()
            .enumerate()
        {
            list = list.child(self.account_row(n, row, cx));
        }
        let submenu = self.account_submenu(right, cx);
        let act = |id: &'static str, icon: &'static str, label: &str, action: Action| {
            item(id, icon, label, palette().text)
                .on_hover(cx.listener(|this, hovered: &bool, _, cx| {
                    if *hovered {
                        this.sub_close_now(cx);
                    }
                }))
                .on_click(cx.listener(move |this, _: &ClickEvent, _, cx| {
                    this.account_action(action.clone());
                    cx.notify();
                }))
        };
        let card = card()
            .id("account-menu-card")
            .absolute()
            .top(px(top))
            .right(px(right))
            .w(px(MENU_WIDTH))
            .max_w(px((self.window_width - 2. * EDGE).max(0.)))
            .max_h(px((self.window_height - top - EDGE).max(0.)))
            .child(
                div()
                    .flex_none()
                    .px(px(8.))
                    .pt(px(4.))
                    .pb(px(2.))
                    .text_size(px(tokens::SMALL))
                    .text_color(rgb(palette().muted))
                    .child(tr("Play as")),
            )
            .child(list)
            .child(dropdown::rule())
            .child(act(
                "account-menu-add",
                "plus-sign",
                tr("Add another account"),
                Action::Add,
            ))
            .child(act(
                "account-menu-manage",
                "user",
                tr("Manage accounts"),
                Action::Manage,
            ))
            .child(act(
                "account-menu-settings",
                "sliders-horizontal",
                tr("Settings"),
                Action::Settings,
            ))
            .on_hover(cx.listener(|this, hovered: &bool, _, cx| {
                this.account_menu.in_card = *hovered;
                if *hovered {
                    this.menu_enter();
                } else {
                    this.menu_leave(cx);
                }
            }))
            // As the dropdown's menu: it fades in from 4 px nearer the chip.
            .with_animation(
                "account-menu-in",
                crate::motion::eased(sunrise_launcher_theme::QUICK),
                move |d, t| d.opacity(t).top(px(top - SLIDE * (1. - t))),
            );
        Some(
            div()
                .id("account-menu")
                .absolute()
                .inset_0()
                .occlude()
                .on_scroll(|_, _, cx| cx.stop_propagation())
                .on_mouse_move(cx.listener(|this, ev: &MouseMoveEvent, _, cx| {
                    this.pointer_moved(ev.position, cx)
                }))
                // On the layer, which has the keyboard until a row does.
                .on_key_down(cx.listener(|this, ev: &KeyDownEvent, window, cx| {
                    let back = match ev.keystroke.key.as_str() {
                        "down" => false,
                        "up" => true,
                        _ => return,
                    };
                    this.step_in_layer(back, window, cx);
                    cx.stop_propagation();
                }))
                .on_mouse_down(
                    MouseButton::Left,
                    cx.listener(|this, _: &MouseDownEvent, _, cx| {
                        this.close_sheet();
                        cx.notify();
                    }),
                )
                .child(card)
                .children(submenu),
        )
    }
}
