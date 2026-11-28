//! Settings › Appearance: the theme library: the themes a player saved or imported, as compact
//! rows. Saving, renaming, importing and exporting open a sheet (`dialogs`), and Delete needs no
//! asking: the theme's folder goes to the system's trash.
use super::preview::strip;
use super::state::Dialog;
use super::*;
use crate::widgets::button::{icon_button_opening, secondary, Style};
use crate::widgets::check::{checkbox, Tick};
use crate::widgets::focus::c;
use gpui::SharedString;
use sunrise_launcher::theme_files::{self, Local};
use sunrise_launcher::themes::library;

/// The "…" menu's width, and how far under its button it starts.
const MENU_WIDTH: f32 = 180.;
const MENU_DROP: f32 = tokens::CONTROL + 4.;

/// What a row's "…" menu does, in the order `theme_menu_pick` takes: the id of its row, its
/// icon and its label.
const MENU: [(&str, &str, &str); 4] = [
    (
        "alt-theme-rename",
        "pencil-edit-02",
        crate::i18n::tr_key("Rename…"),
    ),
    (
        "alt-theme-duplicate",
        "copy-01",
        crate::i18n::tr_key("Duplicate"),
    ),
    (
        "alt-theme-export",
        "upload-04",
        crate::i18n::tr_key("Export…"),
    ),
    (
        "alt-theme-delete",
        "delete-02",
        crate::i18n::tr_key("Delete"),
    ),
];

impl Launcher {
    /// The library: its buttons and a row per saved theme.
    pub(super) fn library_row(&mut self, cx: &mut Context<Self>) -> Div {
        let saved = library::listed(&theme_files::loaded());
        let painted = library::current(&self.settings);
        let ticked: Vec<String> = saved
            .iter()
            .filter(|local| self.ui.settings.appearance.ticked.contains(&local.slug))
            .map(|local| local.slug.clone())
            .collect();
        let save = secondary("alt-library-save", tr("Save as theme…")).on_click(cx.listener(
            |this, _: &ClickEvent, _, cx| {
                this.dialog_open(Dialog::Save);
                cx.notify();
            },
        ));
        let import = secondary("alt-library-import", tr("Import…")).on_click(cx.listener(
            |this, _: &ClickEvent, _, cx| {
                this.dialog_open(Dialog::Import);
                cx.notify();
            },
        ));
        let export = secondary("alt-library-export", tr("Export ticked…"));
        let export = if ticked.is_empty() {
            off(export)
        } else {
            export.on_click(cx.listener(move |this, _: &ClickEvent, _, cx| {
                this.export_open(&ticked);
                cx.notify();
            }))
        };
        let head = self.wide_row(
            tr("Theme library"),
            tr("Colours you saved or imported. Apply loads a theme's colours into the editor above. Tick several to share them in one code."),
            div().child(save).child(import).child(export),
        );
        let mut list = div().flex().flex_col();
        if saved.is_empty() {
            list = list.child(hint(tr(
                "No saved themes yet. Save the colours above, or import a code.",
            )));
        }
        for (n, local) in saved.iter().enumerate() {
            list = list.child(self.library_item(n, local, local.roles == painted, cx));
        }
        div().flex().flex_col().gap(px(10.)).child(head).child(list)
    }

    /// One saved theme: a tick, its colours, its name, Apply and the "…" menu.
    fn library_item(
        &mut self,
        n: usize,
        local: &Local,
        in_use: bool,
        cx: &mut Context<Self>,
    ) -> Stateful<Div> {
        let slug = local.slug.clone();
        let state = &self.ui.settings.appearance;
        let ticked = state.ticked.contains(&slug);
        let open = state.more.as_deref() == Some(slug.as_str());
        let tick = checkbox(
            ("alt-theme-tick", n),
            if ticked { Tick::On } else { Tick::Off },
        )
        .on_click({
            let slug = slug.clone();
            cx.listener(move |this, _: &ClickEvent, _, cx| {
                let ticked = &mut this.ui.settings.appearance.ticked;
                if !ticked.remove(&slug) {
                    ticked.insert(slug.clone());
                }
                cx.notify();
            })
        });
        let apply = secondary(("alt-theme-apply", n), tr("Apply")).on_click({
            let slug = slug.clone();
            cx.listener(move |this, _: &ClickEvent, window, cx| {
                this.theme_apply(&slug, window);
                cx.notify();
            })
        });
        let name = div()
            .flex_1()
            .min_w_0()
            .flex()
            .items_center()
            .gap(px(6.))
            .child(
                div()
                    .truncate()
                    .font_weight(FontWeight::SEMIBOLD)
                    .text_size(px(tokens::LABEL))
                    .child(local.name.clone()),
            )
            .children(in_use.then(|| blocks::icon("check", 13., rgb(palette().text).into())));
        div()
            .id(("alt-theme-row", n))
            .flex()
            .items_center()
            .gap(px(10.))
            .py(px(8.))
            .when(n > 0, |d| d.border_t_1().border_color(palette().rule()))
            .child(tick)
            .child(strip(&local.roles))
            .child(name)
            .child(apply)
            .child(self.theme_menu(n, slug, open, cx))
    }

    /// The "…" button of the theme in `slug`, as Home's cards have it, and under it, while it
    /// is open, the menu: Rename…, Duplicate, Export…, Delete.
    fn theme_menu(&mut self, n: usize, slug: String, open: bool, cx: &mut Context<Self>) -> Div {
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
