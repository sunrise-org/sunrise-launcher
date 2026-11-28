//! Settings › Appearance: the theme library: the themes a player saved or imported, as compact
//! rows, and the one panel under them where a theme is named, deleted, exported or imported.
use super::inputs::{CODE, NAME};
use super::state::Panel;
use super::*;
use crate::widgets::button::{button, danger, primary, secondary, Look, Size, Style};
use crate::widgets::check::{checkbox, Tick};
use crate::widgets::dropdown::{Dropdown, Item};
use sunrise_launcher::theme_files::{self, Local, Roles};
use sunrise_launcher::themes::library;
use sunrise_launcher::themes::share;

/// A theme's colours side by side: the page, the sidebar, the cards, the text, the accent and
/// the danger colour.
fn strip(roles: &Roles) -> Div {
    let band = |colour: u32| div().w(px(14.)).h(px(22.)).bg(rgb(colour));
    div()
        .flex()
        .flex_none()
        .overflow_hidden()
        .rounded(tokens::r(tokens::R_SMALL))
        .border_1()
        .border_color(rgb(palette().line))
        .children(
            [
                roles.background,
                roles.chrome,
                roles.card,
                roles.text,
                roles.accent,
                roles.danger,
            ]
            .map(band),
        )
}

impl Launcher {
    /// The library: its buttons, a row per saved theme, and the open panel.
    pub(super) fn library_row(&mut self, window: &Window, cx: &mut Context<Self>) -> Div {
        let saved = library::listed(&theme_files::loaded());
        let painted = library::current(&self.settings);
        let ticked: Vec<String> = saved
            .iter()
            .filter(|local| self.ui.settings.appearance.ticked.contains(&local.slug))
            .map(|local| local.slug.clone())
            .collect();
        let save = secondary("alt-library-save", tr("Save as theme…")).on_click(cx.listener(
            |this, _: &ClickEvent, window, cx| {
                this.panel_open(Panel::Save, window, cx);
                cx.notify();
            },
        ));
        let import = secondary("alt-library-import", tr("Import…")).on_click(cx.listener(
            |this, _: &ClickEvent, window, cx| {
                this.panel_open(Panel::Import, window, cx);
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
        let panel = self.library_panel(&saved, window, cx);
        div()
            .flex()
            .flex_col()
            .gap(px(10.))
            .child(head)
            .child(list)
            .children(panel)
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
        let apply = button(
            ("alt-theme-apply", n),
            tr("Apply"),
            Look::new(Style::Secondary, Size::Small),
        )
        .on_click({
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

    /// The "…" menu of the theme in `slug`: Rename…, Duplicate, Export…, Delete….
    fn theme_menu(&mut self, n: usize, slug: String, open: bool, cx: &mut Context<Self>) -> Div {
        let items = vec![
            Item::new(tr("Rename…")).shown("…"),
            Item::new(tr("Duplicate")).shown("…"),
            Item::new(tr("Export…")).shown("…"),
            Item::new(tr("Delete…")).shown("…"),
        ];
        Dropdown::new(format!("alt-theme-more-{n}"), items, open)
            .placeholder("…")
            .menu_min(150.)
            .on_toggle({
                let (this, slug) = (cx.entity(), slug.clone());
                move |_, cx| {
                    this.update(cx, |this, cx| {
                        let state = &mut this.ui.settings.appearance;
                        state.header_open = false;
                        state.more = if state.more.as_deref() == Some(slug.as_str()) {
                            None
                        } else {
                            Some(slug.clone())
                        };
                        cx.notify();
                    })
                }
            })
            .on_close({
                let this = cx.entity();
                move |_, cx| {
                    this.update(cx, |this, cx| {
                        this.ui.settings.appearance.more = None;
                        cx.notify();
                    })
                }
            })
            .on_pick({
                let this = cx.entity();
                move |pick, window, cx| {
                    this.update(cx, |this, cx| {
                        this.theme_menu_pick(slug.clone(), pick, window, cx);
                        cx.notify();
                    })
                }
            })
            .build()
    }

    /// What the open panel asks, under the library's rows.
    fn library_panel(
        &mut self,
        saved: &[Local],
        window: &Window,
        cx: &mut Context<Self>,
    ) -> Option<Div> {
        let panel = self.ui.settings.appearance.panel.clone()?;
        let body = match panel {
            Panel::Save => self.name_panel(false, window, cx),
            Panel::Rename(_) => self.name_panel(true, window, cx),
            Panel::Delete(slugs) => self.delete_panel(&slugs, saved, cx),
            Panel::Export(code, count) => self.export_panel(code, count, cx),
            Panel::Import => self.import_panel(window, cx),
        };
        Some(
            div()
                .flex()
                .flex_col()
                .gap(px(10.))
                .p(px(12.))
                .rounded(tokens::r(tokens::R_CARD))
                .bg(rgb(palette().field))
                .border_1()
                .border_color(palette().rule())
                .child(body),
        )
    }

    /// The panel's Cancel, which closes it.
    fn panel_cancel(&self, label: &'static str, cx: &mut Context<Self>) -> Stateful<Div> {
        secondary("alt-panel-cancel", tr(label)).on_click(cx.listener(
            |this, _: &ClickEvent, window, cx| {
                this.text_cancel(window, cx);
                cx.notify();
            },
        ))
    }

    /// `ok` pressed does what the panel is for.
    fn panel_ok(&self, ok: Stateful<Div>, cx: &mut Context<Self>) -> Stateful<Div> {
        ok.on_click(cx.listener(|this, _: &ClickEvent, window, cx| {
            this.panel_confirm(window, cx);
            cx.notify();
        }))
    }

    /// Save as… and Rename…: a name, and the button that takes it.
    fn name_panel(&mut self, renaming: bool, window: &Window, cx: &mut Context<Self>) -> Div {
        let field = self
            .text_input(NAME, Self::panel_confirm, window, cx)
            .w(px(280.))
            .max_w_full();
        let ok = primary(
            "alt-panel-ok",
            if renaming { tr("Rename") } else { tr("Save") },
        );
        div()
            .flex()
            .flex_wrap()
            .items_center()
            .gap(px(8.))
            .child(field)
            .child(self.panel_ok(ok, cx))
            .child(self.panel_cancel(crate::i18n::tr_key("Cancel"), cx))
    }

    /// Asks before the themes in `slugs` are deleted.
    fn delete_panel(&mut self, slugs: &[String], saved: &[Local], cx: &mut Context<Self>) -> Div {
        let names: Vec<&str> = saved
            .iter()
            .filter(|local| slugs.contains(&local.slug))
            .map(|local| local.name.as_str())
            .collect();
        let ask = match names.as_slice() {
            [one] => trf(
                "Delete {name}? Its colours and picture are removed from the themes folder.",
                &[("name", *one)],
            ),
            _ => trf(
                "Delete {count} themes? Their colours and pictures are removed from the themes folder.",
                &[("count", &slugs.len().to_string())],
            ),
        };
        let ok = self.panel_ok(danger("alt-panel-ok", tr("Delete")), cx);
        div()
            .flex()
            .flex_wrap()
            .items_center()
            .gap(px(8.))
            .child(div().flex_1().min_w(px(200.)).child(ask))
            .child(ok)
            .child(self.panel_cancel(crate::i18n::tr_key("Cancel"), cx))
    }

    /// The code for `count` themes: to read, and to copy.
    fn export_panel(&mut self, code: String, count: usize, cx: &mut Context<Self>) -> Div {
        let shown = div()
            .id("alt-export-code")
            .max_h(px(96.))
            .overflow_y_scroll()
            .p(px(10.))
            .rounded(tokens::r(tokens::R_SMALL))
            .bg(rgb(palette().background))
            .border_1()
            .border_color(rgb(palette().line))
            .font_family(theme::MONO)
            .text_size(px(tokens::SMALL))
            .child(crate::selectable::selectable(code.clone()));
        div().flex().flex_col().gap(px(10.)).child(shown).child(
            div()
                .flex()
                .flex_wrap()
                .items_center()
                .gap(px(10.))
                .child(crate::widgets::motion_bits::copy_button(
                    "alt-export-copy",
                    tr("Copy code"),
                    code,
                ))
                .child(hint(trf(
                    "Themes in this code: {count}",
                    &[("count", &count.to_string())],
                )))
                .child(self.panel_cancel(crate::i18n::tr_key("Close"), cx)),
        )
    }

    /// A pasted code, and a look at the themes it holds before they are added.
    fn import_panel(&mut self, window: &Window, cx: &mut Context<Self>) -> Div {
        let typed = self.ui.settings.appearance.code.text().to_string();
        let decoded = (!typed.trim().is_empty()).then(|| share::decode(&typed));
        let field = self
            .text_input(CODE, Self::panel_confirm, window, cx)
            .w_full();
        let ok = primary("alt-panel-ok", tr("Add to library"));
        let ok = match &decoded {
            Some(Ok(_)) => self.panel_ok(ok, cx),
            _ => off(ok),
        };
        let found = match &decoded {
            Some(Ok(themes)) => Some(preview(themes)),
            Some(Err(why)) => Some(
                div()
                    .text_size(px(tokens::SMALL))
                    .text_color(rgb(palette().danger))
                    .child(why.text()),
            ),
            None => None,
        };
        div()
            .flex()
            .flex_col()
            .gap(px(10.))
            .child(field)
            .children(found)
            .child(
                div()
                    .flex()
                    .gap(px(8.))
                    .child(ok)
                    .child(self.panel_cancel(crate::i18n::tr_key("Cancel"), cx)),
            )
    }
}

/// The themes a pasted code holds: their colours and names, and how many there are.
fn preview(themes: &[library::Custom]) -> Div {
    let mut list = div().flex().flex_col().gap(px(6.));
    for theme in themes {
        list = list.child(
            div()
                .flex()
                .items_center()
                .gap(px(10.))
                .child(strip(&theme.roles))
                .child(
                    div()
                        .flex_1()
                        .min_w_0()
                        .truncate()
                        .child(theme.name.clone()),
                ),
        );
    }
    list.child(hint(trf(
        "Themes in this code: {count}",
        &[("count", &themes.len().to_string())],
    )))
}
