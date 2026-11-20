//! Who plays, at the top right: the account chip and the menu under it.
//!
//! The chip shows the selected Microsoft account (its face, its name); signed out, the same spot
//! is Microsoft's own "Sign in with Microsoft". The menu is Umut's switcher: every account signed
//! in here with its face, one click to play as another, and the ways to add one and to manage
//! them, so changing who plays is no trip to Settings.
//!
//! The menu is a sheet (`Sheet::AccountMenu`), so the frame's one rule for layers holds it: it
//! takes the keyboard, Tab and the arrow keys stay inside it, Escape or a press outside closes it and the keyboard
//! goes back to the chip. The sign-in that "Add another account" starts opens over the menu and
//! comes back to it, however it ends.
use super::*;
use crate::frame::{Page, Reach as _};
use crate::paint::palette;
use crate::settings::Section;
use crate::sheets::Sheet;
use gpui::{rgb, MouseButton, MouseDownEvent, Pixels};
use std::cell::Cell;
use std::rc::Rc;

/// The side of a face on the chip and in the menu.
const FACE: f32 = 28.;
const MENU_WIDTH: f32 = 254.;
/// The gap between the chip and its menu, and the least the menu keeps from the window's edge.
const GAP: f32 = 6.;
const EDGE: f32 = 8.;

/// Where the chip was last painted: the menu hangs under it, wherever the frame puts it.
#[derive(Default)]
pub(crate) struct Menu {
    chip: Rc<Cell<Option<Bounds<Pixels>>>>,
}

/// One account under "Play as".
#[derive(Debug, PartialEq)]
pub(crate) struct Row {
    pub uuid: String,
    pub name: String,
    /// The game starts as this one: its row carries the tick.
    pub plays: bool,
}

/// Every account signed in here, in the order they were added.
pub(crate) fn rows(accounts: &Accounts) -> Vec<Row> {
    accounts
        .accounts
        .iter()
        .map(|a| Row {
            uuid: a.uuid.clone(),
            name: a.name.clone(),
            plays: accounts.selected.as_deref() == Some(a.uuid.as_str()),
        })
        .collect()
}

/// What the menu can do.
#[derive(Clone, Debug, PartialEq)]
pub(crate) enum Action {
    PlayAs(String),
    Add,
    Manage,
    Settings,
}

/// The sheet left open by `action`. Adding an account keeps the menu: the sign-in opens over it
/// and gives way to it again. Everything else closes it, which hands the keyboard back to the
/// chip.
pub(crate) fn sheet_after(action: &Action) -> Option<Sheet> {
    matches!(action, Action::Add).then_some(Sheet::AccountMenu)
}

/// Whether the menu is drawn: it is open, and no sign-in is over it.
pub(crate) fn shows(sheet: Option<Sheet>, signing_in: bool) -> bool {
    sheet == Some(Sheet::AccountMenu) && !signing_in
}

/// Whether a sign-in that ended well goes on to start the game: only one that a Play asked for
/// (`asked`, the setup it was pressed on), and only while that setup is still the selected one.
/// A sign-in the menu started asked for nothing.
pub(crate) fn plays_after_sign_in(asked: Option<String>, selected: Option<&str>) -> bool {
    asked.is_some_and(|id| selected == Some(id.as_str()))
}

/// The sign-in screen's title. With accounts already here, a sign-in that no Play asked for and
/// that renews none of them adds one.
pub(crate) fn sign_in_title(for_play: bool, again: bool, signed_in: usize) -> &'static str {
    if signed_in > 0 && !for_play && !again {
        "Add a Microsoft account"
    } else {
        "Sign in to play"
    }
}

/// Where the menu goes: its top, and how far its right edge is from the window's. Under the
/// chip, flush with its right edge.
fn place(chip: Option<Bounds<Pixels>>, window_width: f32) -> (f32, f32) {
    match chip {
        Some(chip) => (
            f32::from(chip.bottom()) + GAP,
            (window_width - f32::from(chip.right())).max(EDGE),
        ),
        None => (tokens::FRAME_INSET + tokens::HEADER + GAP, EDGE),
    }
}

