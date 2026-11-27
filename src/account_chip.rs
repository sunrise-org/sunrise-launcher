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
use gpui::{rgb, Pixels};
use std::cell::{Cell, RefCell};
use std::collections::HashMap;
use std::rc::Rc;

mod menu;

/// The side of a face on the chip, the same height as the search field's icon row.
const CHIP_FACE: f32 = 22.;
/// The side of a face in the menu.
const FACE: f32 = 28.;
/// The chip is as tall as the search field and the signed-out chip beside it.
const CHIP_HEIGHT: f32 = 34.;
const MENU_WIDTH: f32 = 254.;
/// A menu's action row: an icon and a label, one line.
const MENU_ROW: f32 = 32.;
/// The gap between the chip and its menu, and the least the menu keeps from the window's edge.
const GAP: f32 = 6.;
const EDGE: f32 = 8.;

/// Where the chip was last painted: the menu hangs under it, wherever the frame puts it.
#[derive(Default)]
pub(crate) struct Menu {
    chip: Rc<Cell<Option<Bounds<Pixels>>>>,
    /// Where each account's row was last painted: its submenu opens level with it.
    rows: Rc<RefCell<HashMap<String, Bounds<Pixels>>>>,
    /// The account whose submenu is open.
    sub: Option<String>,
    /// Counts the pointer's comings and goings, so a close timer started by a leave does nothing
    /// when the pointer has come back since.
    leaves: u64,
}

/// One account under "Play as".
#[derive(Debug, PartialEq)]
pub(crate) struct Row {
    pub uuid: String,
    pub name: String,
    /// The game starts as this one: its row carries the tick.
    pub plays: bool,
    /// The saved session expired or needs browser sign-in again.
    pub signed_out: bool,
}

/// Every account signed in here, in the order they were added.
pub(crate) fn rows(
    accounts: &Accounts,
    auth_status: &std::collections::HashMap<String, account::session::Status>,
) -> Vec<Row> {
    accounts
        .accounts
        .iter()
        .map(|a| Row {
            uuid: a.uuid.clone(),
            name: a.name.clone(),
            plays: accounts.selected.as_deref() == Some(a.uuid.as_str()),
            signed_out: auth_status
                .get(&a.uuid)
                .is_some_and(|s| s.state == account::session::State::SignInRequired),
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
    /// Signs this account in again, on the sign-in screen.
    SignInAgain(String),
    /// Puts the account's UUID on the clipboard (the row has already done that: this closes the
    /// menu and says so).
    CopyUuid(String),
    /// Makes the account the one that plays and opens the Skins page.
    ManageSkin(String),
    /// Signs the account out of this launcher.
    Remove(String),
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
    crate::i18n::tr_key(if signed_in > 0 && !for_play && !again {
        "Add a Microsoft account"
    } else {
        "Sign in to play"
    })
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
    fn face(&self, uuid: &str, size: f32) -> Div {
        div()
            .flex_none()
            .rounded(crate::tokens::r(crate::tokens::R_SMALL))
            .overflow_hidden()
            .child(
                self.profile_heads
                    .draw(Some(uuid), size, self.scale, &self.theme),
            )
    }

    /// The top bar's account control: the chip, or Microsoft's sign-in button when nobody is
    /// signed in.
    pub(crate) fn account_chip(&mut self, compact: bool, cx: &mut Context<Self>) -> Stateful<Div> {
        if self.accounts.accounts.is_empty() {
            let p = palette();
            let label = tr("Sign in with Microsoft");
            let icon_only = compact && self.bar_width() < 750.;
            // Figma: a small raised chip, the Microsoft mark and "Sign in".
            return div()
                .id("account-sign-in")
                .flex()
                .flex_none()
                .items_center()
                // As tall as the search field beside it.
                .h(px(34.))
                .px(px(12.))
                .gap(px(8.))
                .rounded(crate::tokens::r(crate::tokens::R_CONTROL))
                .bg(rgb(p.chrome_raised))
                .text_size(px(tokens::LABEL))
                .font_weight(FontWeight::MEDIUM)
                .text_color(rgb(p.chrome_text))
                .whitespace_nowrap()
                .cursor_pointer()
                .hover(move |s| s.bg(rgb(p.hover)))
                .reach()
                .tooltip(crate::glass_ui::tip(label))
                .child(crate::microsoft::logo(16.))
                .when(!icon_only, |chip| chip.child(tr("Sign in")))
                .on_click(cx.listener(|this, _, _, cx| {
                    this.account_action(Action::Add);
                    cx.notify();
                }));
        }
        let open = self.ui.sheet == Some(Sheet::AccountMenu);
        let who = self.accounts.selected();
        let name = who.map_or_else(|| tr("Choose who plays").to_string(), |a| a.name.clone());
        let face = match who {
            Some(account) => self.face(&account.uuid, CHIP_FACE),
            // Accounts are here, but the one that played signed out: none is picked yet.
            None => self
                .profile_heads
                .draw(None, CHIP_FACE, self.scale, &self.theme),
        };
        let seen = self.account_menu.chip.clone();
        div()
            .id("account-chip")
            .min_w_0()
            .flex()
            .items_center()
            .gap(px(8.))
            .h(px(CHIP_HEIGHT))
            .pl(px(6.))
            .pr(px(8.))
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
            .when(compact, |chip| {
                chip.tooltip(crate::glass_ui::tip(name.clone()))
            })
            .when(!compact, |chip| {
                chip.child(
                    div().flex().flex_col().min_w_0().max_w(px(160.)).child(
                        div()
                            .truncate()
                            .text_size(px(tokens::BODY))
                            .font_weight(FontWeight::MEDIUM)
                            .text_color(rgb(palette().chrome_text))
                            .child(name),
                    ),
                )
            })
            .child(crate::widgets::motion_bits::chevron(
                "account-chip",
                open,
                12.,
                rgb(palette().chrome_muted).into(),
            ))
            .on_click(cx.listener(|this, _, _, cx| {
                this.account_menu.sub = None;
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
        let locked_by = match &action {
            Action::PlayAs(_) | Action::Add => true,
            // Making it the player is what switches; the account that plays already is no switch.
            Action::ManageSkin(uuid) => self.accounts.selected.as_deref() != Some(uuid.as_str()),
            _ => false,
        };
        if locked_by && self.account_locked() {
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
            Action::SignInAgain(uuid) => self.re_sign_in(&uuid),
            Action::CopyUuid(uuid) => crate::toast::copied(&uuid),
            Action::ManageSkin(uuid) => {
                if self.accounts.selected.as_deref() != Some(uuid.as_str()) {
                    self.play_as(&uuid);
                }
                self.navigate(Page::Skins);
            }
            Action::Remove(uuid) => self.remove_account(&uuid),
        }
    }
}

#[cfg(test)]
mod tests;
