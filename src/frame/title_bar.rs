//! The one title bar across the top of the window (Figma): the sidebar toggle and the brand on
//! the left, the search field in the middle, the account and, where the launcher draws them,
//! the window controls on the right. The launcher's update notice is in the sidebar. Alt+Left goes back (`frame`'s key capture).

use super::*;
use crate::titlebar;
use crate::widgets::button::{icon_only, Look, Size, Style};
use gpui::{div, prelude::*, px, Div, FontWeight, Stateful};

impl Launcher {
    pub(super) fn title_bar(&mut self, window: &mut Window, cx: &mut Context<Self>) -> Div {
        let p = palette();
        let sidebar = self.sidebar_toggle(cx);
        let search = self.search_button(cx);
        let controls = titlebar::controls(&self.theme, window, cx);
        let chip_name = match self.accounts.selected() {
            Some(account) => account.name.clone(),
            // "Choose who plays" or the signed-out button: about as long as a name.
            None => tr("Choose who plays").to_string(),
        };
        let compact_account = chip_is_compact(
            self.window_width,
            crate::account_chip::full_width(&chip_name),
            controls.is_some(),
        );
        let account = self.account_chip(compact_account, cx);
        let drag = titlebar::drag_area(window, cx);
        let over_page =
            self.settings.header_align == sunrise_launcher::backend::HeaderAlign::CenterPanel;
        let rail = crate::rail::rail_width(self.rail_open());
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
            // The search field sits in the bar's middle, whatever stands either side of it; with
            // the Home header centred on the page, in the page's middle over it instead.
            .child(
                div()
                    .absolute()
                    .inset_0()
                    .when(over_page, |d| d.pl(px(rail)))
                    .flex()
                    .items_center()
                    .justify_center()
                    .child(search),
            )
            // The toggle is centred over the folded sidebar's icons (27 px in from the window's
            // edge, past the bar's 8 px padding), and the brand keeps clear of it.
            .child(
                div()
                    .flex_none()
                    .w(px(2. * (crate::rail::CLOSED_ICON_CENTRE - 8.)))
                    .flex()
                    .justify_center()
                    .child(sidebar),
            )
            .child(brand().ml(px(6.)))
            .child(drag)
            .child(account)
            .children(controls)
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

/// The least gap kept between the search field and the account chip.
const CHIP_GAP: f32 = 16.;

/// Whether the account chip shows only its face: when its full width would leave less than
/// `CHIP_GAP` between it and the centred search field. The right end of the bar is the
/// window controls and the chip, with the bar's 8 px padding and gaps.
fn chip_is_compact(window: f32, chip: f32, controls: bool) -> bool {
    let search_right = window / 2. + crate::command_search::search_width(window) / 2.;
    let controls = if controls { 3. * 46. + 8. } else { 0. };
    let chip_left = window - 8. - controls - chip;
    chip_left - search_right < CHIP_GAP
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