impl Launcher {
    fn face(&self, uuid: &str) -> Div {
        div()
            .flex_none()
            .rounded(crate::tokens::r(crate::tokens::R_SMALL))
            .overflow_hidden()
            .child(
                self.profile_heads
                    .draw(Some(uuid), FACE, self.scale, &self.theme),
            )
    }

    /// The top bar's account control: the chip, or Microsoft's sign-in button when nobody is
    /// signed in.
    pub(crate) fn account_chip(&mut self, cx: &mut Context<Self>) -> Stateful<Div> {
        if self.accounts.accounts.is_empty() {
            let p = palette();
            let label = tr("Sign in with Microsoft");
            return crate::frame::input(false)
                .id("account-sign-in")
                .min_w_0()
                .h(px(tokens::FIELD))
                .px(px(12.))
                .gap(px(8.))
                .text_size(px(tokens::LABEL))
                .font_weight(FontWeight::SEMIBOLD)
                .text_color(rgb(p.chrome_text))
                .whitespace_nowrap()
                .cursor_pointer()
                .hover(move |s| s.bg(rgb(p.chrome_raised)))
                .reach_edge()
                .tooltip(crate::glass_ui::tip(label))
                .child(crate::microsoft::logo(18.))
                .child(div().min_w_0().truncate().child(label))
                .on_click(cx.listener(|this, _, _, cx| {
                    this.account_action(Action::Add);
                    cx.notify();
                }));
        }
        let open = self.ui.sheet == Some(Sheet::AccountMenu);
        let who = self.accounts.selected();
        let name = who.map_or_else(|| tr("Choose who plays").to_string(), |a| a.name.clone());
        let face = match who {
            Some(account) => self.face(&account.uuid),
            // Accounts are here, but the one that played signed out: none is picked yet.
            None => self.profile_heads.draw(None, FACE, self.scale, &self.theme),
        };
        let seen = self.account_menu.chip.clone();
        div()
            .id("account-chip")
            .min_w_0()
            .flex()
            .items_center()
            .gap(px(10.))
            .h(px(tokens::FIELD))
            .pl(px(8.))
            .pr(px(10.))
            .rounded(crate::tokens::r(crate::tokens::R_CONTROL))
            .border_1()
            .border_color(palette().input_edge())
            .bg(palette().input_fill())
            .when(open, |d| d.bg(rgb(palette().chrome_raised)))
            .hover(|s| s.bg(rgb(palette().chrome_raised)))
            .cursor_pointer()
            .reach_edge()
            .child(
                gpui::canvas(move |bounds, _, _| seen.set(Some(bounds)), |_, _, _, _| {})
                    .absolute()
                    .inset_0(),
            )
            .child(face)
            .child(
                div().flex().flex_col().min_w_0().max_w(px(160.)).child(
                    div()
                        .truncate()
                        .text_size(px(tokens::BODY))
                        .font_weight(FontWeight::SEMIBOLD)
                        .text_color(rgb(palette().chrome_text))
                        .child(name),
                ),
            )
            .child(crate::glass_ui::icon(
                "arrow-down-01",
                14.,
                rgb(palette().chrome_muted).into(),
            ))
            .on_click(cx.listener(|this, _, _, cx| {
                if this.ui.sheet == Some(Sheet::AccountMenu) {
                    this.close_sheet();
                } else {
                    this.open_sheet(Sheet::AccountMenu);
                }
                cx.notify();
            }))
    }

    /// Does what a row of the menu says.
    pub(crate) fn account_action(&mut self, action: Action) {
        // Who plays is settled while a game starts: the menu stays, and says so.
        if matches!(action, Action::PlayAs(_) | Action::Add) && self.account_locked() {
            self.say_account_locked();
            return;
        }
        self.ui.sheet = sheet_after(&action);
        match action {
            Action::PlayAs(uuid) => self.play_as(&uuid),
            Action::Add => {
                // A sign-in a Play left behind would start the game when this one ends.
                self.cancel_sign_in();
                self.sign_in_to_play();
            }
            Action::Manage => {
                self.settings_open(Section::Account);
                self.navigate(Page::Settings);
            }
            Action::Settings => self.navigate(Page::Settings),
        }
    }

