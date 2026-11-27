//! The chip's menu opens when the pointer rests on the chip and closes when it has left the chip,
//! the card and the submenu. Both go through a short delay, and a counter tells a timer that the
//! pointer has done something since it started, as the submenu's close timer does.
//!
//! The menu's layer covers the window and blocks the mouse behind it, so once it is open the chip
//! no longer hears the pointer leave: the layer follows the pointer over the chip's bounds
//! instead (`pointer_moved`).
use super::*;
use std::time::Duration;

/// How long the pointer rests on the chip before its menu opens: long enough that a pointer
/// passing over the title bar opens nothing.
const OPEN_DELAY: Duration = Duration::from_millis(120);
/// How long the pointer may be away from the chip, the card and the submenu before the menu
/// closes, so a path across the gap between them doesn't lose it.
const CLOSE_DELAY: Duration = Duration::from_millis(250);

impl Launcher {
    /// The pointer came onto or left the closed chip.
    pub(super) fn chip_hover(&mut self, hovered: bool, cx: &mut Context<Self>) {
        // Once the menu is open its layer sits over the chip, which then reports a leave that
        // isn't one.
        if self.ui.sheet == Some(Sheet::AccountMenu) {
            return;
        }
        let menu = &mut self.account_menu;
        menu.hover_chip = hovered;
        menu.hovers += 1;
        if !hovered {
            menu.blocked = false;
            return;
        }
        if menu.blocked {
            return;
        }
        let mine = menu.hovers;
        cx.spawn(async move |this, cx| {
            cx.background_executor().timer(OPEN_DELAY).await;
            let _ = this.update(cx, |this, cx| {
                if this.account_menu.hovers == mine && this.ui.sheet.is_none() {
                    this.account_menu.sub = None;
                    this.open_sheet(Sheet::AccountMenu);
                    cx.notify();
                }
            });
        })
        .detach();
    }

    /// The pointer is on the card or the submenu: no close timer still running may shut the menu.
    pub(super) fn menu_enter(&mut self) {
        self.account_menu.hovers += 1;
    }

    /// The pointer left the chip, the card or the submenu: the menu closes after a short while,
    /// unless the pointer is on one of them again by then.
    pub(super) fn menu_leave(&mut self, cx: &mut Context<Self>) {
        self.account_menu.hovers += 1;
        let mine = self.account_menu.hovers;
        cx.spawn(async move |this, cx| {
            cx.background_executor().timer(CLOSE_DELAY).await;
            let _ = this.update(cx, |this, cx| {
                let menu = &this.account_menu;
                let away = !menu.hover_chip && !menu.in_card && !menu.in_sub;
                if menu.hovers == mine && away && this.ui.sheet == Some(Sheet::AccountMenu) {
                    this.close_sheet();
                    cx.notify();
                }
            });
        })
        .detach();
    }

    /// The pointer moved over the menu's layer: note whether it is on the chip's place.
    pub(super) fn pointer_moved(&mut self, at: gpui::Point<Pixels>, cx: &mut Context<Self>) {
        let on_chip = self
            .account_menu
            .chip
            .get()
            .is_some_and(|chip| chip.contains(&at));
        if on_chip == self.account_menu.hover_chip {
            return;
        }
        self.account_menu.hover_chip = on_chip;
        if on_chip {
            self.menu_enter();
        } else {
            self.menu_leave(cx);
        }
    }

    /// Called while the menu is not drawn: when it has just closed, the pointer's places are
    /// forgotten, and a pointer still on the chip doesn't open it again until it has left.
    pub(super) fn menu_closed(&mut self) {
        let menu = &mut self.account_menu;
        if !menu.was_open {
            return;
        }
        menu.was_open = false;
        menu.blocked = menu.hover_chip;
        menu.in_card = false;
        menu.in_sub = false;
        menu.hovers += 1;
    }
}
