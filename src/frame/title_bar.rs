//! The one title bar across the top of the window: back, the sidebar toggle, the brand, the drag
//! area, search, the update, the account and, where the launcher draws them, the window controls.

use super::*;
use crate::titlebar;
use crate::widgets::button::{icon_only, Look, Size, Style};
use gpui::{div, prelude::*, px, Div, FontWeight, Stateful};

impl Launcher {
    pub(super) fn title_bar(&mut self, window: &mut Window, cx: &mut Context<Self>) -> Div {
        let p = palette();
        let back = self.back_button(cx);
        let sidebar = self.sidebar_toggle(cx);
        let search = self.search_button(cx);
        let update = self.launcher_update_control(cx);
        let compact_account = self.window_width < 970. && update.is_some();
        let account = self.account_chip(compact_account, cx);
        let controls = titlebar::controls(&self.theme, window, cx);
        let drag = titlebar::drag_area(window, cx);
        div()
            .relative()
            .flex()
            .flex_none()
            .items_center()
            .gap(px(8.))
            .h(px(titlebar::HEIGHT))
            .px(px(8.))
            .when_some(titlebar::inset(window), |bar, inset| {
                bar.pt(px(inset)).px(px(inset))
            })
            // The native traffic lights sit in the bar's left end on macOS (`titlebar::options`).
            .when(cfg!(target_os = "macos"), |bar| bar.pl(px(78.)))
            .font_family(sunrise_launcher_theme::glass::FONT)
            .bg(rgb(p.chrome))
            .border_b_1()
            .border_color(p.rule())
            .child(back)
            .child(sidebar)
            .child(brand())
            .child(drag)
            .child(search)
            .children(update)
            .child(account)
            .children(controls)
    }

    /// Back to the page before this one: disabled with nothing to go back to. The keyboard's
    /// Alt+Left does the same (`frame`'s key capture).
    fn back_button(&mut self, cx: &mut Context<Self>) -> Stateful<Div> {
        let can = !self.ui.back.is_empty();
        let look = Look::new(Style::Ghost, Size::Medium)
            .icon("arrow-left-01")
            .disabled(!can);
        let back = icon_only("title-back", look).tooltip(crate::glass_ui::tip(tr("Back")));
        if !can {
            return back;
        }
        back.on_click(cx.listener(|this, _, _, cx| {
            this.go_back();
            cx.notify();
        }))
    }

    /// Shows or hides the sidebar. Its tooltip is read when it shows: a press that is still
    /// easing the sidebar offers what it will do, not what it just did.
    fn sidebar_toggle(&mut self, cx: &mut Context<Self>) -> Stateful<Div> {
        let me = cx.weak_entity();
        icon_only(
            "sidebar-toggle",
            Look::new(Style::Ghost, Size::Medium).icon("sidebar-left"),
        )
        .tooltip(crate::glass_ui::tip_with(move |cx| {
            let open = me
                .upgrade()
                .is_some_and(|me| me.read(cx).settings.sidebar_expanded);
            if open {
                tr("Hide sidebar")
            } else {
                tr("Show sidebar")
            }
            .into()
        }))
        .on_click(cx.listener(|this, _, _, cx| {
            this.toggle_sidebar();
            cx.notify();
        }))
    }
}

/// The sun and the name, at the bar's left, after the sidebar toggle.
fn brand() -> Div {
    let p = palette();
    div()
        .flex()
        .flex_none()
        .items_center()
        .gap(px(8.))
        .child(crate::widgets::nav::sun())
        .child(
            div()
                .whitespace_nowrap()
                .text_size(px(tokens::LEAD))
                .font_weight(FontWeight::SEMIBOLD)
                .text_color(rgb(p.text))
                .child(tr("Sunrise")),
        )
}