    /// The menu, as the layer the frame draws over the page; none while it is closed or the
    /// sign-in it started is in front.
    pub(crate) fn account_menu(&mut self, cx: &mut Context<Self>) -> Option<Stateful<Div>> {
        if !shows(self.ui.sheet, self.owner_screen_open) {
            return None;
        }
        let rows = rows(&self.accounts);
        let mut list = div()
            .id("account-menu-list")
            .flex()
            .flex_col()
            .min_h_0()
            .overflow_y_scroll()
            .track_scroll(&crate::frame::scrolls("account-menu"));
        for (n, row) in rows.into_iter().enumerate() {
            let uuid = row.uuid.clone();
            list = list.child(
                div()
                    .id(ElementId::named_usize("account-menu-play-as", n))
                    .reach()
                    .flex_none()
                    .flex()
                    .items_center()
                    .gap(px(10.))
                    .h(px(tokens::ACTION))
                    .px(px(8.))
                    .cursor_pointer()
                    .hover(|s| s.bg(rgb(palette().hover)))
                    .child(self.face(&row.uuid))
                    .child(
                        div()
                            .flex_1()
                            .min_w_0()
                            .truncate()
                            .font_weight(FontWeight::SEMIBOLD)
                            .child(row.name),
                    )
                    .children(row.plays.then(|| {
                        crate::glass_ui::icon("tick-02", 16., rgb(palette().accent_ink).into())
                    }))
                    .on_click(cx.listener(move |this, _, _, cx| {
                        this.account_action(Action::PlayAs(uuid.clone()));
                        cx.notify();
                    })),
            );
        }
        let act = |id: &'static str, icon: &'static str, label: &str, action: Action| {
            div()
                .id(id)
                .reach()
                .flex_none()
                .flex()
                .items_center()
                .gap(px(10.))
                .h(px(tokens::FIELD))
                .px(px(8.))
                .cursor_pointer()
                .text_size(px(tokens::LABEL))
                .hover(|s| s.bg(rgb(palette().hover)))
                .child(div().flex_none().w(px(FACE)).flex().justify_center().child(
                    crate::glass_ui::icon(icon, 16., rgb(palette().muted).into()),
                ))
                .child(label.to_string())
                .on_click(cx.listener(move |this, _, _, cx| {
                    this.account_action(action.clone());
                    cx.notify();
                }))
        };
        let (top, right) = place(self.account_menu.chip.get(), self.window_width);
        let card = div()
            .absolute()
            .top(px(top))
            .right(px(right))
            .w(px(MENU_WIDTH))
            .max_w(px((self.window_width - 2. * EDGE).max(0.)))
            .max_h(px((self.window_height - top - EDGE).max(0.)))
            .flex()
            .flex_col()
            .p(px(8.))
            .rounded(crate::tokens::r(crate::tokens::R_CARD))
            .border_1()
            .border_color(palette().input_edge())
            .bg(rgb(palette().raised))
            .text_color(rgb(palette().text))
            .shadow_lg()
            .on_mouse_down(MouseButton::Left, |_: &MouseDownEvent, _, cx| {
                cx.stop_propagation()
            })
            .child(
                div()
                    .flex_none()
                    .px(px(8.))
                    .pt(px(4.))
                    .pb(px(6.))
                    .text_size(px(tokens::SMALL))
                    .font_weight(FontWeight::SEMIBOLD)
                    .text_color(rgb(palette().muted))
                    .child(tr("Play as")),
            )
            .child(list)
            .child(
                div()
                    .flex_none()
                    .mx(px(8.))
                    .my(px(6.))
                    .h(px(1.))
                    .bg(rgb(palette().divider)),
            )
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
            ));
        Some(
            div()
                .id("account-menu")
                .absolute()
                .inset_0()
                .occlude()
                .on_scroll(|_, _, cx| cx.stop_propagation())
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
                .child(card),
        )
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn account(uuid: &str, name: &str) -> account::Account {
        account::Account {
            uuid: uuid.into(),
            name: name.into(),
            skin: None,
            cape: None,
        }
    }

    fn three(selected: Option<&str>) -> Accounts {
        let mut accounts = Accounts::default();
        accounts.accounts = vec![
            account("a1", "Juniper"),
            account("b2", "Mossbyte"),
            account("c3", "Cinder"),
        ];
        accounts.selected = selected.map(Into::into);
        accounts
    }

    #[test]
    fn every_account_is_listed_and_only_the_one_that_plays_is_ticked() {
        let listed = rows(&three(Some("b2")));
        let names: Vec<_> = listed.iter().map(|r| r.name.as_str()).collect();
        assert_eq!(names, ["Juniper", "Mossbyte", "Cinder"]);
        let ticked: Vec<_> = listed.iter().map(|r| r.plays).collect();
        assert_eq!(ticked, [false, true, false]);
    }

    #[test]
    fn the_tick_follows_a_switch_and_nobody_is_ticked_when_nobody_is_picked() {
        let mut accounts = three(Some("a1"));
        accounts.play_as("c3").unwrap();
        let listed = rows(&accounts);
        assert_eq!(
            listed.iter().position(|r| r.plays),
            Some(2),
            "the row switched to"
        );
        assert_eq!(listed.iter().filter(|r| r.plays).count(), 1);
        assert!(rows(&three(None)).iter().all(|r| !r.plays));
    }

    #[test]
    fn adding_keeps_the_menu_and_every_other_action_closes_it() {
        assert_eq!(sheet_after(&Action::Add), Some(Sheet::AccountMenu));
        for closes in [
            Action::PlayAs("a1".into()),
            Action::Manage,
            Action::Settings,
        ] {
            assert_eq!(sheet_after(&closes), None, "{closes:?}");
        }
    }

    #[test]
    fn the_menu_is_back_when_the_sign_in_it_started_ends() {
        let open = sheet_after(&Action::Add);
        assert!(!shows(open, true), "the sign-in is in front");
        // Done, refused or cancelled: the sign-in screen closes and nothing else changed.
        assert!(shows(open, false));
        assert!(!shows(None, false));
        assert!(!shows(Some(Sheet::Build), false));
    }

    #[test]
    fn a_sign_in_from_the_menu_adds_an_account_and_never_starts_the_game() {
        assert!(!plays_after_sign_in(None, Some("sunrise")));
        // One a Play asked for does, for the setup it was pressed on.
        assert!(plays_after_sign_in(Some("sunrise".into()), Some("sunrise")));
        assert!(!plays_after_sign_in(Some("pack".into()), Some("sunrise")));
        assert!(!plays_after_sign_in(Some("pack".into()), None));
    }

    #[test]
    fn the_sign_in_opened_to_add_an_account_is_titled_so() {
        assert_eq!(sign_in_title(false, false, 3), "Add a Microsoft account");
        // Nobody signed in yet, a Play that needs it, or one account signing in again.
        assert_eq!(sign_in_title(false, false, 0), "Sign in to play");
        assert_eq!(sign_in_title(true, false, 3), "Sign in to play");
        assert_eq!(sign_in_title(false, true, 3), "Sign in to play");
    }

    #[test]
    fn the_menu_hangs_under_the_chip_and_stays_inside_the_window() {
        let chip = Bounds::new(
            gpui::point(px(1082.), px(20.)),
            gpui::size(px(172.), px(44.)),
        );
        assert_eq!(place(Some(chip), 1280.), (70., 26.));
        // A chip against the window's edge: the menu keeps its margin.
        let flush = Bounds::new(gpui::point(px(628.), px(0.)), gpui::size(px(172.), px(44.)));
        assert_eq!(place(Some(flush), 800.), (50., EDGE));
        assert_eq!(
            place(None, 800.),
            (tokens::FRAME_INSET + tokens::HEADER + GAP, EDGE)
        );
    }
}
