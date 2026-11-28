//! Settings › Appearance: the current theme's bar and Home's header. The colours themselves
//! are edited in the Edit theme dialog (`edit_dialog`).
use super::preview::swatches;
use super::*;
use crate::widgets::button::secondary;
use crate::widgets::dropdown::{Dropdown, Item};
use gpui::SharedString;
use sunrise_launcher::themes::{self, Resolved};

impl Launcher {
    /// Where Home's header stands over the banner.
    pub(super) fn header_row(&mut self, cx: &mut Context<Self>) -> Div {
        use sunrise_launcher::backend::HeaderAlign;
        let current = self.settings.header_align;
        let open = self.ui.settings.appearance.header_open;
        let items: Vec<Item> = HeaderAlign::ALL
            .iter()
            .map(|&a| Item::new(tr(a.label())).current(a == current))
            .collect();
        let dropdown = Dropdown::new("alt-header-align", items, open)
            .on_toggle({
                let this = cx.entity();
                move |_, cx| {
                    this.update(cx, |this, cx| {
                        let was = this.ui.settings.appearance.header_open;
                        this.close_menus();
                        let state = &mut this.ui.settings.appearance;
                        state.more = None;
                        state.header_open = !was;
                        cx.notify();
                    })
                }
            })
            .on_close({
                let this = cx.entity();
                move |_, cx| {
                    this.update(cx, |this, cx| {
                        this.ui.settings.appearance.header_open = false;
                        cx.notify();
                    })
                }
            })
            .on_pick({
                let this = cx.entity();
                move |n, _, cx| {
                    this.update(cx, |this, cx| {
                        if let Some(&picked) = HeaderAlign::ALL.get(n) {
                            this.settings.header_align = picked;
                            this.save_settings();
                        }
                        cx.notify();
                    })
                }
            })
            .build();
        row(
            tr("Home header"),
            tr("Where the profile's name and mark stand over the banner."),
            dropdown,
        )
    }

    /// What the current theme is called: a built-in's name or a saved theme's.
    pub(super) fn current_name(&self) -> String {
        match themes::resolve(self.settings.theme.as_deref()) {
            Resolved::Twilight => tr(themes::TWILIGHT_NAME).to_string(),
            Resolved::Builtin(theme) => tr(theme.name).to_string(),
            Resolved::Local(local) => local.name,
            Resolved::Missing => tr(themes::DAWN_THEME.name).to_string(),
        }
    }

    /// The current theme's bar: its nine colours, its name, Share, and Edit for a saved theme
    /// or Customize (a copy to edit) for a built-in.
    pub(super) fn current_card(&mut self, cx: &mut Context<Self>) -> Div {
        let roles = self.editor_roles();
        let saved = match themes::resolve(self.settings.theme.as_deref()) {
            Resolved::Local(local) => Some(local.slug),
            _ => None,
        };
        let share = secondary("alt-current-share", tr("Share")).on_click(cx.listener(
            |this, _: &ClickEvent, _, cx| {
                this.share_current();
                cx.notify();
            },
        ));
        let label = if saved.is_some() {
            tr("Edit")
        } else {
            tr("Customize")
        };
        let edit = secondary("alt-colours-customize", label).on_click(cx.listener(
            move |this, _: &ClickEvent, window, cx| {
                match &saved {
                    Some(slug) => this.theme_edit(slug, window),
                    None => this.theme_new(window),
                }
                cx.notify();
            },
        ));
        let words = div()
            .flex_1()
            .min_w(px(240.))
            .flex()
            .flex_col()
            .gap(px(8.))
            .child(
                div()
                    .font_weight(FontWeight::SEMIBOLD)
                    .child(self.current_name()),
            )
            .child(swatches(SharedString::from("alt-current-swatch"), &roles));
        div()
            .flex()
            .flex_wrap()
            .items_center()
            .gap_x(px(16.))
            .gap_y(px(10.))
            .p(px(16.))
            .rounded(crate::tokens::r(crate::tokens::R_CARD))
            .border_1()
            .border_color(palette().rule())
            .bg(palette().card())
            .child(words)
            .child(
                div()
                    .flex_none()
                    .flex()
                    .items_center()
                    .gap(px(6.))
                    .child(share)
                    .child(edit),
            )
    }
}
