//! The one title bar across the top of the window (Figma): the sidebar toggle and the brand on
//! the left, the search field in the middle, the update, the account and, where the launcher
//! draws them, the window controls on the right. Alt+Left goes back (`frame`'s key capture).

use super::*;
use crate::titlebar;
use crate::widgets::button::{icon_only, Look, Size, Style};
use gpui::{div, prelude::*, px, Div, FontWeight, Stateful};

impl Launcher {
    pub(super) fn title_bar(&mut self, window: &mut Window, cx: &mut Context<Self>) -> Div {
        let p = palette();
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
            // The search field sits in the bar's middle, whatever stands either side of it.
            .child(
                div()
                    .absolute()
                    .inset_0()
                    .flex()
                    .items_center()
                    .justify_center()
                    .child(search),
            )
            .child(sidebar)
            .child(brand())
            .child(drag)
            .children(update)
            .child(account)
            .children(controls)
    }

    /// Shows or hides the sidebar. Its tooltip is read when it shows: a press that is still
    /// easing the sidebar offers what it will do, not what it just did.
    fn sidebar_toggle(&mut self, cx: &mut Context<Self>) -> Stateful<Div> {
        let me = cx.weak_entity();
        icon_only(
            "sidebar-toggle",
            Look::new(Style::Ghost, Size::Medium).icon("menu-01"),
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
                .text_size(px(tokens::BODY))
                .font_weight(FontWeight::MEDIUM)
                .text_color(rgb(p.text))
                .child(tr("Sunrise")),
        )
}
