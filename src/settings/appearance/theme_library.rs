//! Settings › Appearance: the "…" menu of a saved theme's tile: Edit…, Duplicate, Share… and
//! Delete. Editing and sharing open a sheet (`dialogs`), and Delete needs no asking: the
//! theme's folder goes to the system's trash.
use super::*;
use crate::widgets::button::{icon_button_opening, Style};
use crate::widgets::focus::c;
use gpui::SharedString;

/// The "…" menu's width, and how far under its button it starts.
const MENU_WIDTH: f32 = 180.;
const MENU_DROP: f32 = tokens::CONTROL + 4.;

/// What a row's "…" menu does, in the order `theme_menu_pick` takes: the id of its row, its
/// icon and its label.
const MENU: [(&str, &str, &str); 4] = [
    (
        "alt-theme-edit",
        "pencil-edit-02",
        crate::i18n::tr_key("Edit…"),
    ),
    (
        "alt-theme-duplicate",
        "copy-01",
        crate::i18n::tr_key("Duplicate"),
    ),
    (
        "alt-theme-export",
        "upload-04",
        crate::i18n::tr_key("Share…"),
    ),
    (
        "alt-theme-delete",
        "delete-02",
        crate::i18n::tr_key("Delete"),
    ),
];

impl Launcher {
    /// The "…" button of the theme in `slug`, as Home's cards have it, and under it, while it
    /// is open, the menu: Edit…, Duplicate, Share…, Delete.
    pub(super) fn theme_menu(
        &mut self,
        n: usize,
        slug: String,
        open: bool,
        cx: &mut Context<Self>,
    ) -> Div {
        let button = icon_button_opening(
            SharedString::from(format!("alt-theme-more-{n}")),
            "more-horizontal",
            tr("More actions"),
            Style::Ghost,
            open,
        )
        .on_click({
            let slug = slug.clone();
            cx.listener(move |this, _: &ClickEvent, _, cx| {
                cx.stop_propagation();
                let was = this.ui.settings.appearance.more.as_deref() == Some(slug.as_str());
                this.close_menus();
                let state = &mut this.ui.settings.appearance;
                state.header_open = false;
                state.more = (!was).then(|| slug.clone());
                cx.notify();
            })
        });
        let menu = open.then(|| self.theme_menu_layer(slug, cx));
        div()
            .relative()
            .flex_none()
            .child(button)
            .children(menu.into_iter().flatten())
    }

    /// The open menu: a layer that takes every press outside it, and the rows.
    fn theme_menu_layer(&mut self, slug: String, cx: &mut Context<Self>) -> [gpui::Deferred; 2] {
        let p = palette();
        let close = cx.listener(|this, _: &gpui::MouseDownEvent, _, cx| {
            this.ui.settings.appearance.more = None;
            cx.notify();
        });
        let backdrop = div()
            .absolute()
            .top(px(-4000.))
            .left(px(-4000.))
            .w(px(8000.))
            .h(px(8000.))
            .occlude()
            .on_mouse_down(gpui::MouseButton::Left, close);
        let mut menu = div()
            .id("alt-theme-menu")
            .w(px(MENU_WIDTH))
            .p(px(4.))
            .flex()
            .flex_col()
            .rounded(tokens::r(tokens::R_CONTROL))
            .border_1()
            .border_color(p.input_edge())
            .bg(rgb(p.raised))
            .text_color(rgb(p.text))
            .shadow_lg()
            .occlude()
            .on_mouse_down(gpui::MouseButton::Left, |_, _, cx| cx.stop_propagation())
            .on_scroll(|_: &gpui::ScrollEvent, _, cx| cx.stop_propagation());
        for (pick, (id, icon, label)) in MENU.into_iter().enumerate() {
            let slug = slug.clone();
            let danger = pick == MENU.len() - 1;
            let ink = if danger { p.danger } else { p.text };
            if danger {
                menu = menu.child(div().h(px(1.)).my(px(4.)).bg(rgb(p.divider)));
            }
            menu = menu.child(
                div()
                    .id(id)
                    .flex()
                    .items_center()
                    .gap(px(8.))
                    .h(px(tokens::CONTROL))
                    .px(px(8.))
                    .rounded(tokens::r(tokens::R_SMALL))
                    .text_size(px(tokens::LABEL))
                    .text_color(rgb(ink))
                    .reach()
                    .cursor_pointer()
                    .hover(|s| s.bg(rgb(p.hover)))
                    .child(crate::glass_ui::icon(icon, 16., c(ink)))
                    .child(tr(label))
                    .on_click(cx.listener(move |this, _: &ClickEvent, window, cx| {
                        cx.stop_propagation();
                        this.theme_menu_pick(slug.clone(), pick, window);
                        cx.notify();
                    })),
            );
        }
        let placed = div().absolute().top(px(MENU_DROP)).right_0().child(
            gpui::anchored()
                .anchor(gpui::Anchor::TopRight)
                .snap_to_window_with_margin(px(8.))
                .child(menu),
        );
        [
            gpui::deferred(backdrop).with_priority(9),
            gpui::deferred(placed).with_priority(10),
        ]
    }
}
